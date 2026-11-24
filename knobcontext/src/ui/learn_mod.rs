use crate::error::KnobError;
use crate::ui::action_editor_mod::format_captured_key;
use crate::ui::LearnWindow;
use slint::{CloseRequestResponse, ComponentHandle, SharedString, Timer, TimerMode};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

pub const LEARN_CONFIRM_DELAY_MS: u64 = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LearnPhase {
    Capturing,
    Confirmed(String),
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LearnOutcome {
    Updated {
        preview: String,
        schedule_confirm_ms: Option<u64>,
    },
    Confirmed(String),
    Cancelled,
    Ignored,
}

/// Máquina de estados pura para o Modo Aprender (testável sem janela Slint).
#[derive(Debug, Clone)]
pub struct LearnState {
    phase: LearnPhase,
    active_keys: usize,
    captured_combo: Option<String>,
    live_preview: String,
}

impl Default for LearnState {
    fn default() -> Self {
        Self::new()
    }
}

impl LearnState {
    pub fn new() -> Self {
        Self {
            phase: LearnPhase::Capturing,
            active_keys: 0,
            captured_combo: None,
            live_preview: String::new(),
        }
    }

    #[cfg(test)]
    pub fn phase(&self) -> &LearnPhase {
        &self.phase
    }

    pub fn on_key_pressed(
        &mut self,
        text: &str,
        ctrl: bool,
        alt: bool,
        shift: bool,
        meta: bool,
    ) -> LearnOutcome {
        if self.phase != LearnPhase::Capturing {
            return LearnOutcome::Ignored;
        }

        // Esc isolado cancela sem alterar o campo de ação
        if text == "\u{001b}" && !ctrl && !alt && !shift && !meta {
            self.phase = LearnPhase::Cancelled;
            return LearnOutcome::Cancelled;
        }

        // Enter isolado confirma imediatamente se já houver combinação capturada
        if (text == "\u{000a}" || text == "\u{000d}") && !ctrl && !alt && !shift && !meta {
            if let Some(combo) = self.captured_combo.clone() {
                self.phase = LearnPhase::Confirmed(combo.clone());
                return LearnOutcome::Confirmed(combo);
            }
        }

        self.active_keys = self.active_keys.saturating_add(1);

        if let Some(combo) = format_captured_key(text, ctrl, alt, shift, meta) {
            self.captured_combo = Some(combo.clone());
            self.live_preview = combo.clone();
            LearnOutcome::Updated {
                preview: combo,
                schedule_confirm_ms: None,
            }
        } else {
            let mod_preview = format_modifiers_preview(ctrl, alt, shift, meta);
            if !mod_preview.is_empty() {
                self.live_preview = format!("{mod_preview}+...");
            }
            LearnOutcome::Updated {
                preview: self.live_preview.clone(),
                schedule_confirm_ms: None,
            }
        }
    }

    pub fn on_key_released(
        &mut self,
        _text: &str,
        ctrl: bool,
        alt: bool,
        shift: bool,
        meta: bool,
    ) -> LearnOutcome {
        if self.phase != LearnPhase::Capturing {
            return LearnOutcome::Ignored;
        }

        self.active_keys = self.active_keys.saturating_sub(1);
        let no_modifiers_held = !ctrl && !alt && !shift && !meta;

        if self.active_keys == 0 && no_modifiers_held {
            if let Some(combo) = self.captured_combo.clone() {
                return LearnOutcome::Updated {
                    preview: combo,
                    schedule_confirm_ms: Some(LEARN_CONFIRM_DELAY_MS),
                };
            }
            self.live_preview.clear();
            return LearnOutcome::Updated {
                preview: String::new(),
                schedule_confirm_ms: None,
            };
        }

        LearnOutcome::Ignored
    }

    pub fn on_confirm_timeout(&mut self) -> Option<String> {
        if self.phase != LearnPhase::Capturing || self.active_keys > 0 {
            return None;
        }
        let combo = self.captured_combo.take()?;
        self.phase = LearnPhase::Confirmed(combo.clone());
        Some(combo)
    }

    pub fn cancel(&mut self) {
        self.phase = LearnPhase::Cancelled;
    }
}

fn format_modifiers_preview(ctrl: bool, alt: bool, shift: bool, meta: bool) -> String {
    let mut parts = Vec::new();
    if ctrl {
        parts.push("Ctrl");
    }
    if alt {
        parts.push("Alt");
    }
    if shift {
        parts.push("Shift");
    }
    if meta {
        parts.push("Super");
    }
    parts.join("+")
}

/// Controlador da janela flutuante de captura do Modo Aprender.
///
/// Abre uma janela Slint dedicada com foco de teclado (`wl_keyboard` no Wayland),
/// exibe a combinação em tempo real e confirma automaticamente 200ms após soltar
/// todas as teclas (ou imediatamente com Enter; Esc cancela).
pub struct LearnController {
    window: Rc<RefCell<Option<LearnWindow>>>,
    confirm_timer: Rc<Timer>,
}

impl Default for LearnController {
    fn default() -> Self {
        Self::new()
    }
}

impl LearnController {
    pub fn new() -> Self {
        Self {
            window: Rc::new(RefCell::new(None)),
            confirm_timer: Rc::new(Timer::default()),
        }
    }

    pub fn close(&self) {
        self.confirm_timer.stop();
        if let Some(win) = self.window.borrow_mut().take() {
            let _ = win.hide();
        }
    }

    pub fn open(
        &self,
        target_label: &str,
        on_confirm: impl Fn(String) + 'static,
        on_cancel: impl Fn() + 'static,
    ) -> Result<(), KnobError> {
        self.close();

        let win = LearnWindow::new().map_err(|err| KnobError::UiInit(err.to_string()))?;
        win.set_target_label(SharedString::from(target_label));
        win.set_combo_preview(SharedString::from(""));
        win.set_status_hint(SharedString::from(
            "Pressione a combinação (Solte ou Enter para confirmar • Esc cancela)",
        ));

        let state = Rc::new(RefCell::new(LearnState::new()));
        let on_confirm = Rc::new(on_confirm);
        let on_cancel = Rc::new(on_cancel);
        let win_slot = Rc::clone(&self.window);
        let timer = Rc::clone(&self.confirm_timer);

        // key-pressed
        {
            let weak = win.as_weak();
            let state_ref = Rc::clone(&state);
            let confirm_cb = Rc::clone(&on_confirm);
            let cancel_cb = Rc::clone(&on_cancel);
            let win_slot_ref = Rc::clone(&win_slot);
            let timer_ref = Rc::clone(&timer);
            win.on_key_pressed(move |text, ctrl, alt, shift, meta| {
                timer_ref.stop();
                let outcome =
                    state_ref
                        .borrow_mut()
                        .on_key_pressed(text.as_str(), ctrl, alt, shift, meta);
                match outcome {
                    LearnOutcome::Updated { preview, .. } => {
                        if let Some(w) = weak.upgrade() {
                            w.set_combo_preview(SharedString::from(preview));
                        }
                    }
                    LearnOutcome::Confirmed(combo) => {
                        if let Some(w) = win_slot_ref.borrow_mut().take() {
                            let _ = w.hide();
                        }
                        confirm_cb(combo);
                    }
                    LearnOutcome::Cancelled => {
                        if let Some(w) = win_slot_ref.borrow_mut().take() {
                            let _ = w.hide();
                        }
                        cancel_cb();
                    }
                    LearnOutcome::Ignored => {}
                }
            });
        }

        // key-released
        {
            let weak = win.as_weak();
            let state_ref = Rc::clone(&state);
            let confirm_cb = Rc::clone(&on_confirm);
            let win_slot_ref = Rc::clone(&win_slot);
            let timer_ref = Rc::clone(&timer);
            win.on_key_released(move |text, ctrl, alt, shift, meta| {
                let outcome =
                    state_ref
                        .borrow_mut()
                        .on_key_released(text.as_str(), ctrl, alt, shift, meta);
                if let LearnOutcome::Updated {
                    preview,
                    schedule_confirm_ms,
                } = outcome
                {
                    if let Some(w) = weak.upgrade() {
                        w.set_combo_preview(SharedString::from(preview));
                    }
                    if let Some(ms) = schedule_confirm_ms {
                        let st = Rc::clone(&state_ref);
                        let cb = Rc::clone(&confirm_cb);
                        let ws = Rc::clone(&win_slot_ref);
                        timer_ref.start(
                            TimerMode::SingleShot,
                            Duration::from_millis(ms),
                            move || {
                                if let Some(combo) = st.borrow_mut().on_confirm_timeout() {
                                    if let Some(w) = ws.borrow_mut().take() {
                                        let _ = w.hide();
                                    }
                                    cb(combo);
                                }
                            },
                        );
                    }
                }
            });
        }

        // Fechamento manual pelo botão X do WM
        {
            let state_ref = Rc::clone(&state);
            let cancel_cb = Rc::clone(&on_cancel);
            let win_slot_ref = Rc::clone(&win_slot);
            let timer_ref = Rc::clone(&timer);
            win.window().on_close_requested(move || {
                timer_ref.stop();
                state_ref.borrow_mut().cancel();
                let _ = win_slot_ref.borrow_mut().take();
                cancel_cb();
                CloseRequestResponse::HideWindow
            });
        }

        win.show()
            .map_err(|err| KnobError::UiInit(err.to_string()))?;
        *self.window.borrow_mut() = Some(win);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{LearnOutcome, LearnPhase, LearnState, LEARN_CONFIRM_DELAY_MS};

    #[test]
    fn captures_ctrl_shift_t_and_confirms_after_release_delay() {
        let mut st = LearnState::new();
        // Pressiona Ctrl puro -> preview parcial
        let out_ctrl = st.on_key_pressed("\u{0011}", true, false, false, false);
        assert_eq!(
            out_ctrl,
            LearnOutcome::Updated {
                preview: "Ctrl+...".to_string(),
                schedule_confirm_ms: None,
            }
        );

        // Pressiona T com Ctrl+Shift
        let out_t = st.on_key_pressed("t", true, false, true, false);
        assert_eq!(
            out_t,
            LearnOutcome::Updated {
                preview: "Ctrl+Shift+T".to_string(),
                schedule_confirm_ms: None,
            }
        );

        // Solta T (Ctrl ainda segurado)
        assert_eq!(
            st.on_key_released("t", true, false, false, false),
            LearnOutcome::Ignored
        );

        // Solta Ctrl (todas as teclas soltas) -> agenda confirmação em 200ms
        assert_eq!(
            st.on_key_released("\u{0011}", false, false, false, false),
            LearnOutcome::Updated {
                preview: "Ctrl+Shift+T".to_string(),
                schedule_confirm_ms: Some(LEARN_CONFIRM_DELAY_MS),
            }
        );

        // Timer de 200ms dispara -> confirma
        assert_eq!(st.on_confirm_timeout(), Some("Ctrl+Shift+T".to_string()));
        assert_eq!(
            st.phase(),
            &LearnPhase::Confirmed("Ctrl+Shift+T".to_string())
        );
    }

    #[test]
    fn captures_alt_f4_and_f5_and_confirms_immediately_on_enter() {
        let mut st = LearnState::new();
        let out = st.on_key_pressed("\u{f707}", false, true, false, false);
        assert_eq!(
            out,
            LearnOutcome::Updated {
                preview: "Alt+F4".to_string(),
                schedule_confirm_ms: None,
            }
        );
        let confirm = st.on_key_pressed("\u{000d}", false, false, false, false);
        assert_eq!(confirm, LearnOutcome::Confirmed("Alt+F4".to_string()));

        let mut st_f5 = LearnState::new();
        let out_f5 = st_f5.on_key_pressed("\u{f708}", false, false, false, false);
        assert_eq!(
            out_f5,
            LearnOutcome::Updated {
                preview: "F5".to_string(),
                schedule_confirm_ms: None,
            }
        );
        assert_eq!(
            st_f5.on_key_released("\u{f708}", false, false, false, false),
            LearnOutcome::Updated {
                preview: "F5".to_string(),
                schedule_confirm_ms: Some(LEARN_CONFIRM_DELAY_MS),
            }
        );
        assert_eq!(st_f5.on_confirm_timeout(), Some("F5".to_string()));
    }

    #[test]
    fn escape_cancels_without_confirming() {
        let mut st = LearnState::new();
        let _ = st.on_key_pressed("a", true, false, false, false);
        assert_eq!(
            st.on_key_pressed("\u{001b}", false, false, false, false),
            LearnOutcome::Cancelled
        );
        assert_eq!(st.phase(), &LearnPhase::Cancelled);
        assert_eq!(st.on_confirm_timeout(), None);
    }
}
