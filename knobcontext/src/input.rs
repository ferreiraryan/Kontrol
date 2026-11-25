use std::time::{Duration, Instant};

/// Eventos de alto nível emitidos pelo dispositivo knob (3 teclas + botão rotativo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnobEvent {
    RotateCw,
    RotateCcw,
    KnobPress,
    Key1,
    Key2,
    Key3,
    OpenSettings,
}

/// Janela mínima (ms) para filtrar bounce mecânico ou eco duplicado de teclas discretas
/// (`Key1`, `Key2`, `Key3`, `KnobPress`, `OpenSettings`). Não se aplica ao giro do knob no Linux.
const BUTTON_DEBOUNCE_MS: u64 = 60;
/// Tempo sem repetição de `Ctrl` a partir do qual um `Ctrl DOWN` sem `Ctrl UP` prévio
/// é considerado um novo pressionamento (recupera automaticamente de `WM_KEYUP` perdido).
const STALE_CTRL_RECOVERY_MS: u64 = 600;
/// Janela de debounce no Windows para giros consecutivos na mesma direção (filtra bounce EC11 / eco `hidserv`).
#[cfg(any(target_os = "windows", test))]
const WIN_ROTATE_DEBOUNCE_MS: u64 = 45;
/// Janela de debounce no Windows para cliques discretos (`KnobPress`, `Key1`, `Key2`, `Key3`).
#[cfg(any(target_os = "windows", test))]
const WIN_BUTTON_DEBOUNCE_MS: u64 = 140;
/// Tempo máximo para manter `mute_down` / `play_pause_down` sem `WM_KEYUP` (evita bloqueio permanente se o SO perder o `UP`).
#[cfg(any(target_os = "windows", test))]
const WIN_STUCK_DOWN_TIMEOUT_MS: u64 = 350;

/// Teclas físicas normalizadas independentes de plataforma (`evdev` no Linux, `WH_KEYBOARD_LL` no Windows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawKey {
    Ctrl,
    VolumeUp,
    VolumeDown,
    Mute,
    PlayPause,
    F1,
    F4,
    #[cfg(target_os = "linux")]
    Other,
}

/// Máquina de estados pura que traduz eventos brutos de tecla em `KnobEvent`.
///
/// Trata combos como `Ctrl+F1` (`Key1`), `Ctrl+F4` (`Key2`) e `Key1+Key2` (`OpenSettings`)
/// exigindo que a segunda tecla chegue dentro de `combo_window` enquanto `Ctrl` está
/// pressionado, e emitindo o `KnobEvent` somente quando o modificador (`Ctrl`) é liberado
/// para evitar estado residual de modificadores presos.
#[derive(Debug)]
pub struct EventTranslator {
    combo_window: Duration,
    ctrl_pressed_at: Option<Instant>,
    last_ctrl_down_at: Option<Instant>,
    f1_down: bool,
    f4_down: bool,
    pending_combo: Option<KnobEvent>,
    last_button_event: Option<(KnobEvent, Instant)>,
}

impl EventTranslator {
    pub fn new(combo_window: Duration) -> Self {
        Self {
            combo_window,
            ctrl_pressed_at: None,
            last_ctrl_down_at: None,
            f1_down: false,
            f4_down: false,
            pending_combo: None,
            last_button_event: None,
        }
    }

    #[cfg(any(target_os = "windows", test))]
    pub fn is_ctrl_combo_active(&self, now: Instant) -> bool {
        self.ctrl_pressed_at.is_some()
            && (self.is_within_combo_window(now)
                || self.f1_down
                || self.f4_down
                || self.pending_combo.is_some())
    }

    pub fn process_raw_key(
        &mut self,
        key: RawKey,
        pressed: bool,
        now: Instant,
    ) -> Option<KnobEvent> {
        let raw_event = match (key, pressed) {
            (RawKey::Ctrl, true) => {
                let stale_ctrl_up = self.last_ctrl_down_at.is_some_and(|prev| {
                    now.saturating_duration_since(prev)
                        > Duration::from_millis(STALE_CTRL_RECOVERY_MS)
                });
                if self.ctrl_pressed_at.is_none()
                    || (stale_ctrl_up
                        && !self.f1_down
                        && !self.f4_down
                        && self.pending_combo.is_none())
                {
                    self.ctrl_pressed_at = Some(now);
                }
                self.last_ctrl_down_at = Some(now);
                None
            }
            (RawKey::Ctrl, false) => {
                self.ctrl_pressed_at = None;
                self.last_ctrl_down_at = None;
                self.f1_down = false;
                self.f4_down = false;
                self.pending_combo.take()
            }
            (RawKey::VolumeUp, true) => Some(KnobEvent::RotateCw),
            (RawKey::VolumeDown, true) => Some(KnobEvent::RotateCcw),
            (RawKey::Mute, true) => Some(KnobEvent::KnobPress),
            (RawKey::PlayPause, true) => Some(KnobEvent::Key3),
            (RawKey::F4, true) => {
                if self.ctrl_pressed_at.is_some() {
                    self.f4_down = true;
                    if self.f1_down || self.pending_combo == Some(KnobEvent::Key1) {
                        self.pending_combo = Some(KnobEvent::OpenSettings);
                    } else if self.is_within_combo_window(now) {
                        self.pending_combo = Some(KnobEvent::Key2);
                    }
                }
                None
            }
            (RawKey::F4, false) => {
                self.f4_down = false;
                None
            }
            (RawKey::F1, true) => {
                if self.ctrl_pressed_at.is_some() {
                    self.f1_down = true;
                    if self.f4_down || self.pending_combo == Some(KnobEvent::Key2) {
                        self.pending_combo = Some(KnobEvent::OpenSettings);
                    } else if self.is_within_combo_window(now) {
                        self.pending_combo = Some(KnobEvent::Key1);
                    }
                }
                None
            }
            (RawKey::F1, false) => {
                self.f1_down = false;
                None
            }
            _ => None,
        };

        let event = raw_event?;
        if !matches!(event, KnobEvent::RotateCw | KnobEvent::RotateCcw) {
            if let Some((prev, prev_at)) = self.last_button_event {
                if prev == event
                    && now.saturating_duration_since(prev_at)
                        < Duration::from_millis(BUTTON_DEBOUNCE_MS)
                {
                    return None;
                }
            }
            self.last_button_event = Some((event, now));
        }
        Some(event)
    }

