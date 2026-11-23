use crate::config::{Config, GLOBAL_PROFILE_NAME};
use log::{debug, info, warn};
use std::env;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Estado compartilhado da janela atualmente em foco no compositor.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ContextState {
    pub active_window: Option<String>,
}

impl ContextState {
    pub fn new() -> Self {
        Self {
            active_window: None,
        }
    }

    pub fn set_active_window(&mut self, window_class: Option<String>) {
        self.active_window = window_class.and_then(|s| {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
    }

    #[cfg(test)]
    pub fn active_window(&self) -> Option<&str> {
        self.active_window.as_deref()
    }

    /// Resolve o nome do perfil correspondente à janela ativa, com fallback para `"global"`.
    pub fn profile_for_window(&self, config: &Config) -> String {
        let Some(active_class) = self.active_window.as_deref() else {
            return GLOBAL_PROFILE_NAME.to_string();
        };

        for profile in &config.profiles {
            if let Some(target_class) = profile.window_class.as_deref() {
                let target = target_class.trim();
                if !target.is_empty() && target.eq_ignore_ascii_case(active_class) {
                    return profile.name.clone();
                }
            }
        }

        GLOBAL_PROFILE_NAME.to_string()
    }
}

/// Faz o parsing de uma linha do `socket2` do Hyprland (`EVENT>>DATA`).
///
/// - Retorna `Some(Some(class))` quando `activewindow` informa uma janela ativa.
/// - Retorna `Some(None)` quando `activewindow` informa que nenhuma janela está ativa (`activewindow>>,`).
/// - Retorna `None` para outros eventos irrelevantes.
pub fn parse_hyprland_event(line: &str) -> Option<Option<String>> {
    let payload = line
        .trim_end_matches(['\r', '\n'])
        .strip_prefix("activewindow>>")?;
    let window_class = payload.split(',').next().unwrap_or("").trim();
    if window_class.is_empty() {
        Some(None)
    } else {
        Some(Some(window_class.to_string()))
    }
}

fn parse_activewindow_query_response(response: &str) -> Option<String> {
    for line in response.lines() {
        let trimmed = line.trim();
        if let Some(class) = trimmed.strip_prefix("class:") {
            let value = class.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn hyprland_socket_paths() -> Option<(PathBuf, PathBuf)> {
    let sig = env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .ok()
        .filter(|s| !s.trim().is_empty())?;
    let base = env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));

    let dir = base.join("hypr").join(sig);
    Some((dir.join(".socket.sock"), dir.join(".socket2.sock")))
}

fn query_initial_active_window(socket1_path: &Path) -> Option<String> {
    let mut stream = UnixStream::connect(socket1_path).ok()?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
    stream.write_all(b"activewindow").ok()?;
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    parse_activewindow_query_response(&response)
}

/// Inicia uma thread dedicada escutando eventos `activewindow` do Hyprland (`socket2`).
///
/// Se `HYPRLAND_INSTANCE_SIGNATURE` não estiver presente, loga um único warning e
/// retorna `None` (modo global).
pub fn spawn_hyprland_listener(
    state: Arc<RwLock<ContextState>>,
    running: Arc<AtomicBool>,
) -> Option<JoinHandle<()>> {
    let Some((socket1_path, socket2_path)) = hyprland_socket_paths() else {
        warn!(
            "HYPRLAND_INSTANCE_SIGNATURE ausente; detecção contextual desativada (usando perfil '{}').",
            GLOBAL_PROFILE_NAME
        );
        return None;
    };

    if let Some(initial_class) = query_initial_active_window(&socket1_path) {
        if let Ok(mut guard) = state.write() {
            debug!("Janela ativa inicial no Hyprland: '{initial_class}'");
            guard.set_active_window(Some(initial_class));
        }
    }

    let handle = thread::spawn(move || {
        let mut logged_disconnect = false;

        while running.load(Ordering::SeqCst) {
            match UnixStream::connect(&socket2_path) {
                Ok(stream) => {
                    if logged_disconnect {
                        info!("Reconectado ao socket2 do Hyprland.");
                        logged_disconnect = false;
                    } else {
                        info!(
                            "Listener contextual do Hyprland conectado em {}",
                            socket2_path.display()
                        );
                    }

                    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();

                    while running.load(Ordering::SeqCst) {
                        line.clear();
                        match reader.read_line(&mut line) {
                            Ok(0) => {
                                // EOF: Hyprland reiniciou ou fechou o socket.
                                break;
                            }
                            Ok(_) => {
                                if let Some(maybe_class) = parse_hyprland_event(&line) {
                                    debug!("Hyprland activewindow -> {maybe_class:?}");
                                    if let Ok(mut guard) = state.write() {
                                        guard.set_active_window(maybe_class);
                                    }
                                }
                            }
                            Err(err)
                                if err.kind() == io::ErrorKind::WouldBlock
                                    || err.kind() == io::ErrorKind::TimedOut
                                    || err.kind() == io::ErrorKind::Interrupted =>
                            {
                                continue;
                            }
                            Err(err) => {
                                warn!("Erro ao ler socket2 do Hyprland: {err}");
                                break;
                            }
                        }
                    }
                }
                Err(err) => {
                    if !logged_disconnect {
                        warn!(
                            "Não foi possível conectar ao socket2 do Hyprland ({}): {err}. Mantendo perfil '{}'.",
                            socket2_path.display(),
                            GLOBAL_PROFILE_NAME
                        );
                        logged_disconnect = true;
                    }
                    // Espera curta em fatias para respeitar `running == false` rapidamente.
                    for _ in 0..10 {
                        if !running.load(Ordering::SeqCst) {
                            break;
                        }
                        thread::sleep(Duration::from_millis(100));
                    }
                }
            }
        }
    });

    Some(handle)
}

#[cfg(test)]
mod tests {
    use super::{parse_activewindow_query_response, parse_hyprland_event, ContextState};
    use crate::config::{Config, GLOBAL_PROFILE_NAME};

    #[test]
    fn maps_known_windows_to_matching_profiles() {
        let cfg = Config::default();
        let mut ctx = ContextState::new();

        ctx.set_active_window(Some("firefox".to_string()));
        assert_eq!(ctx.active_window(), Some("firefox"));
        assert_eq!(ctx.profile_for_window(&cfg), "firefox");

        // Case-insensitive match
        ctx.set_active_window(Some("GIMP".to_string()));
        assert_eq!(ctx.profile_for_window(&cfg), "gimp");
    }

    #[test]
    fn falls_back_to_global_for_unmapped_none_or_empty_config() {
        let cfg = Config::default();
        let mut ctx = ContextState::new();

        // Nenhuma janela ativa
        assert_eq!(ctx.profile_for_window(&cfg), GLOBAL_PROFILE_NAME);

        // Janela sem perfil configurado
        ctx.set_active_window(Some("alacritty".to_string()));
        assert_eq!(ctx.profile_for_window(&cfg), GLOBAL_PROFILE_NAME);

        // Config sem perfis
        let empty_cfg = Config {
            profiles: Vec::new(),
            ..Config::default()
        };
        ctx.set_active_window(Some("firefox".to_string()));
        assert_eq!(ctx.profile_for_window(&empty_cfg), GLOBAL_PROFILE_NAME);
    }

    #[test]
    fn parses_hyprland_socket2_activewindow_lines() {
        assert_eq!(
            parse_hyprland_event("activewindow>>firefox,Rust Docs — Mozilla Firefox\n"),
            Some(Some("firefox".to_string()))
        );
        assert_eq!(parse_hyprland_event("activewindow>>,\n"), Some(None));
        assert_eq!(parse_hyprland_event("workspace>>2\n"), None);
    }

    #[test]
    fn parses_socket1_activewindow_query() {
        let sample = "Window 55a1 -> Firefox:\n\tclass: firefox\n\ttitle: Home\n";
        assert_eq!(
            parse_activewindow_query_response(sample),
            Some("firefox".to_string())
        );
        assert_eq!(parse_activewindow_query_response("Invalid\n"), None);
    }
}
