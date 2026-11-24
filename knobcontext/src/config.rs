use crate::action::parse_key_name;
use crate::actions_catalog::{
    action_to_string, parse_action_string, ActionType, MacroStep, MediaAction, ScrollDirection,
};
use crate::context::ProfileMode;
use crate::error::KnobError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashSet;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const DEFAULT_KBD_PATH: &str = "/dev/input/by-id/usb-1189_8890-event-kbd";
pub const DEFAULT_MEDIA_PATH: &str = "/dev/input/by-id/usb-1189_8890-if02-event-kbd";
pub const DEFAULT_COMBO_WINDOW_MS: u64 = 200;
pub const DEFAULT_SCROLL_STEP: i32 = 20;
pub const DEFAULT_MODE: &str = "auto";
pub const GLOBAL_PROFILE_NAME: &str = "global";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub device: DeviceConfig,
    pub active_profile: String,
    /// Modo de perfil persistido: `"auto"` ou `"manual:<nome_do_perfil>"`.
    pub mode: String,
    pub profiles: Vec<ProfileConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceConfig {
    pub kbd_path: PathBuf,
    pub media_path: PathBuf,
    pub combo_window_ms: u64,
    /// Fator multiplicador (em pixels contínuos no Wayland) aplicado por unidade de `Scroll`.
    pub scroll_step: i32,
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
    /// Campo legado mantido apenas para retrocompatibilidade na leitura de TOMLs antigos.
    /// `KnobPress` é reservado para abrir/fechar o overlay e é ignorado na execução.
    #[serde(default, skip_serializing)]
    pub knob_press: Option<ActionConfig>,
    #[serde(default)]
    pub key1: ActionConfig,
    #[serde(default)]
    pub key2: ActionConfig,
    #[serde(default)]
    pub key3: ActionConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ActionConfig {
    VolumeUp,
    VolumeDown,
    VolumeMute,
    MediaPlayPause,
    MediaNext,
    MediaPrev,
    Scroll {
        delta: i32,
        horizontal: bool,
    },
    Shortcut {
        keys: Vec<String>,
    },
    Launch {
        desktop_file: String,
    },
    Open {
        target: String,
    },
    Exec {
        command: String,
    },
    Macro {
        steps: Vec<MacroStep>,
    },
    NextProfile,
    #[default]
    None,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum LegacyActionTable {
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
    None,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ActionSerdeRepr {
    StringRepr(String),
    LegacyTable(LegacyActionTable),
}

impl Serialize for ActionConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_display_string())
    }
}

impl<'de> Deserialize<'de> for ActionConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let repr = ActionSerdeRepr::deserialize(deserializer)?;
        match repr {
            ActionSerdeRepr::StringRepr(text) => {
                Self::parse_from_str(&text).map_err(serde::de::Error::custom)
            }
            ActionSerdeRepr::LegacyTable(tbl) => Ok(match tbl {
                LegacyActionTable::VolumeUp => Self::VolumeUp,
                LegacyActionTable::VolumeDown => Self::VolumeDown,
                LegacyActionTable::VolumeMute => Self::VolumeMute,
                LegacyActionTable::MediaPlayPause => Self::MediaPlayPause,
                LegacyActionTable::MediaNext => Self::MediaNext,
                LegacyActionTable::MediaPrev => Self::MediaPrev,
                LegacyActionTable::Scroll { delta, horizontal } => {
                    Self::Scroll { delta, horizontal }
                }
                LegacyActionTable::Shortcut { keys } => Self::Shortcut { keys },
                LegacyActionTable::NextProfile => Self::NextProfile,
                LegacyActionTable::None => Self::None,
            }),
        }
    }
}

