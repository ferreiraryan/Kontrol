use crate::input::KnobEvent;
use std::time::{Duration, Instant};

pub const DEFAULT_OVERLAY_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Normal,
    Overlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayTransition {
    /// Em modo normal: despachar o evento para `ActionDispatcher`.
    DispatchAction(KnobEvent),
    /// Abre o overlay destacando o perfil ativo atual.
    OpenOverlay { highlighted_index: usize },
    /// Atualiza o destaque no overlay após rotação.
    UpdateHighlight { highlighted_index: usize },
    /// Confirma o perfil destacado, fecha o overlay e volta ao modo normal.
    CommitAndClose { selected_index: usize },
    /// Fecha o overlay por inatividade sem alterar o perfil ativo.
    TimeoutClose,
    /// Evento ignorado/consumido enquanto o overlay está aberto.
    Consume,
}

/// Máquina de estados pura para transição entre modo normal e overlay de perfis.
#[derive(Debug)]
pub struct OverlayController {
    mode: AppMode,
    profile_count: usize,
    active_index: usize,
    highlighted_index: usize,
    settings_open: bool,
    timeout: Duration,
    last_interaction: Option<Instant>,
}

impl OverlayController {
    pub fn new(profile_count: usize, active_index: usize, timeout: Duration) -> Self {
        let clamped_active = if profile_count == 0 {
            0
        } else {
            active_index.min(profile_count - 1)
        };
        Self {
            mode: AppMode::Normal,
            profile_count,
            active_index: clamped_active,
            highlighted_index: clamped_active,
            settings_open: false,
            timeout,
            last_interaction: None,
        }
    }

    #[cfg(test)]
    pub fn mode(&self) -> AppMode {
        self.mode
    }

    pub fn active_index(&self) -> usize {
        self.active_index
    }

    #[cfg(test)]
    pub fn highlighted_index(&self) -> usize {
        self.highlighted_index
    }

    pub fn set_settings_open(&mut self, open: bool) {
        self.settings_open = open;
        if open && self.mode == AppMode::Overlay {
            self.mode = AppMode::Normal;
            self.highlighted_index = self.active_index;
            self.last_interaction = None;
        }
    }

    pub fn set_profile_count(&mut self, count: usize, active_index: usize) {
        self.profile_count = count;
        let clamped = if count == 0 {
            0
        } else {
            active_index.min(count - 1)
        };
        self.active_index = clamped;
        self.highlighted_index = clamped;
    }

    pub fn sync_active_index(&mut self, active_index: usize) {
        if self.profile_count > 0 {
            self.active_index = active_index.min(self.profile_count - 1);
        }
    }

    pub fn handle_event(&mut self, event: KnobEvent, now: Instant) -> OverlayTransition {
        match self.mode {
            AppMode::Normal => match event {
                KnobEvent::KnobPress if self.profile_count > 0 && !self.settings_open => {
                    self.mode = AppMode::Overlay;
                    self.highlighted_index = self.active_index;
                    self.last_interaction = Some(now);
                    OverlayTransition::OpenOverlay {
                        highlighted_index: self.highlighted_index,
                    }
                }
                other => OverlayTransition::DispatchAction(other),
            },
            AppMode::Overlay => match event {
                KnobEvent::RotateCw if self.profile_count > 0 => {
                    self.highlighted_index = (self.highlighted_index + 1) % self.profile_count;
                    self.last_interaction = Some(now);
                    OverlayTransition::UpdateHighlight {
                        highlighted_index: self.highlighted_index,
                    }
                }
                KnobEvent::RotateCcw if self.profile_count > 0 => {
                    self.highlighted_index =
                        (self.highlighted_index + self.profile_count - 1) % self.profile_count;
                    self.last_interaction = Some(now);
                    OverlayTransition::UpdateHighlight {
                        highlighted_index: self.highlighted_index,
                    }
                }
                KnobEvent::KnobPress => {
                    self.active_index = self.highlighted_index;
                    self.mode = AppMode::Normal;
                    self.last_interaction = None;
                    OverlayTransition::CommitAndClose {
                        selected_index: self.active_index,
                    }
                }
                _ => {
                    self.last_interaction = Some(now);
                    OverlayTransition::Consume
                }
            },
        }
    }

