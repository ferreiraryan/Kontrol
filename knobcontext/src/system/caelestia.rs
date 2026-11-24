use crate::error::KnobError;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WallpaperItem {
    pub path: Option<String>,
    pub is_random: bool,
    pub title: String,
    pub category: String,
    pub is_current: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WallpapersSnapshot {
    pub items: Vec<WallpaperItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaelestiaToggleKind {
    Dnd,
    GameMode,
    IdleInhibitor,
    CycleAudioOutput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaelestiaToggleItem {
    pub kind: CaelestiaToggleKind,
    pub title: String,
    pub subtitle: String,
    pub enabled: Option<bool>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CaelestiaTogglesSnapshot {
    pub items: Vec<CaelestiaToggleItem>,
}

fn run_qs_ipc(args: &[&str]) -> Result<String, KnobError> {
    let mut full = vec!["-c", "caelestia", "ipc", "call"];
    full.extend_from_slice(args);
    let output = Command::new("qs").args(&full).output().map_err(|err| {
        KnobError::InputSimulation(format!("falha ao chamar Caelestia IPC: {err}"))
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(KnobError::InputSimulation(format!(
            "Caelestia IPC `{}` falhou: {}",
            args.join(" "),
            stderr.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn format_wallpaper_label(path_str: &str) -> (String, String) {
    let path = Path::new(path_str);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(path_str)
        .replace(['_', '-'], " ");
    let title = stem.trim().to_string();
    let category = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("Wallpaper")
        .to_string();
    (
        if title.is_empty() {
            path_str.to_string()
        } else {
            title
        },
        category,
    )
}

pub fn parse_wallpapers_list(list_raw: &str, current_raw: &str) -> WallpapersSnapshot {
    let current = current_raw.trim();
    let mut items = vec![WallpaperItem {
        path: None,
        is_random: true,
        title: "Wallpaper Aleatório".to_string(),
        category: "Caelestia (troca imagem + cores)".to_string(),
        is_current: false,
    }];

    for line in list_raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (title, category) = format_wallpaper_label(trimmed);
        items.push(WallpaperItem {
            path: Some(trimmed.to_string()),
            is_random: false,
            title,
            category,
            is_current: !current.is_empty() && trimmed == current,
        });
    }

    WallpapersSnapshot { items }
}

pub fn parse_ipc_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "on" => Some(true),
        "false" | "0" | "off" => Some(false),
        _ => None,
    }
}

pub fn fetch_wallpapers() -> Result<WallpapersSnapshot, KnobError> {
    let current = run_qs_ipc(&["wallpaper", "get"]).unwrap_or_default();
    let list = run_qs_ipc(&["wallpaper", "list"])?;
    Ok(parse_wallpapers_list(&list, &current))
}

pub fn set_wallpaper(path: &str) -> Result<(), KnobError> {
    let _ = run_qs_ipc(&["wallpaper", "set", path])?;
    Ok(())
}

pub fn set_random_wallpaper() -> Result<(), KnobError> {
    let status = Command::new("caelestia")
        .args(["wallpaper", "-r"])
        .status()
        .map_err(|err| {
            KnobError::InputSimulation(format!("falha ao executar `caelestia wallpaper -r`: {err}"))
        })?;
    if !status.success() {
        return Err(KnobError::InputSimulation(
            "`caelestia wallpaper -r` retornou erro".to_string(),
        ));
    }
    Ok(())
}

pub fn fetch_caelestia_toggles() -> Result<CaelestiaTogglesSnapshot, KnobError> {
    let dnd = run_qs_ipc(&["notifs", "isDndEnabled"])
        .ok()
        .and_then(|s| parse_ipc_bool(&s));
    let game = run_qs_ipc(&["gameMode", "isEnabled"])
        .ok()
        .and_then(|s| parse_ipc_bool(&s));
    let idle = run_qs_ipc(&["idleInhibitor", "isEnabled"])
        .ok()
        .and_then(|s| parse_ipc_bool(&s));

    Ok(CaelestiaTogglesSnapshot {
        items: vec![
            CaelestiaToggleItem {
                kind: CaelestiaToggleKind::Dnd,
                title: "Não Perturbe (DND)".to_string(),
                subtitle: "Silenciar notificações do Caelestia".to_string(),
                enabled: dnd,
            },
            CaelestiaToggleItem {
                kind: CaelestiaToggleKind::GameMode,
                title: "Game Mode".to_string(),
                subtitle: "Modo de desempenho / animações".to_string(),
                enabled: game,
            },
            CaelestiaToggleItem {
                kind: CaelestiaToggleKind::IdleInhibitor,
                title: "Inibir Idle".to_string(),
                subtitle: "Evitar bloqueio automático da tela".to_string(),
                enabled: idle,
            },
            CaelestiaToggleItem {
                kind: CaelestiaToggleKind::CycleAudioOutput,
                title: "Ciclar Saída de Áudio".to_string(),
                subtitle: "Alterna para a próxima saída no Caelestia".to_string(),
                enabled: None,
            },
        ],
    })
}

pub fn trigger_caelestia_toggle(kind: CaelestiaToggleKind) -> Result<(), KnobError> {
    match kind {
        CaelestiaToggleKind::Dnd => {
            let _ = run_qs_ipc(&["notifs", "toggleDnd"])?;
        }
        CaelestiaToggleKind::GameMode => {
            let _ = run_qs_ipc(&["gameMode", "toggle"])?;
        }
        CaelestiaToggleKind::IdleInhibitor => {
            let _ = run_qs_ipc(&["idleInhibitor", "toggle"])?;
        }
        CaelestiaToggleKind::CycleAudioOutput => {
            let _ = run_qs_ipc(&["audio", "cycleOutput"])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{format_wallpaper_label, parse_ipc_bool, parse_wallpapers_list};

    #[test]
    fn parses_wallpapers_list_and_marks_current() {
        let list = "/home/u/Pictures/Wallpapers/abstract/sun_and_ball.png\n/home/u/Pictures/Wallpapers/nature/forest-mist.jpg\n";
        let current = "/home/u/Pictures/Wallpapers/abstract/sun_and_ball.png\n";
        let snap = parse_wallpapers_list(list, current);

        assert_eq!(snap.items.len(), 3);
        assert!(snap.items[0].is_random);
        assert!(snap.items[1].is_current);
        assert_eq!(snap.items[1].title, "sun and ball");
        assert_eq!(snap.items[1].category, "abstract");
        assert!(!snap.items[2].is_current);
        assert_eq!(snap.items[2].title, "forest mist");
        assert_eq!(snap.items[2].category, "nature");
    }

    #[test]
    fn parses_ipc_booleans_and_formats_labels() {
        assert_eq!(parse_ipc_bool("true\n"), Some(true));
        assert_eq!(parse_ipc_bool("false"), Some(false));
        assert_eq!(parse_ipc_bool("unknown"), None);
        assert_eq!(
            format_wallpaper_label("/tmp/space/nebula_01.png"),
            ("nebula 01".to_string(), "space".to_string())
        );
    }
}
