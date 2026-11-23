mod action;
mod config;
mod error;
mod input;

use crate::action::{ActionDispatcher, DispatchOutcome, EnigoExecutor};
use crate::config::Config;
use crate::error::KnobError;
use crate::input::KnobInput;
use env_logger::Env;
use log::{debug, error, info, warn};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn run() -> Result<(), KnobError> {
    let (config, loaded_path) = Config::load_default_location()?;
    match loaded_path {
        Some(path) => info!("Configuração carregada de {}", path.display()),
        None => {
            info!("Nenhum arquivo de configuração encontrado; usando configuração padrão embutida.")
        }
    }

    let active_idx = config.active_profile_index();
    let executor = EnigoExecutor::new()?;
    let mut dispatcher = ActionDispatcher::new(config.profiles.clone(), active_idx, executor);

    info!(
        "Capturando dispositivos: kbd='{}', media='{}'",
        config.device.kbd_path.display(),
        config.device.media_path.display()
    );
    let mut input = KnobInput::open(&config.device)?;
    info!(
        "KnobContext ativo (perfil: '{}'). Pressione Ctrl+C para encerrar e liberar o dispositivo.",
        dispatcher.active_profile_name()
    );

    let running = Arc::new(AtomicBool::new(true));
    let running_flag = Arc::clone(&running);
    if let Err(err) = ctrlc::set_handler(move || {
        running_flag.store(false, Ordering::SeqCst);
    }) {
        warn!("Não foi possível registrar handler de sinal Ctrl+C: {err}");
    }

    while running.load(Ordering::SeqCst) {
        let events = input.poll_events(100)?;
        for event in events {
            debug!("Evento capturado: {event:?}");
            match dispatcher.dispatch(event) {
                Ok(DispatchOutcome::Executed(action)) => {
                    info!(
                        "[{}] {event:?} -> {action:?}",
                        dispatcher.active_profile_name()
                    );
                }
                Ok(DispatchOutcome::ProfileSwitched(new_profile)) => {
                    info!("Perfil alterado para '{new_profile}'");
                }
                Ok(DispatchOutcome::Ignored) => {
                    debug!("[{}] {event:?} ignorado", dispatcher.active_profile_name());
                }
                Err(err) => {
                    warn!("Falha ao executar ação para {event:?}: {err}");
                }
            }
        }
    }

    drop(input);
    info!("Dispositivos liberados (ungrab). Atalhos originais restaurados.");
    Ok(())
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

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
