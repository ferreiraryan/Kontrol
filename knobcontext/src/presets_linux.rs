use crate::config::{ActionConfig, ProfileConfig, GLOBAL_PROFILE_NAME};

fn shortcut(keys: &[&str]) -> ActionConfig {
    ActionConfig::Shortcut {
        keys: keys.iter().map(|k| (*k).to_string()).collect(),
    }
}

/// Perfis padrão pré-configurados para o primeiro uso no Linux (Wayland/Hyprland).
pub fn default_linux_profiles() -> Vec<ProfileConfig> {
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
            name: "firefox".to_string(),
            window_class: Some("firefox|zen|chromium".to_string()),
            window_title: None,
            fullscreen: None,
            priority: 0,
            inherits: Some(GLOBAL_PROFILE_NAME.to_string()),
            knob_cw: shortcut(&["Ctrl", "Tab"]),
            knob_ccw: shortcut(&["Ctrl", "Shift", "Tab"]),
            knob_press: None,
            key1: shortcut(&["Ctrl", "T"]),
            key2: shortcut(&["Ctrl", "W"]),
            key3: shortcut(&["Ctrl", "R"]),
            hub_modules: None,
        },
        ProfileConfig {
            name: "vscode".to_string(),
            window_class: Some("code|code-oss|codium".to_string()),
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
            name: "gimp".to_string(),
            window_class: Some("gimp".to_string()),
            window_title: None,
            fullscreen: None,
            priority: 0,
            inherits: Some(GLOBAL_PROFILE_NAME.to_string()),
            knob_cw: shortcut(&["]"]),
            knob_ccw: shortcut(&["["]),
            knob_press: None,
            key1: shortcut(&["Ctrl", "Z"]),
            key2: shortcut(&["Ctrl", "Y"]),
            key3: ActionConfig::NextProfile,
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
    ]
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::platform::PlatformKind;

    #[test]
    fn linux_default_profiles_are_valid() {
        let cfg = Config::default_for_platform(PlatformKind::Linux);
        cfg.validate().expect("perfis padrão Linux devem validar");
        let names: Vec<&str> = cfg.profiles.iter().map(|p| p.name.as_str()).collect();
        for expected in ["global", "firefox", "vscode", "gimp", "krita"] {
            assert!(names.contains(&expected));
        }
    }
}
