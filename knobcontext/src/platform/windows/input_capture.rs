use crate::platform::{CapturedKeyEvent, KeyCapture};
use log::{info, warn};
use std::collections::HashSet;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PostThreadMessageW, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, WH_KEYBOARD_LL,
    WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

/// Mapeia um Virtual-Key Code Win32 (`vkCode`) para o código de texto esperado por
/// `LearnState` / `format_captured_key` (compatível com os codepoints do Slint).
pub fn vk_to_slint_text(vk: u32) -> Option<String> {
    let text = match vk {
        // Modificadores puros (codepoints internos do Slint)
        0x10 | 0xA0 => "\u{0010}".to_string(), // Shift / LShift
        0xA1 => "\u{0015}".to_string(),        // RShift
        0x11 | 0xA2 => "\u{0011}".to_string(), // Ctrl / LCtrl
        0xA3 => "\u{0016}".to_string(),        // RCtrl
        0x12 | 0xA4 => "\u{0012}".to_string(), // Alt / LAlt
        0xA5 => "\u{0013}".to_string(),        // RAlt / AltGr
        0x5B => "\u{0017}".to_string(),        // LWin / Super
        0x5C => "\u{0018}".to_string(),        // RWin / SuperR
        // Teclas especiais
        0x08 => "\u{0008}".to_string(), // Backspace
        0x09 => "\u{0009}".to_string(), // Tab
        0x0D => "\u{000d}".to_string(), // Enter / Return
        0x1B => "\u{001b}".to_string(), // Escape
        0x20 => "\u{0020}".to_string(), // Space
        0x2E => "\u{007f}".to_string(), // Delete
        0x21 => "\u{f72c}".to_string(), // PageUp
        0x22 => "\u{f72d}".to_string(), // PageDown
        0x23 => "\u{f72b}".to_string(), // End
        0x24 => "\u{f729}".to_string(), // Home
        0x25 => "\u{f702}".to_string(), // Left
        0x26 => "\u{f700}".to_string(), // Up
        0x27 => "\u{f703}".to_string(), // Right
        0x28 => "\u{f701}".to_string(), // Down
        // F1..F12
        0x70..=0x7B => {
            let cp = 0xF704 + (vk - 0x70);
            char::from_u32(cp)?.to_string()
        }
        // Dígitos 0..9
        0x30..=0x39 => char::from_u32(vk)?.to_string(),
        // Letras A..Z
        0x41..=0x5A => char::from_u32(vk)?.to_string(),
        // Teclas OEM comuns (layout US / ABNT padrão)
        0xBA => ";".to_string(),
        0xBB => "=".to_string(),
        0xBC => ",".to_string(),
        0xBD => "-".to_string(),
        0xBE => ".".to_string(),
        0xBF => "/".to_string(),
        0xC0 => "`".to_string(),
        0xDB => "[".to_string(),
        0xDC => "\\".to_string(),
        0xDD => "]".to_string(),
        0xDE => "'".to_string(),
        _ => return None,
    };
    Some(text)
}

/// Estado puro de rastreamento de teclas e modificadores para a captura global do Modo Aprender.
#[derive(Debug, Default)]
pub struct GlobalCaptureState {
    held_vks: HashSet<u32>,
    ctrl: bool,
    alt: bool,
    shift: bool,
    meta: bool,
}

impl GlobalCaptureState {
    pub fn process_vk(&mut self, vk: u32, pressed: bool) -> Option<CapturedKeyEvent> {
        let is_repeat = pressed && !self.held_vks.insert(vk);
        if !pressed {
            self.held_vks.remove(&vk);
        }

        match vk {
            0x11 | 0xA2 | 0xA3 => self.ctrl = pressed,
            0x12 | 0xA4 | 0xA5 => self.alt = pressed,
            0x10 | 0xA0 | 0xA1 => self.shift = pressed,
            0x5B | 0x5C => self.meta = pressed,
            _ => {}
        }

        let text = vk_to_slint_text(vk)?;
        Some(CapturedKeyEvent {
            text,
            ctrl: self.ctrl,
            alt: self.alt,
            shift: self.shift,
            meta: self.meta,
            pressed,
            repeat: is_repeat,
        })
    }
}

