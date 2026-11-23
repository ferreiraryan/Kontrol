use crate::error::KnobError;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const DEFAULT_KBD_PATH: &str = "/dev/input/by-id/usb-1189_8890-event-kbd";
pub const DEFAULT_MEDIA_PATH: &str = "/dev/input/by-id/usb-1189_8890-if02-event-kbd";
pub const DEFAULT_COMBO_WINDOW_MS: u64 = 200;

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
            active_profile: "volume".to_string(),
            profiles: vec![
                ProfileConfig {
                    name: "volume".to_string(),
                    knob_cw: ActionConfig::VolumeUp,
                    knob_ccw: ActionConfig::VolumeDown,
                    knob_press: ActionConfig::VolumeMute,
                    key1: ActionConfig::MediaPlayPause,
                    key2: ActionConfig::MediaPrev,
                    key3: ActionConfig::MediaNext,
                },
                ProfileConfig {
                    name: "scroll".to_string(),
                    knob_cw: ActionConfig::Scroll {
                        delta: 2,
                        horizontal: false,
                    },
                    knob_ccw: ActionConfig::Scroll {
                        delta: -2,
                        horizontal: false,
                    },
                    knob_press: ActionConfig::NextProfile,
                    key1: ActionConfig::Shortcut {
                        keys: vec!["ctrl".to_string(), "z".to_string()],
                    },
                    key2: ActionConfig::Shortcut {
                        keys: vec!["ctrl".to_string(), "c".to_string()],
                    },
                    key3: ActionConfig::Shortcut {
                        keys: vec!["ctrl".to_string(), "v".to_string()],
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

    pub fn validate(&self) -> Result<(), KnobError> {
        if self.profiles.is_empty() {
            return Err(KnobError::ConfigValidation(
                "ao menos um perfil deve ser definido em `profiles`".to_string(),
            ));
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
    use super::{ActionConfig, Config};
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
