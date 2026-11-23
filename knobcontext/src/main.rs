mod action;
mod config;
mod error;
mod input;
mod ui;

use crate::action::{ActionDispatcher, DispatchOutcome, EnigoExecutor};
use crate::config::Config;
use crate::error::KnobError;
use crate::input::{KnobEvent, KnobInput};
use crate::ui::state::{OverlayController, OverlayTransition, DEFAULT_OVERLAY_TIMEOUT};
use crate::ui::OverlayWindow;
use env_logger::Env;
use log::{debug, error, info, warn};
use slint::{Timer, TimerMode};
use std::cell::RefCell;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

struct AppRuntime {
    controller: OverlayController,
    dispatcher: ActionDispatcher<EnigoExecutor>,
    overlay: OverlayWindow,
}

impl AppRuntime {
    fn handle_knob_event(&mut self, event: KnobEvent) {
        let transition = self.controller.handle_event(event, Instant::now());
        match transition {
            OverlayTransition::DispatchAction(ev) => match self.dispatcher.dispatch(ev) {
                Ok(DispatchOutcome::Executed(action)) => {
                    info!(
                        "[{}] {ev:?} -> {action:?}",
                        self.dispatcher.active_profile_name()
                    );
                }
                Ok(DispatchOutcome::ProfileSwitched(new_profile)) => {
                    self.controller
                        .sync_active_index(self.dispatcher.active_profile_index());
                    info!("Perfil alterado para '{new_profile}'");
                }
                Ok(DispatchOutcome::Ignored) => {
                    debug!(
                        "[{}] {ev:?} ignorado",
                        self.dispatcher.active_profile_name()
                    );
                }
                Err(err) => {
                    warn!("Falha ao executar ação para {ev:?}: {err}");
                }
            },
            OverlayTransition::OpenOverlay { highlighted_index } => {
                debug!("Abrindo overlay (destaque={highlighted_index})");
                if let Err(err) = self
                    .overlay
                    .show(self.controller.active_index(), highlighted_index)
                {
                    warn!("Falha ao abrir overlay: {err}");
                }
            }
            OverlayTransition::UpdateHighlight { highlighted_index } => {
                debug!("Overlay destaque atualizado: {highlighted_index}");
                self.overlay
                    .update_selection(self.controller.active_index(), highlighted_index);
            }
            OverlayTransition::CommitAndClose { selected_index } => {
                self.dispatcher.set_active_profile_index(selected_index);
                info!(
                    "Perfil selecionado no overlay: '{}'",
                    self.dispatcher.active_profile_name()
                );
                if let Err(err) = self.overlay.hide() {
                    warn!("Falha ao fechar overlay: {err}");
                }
            }
            OverlayTransition::TimeoutClose => {
                debug!("Overlay fechado por inatividade.");
                if let Err(err) = self.overlay.hide() {
                    warn!("Falha ao fechar overlay: {err}");
                }
            }
            OverlayTransition::Consume => {
                debug!("Evento {event:?} consumido enquanto overlay está aberto.");
            }
        }
    }

    fn check_timeout(&mut self) {
        if let Some(OverlayTransition::TimeoutClose) = self.controller.check_timeout(Instant::now())
        {
            debug!("Overlay fechado automaticamente após 3s de inatividade.");
            if let Err(err) = self.overlay.hide() {
                warn!("Falha ao fechar overlay no timeout: {err}");
            }
        }
    }
}

fn run() -> Result<(), KnobError> {
    let (config, loaded_path) = Config::load_default_location()?;
    match loaded_path {
        Some(path) => info!("Configuração carregada de {}", path.display()),
        None => {
            info!("Nenhum arquivo de configuração encontrado; usando configuração padrão embutida.")
        }
    }

    let active_idx = config.active_profile_index();
    let profile_names: Vec<String> = config.profiles.iter().map(|p| p.name.clone()).collect();

    let executor = EnigoExecutor::new()?;
    let dispatcher = ActionDispatcher::new(config.profiles.clone(), active_idx, executor);
    let overlay = OverlayWindow::new(profile_names, active_idx)?;
    let controller =
        OverlayController::new(config.profiles.len(), active_idx, DEFAULT_OVERLAY_TIMEOUT);

    info!(
        "Capturando dispositivos: kbd='{}', media='{}'",
        config.device.kbd_path.display(),
        config.device.media_path.display()
    );
    let mut input = KnobInput::open(&config.device)?;
    info!(
        "KnobContext ativo (perfil: '{}'). Pressione o knob para abrir o overlay ou Ctrl+C para sair.",
        dispatcher.active_profile_name()
    );

    let running = Arc::new(AtomicBool::new(true));
    let running_ctrlc = Arc::clone(&running);
    if let Err(err) = ctrlc::set_handler(move || {
        running_ctrlc.store(false, Ordering::SeqCst);
        let _ = slint::quit_event_loop();
    }) {
        warn!("Não foi possível registrar handler de sinal Ctrl+C: {err}");
    }

    let (tx, rx) = mpsc::channel::<KnobEvent>();
    let running_input = Arc::clone(&running);
    let input_thread = thread::spawn(move || {
        while running_input.load(Ordering::SeqCst) {
            match input.poll_events(100) {
                Ok(events) => {
                    for ev in events {
                        if tx.send(ev).is_err() {
                            return;
                        }
                    }
                }
                Err(err) => {
                    error!("Erro na leitura de eventos evdev: {err}");
                    running_input.store(false, Ordering::SeqCst);
                    let _ = slint::quit_event_loop();
                    return;
                }
            }
        }
        // `input` é descartado aqui ao sair do loop, executando `Drop` -> `ungrab()`.
    });

    let runtime = Rc::new(RefCell::new(AppRuntime {
        controller,
        dispatcher,
        overlay,
    }));

    let runtime_for_timer = Rc::clone(&runtime);
    let poll_timer = Timer::default();
    poll_timer.start(TimerMode::Repeated, Duration::from_millis(20), move || {
        let mut rt = runtime_for_timer.borrow_mut();
        while let Ok(ev) = rx.try_recv() {
            rt.handle_knob_event(ev);
        }
        rt.check_timeout();
    });

    let loop_result =
        slint::run_event_loop_until_quit().map_err(|err| KnobError::UiRun(err.to_string()));

    // Encerramento gracioso: garante fechamento do overlay e `ungrab()` dos dispositivos.
    running.store(false, Ordering::SeqCst);
    let _ = runtime.borrow().overlay.hide();
    let _ = input_thread.join();
    info!("Overlay fechado e dispositivos liberados (ungrab). Atalhos originais restaurados.");

    loop_result
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(Env::default().default_filter_or("warn,knobcontext=info")).init();

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            error!("{err}");
            if err.is_permission_denied() {
                eprintln!("\nPermissão insuficiente para abrir ou capturar o dispositivo evdev.");
                eprintln!("Como resolver no Linux:");
                eprintln!("  1) Adicione seu usuário ao grupo `input` e faça novo login:");
                eprintln!("       sudo usermod -aG input \"$USER\"");
                eprintln!("  2) Ou crie uma regra udev em /etc/udev/rules.d/99-knobcontext.rules:");
                eprintln!(
                    "       SUBSYSTEM==\"input\", ATTRS{{idVendor}}==\"1189\", ATTRS{{idProduct}}==\"8890\", MODE=\"0660\", GROUP=\"input\""
                );
                eprintln!("     e recarregue com: sudo udevadm control --reload-rules && sudo udevadm trigger");
            }
            ExitCode::FAILURE
        }
    }
}
