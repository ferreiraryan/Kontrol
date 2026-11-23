use crate::config::DeviceConfig;
use crate::error::KnobError;
use evdev::{Device, EventType, InputEvent, Key};
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
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

/// Máquina de estados pura que traduz eventos brutos `evdev` em `KnobEvent`.
///
/// Trata combos como `Ctrl+F1` (`Key1`), `Ctrl+F4` (`Key2`) e `Key1+Key2` (`OpenSettings`)
/// exigindo que a segunda tecla chegue dentro de `combo_window` enquanto `Ctrl` está
/// pressionado, e emitindo o `KnobEvent` somente quando o modificador (`Ctrl`) recebe `KEY_UP`
/// para evitar estado residual de modificadores presos.
#[derive(Debug)]
pub struct EventTranslator {
    combo_window: Duration,
    ctrl_pressed_at: Option<Instant>,
    f1_down: bool,
    f4_down: bool,
    pending_combo: Option<KnobEvent>,
}

impl EventTranslator {
    pub fn new(combo_window: Duration) -> Self {
        Self {
            combo_window,
            ctrl_pressed_at: None,
            f1_down: false,
            f4_down: false,
            pending_combo: None,
        }
    }

    pub fn process_input_event(&mut self, event: InputEvent, now: Instant) -> Option<KnobEvent> {
        if event.event_type() != EventType::KEY {
            return None;
        }
        self.process_key(Key::new(event.code()), event.value(), now)
    }

