use crate::config::{Config, ProfileConfig, GLOBAL_PROFILE_NAME};
#[cfg(target_os = "linux")]
use log::warn;
use log::{debug, info};
#[cfg(any(target_os = "linux", test))]
use serde::Deserialize;
#[cfg(target_os = "linux")]
use std::env;
#[cfg(target_os = "linux")]
use std::io::{self, BufRead, BufReader, Read, Write};
#[cfg(target_os = "linux")]
use std::os::unix::net::UnixStream;
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Modo de seleção de perfil ativo.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum ProfileMode {
    /// Segue automaticamente a janela em foco no compositor.
    #[default]
    Auto,
    /// Mantém um perfil fixo independente de mudanças de janela até voltar para `Auto`.
    Manual(String),
}

/// Informações contextuais ricas da janela ativa no compositor.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ActiveWindowInfo {
    pub class: String,
    pub title: String,
    pub fullscreen: bool,
}

/// Eventos relevantes recebidos do `socket2` do Hyprland.
#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HyprlandEvent {
    ActiveWindow(Option<(String, String)>),
    Fullscreen(bool),
}

/// Estado compartilhado da janela atualmente em foco no compositor e do modo de perfil.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ContextState {
    pub active_window: Option<ActiveWindowInfo>,
    /// Última janela ativa que não pertence ao próprio KnobContext (usada em "Capturar janela").
    pub last_non_knob_window: Option<ActiveWindowInfo>,
    pub mode: ProfileMode,
}

impl ContextState {
    #[cfg(test)]
    pub fn new() -> Self {
        Self::with_mode(ProfileMode::Auto)
    }

    pub fn with_mode(mode: ProfileMode) -> Self {
        Self {
            active_window: None,
            last_non_knob_window: None,
            mode,
        }
    }

    pub fn set_mode(&mut self, mode: ProfileMode) {
        self.mode = mode;
    }

    pub fn mode(&self) -> &ProfileMode {
        &self.mode
    }

    #[cfg(test)]
    pub fn set_active_window(&mut self, window_class: Option<String>) {
        match window_class {
            Some(cls) => self.set_active_window_info(Some(ActiveWindowInfo {
                class: cls,
                title: String::new(),
                fullscreen: false,
            })),
            None => self.set_active_window_info(None),
        }
    }

    pub fn set_active_window_info(&mut self, info: Option<ActiveWindowInfo>) {
        let normalized = info.and_then(|mut w| {
            w.class = w.class.trim().to_string();
            w.title = w.title.trim().to_string();
            if w.class.is_empty() && w.title.is_empty() {
                None
            } else {
                Some(w)
            }
        });

        if let Some(ref w) = normalized {
            if !is_knobcontext_window(w) {
                self.last_non_knob_window = Some(w.clone());
            }
        }
        self.active_window = normalized;
    }

    #[cfg(target_os = "linux")]
    pub fn set_fullscreen(&mut self, fullscreen: bool) {
        if let Some(ref mut w) = self.active_window {
            w.fullscreen = fullscreen;
            if !is_knobcontext_window(w) {
                self.last_non_knob_window = Some(w.clone());
            }
        }
    }

    pub fn last_captured_window(&self) -> Option<&ActiveWindowInfo> {
        self.last_non_knob_window
            .as_ref()
            .or(self.active_window.as_ref())
    }

    #[cfg(test)]
    pub fn active_window(&self) -> Option<&str> {
        self.active_window.as_ref().map(|w| w.class.as_str())
    }

    /// Resolve qual perfil corresponderia à janela ativa atual (sempre contextual, fallback `"global"`).
    ///
    /// A seleção usa `(profile.priority, specificity_score)`:
    /// - Regras com maior `priority` vencem.
    /// - Em caso de empate, regras mais específicas vencem (ex.: `class + title` vence só `class`).
    pub fn contextual_profile_for_window(&self, config: &Config) -> String {
        let Some(active) = self.active_window.as_ref() else {
            return GLOBAL_PROFILE_NAME.to_string();
        };

        let mut best: Option<(i32, u8, &str)> = None;
        for profile in &config.profiles {
            if let Some(specificity) = profile_match_specificity(profile, active) {
                let candidate = (profile.priority, specificity, profile.name.as_str());
                let is_better = match best {
                    None => true,
                    Some((best_prio, best_spec, _)) => {
                        candidate.0 > best_prio
                            || (candidate.0 == best_prio && candidate.1 > best_spec)
                    }
                };
                if is_better {
                    best = Some(candidate);
                }
            }
        }

        best.map(|(_, _, name)| name.to_string())
            .unwrap_or_else(|| GLOBAL_PROFILE_NAME.to_string())
    }

