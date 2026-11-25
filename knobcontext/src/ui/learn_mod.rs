use crate::error::KnobError;
use crate::ui::action_editor_mod::{format_captured_key, slint_modifier_key_name};
use crate::ui::LearnWindow;
use slint::{CloseRequestResponse, ComponentHandle, SharedString, Timer, TimerMode};
use std::cell::RefCell;
use std::collections::HashSet;
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
///
/// Suporta tanto combinações completas (`Ctrl+Shift+T`, `Alt+F4`, `Super+E`) quanto
/// teclas modificadoras isoladas (`Ctrl`, `Shift`, `Alt`, `Super`) e ignora eventos de
/// repetição (`repeat = true`) para não travar a detecção de soltura.
#[derive(Debug, Clone)]
pub struct LearnState {
    phase: LearnPhase,
    held_keys: HashSet<String>,
    held_ctrl: bool,
    held_alt: bool,
    held_shift: bool,
    held_meta: bool,
    captured_combo: Option<String>,
    modifier_only_combo: Option<String>,
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
            held_keys: HashSet::new(),
            held_ctrl: false,
            held_alt: false,
            held_shift: false,
            held_meta: false,
            captured_combo: None,
            modifier_only_combo: None,
            live_preview: String::new(),
        }
    }

    #[cfg(test)]
    pub fn phase(&self) -> &LearnPhase {
        &self.phase
    }

    fn effective_modifiers(
        &self,
        ctrl: bool,
        alt: bool,
        shift: bool,
        meta: bool,
    ) -> (bool, bool, bool, bool) {
        (
            ctrl || self.held_ctrl,
            alt || self.held_alt,
            shift || self.held_shift,
            meta || self.held_meta,
        )
    }

    pub fn on_key_pressed(
        &mut self,
        text: &str,
        ctrl: bool,
        alt: bool,
        shift: bool,
        meta: bool,
        repeat: bool,
    ) -> LearnOutcome {
        if self.phase != LearnPhase::Capturing || repeat {
            return LearnOutcome::Ignored;
        }

        // Sincroniza modificadores com o estado atual do compositor e garante o próprio
        // modificador quando o evento é o key-down de Ctrl/Alt/Shift/Super.
        self.held_ctrl = ctrl;
        self.held_alt = alt;
        self.held_shift = shift;
        self.held_meta = meta;
        if let Some(mod_name) = slint_modifier_key_name(text) {
            match mod_name {
                "Ctrl" => self.held_ctrl = true,
                "Alt" => self.held_alt = true,
                "Shift" => self.held_shift = true,
                "Super" => self.held_meta = true,
                _ => {}
            }
        }

        let (eff_ctrl, eff_alt, eff_shift, eff_meta) =
            self.effective_modifiers(ctrl, alt, shift, meta);

        // Esc isolado cancela sem alterar o campo de ação
        if text == "\u{001b}" && !eff_ctrl && !eff_alt && !eff_shift && !eff_meta {
            self.phase = LearnPhase::Cancelled;
            return LearnOutcome::Cancelled;
        }

        // Enter isolado confirma imediatamente se já houver combinação capturada
        if (text == "\u{000a}" || text == "\u{000d}")
            && !eff_ctrl
            && !eff_alt
            && !eff_shift
            && !eff_meta
        {
            if let Some(combo) = self
                .captured_combo
                .clone()
                .or_else(|| self.modifier_only_combo.clone())
            {
                self.phase = LearnPhase::Confirmed(combo.clone());
                return LearnOutcome::Confirmed(combo);
            }
        }

        if !text.is_empty() {
            self.held_keys.insert(text.to_string());
        }

        if let Some(combo) = format_captured_key(text, eff_ctrl, eff_alt, eff_shift, eff_meta) {
            self.captured_combo = Some(combo.clone());
            self.live_preview = combo.clone();
            LearnOutcome::Updated {
                preview: combo,
                schedule_confirm_ms: None,
            }
        } else {
            let mod_preview = format_modifiers_preview(eff_ctrl, eff_alt, eff_shift, eff_meta);
            if !mod_preview.is_empty() {
                // Registra como candidato caso o usuário solte apenas modificadores (ex.: `Super` ou `Ctrl`)
                self.modifier_only_combo = Some(mod_preview.clone());
                if self.captured_combo.is_none() {
                    self.live_preview = mod_preview;
                }
            }
            LearnOutcome::Updated {
                preview: self.live_preview.clone(),
                schedule_confirm_ms: None,
            }
        }
    }

    pub fn on_key_released(
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

        self.held_keys.remove(text);
        if let Some(mod_name) = slint_modifier_key_name(text) {
            match mod_name {
                "Ctrl" => self.held_ctrl = false,
                "Alt" => self.held_alt = false,
                "Shift" => self.held_shift = false,
                "Super" => self.held_meta = false,
                _ => {}
            }
        }
        // Sincroniza também quando o compositor já informa modificador solto
        if !ctrl {
            self.held_ctrl = false;
        }
        if !alt {
            self.held_alt = false;
        }
        if !shift {
            self.held_shift = false;
        }
        if !meta {
            self.held_meta = false;
        }

        let no_modifiers_held =
            !self.held_ctrl && !self.held_alt && !self.held_shift && !self.held_meta;

        if self.held_keys.is_empty() && no_modifiers_held {
            if let Some(combo) = self
                .captured_combo
                .clone()
                .or_else(|| self.modifier_only_combo.clone())
            {
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
        if self.phase != LearnPhase::Capturing || !self.held_keys.is_empty() {
            return None;
        }
        let combo = self
            .captured_combo
            .take()
            .or_else(|| self.modifier_only_combo.take())?;
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
    #[cfg(target_os = "windows")]
    global_capture: Rc<RefCell<Option<Box<dyn crate::platform::KeyCapture>>>>,
    #[cfg(target_os = "windows")]
    poll_timer: Rc<Timer>,
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
            #[cfg(target_os = "windows")]
            global_capture: Rc::new(RefCell::new(None)),
            #[cfg(target_os = "windows")]
            poll_timer: Rc::new(Timer::default()),
        }
    }

    pub fn close(&self) {
        self.confirm_timer.stop();
        #[cfg(target_os = "windows")]
        {
            self.poll_timer.stop();
            if let Some(mut cap) = self.global_capture.borrow_mut().take() {
                cap.stop();
            }
        }
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
        #[cfg(target_os = "windows")]
        let status_hint =
            "Captura global ativa — pressione qualquer tecla (incl. Win+E, Alt+Tab) • Esc cancela";
        #[cfg(not(target_os = "windows"))]
        let status_hint = "Pressione a combinação (Solte ou Enter para confirmar • Esc cancela)";
        win.set_status_hint(SharedString::from(status_hint));

        let state = Rc::new(RefCell::new(LearnState::new()));
        let on_confirm = Rc::new(on_confirm);
        let on_cancel = Rc::new(on_cancel);
        let win_slot = Rc::clone(&self.window);
        let timer = Rc::clone(&self.confirm_timer);
        #[cfg(target_os = "windows")]
        let cap_slot = Rc::clone(&self.global_capture);
        #[cfg(target_os = "windows")]
        let poll_timer = Rc::clone(&self.poll_timer);

        #[cfg(target_os = "windows")]
        let cleanup_global: Rc<dyn Fn()> = {
            let cap_ref = Rc::clone(&cap_slot);
            let poll_ref = Rc::clone(&poll_timer);
            Rc::new(move || {
                poll_ref.stop();
                if let Some(mut cap) = cap_ref.borrow_mut().take() {
                    cap.stop();
                }
            })
        };
        #[cfg(not(target_os = "windows"))]
        let cleanup_global: Rc<dyn Fn()> = Rc::new(|| {});

        // key-pressed (Slint fallback / Wayland)
        {
            let weak = win.as_weak();
            let state_ref = Rc::clone(&state);
            let confirm_cb = Rc::clone(&on_confirm);
            let cancel_cb = Rc::clone(&on_cancel);
            let win_slot_ref = Rc::clone(&win_slot);
            let timer_ref = Rc::clone(&timer);
            let cleanup_ref = Rc::clone(&cleanup_global);
            win.on_key_pressed(move |text, ctrl, alt, shift, meta, repeat| {
                if !repeat {
                    timer_ref.stop();
                }
                let outcome = state_ref.borrow_mut().on_key_pressed(
                    text.as_str(),
                    ctrl,
                    alt,
                    shift,
                    meta,
                    repeat,
                );
                match outcome {
                    LearnOutcome::Updated { preview, .. } => {
                        if let Some(w) = weak.upgrade() {
                            w.set_combo_preview(SharedString::from(preview));
                        }
                    }
                    LearnOutcome::Confirmed(combo) => {
                        cleanup_ref();
                        if let Some(w) = win_slot_ref.borrow_mut().take() {
                            let _ = w.hide();
                        }
                        confirm_cb(combo);
                    }
                    LearnOutcome::Cancelled => {
                        cleanup_ref();
                        if let Some(w) = win_slot_ref.borrow_mut().take() {
                            let _ = w.hide();
                        }
                        cancel_cb();
                    }
                    LearnOutcome::Ignored => {}
                }
            });
        }

        // key-released (Slint fallback / Wayland)
        {
            let weak = win.as_weak();
            let state_ref = Rc::clone(&state);
            let confirm_cb = Rc::clone(&on_confirm);
            let win_slot_ref = Rc::clone(&win_slot);
            let timer_ref = Rc::clone(&timer);
            let cleanup_ref = Rc::clone(&cleanup_global);
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
                        let cl = Rc::clone(&cleanup_ref);
                        timer_ref.start(
                            TimerMode::SingleShot,
                            Duration::from_millis(ms),
                            move || {
                                if let Some(combo) = st.borrow_mut().on_confirm_timeout() {
                                    cl();
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

        // Cancelamento via botão "Cancelar (Esc)" na interface
        {
            let state_ref = Rc::clone(&state);
            let cancel_cb = Rc::clone(&on_cancel);
            let win_slot_ref = Rc::clone(&win_slot);
            let timer_ref = Rc::clone(&timer);
            let cleanup_ref = Rc::clone(&cleanup_global);
            win.on_cancel_requested(move || {
                timer_ref.stop();
                cleanup_ref();
                state_ref.borrow_mut().cancel();
                if let Some(w) = win_slot_ref.borrow_mut().take() {
                    let _ = w.hide();
                }
                cancel_cb();
            });
        }

        // Fechamento manual pelo botão X do WM
        {
            let state_ref = Rc::clone(&state);
            let cancel_cb = Rc::clone(&on_cancel);
            let win_slot_ref = Rc::clone(&win_slot);
            let timer_ref = Rc::clone(&timer);
            let cleanup_ref = Rc::clone(&cleanup_global);
            win.window().on_close_requested(move || {
                timer_ref.stop();
                cleanup_ref();
                state_ref.borrow_mut().cancel();
                let _ = win_slot_ref.borrow_mut().take();
                cancel_cb();
                CloseRequestResponse::HideWindow
            });
        }

        // No Windows, ativa captura global temporária (`WH_KEYBOARD_LL`) exclusivamente enquanto a janela estiver aberta
        #[cfg(target_os = "windows")]
        {
            match crate::platform::windows::input_capture::WinGlobalKeyCapture::start() {
                Ok(cap) => {
                    *cap_slot.borrow_mut() = Some(Box::new(cap));
                    let weak = win.as_weak();
                    let state_ref = Rc::clone(&state);
                    let confirm_cb = Rc::clone(&on_confirm);
                    let cancel_cb = Rc::clone(&on_cancel);
                    let win_slot_ref = Rc::clone(&win_slot);
                    let confirm_timer_ref = Rc::clone(&timer);
                    let cap_poll_ref = Rc::clone(&cap_slot);
                    let cleanup_ref = Rc::clone(&cleanup_global);
                    poll_timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
                        let events = cap_poll_ref
                            .borrow()
                            .as_ref()
                            .map(|c| c.poll_events())
                            .unwrap_or_default();
                        for ev in events {
                            if ev.pressed {
                                if !ev.repeat {
                                    confirm_timer_ref.stop();
                                }
                                let outcome = state_ref.borrow_mut().on_key_pressed(
                                    &ev.text, ev.ctrl, ev.alt, ev.shift, ev.meta, ev.repeat,
                                );
                                match outcome {
                                    LearnOutcome::Updated { preview, .. } => {
                                        if let Some(w) = weak.upgrade() {
                                            w.set_combo_preview(SharedString::from(preview));
                                        }
                                    }
                                    LearnOutcome::Confirmed(combo) => {
                                        confirm_timer_ref.stop();
                                        cleanup_ref();
                                        if let Some(w) = win_slot_ref.borrow_mut().take() {
                                            let _ = w.hide();
                                        }
                                        confirm_cb(combo);
                                        return;
                                    }
                                    LearnOutcome::Cancelled => {
                                        confirm_timer_ref.stop();
                                        cleanup_ref();
                                        if let Some(w) = win_slot_ref.borrow_mut().take() {
                                            let _ = w.hide();
                                        }
                                        cancel_cb();
                                        return;
                                    }
                                    LearnOutcome::Ignored => {}
                                }
                            } else {
                                let outcome = state_ref
                                    .borrow_mut()
                                    .on_key_released(&ev.text, ev.ctrl, ev.alt, ev.shift, ev.meta);
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
                                        let cl = Rc::clone(&cleanup_ref);
                                        confirm_timer_ref.start(
                                            TimerMode::SingleShot,
                                            Duration::from_millis(ms),
                                            move || {
                                                if let Some(combo) =
                                                    st.borrow_mut().on_confirm_timeout()
                                                {
                                                    cl();
                                                    if let Some(w) = ws.borrow_mut().take() {
                                                        let _ = w.hide();
                                                    }
                                                    cb(combo);
                                                }
                                            },
                                        );
                                    }
                                }
                            }
                        }
                    });
                }
                Err(err) => {
                    log::warn!(
                        "Captura global Win32 indisponível ({err}); usando fallback por foco Slint."
                    );
                }
            }
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
    fn captures_ctrl_shift_t_with_repeats_and_confirms_after_release_delay() {
        let mut st = LearnState::new();
        // Pressiona Ctrl puro -> preview "Ctrl"
        let out_ctrl = st.on_key_pressed("\u{0011}", true, false, false, false, false);
        assert_eq!(
            out_ctrl,
            LearnOutcome::Updated {
                preview: "Ctrl".to_string(),
                schedule_confirm_ms: None,
            }
        );
        // Evento de repetição enquanto segura Ctrl é ignorado e não trava a soltura
        assert_eq!(
            st.on_key_pressed("\u{0011}", true, false, false, false, true),
            LearnOutcome::Ignored
        );

        // Pressiona T com Ctrl+Shift
        let out_t = st.on_key_pressed("t", true, false, true, false, false);
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
    fn captures_standalone_modifiers_like_ctrl_shift_and_super() {
        // Super isolado (\u{0017}) mesmo quando `meta` ainda vem false no key-pressed
        let mut st_super = LearnState::new();
        assert_eq!(
            st_super.on_key_pressed("\u{0017}", false, false, false, false, false),
            LearnOutcome::Updated {
                preview: "Super".to_string(),
                schedule_confirm_ms: None,
            }
        );
        assert_eq!(
            st_super.on_key_released("\u{0017}", false, false, false, false),
            LearnOutcome::Updated {
                preview: "Super".to_string(),
                schedule_confirm_ms: Some(LEARN_CONFIRM_DELAY_MS),
            }
        );
        assert_eq!(st_super.on_confirm_timeout(), Some("Super".to_string()));

        // Ctrl+Shift isolados
        let mut st_mod = LearnState::new();
        let _ = st_mod.on_key_pressed("\u{0011}", true, false, false, false, false);
        let _ = st_mod.on_key_pressed("\u{0010}", true, false, true, false, false);
        let _ = st_mod.on_key_released("\u{0010}", true, false, false, false);
        assert_eq!(
            st_mod.on_key_released("\u{0011}", false, false, false, false),
            LearnOutcome::Updated {
                preview: "Ctrl+Shift".to_string(),
                schedule_confirm_ms: Some(LEARN_CONFIRM_DELAY_MS),
            }
        );
        assert_eq!(st_mod.on_confirm_timeout(), Some("Ctrl+Shift".to_string()));
    }

    #[test]
    fn captures_alt_f4_and_f5_and_confirms_immediately_on_enter() {
        let mut st = LearnState::new();
        let out = st.on_key_pressed("\u{f707}", false, true, false, false, false);
        assert_eq!(
            out,
            LearnOutcome::Updated {
                preview: "Alt+F4".to_string(),
                schedule_confirm_ms: None,
            }
        );
        let confirm = st.on_key_pressed("\u{000d}", false, false, false, false, false);
        assert_eq!(confirm, LearnOutcome::Confirmed("Alt+F4".to_string()));

        let mut st_f5 = LearnState::new();
        let out_f5 = st_f5.on_key_pressed("\u{f708}", false, false, false, false, false);
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
        let _ = st.on_key_pressed("a", true, false, false, false, false);
        assert_eq!(
            st.on_key_pressed("\u{001b}", false, false, false, false, false),
            LearnOutcome::Cancelled
        );
        assert_eq!(st.phase(), &LearnPhase::Cancelled);
        assert_eq!(st.on_confirm_timeout(), None);
    }
}
