use crate::actions_catalog::{resolve_desktop_exec, MacroStep};
use crate::config::{ActionConfig, ProfileConfig};
use crate::error::KnobError;
use crate::input::KnobEvent;
use enigo::{Axis, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use std::process::Command;
use std::thread;
use std::time::Duration;

/// Abstração para execução de ações, permitindo testes unitários com mocks.
pub trait ActionExecutor {
    fn execute(&mut self, action: &ActionConfig) -> Result<(), KnobError>;
    /// Libera preventivamente todas as teclas modificadoras (Shift, Ctrl, Alt, Meta)
    /// para evitar que fiquem presas após cancelamento de captura ou erro.
    fn release_modifiers(&mut self) {}
}

/// Backend de baixo nível para envio de eventos de tecla (`Press`/`Release`),
/// permitindo testar o `KeyReleaseGuard` (RAII) com falhas injetadas sem hardware real.
trait KeyBackend {
    fn send_key(&mut self, key: Key, direction: Direction) -> Result<(), KnobError>;
}

impl KeyBackend for Enigo {
    fn send_key(&mut self, key: Key, direction: Direction) -> Result<(), KnobError> {
        self.key(key, direction)
            .map_err(|err| KnobError::InputSimulation(err.to_string()))
    }
}

/// Guard RAII que garante `Direction::Release` (em ordem reversa) para todas as teclas
/// pressionadas através dele, mesmo em caso de erro ou early-return no meio de um atalho/macro.
struct KeyReleaseGuard<'a, B: KeyBackend> {
    backend: &'a mut B,
    pressed: Vec<Key>,
}

impl<'a, B: KeyBackend> KeyReleaseGuard<'a, B> {
    fn new(backend: &'a mut B) -> Self {
        Self {
            backend,
            pressed: Vec::new(),
        }
    }

    fn press(&mut self, key: Key) -> Result<(), KnobError> {
        self.backend.send_key(key, Direction::Press)?;
        self.pressed.push(key);
        Ok(())
    }

    fn release_all(&mut self) {
        while let Some(key) = self.pressed.pop() {
            let _ = self.backend.send_key(key, Direction::Release);
        }
    }
}

impl<B: KeyBackend> Drop for KeyReleaseGuard<'_, B> {
    fn drop(&mut self) {
        self.release_all();
    }
}

fn execute_shortcut_with_backend<B: KeyBackend>(
    backend: &mut B,
    keys: &[String],
) -> Result<(), KnobError> {
    if keys.is_empty() {
        return Err(KnobError::InvalidAction(
            "atalho `shortcut` não pode ter lista `keys` vazia".to_string(),
        ));
    }
    let parsed: Vec<Key> = keys
        .iter()
        .map(|k| parse_key_name(k))
        .collect::<Result<_, _>>()?;

    let mut guard = KeyReleaseGuard::new(backend);
    for &key in &parsed {
        guard.press(key)?;
    }
    guard.release_all();
    Ok(())
}

fn execute_macro_with_backend<B: KeyBackend>(
    backend: &mut B,
    steps: &[MacroStep],
) -> Result<(), KnobError> {
    for step in steps {
        match step {
            MacroStep::Keys(keys) => execute_shortcut_with_backend(backend, keys)?,
            MacroStep::WaitMs(ms) => thread::sleep(Duration::from_millis(*ms)),
        }
    }
    Ok(())
}

/// Resultado do despacho de um `KnobEvent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchOutcome {
    Executed(ActionConfig),
    ProfileSwitched(String),
    Ignored,
}

/// Despacha `KnobEvent` de acordo com o perfil solicitado.
pub struct ActionDispatcher<E: ActionExecutor> {
    profiles: Vec<ProfileConfig>,
    active_index: usize,
    executor: E,
}

impl<E: ActionExecutor> ActionDispatcher<E> {
    pub fn new(profiles: Vec<ProfileConfig>, active_index: usize, executor: E) -> Self {
        let safe_index = if profiles.is_empty() {
            0
        } else {
            active_index.min(profiles.len() - 1)
        };
        Self {
            profiles,
            active_index: safe_index,
            executor,
        }
    }

    pub fn update_profiles(&mut self, profiles: Vec<ProfileConfig>) {
        self.profiles = profiles;
        if self.profiles.is_empty() || self.active_index >= self.profiles.len() {
            self.active_index = 0;
        }
    }

    pub fn active_profile_name(&self) -> &str {
        self.profiles
            .get(self.active_index)
            .map(|p| p.name.as_str())
            .unwrap_or("<nenhum>")
    }

    pub fn active_profile_index(&self) -> usize {
        self.active_index
    }

