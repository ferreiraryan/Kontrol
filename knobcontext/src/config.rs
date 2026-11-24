use crate::action::parse_key_name;
use crate::actions_catalog::{
    action_to_string, parse_action_string, ActionType, MacroStep, MediaAction, ScrollDirection,
};
use crate::context::ProfileMode;
use crate::error::KnobError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, HashSet};
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
pub const DEFAULT_OVERLAY_BROWSE_TIMEOUT_MS: u64 = 5_000;
pub const DEFAULT_OVERLAY_ADJUST_TIMEOUT_MS: u64 = 10_000;
pub const DEFAULT_VOLUME_STEP: i32 = 5;
pub const DEFAULT_BRIGHTNESS_STEP: i32 = 5;
pub const DEFAULT_MAX_VOLUME_PERCENT: i32 = 150;
pub const DEFAULT_OVERLAY_BOTTOM_MARGIN_PX: i32 = 20;

fn default_true() -> bool {
    true
}

fn default_bottom_margin_px() -> i32 {
    DEFAULT_OVERLAY_BOTTOM_MARGIN_PX
}

fn default_slider_step() -> i32 {
    5
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayKey3Behavior {
    /// Key3 atua como Confirmar/Entrar alternativo (além do KnobPress).
    #[default]
    Confirm,
    /// Key3 fecha o overlay imediatamente.
    Close,
}

/// Resolve a ação efetiva de um slot de slider, priorizando `action` (novo formato completo)
/// com fallback automático para o campo legado `command` (`ActionConfig::Exec`).
pub fn resolve_slider_action(action: &ActionConfig, legacy_command: &str) -> ActionConfig {
    if !matches!(action, ActionConfig::None) {
        return action.clone();
    }
    let trimmed = legacy_command.trim();
    if !trimmed.is_empty() {
        ActionConfig::Exec {
            command: trimmed.to_string(),
        }
    } else {
        ActionConfig::None
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomGroupSlider {
    #[serde(default = "default_slider_step")]
    pub step: i32,
    #[serde(default)]
    pub infinite: bool,
    #[serde(default)]
    pub up_action: ActionConfig,
    #[serde(default)]
    pub down_action: ActionConfig,
    #[serde(default)]
    pub toggle_action: ActionConfig,
    /// Campos legados mantidos para retrocompatibilidade com TOMLs antigos.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub up_command: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub down_command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toggle_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_command: Option<String>,
}

impl CustomGroupSlider {
    pub fn effective_up_action(&self) -> ActionConfig {
        resolve_slider_action(&self.up_action, &self.up_command)
    }

    pub fn effective_down_action(&self) -> ActionConfig {
        resolve_slider_action(&self.down_action, &self.down_command)
    }

    pub fn effective_toggle_action(&self) -> ActionConfig {
        resolve_slider_action(
            &self.toggle_action,
            self.toggle_command.as_deref().unwrap_or(""),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomGroupEntry {
    pub label: String,
    #[serde(default)]
    pub action: ActionConfig,
    /// Quando presente, esta entrada do submenu atua como um Slider interativo no Hub.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slider: Option<CustomGroupSlider>,
    /// Caminho opcional de imagem/ícone exibido no carrossel para esta entrada.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OverlayItemKind {
    Profiles,
    AudioMixer,
    AudioOutputs,
    Workspaces,
    Brightness,
    CaelestiaWallpapers,
    CaelestiaToggles,
    BrowserTabs,
    MediaControl,
    Zoom,
    ScrollPage,
    BrushSize,
    UndoRedo,
    QuickAction {
        action: ActionConfig,
    },
    CustomGroup {
        #[serde(default)]
        entries: Vec<CustomGroupEntry>,
    },
    CustomSlider {
        #[serde(default = "default_slider_step")]
        step: i32,
        #[serde(default)]
        infinite: bool,
        #[serde(default)]
        up_action: Box<ActionConfig>,
        #[serde(default)]
        down_action: Box<ActionConfig>,
        #[serde(default)]
        toggle_action: Box<ActionConfig>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        up_command: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        down_command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        toggle_command: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        read_command: Option<String>,
    },
}

impl OverlayItemKind {
    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::Profiles => "Perfis / Modo",
            Self::AudioMixer => "Mixer de Áudio",
            Self::AudioOutputs => "Saída de Som",
            Self::Workspaces => "Workspaces (Hyprland)",
            Self::Brightness => "Brilho dos Monitores",
            Self::CaelestiaWallpapers => "Wallpapers (Caelestia)",
            Self::CaelestiaToggles => "Controles Caelestia",
            Self::BrowserTabs => "Abas do Navegador",
            Self::MediaControl => "Controle de Mídia",
            Self::Zoom => "Zoom",
            Self::ScrollPage => "Navegação de Página",
            Self::BrushSize => "Tamanho do Pincel",
            Self::UndoRedo => "Desfazer / Refazer",
            Self::QuickAction { .. } => "Ação Rápida",
            Self::CustomGroup { .. } => "Submenu Customizado",
            Self::CustomSlider { .. } => "Slider Customizado",
        }
    }
}

/// Modelos prontos de módulos nativos disponíveis no seletor "+ Adicionar módulo".
pub fn builtin_hub_module_templates() -> Vec<OverlayItemConfig> {
    vec![
        OverlayItemConfig {
            id: "browser_tabs".to_string(),
            title: "Abas".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::BrowserTabs,
        },
        OverlayItemConfig {
            id: "media_control".to_string(),
            title: "Mídia".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::MediaControl,
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
        OverlayItemConfig {
            id: "brush_size".to_string(),
            title: "Pincel".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::BrushSize,
        },
        OverlayItemConfig {
            id: "undo_redo".to_string(),
            title: "Desfazer".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::UndoRedo,
        },
        OverlayItemConfig {
            id: "profiles".to_string(),
            title: "Perfis".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::Profiles,
        },
        OverlayItemConfig {
            id: "audio_mixer".to_string(),
            title: "Sons & Apps".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::AudioMixer,
        },
        OverlayItemConfig {
            id: "audio_outputs".to_string(),
            title: "Saída de Som".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::AudioOutputs,
        },
        OverlayItemConfig {
            id: "workspaces".to_string(),
            title: "Workspaces".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::Workspaces,
        },
        OverlayItemConfig {
            id: "brightness".to_string(),
            title: "Brilho".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::Brightness,
        },
        OverlayItemConfig {
            id: "caelestia_wallpapers".to_string(),
            title: "Wallpapers".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::CaelestiaWallpapers,
        },
        OverlayItemConfig {
            id: "caelestia_toggles".to_string(),
            title: "Caelestia".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::CaelestiaToggles,
        },
    ]
}

fn deserialize_resilient_hub_modules<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<OverlayItemConfig>>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw: Option<Vec<toml::Value>> = Option::deserialize(deserializer)?;
    let Some(entries) = raw else {
        return Ok(None);
    };

    let mut parsed = Vec::with_capacity(entries.len());
    for (idx, value) in entries.into_iter().enumerate() {
        let toml::Value::Table(mut table) = value else {
            log::warn!(
                "Entrada inválida em `hub_modules` (índice {idx}): esperado tabela TOML, ignorando."
            );
            continue;
        };

        // Aceita aliases amigáveis `type` -> `kind` e `label` -> `title`
        if !table.contains_key("kind") {
            if let Some(ty) = table.remove("type") {
                table.insert("kind".to_string(), ty);
            }
        }
        if !table.contains_key("title") {
            if let Some(lbl) = table.remove("label") {
                table.insert("title".to_string(), lbl);
            }
        }
        if !table.contains_key("id") {
            let fallback_id = table
                .get("kind")
                .and_then(|v| v.as_str())
                .map(|k| format!("{k}_{idx}"))
                .unwrap_or_else(|| format!("module_{idx}"));
            table.insert("id".to_string(), toml::Value::String(fallback_id));
        }

        match toml::Value::Table(table).try_into::<OverlayItemConfig>() {
            Ok(item) => parsed.push(item),
            Err(err) => {
                log::warn!("Módulo do Hub desconhecido ou inválido ignorado no TOML: {err}");
            }
        }
    }

    Ok(Some(parsed))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlayItemConfig {
    pub id: String,
    pub title: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Caminho opcional de imagem/ícone exibido no carrossel para este item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(flatten)]
    pub kind: OverlayItemKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayConfig {
    pub browse_timeout_ms: u64,
    pub adjust_timeout_ms: u64,
    pub volume_step: i32,
    pub brightness_step: i32,
    pub max_volume_percent: i32,
    pub key3_behavior: OverlayKey3Behavior,
    /// Exibe thumbnails de wallpapers e ícones de aplicativos quando disponíveis (opcional).
    #[serde(default = "default_true")]
    pub show_images: bool,
    /// Margem inferior (em pixels) do dock flutuante na base central da tela.
    #[serde(default = "default_bottom_margin_px")]
    pub bottom_margin_px: i32,
    /// Habilita animações suaves de 60fps no carrossel e transições de tela do overlay.
    #[serde(default = "default_true")]
    pub animations_enabled: bool,
    pub items: Vec<OverlayItemConfig>,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            browse_timeout_ms: DEFAULT_OVERLAY_BROWSE_TIMEOUT_MS,
            adjust_timeout_ms: DEFAULT_OVERLAY_ADJUST_TIMEOUT_MS,
            volume_step: DEFAULT_VOLUME_STEP,
            brightness_step: DEFAULT_BRIGHTNESS_STEP,
            max_volume_percent: DEFAULT_MAX_VOLUME_PERCENT,
            key3_behavior: OverlayKey3Behavior::Confirm,
            show_images: true,
            bottom_margin_px: DEFAULT_OVERLAY_BOTTOM_MARGIN_PX,
            animations_enabled: true,
            items: default_overlay_items(),
        }
    }
}

pub fn default_overlay_items() -> Vec<OverlayItemConfig> {
    vec![
        OverlayItemConfig {
            id: "profiles".to_string(),
            title: "Perfis".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::Profiles,
        },
        OverlayItemConfig {
            id: "audio_mixer".to_string(),
            title: "Sons & Apps".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::AudioMixer,
        },
        OverlayItemConfig {
            id: "audio_outputs".to_string(),
            title: "Saída de Som".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::AudioOutputs,
        },
        OverlayItemConfig {
            id: "workspaces".to_string(),
            title: "Workspaces".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::Workspaces,
        },
        OverlayItemConfig {
            id: "brightness".to_string(),
            title: "Brilho".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::Brightness,
        },
        OverlayItemConfig {
            id: "caelestia_wallpapers".to_string(),
            title: "Wallpapers".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::CaelestiaWallpapers,
        },
        OverlayItemConfig {
            id: "caelestia_toggles".to_string(),
            title: "Caelestia".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::CaelestiaToggles,
        },
    ]
}

pub const DEFAULT_OBS_WEBSOCKET_PORT: u16 = 4455;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ObsProviderConfig {
    pub enabled: bool,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

impl Default for ObsProviderConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_OBS_WEBSOCKET_PORT,
            password: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProvidersConfig {
    pub obs: ObsProviderConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub device: DeviceConfig,
    pub active_profile: String,
    /// Modo de perfil persistido: `"auto"` ou `"manual:<nome_do_perfil>"`.
    pub mode: String,
    pub overlay: OverlayConfig,
    #[serde(default)]
    pub providers: ProvidersConfig,
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

fn is_zero(value: &i32) -> bool {
    *value == 0
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileConfig {
    pub name: String,
    /// `WINDOWCLASS` do Hyprland associada a este perfil (aceita vários padrões separados por `|` ou `,`, ex.: `"firefox|zen|chromium"`).
    /// `None` para o perfil `"global"` ou perfis apenas manuais.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_class: Option<String>,
    /// Substrings case-insensitive do título da janela separadas por `|` (ex.: `"YouTube|Twitch|Netflix"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_title: Option<String>,
    /// Se `Some(true)`, aplica apenas em janelas fullscreen; se `Some(false)`, apenas fora de fullscreen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fullscreen: Option<bool>,
    /// Prioridade explícita na resolução contextual (maior vence; empate resolve por especificidade).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub priority: i32,
    /// Nome do perfil pai para herdar ações não configuradas (`None`) neste perfil (ex.: `"global"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherits: Option<String>,
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
    /// Módulos do Hub exibidos quando este perfil está ativo.
    ///
    /// Semântica de herança:
    /// - `None` (omitido no TOML): herda da cadeia `inherits` até `global` / `overlay.items`.
    /// - `Some(vec![])`: lista vazia explícita (Hub vazio para este perfil).
    /// - `Some(vec![...])`: substitui os módulos do Hub para este perfil.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_resilient_hub_modules"
    )]
    pub hub_modules: Option<Vec<OverlayItemConfig>>,
}

impl ProfileConfig {
    /// Retorna `true` se o perfil define ao menos um critério contextual (`window_class`, `window_title` ou `fullscreen`).
    pub fn has_context_rule(&self) -> bool {
        self.window_class
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
            || self
                .window_title
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
            || self.fullscreen.is_some()
    }
}

/// Presets contextuais prontos que o usuário pode adicionar com 1 clique nas Configurações.
#[derive(Debug, Clone)]
pub struct ProfilePreset {
    pub label: &'static str,
    pub profile: ProfileConfig,
}

pub fn builtin_profile_presets() -> Vec<ProfilePreset> {
    vec![
        ProfilePreset {
            label: "Navegador (Abas)",
            profile: ProfileConfig {
                name: "navegador".to_string(),
                window_class: Some("firefox|zen|chromium|brave-browser|google-chrome".to_string()),
                window_title: None,
                fullscreen: None,
                priority: 0,
                inherits: Some(GLOBAL_PROFILE_NAME.to_string()),
                knob_cw: ActionConfig::Shortcut {
                    keys: vec!["Ctrl".into(), "Tab".into()],
                },
                knob_ccw: ActionConfig::Shortcut {
                    keys: vec!["Ctrl".into(), "Shift".into(), "Tab".into()],
                },
                knob_press: None,
                key1: ActionConfig::Shortcut {
                    keys: vec!["Ctrl".into(), "T".into()],
                },
                key2: ActionConfig::Shortcut {
                    keys: vec!["Ctrl".into(), "W".into()],
                },
                key3: ActionConfig::Shortcut {
                    keys: vec!["Ctrl".into(), "R".into()],
                },
                hub_modules: Some(vec![
                    OverlayItemConfig {
                        id: "browser_tabs".to_string(),
                        title: "Abas".to_string(),
                        enabled: true,
                        icon: None,
                        kind: OverlayItemKind::BrowserTabs,
                    },
                    OverlayItemConfig {
                        id: "media_control".to_string(),
                        title: "Mídia".to_string(),
                        enabled: true,
                        icon: None,
                        kind: OverlayItemKind::MediaControl,
                    },
                    OverlayItemConfig {
                        id: "zoom".to_string(),
                        title: "Zoom".to_string(),
                        enabled: true,
                        icon: None,
                        kind: OverlayItemKind::Zoom,
                    },
                    OverlayItemConfig {
                        id: "audio_mixer".to_string(),
                        title: "Sons & Apps".to_string(),
                        enabled: true,
                        icon: None,
                        kind: OverlayItemKind::AudioMixer,
                    },
                ]),
            },
        },
        ProfilePreset {
            label: "Vídeo no Navegador (YouTube/Twitch)",
            profile: ProfileConfig {
                name: "video_web".to_string(),
                window_class: Some("firefox|zen|chromium|brave-browser|google-chrome".to_string()),
                window_title: Some("YouTube|Twitch|Netflix|Prime Video".to_string()),
                fullscreen: None,
                priority: 20,
                inherits: Some(GLOBAL_PROFILE_NAME.to_string()),
                knob_cw: ActionConfig::Shortcut {
                    keys: vec!["Right".into()],
                },
                knob_ccw: ActionConfig::Shortcut {
                    keys: vec!["Left".into()],
                },
                knob_press: None,
                key1: ActionConfig::Shortcut {
                    keys: vec!["F".into()],
                },
                key2: ActionConfig::Shortcut {
                    keys: vec!["M".into()],
                },
                key3: ActionConfig::Shortcut {
                    keys: vec!["Space".into()],
                },
                hub_modules: Some(vec![
                    OverlayItemConfig {
                        id: "media_control".to_string(),
                        title: "Mídia".to_string(),
                        enabled: true,
                        icon: None,
                        kind: OverlayItemKind::MediaControl,
                    },
                    OverlayItemConfig {
                        id: "audio_mixer".to_string(),
                        title: "Sons & Apps".to_string(),
                        enabled: true,
                        icon: None,
                        kind: OverlayItemKind::AudioMixer,
                    },
                    OverlayItemConfig {
                        id: "brightness".to_string(),
                        title: "Brilho".to_string(),
                        enabled: true,
                        icon: None,
                        kind: OverlayItemKind::Brightness,
                    },
                ]),
            },
        },
        ProfilePreset {
            label: "Editor / IDE (VS Code / Cursor / Zed)",
            profile: ProfileConfig {
                name: "editor".to_string(),
                window_class: Some("code|code-oss|cursor|zed".to_string()),
                window_title: None,
                fullscreen: None,
                priority: 5,
                inherits: Some(GLOBAL_PROFILE_NAME.to_string()),
                knob_cw: ActionConfig::Shortcut {
                    keys: vec!["Ctrl".into(), "PageDown".into()],
                },
                knob_ccw: ActionConfig::Shortcut {
                    keys: vec!["Ctrl".into(), "PageUp".into()],
                },
                knob_press: None,
                key1: ActionConfig::Shortcut {
                    keys: vec!["Ctrl".into(), "Z".into()],
                },
                key2: ActionConfig::Shortcut {
                    keys: vec!["Ctrl".into(), "Shift".into(), "Z".into()],
                },
                key3: ActionConfig::Shortcut {
                    keys: vec!["Ctrl".into(), "S".into()],
                },
                hub_modules: Some(vec![
                    OverlayItemConfig {
                        id: "undo_redo".to_string(),
                        title: "Desfazer".to_string(),
                        enabled: true,
                        icon: None,
                        kind: OverlayItemKind::UndoRedo,
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
                    OverlayItemConfig {
                        id: "workspaces".to_string(),
                        title: "Workspaces".to_string(),
                        enabled: true,
                        icon: None,
                        kind: OverlayItemKind::Workspaces,
                    },
                ]),
            },
        },
        ProfilePreset {
            label: "GIMP / Krita (Pincel)",
            profile: ProfileConfig {
                name: "desenho".to_string(),
                window_class: Some("gimp|krita".to_string()),
                window_title: None,
                fullscreen: None,
                priority: 5,
                inherits: Some(GLOBAL_PROFILE_NAME.to_string()),
                knob_cw: ActionConfig::Shortcut {
                    keys: vec!["]".into()],
                },
                knob_ccw: ActionConfig::Shortcut {
                    keys: vec!["[".into()],
                },
                knob_press: None,
                key1: ActionConfig::Shortcut {
                    keys: vec!["Ctrl".into(), "Z".into()],
                },
                key2: ActionConfig::Shortcut {
                    keys: vec!["Ctrl".into(), "Shift".into(), "Z".into()],
                },
                key3: ActionConfig::Shortcut {
                    keys: vec!["B".into()],
                },
                hub_modules: Some(vec![
                    OverlayItemConfig {
                        id: "brush_size".to_string(),
                        title: "Pincel".to_string(),
                        enabled: true,
                        icon: None,
                        kind: OverlayItemKind::BrushSize,
                    },
                    OverlayItemConfig {
                        id: "undo_redo".to_string(),
                        title: "Desfazer".to_string(),
                        enabled: true,
                        icon: None,
                        kind: OverlayItemKind::UndoRedo,
                    },
                    OverlayItemConfig {
                        id: "zoom".to_string(),
                        title: "Zoom".to_string(),
                        enabled: true,
                        icon: None,
                        kind: OverlayItemKind::Zoom,
                    },
                    OverlayItemConfig {
                        id: "audio_mixer".to_string(),
                        title: "Sons & Apps".to_string(),
                        enabled: true,
                        icon: None,
                        kind: OverlayItemKind::AudioMixer,
                    },
                ]),
            },
        },
        ProfilePreset {
            label: "Terminal (Scroll)",
            profile: ProfileConfig {
                name: "terminal".to_string(),
                window_class: Some("kitty|alacritty|foot|wezterm|ghostty".to_string()),
                window_title: None,
                fullscreen: None,
                priority: 0,
                inherits: Some(GLOBAL_PROFILE_NAME.to_string()),
                knob_cw: ActionConfig::Scroll {
                    delta: -3,
                    horizontal: false,
                },
                knob_ccw: ActionConfig::Scroll {
                    delta: 3,
                    horizontal: false,
                },
                knob_press: None,
                key1: ActionConfig::Shortcut {
                    keys: vec!["Ctrl".into(), "Shift".into(), "C".into()],
                },
                key2: ActionConfig::Shortcut {
                    keys: vec!["Ctrl".into(), "Shift".into(), "V".into()],
                },
                key3: ActionConfig::None,
                hub_modules: None,
            },
        },
        ProfilePreset {
            label: "Tela Cheia (Mídia / Jogos)",
            profile: ProfileConfig {
                name: "fullscreen".to_string(),
                window_class: None,
                window_title: None,
                fullscreen: Some(true),
                priority: 10,
                inherits: Some(GLOBAL_PROFILE_NAME.to_string()),
                knob_cw: ActionConfig::VolumeUp,
                knob_ccw: ActionConfig::VolumeDown,
                knob_press: None,
                key1: ActionConfig::MediaPrev,
                key2: ActionConfig::MediaNext,
                key3: ActionConfig::MediaPlayPause,
                hub_modules: None,
            },
        },
    ]
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
    Provider {
        provider_id: String,
        command_id: String,
        args: BTreeMap<String, String>,
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
            ActionType::Provider {
                provider_id,
                command_id,
                args,
            } => Self::Provider {
                provider_id,
                command_id,
                args,
            },
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
            Self::Provider {
                provider_id,
                command_id,
                args,
            } => ActionType::Provider {
                provider_id: provider_id.clone(),
                command_id: command_id.clone(),
                args: args.clone(),
            },
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
            Self::Provider {
                provider_id,
                command_id,
                args,
            } => {
                if provider_id.trim().is_empty() || command_id.trim().is_empty() {
                    return Err(KnobError::InvalidAction(
                        "Provider: requer id do provider e comando".to_string(),
                    ));
                }
                crate::providers::validate_builtin_provider_action(provider_id, command_id, args)
                    .map_err(|err| KnobError::InvalidAction(err.to_string()))?;
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
            overlay: OverlayConfig::default(),
            providers: ProvidersConfig::default(),
            profiles: vec![
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
                    window_class: Some("firefox".to_string()),
                    window_title: None,
                    fullscreen: None,
                    priority: 0,
                    inherits: Some(GLOBAL_PROFILE_NAME.to_string()),
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
                    hub_modules: None,
                },
                ProfileConfig {
                    name: "gimp".to_string(),
                    window_class: Some("gimp".to_string()),
                    window_title: None,
                    fullscreen: None,
                    priority: 0,
                    inherits: Some(GLOBAL_PROFILE_NAME.to_string()),
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
                    hub_modules: None,
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
        if !(1..=50).contains(&self.overlay.volume_step) {
            return Err(KnobError::ConfigValidation(format!(
                "overlay.volume_step deve estar entre 1 e 50 (recebido: {})",
                self.overlay.volume_step
            )));
        }
        if !(1..=50).contains(&self.overlay.brightness_step) {
            return Err(KnobError::ConfigValidation(format!(
                "overlay.brightness_step deve estar entre 1 e 50 (recebido: {})",
                self.overlay.brightness_step
            )));
        }
        if !(100..=200).contains(&self.overlay.max_volume_percent) {
            return Err(KnobError::ConfigValidation(format!(
                "overlay.max_volume_percent deve estar entre 100 e 200 (recebido: {})",
                self.overlay.max_volume_percent
            )));
        }
        for item in &self.overlay.items {
            if item.id.trim().is_empty() || item.title.trim().is_empty() {
                return Err(KnobError::ConfigValidation(
                    "itens do overlay devem possuir `id` e `title` não vazios".to_string(),
                ));
            }
            match &item.kind {
                OverlayItemKind::QuickAction { action } => action.validate()?,
                OverlayItemKind::CustomSlider {
                    up_action,
                    down_action,
                    toggle_action,
                    ..
                } => {
                    up_action.validate()?;
                    down_action.validate()?;
                    toggle_action.validate()?;
                }
                OverlayItemKind::CustomGroup { entries } => {
                    for entry in entries {
                        if let Some(ref sl) = entry.slider {
                            sl.up_action.validate()?;
                            sl.down_action.validate()?;
                            sl.toggle_action.validate()?;
                        } else {
                            entry.action.validate()?;
                        }
                    }
                }
                _ => {}
            }
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
            if let Some(ref modules) = profile.hub_modules {
                for item in modules {
                    if item.id.trim().is_empty() || item.title.trim().is_empty() {
                        return Err(KnobError::ConfigValidation(format!(
                            "módulos do Hub no perfil '{}' devem possuir `id` e `title` não vazios",
                            profile.name
                        )));
                    }
                    match &item.kind {
                        OverlayItemKind::QuickAction { action } => action.validate()?,
                        OverlayItemKind::CustomGroup { entries } => {
                            for entry in entries {
                                entry.action.validate()?;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        if !self.profiles.iter().any(|p| p.name == self.active_profile) {
            return Err(KnobError::ConfigValidation(format!(
                "perfil ativo '{}' não encontrado na lista `profiles`",
                self.active_profile
            )));
        }

        // Valida referências e ciclos em `inherits`
        for profile in &self.profiles {
            if let Some(parent_name) = profile
                .inherits
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                if parent_name.eq_ignore_ascii_case(&profile.name) {
                    return Err(KnobError::ConfigValidation(format!(
                        "perfil '{}' não pode herdar de si mesmo",
                        profile.name
                    )));
                }
                if !self
                    .profiles
                    .iter()
                    .any(|p| p.name.eq_ignore_ascii_case(parent_name))
                {
                    return Err(KnobError::ConfigValidation(format!(
                        "perfil '{}' herda de perfil inexistente '{}'",
                        profile.name, parent_name
                    )));
                }
            }
        }
        for start in &self.profiles {
            let mut visited = HashSet::new();
            let mut current = Some(start.name.to_ascii_lowercase());
            while let Some(name_lower) = current {
                if !visited.insert(name_lower.clone()) {
                    return Err(KnobError::ConfigValidation(format!(
                        "ciclo detectado na herança `inherits` do perfil '{}'",
                        start.name
                    )));
                }
                current = self
                    .profiles
                    .iter()
                    .find(|p| p.name.eq_ignore_ascii_case(&name_lower))
                    .and_then(|p| p.inherits.as_deref())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_ascii_lowercase);
            }
        }
        Ok(())
    }

    pub fn active_profile_index(&self) -> usize {
        self.profiles
            .iter()
            .position(|p| p.name == self.active_profile)
            .unwrap_or(0)
    }

    /// Resolve a lista de módulos do Hub efetiva para um perfil, seguindo a cadeia `inherits`.
    ///
    /// Retorna `(módulos_efetivos, herdado_de)`:
    /// - `herdado_de == None`: o próprio perfil define `hub_modules` (ou é `global` usando `overlay.items`).
    /// - `herdado_de == Some(nome)`: herdado do perfil pai `nome` (ou `"global"`).
    ///
    /// Semântica:
    /// - `None` em `hub_modules` → sobe pela cadeia `inherits` até encontrar `Some(...)`, caindo para `global` / `overlay.items`.
    /// - `Some(vec![])` → lista vazia explícita (não herda).
    /// - `Some(vec![...])` → substitui os módulos do pai.
    pub fn resolve_hub_modules(
        &self,
        profile_name: &str,
    ) -> (Vec<OverlayItemConfig>, Option<String>) {
        let mut visited = HashSet::new();
        let mut current = Some(profile_name.to_string());
        let mut is_first = true;

        while let Some(name) = current {
            let key = name.to_ascii_lowercase();
            if !visited.insert(key) {
                break;
            }
            let Some(profile) = self
                .profiles
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case(&name))
            else {
                break;
            };

            if let Some(ref mods) = profile.hub_modules {
                let inherited_from = if is_first {
                    None
                } else {
                    Some(profile.name.clone())
                };
                return (mods.clone(), inherited_from);
            }

            is_first = false;
            current = profile
                .inherits
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string);
        }

        // Fallback para `global.hub_modules` se existir e não foi visitado ainda
        if !visited.contains(GLOBAL_PROFILE_NAME) {
            if let Some(global) = self
                .profiles
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case(GLOBAL_PROFILE_NAME))
            {
                if let Some(ref mods) = global.hub_modules {
                    return (mods.clone(), Some(global.name.clone()));
                }
            }
        }

        let inherited_from = if profile_name.eq_ignore_ascii_case(GLOBAL_PROFILE_NAME) {
            None
        } else {
            Some(GLOBAL_PROFILE_NAME.to_string())
        };
        (self.overlay.items.clone(), inherited_from)
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
            window_title: None,
            fullscreen: None,
            priority: 0,
            inherits: None,
            knob_cw: ActionConfig::None,
            knob_ccw: ActionConfig::None,
            knob_press: None,
            key1: ActionConfig::None,
            key2: ActionConfig::None,
            key3: ActionConfig::None,
            hub_modules: None,
        });
        assert!(matches!(
            cfg.validate(),
            Err(KnobError::ConfigValidation(_))
        ));
    }

    #[test]
    fn validates_inherits_and_detects_cycles() {
        let mut cfg = Config::default();
        cfg.profiles[1].inherits = Some("nao_existe".to_string());
        assert!(matches!(
            cfg.validate(),
            Err(KnobError::ConfigValidation(_))
        ));

        // Ciclo A -> B -> A
        let mut cyclic = Config::default();
        cyclic.profiles[1].inherits = Some("gimp".to_string());
        cyclic.profiles[2].inherits = Some("firefox".to_string());
        assert!(matches!(
            cyclic.validate(),
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

    #[test]
    fn overlay_config_custom_items_roundtrip_toml() {
        use super::{
            CustomGroupEntry, CustomGroupSlider, OverlayItemConfig, OverlayItemKind,
            OverlayKey3Behavior,
        };

        let path = unique_temp_path("overlay_roundtrip");
        let mut cfg = Config::default();
        cfg.overlay.browse_timeout_ms = 6_000;
        cfg.overlay.adjust_timeout_ms = 12_000;
        cfg.overlay.volume_step = 10;
        cfg.overlay.brightness_step = 5;
        cfg.overlay.key3_behavior = OverlayKey3Behavior::Close;
        cfg.overlay.show_images = false;
        cfg.overlay.bottom_margin_px = 24;
        cfg.overlay.animations_enabled = false;
        cfg.overlay.items.push(OverlayItemConfig {
            id: "quick_term".to_string(),
            title: "Abrir Terminal".to_string(),
            enabled: true,
            icon: Some("/usr/share/icons/hicolor/128x128/apps/kitty.png".to_string()),
            kind: OverlayItemKind::QuickAction {
                action: ActionConfig::Exec {
                    command: "kitty".to_string(),
                },
            },
        });
        cfg.overlay.items.push(OverlayItemConfig {
            id: "custom_group_1".to_string(),
            title: "Ferramentas".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::CustomGroup {
                entries: vec![
                    CustomGroupEntry {
                        label: "Copiar".to_string(),
                        action: ActionConfig::Shortcut {
                            keys: vec!["Ctrl".into(), "C".into()],
                        },
                        slider: None,
                        icon: None,
                    },
                    CustomGroupEntry {
                        label: "Zoom Tela".to_string(),
                        action: ActionConfig::None,
                        slider: Some(CustomGroupSlider {
                            step: 10,
                            infinite: true,
                            up_action: ActionConfig::Shortcut {
                                keys: vec!["Ctrl".into(), "Plus".into()],
                            },
                            down_action: ActionConfig::Shortcut {
                                keys: vec!["Ctrl".into(), "Minus".into()],
                            },
                            toggle_action: ActionConfig::Shortcut {
                                keys: vec!["Ctrl".into(), "0".into()],
                            },
                            up_command: String::new(),
                            down_command: String::new(),
                            toggle_command: None,
                            read_command: Some("echo 40".to_string()),
                        }),
                        icon: Some("/tmp/zoom_icon.png".to_string()),
                    },
                ],
            },
        });
        cfg.overlay.items.push(OverlayItemConfig {
            id: "custom_slider_1".to_string(),
            title: "Zoom Hyprland".to_string(),
            enabled: true,
            icon: None,
            kind: OverlayItemKind::CustomSlider {
                step: 5,
                infinite: false,
                up_action: Box::new(ActionConfig::Exec {
                    command: "hyprctl keyword cursor:zoom_factor 2".to_string(),
                }),
                down_action: Box::new(ActionConfig::Exec {
                    command: "hyprctl keyword cursor:zoom_factor 1".to_string(),
                }),
                toggle_action: Box::new(ActionConfig::MediaPlayPause),
                up_command: String::new(),
                down_command: String::new(),
                toggle_command: None,
                read_command: Some("echo 50".to_string()),
            },
        });

        cfg.save_to_path(&path).expect("salvar overlay config");
        let reloaded = Config::load_from_path(&path).expect("recarregar overlay config");
        let _ = fs::remove_file(&path);
        assert_eq!(cfg.overlay, reloaded.overlay);
    }

    #[test]
    fn resolves_contextual_hub_modules_with_inheritance_and_explicit_empty() {
        use super::{OverlayItemConfig, OverlayItemKind};

        let mut cfg = Config::default();
        // Sem `hub_modules` em `firefox` -> herda do `global` (overlay.items)
        let (mods_default, inherited) = cfg.resolve_hub_modules("firefox");
        assert_eq!(mods_default.len(), cfg.overlay.items.len());
        assert_eq!(inherited.as_deref(), Some("global"));

        // `firefox` define lista customizada -> substitui o pai
        cfg.profiles[1].hub_modules = Some(vec![
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
        ]);
        let (mods_ff, inherited_ff) = cfg.resolve_hub_modules("firefox");
        assert_eq!(mods_ff.len(), 2);
        assert_eq!(mods_ff[0].title, "Abas");
        assert_eq!(inherited_ff, None);

        // `gimp` herda de `firefox` sem definir `hub_modules` -> herda os 2 módulos de `firefox`
        cfg.profiles[2].inherits = Some("firefox".to_string());
        let (mods_gimp, inherited_gimp) = cfg.resolve_hub_modules("gimp");
        assert_eq!(mods_gimp.len(), 2);
        assert_eq!(inherited_gimp.as_deref(), Some("firefox"));

        // `gimp` define lista vazia explícita -> Hub vazio (não herda!)
        cfg.profiles[2].hub_modules = Some(Vec::new());
        let (mods_empty, inherited_empty) = cfg.resolve_hub_modules("gimp");
        assert!(mods_empty.is_empty());
        assert_eq!(inherited_empty, None);
    }

    #[test]
    fn ignores_unknown_hub_module_types_in_toml() {
        let toml_text = r#"
active_profile = "global"

[[profiles]]
name = "global"
knob_cw = "VolumeUp"
knob_ccw = "VolumeDown"

[[profiles]]
name = "firefox"
window_class = "firefox"
inherits = "global"

[[profiles.hub_modules]]
type = "browser_tabs"
label = "Abas"

[[profiles.hub_modules]]
type = "modulo_que_nao_existe"
label = "Invalido"

[[profiles.hub_modules]]
type = "undo_redo"
label = "Desfazer"
"#;
        let path = unique_temp_path("unknown_hub_mod");
        fs::write(&path, toml_text).expect("gravar toml");
        let cfg = Config::load_from_path(&path).expect("carregar ignorando inválido");
        let _ = fs::remove_file(&path);

        let (mods, inherited) = cfg.resolve_hub_modules("firefox");
        assert_eq!(inherited, None);
        assert_eq!(mods.len(), 2);
        assert_eq!(mods[0].title, "Abas");
        assert_eq!(mods[1].title, "Desfazer");
    }
}
