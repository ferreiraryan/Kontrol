mod action;
mod action_presets;
mod actions_catalog;
mod config;
mod context;
mod error;
mod input;
mod overlay_hub;
mod platform;
mod presets_linux;
mod presets_windows;
mod providers;
mod scan;
mod system;
mod tray;
mod ui;

use crate::action::{ActionDispatcher, DispatchOutcome, EnigoExecutor};
use crate::config::Config;
use crate::context::{spawn_hyprland_listener, ContextState, ProfileMode};
use crate::error::KnobError;
use crate::input::{KnobEvent, KnobInput};
use crate::overlay_hub::{HubController, HubEffect};
use crate::providers::ProviderRegistry;
use crate::system::{SystemRequest, SystemResponse, SystemWorker};
use crate::tray::{TrayCommand, TrayController};
use crate::ui::hud_mod::{HudController, HUD_HEIGHT_PX, HUD_WIDTH_PX};
use crate::ui::settings_mod::SettingsController;
use crate::ui::{OverlayWindow, OVERLAY_HEIGHT_PX, OVERLAY_WIDTH_PX};
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
    providers: Arc<RwLock<ProviderRegistry>>,
    hub: HubController,
    dispatcher: ActionDispatcher<EnigoExecutor>,
    overlay: OverlayWindow,
    hud: HudController,
    system: SystemWorker,
    tray: TrayController,
}

impl AppRuntime {
    fn current_mode_and_contextual(&self) -> (ProfileMode, String) {
        if let Ok(ctx) = self.context.read() {
            (
                ctx.mode().clone(),
                ctx.contextual_profile_for_window(&self.config),
            )
        } else {
            (ProfileMode::Auto, self.config.active_profile.clone())
        }
    }

    fn sync_tray(&mut self) {
        let (mode, contextual) = self.current_mode_and_contextual();
        let profile_names: Vec<String> = self
            .config
            .profiles
            .iter()
            .map(|p| p.name.clone())
            .collect();
        self.tray
            .sync_if_changed(&mode, &contextual, &profile_names);
    }