impl ActionConfig {
    pub fn from_action_type(action: ActionType) -> Self {
        match action {
            ActionType::Shortcut(keys) => Self::Shortcut { keys },
            ActionType::Media(MediaAction::VolumeUp) => Self::VolumeUp,
            ActionType::Media(MediaAction::VolumeDown) => Self::VolumeDown,
            ActionType::Media(MediaAction::Mute) => Self::VolumeMute,
            ActionType::Media(MediaAction::PlayPause) => Self::MediaPlayPause,
            ActionType::Media(MediaAction::Prev) => Self::MediaPrev,
            ActionType::Media(MediaAction::Next) => Self::MediaNext,
            ActionType::LaunchApp(desktop_file) => Self::Launch { desktop_file },
            ActionType::OpenUrl(target) | ActionType::OpenFile(target) => Self::Open { target },
            ActionType::ExecCommand(command) => Self::Exec { command },
            ActionType::Scroll { direction, amount } => {
                let amount_i32 = i32::from(amount);
                match direction {
                    ScrollDirection::Up => Self::Scroll {
                        delta: -amount_i32,
                        horizontal: false,
                    },
                    ScrollDirection::Down => Self::Scroll {
                        delta: amount_i32,
                        horizontal: false,
                    },
                    ScrollDirection::Left => Self::Scroll {
                        delta: -amount_i32,
                        horizontal: true,
                    },
                    ScrollDirection::Right => Self::Scroll {
                        delta: amount_i32,
                        horizontal: true,
                    },
                }
            }
            ActionType::Macro(steps) => Self::Macro { steps },
            ActionType::None => Self::None,
        }
    }

    pub fn to_action_type(&self) -> ActionType {
        match self {
            Self::Shortcut { keys } => ActionType::Shortcut(keys.clone()),
            Self::VolumeUp => ActionType::Media(MediaAction::VolumeUp),
            Self::VolumeDown => ActionType::Media(MediaAction::VolumeDown),
            Self::VolumeMute => ActionType::Media(MediaAction::Mute),
            Self::MediaPlayPause => ActionType::Media(MediaAction::PlayPause),
            Self::MediaPrev => ActionType::Media(MediaAction::Prev),
            Self::MediaNext => ActionType::Media(MediaAction::Next),
            Self::Launch { desktop_file } => ActionType::LaunchApp(desktop_file.clone()),
            Self::Open { target } => {
                if target.starts_with("file://") {
                    ActionType::OpenFile(target.clone())
                } else {
                    ActionType::OpenUrl(target.clone())
                }
            }
            Self::Exec { command } => ActionType::ExecCommand(command.clone()),
            Self::Scroll { delta, horizontal } => {
                let amount = delta.unsigned_abs().clamp(1, 10) as u8;
                let direction = match (*horizontal, *delta >= 0) {
                    (false, false) => ScrollDirection::Up,
                    (false, true) => ScrollDirection::Down,
                    (true, false) => ScrollDirection::Left,
                    (true, true) => ScrollDirection::Right,
                };
                ActionType::Scroll { direction, amount }
            }
            Self::Macro { steps } => ActionType::Macro(steps.clone()),
            Self::NextProfile | Self::None => ActionType::None,
        }
    }

