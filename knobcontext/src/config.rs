use crate::action::parse_key_name;
use crate::error::KnobError;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const DEFAULT_KBD_PATH: &str = "/dev/input/by-id/usb-1189_8890-event-kbd";
pub const DEFAULT_MEDIA_PATH: &str = "/dev/input/by-id/usb-1189_8890-if02-event-kbd";
pub const DEFAULT_COMBO_WINDOW_MS: u64 = 200;
pub const GLOBAL_PROFILE_NAME: &str = "global";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub device: DeviceConfig,
    pub active_profile: String,
    pub profiles: Vec<ProfileConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceConfig {
    pub kbd_path: PathBuf,
    pub media_path: PathBuf,
    pub combo_window_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileConfig {
    pub name: String,
    /// `WINDOWCLASS` do Hyprland associada a este perfil (ex.: `"firefox"`, `"gimp"`).
    /// `None` para o perfil `"global"` ou perfis apenas manuais.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_class: Option<String>,
    #[serde(default)]
    pub knob_cw: ActionConfig,
    #[serde(default)]
    pub knob_ccw: ActionConfig,
    #[serde(default)]
    pub knob_press: ActionConfig,
    #[serde(default)]
    pub key1: ActionConfig,
    #[serde(default)]
    pub key2: ActionConfig,
    #[serde(default)]
    pub key3: ActionConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActionConfig {
    VolumeUp,
    VolumeDown,
    VolumeMute,
    MediaPlayPause,
    MediaNext,
    MediaPrev,
    Scroll {
        delta: i32,
        #[serde(default)]
        horizontal: bool,
    },
    Shortcut {
        keys: Vec<String>,
    },
    NextProfile,
    #[default]
    None,
}

impl ActionConfig {
    /// Valida se a ação é executável (ex.: teclas válidas em `Shortcut`).
    pub fn validate(&self) -> Result<(), KnobError> {
        if let Self::Shortcut { keys } = self {
            if keys.is_empty() {
                return Err(KnobError::InvalidAction(
                    "atalho `shortcut` não pode ter lista `keys` vazia".to_string(),
                ));
            }
            for key in keys {
                parse_key_name(key)?;
            }
        }
        Ok(())
    }

    /// Formata a ação em texto amigável para edição na janela de configurações.
    pub fn to_display_string(&self) -> String {
        match self {
            Self::VolumeUp => "VolumeUp".to_string(),
            Self::VolumeDown => "VolumeDown".to_string(),
            Self::VolumeMute => "VolumeMute".to_string(),
            Self::MediaPlayPause => "MediaPlayPause".to_string(),
            Self::MediaNext => "MediaNext".to_string(),
            Self::MediaPrev => "MediaPrev".to_string(),
            Self::Scroll { delta, horizontal } => {
                if *horizontal {
                    format!("HScroll({delta})")
                } else {
                    format!("Scroll({delta})")
                }
            }
            Self::Shortcut { keys } => keys.join("+"),
            Self::NextProfile => "NextProfile".to_string(),
            Self::None => "None".to_string(),
        }
    }

    /// Faz o parsing de texto amigável (ex.: `"VolumeUp"`, `"Ctrl+Tab"`, `"next_tab"`, `"Scroll(2)"`).
    pub fn parse_from_str(input: &str) -> Result<Self, KnobError> {
        let trimmed = input.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
            return Ok(Self::None);
        }

        let lower = trimmed.to_ascii_lowercase();
        match lower.as_str() {
            "volumeup" | "volume_up" => return Ok(Self::VolumeUp),
            "volumedown" | "volume_down" => return Ok(Self::VolumeDown),
            "volumemute" | "volume_mute" | "mute" => return Ok(Self::VolumeMute),
            "mediaplaypause" | "media_play_pause" | "playpause" => {
                return Ok(Self::MediaPlayPause);
            }
            "medianext" | "media_next" | "next" => return Ok(Self::MediaNext),
            "mediaprev" | "media_prev" | "prev" => return Ok(Self::MediaPrev),
            "nextprofile" | "next_profile" => return Ok(Self::NextProfile),
            "next_tab" | "nexttab" => {
                return Ok(Self::Shortcut {
                    keys: vec!["ctrl".to_string(), "tab".to_string()],
                });
            }
            "prev_tab" | "prevtab" => {
                return Ok(Self::Shortcut {
                    keys: vec!["ctrl".to_string(), "shift".to_string(), "tab".to_string()],
                });
            }
            _ => {}
        }

        if let Some(inner) = lower
            .strip_prefix("scroll(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let delta = inner.trim().parse::<i32>().map_err(|_| {
                KnobError::InvalidAction(format!("delta inválido em Scroll: '{trimmed}'"))
            })?;
            return Ok(Self::Scroll {
                delta,
                horizontal: false,
            });
        }

        if let Some(inner) = lower
            .strip_prefix("hscroll(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let delta = inner.trim().parse::<i32>().map_err(|_| {
                KnobError::InvalidAction(format!("delta inválido em HScroll: '{trimmed}'"))
            })?;
            return Ok(Self::Scroll {
                delta,
                horizontal: true,
            });
        }

        let keys: Vec<String> = trimmed
            .split('+')
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect();
        let action = Self::Shortcut { keys };
        action.validate()?;
        Ok(action)
    }
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            kbd_path: PathBuf::from(DEFAULT_KBD_PATH),
            media_path: PathBuf::from(DEFAULT_MEDIA_PATH),
            combo_window_ms: DEFAULT_COMBO_WINDOW_MS,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            device: DeviceConfig::default(),
            active_profile: GLOBAL_PROFILE_NAME.to_string(),
            profiles: vec![
                ProfileConfig {
                    name: GLOBAL_PROFILE_NAME.to_string(),
                    window_class: None,
                    knob_cw: ActionConfig::VolumeUp,
                    knob_ccw: ActionConfig::VolumeDown,
                    knob_press: ActionConfig::VolumeMute,
                    key1: ActionConfig::MediaPrev,
                    key2: ActionConfig::MediaNext,
                    key3: ActionConfig::MediaPlayPause,
                },
                ProfileConfig {
                    name: "firefox".to_string(),
                    window_class: Some("firefox".to_string()),
                    knob_cw: ActionConfig::Shortcut {
                        keys: vec!["ctrl".to_string(), "tab".to_string()],
                    },
                    knob_ccw: ActionConfig::Shortcut {
                        keys: vec!["ctrl".to_string(), "shift".to_string(), "tab".to_string()],
                    },
                    knob_press: ActionConfig::None,
                    key1: ActionConfig::Shortcut {
                        keys: vec!["ctrl".to_string(), "t".to_string()],
                    },
                    key2: ActionConfig::Shortcut {
                        keys: vec!["ctrl".to_string(), "w".to_string()],
                    },
                    key3: ActionConfig::Shortcut {
                        keys: vec!["ctrl".to_string(), "r".to_string()],
                    },
                },
                ProfileConfig {
                    name: "gimp".to_string(),
                    window_class: Some("gimp".to_string()),
                    knob_cw: ActionConfig::Shortcut {
                        keys: vec!["]".to_string()],
                    },
                    knob_ccw: ActionConfig::Shortcut {
                        keys: vec!["[".to_string()],
                    },
                    knob_press: ActionConfig::None,
                    key1: ActionConfig::Shortcut {
                        keys: vec!["ctrl".to_string(), "z".to_string()],
                    },
                    key2: ActionConfig::Shortcut {
                        keys: vec!["ctrl".to_string(), "shift".to_string(), "z".to_string()],
                    },
                    key3: ActionConfig::Shortcut {
                        keys: vec!["b".to_string()],
                    },
                },
            ],
        }
    }
}