    /// Resolve o nome do perfil efetivo respeitando `ProfileMode`:
    /// - Em `Auto`: consulta o contexto da janela ativa (fallback `"global"`).
    /// - Em `Manual(p)`: retorna `p` independente da janela ativa.
    pub fn profile_for_window(&self, config: &Config) -> String {
        match &self.mode {
            ProfileMode::Auto => self.contextual_profile_for_window(config),
            ProfileMode::Manual(profile_name) => profile_name.clone(),
        }
    }
}

fn is_knobcontext_window(info: &ActiveWindowInfo) -> bool {
    let class_lower = info.class.to_ascii_lowercase();
    let title_lower = info.title.to_ascii_lowercase();
    class_lower.contains("knobcontext") || title_lower.starts_with("knobcontext")
}

/// Avalia se um perfil casa com a janela ativa, retornando sua pontuação de especificidade.
/// Retorna `None` se o perfil não possui critérios contextuais ou se algum critério falhar.
pub fn profile_match_specificity(profile: &ProfileConfig, active: &ActiveWindowInfo) -> Option<u8> {
    if !profile.has_context_rule() {
        return None;
    }

    let mut score = 0u8;

    if let Some(raw_classes) = profile.window_class.as_deref() {
        let trimmed = raw_classes.trim();
        if !trimmed.is_empty() {
            if !matches_class_patterns(trimmed, &active.class) {
                return None;
            }
            score = score.saturating_add(1);
        }
    }

    if let Some(raw_titles) = profile.window_title.as_deref() {
        let trimmed = raw_titles.trim();
        if !trimmed.is_empty() {
            if !matches_title_patterns(trimmed, &active.title) {
                return None;
            }
            score = score.saturating_add(2);
        }
    }

    if let Some(required_fullscreen) = profile.fullscreen {
        if active.fullscreen != required_fullscreen {
            return None;
        }
        score = score.saturating_add(1);
    }

    Some(score)
}

fn matches_class_patterns(pattern_list: &str, active_class: &str) -> bool {
    let actual = active_class.trim().to_ascii_lowercase();
    if actual.is_empty() {
        return false;
    }
    pattern_list
        .split(['|', ','])
        .map(|tok| tok.trim().to_ascii_lowercase())
        .filter(|tok| !tok.is_empty())
        .any(|pat| wildcard_match(&pat, &actual))
}