    #[cfg(target_os = "linux")]
    pub fn process_input_event(
        &mut self,
        event: evdev::InputEvent,
        now: Instant,
    ) -> Option<KnobEvent> {
        if event.event_type() != evdev::EventType::KEY {
            return None;
        }
        self.process_key(evdev::Key::new(event.code()), event.value(), now)
    }

    #[cfg(target_os = "linux")]
    pub fn process_key(&mut self, key: evdev::Key, value: i32, now: Instant) -> Option<KnobEvent> {
        // value: 1 = press, 0 = release, 2 = repeat
        let pressed = match value {
            1 => true,
            0 => false,
            _ => return None,
        };
        let raw = match key {
            evdev::Key::KEY_LEFTCTRL | evdev::Key::KEY_RIGHTCTRL => RawKey::Ctrl,
            evdev::Key::KEY_VOLUMEUP => RawKey::VolumeUp,
            evdev::Key::KEY_VOLUMEDOWN => RawKey::VolumeDown,
            evdev::Key::KEY_MUTE => RawKey::Mute,
            evdev::Key::KEY_PLAYPAUSE => RawKey::PlayPause,
            evdev::Key::KEY_F1 => RawKey::F1,
            evdev::Key::KEY_F4 => RawKey::F4,
            _ => RawKey::Other,
        };
        self.process_raw_key(raw, pressed, now)
    }

