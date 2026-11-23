use crate::config::{ActionConfig, ProfileConfig};
use crate::error::KnobError;
use crate::input::KnobEvent;
use enigo::{Axis, Direction, Enigo, Key, Keyboard, Mouse, Settings};

/// Abstração para execução de ações, permitindo testes unitários com mocks.
pub trait ActionExecutor {
    fn execute(&mut self, action: &ActionConfig) -> Result<(), KnobError>;
}

/// Resultado do despacho de um `KnobEvent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchOutcome {
    Executed(ActionConfig),
    ProfileSwitched(String),
    Ignored,
}

/// Despacha `KnobEvent` de acordo com o perfil global ativo.
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

    pub fn active_profile_name(&self) -> &str {
        self.profiles
            .get(self.active_index)
            .map(|p| p.name.as_str())
            .unwrap_or("<nenhum>")
    }

    pub fn dispatch(&mut self, event: KnobEvent) -> Result<DispatchOutcome, KnobError> {
        let Some(profile) = self.profiles.get(self.active_index) else {
            return Ok(DispatchOutcome::Ignored);
        };

        let action = match event {
            KnobEvent::RotateCw => profile.knob_cw.clone(),
            KnobEvent::RotateCcw => profile.knob_ccw.clone(),
            KnobEvent::KnobPress => profile.knob_press.clone(),
            KnobEvent::Key1 => profile.key1.clone(),
            KnobEvent::Key2 => profile.key2.clone(),
            KnobEvent::Key3 => profile.key3.clone(),
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

/// Executor real baseado na crate `enigo` (backend Wayland).
pub struct EnigoExecutor {
    enigo: Enigo,
}

impl EnigoExecutor {
    pub fn new() -> Result<Self, KnobError> {
        let enigo = Enigo::new(&Settings::default())
            .map_err(|err| KnobError::InputSimulationInit(err.to_string()))?;
        Ok(Self { enigo })
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
                self.enigo
                    .scroll(*delta, axis)
                    .map_err(|err| KnobError::InputSimulation(err.to_string()))
            }
            ActionConfig::Shortcut { keys } => {
                if keys.is_empty() {
                    return Err(KnobError::InvalidAction(
                        "atalho `shortcut` não pode ter lista `keys` vazia".to_string(),
                    ));
                }
                let parsed: Vec<Key> = keys
                    .iter()
                    .map(|k| parse_key_name(k))
                    .collect::<Result<_, _>>()?;

                for &key in &parsed {
                    self.enigo
                        .key(key, Direction::Press)
                        .map_err(|err| KnobError::InputSimulation(err.to_string()))?;
                }
                for &key in parsed.iter().rev() {
                    self.enigo
                        .key(key, Direction::Release)
                        .map_err(|err| KnobError::InputSimulation(err.to_string()))?;
                }
                Ok(())
            }
            ActionConfig::NextProfile | ActionConfig::None => Ok(()),
        }
    }
}

impl EnigoExecutor {
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
    use super::{parse_key_name, ActionDispatcher, ActionExecutor, DispatchOutcome};
    use crate::config::{ActionConfig, Config};
    use crate::error::KnobError;
    use crate::input::KnobEvent;
    use enigo::Key;

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
            let outcome = dispatcher.dispatch(ev).expect("despacho deve funcionar");
            assert!(matches!(outcome, DispatchOutcome::Executed(_)));
        }

        assert_eq!(
            dispatcher.executor.executed,
            vec![
                ActionConfig::VolumeUp,
                ActionConfig::VolumeDown,
                ActionConfig::VolumeMute,
                ActionConfig::MediaPlayPause,
                ActionConfig::MediaPrev,
                ActionConfig::MediaNext,
            ]
        );
    }

    #[test]
    fn switches_profiles_on_next_profile_action() {
        let mut cfg = Config::default();
        cfg.profiles[0].key3 = ActionConfig::NextProfile;
        let mut dispatcher = ActionDispatcher::new(cfg.profiles, 0, MockExecutor::default());

        assert_eq!(dispatcher.active_profile_name(), "volume");
        let switched = dispatcher
            .dispatch(KnobEvent::Key3)
            .expect("troca de perfil");
        assert_eq!(
            switched,
            DispatchOutcome::ProfileSwitched("scroll".to_string())
        );
        assert_eq!(dispatcher.active_profile_name(), "scroll");

        // No perfil "scroll", RotateCw deve disparar Scroll
        let next = dispatcher
            .dispatch(KnobEvent::RotateCw)
            .expect("ação no segundo perfil");
        assert_eq!(
            next,
            DispatchOutcome::Executed(ActionConfig::Scroll {
                delta: 2,
                horizontal: false,
            })
        );
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
