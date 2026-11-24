use crate::config::{
    ActionConfig, Config, OverlayConfig, OverlayItemConfig, OverlayItemKind, OverlayKey3Behavior,
};
use crate::context::ProfileMode;
use crate::input::KnobEvent;
use crate::system::audio::{AudioMixerSnapshot, AudioOutputsSnapshot};
use crate::system::{SystemRequest, SystemResponse};
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub const VISIBLE_HUB_ROWS: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubScreen {
    Root,
    Profiles,
    AudioMixer,
    AudioOutputs,
    Workspaces,
    Brightness,
    CaelestiaWallpapers,
    CaelestiaToggles,
    CustomGroup { item_id: String, title: String },
    CustomSlider { item_id: String, title: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionMode {
    Browsing,
    Adjusting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubRowView {
    pub title: String,
    pub subtitle: String,
    pub badge: String,
    pub value_percent: i32,
    pub max_percent: i32,
    pub is_slider: bool,
    pub is_active: bool,
    pub is_muted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubViewModel {
    pub breadcrumb: String,
    pub context_badge: String,
    pub adjusting: bool,
    pub loading: bool,
    pub empty_message: String,
    pub rows: Vec<HubRowView>,
    pub highlighted_visible_index: usize,
    pub scroll_indicator: String,
    pub footer_hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubEffect {
    /// Em modo normal: despacha evento para o perfil ativo.
    DispatchNormal(KnobEvent),
    /// Abre ou atualiza o Overlay Slint com o ViewModel calculado.
    RenderOverlay(HubViewModel),
    /// Fecha o Overlay Slint.
    CloseOverlay,
    /// Altera e persiste `ProfileMode` (`Auto` / `Manual`).
    SetProfileMode(ProfileMode),
    /// Envia uma requisição assíncrona ao `SystemWorker`.
    System(SystemRequest),
    /// Executa uma `ActionConfig` diretamente via `ActionDispatcher`.
    ExecuteAction(ActionConfig),
}

/// Máquina de estados hierárquica do Hub Interativo do KnobContext.
pub struct HubController {
    open: bool,
    settings_open: bool,
    screen: HubScreen,
    interaction: InteractionMode,
    root_cursor: usize,
    sub_cursor: usize,
    last_interaction: Option<Instant>,
    loading: bool,
    audio_mixer: AudioMixerSnapshot,
    audio_outputs: AudioOutputsSnapshot,
    custom_slider_values: HashMap<String, i32>,
}

impl HubController {
    pub fn new() -> Self {
        Self {
            open: false,
            settings_open: false,
            screen: HubScreen::Root,
            interaction: InteractionMode::Browsing,
            root_cursor: 0,
            sub_cursor: 0,
            last_interaction: None,
            loading: false,
            audio_mixer: AudioMixerSnapshot::default(),
            audio_outputs: AudioOutputsSnapshot::default(),
            custom_slider_values: HashMap::new(),
        }
    }

    #[cfg(test)]
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn set_settings_open(&mut self, open: bool) {
        self.settings_open = open;
        if open && self.open {
            self.close_internal();
        }
    }

    fn close_internal(&mut self) {
        self.open = false;
        self.screen = HubScreen::Root;
        self.interaction = InteractionMode::Browsing;
        self.last_interaction = None;
        self.loading = false;
    }

    fn enabled_root_items<'a>(&self, overlay: &'a OverlayConfig) -> Vec<&'a OverlayItemConfig> {
        overlay.items.iter().filter(|i| i.enabled).collect()
    }

    fn is_confirm_event(event: KnobEvent, overlay: &OverlayConfig) -> bool {
        match event {
            KnobEvent::KnobPress => true,
            KnobEvent::Key3 => overlay.key3_behavior == OverlayKey3Behavior::Confirm,
            _ => false,
        }
    }

    pub fn handle_event(
        &mut self,
        event: KnobEvent,
        now: Instant,
        config: &Config,
        mode: &ProfileMode,
        contextual_profile: &str,
    ) -> Vec<HubEffect> {
        if !self.open {
            if event == KnobEvent::KnobPress && !self.settings_open {
                let items = self.enabled_root_items(&config.overlay);
                if items.is_empty() {
                    return Vec::new();
                }
                self.open = true;
                self.screen = HubScreen::Root;
                self.interaction = InteractionMode::Browsing;
                self.root_cursor = self.root_cursor.min(items.len().saturating_sub(1));
                self.last_interaction = Some(now);
                self.loading = false;
                let vm = self.build_view_model(config, mode, contextual_profile);
                return vec![HubEffect::RenderOverlay(vm)];
            }
            return vec![HubEffect::DispatchNormal(event)];
        }

        // Overlay aberto
        self.last_interaction = Some(now);

        if event == KnobEvent::Key3 && config.overlay.key3_behavior == OverlayKey3Behavior::Close {
            self.close_internal();
            return vec![HubEffect::CloseOverlay];
        }

        if event == KnobEvent::Key1 {
            return self.handle_back(config, mode, contextual_profile);
        }

        if event == KnobEvent::Key2 {
            return self.handle_secondary(config, mode, contextual_profile);
        }

        if Self::is_confirm_event(event, &config.overlay) {
            return self.handle_confirm(config, mode, contextual_profile);
        }

        match event {
            KnobEvent::RotateCw => self.handle_rotate(1, config, mode, contextual_profile),
            KnobEvent::RotateCcw => self.handle_rotate(-1, config, mode, contextual_profile),
            _ => Vec::new(),
        }
    }

    pub fn apply_system_response(
        &mut self,
        response: SystemResponse,
        config: &Config,
        mode: &ProfileMode,
        contextual_profile: &str,
    ) -> Option<HubEffect> {
        let relevant = match response {
            SystemResponse::AudioMixer(snap) => {
                self.audio_mixer = snap;
                if matches!(self.screen, HubScreen::AudioMixer) {
                    self.loading = false;
                    let len = self.audio_mixer.streams.len();
                    if len == 0 {
                        self.sub_cursor = 0;
                        self.interaction = InteractionMode::Browsing;
                    } else {
                        self.sub_cursor = self.sub_cursor.min(len - 1);
                    }
                    true
                } else {
                    false
                }
            }
            SystemResponse::AudioOutputs(snap) => {
                self.audio_outputs = snap;
                if matches!(self.screen, HubScreen::AudioOutputs) {
                    self.loading = false;
                    let len = self.audio_outputs.outputs.len();
                    if len == 0 {
                        self.sub_cursor = 0;
                    } else {
                        self.sub_cursor = self.sub_cursor.min(len - 1);
                    }
                    true
                } else {
                    false
                }
            }
            SystemResponse::CustomSliderValue { item_id, percent } => {
                self.custom_slider_values.insert(item_id.clone(), percent);
                if matches!(&self.screen, HubScreen::CustomSlider { item_id: cur, .. } if cur == &item_id)
                {
                    self.loading = false;
                    true
                } else {
                    false
                }
            }
            SystemResponse::Error(_) => {
                if self.loading {
                    self.loading = false;
                    true
                } else {
                    false
                }
            }
        };

        if self.open && relevant {
            Some(HubEffect::RenderOverlay(self.build_view_model(
                config,
                mode,
                contextual_profile,
            )))
        } else {
            None
        }
    }

    pub fn check_timeout(&mut self, now: Instant, overlay: &OverlayConfig) -> Option<HubEffect> {
        if !self.open {
            return None;
        }
        let last = self.last_interaction?;
        let timeout_ms = match self.interaction {
            InteractionMode::Browsing => overlay.browse_timeout_ms.max(1_000),
            InteractionMode::Adjusting => overlay.adjust_timeout_ms.max(2_000),
        };
        if now.saturating_duration_since(last) >= Duration::from_millis(timeout_ms) {
            self.close_internal();
            Some(HubEffect::CloseOverlay)
        } else {
            None
        }
    }

    fn handle_back(
        &mut self,
        config: &Config,
        mode: &ProfileMode,
        contextual_profile: &str,
    ) -> Vec<HubEffect> {
        if self.interaction == InteractionMode::Adjusting {
            self.interaction = InteractionMode::Browsing;
            return vec![HubEffect::RenderOverlay(self.build_view_model(
                config,
                mode,
                contextual_profile,
            ))];
        }
        if self.screen != HubScreen::Root {
            self.screen = HubScreen::Root;
            self.loading = false;
            return vec![HubEffect::RenderOverlay(self.build_view_model(
                config,
                mode,
                contextual_profile,
            ))];
        }
        self.close_internal();
        vec![HubEffect::CloseOverlay]
    }

    fn handle_rotate(
        &mut self,
        dir: i32,
        config: &Config,
        mode: &ProfileMode,
        contextual_profile: &str,
    ) -> Vec<HubEffect> {
        if self.interaction == InteractionMode::Adjusting {
            return self.handle_adjust_rotation(dir, config, mode, contextual_profile);
        }

        let count = self.current_item_count(config);
        if count == 0 {
            return Vec::new();
        }

        let cursor = match self.screen {
            HubScreen::Root => &mut self.root_cursor,
            _ => &mut self.sub_cursor,
        };
        if dir > 0 {
            *cursor = (*cursor + 1) % count;
        } else {
            *cursor = (*cursor + count - 1) % count;
        }

        vec![HubEffect::RenderOverlay(self.build_view_model(
            config,
            mode,
            contextual_profile,
        ))]
    }

    fn handle_adjust_rotation(
        &mut self,
        dir: i32,
        config: &Config,
        mode: &ProfileMode,
        contextual_profile: &str,
    ) -> Vec<HubEffect> {
        let mut effects = Vec::new();
        match &self.screen {
            HubScreen::AudioMixer => {
                let step = config.overlay.volume_step.max(1);
                let max_vol = config.overlay.max_volume_percent.max(100);
                if let Some(stream) = self.audio_mixer.streams.get_mut(self.sub_cursor) {
                    let next = (stream.volume_percent + dir * step).clamp(0, max_vol);
                    stream.volume_percent = next;
                    if stream.is_master {
                        effects.push(HubEffect::System(SystemRequest::SetMasterVolume {
                            sink_name: stream.sink_name.clone().unwrap_or_default(),
                            percent: next,
                            max_percent: max_vol,
                        }));
                    } else if let Some(idx) = stream.sink_input_index {
                        effects.push(HubEffect::System(SystemRequest::SetAppVolume {
                            index: idx,
                            percent: next,
                            max_percent: max_vol,
                        }));
                    }
                }
            }
            HubScreen::CustomSlider { item_id, .. } => {
                let id_clone = item_id.clone();
                if let Some(OverlayItemConfig {
                    kind:
                        OverlayItemKind::CustomSlider {
                            step,
                            up_command,
                            down_command,
                            ..
                        },
                    ..
                }) = config.overlay.items.iter().find(|i| i.id == id_clone)
                {
                    let cur = self
                        .custom_slider_values
                        .get(&id_clone)
                        .copied()
                        .unwrap_or(50);
                    let next = (cur + dir * (*step).max(1)).clamp(0, 100);
                    self.custom_slider_values.insert(id_clone, next);
                    let cmd = if dir > 0 { up_command } else { down_command };
                    if !cmd.trim().is_empty() {
                        effects.push(HubEffect::System(SystemRequest::RunShellCommand {
                            command: cmd.clone(),
                        }));
                    }
                }
            }
            _ => {}
        }

        effects.push(HubEffect::RenderOverlay(self.build_view_model(
            config,
            mode,
            contextual_profile,
        )));
        effects
    }

    fn handle_secondary(
        &mut self,
        config: &Config,
        mode: &ProfileMode,
        contextual_profile: &str,
    ) -> Vec<HubEffect> {
        let mut effects = Vec::new();
        match &self.screen {
            HubScreen::AudioMixer => {
                if let Some(stream) = self.audio_mixer.streams.get_mut(self.sub_cursor) {
                    stream.muted = !stream.muted;
                    if stream.is_master {
                        effects.push(HubEffect::System(SystemRequest::ToggleMasterMute {
                            sink_name: stream.sink_name.clone().unwrap_or_default(),
                        }));
                    } else if let Some(idx) = stream.sink_input_index {
                        effects.push(HubEffect::System(SystemRequest::ToggleAppMute {
                            index: idx,
                        }));
                    }
                    effects.push(HubEffect::RenderOverlay(self.build_view_model(
                        config,
                        mode,
                        contextual_profile,
                    )));
                }
            }
            HubScreen::CustomSlider { item_id, .. } => {
                if let Some(OverlayItemConfig {
                    kind:
                        OverlayItemKind::CustomSlider {
                            toggle_command: Some(cmd),
                            ..
                        },
                    ..
                }) = config.overlay.items.iter().find(|i| i.id == *item_id)
                {
                    if !cmd.trim().is_empty() {
                        effects.push(HubEffect::System(SystemRequest::RunShellCommand {
                            command: cmd.clone(),
                        }));
                    }
                }
            }
            _ => {}
        }
        effects
    }

    fn handle_confirm(
        &mut self,
        config: &Config,
        mode: &ProfileMode,
        contextual_profile: &str,
    ) -> Vec<HubEffect> {
        // Se está travado ajustando, confirmar solta o ajuste!
        if self.interaction == InteractionMode::Adjusting {
            self.interaction = InteractionMode::Browsing;
            return vec![HubEffect::RenderOverlay(self.build_view_model(
                config,
                mode,
                contextual_profile,
            ))];
        }

        match self.screen.clone() {
            HubScreen::Root => {
                let items = self.enabled_root_items(&config.overlay);
                let Some(item) = items.get(self.root_cursor).copied().cloned() else {
                    return Vec::new();
                };
                self.enter_root_item(item, config, mode, contextual_profile)
            }
            HubScreen::Profiles => {
                let chosen_mode = if self.sub_cursor == 0 {
                    ProfileMode::Auto
                } else if let Some(profile) = config.profiles.get(self.sub_cursor - 1) {
                    ProfileMode::Manual(profile.name.clone())
                } else {
                    return Vec::new();
                };
                self.close_internal();
                vec![
                    HubEffect::SetProfileMode(chosen_mode),
                    HubEffect::CloseOverlay,
                ]
            }
            HubScreen::AudioMixer => {
                if self.audio_mixer.streams.get(self.sub_cursor).is_some() {
                    self.interaction = InteractionMode::Adjusting;
                    vec![HubEffect::RenderOverlay(self.build_view_model(
                        config,
                        mode,
                        contextual_profile,
                    ))]
                } else {
                    Vec::new()
                }
            }
            HubScreen::AudioOutputs => {
                if let Some(output) = self.audio_outputs.outputs.get(self.sub_cursor).cloned() {
                    for out in &mut self.audio_outputs.outputs {
                        out.is_default = out.name == output.name;
                    }
                    vec![
                        HubEffect::System(SystemRequest::SetDefaultAudioOutput {
                            sink_name: output.name,
                        }),
                        HubEffect::RenderOverlay(self.build_view_model(
                            config,
                            mode,
                            contextual_profile,
                        )),
                    ]
                } else {
                    Vec::new()
                }
            }
            HubScreen::CustomGroup { item_id, .. } => {
                let action = config
                    .overlay
                    .items
                    .iter()
                    .find(|i| i.id == item_id)
                    .and_then(|item| match &item.kind {
                        OverlayItemKind::CustomGroup { entries } => {
                            entries.get(self.sub_cursor).map(|e| e.action.clone())
                        }
                        _ => None,
                    });
                if let Some(act) = action {
                    self.close_internal();
                    vec![HubEffect::ExecuteAction(act), HubEffect::CloseOverlay]
                } else {
                    Vec::new()
                }
            }
            HubScreen::CustomSlider { .. } => {
                self.interaction = InteractionMode::Adjusting;
                vec![HubEffect::RenderOverlay(self.build_view_model(
                    config,
                    mode,
                    contextual_profile,
                ))]
            }
            HubScreen::Workspaces
            | HubScreen::Brightness
            | HubScreen::CaelestiaWallpapers
            | HubScreen::CaelestiaToggles => Vec::new(),
        }
    }

    fn enter_root_item(
        &mut self,
        item: OverlayItemConfig,
        config: &Config,
        mode: &ProfileMode,
        contextual_profile: &str,
    ) -> Vec<HubEffect> {
        let mut effects = Vec::new();
        self.sub_cursor = 0;
        self.interaction = InteractionMode::Browsing;

        match item.kind {
            OverlayItemKind::Profiles => {
                self.screen = HubScreen::Profiles;
                self.loading = false;
                self.sub_cursor = match mode {
                    ProfileMode::Auto => 0,
                    ProfileMode::Manual(name) => config
                        .profiles
                        .iter()
                        .position(|p| p.name.eq_ignore_ascii_case(name))
                        .map(|i| i + 1)
                        .unwrap_or(0),
                };
            }
            OverlayItemKind::AudioMixer => {
                self.screen = HubScreen::AudioMixer;
                self.loading = true;
                effects.push(HubEffect::System(SystemRequest::FetchAudioMixer));
            }
            OverlayItemKind::AudioOutputs => {
                self.screen = HubScreen::AudioOutputs;
                self.loading = true;
                effects.push(HubEffect::System(SystemRequest::FetchAudioOutputs));
            }
            OverlayItemKind::Workspaces => {
                self.screen = HubScreen::Workspaces;
                self.loading = false;
            }
            OverlayItemKind::Brightness => {
                self.screen = HubScreen::Brightness;
                self.loading = false;
            }
            OverlayItemKind::CaelestiaWallpapers => {
                self.screen = HubScreen::CaelestiaWallpapers;
                self.loading = false;
            }
            OverlayItemKind::CaelestiaToggles => {
                self.screen = HubScreen::CaelestiaToggles;
                self.loading = false;
            }
            OverlayItemKind::QuickAction { action } => {
                self.close_internal();
                return vec![HubEffect::ExecuteAction(action), HubEffect::CloseOverlay];
            }
            OverlayItemKind::CustomGroup { .. } => {
                self.screen = HubScreen::CustomGroup {
                    item_id: item.id,
                    title: item.title,
                };
                self.loading = false;
            }
            OverlayItemKind::CustomSlider { read_command, .. } => {
                self.screen = HubScreen::CustomSlider {
                    item_id: item.id.clone(),
                    title: item.title,
                };
                self.interaction = InteractionMode::Adjusting;
                self.loading = false;
                if let Some(cmd) = read_command.filter(|s| !s.trim().is_empty()) {
                    effects.push(HubEffect::System(SystemRequest::ReadCustomSlider {
                        item_id: item.id,
                        read_command: cmd,
                    }));
                }
            }
        }

        effects.push(HubEffect::RenderOverlay(self.build_view_model(
            config,
            mode,
            contextual_profile,
        )));
        effects
    }

    fn current_item_count(&self, config: &Config) -> usize {
        match &self.screen {
            HubScreen::Root => self.enabled_root_items(&config.overlay).len(),
            HubScreen::Profiles => config.profiles.len() + 1,
            HubScreen::AudioMixer => self.audio_mixer.streams.len(),
            HubScreen::AudioOutputs => self.audio_outputs.outputs.len(),
            HubScreen::CustomGroup { item_id, .. } => config
                .overlay
                .items
                .iter()
                .find(|i| i.id == *item_id)
                .and_then(|i| match &i.kind {
                    OverlayItemKind::CustomGroup { entries } => Some(entries.len()),
                    _ => None,
                })
                .unwrap_or(0),
            HubScreen::CustomSlider { .. } => 1,
            HubScreen::Workspaces
            | HubScreen::Brightness
            | HubScreen::CaelestiaWallpapers
            | HubScreen::CaelestiaToggles => 0,
        }
    }

    pub fn build_view_model(
        &self,
        config: &Config,
        mode: &ProfileMode,
        contextual_profile: &str,
    ) -> HubViewModel {
        let context_badge = match mode {
            ProfileMode::Auto => format!("Auto • {contextual_profile}"),
            ProfileMode::Manual(name) => format!("Manual • {name}"),
        };

        let (breadcrumb, all_rows, empty_message, footer_hint) = match &self.screen {
            HubScreen::Root => {
                let rows: Vec<HubRowView> = self
                    .enabled_root_items(&config.overlay)
                    .into_iter()
                    .map(|item| {
                        let subtitle = match &item.kind {
                            OverlayItemKind::Profiles => context_badge.clone(),
                            OverlayItemKind::AudioMixer => {
                                "Volume global e por aplicativo".to_string()
                            }
                            OverlayItemKind::AudioOutputs => {
                                "Alternar dispositivo de saída".to_string()
                            }
                            OverlayItemKind::Workspaces => "Workspaces do Hyprland".to_string(),
                            OverlayItemKind::Brightness => "Brilho dos monitores".to_string(),
                            OverlayItemKind::CaelestiaWallpapers => {
                                "Trocar wallpaper (Caelestia)".to_string()
                            }
                            OverlayItemKind::CaelestiaToggles => {
                                "DND, Game Mode e Idle".to_string()
                            }
                            OverlayItemKind::QuickAction { action } => action.to_display_string(),
                            OverlayItemKind::CustomGroup { entries } => {
                                format!("{} ações", entries.len())
                            }
                            OverlayItemKind::CustomSlider { .. } => {
                                "Controle deslizante customizado".to_string()
                            }
                        };
                        HubRowView {
                            title: item.title.clone(),
                            subtitle,
                            badge: "›".to_string(),
                            value_percent: -1,
                            max_percent: 100,
                            is_slider: false,
                            is_active: false,
                            is_muted: false,
                        }
                    })
                    .collect();
                (
                    "KnobContext Hub".to_string(),
                    rows,
                    "Nenhum módulo habilitado no Hub.".to_string(),
                    "Girar: Navegar • Knob/K3: Abrir • K1: Fechar".to_string(),
                )
            }
            HubScreen::Profiles => {
                let mut rows = Vec::with_capacity(config.profiles.len() + 1);
                let is_auto = matches!(mode, ProfileMode::Auto);
                rows.push(HubRowView {
                    title: "Automático".to_string(),
                    subtitle: format!("Segue janela ativa ({contextual_profile})"),
                    badge: if is_auto { "ATIVO" } else { "" }.to_string(),
                    value_percent: -1,
                    max_percent: 100,
                    is_slider: false,
                    is_active: is_auto,
                    is_muted: false,
                });
                for profile in &config.profiles {
                    let active = matches!(mode, ProfileMode::Manual(m) if m.eq_ignore_ascii_case(&profile.name));
                    let rule_desc = match (&profile.window_class, &profile.window_title) {
                        (Some(c), Some(t)) => format!("{c} • {t}"),
                        (Some(c), None) => c.clone(),
                        (None, Some(t)) => t.clone(),
                        (None, None) => "Perfil manual / global".to_string(),
                    };
                    rows.push(HubRowView {
                        title: profile.name.clone(),
                        subtitle: rule_desc,
                        badge: if active { "ATIVO" } else { "" }.to_string(),
                        value_percent: -1,
                        max_percent: 100,
                        is_slider: false,
                        is_active: active,
                        is_muted: false,
                    });
                }
                (
                    "Hub › Perfis".to_string(),
                    rows,
                    "Nenhum perfil configurado.".to_string(),
                    "Girar: Escolher • Knob/K3: Selecionar • K1: Voltar".to_string(),
                )
            }
            HubScreen::AudioMixer => {
                let max_vol = config.overlay.max_volume_percent.max(100);
                let rows: Vec<HubRowView> = self
                    .audio_mixer
                    .streams
                    .iter()
                    .map(|s| HubRowView {
                        title: s.title.clone(),
                        subtitle: s.subtitle.clone(),
                        badge: if s.muted {
                            format!("MUTE ({}%)", s.volume_percent)
                        } else {
                            format!("{}%", s.volume_percent)
                        },
                        value_percent: s.volume_percent,
                        max_percent: max_vol,
                        is_slider: true,
                        is_active: s.is_master,
                        is_muted: s.muted,
                    })
                    .collect();
                let hint = if self.interaction == InteractionMode::Adjusting {
                    "Girar: Volume • Knob/K3/K1: Soltar • K2: Mudo".to_string()
                } else {
                    "Girar: App • Knob/K3: Ajustar • K2: Mudo • K1: Voltar".to_string()
                };
                (
                    "Hub › Sons & Apps".to_string(),
                    rows,
                    "Nenhum dispositivo ou aplicativo de áudio ativo.".to_string(),
                    hint,
                )
            }
            HubScreen::AudioOutputs => {
                let rows: Vec<HubRowView> = self
                    .audio_outputs
                    .outputs
                    .iter()
                    .map(|o| HubRowView {
                        title: o.description.clone(),
                        subtitle: if o.muted {
                            format!("Mudo • {}%", o.volume_percent)
                        } else {
                            format!("Volume {}%", o.volume_percent)
                        },
                        badge: if o.is_default { "ATIVO" } else { "" }.to_string(),
                        value_percent: -1,
                        max_percent: 100,
                        is_slider: false,
                        is_active: o.is_default,
                        is_muted: o.muted,
                    })
                    .collect();
                (
                    "Hub › Saída de Som".to_string(),
                    rows,
                    "Nenhuma saída de áudio encontrada.".to_string(),
                    "Girar: Saída • Knob/K3: Ativar • K1: Voltar".to_string(),
                )
            }
            HubScreen::CustomGroup { item_id, title } => {
                let rows: Vec<HubRowView> = config
                    .overlay
                    .items
                    .iter()
                    .find(|i| i.id == *item_id)
                    .and_then(|i| match &i.kind {
                        OverlayItemKind::CustomGroup { entries } => Some(
                            entries
                                .iter()
                                .map(|e| HubRowView {
                                    title: e.label.clone(),
                                    subtitle: e.action.to_display_string(),
                                    badge: "▶".to_string(),
                                    value_percent: -1,
                                    max_percent: 100,
                                    is_slider: false,
                                    is_active: false,
                                    is_muted: false,
                                })
                                .collect(),
                        ),
                        _ => None,
                    })
                    .unwrap_or_default();
                (
                    format!("Hub › {title}"),
                    rows,
                    "Grupo vazio. Adicione ações nas Configurações.".to_string(),
                    "Girar: Escolher • Knob/K3: Executar • K1: Voltar".to_string(),
                )
            }
            HubScreen::CustomSlider { item_id, title } => {
                let val = self
                    .custom_slider_values
                    .get(item_id)
                    .copied()
                    .unwrap_or(50);
                let rows = vec![HubRowView {
                    title: title.clone(),
                    subtitle: "Gire o knob para ajustar".to_string(),
                    badge: format!("{val}%"),
                    value_percent: val,
                    max_percent: 100,
                    is_slider: true,
                    is_active: true,
                    is_muted: false,
                }];
                let hint = if self.interaction == InteractionMode::Adjusting {
                    "Girar: Ajustar • Knob/K3/K1: Soltar • K2: Toggle".to_string()
                } else {
                    "Knob/K3: Ajustar • K2: Toggle • K1: Voltar".to_string()
                };
                (format!("Hub › {title}"), rows, String::new(), hint)
            }
            HubScreen::Workspaces => (
                "Hub › Workspaces".to_string(),
                Vec::new(),
                "Módulo Workspaces será ativado na Fase C.".to_string(),
                "K1: Voltar".to_string(),
            ),
            HubScreen::Brightness => (
                "Hub › Brilho".to_string(),
                Vec::new(),
                "Módulo Brilho será ativado na Fase C.".to_string(),
                "K1: Voltar".to_string(),
            ),
            HubScreen::CaelestiaWallpapers => (
                "Hub › Wallpapers".to_string(),
                Vec::new(),
                "Módulo Wallpapers será ativado na Fase C.".to_string(),
                "K1: Voltar".to_string(),
            ),
            HubScreen::CaelestiaToggles => (
                "Hub › Caelestia".to_string(),
                Vec::new(),
                "Módulo Caelestia será ativado na Fase C.".to_string(),
                "K1: Voltar".to_string(),
            ),
        };

        let cursor = match self.screen {
            HubScreen::Root => self.root_cursor,
            _ => self.sub_cursor,
        };
        let total = all_rows.len();
        let (rows, highlighted_visible_index, scroll_indicator) =
            slice_visible_window(all_rows, cursor, VISIBLE_HUB_ROWS);

        HubViewModel {
            breadcrumb,
            context_badge,
            adjusting: self.interaction == InteractionMode::Adjusting,
            loading: self.loading && total == 0,
            empty_message,
            rows,
            highlighted_visible_index,
            scroll_indicator,
            footer_hint,
        }
    }
}

fn slice_visible_window(
    items: Vec<HubRowView>,
    cursor: usize,
    window_size: usize,
) -> (Vec<HubRowView>, usize, String) {
    let total = items.len();
    if total == 0 {
        return (Vec::new(), 0, String::new());
    }
    let clamped = cursor.min(total - 1);
    if total <= window_size {
        return (items, clamped, format!("{} / {total}", clamped + 1));
    }
    let half = window_size / 2;
    let start = if clamped <= half {
        0
    } else if clamped + (window_size - half) >= total {
        total - window_size
    } else {
        clamped - half
    };
    let end = (start + window_size).min(total);
    let visible = items.into_iter().skip(start).take(end - start).collect();
    (
        visible,
        clamped - start,
        format!("{} / {total}", clamped + 1),
    )
}

#[cfg(test)]
mod tests {
    use super::{HubController, HubEffect};
    use crate::config::Config;
    use crate::context::ProfileMode;
    use crate::input::KnobEvent;
    use crate::system::audio::{AudioMixerSnapshot, AudioStreamItem};
    use crate::system::{SystemRequest, SystemResponse};
    use std::time::{Duration, Instant};

    #[test]
    fn full_audio_mixer_navigation_lock_adjust_mute_unlock_back_flow() {
        let cfg = Config::default();
        let mode = ProfileMode::Auto;
        let mut hub = HubController::new();
        let t0 = Instant::now();

        // 1. Modo normal despacha eventos
        assert_eq!(
            hub.handle_event(KnobEvent::RotateCw, t0, &cfg, &mode, "global"),
            vec![HubEffect::DispatchNormal(KnobEvent::RotateCw)]
        );

        // 2. KnobPress abre o Hub Raiz
        let fx = hub.handle_event(KnobEvent::KnobPress, t0, &cfg, &mode, "firefox");
        assert!(hub.is_open());
        assert!(
            matches!(fx.first(), Some(HubEffect::RenderOverlay(vm)) if vm.breadcrumb == "KnobContext Hub")
        );

        // 3. Gira 1x (Perfis -> Sons & Apps) e confirma com Key3
        let _ = hub.handle_event(
            KnobEvent::RotateCw,
            t0 + Duration::from_millis(100),
            &cfg,
            &mode,
            "firefox",
        );
        let fx = hub.handle_event(
            KnobEvent::Key3,
            t0 + Duration::from_millis(200),
            &cfg,
            &mode,
            "firefox",
        );
        assert!(fx.contains(&HubEffect::System(SystemRequest::FetchAudioMixer)));

        // 4. Snapshot chega do SystemWorker (Master + Firefox)
        let snap = AudioMixerSnapshot {
            streams: vec![
                AudioStreamItem {
                    id: "master:hdmi".into(),
                    is_master: true,
                    sink_name: Some("hdmi".into()),
                    sink_input_index: None,
                    title: "Volume Global".into(),
                    subtitle: "HDMI".into(),
                    volume_percent: 80,
                    muted: false,
                },
                AudioStreamItem {
                    id: "app:42".into(),
                    is_master: false,
                    sink_name: None,
                    sink_input_index: Some(42),
                    title: "Firefox".into(),
                    subtitle: "YouTube".into(),
                    volume_percent: 60,
                    muted: false,
                },
            ],
        };
        assert!(hub
            .apply_system_response(SystemResponse::AudioMixer(snap), &cfg, &mode, "firefox")
            .is_some());

        // 5. Gira até Firefox e pressiona KnobPress para entrar em Adjusting
        let _ = hub.handle_event(
            KnobEvent::RotateCw,
            t0 + Duration::from_millis(300),
            &cfg,
            &mode,
            "firefox",
        );
        let fx = hub.handle_event(
            KnobEvent::KnobPress,
            t0 + Duration::from_millis(400),
            &cfg,
            &mode,
            "firefox",
        );
        assert!(matches!(fx.first(), Some(HubEffect::RenderOverlay(vm)) if vm.adjusting));

        // 6. Gira no modo Adjusting -> altera volume do Firefox de 60% -> 65%
        let fx = hub.handle_event(
            KnobEvent::RotateCw,
            t0 + Duration::from_millis(500),
            &cfg,
            &mode,
            "firefox",
        );
        assert!(fx.contains(&HubEffect::System(SystemRequest::SetAppVolume {
            index: 42,
            percent: 65,
            max_percent: 150,
        })));

        // 7. Key2 alterna Mute do Firefox
        let fx = hub.handle_event(
            KnobEvent::Key2,
            t0 + Duration::from_millis(600),
            &cfg,
            &mode,
            "firefox",
        );
        assert!(
            fx.contains(&HubEffect::System(SystemRequest::ToggleAppMute {
                index: 42,
            }))
        );

        // 8. KnobPress solta Adjusting -> volta para navegação no Mixer
        let fx = hub.handle_event(
            KnobEvent::KnobPress,
            t0 + Duration::from_millis(700),
            &cfg,
            &mode,
            "firefox",
        );
        assert!(matches!(fx.first(), Some(HubEffect::RenderOverlay(vm)) if !vm.adjusting));

        // 9. Key1 volta ao Hub Raiz; Key1 novamente fecha o overlay
        let fx = hub.handle_event(
            KnobEvent::Key1,
            t0 + Duration::from_millis(800),
            &cfg,
            &mode,
            "firefox",
        );
        assert!(
            matches!(fx.first(), Some(HubEffect::RenderOverlay(vm)) if vm.breadcrumb == "KnobContext Hub")
        );

        let fx = hub.handle_event(
            KnobEvent::Key1,
            t0 + Duration::from_millis(900),
            &cfg,
            &mode,
            "firefox",
        );
        assert_eq!(fx, vec![HubEffect::CloseOverlay]);
        assert!(!hub.is_open());
    }

    #[test]
    fn timeout_is_longer_in_adjusting_mode() {
        let cfg = Config::default();
        let mode = ProfileMode::Auto;
        let mut hub = HubController::new();
        let t0 = Instant::now();

        let _ = hub.handle_event(KnobEvent::KnobPress, t0, &cfg, &mode, "global");
        // Em Browsing (default 5s): 4s não fecha, 5s fecha
        assert_eq!(
            hub.check_timeout(t0 + Duration::from_secs(4), &cfg.overlay),
            None
        );
        assert_eq!(
            hub.check_timeout(t0 + Duration::from_secs(5), &cfg.overlay),
            Some(HubEffect::CloseOverlay)
        );
        assert!(!hub.is_open());
    }
}
