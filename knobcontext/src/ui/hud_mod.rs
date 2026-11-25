use crate::config::ActionConfig;
use crate::error::KnobError;
use crate::providers::smtc::{SmtcHubSnapshot, SmtcThumbnail};
use crate::ui::MiniHud;
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer, SharedString};
use std::time::{Duration, Instant};

pub const HUD_WIDTH_PX: i32 = 244;
pub const HUD_HEIGHT_PX: i32 = 68;
pub const DEFAULT_HUD_TIMEOUT: Duration = Duration::from_millis(500);

/// Conteúdo exibido no Mini-HUD ao girar o knob fora do Hub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudViewData {
    pub action_key: String,
    pub glyph: String,
    pub label: String,
    pub value_text: String,
    pub has_progress: bool,
    pub value_percent: i32,
    pub max_percent: i32,
    pub is_muted: bool,
    pub repeat_count: u32,
}

/// Máquina de estados pura para acumular giros rápidos e gerenciar o timeout de 500ms do Mini-HUD.
#[derive(Debug, Clone)]
pub struct HudStateMachine {
    visible: bool,
    timeout: Duration,
    last_trigger: Option<Instant>,
    current: Option<HudViewData>,
    last_known_volume: Option<(i32, bool)>,
}

impl HudStateMachine {
    pub fn new(timeout: Duration) -> Self {
        Self {
            visible: false,
            timeout,
            last_trigger: None,
            current: None,
            last_known_volume: None,
        }
    }

    #[cfg(test)]
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Registra uma ação disparada por `RotateCw`/`RotateCcw` fora do Hub.
    ///
    /// Se o HUD já estiver aberto para a mesma categoria (ex.: volume ou mesmo atalho),
    /// acumula o valor/contador e reinicia o timeout de 500ms.
    /// Retorna `(view_data, should_query_volume)`.
    pub fn trigger_action(
        &mut self,
        action: &ActionConfig,
        volume_step: i32,
        max_volume: i32,
        now: Instant,
    ) -> (HudViewData, bool) {
        self.last_trigger = Some(now);
        self.visible = true;

        match action {
            ActionConfig::VolumeUp | ActionConfig::VolumeDown | ActionConfig::VolumeMute => {
                let delta = match action {
                    ActionConfig::VolumeUp => volume_step.max(1),
                    ActionConfig::VolumeDown => -volume_step.max(1),
                    _ => 0,
                };
                let (base_vol, base_muted) = self.last_known_volume.unwrap_or((50, false));
                let next_muted = if matches!(action, ActionConfig::VolumeMute) {
                    !base_muted
                } else {
                    base_muted
                };
                let next_vol = (base_vol + delta).clamp(0, max_volume.max(100));
                self.last_known_volume = Some((next_vol, next_muted));

                let data = HudViewData {
                    action_key: "volume".to_string(),
                    glyph: if next_muted { "✕" } else { "♫" }.to_string(),
                    label: "Volume".to_string(),
                    value_text: if next_muted {
                        format!("MUTE ({next_vol}%)")
                    } else {
                        format!("{next_vol}%")
                    },
                    has_progress: true,
                    value_percent: next_vol,
                    max_percent: max_volume.max(100),
                    is_muted: next_muted,
                    repeat_count: 1,
                };
                self.current = Some(data.clone());
                (data, true)
            }
            other => {
                let (glyph, label) = describe_non_continuous_action(other);
                let action_key = other.to_display_string();
                let next_count = match &self.current {
                    Some(prev) if prev.action_key == action_key => {
                        prev.repeat_count.saturating_add(1)
                    }
                    _ => 1,
                };
                let value_text = if next_count > 1 {
                    format!("×{next_count}")
                } else {
                    String::new()
                };
                let data = HudViewData {
                    action_key,
                    glyph: glyph.to_string(),
                    label,
                    value_text,
                    has_progress: false,
                    value_percent: 0,
                    max_percent: 100,
                    is_muted: false,
                    repeat_count: next_count,
                };
                self.current = Some(data.clone());
                (data, false)
            }
        }
    }

