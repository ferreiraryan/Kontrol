use crate::config::{
    ActionConfig, OverlayItemConfig, OverlayItemKind, ProfileConfig, GLOBAL_PROFILE_NAME,
};
use std::collections::BTreeMap;

fn shortcut(keys: &[&str]) -> ActionConfig {
    ActionConfig::Shortcut {
        keys: keys.iter().map(|k| (*k).to_string()).collect(),
    }
}

fn smtc(cmd: &str) -> ActionConfig {
    ActionConfig::Provider {
        provider_id: "smtc".to_string(),
        command_id: cmd.to_string(),
        args: BTreeMap::new(),
    }
}

/// Módulos padrão do Hub otimizados para o Windows (sem módulos exclusivos do Hyprland/Caelestia).
pub fn default_windows_hub_items() -> Vec<OverlayItemConfig> {
    vec![
        OverlayItemConfig {
            id: "profiles".to_string(),
            title: "Perfis".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::Profiles,
        },
        OverlayItemConfig {
            id: "media_control".to_string(),
            title: "Mídia".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::MediaControl,
        },
        OverlayItemConfig {
            id: "browser_tabs".to_string(),
            title: "Abas".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::BrowserTabs,
        },
        OverlayItemConfig {
            id: "zoom".to_string(),
            title: "Zoom".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::Zoom,
        },
        OverlayItemConfig {
            id: "scroll_page".to_string(),
            title: "Página".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::ScrollPage,
        },
    ]
}