    /// Valida se a ação é executável.
    pub fn validate(&self) -> Result<(), KnobError> {
        match self {
            Self::Shortcut { keys } => {
                if keys.is_empty() {
                    return Err(KnobError::InvalidAction(
                        "atalho `shortcut` não pode ter lista `keys` vazia".to_string(),
                    ));
                }
                for key in keys {
                    parse_key_name(key)?;
                }
            }
            Self::Launch { desktop_file } => {
                if desktop_file.trim().is_empty() {
                    return Err(KnobError::InvalidAction(
                        "Launch: requer arquivo .desktop".to_string(),
                    ));
                }
            }
            Self::Open { target } => {
                let _ = parse_action_string(&format!("Open:{target}"))?;
            }
            Self::Exec { command } => {
                if command.trim().is_empty() {
                    return Err(KnobError::InvalidAction(
                        "Exec: não pode ter comando vazio".to_string(),
                    ));
                }
            }
            Self::Macro { steps } => {
                if steps.is_empty() {
                    return Err(KnobError::InvalidAction(
                        "Macro: precisa de ao menos um passo".to_string(),
                    ));
                }
                for step in steps {
                    match step {
                        MacroStep::Keys(keys) => {
                            if keys.is_empty() {
                                return Err(KnobError::InvalidAction(
                                    "Macro: passo de tecla vazio".to_string(),
                                ));
                            }
                            for k in keys {
                                parse_key_name(k)?;
                            }
                        }
                        MacroStep::WaitMs(ms) if *ms == 0 || *ms > 60_000 => {
                            return Err(KnobError::InvalidAction(format!(
                                "Macro: espera inválida ({ms} ms)"
                            )));
                        }
                        MacroStep::WaitMs(_) => {}
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Formata a ação na string canônica salva no TOML.
    pub fn to_display_string(&self) -> String {
        if matches!(self, Self::NextProfile) {
            return "NextProfile".to_string();
        }
        action_to_string(&self.to_action_type())
    }

    /// Faz o parsing de string de ação (incluindo todos os prefixos da Fase 4C).
    pub fn parse_from_str(input: &str) -> Result<Self, KnobError> {
        let trimmed = input.trim();
        if trimmed.eq_ignore_ascii_case("nextprofile")
            || trimmed.eq_ignore_ascii_case("next_profile")
        {
            return Ok(Self::NextProfile);
        }
        let parsed = parse_action_string(trimmed)?;
        Ok(Self::from_action_type(parsed))
    }
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            kbd_path: PathBuf::from(DEFAULT_KBD_PATH),
            media_path: PathBuf::from(DEFAULT_MEDIA_PATH),
            combo_window_ms: DEFAULT_COMBO_WINDOW_MS,
            scroll_step: DEFAULT_SCROLL_STEP,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            device: DeviceConfig::default(),
            active_profile: GLOBAL_PROFILE_NAME.to_string(),
            mode: DEFAULT_MODE.to_string(),
            profiles: vec![
                ProfileConfig {
                    name: GLOBAL_PROFILE_NAME.to_string(),
                    window_class: None,
                    knob_cw: ActionConfig::VolumeUp,
                    knob_ccw: ActionConfig::VolumeDown,
                    knob_press: None,
                    key1: ActionConfig::MediaPrev,
                    key2: ActionConfig::MediaNext,
                    key3: ActionConfig::MediaPlayPause,
                },
                ProfileConfig {
                    name: "firefox".to_string(),
                    window_class: Some("firefox".to_string()),
                    knob_cw: ActionConfig::Shortcut {
                        keys: vec!["Ctrl".to_string(), "Tab".to_string()],
                    },
                    knob_ccw: ActionConfig::Shortcut {
                        keys: vec!["Ctrl".to_string(), "Shift".to_string(), "Tab".to_string()],
                    },
                    knob_press: None,
                    key1: ActionConfig::Shortcut {
                        keys: vec!["Ctrl".to_string(), "T".to_string()],
                    },
                    key2: ActionConfig::Shortcut {
                        keys: vec!["Ctrl".to_string(), "W".to_string()],
                    },
                    key3: ActionConfig::Shortcut {
                        keys: vec!["Ctrl".to_string(), "R".to_string()],
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
                    knob_press: None,
                    key1: ActionConfig::Shortcut {
                        keys: vec!["Ctrl".to_string(), "Z".to_string()],
                    },
                    key2: ActionConfig::Shortcut {
                        keys: vec!["Ctrl".to_string(), "Shift".to_string(), "Z".to_string()],
                    },
                    key3: ActionConfig::Shortcut {
                        keys: vec!["B".to_string()],
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
        if config
            .profiles
            .iter()
            .any(|p| matches!(&p.knob_press, Some(act) if !matches!(act, ActionConfig::None)))
        {
            log::warn!(
                "O campo `knob_press` no config.toml é legado e será ignorado: KnobPress é reservado para o overlay."
            );
        }
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
        if !(1..=200).contains(&self.device.scroll_step) {
            return Err(KnobError::ConfigValidation(format!(
                "device.scroll_step deve estar entre 1 e 200 (recebido: {})",
                self.device.scroll_step
            )));
        }
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

    /// Resolve o `ProfileMode` a partir do campo persistido `mode` (`"auto"` ou `"manual:<nome>"`).
    /// Se o perfil manual não existir na lista de perfis, loga warning e cai em `ProfileMode::Auto`.
    pub fn resolve_profile_mode(&self) -> ProfileMode {
        let trimmed = self.mode.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(DEFAULT_MODE) {
            return ProfileMode::Auto;
        }
        if trimmed.len() > 7 && trimmed[..7].eq_ignore_ascii_case("manual:") {
            let target = trimmed[7..].trim();
            if let Some(found) = self
                .profiles
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case(target))
            {
                return ProfileMode::Manual(found.name.clone());
            }
            log::warn!(
                "Modo persistido '{}' referencia perfil inexistente '{}'; caindo em modo Automático.",
                self.mode,
                target
            );
            return ProfileMode::Auto;
        }
        log::warn!(
            "Valor de `mode` desconhecido no config.toml ('{}'); usando modo Automático.",
            self.mode
        );
        ProfileMode::Auto
    }

    pub fn set_profile_mode(&mut self, mode: &ProfileMode) {
        self.mode = match mode {
            ProfileMode::Auto => DEFAULT_MODE.to_string(),
            ProfileMode::Manual(name) => format!("manual:{name}"),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::{ActionConfig, Config, ProfileConfig};
    use crate::actions_catalog::MacroStep;
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
    fn loads_legacy_and_string_toml_configs() {
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
knob_ccw = "Scroll:up:3"
knob_press = "Launch:firefox.desktop"
key1 = "Open:https://example.com"
key2 = "Exec:playerctl play-pause"
key3 = "Macro:Ctrl+C,Wait(100),Ctrl+V"
"#;
        fs::write(&path, raw).expect("escrever arquivo temporário");
        let cfg = Config::load_from_path(&path).expect("config válida");
        let _ = fs::remove_file(&path);

        assert_eq!(cfg.active_profile, "brush");
        assert_eq!(cfg.profiles[0].window_class.as_deref(), Some("gimp"));
        assert_eq!(
            cfg.profiles[0].knob_cw,
            ActionConfig::Shortcut {
                keys: vec!["]".to_string()]
            }
        );
        assert_eq!(
            cfg.profiles[0].knob_ccw,
            ActionConfig::Scroll {
                delta: -3,
                horizontal: false
            }
        );
        assert_eq!(
            cfg.profiles[0].knob_press,
            Some(ActionConfig::Launch {
                desktop_file: "firefox.desktop".to_string()
            })
        );
        assert_eq!(
            cfg.profiles[0].key3,
            ActionConfig::Macro {
                steps: vec![
                    MacroStep::Keys(vec!["Ctrl".into(), "C".into()]),
                    MacroStep::WaitMs(100),
                    MacroStep::Keys(vec!["Ctrl".into(), "V".into()]),
                ]
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
            knob_press: None,
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
                keys: vec!["Ctrl".to_string(), "Tab".to_string()]
            })
        );
        assert_eq!(
            ActionConfig::parse_from_str("Scroll:up:3").ok(),
            Some(ActionConfig::Scroll {
                delta: -3,
                horizontal: false
            })
        );
        assert_eq!(
            ActionConfig::Shortcut {
                keys: vec!["Ctrl".to_string(), "Tab".to_string()]
            }
            .to_display_string(),
            "Ctrl+Tab"
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
    fn resolves_and_persists_profile_mode_roundtrip() {
        let path = unique_temp_path("mode_roundtrip");
        let mut cfg = Config::default();
        assert_eq!(
            cfg.resolve_profile_mode(),
            crate::context::ProfileMode::Auto
        );

        cfg.set_profile_mode(&crate::context::ProfileMode::Manual("global".to_string()));
        assert_eq!(cfg.mode, "manual:global");
        cfg.save_to_path(&path)
            .expect("salvar config com modo manual");

        let reloaded = Config::load_from_path(&path).expect("recarregar config");
        let _ = fs::remove_file(&path);
        assert_eq!(
            reloaded.resolve_profile_mode(),
            crate::context::ProfileMode::Manual("global".to_string())
        );

        // Perfil inexistente em `mode = "manual:..."` deve cair em Auto
        let missing_profile_cfg = Config {
            mode: "manual:perfil_inexistente".to_string(),
            ..Default::default()
        };
        assert_eq!(
            missing_profile_cfg.resolve_profile_mode(),
            crate::context::ProfileMode::Auto
        );
    }

    #[test]
    fn unknown_active_profile_returns_validation_error() {
        let path = unique_temp_path("invalid_profile");
        let raw = r#"
active_profile = "inexistente"

[[profiles]]
name = "volume"
knob_cw = "VolumeUp"
"#;
        fs::write(&path, raw).expect("escrever arquivo temporário");
        let err = Config::load_from_path(&path).expect_err("deve falhar na validação");
        let _ = fs::remove_file(&path);

        assert!(matches!(err, KnobError::ConfigValidation(_)));
    }
}
