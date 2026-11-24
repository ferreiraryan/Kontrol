mod action;
mod actions_catalog;
mod config;
mod context;
mod error;
mod input;
mod ui;

use crate::action::{ActionDispatcher, DispatchOutcome, EnigoExecutor};
use crate::config::Config;
use crate::context::{spawn_hyprland_listener, ContextState, ProfileMode};
use crate::error::KnobError;
use crate::input::{KnobEvent, KnobInput};
use crate::ui::settings_mod::SettingsController;
use crate::ui::state::{OverlayController, OverlayTransition, DEFAULT_OVERLAY_TIMEOUT};
use crate::ui::OverlayWindow;
use env_logger::Env;
use log::{debug, error, info, warn};
use slint::{Timer, TimerMode};
use std::cell::RefCell;
use std::env;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

struct AppRuntime {
    config: Config,
    context: Arc<RwLock<ContextState>>,
    controller: OverlayController,
    dispatcher: ActionDispatcher<EnigoExecutor>,
    overlay: OverlayWindow,
}

impl AppRuntime {
    fn overlay_active_index_and_subtitle(&self) -> (usize, String) {
        if let Ok(ctx) = self.context.read() {
            match ctx.mode() {
                ProfileMode::Auto => {
                    let contextual = ctx.contextual_profile_for_window(&self.config);
                    (0, format!("Automático ({contextual})"))
                }
                ProfileMode::Manual(profile_name) => {
                    let idx = self
                        .dispatcher
                        .find_profile_index(profile_name)
                        .map(|i| i + 1)
                        .unwrap_or(0);
                    (idx, "Manual".to_string())
                }
            }
        } else {
            (0, format!("Automático ({})", self.config.active_profile))
        }
    }

    fn persist_mode(&mut self, mode: ProfileMode) {
        if let Ok(mut ctx) = self.context.write() {
            ctx.set_mode(mode.clone());
        }
        self.config.set_profile_mode(&mode);
        if let Err(err) = self.config.save_default_location() {
            warn!("Não foi possível persistir `mode` no config.toml: {err}");
        }
    }

    fn apply_saved_config(&mut self, mut new_config: Config) {
        let current_mode = self
            .context
            .read()
            .map(|ctx| ctx.mode().clone())
            .unwrap_or(ProfileMode::Auto);
        new_config.set_profile_mode(&current_mode);
        let validated_mode = new_config.resolve_profile_mode();
        new_config.set_profile_mode(&validated_mode);
        if let Ok(mut ctx) = self.context.write() {
            ctx.set_mode(validated_mode);
        }

        let profile_names: Vec<String> =
            new_config.profiles.iter().map(|p| p.name.clone()).collect();
        self.dispatcher.update_profiles(new_config.profiles.clone());
        self.config = new_config;

        let (overlay_idx, subtitle) = self.overlay_active_index_and_subtitle();
        self.overlay
            .update_profiles(profile_names, overlay_idx, &subtitle);
        self.controller
            .set_profile_count(self.config.profiles.len() + 1, overlay_idx);
        info!("Configuração salva e aplicada em tempo real.");
    }

    fn sync_context(&mut self) -> String {
        let (overlay_idx, _) = self.overlay_active_index_and_subtitle();
        self.controller.sync_active_index(overlay_idx);

        let resolved = if let Ok(ctx) = self.context.read() {
            ctx.profile_for_window(&self.config)
        } else {
            self.config.active_profile.clone()
        };

        if let Some(idx) = self.dispatcher.find_profile_index(&resolved) {
            self.dispatcher.set_active_profile_index(idx);
        }

        resolved
    }