    pub fn check_timeout(&mut self, now: Instant) -> Option<OverlayTransition> {
        if self.mode != AppMode::Overlay {
            return None;
        }
        let last = self.last_interaction?;
        if now.saturating_duration_since(last) >= self.timeout {
            self.mode = AppMode::Normal;
            self.highlighted_index = self.active_index;
            self.last_interaction = None;
            Some(OverlayTransition::TimeoutClose)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AppMode, OverlayController, OverlayTransition, DEFAULT_OVERLAY_TIMEOUT};
    use crate::input::KnobEvent;
    use std::time::{Duration, Instant};

    #[test]
    fn normal_mode_dispatches_non_press_events_and_opens_on_knob_press() {
        let mut ctrl = OverlayController::new(3, 0, DEFAULT_OVERLAY_TIMEOUT);
        let t0 = Instant::now();

        assert_eq!(ctrl.mode(), AppMode::Normal);
        assert_eq!(
            ctrl.handle_event(KnobEvent::RotateCw, t0),
            OverlayTransition::DispatchAction(KnobEvent::RotateCw)
        );
        assert_eq!(
            ctrl.handle_event(KnobEvent::Key1, t0),
            OverlayTransition::DispatchAction(KnobEvent::Key1)
        );

        assert_eq!(
            ctrl.handle_event(KnobEvent::KnobPress, t0),
            OverlayTransition::OpenOverlay {
                highlighted_index: 0
            }
        );
        assert_eq!(ctrl.mode(), AppMode::Overlay);
    }

    #[test]
    fn overlay_rotates_circularly_and_commits_on_knob_press() {
        let mut ctrl = OverlayController::new(3, 0, DEFAULT_OVERLAY_TIMEOUT);
        let t0 = Instant::now();

        ctrl.handle_event(KnobEvent::KnobPress, t0);

        // Wrap-around para trás: 0 -> 2
        assert_eq!(
            ctrl.handle_event(KnobEvent::RotateCcw, t0 + Duration::from_millis(200)),
            OverlayTransition::UpdateHighlight {
                highlighted_index: 2
            }
        );
        // Wrap-around para frente: 2 -> 0 -> 1
        assert_eq!(
            ctrl.handle_event(KnobEvent::RotateCw, t0 + Duration::from_millis(400)),
            OverlayTransition::UpdateHighlight {
                highlighted_index: 0
            }
        );
        assert_eq!(
            ctrl.handle_event(KnobEvent::RotateCw, t0 + Duration::from_millis(600)),
            OverlayTransition::UpdateHighlight {
                highlighted_index: 1
            }
        );

        // Clique confirma perfil 1 e fecha
        assert_eq!(
            ctrl.handle_event(KnobEvent::KnobPress, t0 + Duration::from_millis(800)),
            OverlayTransition::CommitAndClose { selected_index: 1 }
        );
        assert_eq!(ctrl.mode(), AppMode::Normal);
        assert_eq!(ctrl.active_index(), 1);
    }

    #[test]
    fn overlay_closes_after_timeout_without_changing_active_profile() {
        let mut ctrl = OverlayController::new(3, 0, Duration::from_secs(3));
        let t0 = Instant::now();

        ctrl.handle_event(KnobEvent::KnobPress, t0);
        ctrl.handle_event(KnobEvent::RotateCw, t0 + Duration::from_secs(2));
        assert_eq!(ctrl.highlighted_index(), 1);

        // 2s após a última rotação (t0 + 4s): ainda dentro dos 3s de inatividade
        assert_eq!(ctrl.check_timeout(t0 + Duration::from_secs(4)), None);
        assert_eq!(ctrl.mode(), AppMode::Overlay);

        // 3s após a última rotação (t0 + 5s): timeout fecha sem mudar active_index
        assert_eq!(
            ctrl.check_timeout(t0 + Duration::from_secs(5)),
            Some(OverlayTransition::TimeoutClose)
        );
        assert_eq!(ctrl.mode(), AppMode::Normal);
        assert_eq!(ctrl.active_index(), 0);
    }

    #[test]
    fn overlay_consumes_side_keys_without_dispatching_actions() {
        let mut ctrl = OverlayController::new(2, 0, DEFAULT_OVERLAY_TIMEOUT);
        let t0 = Instant::now();

        ctrl.handle_event(KnobEvent::KnobPress, t0);
        assert_eq!(
            ctrl.handle_event(KnobEvent::Key2, t0 + Duration::from_millis(100)),
            OverlayTransition::Consume
        );
        assert_eq!(ctrl.mode(), AppMode::Overlay);
    }

    #[test]
    fn overlay_does_not_open_while_settings_window_is_open() {
        let mut ctrl = OverlayController::new(3, 0, DEFAULT_OVERLAY_TIMEOUT);
        let t0 = Instant::now();

        ctrl.set_settings_open(true);
        assert_eq!(
            ctrl.handle_event(KnobEvent::KnobPress, t0),
            OverlayTransition::DispatchAction(KnobEvent::KnobPress)
        );
        assert_eq!(ctrl.mode(), AppMode::Normal);

        ctrl.set_settings_open(false);
        assert_eq!(
            ctrl.handle_event(KnobEvent::KnobPress, t0 + Duration::from_millis(50)),
            OverlayTransition::OpenOverlay {
                highlighted_index: 0
            }
        );
    }
}
