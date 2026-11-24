use crate::actions_catalog::{list_desktop_apps, DesktopApp};
use crate::config::{
    ActionConfig, Config, OverlayConfig, OverlayItemConfig, OverlayItemKind, OverlayKey3Behavior,
};
use crate::context::ProfileMode;
use crate::input::KnobEvent;
use crate::system::audio::{AudioMixerSnapshot, AudioOutputsSnapshot};
use crate::system::brightness::BrightnessSnapshot;
use crate::system::caelestia::{CaelestiaTogglesSnapshot, WallpapersSnapshot};
use crate::system::hyprland::WorkspacesSnapshot;
use crate::system::icons::resolve_app_icon_path;
use crate::system::thumbs::{ThumbnailMode, ThumbnailRequest};
use crate::system::{SystemRequest, SystemResponse};
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub const CAROUSEL_SLOTS: usize = 7;
pub const THUMB_COVER_WIDTH: u32 = 240;
pub const THUMB_COVER_HEIGHT: u32 = 140;
pub const THUMB_ICON_SIZE: u32 = 96;

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
pub struct HubCardView {
    pub is_empty_slot: bool,
    pub title: String,
    pub subtitle: String,
    pub badge: String,
    pub glyph: String,
    pub image_path: Option<String>,
    pub image_is_cover: bool,
    pub value_percent: i32,
    pub max_percent: i32,
    pub is_slider: bool,
    pub is_active: bool,
    pub is_muted: bool,
}