    /// Atualiza o Mini-HUD com o volume real retornado assincronamente pelo `SystemWorker`.
    pub fn apply_master_volume(
        &mut self,
        percent: i32,
        muted: bool,
        max_volume: i32,
    ) -> Option<HudViewData> {
        let clamped = percent.clamp(0, max_volume.max(100));
        self.last_known_volume = Some((clamped, muted));
        if !self.visible {
            return None;
        }
        let current = self.current.as_mut()?;
        if current.action_key != "volume" {
            return None;
        }
        current.glyph = if muted { "✕" } else { "♫" }.to_string();
        current.value_percent = clamped;
        current.max_percent = max_volume.max(100);
        current.is_muted = muted;
        current.value_text = if muted {
            format!("MUTE ({clamped}%)")
        } else {
            format!("{clamped}%")
        };
        Some(current.clone())
    }

    /// Retorna `true` se o timeout de 500ms expirou e o HUD deve ser fechado.
    pub fn check_timeout(&mut self, now: Instant) -> bool {
        if !self.visible {
            return false;
        }
        let Some(last) = self.last_trigger else {
            return false;
        };
        if now.saturating_duration_since(last) >= self.timeout {
            self.dismiss();
            true
        } else {
            false
        }
    }

    pub fn dismiss(&mut self) {
        self.visible = false;
        self.last_trigger = None;
        self.current = None;
    }
}

fn describe_non_continuous_action(action: &ActionConfig) -> (&'static str, String) {
    match action {
        ActionConfig::MediaPlayPause => ("⏯", "Play / Pause".to_string()),
        ActionConfig::MediaNext => ("⏭", "Próxima faixa".to_string()),
        ActionConfig::MediaPrev => ("⏮", "Faixa anterior".to_string()),
        ActionConfig::Scroll { delta, horizontal } => {
            let amount = delta.unsigned_abs();
            let (glyph, dir) = match (*horizontal, *delta >= 0) {
                (false, false) => ("↑", "Scroll Cima"),
                (false, true) => ("↓", "Scroll Baixo"),
                (true, false) => ("←", "Scroll Esq."),
                (true, true) => ("→", "Scroll Dir."),
            };
            (glyph, format!("{dir} ×{amount}"))
        }
        ActionConfig::Shortcut { keys } => ("⌨", keys.join("+")),
        ActionConfig::Launch { desktop_file } => {
            ("▶", desktop_file.trim_end_matches(".desktop").to_string())
        }
        ActionConfig::Open { target } => ("↗", target.clone()),
        ActionConfig::Exec { command } => ("$", command.clone()),
        ActionConfig::Macro { steps } => ("⚡", format!("Macro ({} passos)", steps.len())),
        ActionConfig::Provider {
            provider_id,
            command_id,
            ..
        } => ("◈", format!("{provider_id} • {command_id}")),
        ActionConfig::NextProfile => ("◈", "Próximo perfil".to_string()),
        ActionConfig::VolumeUp
        | ActionConfig::VolumeDown
        | ActionConfig::VolumeMute
        | ActionConfig::None => ("•", action.to_display_string()),
    }
}

/// Controlador da janela independente `MiniHud` no Slint.
///
/// Recria a instância `MiniHud` ao abrir após um `hide()` para manter `floating` no Wayland,
/// mas **reutiliza** a janela enquanto já estiver visível para acumular giros sem flicker.
pub struct HudController {
    window: Option<MiniHud>,
    state: HudStateMachine,
    cached_smtc_thumb: Option<SmtcThumbnail>,
    use_backdrop_blur: bool,
    animations_enabled: bool,
}

impl HudController {
    pub fn new() -> Self {
        Self {
            window: None,
            state: HudStateMachine::new(DEFAULT_HUD_TIMEOUT),
            cached_smtc_thumb: None,
            use_backdrop_blur: false,
            animations_enabled: true,
        }
    }