    fn is_within_combo_window(&self, now: Instant) -> bool {
        self.ctrl_pressed_at.is_some_and(|pressed_at| {
            now.saturating_duration_since(pressed_at) <= self.combo_window
        })
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::{EventTranslator, KnobEvent};
    use crate::config::DeviceConfig;
    use crate::error::KnobError;
    use evdev::Device;
    use std::io;
    use std::os::fd::AsRawFd;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    /// Wrapper RAII sobre um `evdev::Device` capturado com `grab()`, garantindo `ungrab()` no `Drop`.
    struct GrabbedDevice {
        path: PathBuf,
        device: Device,
        grabbed: bool,
    }

    impl GrabbedDevice {
        fn open_and_grab(path: &Path) -> Result<Self, KnobError> {
            let mut device = Device::open(path).map_err(|source| KnobError::DeviceOpen {
                path: path.to_path_buf(),
                source,
            })?;

            device.grab().map_err(|source| KnobError::DeviceGrab {
                path: path.to_path_buf(),
                source,
            })?;

            set_nonblocking(device.as_raw_fd()).map_err(|source| KnobError::DeviceOpen {
                path: path.to_path_buf(),
                source,
            })?;

            Ok(Self {
                path: path.to_path_buf(),
                device,
                grabbed: true,
            })
        }

        fn as_raw_fd(&self) -> i32 {
            self.device.as_raw_fd()
        }

        fn drain_events(
            &mut self,
            translator: &mut EventTranslator,
            out: &mut Vec<KnobEvent>,
        ) -> Result<(), KnobError> {
            loop {
                match self.device.fetch_events() {
                    Ok(events) => {
                        let now = Instant::now();
                        for ev in events {
                            if let Some(knob_event) = translator.process_input_event(ev, now) {
                                out.push(knob_event);
                            }
                        }
                    }
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                    Err(source) => {
                        return Err(KnobError::DeviceRead {
                            path: self.path.clone(),
                            source,
                        });
                    }
                }
            }
            Ok(())
        }
    }

    impl Drop for GrabbedDevice {
        fn drop(&mut self) {
            if self.grabbed {
                let _ = self.device.ungrab();
                self.grabbed = false;
            }
        }
    }

    fn set_nonblocking(fd: i32) -> io::Result<()> {
        // SAFETY: `fd` é um descritor de arquivo válido obtido de `evdev::Device::as_raw_fd()`.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: aplica `O_NONBLOCK` ao mesmo descritor válido.
        let res = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
        if res < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Gerencia os dois nós `evdev` do knob com `grab()` exclusivo e leitura multiplexada via `poll`.
    pub struct KnobInput {
        kbd: GrabbedDevice,
        media: GrabbedDevice,
        translator: EventTranslator,
    }

    impl KnobInput {
        pub fn open(config: &DeviceConfig) -> Result<Self, KnobError> {
            let kbd = GrabbedDevice::open_and_grab(&config.kbd_path)?;
            let media = GrabbedDevice::open_and_grab(&config.media_path)?;
            let translator = EventTranslator::new(Duration::from_millis(config.combo_window_ms));
            Ok(Self {
                kbd,
                media,
                translator,
            })
        }

        /// Aguarda eventos por até `timeout_ms` milissegundos e retorna todos os `KnobEvent` traduzidos.
        pub fn poll_events(&mut self, timeout_ms: i32) -> Result<Vec<KnobEvent>, KnobError> {
            let mut fds = [
                libc::pollfd {
                    fd: self.kbd.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: self.media.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];

            // SAFETY: `fds` é um array válido de 2 `pollfd` com descritores abertos.
            let ready =
                unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
            if ready < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    return Ok(Vec::new());
                }
                return Err(KnobError::DeviceRead {
                    path: self.kbd.path.clone(),
                    source: err,
                });
            }
            if ready == 0 {
                return Ok(Vec::new());
            }

            let mut events = Vec::new();
            if (fds[0].revents & libc::POLLIN) != 0 {
                self.kbd.drain_events(&mut self.translator, &mut events)?;
            }
            if (fds[1].revents & libc::POLLIN) != 0 {
                self.media.drain_events(&mut self.translator, &mut events)?;
            }
            Ok(events)
        }
    }
}

#[cfg(any(target_os = "windows", test))]
pub const ENIGO_EXTRA_INFO_MARKER: usize = 0x4B4E_4F42;
#[cfg(any(target_os = "windows", test))]
pub(crate) const VK_CONTROL_CODE: u32 = 0x11;
#[cfg(any(target_os = "windows", test))]
pub(crate) const VK_F1_CODE: u32 = 0x70;
#[cfg(any(target_os = "windows", test))]
pub(crate) const VK_F4_CODE: u32 = 0x73;
#[cfg(any(target_os = "windows", test))]
pub(crate) const VK_LCONTROL_CODE: u32 = 0xA2;
#[cfg(any(target_os = "windows", test))]
pub(crate) const VK_RCONTROL_CODE: u32 = 0xA3;
#[cfg(any(target_os = "windows", test))]
pub(crate) const VK_VOLUME_MUTE_CODE: u32 = 0xAD;
#[cfg(any(target_os = "windows", test))]
pub(crate) const VK_VOLUME_DOWN_CODE: u32 = 0xAE;
#[cfg(any(target_os = "windows", test))]
pub(crate) const VK_VOLUME_UP_CODE: u32 = 0xAF;
#[cfg(any(target_os = "windows", test))]
pub(crate) const VK_MEDIA_PLAY_PAUSE_CODE: u32 = 0xB3;

/// Máquina de estados pura para o hook `WH_KEYBOARD_LL` no Windows.
///
/// Decide quais eventos de hardware devem ser bloqueados (engolidos) para impedir
/// que os atalhos de firmware do dispositivo `1189:8890` vazem para o Windows,
/// preservando teclas injetadas pelo próprio `EnigoExecutor` (identificadas de forma
/// determinística por `dwExtraInfo == ENIGO_EXTRA_INFO_MARKER`), filtrando auto-repeat
/// do Windows em botões discretos e preservando o uso normal de `Ctrl`/`F1`/`F4`.
#[cfg(any(target_os = "windows", test))]
#[derive(Debug)]
pub(crate) struct WinHookState {
    translator: EventTranslator,
    mute_down_at: Option<Instant>,
    play_pause_down_at: Option<Instant>,
    last_rotate: Option<(KnobEvent, Instant)>,
    last_button: Option<(KnobEvent, Instant)>,
    swallowed_f1: bool,
    swallowed_f4: bool,
}

#[cfg(any(target_os = "windows", test))]
impl WinHookState {
    pub(crate) fn new(combo_window: Duration) -> Self {
        Self {
            translator: EventTranslator::new(combo_window),
            mute_down_at: None,
            play_pause_down_at: None,
            last_rotate: None,
            last_button: None,
            swallowed_f1: false,
            swallowed_f4: false,
        }
    }

    fn filter_win_event(&mut self, event: Option<KnobEvent>, now: Instant) -> Option<KnobEvent> {
        let ev = event?;
        match ev {
            KnobEvent::RotateCw | KnobEvent::RotateCcw => {
                if let Some((prev, prev_at)) = self.last_rotate {
                    if prev == ev
                        && now.saturating_duration_since(prev_at)
                            < Duration::from_millis(WIN_ROTATE_DEBOUNCE_MS)
                    {
                        return None;
                    }
                }
                self.last_rotate = Some((ev, now));
                Some(ev)
            }
            _ => {
                if let Some((prev, prev_at)) = self.last_button {
                    if prev == ev
                        && now.saturating_duration_since(prev_at)
                            < Duration::from_millis(WIN_BUTTON_DEBOUNCE_MS)
                    {
                        return None;
                    }
                }
                self.last_button = Some((ev, now));
                Some(ev)
            }
        }
    }