    fn handle_knob_event(&mut self, event: KnobEvent, settings: &SettingsController) {
        if event == KnobEvent::OpenSettings {
            self.open_settings(settings);
            return;
        }

        let profile_name = self.sync_context();
        let transition = self.controller.handle_event(event, Instant::now());
        match transition {
            OverlayTransition::DispatchAction(ev) => {
                match self.dispatcher.dispatch(ev, &profile_name) {
                    Ok(DispatchOutcome::Executed(action)) => {
                        info!(
                            "[{}] {ev:?} -> {action:?}",
                            self.dispatcher.active_profile_name()
                        );
                    }
                    Ok(DispatchOutcome::ProfileSwitched(new_profile)) => {
                        self.persist_mode(ProfileMode::Manual(new_profile.clone()));
                        let (overlay_idx, _) = self.overlay_active_index_and_subtitle();
                        self.controller.sync_active_index(overlay_idx);
                        info!("Perfil alterado para Manual: '{new_profile}'");
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
                }
            }
            OverlayTransition::OpenOverlay { highlighted_index } => {
                let (active_idx, subtitle) = self.overlay_active_index_and_subtitle();
                debug!("Abrindo overlay (ativo={active_idx}, destaque={highlighted_index})");
                if let Err(err) = self.overlay.show(active_idx, highlighted_index, &subtitle) {
                    warn!("Falha ao abrir overlay: {err}");
                }
            }
            OverlayTransition::UpdateHighlight { highlighted_index } => {
                let (active_idx, subtitle) = self.overlay_active_index_and_subtitle();
                debug!("Overlay destaque atualizado: {highlighted_index}");
                self.overlay
                    .update_selection(active_idx, highlighted_index, &subtitle);
            }
            OverlayTransition::CommitAndClose { selected_index } => {
                if selected_index == 0 {
                    self.persist_mode(ProfileMode::Auto);
                    let resolved = self.sync_context();
                    info!("Modo alterado no overlay: Automático ('{resolved}')");
                } else if let Some(profile) = self.config.profiles.get(selected_index - 1) {
                    let chosen = profile.name.clone();
                    self.persist_mode(ProfileMode::Manual(chosen.clone()));
                    self.dispatcher.set_active_profile_index(selected_index - 1);
                    info!("Modo alterado no overlay: Manual ('{chosen}')");
                }
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

    fn open_settings(&mut self, settings: &SettingsController) {
        self.controller.set_settings_open(true);
        let _ = self.overlay.hide();
        if let Err(err) = settings.open(&self.config) {
            warn!("Falha ao abrir janela de configurações: {err}");
            self.controller.set_settings_open(false);
        } else {
            info!("Janela de configurações aberta (grab mantido ativo).");
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
    let open_settings_on_start = env::args().skip(1).any(|arg| arg == "--settings");

    let (config, loaded_path) = Config::load_default_location()?;
    match loaded_path {
        Some(path) => info!("Configuração carregada de {}", path.display()),
        None => {
            info!("Nenhum arquivo de configuração encontrado; usando configuração padrão embutida.")
        }
    }

    let active_idx = config.active_profile_index();
    let initial_mode = config.resolve_profile_mode();
    let initial_overlay_idx = match &initial_mode {
        ProfileMode::Auto => 0,
        ProfileMode::Manual(name) => config
            .profiles
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(name))
            .map(|i| i + 1)
            .unwrap_or(0),
    };
    let profile_names: Vec<String> = config.profiles.iter().map(|p| p.name.clone()).collect();

    let executor = EnigoExecutor::new(config.device.scroll_step)?;
    let dispatcher = ActionDispatcher::new(config.profiles.clone(), active_idx, executor);
    let overlay = OverlayWindow::new(profile_names, initial_overlay_idx)?;
    let controller = OverlayController::new(
        config.profiles.len() + 1,
        initial_overlay_idx,
        DEFAULT_OVERLAY_TIMEOUT,
    );

    info!(
        "Capturando dispositivos: kbd='{}', media='{}'",
        config.device.kbd_path.display(),
        config.device.media_path.display()
    );
    let mut input = KnobInput::open(&config.device)?;

    let running = Arc::new(AtomicBool::new(true));
    let running_ctrlc = Arc::clone(&running);
    if let Err(err) = ctrlc::set_handler(move || {
        running_ctrlc.store(false, Ordering::SeqCst);
        let _ = slint::quit_event_loop();
    }) {
        warn!("Não foi possível registrar handler de sinal Ctrl+C: {err}");
    }

    let context = Arc::new(RwLock::new(ContextState::with_mode(initial_mode.clone())));
    let hyprland_thread = spawn_hyprland_listener(Arc::clone(&context), Arc::clone(&running));

    info!(
        "KnobContext ativo (modo inicial: {:?}). Pressione o knob para o overlay, Key1+Key2 (ou --settings) para configurações, ou Ctrl+C para sair.",
        initial_mode
    );

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

    let initial_config_for_settings = config.clone();
    let runtime = Rc::new(RefCell::new(AppRuntime {
        config,
        context,
        controller,
        dispatcher,
        overlay,
    }));

    let runtime_on_saved = Rc::clone(&runtime);
    let runtime_on_closed = Rc::clone(&runtime);
    let runtime_on_release = Rc::clone(&runtime);
    let settings = Rc::new(SettingsController::new(
        initial_config_for_settings,
        move |saved_config| {
            runtime_on_saved
                .borrow_mut()
                .apply_saved_config(saved_config);
        },
        move || {
            runtime_on_closed
                .borrow_mut()
                .controller
                .set_settings_open(false);
            debug!("Janela de configurações fechada; overlay reabilitado.");
        },
        move || {
            runtime_on_release
                .borrow_mut()
                .dispatcher
                .release_modifiers();
            debug!("Modificadores liberados preventivamente após captura/fechamento.");
        },
    )?);

    if open_settings_on_start {
        runtime.borrow_mut().open_settings(&settings);
    }

    let runtime_for_timer = Rc::clone(&runtime);
    let settings_for_timer = Rc::clone(&settings);
    let poll_timer = Timer::default();
    poll_timer.start(TimerMode::Repeated, Duration::from_millis(20), move || {
        let mut rt = runtime_for_timer.borrow_mut();
        while let Ok(ev) = rx.try_recv() {
            rt.handle_knob_event(ev, &settings_for_timer);
        }
        rt.check_timeout();
    });

    let loop_result =
        slint::run_event_loop_until_quit().map_err(|err| KnobError::UiRun(err.to_string()));

    // Encerramento gracioso: fecha janelas, encerra listener e faz `ungrab()` dos dispositivos.
    running.store(false, Ordering::SeqCst);
    let _ = settings.hide();
    let _ = runtime.borrow().overlay.hide();
    let _ = input_thread.join();
    if let Some(handle) = hyprland_thread {
        let _ = handle.join();
    }
    info!("Janelas fechadas e dispositivos liberados (ungrab). Atalhos originais restaurados.");

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