impl HubCardView {
    pub fn empty() -> Self {
        Self {
            is_empty_slot: true,
            title: String::new(),
            subtitle: String::new(),
            badge: String::new(),
            glyph: String::new(),
            image_path: None,
            image_is_cover: false,
            value_percent: -1,
            max_percent: 100,
            is_slider: false,
            is_active: false,
            is_muted: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubViewModel {
    pub parent_pill: String,
    pub active_pill: String,
    pub status_pill: String,
    pub adjusting: bool,
    pub loading: bool,
    pub empty_message: String,
    /// Exatamente 7 slots do carrossel horizontal, com o item selecionado fixo no centro (`3`)
    /// e os slots `0` e `6` atuando como bordas de entrada/saída durante o deslize suave.
    pub cards: Vec<HubCardView>,
    /// Lista de thumbnails extras (vizinhos imediatos) para pre-fetch suave em background.
    pub prefetch_requests: Vec<ThumbnailRequest>,
    pub scroll_indicator: String,
    pub footer_hint: String,
    /// Deslocamento discreto do último giro de navegação (`+1`, `-1` ou `0`) para o animador.
    pub scroll_delta: i32,
    /// `true` quando ocorreu transição de tela (Hub ↔ submenu) para animar a troca.
    pub screen_changed: bool,
    pub animations_enabled: bool,
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

/// Máquina de estados hierárquica do Hub Interativo em Carrossel Horizontal.
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
    workspaces: WorkspacesSnapshot,
    brightness: BrightnessSnapshot,
    wallpapers: WallpapersSnapshot,
    caelestia_toggles: CaelestiaTogglesSnapshot,
    custom_slider_values: HashMap<String, i32>,
    desktop_apps: Vec<DesktopApp>,
    icon_path_cache: HashMap<String, Option<String>>,
}

impl HubController {
    pub fn new() -> Self {
        Self::with_desktop_apps(list_desktop_apps())
    }

    pub fn with_desktop_apps(desktop_apps: Vec<DesktopApp>) -> Self {
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
            workspaces: WorkspacesSnapshot::default(),
            brightness: BrightnessSnapshot::default(),
            wallpapers: WallpapersSnapshot::default(),
            caelestia_toggles: CaelestiaTogglesSnapshot::default(),
            custom_slider_values: HashMap::new(),
            desktop_apps,
            icon_path_cache: HashMap::new(),
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

    fn resolve_icon_cached(&mut self, hint: &str) -> Option<String> {
        let key = hint.trim().to_ascii_lowercase();
        if key.is_empty() {
            return None;
        }
        if let Some(cached) = self.icon_path_cache.get(&key) {
            return cached.clone();
        }
        let resolved = resolve_app_icon_path(hint, &self.desktop_apps)
            .map(|p| p.to_string_lossy().into_owned());
        self.icon_path_cache.insert(key, resolved.clone());
        resolved
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
                    self.clamp_sub_cursor(self.audio_mixer.streams.len());
                    true
                } else {
                    false
                }
            }
            SystemResponse::AudioOutputs(snap) => {
                self.audio_outputs = snap;
                if matches!(self.screen, HubScreen::AudioOutputs) {
                    self.loading = false;
                    self.clamp_sub_cursor(self.audio_outputs.outputs.len());
                    true
                } else {
                    false
                }
            }
            SystemResponse::Workspaces(snap) => {
                let first_load = self.loading && matches!(self.screen, HubScreen::Workspaces);
                self.workspaces = snap;
                if matches!(self.screen, HubScreen::Workspaces) {
                    self.loading = false;
                    if first_load {
                        if let Some(active_pos) =
                            self.workspaces.workspaces.iter().position(|w| w.is_active)
                        {
                            self.sub_cursor = active_pos;
                        }
                    }
                    self.clamp_sub_cursor(self.workspaces.workspaces.len());
                    true
                } else {
                    false
                }
            }
            SystemResponse::Brightness(snap) => {
                self.brightness = snap;
                if matches!(self.screen, HubScreen::Brightness) {
                    self.loading = false;
                    self.clamp_sub_cursor(self.brightness.monitors.len());
                    true
                } else {
                    false
                }
            }
            SystemResponse::Wallpapers(snap) => {
                let first_load =
                    self.loading && matches!(self.screen, HubScreen::CaelestiaWallpapers);
                self.wallpapers = snap;
                if matches!(self.screen, HubScreen::CaelestiaWallpapers) {
                    self.loading = false;
                    if first_load {
                        if let Some(cur_pos) =
                            self.wallpapers.items.iter().position(|w| w.is_current)
                        {
                            self.sub_cursor = cur_pos;
                        }
                    }
                    self.clamp_sub_cursor(self.wallpapers.items.len());
                    true
                } else {
                    false
                }
            }
            SystemResponse::CaelestiaToggles(snap) => {
                self.caelestia_toggles = snap;
                if matches!(self.screen, HubScreen::CaelestiaToggles) {
                    self.loading = false;
                    self.clamp_sub_cursor(self.caelestia_toggles.items.len());
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
            SystemResponse::ThumbnailReady(_) => false,
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

    fn clamp_sub_cursor(&mut self, len: usize) {
        if len == 0 {
            self.sub_cursor = 0;
            self.interaction = InteractionMode::Browsing;
        } else {
            self.sub_cursor = self.sub_cursor.min(len - 1);
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
            let mut vm = self.build_view_model(config, mode, contextual_profile);
            vm.screen_changed = true;
            return vec![HubEffect::RenderOverlay(vm)];
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

        let mut vm = self.build_view_model(config, mode, contextual_profile);
        if count > 1 {
            vm.scroll_delta = dir.signum();
        }
        vec![HubEffect::RenderOverlay(vm)]
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
            HubScreen::Brightness => {
                let step = config.overlay.brightness_step.max(1);
                if let Some(mon) = self.brightness.monitors.get_mut(self.sub_cursor) {
                    let next = (mon.percent + dir * step).clamp(0, 100);
                    let is_all = mon.is_all;
                    let monitor_name = mon.monitor_name.clone();
                    mon.percent = next;
                    if is_all {
                        for other in &mut self.brightness.monitors {
                            other.percent = next;
                        }
                    }
                    effects.push(HubEffect::System(SystemRequest::SetBrightness {
                        monitor_name,
                        percent: next,
                    }));
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
            HubScreen::Brightness => {
                if let Some(pct) = self
                    .brightness
                    .monitors
                    .get(self.sub_cursor)
                    .map(|m| m.percent)
                {
                    for mon in &mut self.brightness.monitors {
                        mon.percent = pct;
                    }
                    effects.push(HubEffect::System(SystemRequest::SetBrightness {
                        monitor_name: None,
                        percent: pct,
                    }));
                    effects.push(HubEffect::RenderOverlay(self.build_view_model(
                        config,
                        mode,
                        contextual_profile,
                    )));
                }
            }
            HubScreen::Workspaces => {
                effects.push(HubEffect::System(SystemRequest::ToggleSpecialWorkspace));
            }
            HubScreen::CaelestiaWallpapers => {
                effects.push(HubEffect::System(SystemRequest::SetRandomWallpaper));
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
            HubScreen::Workspaces => {
                if let Some(ws) = self.workspaces.workspaces.get(self.sub_cursor).cloned() {
                    for item in &mut self.workspaces.workspaces {
                        item.is_active = item.id == ws.id;
                    }
                    vec![
                        HubEffect::System(SystemRequest::SwitchWorkspace { id: ws.id }),
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
            HubScreen::Brightness => {
                if self.brightness.monitors.get(self.sub_cursor).is_some() {
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
            HubScreen::CaelestiaWallpapers => {
                let Some(wp) = self.wallpapers.items.get(self.sub_cursor).cloned() else {
                    return Vec::new();
                };
                if wp.is_random {
                    vec![HubEffect::System(SystemRequest::SetRandomWallpaper)]
                } else if let Some(path) = wp.path {
                    for item in &mut self.wallpapers.items {
                        item.is_current = item.path.as_deref() == Some(path.as_str());
                    }
                    vec![
                        HubEffect::System(SystemRequest::SetWallpaper { path }),
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
            HubScreen::CaelestiaToggles => {
                if let Some(toggle) = self.caelestia_toggles.items.get_mut(self.sub_cursor) {
                    let kind = toggle.kind;
                    if let Some(state) = toggle.enabled.as_mut() {
                        *state = !*state;
                    }
                    vec![
                        HubEffect::System(SystemRequest::TriggerCaelestiaToggle { kind }),
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
                self.loading = true;
                effects.push(HubEffect::System(SystemRequest::FetchWorkspaces));
            }
            OverlayItemKind::Brightness => {
                self.screen = HubScreen::Brightness;
                self.loading = true;
                effects.push(HubEffect::System(SystemRequest::FetchBrightness));
            }
            OverlayItemKind::CaelestiaWallpapers => {
                self.screen = HubScreen::CaelestiaWallpapers;
                self.loading = true;
                effects.push(HubEffect::System(SystemRequest::FetchWallpapers));
            }
            OverlayItemKind::CaelestiaToggles => {
                self.screen = HubScreen::CaelestiaToggles;
                self.loading = true;
                effects.push(HubEffect::System(SystemRequest::FetchCaelestiaToggles));
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

        let mut vm = self.build_view_model(config, mode, contextual_profile);
        vm.screen_changed = true;
        effects.push(HubEffect::RenderOverlay(vm));
        effects
    }

    fn current_item_count(&self, config: &Config) -> usize {
        match &self.screen {
            HubScreen::Root => self.enabled_root_items(&config.overlay).len(),
            HubScreen::Profiles => config.profiles.len() + 1,
            HubScreen::AudioMixer => self.audio_mixer.streams.len(),
            HubScreen::AudioOutputs => self.audio_outputs.outputs.len(),
            HubScreen::Workspaces => self.workspaces.workspaces.len(),
            HubScreen::Brightness => self.brightness.monitors.len(),
            HubScreen::CaelestiaWallpapers => self.wallpapers.items.len(),
            HubScreen::CaelestiaToggles => self.caelestia_toggles.items.len(),
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
        }
    }

    pub fn build_view_model(
        &mut self,
        config: &Config,
        mode: &ProfileMode,
        contextual_profile: &str,
    ) -> HubViewModel {
        let show_images = config.overlay.show_images;
        let context_badge = match mode {
            ProfileMode::Auto => format!("Auto • {contextual_profile}"),
            ProfileMode::Manual(name) => format!("Manual • {name}"),
        };

        let (parent_pill, active_pill, all_cards, empty_message, footer_hint) = match self
            .screen
            .clone()
        {
            HubScreen::Root => {
                let items: Vec<OverlayItemConfig> = self
                    .enabled_root_items(&config.overlay)
                    .into_iter()
                    .cloned()
                    .collect();
                let mut cards = Vec::with_capacity(items.len());
                for item in items {
                    let (subtitle, glyph, auto_icon_hint) = match &item.kind {
                        OverlayItemKind::Profiles => (context_badge.clone(), "◈", None),
                        OverlayItemKind::AudioMixer => ("Global & Apps".to_string(), "♫", None),
                        OverlayItemKind::AudioOutputs => ("Dispositivos".to_string(), "◎", None),
                        OverlayItemKind::Workspaces => ("Hyprland".to_string(), "⊞", None),
                        OverlayItemKind::Brightness => ("Monitores".to_string(), "☀", None),
                        OverlayItemKind::CaelestiaWallpapers => {
                            ("Caelestia".to_string(), "▣", None)
                        }
                        OverlayItemKind::CaelestiaToggles => ("Sistema".to_string(), "⚡", None),
                        OverlayItemKind::QuickAction { action } => {
                            let hint = match action {
                                ActionConfig::Launch { desktop_file } => Some(desktop_file.clone()),
                                _ => None,
                            };
                            (action.to_display_string(), "▶", hint)
                        }
                        OverlayItemKind::CustomGroup { entries } => {
                            (format!("{} ações", entries.len()), "☰", None)
                        }
                        OverlayItemKind::CustomSlider { .. } => ("Slider".to_string(), "━", None),
                    };
                    let image_path = if show_images {
                        item.icon
                            .as_deref()
                            .and_then(|p| self.resolve_icon_cached(p))
                            .or_else(|| {
                                auto_icon_hint
                                    .as_deref()
                                    .and_then(|h| self.resolve_icon_cached(h))
                            })
                    } else {
                        None
                    };
                    let image_is_cover = item.icon.is_some() && image_path.is_some();
                    cards.push(HubCardView {
                        is_empty_slot: false,
                        title: item.title,
                        subtitle,
                        badge: String::new(),
                        glyph: glyph.to_string(),
                        image_path,
                        image_is_cover,
                        value_percent: -1,
                        max_percent: 100,
                        is_slider: false,
                        is_active: false,
                        is_muted: false,
                    });
                }
                (
                    context_badge.clone(),
                    "KnobContext".to_string(),
                    cards,
                    "Nenhum módulo habilitado no Hub.".to_string(),
                    "Girar: Navegar • Knob/K3: Abrir • K1: Fechar".to_string(),
                )
            }
            HubScreen::Profiles => {
                let mut cards = Vec::with_capacity(config.profiles.len() + 1);
                let is_auto = matches!(mode, ProfileMode::Auto);
                cards.push(HubCardView {
                    is_empty_slot: false,
                    title: "Automático".to_string(),
                    subtitle: contextual_profile.to_string(),
                    badge: if is_auto { "ATIVO" } else { "" }.to_string(),
                    glyph: "✦".to_string(),
                    image_path: None,
                    image_is_cover: false,
                    value_percent: -1,
                    max_percent: 100,
                    is_slider: false,
                    is_active: is_auto,
                    is_muted: false,
                });
                for profile in &config.profiles {
                    let active = matches!(mode, ProfileMode::Manual(m) if m.eq_ignore_ascii_case(&profile.name));
                    let first_class = profile
                        .window_class
                        .as_deref()
                        .and_then(|c| c.split(['|', ',']).next())
                        .map(str::trim)
                        .filter(|s| !s.is_empty());
                    let image_path = if show_images {
                        first_class.and_then(|cls| self.resolve_icon_cached(cls))
                    } else {
                        None
                    };
                    let rule_desc = match (&profile.window_class, &profile.window_title) {
                        (Some(c), Some(t)) => format!("{c} • {t}"),
                        (Some(c), None) => c.clone(),
                        (None, Some(t)) => t.clone(),
                        (None, None) => "Global / manual".to_string(),
                    };
                    cards.push(HubCardView {
                        is_empty_slot: false,
                        title: profile.name.clone(),
                        subtitle: rule_desc,
                        badge: if active { "ATIVO" } else { "" }.to_string(),
                        glyph: "◈".to_string(),
                        image_path,
                        image_is_cover: false,
                        value_percent: -1,
                        max_percent: 100,
                        is_slider: false,
                        is_active: active,
                        is_muted: false,
                    });
                }
                (
                    "Hub".to_string(),
                    "Perfis".to_string(),
                    cards,
                    "Nenhum perfil configurado.".to_string(),
                    "Girar: Escolher • Knob/K3: Ativar • K1: Voltar".to_string(),
                )
            }
            HubScreen::AudioMixer => {
                let max_vol = config.overlay.max_volume_percent.max(100);
                let streams = self.audio_mixer.streams.clone();
                let mut cards = Vec::with_capacity(streams.len());
                for s in streams {
                    let image_path = if show_images && !s.is_master {
                        self.resolve_icon_cached(&s.title)
                    } else {
                        None
                    };
                    cards.push(HubCardView {
                        is_empty_slot: false,
                        title: s.title,
                        subtitle: s.subtitle,
                        badge: if s.muted {
                            "MUTE".to_string()
                        } else {
                            format!("{}%", s.volume_percent)
                        },
                        glyph: if s.is_master { "♫" } else { "♪" }.to_string(),
                        image_path,
                        image_is_cover: false,
                        value_percent: s.volume_percent,
                        max_percent: max_vol,
                        is_slider: true,
                        is_active: s.is_master,
                        is_muted: s.muted,
                    });
                }
                let hint = if self.interaction == InteractionMode::Adjusting {
                    "Girar: Volume • Knob/K3/K1: Soltar • K2: Mudo".to_string()
                } else {
                    "Girar: App • Knob/K3: Ajustar • K2: Mudo • K1: Voltar".to_string()
                };
                (
                    "Hub".to_string(),
                    "Sons & Apps".to_string(),
                    cards,
                    "Nenhum canal de áudio ativo.".to_string(),
                    hint,
                )
            }
            HubScreen::AudioOutputs => {
                let cards: Vec<HubCardView> = self
                    .audio_outputs
                    .outputs
                    .iter()
                    .map(|o| HubCardView {
                        is_empty_slot: false,
                        title: o.description.clone(),
                        subtitle: if o.muted {
                            format!("Mudo • {}%", o.volume_percent)
                        } else {
                            format!("{}%", o.volume_percent)
                        },
                        badge: if o.is_default { "ATIVO" } else { "" }.to_string(),
                        glyph: "◎".to_string(),
                        image_path: None,
                        image_is_cover: false,
                        value_percent: -1,
                        max_percent: 100,
                        is_slider: false,
                        is_active: o.is_default,
                        is_muted: o.muted,
                    })
                    .collect();
                (
                    "Hub".to_string(),
                    "Saída de Som".to_string(),
                    cards,
                    "Nenhuma saída de áudio encontrada.".to_string(),
                    "Girar: Saída • Knob/K3: Ativar • K1: Voltar".to_string(),
                )
            }
            HubScreen::Workspaces => {
                let cards: Vec<HubCardView> = self
                    .workspaces
                    .workspaces
                    .iter()
                    .map(|w| {
                        let subtitle = if w.is_new_slot {
                            "Livre".to_string()
                        } else {
                            format!("{} • {} jan.", w.monitor, w.windows)
                        };
                        let title = if w.is_new_slot {
                            format!("+ WS {}", w.id)
                        } else {
                            format!("Workspace {}", w.name)
                        };
                        HubCardView {
                            is_empty_slot: false,
                            title,
                            subtitle,
                            badge: if w.is_active { "ATIVO" } else { "" }.to_string(),
                            glyph: if w.is_new_slot {
                                "+".to_string()
                            } else {
                                w.name.clone()
                            },
                            image_path: None,
                            image_is_cover: false,
                            value_percent: -1,
                            max_percent: 100,
                            is_slider: false,
                            is_active: w.is_active,
                            is_muted: false,
                        }
                    })
                    .collect();
                (
                    "Hub".to_string(),
                    "Workspaces".to_string(),
                    cards,
                    "Hyprland IPC indisponível.".to_string(),
                    "Girar: WS • Knob/K3: Ir • K2: Special • K1: Voltar".to_string(),
                )
            }
            HubScreen::Brightness => {
                let cards: Vec<HubCardView> = self
                    .brightness
                    .monitors
                    .iter()
                    .map(|m| HubCardView {
                        is_empty_slot: false,
                        title: m.title.clone(),
                        subtitle: m.subtitle.clone(),
                        badge: format!("{}%", m.percent),
                        glyph: "☀".to_string(),
                        image_path: None,
                        image_is_cover: false,
                        value_percent: m.percent,
                        max_percent: 100,
                        is_slider: true,
                        is_active: m.is_all,
                        is_muted: false,
                    })
                    .collect();
                let hint = if self.interaction == InteractionMode::Adjusting {
                    "Girar: Brilho • Knob/K3/K1: Soltar • K2: Sincronizar".to_string()
                } else {
                    "Girar: Monitor • Knob/K3: Ajustar • K2: Sincronizar • K1: Voltar".to_string()
                };
                (
                    "Hub".to_string(),
                    "Brilho".to_string(),
                    cards,
                    "Nenhum monitor/backlight disponível.".to_string(),
                    hint,
                )
            }
            HubScreen::CaelestiaWallpapers => {
                let cards: Vec<HubCardView> = self
                    .wallpapers
                    .items
                    .iter()
                    .map(|w| HubCardView {
                        is_empty_slot: false,
                        title: w.title.clone(),
                        subtitle: w.category.clone(),
                        badge: if w.is_current {
                            "ATUAL".to_string()
                        } else {
                            String::new()
                        },
                        glyph: if w.is_random { "🎲" } else { "▣" }.to_string(),
                        image_path: if show_images { w.path.clone() } else { None },
                        image_is_cover: true,
                        value_percent: -1,
                        max_percent: 100,
                        is_slider: false,
                        is_active: w.is_current,
                        is_muted: false,
                    })
                    .collect();
                (
                    "Hub".to_string(),
                    "Wallpapers".to_string(),
                    cards,
                    "Nenhum wallpaper encontrado via Caelestia IPC.".to_string(),
                    "Girar: Lista • Knob/K3: Aplicar • K2: Aleatório • K1: Voltar".to_string(),
                )
            }
            HubScreen::CaelestiaToggles => {
                let cards: Vec<HubCardView> = self
                    .caelestia_toggles
                    .items
                    .iter()
                    .map(|t| {
                        let badge = match t.enabled {
                            Some(true) => "ON",
                            Some(false) => "OFF",
                            None => "▶",
                        };
                        HubCardView {
                            is_empty_slot: false,
                            title: t.title.clone(),
                            subtitle: t.subtitle.clone(),
                            badge: badge.to_string(),
                            glyph: "⚡".to_string(),
                            image_path: None,
                            image_is_cover: false,
                            value_percent: -1,
                            max_percent: 100,
                            is_slider: false,
                            is_active: t.enabled == Some(true),
                            is_muted: false,
                        }
                    })
                    .collect();
                (
                    "Hub".to_string(),
                    "Caelestia".to_string(),
                    cards,
                    "Caelestia Shell IPC indisponível.".to_string(),
                    "Girar: Controle • Knob/K3: Alternar • K1: Voltar".to_string(),
                )
            }
            HubScreen::CustomGroup { item_id, title } => {
                let entries = config
                    .overlay
                    .items
                    .iter()
                    .find(|i| i.id == item_id)
                    .and_then(|i| match &i.kind {
                        OverlayItemKind::CustomGroup { entries } => Some(entries.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                let mut cards = Vec::with_capacity(entries.len());
                for e in entries {
                    let auto_hint = match &e.action {
                        ActionConfig::Launch { desktop_file } => Some(desktop_file.clone()),
                        _ => None,
                    };
                    let image_path = if show_images {
                        e.icon
                            .as_deref()
                            .and_then(|p| self.resolve_icon_cached(p))
                            .or_else(|| {
                                auto_hint
                                    .as_deref()
                                    .and_then(|h| self.resolve_icon_cached(h))
                            })
                    } else {
                        None
                    };
                    let image_is_cover = e.icon.is_some() && image_path.is_some();
                    cards.push(HubCardView {
                        is_empty_slot: false,
                        title: e.label,
                        subtitle: e.action.to_display_string(),
                        badge: "▶".to_string(),
                        glyph: "▶".to_string(),
                        image_path,
                        image_is_cover,
                        value_percent: -1,
                        max_percent: 100,
                        is_slider: false,
                        is_active: false,
                        is_muted: false,
                    });
                }
                (
                    "Hub".to_string(),
                    title,
                    cards,
                    "Grupo vazio. Adicione ações nas Configurações.".to_string(),
                    "Girar: Escolher • Knob/K3: Executar • K1: Voltar".to_string(),
                )
            }
            HubScreen::CustomSlider { item_id, title } => {
                let val = self
                    .custom_slider_values
                    .get(&item_id)
                    .copied()
                    .unwrap_or(50);
                let cards = vec![HubCardView {
                    is_empty_slot: false,
                    title: title.clone(),
                    subtitle: "Gire o knob para ajustar".to_string(),
                    badge: format!("{val}%"),
                    glyph: "━".to_string(),
                    image_path: None,
                    image_is_cover: false,
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
                ("Hub".to_string(), title, cards, String::new(), hint)
            }
        };

        let cursor = match self.screen {
            HubScreen::Root => self.root_cursor,
            _ => self.sub_cursor,
        };
        let total = all_cards.len();
        let (cards, prefetch_requests, scroll_indicator) =
            build_centered_carousel(&all_cards, cursor);

        let status_pill = if self.interaction == InteractionMode::Adjusting {
            "● AJUSTANDO".to_string()
        } else if !scroll_indicator.is_empty() {
            scroll_indicator.clone()
        } else {
            context_badge
        };

        HubViewModel {
            parent_pill,
            active_pill,
            status_pill,
            adjusting: self.interaction == InteractionMode::Adjusting,
            loading: self.loading && total == 0,
            empty_message,
            cards,
            prefetch_requests,
            scroll_indicator,
            footer_hint,
            scroll_delta: 0,
            screen_changed: false,
            animations_enabled: config.overlay.animations_enabled,
        }
    }
}

fn card_thumbnail_request(card: &HubCardView) -> Option<ThumbnailRequest> {
    let path = card.image_path.as_ref()?;
    let mode = if card.image_is_cover {
        ThumbnailMode::Cover {
            width: THUMB_COVER_WIDTH,
            height: THUMB_COVER_HEIGHT,
        }
    } else {
        ThumbnailMode::Contain {
            size: THUMB_ICON_SIZE,
        }
    };
    Some(ThumbnailRequest {
        path: path.clone(),
        mode,
    })
}

/// Constrói exatamente 7 slots horizontais com o item `cursor` fixo no centro (`index = 3`).
///
/// - Se `total >= 5`: carrossel circular contínuo (`cursor - 3 ..= cursor + 3`) + pre-fetch de `±4`.
/// - Se `total < 5`: centraliza os itens disponíveis sem duplicá-los em `1..=5`, preenchendo pontas com `HubCardView::empty()`.
pub fn build_centered_carousel(
    items: &[HubCardView],
    cursor: usize,
) -> (Vec<HubCardView>, Vec<ThumbnailRequest>, String) {
    let total = items.len();
    if total == 0 {
        return (Vec::new(), Vec::new(), String::new());
    }

    let clamped = cursor.min(total - 1);
    let indicator = format!("{} / {total}", clamped + 1);
    let mut slots = vec![HubCardView::empty(); CAROUSEL_SLOTS];
    let mut prefetch = Vec::new();

    if total >= 5 {
        for (slot_idx, offset) in (-3isize..=3isize).enumerate() {
            let idx = (clamped as isize + offset).rem_euclid(total as isize) as usize;
            slots[slot_idx] = items[idx].clone();
        }
        // Pre-fetch dos vizinhos além da janela de entrada/saída (-4 e +4)
        for offset in [-4isize, 4isize] {
            let idx = (clamped as isize + offset).rem_euclid(total as isize) as usize;
            if let Some(req) = card_thumbnail_request(&items[idx]) {
                prefetch.push(req);
            }
        }
    } else {
        // Sem duplicação: item selecionado no slot 3 (centro); anteriores em 2, 1; próximos em 4, 5
        slots[3] = items[clamped].clone();
        if clamped >= 1 {
            slots[2] = items[clamped - 1].clone();
        }
        if clamped >= 2 {
            slots[1] = items[clamped - 2].clone();
        }
        if clamped + 1 < total {
            slots[4] = items[clamped + 1].clone();
        }
        if clamped + 2 < total {
            slots[5] = items[clamped + 2].clone();
        }
    }

    (slots, prefetch, indicator)
}

#[cfg(test)]
mod tests {
    use super::{build_centered_carousel, HubCardView, HubController, HubEffect};
    use crate::config::Config;
    use crate::context::ProfileMode;
    use crate::input::KnobEvent;
    use crate::system::audio::{AudioMixerSnapshot, AudioStreamItem};
    use crate::system::brightness::{BrightnessSnapshot, MonitorBrightnessItem};
    use crate::system::caelestia::{WallpaperItem, WallpapersSnapshot};
    use crate::system::{SystemRequest, SystemResponse};
    use std::time::{Duration, Instant};

    fn dummy_card(name: &str) -> HubCardView {
        HubCardView {
            is_empty_slot: false,
            title: name.to_string(),
            subtitle: String::new(),
            badge: String::new(),
            glyph: String::new(),
            image_path: None,
            image_is_cover: false,
            value_percent: -1,
            max_percent: 100,
            is_slider: false,
            is_active: false,
            is_muted: false,
        }
    }

    #[test]
    fn centered_carousel_handles_small_and_circular_lists() {
        // 3 itens com cursor no meio (1): [A, B, C] nos slots [2, 3, 4], pontas vazias
        let three = vec![dummy_card("A"), dummy_card("B"), dummy_card("C")];
        let (slots, _, ind) = build_centered_carousel(&three, 1);
        assert_eq!(ind, "2 / 3");
        assert_eq!(slots.len(), 7);
        assert!(slots[0].is_empty_slot);
        assert!(slots[1].is_empty_slot);
        assert_eq!(slots[2].title, "A");
        assert_eq!(slots[3].title, "B");
        assert_eq!(slots[4].title, "C");
        assert!(slots[5].is_empty_slot);
        assert!(slots[6].is_empty_slot);

        // 7 itens com cursor no 0: wrap circular -> [E, F, G, A, B, C, D] (centro em 3 = "A")
        let seven = vec![
            dummy_card("A"),
            dummy_card("B"),
            dummy_card("C"),
            dummy_card("D"),
            dummy_card("E"),
            dummy_card("F"),
            dummy_card("G"),
        ];
        let (slots, _, ind) = build_centered_carousel(&seven, 0);
        assert_eq!(ind, "1 / 7");
        let titles: Vec<&str> = slots.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["E", "F", "G", "A", "B", "C", "D"]);
    }

    #[test]
    fn full_audio_mixer_navigation_lock_adjust_mute_unlock_back_flow() {
        let cfg = Config::default();
        let mode = ProfileMode::Auto;
        let mut hub = HubController::with_desktop_apps(Vec::new());
        let t0 = Instant::now();

        assert_eq!(
            hub.handle_event(KnobEvent::RotateCw, t0, &cfg, &mode, "global"),
            vec![HubEffect::DispatchNormal(KnobEvent::RotateCw)]
        );

        let fx = hub.handle_event(KnobEvent::KnobPress, t0, &cfg, &mode, "firefox");
        assert!(hub.is_open());
        assert!(
            matches!(fx.first(), Some(HubEffect::RenderOverlay(vm)) if vm.active_pill == "KnobContext" && vm.cards.len() == 7)
        );

        let fx_rot = hub.handle_event(
            KnobEvent::RotateCw,
            t0 + Duration::from_millis(100),
            &cfg,
            &mode,
            "firefox",
        );
        assert!(
            matches!(fx_rot.first(), Some(HubEffect::RenderOverlay(vm)) if vm.scroll_delta == 1)
        );
        let fx = hub.handle_event(
            KnobEvent::Key3,
            t0 + Duration::from_millis(200),
            &cfg,
            &mode,
            "firefox",
        );
        assert!(fx.contains(&HubEffect::System(SystemRequest::FetchAudioMixer)));
        assert!(fx
            .iter()
            .any(|e| matches!(e, HubEffect::RenderOverlay(vm) if vm.screen_changed)));

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

        let fx = hub.handle_event(
            KnobEvent::KnobPress,
            t0 + Duration::from_millis(700),
            &cfg,
            &mode,
            "firefox",
        );
        assert!(matches!(fx.first(), Some(HubEffect::RenderOverlay(vm)) if !vm.adjusting));

        let fx = hub.handle_event(
            KnobEvent::Key1,
            t0 + Duration::from_millis(800),
            &cfg,
            &mode,
            "firefox",
        );
        assert!(
            matches!(fx.first(), Some(HubEffect::RenderOverlay(vm)) if vm.active_pill == "KnobContext")
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
    fn brightness_adjusting_and_wallpaper_apply_only_on_confirm() {
        let cfg = Config::default();
        let mode = ProfileMode::Auto;
        let mut hub = HubController::with_desktop_apps(Vec::new());
        let t0 = Instant::now();

        let _ = hub.handle_event(KnobEvent::KnobPress, t0, &cfg, &mode, "global");
        for i in 1..=4 {
            let _ = hub.handle_event(
                KnobEvent::RotateCw,
                t0 + Duration::from_millis(i * 50),
                &cfg,
                &mode,
                "global",
            );
        }
        let fx = hub.handle_event(
            KnobEvent::KnobPress,
            t0 + Duration::from_millis(300),
            &cfg,
            &mode,
            "global",
        );
        assert!(fx.contains(&HubEffect::System(SystemRequest::FetchBrightness)));

        let _ = hub.apply_system_response(
            SystemResponse::Brightness(BrightnessSnapshot {
                monitors: vec![MonitorBrightnessItem {
                    id: "mon:DP-3".into(),
                    is_all: false,
                    monitor_name: Some("DP-3".into()),
                    title: "DP-3".into(),
                    subtitle: "GA271".into(),
                    percent: 30,
                }],
            }),
            &cfg,
            &mode,
            "global",
        );
        let _ = hub.handle_event(
            KnobEvent::KnobPress,
            t0 + Duration::from_millis(350),
            &cfg,
            &mode,
            "global",
        );
        let fx = hub.handle_event(
            KnobEvent::RotateCw,
            t0 + Duration::from_millis(400),
            &cfg,
            &mode,
            "global",
        );
        assert!(
            fx.contains(&HubEffect::System(SystemRequest::SetBrightness {
                monitor_name: Some("DP-3".into()),
                percent: 35,
            }))
        );

        let _ = hub.handle_event(
            KnobEvent::Key1,
            t0 + Duration::from_millis(450),
            &cfg,
            &mode,
            "global",
        );
        let _ = hub.handle_event(
            KnobEvent::Key1,
            t0 + Duration::from_millis(500),
            &cfg,
            &mode,
            "global",
        );
        let _ = hub.handle_event(
            KnobEvent::RotateCw,
            t0 + Duration::from_millis(550),
            &cfg,
            &mode,
            "global",
        );
        let fx = hub.handle_event(
            KnobEvent::KnobPress,
            t0 + Duration::from_millis(600),
            &cfg,
            &mode,
            "global",
        );
        assert!(fx.contains(&HubEffect::System(SystemRequest::FetchWallpapers)));

        let _ = hub.apply_system_response(
            SystemResponse::Wallpapers(WallpapersSnapshot {
                items: vec![
                    WallpaperItem {
                        path: None,
                        is_random: true,
                        title: "Aleatório".into(),
                        category: "Caelestia".into(),
                        is_current: false,
                    },
                    WallpaperItem {
                        path: Some("/wp/a.png".into()),
                        is_random: false,
                        title: "a".into(),
                        category: "wp".into(),
                        is_current: false,
                    },
                ],
            }),
            &cfg,
            &mode,
            "global",
        );

        let fx_rot = hub.handle_event(
            KnobEvent::RotateCw,
            t0 + Duration::from_millis(650),
            &cfg,
            &mode,
            "global",
        );
        assert!(!fx_rot
            .iter()
            .any(|e| matches!(e, HubEffect::System(SystemRequest::SetWallpaper { .. }))));

        let fx_ok = hub.handle_event(
            KnobEvent::KnobPress,
            t0 + Duration::from_millis(700),
            &cfg,
            &mode,
            "global",
        );
        assert!(
            fx_ok.contains(&HubEffect::System(SystemRequest::SetWallpaper {
                path: "/wp/a.png".into(),
            }))
        );
    }

    #[test]
    fn timeout_is_longer_in_adjusting_mode() {
        let cfg = Config::default();
        let mode = ProfileMode::Auto;
        let mut hub = HubController::with_desktop_apps(Vec::new());
        let t0 = Instant::now();

        let _ = hub.handle_event(KnobEvent::KnobPress, t0, &cfg, &mode, "global");
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