    pub fn set_active_profile_index(&mut self, index: usize) {
        if !self.profiles.is_empty() {
            self.active_index = index.min(self.profiles.len() - 1);
        }
    }

    pub fn find_profile_index(&self, profile_name: &str) -> Option<usize> {
        self.profiles
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(profile_name))
    }

    pub fn release_modifiers(&mut self) {
        self.executor.release_modifiers();
    }

    pub fn dispatch(
        &mut self,
        event: KnobEvent,
        profile_name: &str,
    ) -> Result<DispatchOutcome, KnobError> {
        let target_idx = self
            .find_profile_index(profile_name)
            .or_else(|| self.find_profile_index(crate::config::GLOBAL_PROFILE_NAME))
            .unwrap_or(self.active_index);

        let Some(profile) = self.profiles.get(target_idx) else {
            return Ok(DispatchOutcome::Ignored);
        };
        self.active_index = target_idx;

        let action = match event {
            KnobEvent::RotateCw => profile.knob_cw.clone(),
            KnobEvent::RotateCcw => profile.knob_ccw.clone(),
            KnobEvent::KnobPress => profile.knob_press.clone(),
            KnobEvent::Key1 => profile.key1.clone(),
            KnobEvent::Key2 => profile.key2.clone(),
            KnobEvent::Key3 => profile.key3.clone(),
            KnobEvent::OpenSettings => return Ok(DispatchOutcome::Ignored),
        };

        match action {
            ActionConfig::None => Ok(DispatchOutcome::Ignored),
            ActionConfig::NextProfile => {
                self.active_index = (self.active_index + 1) % self.profiles.len();
                Ok(DispatchOutcome::ProfileSwitched(
                    self.active_profile_name().to_string(),
                ))
            }
            other => {
                self.executor.execute(&other)?;
                Ok(DispatchOutcome::Executed(other))
            }
        }
    }
}

/// Calcula o deslocamento contínuo (pixels no Wayland `wlr_virtual_pointer::axis`)
/// multiplicando a quantidade de cliques pelo fator `scroll_step`.
pub fn compute_scroll_delta(delta: i32, scroll_step: i32) -> i32 {
    delta.saturating_mul(scroll_step.max(1))
}

/// Executor real baseado na crate `enigo` (backend Wayland).
pub struct EnigoExecutor {
    enigo: Enigo,
    scroll_step: i32,
}

impl EnigoExecutor {
    pub fn new(scroll_step: i32) -> Result<Self, KnobError> {
        let enigo = Enigo::new(&Settings::default())
            .map_err(|err| KnobError::InputSimulationInit(err.to_string()))?;
        Ok(Self {
            enigo,
            scroll_step: scroll_step.max(1),
        })
    }
}

impl ActionExecutor for EnigoExecutor {
    fn execute(&mut self, action: &ActionConfig) -> Result<(), KnobError> {
        match action {
            ActionConfig::VolumeUp => self.click_key(Key::VolumeUp),
            ActionConfig::VolumeDown => self.click_key(Key::VolumeDown),
            ActionConfig::VolumeMute => self.click_key(Key::VolumeMute),
            ActionConfig::MediaPlayPause => self.click_key(Key::MediaPlayPause),
            ActionConfig::MediaNext => self.click_key(Key::MediaNextTrack),
            ActionConfig::MediaPrev => self.click_key(Key::MediaPrevTrack),
            ActionConfig::Scroll { delta, horizontal } => {
                let axis = if *horizontal {
                    Axis::Horizontal
                } else {
                    Axis::Vertical
                };
                let scaled = compute_scroll_delta(*delta, self.scroll_step);
                self.enigo
                    .scroll(scaled, axis)
                    .map_err(|err| KnobError::InputSimulation(err.to_string()))
            }
            ActionConfig::Shortcut { keys } => self.execute_shortcut(keys),
            ActionConfig::Launch { desktop_file } => {
                let tokens = resolve_desktop_exec(desktop_file)?;
                let program = tokens
                    .first()
                    .ok_or_else(|| KnobError::InvalidAction("Exec= vazio".to_string()))?;
                Command::new(program)
                    .args(&tokens[1..])
                    .spawn()
                    .map_err(|err| {
                        KnobError::InputSimulation(format!(
                            "falha ao abrir aplicativo '{desktop_file}' ({program}): {err}"
                        ))
                    })?;
                Ok(())
            }
            ActionConfig::Open { target } => open::that_detached(target).map_err(|err| {
                KnobError::InputSimulation(format!("falha ao abrir '{target}': {err}"))
            }),
            ActionConfig::Exec { command } => {
                Command::new("sh")
                    .arg("-c")
                    .arg(command)
                    .spawn()
                    .map_err(|err| {
                        KnobError::InputSimulation(format!(
                            "falha ao executar comando '{command}': {err}"
                        ))
                    })?;
                Ok(())
            }
            ActionConfig::Macro { steps } => execute_macro_with_backend(&mut self.enigo, steps),
            ActionConfig::NextProfile | ActionConfig::None => Ok(()),
        }
    }