    /// Processa uma tecla recebida pelo `WH_KEYBOARD_LL`.
    /// Retorna `(swallow, Option<KnobEvent>)`.
    ///
    /// Nota: no Windows, o serviço HID (`hidserv`) sintetiza `VK_VOLUME_*` e `VK_MEDIA_PLAY_PAUSE`
    /// a partir da interface Consumer Control (`0x000C`) com `LLKHF_INJECTED` (`flags=0x11`).
    /// Por isso, eventos originados pelo próprio `EnigoExecutor` são distinguidos sem race
    /// condition via `dwExtraInfo == ENIGO_EXTRA_INFO_MARKER` (`self_injecting == true`).
    pub(crate) fn handle_key(
        &mut self,
        vk: u32,
        pressed: bool,
        injected: bool,
        active: bool,
        self_injecting: bool,
        now: Instant,
    ) -> (bool, Option<KnobEvent>) {
        if !active {
            self.mute_down_at = None;
            self.play_pause_down_at = None;
            return (false, None);
        }
        if self_injecting {
            return (false, None);
        }

        let (swallow, raw_ev) = match vk {
            VK_VOLUME_UP_CODE => (
                true,
                self.translator
                    .process_raw_key(RawKey::VolumeUp, pressed, now),
            ),
            VK_VOLUME_DOWN_CODE => (
                true,
                self.translator
                    .process_raw_key(RawKey::VolumeDown, pressed, now),
            ),
            VK_VOLUME_MUTE_CODE => {
                if pressed {
                    let still_held = self.mute_down_at.is_some_and(|t| {
                        now.saturating_duration_since(t)
                            < Duration::from_millis(WIN_STUCK_DOWN_TIMEOUT_MS)
                    });
                    if still_held {
                        return (true, None);
                    }
                    self.mute_down_at = Some(now);
                } else {
                    self.mute_down_at = None;
                }
                (
                    true,
                    self.translator.process_raw_key(RawKey::Mute, pressed, now),
                )
            }
            VK_MEDIA_PLAY_PAUSE_CODE => {
                if pressed {
                    let still_held = self.play_pause_down_at.is_some_and(|t| {
                        now.saturating_duration_since(t)
                            < Duration::from_millis(WIN_STUCK_DOWN_TIMEOUT_MS)
                    });
                    if still_held {
                        return (true, None);
                    }
                    self.play_pause_down_at = Some(now);
                } else {
                    self.play_pause_down_at = None;
                }
                (
                    true,
                    self.translator
                        .process_raw_key(RawKey::PlayPause, pressed, now),
                )
            }
            VK_LCONTROL_CODE | VK_RCONTROL_CODE | VK_CONTROL_CODE => {
                if injected {
                    return (false, None);
                }
                // Nunca engole `Ctrl` para não quebrar atalhos comuns (`Ctrl+C`, `Ctrl+V`, etc.).
                let ev = self.translator.process_raw_key(RawKey::Ctrl, pressed, now);
                (false, ev)
            }
            VK_F1_CODE => {
                if injected {
                    return (false, None);
                }
                if pressed {
                    if self.translator.is_ctrl_combo_active(now) {
                        self.swallowed_f1 = true;
                        let ev = self.translator.process_raw_key(RawKey::F1, true, now);
                        (true, ev)
                    } else {
                        (false, None)
                    }
                } else {
                    let ev = self.translator.process_raw_key(RawKey::F1, false, now);
                    let swallow = self.swallowed_f1;
                    self.swallowed_f1 = false;
                    (swallow, ev)
                }
            }
            VK_F4_CODE => {
                if injected {
                    return (false, None);
                }
                if pressed {
                    if self.translator.is_ctrl_combo_active(now) {
                        self.swallowed_f4 = true;
                        let ev = self.translator.process_raw_key(RawKey::F4, true, now);
                        (true, ev)
                    } else {
                        (false, None)
                    }
                } else {
                    let ev = self.translator.process_raw_key(RawKey::F4, false, now);
                    let swallow = self.swallowed_f4;
                    self.swallowed_f4 = false;
                    (swallow, ev)
                }
            }
            _ => (false, None),
        };
        (swallow, self.filter_win_event(raw_ev, now))
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::{KnobEvent, WinHookState, ENIGO_EXTRA_INFO_MARKER};
    use crate::config::DeviceConfig;
    use crate::error::KnobError;
    use hidapi::HidApi;
    use log::{debug, info, warn};
    use std::ptr;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, PostThreadMessageW, SetWindowsHookExW,
        TranslateMessage, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG,
        WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    const KNOB_VID: u16 = 0x1189;
    const KNOB_PID: u16 = 0x8890;
    const RESCAN_CONNECTED_INTERVAL: Duration = Duration::from_secs(5);
    const RESCAN_DISCONNECTED_INTERVAL: Duration = Duration::from_secs(2);

    struct HookShared {
        state: WinHookState,
        tx: Sender<KnobEvent>,
        device_connected: Arc<AtomicBool>,
    }

    static HOOK_SHARED: Mutex<Option<HookShared>> = Mutex::new(None);

    unsafe extern "system" fn ll_keyboard_proc(
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
                let self_injecting = kb.dwExtraInfo == ENIGO_EXTRA_INFO_MARKER;
                if self_injecting {
                    return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
                }
                let injected = (kb.flags & LLKHF_INJECTED) != 0;

                if let Ok(mut guard) = HOOK_SHARED.lock() {
                    if let Some(shared) = guard.as_mut() {
                        let active = shared.device_connected.load(Ordering::Relaxed);
                        let (swallow, event) = shared.state.handle_key(
                            kb.vkCode,
                            is_down,
                            injected,
                            active,
                            false,
                            Instant::now(),
                        );
                        if let Some(ev) = event {
                            debug!(
                                "WH_KEYBOARD_LL -> {:?} (vk=0x{:02X} flags=0x{:02X} extra=0x{:X})",
                                ev, kb.vkCode, kb.flags, kb.dwExtraInfo
                            );
                            let _ = shared.tx.send(ev);
                        }
                        if swallow {
                            return 1;
                        }
                    }
                }
            }
        }
        unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
    }