fn matches_title_patterns(pattern_list: &str, active_title: &str) -> bool {
    let actual = active_title.to_ascii_lowercase();
    if actual.is_empty() {
        return false;
    }
    pattern_list
        .split('|')
        .map(|tok| tok.trim().to_ascii_lowercase())
        .filter(|tok| !tok.is_empty())
        .any(|needle| actual.contains(&needle))
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    if !pattern.contains('*') {
        return pattern == text;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut rest = text;
    let starts_with_wild = pattern.starts_with('*');
    let ends_with_wild = pattern.ends_with('*');

    for (idx, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if idx == 0 && !starts_with_wild {
            let Some(after) = rest.strip_prefix(part) else {
                return false;
            };
            rest = after;
        } else if idx == parts.len() - 1 && !ends_with_wild {
            return rest.ends_with(part);
        } else if let Some(pos) = rest.find(part) {
            rest = &rest[pos + part.len()..];
        } else {
            return false;
        }
    }
    true
}

/// Faz o parsing de uma linha do `socket2` do Hyprland (`EVENT>>DATA`).
#[cfg(any(target_os = "linux", test))]
pub fn parse_hyprland_event(line: &str) -> Option<HyprlandEvent> {
    let clean = line.trim_end_matches(['\r', '\n']);
    if let Some(payload) = clean.strip_prefix("activewindow>>") {
        let (class_raw, title_raw) = payload.split_once(',').unwrap_or((payload, ""));
        let window_class = class_raw.trim();
        let window_title = title_raw.trim();
        if window_class.is_empty() && window_title.is_empty() {
            return Some(HyprlandEvent::ActiveWindow(None));
        }
        return Some(HyprlandEvent::ActiveWindow(Some((
            window_class.to_string(),
            window_title.to_string(),
        ))));
    }

    if let Some(payload) = clean.strip_prefix("fullscreen>>") {
        let flag = payload.trim() == "1";
        return Some(HyprlandEvent::Fullscreen(flag));
    }

    None
}

#[cfg(any(target_os = "linux", test))]
#[derive(Deserialize)]
struct HyprActiveWindowJson {
    #[serde(default)]
    class: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    fullscreen: i32,
}

#[cfg(any(target_os = "linux", test))]
fn parse_activewindow_json_response(response: &str) -> Option<ActiveWindowInfo> {
    let parsed: HyprActiveWindowJson = serde_json::from_str(response).ok()?;
    let class = parsed.class.trim().to_string();
    let title = parsed.title.trim().to_string();
    if class.is_empty() && title.is_empty() {
        return None;
    }
    Some(ActiveWindowInfo {
        class,
        title,
        fullscreen: parsed.fullscreen != 0,
    })
}

#[cfg(target_os = "linux")]
pub fn hyprland_socket_paths() -> Option<(PathBuf, PathBuf)> {
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

#[cfg(target_os = "linux")]
fn query_initial_active_window(socket1_path: &Path) -> Option<ActiveWindowInfo> {
    let mut stream = UnixStream::connect(socket1_path).ok()?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
    stream.write_all(b"j/activewindow").ok()?;
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    parse_activewindow_json_response(&response)
}

/// Inicia uma thread dedicada escutando eventos `activewindow` e `fullscreen` do Hyprland (`socket2`).
///
/// Se `HYPRLAND_INSTANCE_SIGNATURE` não estiver presente, loga um único warning e
/// retorna `None` (modo global).
#[cfg(target_os = "linux")]
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

    if let Some(initial) = query_initial_active_window(&socket1_path) {
        if let Ok(mut guard) = state.write() {
            debug!(
                "Janela ativa inicial no Hyprland: class='{}', title='{}', fullscreen={}",
                initial.class, initial.title, initial.fullscreen
            );
            guard.set_active_window_info(Some(initial));
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
                            Ok(_) => match parse_hyprland_event(&line) {
                                Some(HyprlandEvent::ActiveWindow(Some((class, title)))) => {
                                    // Consulta o estado fullscreen real da nova janela (rápido via socket1)
                                    let fullscreen = query_initial_active_window(&socket1_path)
                                        .filter(|w| w.class.eq_ignore_ascii_case(&class))
                                        .map(|w| w.fullscreen)
                                        .unwrap_or(false);
                                    debug!(
                                        "Hyprland activewindow -> class='{class}', title='{title}', fullscreen={fullscreen}"
                                    );
                                    if let Ok(mut guard) = state.write() {
                                        guard.set_active_window_info(Some(ActiveWindowInfo {
                                            class,
                                            title,
                                            fullscreen,
                                        }));
                                    }
                                }
                                Some(HyprlandEvent::ActiveWindow(None)) => {
                                    debug!("Hyprland activewindow -> None");
                                    if let Ok(mut guard) = state.write() {
                                        guard.set_active_window_info(None);
                                    }
                                }
                                Some(HyprlandEvent::Fullscreen(fs)) => {
                                    debug!("Hyprland fullscreen -> {fs}");
                                    if let Ok(mut guard) = state.write() {
                                        guard.set_fullscreen(fs);
                                    }
                                }
                                None => {}
                            },
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

#[cfg(target_os = "windows")]
fn query_windows_active_window() -> Option<ActiveWindowInfo> {
    use std::path::Path;
    use windows_sys::Win32::Foundation::{CloseHandle, MAX_PATH, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
        GetWindowThreadProcessId,
    };

    // SAFETY: chamadas Win32 padrão com buffers alocados e handles fechados corretamente.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return None;
        }

        let len = GetWindowTextLengthW(hwnd);
        let title = if len > 0 {
            let mut buf = vec![0u16; (len as usize) + 1];
            let read = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
            if read > 0 {
                String::from_utf16_lossy(&buf[..read as usize])
                    .trim()
                    .to_string()
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        let class = if pid != 0 {
            let proc_handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if !proc_handle.is_null() {
                let mut path_buf = vec![0u16; (MAX_PATH as usize) * 4];
                let mut size = path_buf.len() as u32;
                let ok =
                    QueryFullProcessImageNameW(proc_handle, 0, path_buf.as_mut_ptr(), &mut size);
                CloseHandle(proc_handle);
                if ok != 0 && size > 0 {
                    let full = String::from_utf16_lossy(&path_buf[..size as usize]);
                    Path::new(&full)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_ascii_lowercase()
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        if class.is_empty() && title.is_empty() {
            return None;
        }

        let mut fullscreen = false;
        let is_shell_desktop = matches!(
            class.as_str(),
            "explorer" | "searchhost" | "shellexperiencehost"
        ) && title.is_empty();
        if !is_shell_desktop {
            let mut rect: RECT = std::mem::zeroed();
            if GetWindowRect(hwnd, &mut rect) != 0 {
                let hmon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
                let mut mi: MONITORINFO = std::mem::zeroed();
                mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
                if GetMonitorInfoW(hmon, &mut mi) != 0 {
                    let m = mi.rcMonitor;
                    fullscreen = rect.left <= m.left
                        && rect.top <= m.top
                        && rect.right >= m.right
                        && rect.bottom >= m.bottom;
                }
            }
        }

        Some(ActiveWindowInfo {
            class,
            title,
            fullscreen,
        })
    }
}

/// Inicia uma thread dedicada monitorando a janela em primeiro plano no Windows (Win32).
#[cfg(target_os = "windows")]
pub fn spawn_hyprland_listener(
    state: Arc<RwLock<ContextState>>,
    running: Arc<AtomicBool>,
) -> Option<JoinHandle<()>> {
    info!("Listener contextual Win32 (janela em primeiro plano) iniciado.");
    let handle = thread::spawn(move || {
        let mut last: Option<ActiveWindowInfo> = None;
        while running.load(Ordering::SeqCst) {
            let current = query_windows_active_window();
            if current != last {
                if let Some(ref info_win) = current {
                    debug!(
                        "Win32 activewindow -> class='{}', title='{}', fullscreen={}",
                        info_win.class, info_win.title, info_win.fullscreen
                    );
                }
                if let Ok(mut guard) = state.write() {
                    guard.set_active_window_info(current.clone());
                }
                last = current;
            }
            thread::sleep(Duration::from_millis(200));
        }
    });
    Some(handle)
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn spawn_hyprland_listener(
    _state: Arc<RwLock<ContextState>>,
    _running: Arc<AtomicBool>,
) -> Option<JoinHandle<()>> {
    None
}

#[cfg(test)]
mod tests {
    use super::{
        parse_activewindow_json_response, parse_hyprland_event, ActiveWindowInfo, ContextState,
        HyprlandEvent, ProfileMode,
    };
    use crate::config::{ActionConfig, Config, ProfileConfig, GLOBAL_PROFILE_NAME};

    #[test]
    fn manual_mode_overrides_window_context_and_auto_restores_it() {
        let cfg = Config::default();
        let mut ctx = ContextState::new();
        assert_eq!(*ctx.mode(), ProfileMode::Auto);

        // Em Auto: segue Firefox
        ctx.set_active_window(Some("firefox".to_string()));
        assert_eq!(ctx.profile_for_window(&cfg), "firefox");

        // Em Manual("global"): ignora Firefox e GIMP
        ctx.set_mode(ProfileMode::Manual("global".to_string()));
        assert_eq!(ctx.profile_for_window(&cfg), "global");
        ctx.set_active_window(Some("gimp".to_string()));
        assert_eq!(ctx.profile_for_window(&cfg), "global");
        assert_eq!(ctx.contextual_profile_for_window(&cfg), "gimp");

        // Voltar para Auto: imediatamente segue GIMP
        ctx.set_mode(ProfileMode::Auto);
        assert_eq!(ctx.profile_for_window(&cfg), "gimp");
    }

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
    fn matches_multi_class_title_fullscreen_and_priority() {
        let mut cfg = Config::default();
        cfg.profiles.push(ProfileConfig {
            name: "youtube".to_string(),
            window_class: Some("firefox|zen|chromium".to_string()),
            window_title: Some("YouTube|Twitch".to_string()),
            fullscreen: None,
            priority: 10,
            inherits: Some("global".to_string()),
            knob_cw: ActionConfig::Shortcut {
                keys: vec!["Right".into()],
            },
            knob_ccw: ActionConfig::Shortcut {
                keys: vec!["Left".into()],
            },
            knob_press: None,
            key1: ActionConfig::None,
            key2: ActionConfig::None,
            key3: ActionConfig::None,
            hub_modules: None,
        });
        cfg.profiles.push(ProfileConfig {
            name: "games_fs".to_string(),
            window_class: None,
            window_title: None,
            fullscreen: Some(true),
            priority: 5,
            inherits: None,
            knob_cw: ActionConfig::VolumeUp,
            knob_ccw: ActionConfig::VolumeDown,
            knob_press: None,
            key1: ActionConfig::None,
            key2: ActionConfig::None,
            key3: ActionConfig::None,
            hub_modules: None,
        });

        let mut ctx = ContextState::new();
        // Firefox em aba normal -> perfil firefox
        ctx.set_active_window_info(Some(ActiveWindowInfo {
            class: "firefox".to_string(),
            title: "Rust Docs — Mozilla Firefox".to_string(),
            fullscreen: false,
        }));
        assert_eq!(ctx.profile_for_window(&cfg), "firefox");

        // Firefox no YouTube -> regra mais específica + maior prioridade vence!
        ctx.set_active_window_info(Some(ActiveWindowInfo {
            class: "firefox".to_string(),
            title: "Lo-fi Hip Hop - YouTube — Mozilla Firefox".to_string(),
            fullscreen: false,
        }));
        assert_eq!(ctx.profile_for_window(&cfg), "youtube");

        // Outro app em fullscreen -> regra games_fs
        ctx.set_active_window_info(Some(ActiveWindowInfo {
            class: "steam_app_1234".to_string(),
            title: "Game".to_string(),
            fullscreen: true,
        }));
        assert_eq!(ctx.profile_for_window(&cfg), "games_fs");

        // Última janela não-KnobContext é preservada quando KnobContext Settings abre
        ctx.set_active_window_info(Some(ActiveWindowInfo {
            class: "knobcontext".to_string(),
            title: "KnobContext Settings".to_string(),
            fullscreen: false,
        }));
        assert_eq!(
            ctx.last_captured_window().map(|w| w.class.as_str()),
            Some("steam_app_1234")
        );
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
    fn parses_hyprland_socket2_activewindow_and_fullscreen_lines() {
        assert_eq!(
            parse_hyprland_event("activewindow>>firefox,Rust, Docs — Mozilla Firefox\n"),
            Some(HyprlandEvent::ActiveWindow(Some((
                "firefox".to_string(),
                "Rust, Docs — Mozilla Firefox".to_string()
            ))))
        );
        assert_eq!(
            parse_hyprland_event("activewindow>>,\n"),
            Some(HyprlandEvent::ActiveWindow(None))
        );
        assert_eq!(
            parse_hyprland_event("fullscreen>>1\n"),
            Some(HyprlandEvent::Fullscreen(true))
        );
        assert_eq!(
            parse_hyprland_event("fullscreen>>0\n"),
            Some(HyprlandEvent::Fullscreen(false))
        );
        assert_eq!(parse_hyprland_event("workspace>>2\n"), None);
    }

    #[test]
    fn parses_socket1_activewindow_json_query() {
        let sample = r#"{"class":"firefox","title":"YouTube — Mozilla Firefox","fullscreen":1}"#;
        assert_eq!(
            parse_activewindow_json_response(sample),
            Some(ActiveWindowInfo {
                class: "firefox".to_string(),
                title: "YouTube — Mozilla Firefox".to_string(),
                fullscreen: true,
            })
        );
        assert_eq!(parse_activewindow_json_response("{}"), None);
    }
}