struct CaptureHookShared {
    state: GlobalCaptureState,
    tx: Sender<CapturedKeyEvent>,
    active: Arc<AtomicBool>,
}

static LEARN_HOOK_SHARED: Mutex<Option<CaptureHookShared>> = Mutex::new(None);

unsafe extern "system" fn learn_ll_keyboard_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 {
        let msg = wparam as u32;
        let pressed = match msg {
            WM_KEYDOWN | WM_SYSKEYDOWN => Some(true),
            WM_KEYUP | WM_SYSKEYUP => Some(false),
            _ => None,
        };
        if let Some(is_down) = pressed {
            let kb = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
            let injected = (kb.flags & LLKHF_INJECTED) != 0;
            if !injected {
                if let Ok(mut guard) = LEARN_HOOK_SHARED.lock() {
                    if let Some(shared) = guard.as_mut() {
                        if shared.active.load(Ordering::Relaxed) {
                            if let Some(ev) = shared.state.process_vk(kb.vkCode, is_down) {
                                let _ = shared.tx.send(ev);
                            }
                            // Engole a tecla enquanto o Modo Aprender está aberto para impedir
                            // efeitos colaterais no sistema (ex.: `Win+E` abrir Explorer, `Alt+Tab` trocar janela).
                            return 1;
                        }
                    }
                }
            }
        }
    }
    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

/// Captura global temporária baseada em `WH_KEYBOARD_LL` (Windows).
///
/// **Segurança**: o hook é instalado exclusivamente ao abrir o Modo Aprender e removido
/// imediatamente em `stop()` / `Drop` ao confirmar ou cancelar — nunca fica ativo em background.
pub struct WinGlobalKeyCapture {
    rx: Receiver<CapturedKeyEvent>,
    active: Arc<AtomicBool>,
    hook_thread_id: Arc<AtomicU32>,
    hook_thread: Option<JoinHandle<()>>,
}

impl WinGlobalKeyCapture {
    pub fn start() -> Result<Self, String> {
        let (tx, rx) = mpsc::channel::<CapturedKeyEvent>();
        let active = Arc::new(AtomicBool::new(true));
        if let Ok(mut guard) = LEARN_HOOK_SHARED.lock() {
            *guard = Some(CaptureHookShared {
                state: GlobalCaptureState::default(),
                tx,
                active: Arc::clone(&active),
            });
        }

        let hook_thread_id = Arc::new(AtomicU32::new(0));
        let tid_ref = Arc::clone(&hook_thread_id);
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

        let handle = thread::spawn(move || {
            // SAFETY: hook Win32 instalado e removido na mesma thread com loop de mensagens.
            unsafe {
                let tid = GetCurrentThreadId();
                tid_ref.store(tid, Ordering::SeqCst);

                let hook = SetWindowsHookExW(
                    WH_KEYBOARD_LL,
                    Some(learn_ll_keyboard_proc),
                    ptr::null_mut(),
                    0,
                );
                if hook.is_null() {
                    let err = std::io::Error::last_os_error();
                    let _ = ready_tx.send(Err(err.to_string()));
                    return;
                }
                let _ = ready_tx.send(Ok(()));

                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }

                UnhookWindowsHookEx(hook);
            }
        });

        match ready_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(())) => {
                info!("Captura global do Modo Aprender (WH_KEYBOARD_LL) ativada.");
                Ok(Self {
                    rx,
                    active,
                    hook_thread_id,
                    hook_thread: Some(handle),
                })
            }
            Ok(Err(err)) => {
                warn!("Falha ao ativar captura global do Modo Aprender: {err}");
                Err(err)
            }
            Err(_) => Err("timeout ao iniciar hook global do Modo Aprender".to_string()),
        }
    }
}