/// Perfis padrão pré-configurados para o primeiro uso no Windows (baseados no scan do sistema).
///
/// Inclui: Navegadores (Firefox/Chrome/Edge), VSCode, Explorer, Krita, Adobe Photoshop,
/// Adobe Illustrator, DaVinci Resolve e Spotify (via provider SMTC).
///
/// Nota: perfis de jogos NÃO são incluídos por padrão porque jogos competitivos com
/// anti-cheat (CS/VAC, Valorant/Vanguard, etc.) bloqueiam simulação de teclado por design.
pub fn default_windows_profiles() -> Vec<ProfileConfig> {
    vec![
        ProfileConfig {
            name: GLOBAL_PROFILE_NAME.to_string(),
            window_class: None,
            window_title: None,
            fullscreen: None,
            priority: 0,
            inherits: None,
            knob_cw: ActionConfig::VolumeUp,
            knob_ccw: ActionConfig::VolumeDown,
            knob_press: None,
            key1: ActionConfig::MediaPrev,
            key2: ActionConfig::MediaNext,
            key3: ActionConfig::MediaPlayPause,
            hub_modules: None,
        },
        ProfileConfig {
            name: "navegadores".to_string(),
            window_class: Some("firefox|chrome|msedge".to_string()),
            window_title: None,
            fullscreen: None,
            priority: 0,
            inherits: Some(GLOBAL_PROFILE_NAME.to_string()),
            knob_cw: shortcut(&["Ctrl", "Tab"]),
            knob_ccw: shortcut(&["Ctrl", "Shift", "Tab"]),
            knob_press: None,
            key1: shortcut(&["Ctrl", "T"]),
            key2: shortcut(&["Ctrl", "W"]),
            key3: shortcut(&["Ctrl", "Shift", "T"]),
            hub_modules: None,
        },
        ProfileConfig {
            name: "vscode".to_string(),
            window_class: Some("code".to_string()),
            window_title: None,
            fullscreen: None,
            priority: 0,
            inherits: Some(GLOBAL_PROFILE_NAME.to_string()),
            knob_cw: shortcut(&["Ctrl", "Shift", "P"]),
            knob_ccw: shortcut(&["Ctrl", "P"]),
            knob_press: None,
            key1: shortcut(&["Ctrl", "`"]),
            key2: shortcut(&["Ctrl", "S"]),
            key3: ActionConfig::None,
            hub_modules: None,
        },
        ProfileConfig {
            name: "explorer".to_string(),
            window_class: Some("explorer".to_string()),
            window_title: None,
            fullscreen: None,
            priority: 0,
            inherits: Some(GLOBAL_PROFILE_NAME.to_string()),
            knob_cw: shortcut(&["Alt", "Right"]),
            knob_ccw: shortcut(&["Alt", "Left"]),
            knob_press: None,
            key1: shortcut(&["Ctrl", "C"]),
            key2: shortcut(&["Ctrl", "V"]),
            key3: ActionConfig::None,
            hub_modules: None,
        },
        ProfileConfig {
            name: "krita".to_string(),
            window_class: Some("krita".to_string()),
            window_title: None,
            fullscreen: None,
            priority: 0,
            inherits: Some(GLOBAL_PROFILE_NAME.to_string()),
            knob_cw: shortcut(&["]"]),
            knob_ccw: shortcut(&["["]),
            knob_press: None,
            key1: shortcut(&["Ctrl", "Z"]),
            key2: shortcut(&["Ctrl", "Shift", "Z"]),
            key3: shortcut(&["E"]),
            hub_modules: None,
        },
        ProfileConfig {
            name: "photoshop".to_string(),
            window_class: Some("photoshop".to_string()),
            window_title: None,
            fullscreen: None,
            priority: 0,
            inherits: Some(GLOBAL_PROFILE_NAME.to_string()),
            knob_cw: shortcut(&["]"]),
            knob_ccw: shortcut(&["["]),
            knob_press: None,
            key1: shortcut(&["Ctrl", "Z"]),
            key2: shortcut(&["B"]),
            key3: shortcut(&["E"]),
            hub_modules: None,
        },
        ProfileConfig {
            name: "illustrator".to_string(),
            window_class: Some("illustrator".to_string()),
            window_title: None,
            fullscreen: None,
            priority: 0,
            inherits: Some(GLOBAL_PROFILE_NAME.to_string()),
            knob_cw: shortcut(&["Ctrl", "Plus"]),
            knob_ccw: shortcut(&["Ctrl", "Minus"]),
            knob_press: None,
            key1: shortcut(&["Ctrl", "Z"]),
            key2: shortcut(&["V"]),
            key3: shortcut(&["P"]),
            hub_modules: None,
        },
        ProfileConfig {
            name: "resolve".to_string(),
            window_class: Some("resolve".to_string()),
            window_title: None,
            fullscreen: None,
            priority: 0,
            inherits: Some(GLOBAL_PROFILE_NAME.to_string()),
            knob_cw: shortcut(&["L"]),
            knob_ccw: shortcut(&["J"]),
            knob_press: None,
            key1: shortcut(&["K"]),
            key2: shortcut(&["I"]),
            key3: shortcut(&["O"]),
            hub_modules: None,
        },
        ProfileConfig {
            name: "spotify".to_string(),
            window_class: Some("spotify".to_string()),
            window_title: None,
            fullscreen: None,
            priority: 0,
            inherits: Some(GLOBAL_PROFILE_NAME.to_string()),
            knob_cw: ActionConfig::VolumeUp,
            knob_ccw: ActionConfig::VolumeDown,
            knob_press: None,
            key1: smtc("previous"),
            key2: smtc("next"),
            key3: smtc("play_pause"),
            hub_modules: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::default_windows_profiles;
    use crate::config::{Config, OverlayConfig};
    use crate::platform::PlatformKind;

    #[test]
    fn windows_default_profiles_are_valid_and_exclude_games_and_obs() {
        let cfg = Config::default_for_platform(PlatformKind::Windows);
        cfg.validate().expect("perfis padrão Windows devem validar");

        let names: Vec<&str> = cfg.profiles.iter().map(|p| p.name.as_str()).collect();
        for expected in [
            "global",
            "navegadores",
            "vscode",
            "explorer",
            "krita",
            "photoshop",
            "illustrator",
            "resolve",
            "spotify",
        ] {
            assert!(
                names.contains(&expected),
                "perfil esperado ausente: {expected}"
            );
        }
        assert!(!names.contains(&"jogos"));
        assert!(!names.contains(&"steam"));
        assert!(!names.contains(&"obs"));
        assert_eq!(default_windows_profiles().len(), 9);
        assert_ne!(cfg.overlay.items, OverlayConfig::default().items);
    }
}