    pub fn process_key(&mut self, key: Key, value: i32, now: Instant) -> Option<KnobEvent> {
        // value: 1 = press, 0 = release, 2 = repeat
        match (key, value) {
            (Key::KEY_LEFTCTRL | Key::KEY_RIGHTCTRL, 1) => {
                self.ctrl_pressed_at = Some(now);
                None
            }
            (Key::KEY_LEFTCTRL | Key::KEY_RIGHTCTRL, 0) => {
                self.ctrl_pressed_at = None;
                self.f1_down = false;
                self.f4_down = false;
                self.pending_combo.take()
            }
            (Key::KEY_VOLUMEUP, 1) => Some(KnobEvent::RotateCw),
            (Key::KEY_VOLUMEDOWN, 1) => Some(KnobEvent::RotateCcw),
            (Key::KEY_MUTE, 1) => Some(KnobEvent::KnobPress),
            (Key::KEY_PLAYPAUSE, 1) => Some(KnobEvent::Key3),
            (Key::KEY_F4, 1) => {
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
            (Key::KEY_F4, 0) => {
                self.f4_down = false;
                None
            }
            (Key::KEY_F1, 1) => {
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
            (Key::KEY_F1, 0) => {
                self.f1_down = false;
                None
            }
            _ => None,
        }
    }

    fn is_within_combo_window(&self, now: Instant) -> bool {
        self.ctrl_pressed_at.is_some_and(|pressed_at| {
            now.saturating_duration_since(pressed_at) <= self.combo_window
        })
    }
}

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
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
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

#[cfg(test)]
mod tests {
    use super::{EventTranslator, KnobEvent};
    use evdev::{EventType, InputEvent, Key};
    use std::time::{Duration, Instant};

    fn key_event(key: Key, value: i32) -> InputEvent {
        InputEvent::new(EventType::KEY, key.code(), value)
    }

    #[test]
    fn translates_single_key_and_knob_events() {
        let mut tr = EventTranslator::new(Duration::from_millis(200));
        let t0 = Instant::now();

        assert_eq!(
            tr.process_input_event(key_event(Key::KEY_VOLUMEUP, 1), t0),
            Some(KnobEvent::RotateCw)
        );
        assert_eq!(
            tr.process_input_event(key_event(Key::KEY_VOLUMEUP, 0), t0),
            None
        );
        assert_eq!(
            tr.process_input_event(key_event(Key::KEY_VOLUMEDOWN, 1), t0),
            Some(KnobEvent::RotateCcw)
        );
        assert_eq!(
            tr.process_input_event(key_event(Key::KEY_MUTE, 1), t0),
            Some(KnobEvent::KnobPress)
        );
        assert_eq!(
            tr.process_input_event(key_event(Key::KEY_PLAYPAUSE, 1), t0),
            Some(KnobEvent::Key3)
        );
    }

    #[test]
    fn translates_ctrl_combos_within_window() {
        let mut tr = EventTranslator::new(Duration::from_millis(200));
        let t0 = Instant::now();

        // Ctrl+F4 dentro de 50ms -> emite Key2 quando Ctrl recebe KEY_UP
        assert_eq!(
            tr.process_input_event(key_event(Key::KEY_LEFTCTRL, 1), t0),
            None
        );
        assert_eq!(
            tr.process_input_event(key_event(Key::KEY_F4, 1), t0 + Duration::from_millis(50)),
            None
        );
        assert_eq!(
            tr.process_input_event(key_event(Key::KEY_F4, 0), t0 + Duration::from_millis(55)),
            None
        );
        assert_eq!(
            tr.process_input_event(
                key_event(Key::KEY_LEFTCTRL, 0),
                t0 + Duration::from_millis(60)
            ),
            Some(KnobEvent::Key2)
        );

        // Ctrl+F1 dentro de 10ms -> emite Key1 quando Ctrl recebe KEY_UP
        let t1 = t0 + Duration::from_millis(500);
        assert_eq!(
            tr.process_input_event(key_event(Key::KEY_LEFTCTRL, 1), t1),
            None
        );
        assert_eq!(
            tr.process_input_event(key_event(Key::KEY_F1, 1), t1 + Duration::from_millis(10)),
            None
        );
        assert_eq!(
            tr.process_input_event(
                key_event(Key::KEY_LEFTCTRL, 0),
                t1 + Duration::from_millis(20)
            ),
            Some(KnobEvent::Key1)
        );
    }

    #[test]
    fn ignores_lone_ctrl_expired_combo_and_unmodified_f_keys() {
        let mut tr = EventTranslator::new(Duration::from_millis(200));
        let t0 = Instant::now();

        // Ctrl sozinho não emite nada
        assert_eq!(
            tr.process_input_event(key_event(Key::KEY_LEFTCTRL, 1), t0),
            None
        );
        assert_eq!(
            tr.process_input_event(
                key_event(Key::KEY_LEFTCTRL, 0),
                t0 + Duration::from_millis(20)
            ),
            None
        );

        // F4 sem Ctrl não emite nada
        assert_eq!(
            tr.process_input_event(key_event(Key::KEY_F4, 1), t0 + Duration::from_millis(40)),
            None
        );

        // Ctrl seguido de F1 após 250ms (janela expirada) não emite nada
        let t2 = t0 + Duration::from_millis(300);
        assert_eq!(
            tr.process_input_event(key_event(Key::KEY_LEFTCTRL, 1), t2),
            None
        );
        assert_eq!(
            tr.process_input_event(key_event(Key::KEY_F1, 1), t2 + Duration::from_millis(250)),
            None
        );
    }

    #[test]
    fn ignores_non_key_events_like_msc_scan() {
        let mut tr = EventTranslator::new(Duration::from_millis(200));
        let t0 = Instant::now();
        let misc = InputEvent::new(EventType::MISC, 4, 786_665);
        assert_eq!(tr.process_input_event(misc, t0), None);
    }

    #[test]
    fn emits_open_settings_when_key1_and_key2_pressed_simultaneously() {
        let mut tr = EventTranslator::new(Duration::from_millis(200));
        let t0 = Instant::now();

        assert_eq!(
            tr.process_input_event(key_event(Key::KEY_LEFTCTRL, 1), t0),
            None
        );
        assert_eq!(
            tr.process_input_event(key_event(Key::KEY_F1, 1), t0 + Duration::from_millis(10)),
            None
        );
        // Enquanto F1 ainda está pressionado, Ctrl+F4 promove o combo pendente para OpenSettings
        assert_eq!(
            tr.process_input_event(key_event(Key::KEY_F4, 1), t0 + Duration::from_millis(30)),
            None
        );
        assert_eq!(
            tr.process_input_event(
                key_event(Key::KEY_LEFTCTRL, 0),
                t0 + Duration::from_millis(40)
            ),
            Some(KnobEvent::OpenSettings)
        );
    }
}