    fn release_modifiers(&mut self) {
        for key in [
            Key::Shift,
            Key::RShift,
            Key::Control,
            Key::RControl,
            Key::Alt,
            Key::Meta,
        ] {
            let _ = self.enigo.key(key, Direction::Release);
        }
    }
}

impl EnigoExecutor {
    fn execute_shortcut(&mut self, keys: &[String]) -> Result<(), KnobError> {
        execute_shortcut_with_backend(&mut self.enigo, keys)
    }

    fn click_key(&mut self, key: Key) -> Result<(), KnobError> {
        self.enigo
            .key(key, Direction::Click)
            .map_err(|err| KnobError::InputSimulation(err.to_string()))
    }
}

/// Converte nomes de teclas amigáveis no TOML (ex.: `"ctrl"`, `"f4"`, `"]"`) para `enigo::Key`.
pub fn parse_key_name(name: &str) -> Result<Key, KnobError> {
    let normalized = name.trim().to_ascii_lowercase();
    let key = match normalized.as_str() {
        "ctrl" | "control" | "lctrl" => Key::Control,
        "rctrl" => Key::RControl,
        "shift" | "lshift" => Key::Shift,
        "rshift" => Key::RShift,
        "alt" | "lalt" => Key::Alt,
        "meta" | "super" | "win" => Key::Meta,
        "enter" | "return" => Key::Return,
        "esc" | "escape" => Key::Escape,
        "space" => Key::Space,
        "tab" => Key::Tab,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "up" => Key::UpArrow,
        "down" => Key::DownArrow,
        "left" => Key::LeftArrow,
        "right" => Key::RightArrow,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        single if single.chars().count() == 1 => {
            let ch = single
                .chars()
                .next()
                .ok_or_else(|| KnobError::InvalidAction(format!("tecla vazia: '{name}'")))?;
            Key::Unicode(ch)
        }
        _ => {
            return Err(KnobError::InvalidAction(format!(
                "tecla desconhecida no atalho: '{name}'"
            )));
        }
    };
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::{
        execute_macro_with_backend, execute_shortcut_with_backend, parse_key_name,
        ActionDispatcher, ActionExecutor, DispatchOutcome, KeyBackend, KeyReleaseGuard,
    };
    use crate::actions_catalog::MacroStep;
    use crate::config::{ActionConfig, Config};
    use crate::error::KnobError;
    use crate::input::KnobEvent;
    use enigo::{Direction, Key};

    struct RecordingKeyBackend {
        events: Vec<(Key, Direction)>,
        fail_on_key: Option<Key>,
    }

    impl RecordingKeyBackend {
        fn new(fail_on_key: Option<Key>) -> Self {
            Self {
                events: Vec::new(),
                fail_on_key,
            }
        }

        fn net_pressed_count(&self, target: Key) -> isize {
            self.events
                .iter()
                .filter(|(k, _)| *k == target)
                .map(|(_, d)| match d {
                    Direction::Press => 1,
                    Direction::Release => -1,
                    Direction::Click => 0,
                })
                .sum()
        }
    }

    impl KeyBackend for RecordingKeyBackend {
        fn send_key(&mut self, key: Key, direction: Direction) -> Result<(), KnobError> {
            if direction == Direction::Press && self.fail_on_key == Some(key) {
                return Err(KnobError::InputSimulation("falha simulada".to_string()));
            }
            self.events.push((key, direction));
            Ok(())
        }
    }

    #[test]
    fn key_release_guard_leaves_no_residual_state_on_shift_down_up() {
        let mut backend = RecordingKeyBackend::new(None);
        {
            let mut guard = KeyReleaseGuard::new(&mut backend);
            guard.press(Key::Shift).expect("shift down");
            guard.press(Key::Tab).expect("tab down");
        }
        assert_eq!(backend.net_pressed_count(Key::Shift), 0);
        assert_eq!(backend.net_pressed_count(Key::Tab), 0);
        assert_eq!(
            backend.events,
            vec![
                (Key::Shift, Direction::Press),
                (Key::Tab, Direction::Press),
                (Key::Tab, Direction::Release),
                (Key::Shift, Direction::Release),
            ]
        );
    }

    #[test]
    fn key_release_guard_releases_shift_when_error_occurs_mid_macro() {
        // Simula Shift+Tab onde Tab falha no meio da macro: Shift DEVE ser solto pelo Drop!
        let mut backend = RecordingKeyBackend::new(Some(Key::Tab));
        let steps = vec![
            MacroStep::Keys(vec!["Shift".into(), "Tab".into()]),
            MacroStep::Keys(vec!["Ctrl".into(), "V".into()]),
        ];

        let err = execute_macro_with_backend(&mut backend, &steps);
        assert!(err.is_err());
        assert_eq!(
            backend.net_pressed_count(Key::Shift),
            0,
            "Shift não pode ficar preso após erro no meio da macro"
        );
        assert_eq!(
            backend.events,
            vec![
                (Key::Shift, Direction::Press),
                (Key::Shift, Direction::Release),
            ]
        );

        // Também valida atalho isolado falhando após Ctrl+Shift
        let mut backend2 = RecordingKeyBackend::new(Some(Key::Unicode('z')));
        let err2 = execute_shortcut_with_backend(
            &mut backend2,
            &["Ctrl".into(), "Shift".into(), "Z".into()],
        );
        assert!(err2.is_err());
        assert_eq!(backend2.net_pressed_count(Key::Control), 0);
        assert_eq!(backend2.net_pressed_count(Key::Shift), 0);
    }

    #[derive(Default)]
    struct MockExecutor {
        executed: Vec<ActionConfig>,
    }

    impl ActionExecutor for MockExecutor {
        fn execute(&mut self, action: &ActionConfig) -> Result<(), KnobError> {
            self.executed.push(action.clone());
            Ok(())
        }
    }

    #[test]
    fn dispatches_expected_action_for_every_knob_event() {
        let cfg = Config::default();
        let active_idx = cfg.active_profile_index();
        let mut dispatcher =
            ActionDispatcher::new(cfg.profiles, active_idx, MockExecutor::default());

        let events = [
            KnobEvent::RotateCw,
            KnobEvent::RotateCcw,
            KnobEvent::KnobPress,
            KnobEvent::Key1,
            KnobEvent::Key2,
            KnobEvent::Key3,
        ];
        for ev in events {
            let outcome = dispatcher
                .dispatch(ev, "global")
                .expect("despacho deve funcionar");
            assert!(matches!(outcome, DispatchOutcome::Executed(_)));
        }

        assert_eq!(
            dispatcher.executor.executed,
            vec![
                ActionConfig::VolumeUp,
                ActionConfig::VolumeDown,
                ActionConfig::VolumeMute,
                ActionConfig::MediaPrev,
                ActionConfig::MediaNext,
                ActionConfig::MediaPlayPause,
            ]
        );
    }

    #[test]
    fn dispatches_contextual_profile_by_name() {
        let cfg = Config::default();
        let mut dispatcher = ActionDispatcher::new(cfg.profiles, 0, MockExecutor::default());

        let outcome = dispatcher
            .dispatch(KnobEvent::RotateCw, "gimp")
            .expect("despacho no perfil gimp");
        assert_eq!(
            outcome,
            DispatchOutcome::Executed(ActionConfig::Shortcut {
                keys: vec!["]".to_string()],
            })
        );
        assert_eq!(dispatcher.active_profile_name(), "gimp");
    }

    #[test]
    fn switches_profiles_on_next_profile_action() {
        let mut cfg = Config::default();
        cfg.profiles[0].key3 = ActionConfig::NextProfile;
        let mut dispatcher = ActionDispatcher::new(cfg.profiles, 0, MockExecutor::default());

        assert_eq!(dispatcher.active_profile_name(), "global");
        let switched = dispatcher
            .dispatch(KnobEvent::Key3, "global")
            .expect("troca de perfil");
        assert_eq!(
            switched,
            DispatchOutcome::ProfileSwitched("firefox".to_string())
        );
        assert_eq!(dispatcher.active_profile_name(), "firefox");
    }

    #[test]
    fn scroll_delta_is_multiplied_by_scroll_step() {
        let action = ActionConfig::parse_from_str("Scroll:up:3").expect("Scroll:up:3 válido");
        match action {
            ActionConfig::Scroll { delta, horizontal } => {
                assert!(!horizontal);
                assert_eq!(delta, -3);
                assert_eq!(super::compute_scroll_delta(delta, 20), -60);
                assert_eq!(super::compute_scroll_delta(delta, 25), -75);
            }
            other => panic!("esperava Scroll, recebeu {other:?}"),
        }
    }

    #[test]
    fn parses_key_names_and_rejects_unknown() {
        assert_eq!(parse_key_name("ctrl").ok(), Some(Key::Control));
        assert_eq!(parse_key_name("F4").ok(), Some(Key::F4));
        assert_eq!(parse_key_name("]").ok(), Some(Key::Unicode(']')));
        assert!(matches!(
            parse_key_name("tecla_inexistente"),
            Err(KnobError::InvalidAction(_))
        ));
    }
}