    pub fn set_render_options(&mut self, use_backdrop_blur: bool, animations_enabled: bool) {
        self.use_backdrop_blur = use_backdrop_blur;
        self.animations_enabled = animations_enabled;
    }

    /// Indica se a ação executada deve exibir capa de álbum SMTC quando disponível.
    pub fn action_supports_media_art(action: &ActionConfig) -> bool {
        matches!(
            action,
            ActionConfig::VolumeUp
                | ActionConfig::VolumeDown
                | ActionConfig::VolumeMute
                | ActionConfig::MediaPlayPause
                | ActionConfig::MediaNext
                | ActionConfig::MediaPrev
        ) || matches!(
            action,
            ActionConfig::Provider { provider_id, .. }
                if provider_id.eq_ignore_ascii_case("smtc")
                    || provider_id.eq_ignore_ascii_case("mpris")
        )
    }

    /// Atualiza o cache de capa SMTC para o Mini-HUD e repinta caso esteja visível em modo mídia/volume.
    pub fn update_smtc_media(&mut self, snap: &SmtcHubSnapshot) -> Result<(), KnobError> {
        self.cached_smtc_thumb = snap.session.as_ref().and_then(|s| s.thumbnail.clone());
        if let Some(current) = self.state.current.clone() {
            if self.window.is_some() {
                let _ = self.render_data(&current)?;
            }
        }
        Ok(())
    }

    /// Exibe ou atualiza o Mini-HUD. Retorna `(just_opened, should_query_volume)`.
    pub fn show_for_action(
        &mut self,
        action: &ActionConfig,
        volume_step: i32,
        max_volume: i32,
    ) -> Result<(bool, bool), KnobError> {
        let (data, should_query_vol) =
            self.state
                .trigger_action(action, volume_step, max_volume, Instant::now());
        let show_art = Self::action_supports_media_art(action);
        let just_opened = self.render_data_with_art(&data, show_art)?;
        Ok((just_opened, should_query_vol))
    }

    pub fn update_master_volume(
        &mut self,
        percent: i32,
        muted: bool,
        max_volume: i32,
    ) -> Result<(), KnobError> {
        if let Some(data) = self.state.apply_master_volume(percent, muted, max_volume) {
            let _ = self.render_data_with_art(&data, true)?;
        }
        Ok(())
    }

    pub fn check_timeout(&mut self) -> Result<(), KnobError> {
        if self.state.check_timeout(Instant::now()) {
            self.hide_window_only()?;
        }
        Ok(())
    }

    pub fn hide(&mut self) -> Result<(), KnobError> {
        self.state.dismiss();
        self.hide_window_only()
    }

    fn render_data(&mut self, data: &HudViewData) -> Result<bool, KnobError> {
        let is_media_or_vol = data.action_key == "volume"
            || data.action_key.to_ascii_lowercase().contains("media")
            || data
                .action_key
                .to_ascii_lowercase()
                .starts_with("provider:smtc");
        self.render_data_with_art(data, is_media_or_vol)
    }