impl KeyCapture for WinGlobalKeyCapture {
    fn poll_events(&self) -> Vec<CapturedKeyEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = self.rx.try_recv() {
            out.push(ev);
        }
        out
    }

    fn stop(&mut self) {
        self.active.store(false, Ordering::SeqCst);
        let tid = self.hook_thread_id.swap(0, Ordering::SeqCst);
        if tid != 0 {
            // SAFETY: envia `WM_QUIT` para encerrar a thread dedicada do hook imediatamente.
            unsafe {
                PostThreadMessageW(tid, WM_QUIT, 0, 0);
            }
        }
        if let Some(handle) = self.hook_thread.take() {
            let _ = handle.join();
            info!("Captura global do Modo Aprender (WH_KEYBOARD_LL) desinstalada.");
        }
        if let Ok(mut guard) = LEARN_HOOK_SHARED.lock() {
            *guard = None;
        }
    }
}

impl Drop for WinGlobalKeyCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::{vk_to_slint_text, GlobalCaptureState};
    use crate::ui::learn_mod::{LearnOutcome, LearnState};

    #[test]
    fn captures_win_e_and_alt_tab_through_learn_state() {
        let mut cap = GlobalCaptureState::default();
        let mut learn = LearnState::new();

        // Win + E
        let ev_win_down = cap.process_vk(0x5B, true).expect("LWin down");
        let _ = learn.on_key_pressed(
            &ev_win_down.text,
            ev_win_down.ctrl,
            ev_win_down.alt,
            ev_win_down.shift,
            ev_win_down.meta,
            ev_win_down.repeat,
        );
        let ev_e_down = cap.process_vk(0x45, true).expect("E down");
        let out = learn.on_key_pressed(
            &ev_e_down.text,
            ev_e_down.ctrl,
            ev_e_down.alt,
            ev_e_down.shift,
            ev_e_down.meta,
            ev_e_down.repeat,
        );
        assert_eq!(
            out,
            LearnOutcome::Updated {
                preview: "Super+E".to_string(),
                schedule_confirm_ms: None,
            }
        );

        let ev_e_up = cap.process_vk(0x45, false).expect("E up");
        let _ = learn.on_key_released(
            &ev_e_up.text,
            ev_e_up.ctrl,
            ev_e_up.alt,
            ev_e_up.shift,
            ev_e_up.meta,
        );
        let ev_win_up = cap.process_vk(0x5B, false).expect("LWin up");
        let rel = learn.on_key_released(
            &ev_win_up.text,
            ev_win_up.ctrl,
            ev_win_up.alt,
            ev_win_up.shift,
            ev_win_up.meta,
        );
        assert!(matches!(
            rel,
            LearnOutcome::Updated {
                preview,
                schedule_confirm_ms: Some(_)
            } if preview == "Super+E"
        ));

        // Alt + Tab
        let mut cap2 = GlobalCaptureState::default();
        let mut learn2 = LearnState::new();
        let ev_alt = cap2.process_vk(0xA4, true).expect("LAlt down");
        let _ = learn2.on_key_pressed(
            &ev_alt.text,
            ev_alt.ctrl,
            ev_alt.alt,
            ev_alt.shift,
            ev_alt.meta,
            ev_alt.repeat,
        );
        let ev_tab = cap2.process_vk(0x09, true).expect("Tab down");
        let out2 = learn2.on_key_pressed(
            &ev_tab.text,
            ev_tab.ctrl,
            ev_tab.alt,
            ev_tab.shift,
            ev_tab.meta,
            ev_tab.repeat,
        );
        assert_eq!(
            out2,
            LearnOutcome::Updated {
                preview: "Alt+Tab".to_string(),
                schedule_confirm_ms: None,
            }
        );

        // Valida mapeamento de teclas OEM e F1..F12
        assert_eq!(vk_to_slint_text(0xC0).as_deref(), Some("`"));
        assert_eq!(vk_to_slint_text(0xDB).as_deref(), Some("["));
        assert_eq!(vk_to_slint_text(0xDD).as_deref(), Some("]"));
        assert_eq!(vk_to_slint_text(0x74).as_deref(), Some("\u{f708}")); // F5
    }
}