impl Config {
    /// Retorna o caminho padrão `~/.config/knobcontext/config.toml` (respeitando `XDG_CONFIG_HOME`).
    pub fn default_path() -> Option<PathBuf> {
        if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
            if !xdg.trim().is_empty() {
                return Some(PathBuf::from(xdg).join("knobcontext").join("config.toml"));
            }
        }
        env::var("HOME")
            .ok()
            .filter(|h| !h.trim().is_empty())
            .map(|home| {
                PathBuf::from(home)
                    .join(".config")
                    .join("knobcontext")
                    .join("config.toml")
            })
    }

    /// Carrega a configuração do caminho padrão; se ausente, retorna `Config::default()`.
    pub fn load_default_location() -> Result<(Self, Option<PathBuf>), KnobError> {
        match Self::default_path() {
            Some(path) => {
                let exists = path.exists();
                let cfg = Self::load_from_path(&path)?;
                Ok((cfg, if exists { Some(path) } else { None }))
            }
            None => Ok((Self::default(), None)),
        }
    }

    /// Carrega a configuração de um arquivo TOML; se o arquivo não existir, usa `Config::default()`.
    pub fn load_from_path(path: &Path) -> Result<Self, KnobError> {
        let content = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => {
                return Err(KnobError::ConfigRead {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };

        let config: Self = toml::from_str(&content).map_err(|err| KnobError::ConfigParse {
            path: path.to_path_buf(),
            message: err.to_string(),
        })?;
        config.validate()?;
        Ok(config)
    }

    /// Valida e salva a configuração em formato TOML legível no caminho especificado.
    pub fn save_to_path(&self, path: &Path) -> Result<(), KnobError> {
        self.validate()?;
        let serialized = toml::to_string_pretty(self).map_err(|err| {
            KnobError::ConfigValidation(format!("falha ao serializar TOML: {err}"))
        })?;
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|source| KnobError::ConfigWrite {
                    path: path.to_path_buf(),
                    source,
                })?;
            }
        }
        fs::write(path, serialized).map_err(|source| KnobError::ConfigWrite {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Salva no caminho padrão `~/.config/knobcontext/config.toml`.
    pub fn save_default_location(&self) -> Result<PathBuf, KnobError> {
        let path = Self::default_path().ok_or_else(|| {
            KnobError::ConfigValidation(
                "não foi possível determinar ~/.config/knobcontext/config.toml (HOME/XDG_CONFIG_HOME ausente)"
                    .to_string(),
            )
        })?;
        self.save_to_path(&path)?;
        Ok(path)
    }

    pub fn validate(&self) -> Result<(), KnobError> {
        if self.profiles.is_empty() {
            return Err(KnobError::ConfigValidation(
                "ao menos um perfil deve ser definido em `profiles`".to_string(),
            ));
        }

        let mut seen = HashSet::new();
        for profile in &self.profiles {
            let trimmed = profile.name.trim();
            if trimmed.is_empty() {
                return Err(KnobError::ConfigValidation(
                    "nome de perfil não pode ser vazio".to_string(),
                ));
            }
            if !seen.insert(trimmed.to_ascii_lowercase()) {
                return Err(KnobError::ConfigValidation(format!(
                    "nome de perfil duplicado: '{}'",
                    profile.name
                )));
            }
            for action in [
                &profile.knob_cw,
                &profile.knob_ccw,
                &profile.knob_press,
                &profile.key1,
                &profile.key2,
                &profile.key3,
            ] {
                action.validate()?;
            }
        }

        if !self.profiles.iter().any(|p| p.name == self.active_profile) {
            return Err(KnobError::ConfigValidation(format!(
                "perfil ativo '{}' não encontrado na lista `profiles`",
                self.active_profile
            )));
        }
        Ok(())
    }

    pub fn active_profile_index(&self) -> usize {
        self.profiles
            .iter()
            .position(|p| p.name == self.active_profile)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::{ActionConfig, Config, ProfileConfig};
    use crate::error::KnobError;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_path(suffix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "knobcontext_test_{}_{}_{}.toml",
            std::process::id(),
            nanos,
            suffix
        ))
    }

    #[test]
    fn missing_file_falls_back_to_default() {
        let missing = unique_temp_path("missing");
        let cfg = Config::load_from_path(&missing).expect("deve usar default quando ausente");
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn loads_valid_toml_config() {
        let path = unique_temp_path("valid");
        let raw = r#"
active_profile = "brush"

[device]
kbd_path = "/dev/input/by-id/usb-1189_8890-event-kbd"
media_path = "/dev/input/by-id/usb-1189_8890-if02-event-kbd"
combo_window_ms = 150

[[profiles]]
name = "brush"
window_class = "gimp"
knob_cw = { type = "shortcut", keys = ["]"] }
knob_ccw = { type = "shortcut", keys = ["["] }
knob_press = { type = "volume_mute" }
key1 = { type = "media_play_pause" }
key2 = { type = "scroll", delta = 3 }
key3 = { type = "next_profile" }
"#;
        fs::write(&path, raw).expect("escrever arquivo temporário");
        let cfg = Config::load_from_path(&path).expect("config válida");
        let _ = fs::remove_file(&path);

        assert_eq!(cfg.active_profile, "brush");
        assert_eq!(cfg.device.combo_window_ms, 150);
        assert_eq!(cfg.profiles.len(), 1);
        assert_eq!(cfg.profiles[0].window_class.as_deref(), Some("gimp"));
        assert_eq!(
            cfg.profiles[0].knob_cw,
            ActionConfig::Shortcut {
                keys: vec!["]".to_string()]
            }
        );
        assert_eq!(
            cfg.profiles[0].key2,
            ActionConfig::Scroll {
                delta: 3,
                horizontal: false
            }
        );
    }

    #[test]
    fn saves_and_reloads_toml_roundtrip() {
        let path = unique_temp_path("roundtrip");
        let original = Config::default();
        original
            .save_to_path(&path)
            .expect("salvar configuração válida");
        let reloaded = Config::load_from_path(&path).expect("recarregar configuração");
        let _ = fs::remove_file(&path);

        assert_eq!(original, reloaded);
    }

    #[test]
    fn rejects_duplicate_profile_names() {
        let mut cfg = Config::default();
        cfg.profiles.push(ProfileConfig {
            name: "Global".to_string(),
            window_class: None,
            knob_cw: ActionConfig::None,
            knob_ccw: ActionConfig::None,
            knob_press: ActionConfig::None,
            key1: ActionConfig::None,
            key2: ActionConfig::None,
            key3: ActionConfig::None,
        });
        assert!(matches!(
            cfg.validate(),
            Err(KnobError::ConfigValidation(_))
        ));
    }

    #[test]
    fn parses_and_formats_action_strings() {
        assert_eq!(
            ActionConfig::parse_from_str("VolumeUp").ok(),
            Some(ActionConfig::VolumeUp)
        );
        assert_eq!(
            ActionConfig::parse_from_str("next_tab").ok(),
            Some(ActionConfig::Shortcut {
                keys: vec!["ctrl".to_string(), "tab".to_string()]
            })
        );
        assert_eq!(
            ActionConfig::parse_from_str("Ctrl+Shift+Z").ok(),
            Some(ActionConfig::Shortcut {
                keys: vec!["Ctrl".to_string(), "Shift".to_string(), "Z".to_string()]
            })
        );
        assert_eq!(
            ActionConfig::parse_from_str("Scroll(-3)").ok(),
            Some(ActionConfig::Scroll {
                delta: -3,
                horizontal: false
            })
        );
        assert_eq!(
            ActionConfig::Shortcut {
                keys: vec!["ctrl".to_string(), "tab".to_string()]
            }
            .to_display_string(),
            "ctrl+tab"
        );
        assert!(ActionConfig::parse_from_str("tecla_invalida_xyz").is_err());
    }

    #[test]
    fn malformed_toml_returns_clear_parse_error() {
        let path = unique_temp_path("malformed");
        fs::write(&path, "active_profile = [unclosed").expect("escrever arquivo temporário");
        let err = Config::load_from_path(&path).expect_err("deve falhar com ConfigParse");
        let _ = fs::remove_file(&path);

        assert!(matches!(err, KnobError::ConfigParse { .. }));
    }

    #[test]
    fn unknown_active_profile_returns_validation_error() {
        let path = unique_temp_path("invalid_profile");
        let raw = r#"
active_profile = "inexistente"

[[profiles]]
name = "volume"
knob_cw = { type = "volume_up" }
"#;
        fs::write(&path, raw).expect("escrever arquivo temporário");
        let err = Config::load_from_path(&path).expect_err("deve falhar na validação");
        let _ = fs::remove_file(&path);

        assert!(matches!(err, KnobError::ConfigValidation(_)));
    }
}