    fn render_data_with_art(
        &mut self,
        data: &HudViewData,
        allow_art: bool,
    ) -> Result<bool, KnobError> {
        let just_opened = self.window.is_none();
        if self.window.is_none() {
            let win = MiniHud::new().map_err(|err| KnobError::UiInit(err.to_string()))?;
            self.window = Some(win);
        }
        if let Some(win) = &self.window {
            win.set_glyph(SharedString::from(data.glyph.as_str()));
            win.set_label(SharedString::from(data.label.as_str()));
            win.set_value_text(SharedString::from(data.value_text.as_str()));
            win.set_has_progress(data.has_progress);
            win.set_value_percent(data.value_percent);
            win.set_max_percent(data.max_percent);
            win.set_is_muted(data.is_muted);
            win.set_use_backdrop_blur(self.use_backdrop_blur);
            win.set_animations_enabled(self.animations_enabled);
            if allow_art {
                if let Some(ref thumb) = self.cached_smtc_thumb {
                    let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                        &thumb.rgba,
                        thumb.width,
                        thumb.height,
                    );
                    win.set_art_source(Image::from_rgba8(buffer));
                    win.set_has_art(true);
                } else {
                    win.set_has_art(false);
                }
            } else {
                win.set_has_art(false);
            }
            if just_opened {
                crate::ui::show_popup_without_stealing_focus("KnobContext HUD", || {
                    win.show().map_err(|err| KnobError::UiRun(err.to_string()))
                })?;
            }
        }
        Ok(just_opened)
    }

    fn hide_window_only(&mut self) -> Result<(), KnobError> {
        if let Some(win) = self.window.take() {
            win.hide()
                .map_err(|err| KnobError::UiRun(err.to_string()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{HudStateMachine, DEFAULT_HUD_TIMEOUT};
    use crate::config::ActionConfig;
    use std::time::{Duration, Instant};

    #[test]
    fn accumulates_consecutive_volume_turns_and_resets_500ms_timeout() {
        let mut sm = HudStateMachine::new(DEFAULT_HUD_TIMEOUT);
        let t0 = Instant::now();

        // Sincroniza volume inicial conhecido = 40%
        let _ = sm.apply_master_volume(40, false, 150);
        assert!(!sm.is_visible());

        // 1º giro (+5% -> 45%)
        let (d1, query1) = sm.trigger_action(&ActionConfig::VolumeUp, 5, 150, t0);
        assert!(query1);
        assert!(sm.is_visible());
        assert_eq!(d1.value_percent, 45);
        assert_eq!(d1.value_text, "45%");
        assert!(d1.has_progress);

        // 300ms depois: 2º giro (+5% -> 50%) reinicia o timeout
        let (d2, _) = sm.trigger_action(
            &ActionConfig::VolumeUp,
            5,
            150,
            t0 + Duration::from_millis(300),
        );
        assert_eq!(d2.value_percent, 50);

        // Em t0 + 600ms (apenas 300ms desde o último giro): ainda visível!
        assert!(!sm.check_timeout(t0 + Duration::from_millis(600)));
        assert!(sm.is_visible());

        // Em t0 + 800ms (500ms após o último giro): fecha!
        assert!(sm.check_timeout(t0 + Duration::from_millis(800)));
        assert!(!sm.is_visible());
    }

    #[test]
    fn non_continuous_actions_show_no_progress_and_accumulate_repeat_count() {
        let mut sm = HudStateMachine::new(DEFAULT_HUD_TIMEOUT);
        let t0 = Instant::now();
        let tab = ActionConfig::Shortcut {
            keys: vec!["Ctrl".into(), "Tab".into()],
        };

        let (d1, query) = sm.trigger_action(&tab, 5, 150, t0);
        assert!(!query);
        assert!(!d1.has_progress);
        assert_eq!(d1.label, "Ctrl+Tab");
        assert_eq!(d1.value_text, "");

        let (d2, _) = sm.trigger_action(&tab, 5, 150, t0 + Duration::from_millis(120));
        assert_eq!(d2.repeat_count, 2);
        assert_eq!(d2.value_text, "×2");

        // Trocar para outra ação reinicia o contador
        let (d3, _) = sm.trigger_action(
            &ActionConfig::MediaPlayPause,
            5,
            150,
            t0 + Duration::from_millis(200),
        );
        assert_eq!(d3.repeat_count, 1);
        assert_eq!(d3.label, "Play / Pause");
    }

    #[test]
    fn real_volume_update_refreshes_visible_hud_only() {
        let mut sm = HudStateMachine::new(DEFAULT_HUD_TIMEOUT);
        let t0 = Instant::now();
        let _ = sm.trigger_action(&ActionConfig::VolumeUp, 5, 150, t0);

        let updated = sm
            .apply_master_volume(72, true, 150)
            .expect("deve atualizar HUD visível");
        assert_eq!(updated.value_percent, 72);
        assert!(updated.is_muted);
        assert_eq!(updated.value_text, "MUTE (72%)");

        sm.dismiss();
        assert!(sm.apply_master_volume(80, false, 150).is_none());
    }
}