    fn is_knob_present(api: &mut HidApi) -> bool {
        if let Err(err) = api.refresh_devices() {
            debug!("Falha ao atualizar lista HID: {err}");
            return false;
        }
        api.device_list()
            .any(|info| info.vendor_id() == KNOB_VID && info.product_id() == KNOB_PID)
    }

    /// Gerencia a captura exclusiva (via `WH_KEYBOARD_LL`) dos eventos do knob `1189:8890` no Windows.
    pub struct KnobInput {
        api: Option<HidApi>,
        rx: Receiver<KnobEvent>,
        device_connected: Arc<AtomicBool>,
        hook_thread_id: Arc<AtomicU32>,
        hook_thread: Option<JoinHandle<()>>,
        last_scan: Instant,
    }

    impl KnobInput {
        pub fn open(config: &DeviceConfig) -> Result<Self, KnobError> {
            let mut api = match HidApi::new() {
                Ok(api) => Some(api),
                Err(err) => {
                    warn!("Falha ao inicializar HidApi no Windows: {err}.");
                    None
                }
            };

            let connected_now = api.as_mut().is_some_and(is_knob_present);
            let device_connected = Arc::new(AtomicBool::new(connected_now));
            if connected_now {
                info!(
                    "Dispositivo HID {:04x}:{:04x} detectado. Interceptação Win32 (WH_KEYBOARD_LL) ativada.",
                    KNOB_VID, KNOB_PID
                );
            } else {
                warn!(
                    "Dispositivo HID {:04x}:{:04x} não detectado no momento. Interceptação aguardando conexão.",
                    KNOB_VID, KNOB_PID
                );
            }

            let (tx, rx) = mpsc::channel::<KnobEvent>();
            let combo_window = Duration::from_millis(config.combo_window_ms);
            if let Ok(mut guard) = HOOK_SHARED.lock() {
                *guard = Some(HookShared {
                    state: WinHookState::new(combo_window),
                    tx,
                    device_connected: Arc::clone(&device_connected),
                });
            }

            let hook_thread_id = Arc::new(AtomicU32::new(0));
            let hook_tid_clone = Arc::clone(&hook_thread_id);
            let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

            let hook_thread = thread::spawn(move || {
                // SAFETY: APIs Win32 padrão; hook instalado e removido na mesma thread com loop de mensagens.
                unsafe {
                    let tid = GetCurrentThreadId();
                    hook_tid_clone.store(tid, Ordering::SeqCst);

                    let hook = SetWindowsHookExW(
                        WH_KEYBOARD_LL,
                        Some(ll_keyboard_proc),
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
                    info!("Hook Win32 WH_KEYBOARD_LL instalado (atalhos de firmware bloqueados).");
                }
                Ok(Err(err)) => {
                    warn!("Falha ao instalar hook WH_KEYBOARD_LL: {err}.");
                }
                Err(_) => {
                    warn!("Timeout ao inicializar thread do hook WH_KEYBOARD_LL.");
                }
            }

            Ok(Self {
                api,
                rx,
                device_connected,
                hook_thread_id,
                hook_thread: Some(hook_thread),
                last_scan: Instant::now(),
            })
        }

        fn refresh_presence_if_due(&mut self) {
            let interval = if self.device_connected.load(Ordering::Relaxed) {
                RESCAN_CONNECTED_INTERVAL
            } else {
                RESCAN_DISCONNECTED_INTERVAL
            };
            if self.last_scan.elapsed() < interval {
                return;
            }
            self.last_scan = Instant::now();
            let Some(api) = self.api.as_mut() else {
                return;
            };
            let present = is_knob_present(api);
            let prev = self.device_connected.swap(present, Ordering::Relaxed);
            if present && !prev {
                info!(
                    "Dispositivo HID {:04x}:{:04x} reconectado. Interceptação ativada.",
                    KNOB_VID, KNOB_PID
                );
            } else if !present && prev {
                warn!(
                    "Dispositivo HID {:04x}:{:04x} desconectado. Interceptação pausada.",
                    KNOB_VID, KNOB_PID
                );
            }
        }

        pub fn poll_events(&mut self, timeout_ms: i32) -> Result<Vec<KnobEvent>, KnobError> {
            self.refresh_presence_if_due();

            let mut events = Vec::new();
            let wait = Duration::from_millis(timeout_ms.clamp(5, 100) as u64);
            match self.rx.recv_timeout(wait) {
                Ok(ev) => {
                    events.push(ev);
                    while let Ok(more) = self.rx.try_recv() {
                        events.push(more);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {}
            }
            Ok(events)
        }
    }

    impl Drop for KnobInput {
        fn drop(&mut self) {
            self.device_connected.store(false, Ordering::SeqCst);
            let tid = self.hook_thread_id.load(Ordering::SeqCst);
            if tid != 0 {
                // SAFETY: envia `WM_QUIT` para encerrar o loop `GetMessageW` da thread do hook.
                unsafe {
                    PostThreadMessageW(tid, WM_QUIT, 0, 0);
                }
            }
            if let Some(handle) = self.hook_thread.take() {
                let _ = handle.join();
            }
            if let Ok(mut guard) = HOOK_SHARED.lock() {
                *guard = None;
            }
        }
    }
}

pub use platform::KnobInput;

#[cfg(test)]
mod tests {
    use super::{
        EventTranslator, KnobEvent, RawKey, WinHookState, VK_F1_CODE, VK_F4_CODE, VK_LCONTROL_CODE,
        VK_MEDIA_PLAY_PAUSE_CODE, VK_VOLUME_DOWN_CODE, VK_VOLUME_MUTE_CODE, VK_VOLUME_UP_CODE,
    };
    use std::time::{Duration, Instant};

    #[test]
    fn translates_single_key_and_knob_events() {
        let mut tr = EventTranslator::new(Duration::from_millis(200));
        let t0 = Instant::now();

        assert_eq!(
            tr.process_raw_key(RawKey::VolumeUp, true, t0),
            Some(KnobEvent::RotateCw)
        );
        assert_eq!(tr.process_raw_key(RawKey::VolumeUp, false, t0), None);
        assert_eq!(
            tr.process_raw_key(RawKey::VolumeDown, true, t0),
            Some(KnobEvent::RotateCcw)
        );
        assert_eq!(
            tr.process_raw_key(RawKey::Mute, true, t0),
            Some(KnobEvent::KnobPress)
        );
        assert_eq!(
            tr.process_raw_key(RawKey::PlayPause, true, t0),
            Some(KnobEvent::Key3)
        );
    }

    #[test]
    fn translates_ctrl_combos_within_window() {
        let mut tr = EventTranslator::new(Duration::from_millis(200));
        let t0 = Instant::now();

        assert_eq!(tr.process_raw_key(RawKey::Ctrl, true, t0), None);
        assert_eq!(
            tr.process_raw_key(RawKey::F4, true, t0 + Duration::from_millis(50)),
            None
        );
        assert_eq!(
            tr.process_raw_key(RawKey::F4, false, t0 + Duration::from_millis(55)),
            None
        );
        assert_eq!(
            tr.process_raw_key(RawKey::Ctrl, false, t0 + Duration::from_millis(60)),
            Some(KnobEvent::Key2)
        );

        let t1 = t0 + Duration::from_millis(500);
        assert_eq!(tr.process_raw_key(RawKey::Ctrl, true, t1), None);
        assert_eq!(
            tr.process_raw_key(RawKey::F1, true, t1 + Duration::from_millis(10)),
            None
        );
        assert_eq!(
            tr.process_raw_key(RawKey::Ctrl, false, t1 + Duration::from_millis(20)),
            Some(KnobEvent::Key1)
        );
    }

    #[test]
    fn ignores_lone_ctrl_expired_combo_and_unmodified_f_keys() {
        let mut tr = EventTranslator::new(Duration::from_millis(200));
        let t0 = Instant::now();

        assert_eq!(tr.process_raw_key(RawKey::Ctrl, true, t0), None);
        assert_eq!(
            tr.process_raw_key(RawKey::Ctrl, false, t0 + Duration::from_millis(20)),
            None
        );
        assert_eq!(
            tr.process_raw_key(RawKey::F4, true, t0 + Duration::from_millis(40)),
            None
        );

        let t2 = t0 + Duration::from_millis(300);
        assert_eq!(tr.process_raw_key(RawKey::Ctrl, true, t2), None);
        assert_eq!(
            tr.process_raw_key(RawKey::F1, true, t2 + Duration::from_millis(250)),
            None
        );
    }

    #[test]
    fn emits_open_settings_when_key1_and_key2_pressed_simultaneously() {
        let mut tr = EventTranslator::new(Duration::from_millis(200));
        let t0 = Instant::now();

        assert_eq!(tr.process_raw_key(RawKey::Ctrl, true, t0), None);
        assert_eq!(
            tr.process_raw_key(RawKey::F1, true, t0 + Duration::from_millis(10)),
            None
        );
        assert_eq!(
            tr.process_raw_key(RawKey::F4, true, t0 + Duration::from_millis(30)),
            None
        );
        assert_eq!(
            tr.process_raw_key(RawKey::Ctrl, false, t0 + Duration::from_millis(40)),
            Some(KnobEvent::OpenSettings)
        );
    }

    #[test]
    fn debounces_rapid_duplicate_button_presses_without_affecting_rotation() {
        let mut tr = EventTranslator::new(Duration::from_millis(200));
        let t0 = Instant::now();

        // Primeiro Key1 emite normalmente
        let _ = tr.process_raw_key(RawKey::Ctrl, true, t0);
        let _ = tr.process_raw_key(RawKey::F1, true, t0 + Duration::from_millis(5));
        assert_eq!(
            tr.process_raw_key(RawKey::Ctrl, false, t0 + Duration::from_millis(10)),
            Some(KnobEvent::Key1)
        );

        // Bounce / eco 25ms depois é suprimido
        let t_bounce = t0 + Duration::from_millis(35);
        let _ = tr.process_raw_key(RawKey::Ctrl, true, t_bounce);
        let _ = tr.process_raw_key(RawKey::F1, true, t_bounce + Duration::from_millis(2));
        assert_eq!(
            tr.process_raw_key(RawKey::Ctrl, false, t_bounce + Duration::from_millis(5)),
            None
        );

        // Novo pressionamento intencional após 80ms funciona normalmente
        let t_next = t0 + Duration::from_millis(90);
        let _ = tr.process_raw_key(RawKey::Ctrl, true, t_next);
        let _ = tr.process_raw_key(RawKey::F1, true, t_next + Duration::from_millis(5));
        assert_eq!(
            tr.process_raw_key(RawKey::Ctrl, false, t_next + Duration::from_millis(10)),
            Some(KnobEvent::Key1)
        );

        // Giros rápidos consecutivos (10ms) NÃO são suprimidos
        assert_eq!(
            tr.process_raw_key(RawKey::VolumeUp, true, t0),
            Some(KnobEvent::RotateCw)
        );
        assert_eq!(
            tr.process_raw_key(RawKey::VolumeUp, true, t0 + Duration::from_millis(10)),
            Some(KnobEvent::RotateCw)
        );
    }

    #[test]
    fn win_hook_swallows_hardware_media_and_ctrl_f_combos_but_allows_injected_and_plain_keys() {
        let mut hook = WinHookState::new(Duration::from_millis(200));
        let t0 = Instant::now();

        // No Windows, `hidserv` sintetiza `VK_VOLUME_*` com `injected=true`; deve ser engolido e traduzido
        assert_eq!(
            hook.handle_key(VK_VOLUME_UP_CODE, true, true, true, false, t0),
            (true, Some(KnobEvent::RotateCw))
        );
        assert_eq!(
            hook.handle_key(VK_VOLUME_UP_CODE, false, true, true, false, t0),
            (true, None)
        );
        assert_eq!(
            hook.handle_key(VK_VOLUME_DOWN_CODE, true, true, true, false, t0),
            (true, Some(KnobEvent::RotateCcw))
        );
        assert_eq!(
            hook.handle_key(VK_VOLUME_MUTE_CODE, true, true, true, false, t0),
            (true, Some(KnobEvent::KnobPress))
        );
        assert_eq!(
            hook.handle_key(VK_MEDIA_PLAY_PAUSE_CODE, true, false, true, false, t0),
            (true, Some(KnobEvent::Key3))
        );

        // Eventos injetados pelo próprio `EnigoExecutor` (self_injecting=true) NUNCA são engolidos
        assert_eq!(
            hook.handle_key(VK_VOLUME_UP_CODE, true, true, true, true, t0),
            (false, None)
        );

        // Quando dispositivo está desconectado (active=false), nada é engolido
        assert_eq!(
            hook.handle_key(VK_VOLUME_UP_CODE, true, false, false, false, t0),
            (false, None)
        );

        // F1 / F4 sem Ctrl ou injetados não são engolidos
        assert_eq!(
            hook.handle_key(VK_F1_CODE, true, false, true, false, t0),
            (false, None)
        );
        assert_eq!(
            hook.handle_key(VK_F4_CODE, true, false, true, false, t0),
            (false, None)
        );
        assert_eq!(
            hook.handle_key(VK_F4_CODE, true, true, true, false, t0),
            (false, None)
        );

        // Ctrl+F4 dentro da janela: Ctrl passa, F4 (down/up) é engolido e Key2 sai no Ctrl up
        assert_eq!(
            hook.handle_key(VK_LCONTROL_CODE, true, false, true, false, t0),
            (false, None)
        );
        assert_eq!(
            hook.handle_key(
                VK_F4_CODE,
                true,
                false,
                true,
                false,
                t0 + Duration::from_millis(15)
            ),
            (true, None)
        );
        assert_eq!(
            hook.handle_key(
                VK_F4_CODE,
                false,
                false,
                true,
                false,
                t0 + Duration::from_millis(20)
            ),
            (true, None)
        );
        assert_eq!(
            hook.handle_key(
                VK_LCONTROL_CODE,
                false,
                false,
                true,
                false,
                t0 + Duration::from_millis(25)
            ),
            (false, Some(KnobEvent::Key2))
        );
    }

    #[test]
    fn win_hook_ignores_auto_repeat_on_buttons_and_passes_all_self_injected_keys() {
        let mut hook = WinHookState::new(Duration::from_millis(200));
        let t0 = Instant::now();

        // Auto-repeat (múltiplos WM_KEYDOWN antes do WM_KEYUP) em Mute e PlayPause não duplica
        assert_eq!(
            hook.handle_key(VK_VOLUME_MUTE_CODE, true, true, true, false, t0),
            (true, Some(KnobEvent::KnobPress))
        );
        assert_eq!(
            hook.handle_key(
                VK_VOLUME_MUTE_CODE,
                true,
                true,
                true,
                false,
                t0 + Duration::from_millis(120)
            ),
            (true, None)
        );
        assert_eq!(
            hook.handle_key(
                VK_VOLUME_MUTE_CODE,
                false,
                true,
                true,
                false,
                t0 + Duration::from_millis(150)
            ),
            (true, None)
        );

        assert_eq!(
            hook.handle_key(VK_MEDIA_PLAY_PAUSE_CODE, true, true, true, false, t0),
            (true, Some(KnobEvent::Key3))
        );
        assert_eq!(
            hook.handle_key(
                VK_MEDIA_PLAY_PAUSE_CODE,
                true,
                true,
                true,
                false,
                t0 + Duration::from_millis(120)
            ),
            (true, None)
        );
        assert_eq!(
            hook.handle_key(
                VK_MEDIA_PLAY_PAUSE_CODE,
                false,
                true,
                true,
                false,
                t0 + Duration::from_millis(150)
            ),
            (true, None)
        );

        // Qualquer evento marcado com dwExtraInfo do próprio Enigo (self_injecting=true) passa livre
        for vk in [
            VK_VOLUME_UP_CODE,
            VK_VOLUME_DOWN_CODE,
            VK_VOLUME_MUTE_CODE,
            VK_MEDIA_PLAY_PAUSE_CODE,
            VK_LCONTROL_CODE,
            VK_F1_CODE,
            VK_F4_CODE,
        ] {
            assert_eq!(
                hook.handle_key(vk, true, true, true, true, t0),
                (false, None)
            );
            assert_eq!(
                hook.handle_key(vk, false, true, true, true, t0),
                (false, None)
            );
        }
    }

    #[test]
    fn win_hook_filters_rotation_and_button_bounce_and_recovers_from_lost_keyup() {
        let mut hook = WinHookState::new(Duration::from_millis(200));
        let t0 = Instant::now();

        // Giro CW + bounce 20ms depois -> apenas 1 RotateCw; reversão imediata CCW passa
        assert_eq!(
            hook.handle_key(VK_VOLUME_UP_CODE, true, true, true, false, t0),
            (true, Some(KnobEvent::RotateCw))
        );
        assert_eq!(
            hook.handle_key(
                VK_VOLUME_UP_CODE,
                true,
                true,
                true,
                false,
                t0 + Duration::from_millis(20)
            ),
            (true, None)
        );
        assert_eq!(
            hook.handle_key(
                VK_VOLUME_DOWN_CODE,
                true,
                true,
                true,
                false,
                t0 + Duration::from_millis(25)
            ),
            (true, Some(KnobEvent::RotateCcw))
        );
        // Novo giro CW após 50ms do primeiro RotateCw passa normalmente
        assert_eq!(
            hook.handle_key(
                VK_VOLUME_UP_CODE,
                true,
                true,
                true,
                false,
                t0 + Duration::from_millis(55)
            ),
            (true, Some(KnobEvent::RotateCw))
        );

        // Bounce em botão (down/up/down em 80ms) é filtrado; novo clique após 400ms mesmo sem UP perdido recupera
        let t1 = t0 + Duration::from_secs(1);
        assert_eq!(
            hook.handle_key(VK_VOLUME_MUTE_CODE, true, true, true, false, t1),
            (true, Some(KnobEvent::KnobPress))
        );
        assert_eq!(
            hook.handle_key(
                VK_VOLUME_MUTE_CODE,
                false,
                true,
                true,
                false,
                t1 + Duration::from_millis(20)
            ),
            (true, None)
        );
        assert_eq!(
            hook.handle_key(
                VK_VOLUME_MUTE_CODE,
                true,
                true,
                true,
                false,
                t1 + Duration::from_millis(80)
            ),
            (true, None)
        );
        // Sem enviar WM_KEYUP, após 400ms o estado destrava automaticamente e aceita novo clique
        assert_eq!(
            hook.handle_key(
                VK_VOLUME_MUTE_CODE,
                true,
                true,
                true,
                false,
                t1 + Duration::from_millis(450)
            ),
            (true, Some(KnobEvent::KnobPress))
        );

        // Ctrl UP perdido: após >600ms, novo Ctrl+F1 (Key1) recupera automaticamente
        let t2 = t1 + Duration::from_secs(2);
        assert_eq!(
            hook.handle_key(VK_LCONTROL_CODE, true, false, true, false, t2),
            (false, None)
        );
        let t3 = t2 + Duration::from_millis(800);
        assert_eq!(
            hook.handle_key(VK_LCONTROL_CODE, true, false, true, false, t3),
            (false, None)
        );
        assert_eq!(
            hook.handle_key(
                VK_F1_CODE,
                true,
                false,
                true,
                false,
                t3 + Duration::from_millis(10)
            ),
            (true, None)
        );
        assert_eq!(
            hook.handle_key(
                VK_F1_CODE,
                false,
                false,
                true,
                false,
                t3 + Duration::from_millis(15)
            ),
            (true, None)
        );
        assert_eq!(
            hook.handle_key(
                VK_LCONTROL_CODE,
                false,
                false,
                true,
                false,
                t3 + Duration::from_millis(20)
            ),
            (false, Some(KnobEvent::Key1))
        );
    }
}