    fn handle_tray_command(
        &mut self,
        cmd: TrayCommand,
        settings: &SettingsController,
        running: &Arc<AtomicBool>,
    ) {
        match cmd {
            TrayCommand::OpenSettings => self.open_settings(settings),
            TrayCommand::SetMode(new_mode) => {
                self.persist_mode(new_mode.clone());
                let resolved = self.sync_context();
                self.sync_tray();
                info!(
                    "Modo alterado via tray: {:?} (perfil: '{resolved}')",
                    new_mode
                );
            }
            TrayCommand::Quit => {
                info!("Encerramento solicitado pelo menu do tray.");
                running.store(false, Ordering::SeqCst);
                let _ = slint::quit_event_loop();
            }
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

        self.dispatcher.update_profiles(new_config.profiles.clone());
        if let Ok(mut reg) = self.providers.write() {
            *reg = ProviderRegistry::default_registry(&new_config.providers);
        }
        self.config = new_config;
        info!("Configuração salva e aplicada em tempo real.");
    }

    fn sync_context(&mut self) -> String {
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

    fn apply_hub_effects(&mut self, effects: Vec<HubEffect>) {
        for effect in effects {
            match effect {
                HubEffect::DispatchNormal(ev) => {
                    let profile_name = self.sync_context();
                    match self.dispatcher.dispatch(ev, &profile_name) {
                        Ok(DispatchOutcome::Executed(action)) => {
                            info!(
                                "[{}] {ev:?} -> {action:?}",
                                self.dispatcher.active_profile_name()
                            );
                            if matches!(ev, KnobEvent::RotateCw | KnobEvent::RotateCcw) {
                                match self.hud.show_for_action(
                                    &action,
                                    self.config.overlay.volume_step,
                                    self.config.overlay.max_volume_percent,
                                ) {
                                    Ok((just_opened, should_query_vol)) => {
                                        if just_opened {
                                            self.system.send(SystemRequest::PositionHud {
                                                width: HUD_WIDTH_PX,
                                                height: HUD_HEIGHT_PX,
                                                margin: self.config.overlay.bottom_margin_px,
                                            });
                                        }
                                        if should_query_vol {
                                            self.system.send(SystemRequest::FetchAudioMixer);
                                        }
                                        if platform::PlatformKind::current().is_windows()
                                            && HudController::action_supports_media_art(&action)
                                        {
                                            self.system.send(SystemRequest::FetchSmtcMedia);
                                        }
                                    }
                                    Err(err) => {
                                        warn!("Falha ao exibir Mini-HUD: {err}");
                                    }
                                }
                            }
                        }
                        Ok(DispatchOutcome::ProfileSwitched(new_profile)) => {
                            self.persist_mode(ProfileMode::Manual(new_profile.clone()));
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
                HubEffect::RenderOverlay(vm) => {
                    let _ = self.hud.hide();
                    match self.overlay.render(&vm) {
                        Ok(outcome) => {
                            if outcome.just_opened {
                                self.system.send(SystemRequest::PositionOverlay {
                                    width: OVERLAY_WIDTH_PX,
                                    height: OVERLAY_HEIGHT_PX,
                                    bottom_margin: self.config.overlay.bottom_margin_px,
                                });
                            }
                            for req in outcome.missing_thumbnails {
                                self.system.send(SystemRequest::LoadThumbnail(req));
                            }
                        }
                        Err(err) => {
                            warn!("Falha ao renderizar overlay: {err}");
                        }
                    }
                }
                HubEffect::CloseOverlay => {
                    if let Err(err) = self.overlay.hide() {
                        warn!("Falha ao fechar overlay: {err}");
                    }
                }
                HubEffect::SetProfileMode(new_mode) => {
                    self.persist_mode(new_mode.clone());
                    let resolved = self.sync_context();
                    info!(
                        "Modo alterado no Hub: {:?} (perfil: '{resolved}')",
                        new_mode
                    );
                }
                HubEffect::System(req) => {
                    self.system.send(req);
                }
                HubEffect::ExecuteAction(action) => {
                    if let Err(err) = self.dispatcher.execute_direct(&action) {
                        warn!("Falha ao executar ação do Hub ({action:?}): {err}");
                    }
                }
            }
        }
    }

    fn handle_knob_event(&mut self, event: KnobEvent, settings: &SettingsController) {
        if event == KnobEvent::OpenSettings {
            self.open_settings(settings);
            return;
        }

        let (mode, contextual) = self.current_mode_and_contextual();
        let effects =
            self.hub
                .handle_event(event, Instant::now(), &self.config, &mode, &contextual);
        self.apply_hub_effects(effects);
    }

    fn drain_system_responses(&mut self) {
        while let Some(resp) = self.system.try_recv() {
            if let SystemResponse::ThumbnailReady(decoded) = resp {
                if let Err(err) = self.overlay.on_thumbnail_ready(decoded) {
                    warn!("Falha ao atualizar thumbnail no overlay: {err}");
                }
                continue;
            }
            if let SystemResponse::AudioMixer(ref snap) = resp {
                if let Some(master) = snap.streams.iter().find(|s| s.is_master) {
                    let _ = self.hud.update_master_volume(
                        master.volume_percent,
                        master.muted,
                        self.config.overlay.max_volume_percent,
                    );
                }
            }
            if let SystemResponse::SmtcMedia(ref snap) = resp {
                if let Err(err) = self.hud.update_smtc_media(snap) {
                    warn!("Falha ao atualizar capa SMTC no Mini-HUD: {err}");
                }
            }
            let (mode, contextual) = self.current_mode_and_contextual();
            if let Some(effect) =
                self.hub
                    .apply_system_response(resp, &self.config, &mode, &contextual)
            {
                self.apply_hub_effects(vec![effect]);
            }
        }
    }

    fn open_settings(&mut self, settings: &SettingsController) {
        self.hub.set_settings_open(true);
        let _ = self.overlay.hide();
        let _ = self.hud.hide();
        if let Err(err) = settings.open(&self.config) {
            warn!("Falha ao abrir janela de configurações: {err}");
            self.hub.set_settings_open(false);
        } else {
            info!("Janela de configurações aberta (grab mantido ativo).");
        }
    }

    fn check_timeout(&mut self) {
        if let Err(err) = self.hud.check_timeout() {
            warn!("Falha ao fechar Mini-HUD no timeout: {err}");
        }
        if let Some(effect) = self.hub.check_timeout(Instant::now(), &self.config.overlay) {
            debug!("Overlay fechado automaticamente por inatividade.");
            self.apply_hub_effects(vec![effect]);
        }
    }
}

#[cfg(target_os = "linux")]
fn check_udev_rule_installed() {
    let rules_path = std::path::Path::new("/etc/udev/rules.d/99-knobcontext.rules");
    if !rules_path.exists() {
        warn!(
            "Regra udev não encontrada em {}. Rode install.sh do AppImage ou copie 99-knobcontext.rules para /etc/udev/rules.d/. Sem isso, o app pode não conseguir acessar o dispositivo.",
            rules_path.display()
        );
    }
}

#[cfg(not(target_os = "linux"))]
fn check_udev_rule_installed() {}

#[cfg(target_os = "windows")]
fn select_slint_renderer(renderer: crate::config::UiRendererKind) -> bool {
    match renderer {
        crate::config::UiRendererKind::Femtovg => {
            match slint::BackendSelector::new()
                .backend_name("winit".into())
                .renderer_name("femtovg".into())
                .select()
            {
                Ok(()) => {
                    info!("Slint renderer selecionado: femtovg (backdrop-blur nativo ativado).");
                    true
                }
                Err(err) => {
                    warn!(
                        "Renderer `femtovg` indisponível ({err}); aplicando fallback automático para `software`."
                    );
                    let _ = slint::BackendSelector::new()
                        .backend_name("winit".into())
                        .renderer_name("software".into())
                        .select();
                    false
                }
            }
        }
        crate::config::UiRendererKind::Software => {
            let _ = slint::BackendSelector::new()
                .backend_name("winit".into())
                .renderer_name("software".into())
                .select();
            info!("Slint renderer selecionado: software (glassmorphism simulado).");
            false
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn select_slint_renderer(_renderer: crate::config::UiRendererKind) -> bool {
    false
}

fn run() -> Result<(), KnobError> {
    if env::args().skip(1).any(|arg| arg == "--scan") {
        scan::run_scan_command().map_err(KnobError::InputSimulation)?;
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        // Garante COM em modo STA na thread principal antes de qualquer crate/UI (`winit::OleInitialize`).
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            );
        }
    }
    check_udev_rule_installed();
    let open_settings_on_start = env::args().skip(1).any(|arg| arg == "--settings");

    let (config, loaded_path) = Config::load_default_location()?;
    match loaded_path {
        Some(path) => info!("Configuração carregada de {}", path.display()),
        None => {
            info!("Nenhum arquivo de configuração encontrado; usando configuração padrão embutida.")
        }
    }

    let use_backdrop_blur = select_slint_renderer(config.ui.renderer);
    let reduced_motion = platform::prefers_reduced_motion();
    let effective_anims =
        platform::effective_animations_enabled(config.overlay.animations_enabled, reduced_motion);

    let active_idx = config.active_profile_index();
    let initial_mode = config.resolve_profile_mode();
    let providers = Arc::new(RwLock::new(ProviderRegistry::default_registry(
        &config.providers,
    )));

    let executor = EnigoExecutor::new(config.device.scroll_step, Arc::clone(&providers))?;
    let dispatcher = ActionDispatcher::new(config.profiles.clone(), active_idx, executor);
    let mut overlay = OverlayWindow::new();
    overlay.set_use_backdrop_blur(use_backdrop_blur);
    let mut hud = HudController::new();
    hud.set_render_options(use_backdrop_blur, effective_anims);
    let hub = HubController::new();
    let system = SystemWorker::spawn();
    system::hyprland::register_overlay_window_rule(config.overlay.bottom_margin_px);
    system::hyprland::register_hud_window_rule(config.overlay.bottom_margin_px);

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
        "KnobContext ativo (modo inicial: {:?}). Pressione o knob para o Hub, Key1+Key2 (ou --settings) para configurações, ou Ctrl+C para sair.",
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

    let initial_contextual = context
        .read()
        .map(|ctx| ctx.contextual_profile_for_window(&config))
        .unwrap_or_else(|_| config.active_profile.clone());
    let tray_profile_names: Vec<String> = config.profiles.iter().map(|p| p.name.clone()).collect();
    let (tray_tx, tray_rx) = mpsc::channel::<TrayCommand>();
    let tray = TrayController::spawn(
        initial_mode.clone(),
        initial_contextual,
        tray_profile_names,
        tray_tx,
    );

    let initial_config_for_settings = config.clone();
    let runtime = Rc::new(RefCell::new(AppRuntime {
        config,
        context,
        providers,
        hub,
        dispatcher,
        overlay,
        hud,
        system,
        tray,
    }));

    let runtime_on_saved = Rc::clone(&runtime);
    let runtime_on_closed = Rc::clone(&runtime);
    let runtime_on_release = Rc::clone(&runtime);
    let context_for_capture = Arc::clone(&runtime.borrow().context);
    let settings = Rc::new(SettingsController::new(
        initial_config_for_settings,
        move |saved_config| {
            runtime_on_saved
                .borrow_mut()
                .apply_saved_config(saved_config);
        },
        move || {
            runtime_on_closed.borrow_mut().hub.set_settings_open(false);
            debug!("Janela de configurações fechada; Hub reabilitado.");
        },
        move || {
            runtime_on_release
                .borrow_mut()
                .dispatcher
                .release_modifiers();
            debug!("Modificadores liberados preventivamente após captura/fechamento.");
        },
        move || {
            context_for_capture
                .read()
                .ok()
                .and_then(|ctx| ctx.last_captured_window().cloned())
        },
    )?);

    if open_settings_on_start {
        runtime.borrow_mut().open_settings(&settings);
    }

    let runtime_for_timer = Rc::clone(&runtime);
    let settings_for_timer = Rc::clone(&settings);
    let running_for_timer = Arc::clone(&running);
    let poll_timer = Timer::default();
    poll_timer.start(TimerMode::Repeated, Duration::from_millis(20), move || {
        let mut rt = runtime_for_timer.borrow_mut();
        while let Ok(ev) = rx.try_recv() {
            rt.handle_knob_event(ev, &settings_for_timer);
        }
        while let Ok(cmd) = tray_rx.try_recv() {
            rt.handle_tray_command(cmd, &settings_for_timer, &running_for_timer);
        }
        rt.drain_system_responses();
        rt.sync_tray();
        rt.check_timeout();
    });

    let loop_result =
        slint::run_event_loop_until_quit().map_err(|err| KnobError::UiRun(err.to_string()));

    // Encerramento gracioso: fecha janelas, encerra workers/tray e faz `ungrab()` dos dispositivos.
    running.store(false, Ordering::SeqCst);
    let _ = settings.hide();
    {
        let mut rt = runtime.borrow_mut();
        let _ = rt.overlay.hide();
        let _ = rt.hud.hide();
        rt.system.shutdown();
        rt.tray.shutdown();
    }
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
