use crate::actions_catalog::{
    action_type_index, parse_shortcut_keys, ActionType, DesktopApp, MacroStep, MediaAction,
    ScrollDirection,
};
use crate::error::KnobError;

/// Identificador dos 5 controles físicos editáveis na janela de configurações
/// (`KnobPress` é reservado para abrir/fechar o overlay).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlSlot {
    RotateCw,
    RotateCcw,
    Key1,
    Key2,
    Key3,
}

impl ControlSlot {
    pub fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::RotateCw),
            1 => Some(Self::RotateCcw),
            2 => Some(Self::Key1),
            3 => Some(Self::Key2),
            4 => Some(Self::Key3),
            _ => None,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::RotateCw => "RotateCw (Horário)",
            Self::RotateCcw => "RotateCcw (Anti-horário)",
            Self::Key1 => "Key1 (Esquerda)",
            Self::Key2 => "Key2 (Meio)",
            Self::Key3 => "Key3 (Direita)",
        }
    }
}

/// Máquina de estados pura do popover de edição de ações (testável sem Slint).
#[derive(Debug, Clone)]
pub struct ActionEditorState {
    pub is_open: bool,
    pub target_slot: Option<ControlSlot>,
    pub custom_title: Option<String>,
    pub selected_type_index: usize,
    pub shortcut_text: String,
    pub capturing_shortcut: bool,
    pub media_index: usize,
    pub app_search: String,
    pub selected_desktop_file: String,
    pub url_text: String,
    pub file_uri: String,
    pub exec_command: String,
    pub scroll_direction_index: usize,
    pub scroll_amount: u8,
    pub macro_steps: Vec<MacroStep>,
    pub macro_key_input: String,
    pub macro_wait_ms_input: String,
    pub capturing_macro_key: bool,
}

impl Default for ActionEditorState {
    fn default() -> Self {
        Self {
            is_open: false,
            target_slot: None,
            custom_title: None,
            selected_type_index: 8, // Nenhuma ação
            shortcut_text: String::new(),
            capturing_shortcut: false,
            media_index: 0,
            app_search: String::new(),
            selected_desktop_file: String::new(),
            url_text: String::new(),
            file_uri: String::new(),
            exec_command: String::new(),
            scroll_direction_index: 0,
            scroll_amount: 1,
            macro_steps: Vec::new(),
            macro_key_input: String::new(),
            macro_wait_ms_input: "100".to_string(),
            capturing_macro_key: false,
        }
    }
}

impl ActionEditorState {
    pub fn open(&mut self, slot: ControlSlot, initial: &ActionType) {
        self.open_internal(Some(slot), None, initial);
    }

    pub fn open_custom(&mut self, title: String, initial: &ActionType) {
        self.open_internal(None, Some(title), initial);
    }

    fn open_internal(
        &mut self,
        slot: Option<ControlSlot>,
        custom_title: Option<String>,
        initial: &ActionType,
    ) {
        *self = Self::default();
        self.is_open = true;
        self.target_slot = slot;
        self.custom_title = custom_title;
        self.selected_type_index = action_type_index(initial);

        match initial {
            ActionType::Shortcut(keys) => {
                self.shortcut_text = keys.join("+");
            }
            ActionType::Media(media) => {
                self.media_index = MediaAction::ALL
                    .iter()
                    .position(|m| m == media)
                    .unwrap_or(0);
            }
            ActionType::LaunchApp(file) => {
                self.selected_desktop_file = file.clone();
            }
            ActionType::OpenUrl(url) => {
                self.url_text = url.clone();
            }
            ActionType::OpenFile(uri) => {
                self.file_uri = uri.clone();
            }
            ActionType::ExecCommand(cmd) => {
                self.exec_command = cmd.clone();
            }
            ActionType::Scroll { direction, amount } => {
                self.scroll_direction_index = ScrollDirection::ALL
                    .iter()
                    .position(|d| d == direction)
                    .unwrap_or(0);
                self.scroll_amount = (*amount).clamp(1, 10);
            }
            ActionType::Macro(steps) => {
                self.macro_steps = steps.clone();
            }
            ActionType::None => {}
        }
    }

    pub fn select_type(&mut self, index: usize) {
        self.selected_type_index = index.min(8);
        self.capturing_shortcut = false;
        self.capturing_macro_key = false;
    }

    pub fn cancel(&mut self) {
        self.is_open = false;
        self.target_slot = None;
        self.capturing_shortcut = false;
        self.capturing_macro_key = false;
    }

    pub fn add_macro_key_step(&mut self, raw_combo: &str) -> Result<(), KnobError> {
        let keys = parse_shortcut_keys(raw_combo)?;
        self.macro_steps.push(MacroStep::Keys(keys));
        self.macro_key_input.clear();
        self.capturing_macro_key = false;
        Ok(())
    }

    pub fn add_macro_wait_step(&mut self, raw_ms: &str) -> Result<(), KnobError> {
        let ms = raw_ms.trim().parse::<u64>().map_err(|_| {
            KnobError::InvalidAction(format!("tempo de espera inválido: '{raw_ms}'"))
        })?;
        if ms == 0 || ms > 60_000 {
            return Err(KnobError::InvalidAction(format!(
                "espera deve estar entre 1 e 60000 ms (recebido: {ms})"
            )));
        }
        self.macro_steps.push(MacroStep::WaitMs(ms));
        Ok(())
    }

    pub fn remove_macro_step(&mut self, index: usize) {
        if index < self.macro_steps.len() {
            self.macro_steps.remove(index);
        }
    }

    pub fn filtered_apps<'a>(&self, all_apps: &'a [DesktopApp]) -> Vec<&'a DesktopApp> {
        let query = self.app_search.trim().to_ascii_lowercase();
        if query.is_empty() {
            all_apps.iter().collect()
        } else {
            all_apps
                .iter()
                .filter(|app| {
                    app.name.to_ascii_lowercase().contains(&query)
                        || app.desktop_file.to_ascii_lowercase().contains(&query)
                })
                .collect()
        }
    }

    /// Constrói e valida o `ActionType` final a partir do estado atual do popover.
    pub fn build_action(&self) -> Result<ActionType, KnobError> {
        match self.selected_type_index {
            0 => {
                let keys = parse_shortcut_keys(&self.shortcut_text)?;
                Ok(ActionType::Shortcut(keys))
            }
            1 => {
                let media = MediaAction::ALL
                    .get(self.media_index)
                    .copied()
                    .unwrap_or(MediaAction::VolumeUp);
                Ok(ActionType::Media(media))
            }
            2 => {
                let file = self.selected_desktop_file.trim();
                if file.is_empty() {
                    return Err(KnobError::InvalidAction(
                        "selecione um aplicativo da lista".to_string(),
                    ));
                }
                Ok(ActionType::LaunchApp(file.to_string()))
            }
            3 => {
                let url = self.url_text.trim();
                let parsed = crate::actions_catalog::parse_action_string(&format!("Open:{url}"))?;
                if matches!(parsed, ActionType::OpenUrl(_)) {
                    Ok(parsed)
                } else {
                    Err(KnobError::InvalidAction(
                        "informe uma URL começando com http:// ou https://".to_string(),
                    ))
                }
            }
            4 => {
                let raw = self.file_uri.trim();
                if raw.is_empty() {
                    return Err(KnobError::InvalidAction(
                        "informe ou escolha um arquivo".to_string(),
                    ));
                }
                let uri = if raw.starts_with("file://") {
                    raw.to_string()
                } else {
                    format!("file://{raw}")
                };
                crate::actions_catalog::parse_action_string(&format!("Open:{uri}"))
            }
            5 => {
                let cmd = self.exec_command.trim();
                if cmd.is_empty() {
                    return Err(KnobError::InvalidAction(
                        "Exec: não pode ter comando vazio".to_string(),
                    ));
                }
                Ok(ActionType::ExecCommand(cmd.to_string()))
            }
            6 => {
                if !(1..=10).contains(&self.scroll_amount) {
                    return Err(KnobError::InvalidAction(format!(
                        "quantidade de Scroll fora de 1..=10: {}",
                        self.scroll_amount
                    )));
                }
                let direction = ScrollDirection::ALL
                    .get(self.scroll_direction_index)
                    .copied()
                    .unwrap_or(ScrollDirection::Up);
                Ok(ActionType::Scroll {
                    direction,
                    amount: self.scroll_amount,
                })
            }
            7 => {
                if self.macro_steps.is_empty() {
                    return Err(KnobError::InvalidAction(
                        "adicione ao menos um passo na macro".to_string(),
                    ));
                }
                Ok(ActionType::Macro(self.macro_steps.clone()))
            }
            _ => Ok(ActionType::None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ActionEditorState, ControlSlot};
    use crate::actions_catalog::{ActionType, DesktopApp, MacroStep, MediaAction, ScrollDirection};

    #[test]
    fn opens_switches_types_builds_and_cancels() {
        let mut st = ActionEditorState::default();
        st.open(ControlSlot::Key1, &ActionType::Media(MediaAction::VolumeUp));
        assert!(st.is_open);
        assert_eq!(st.target_slot, Some(ControlSlot::Key1));
        assert_eq!(st.selected_type_index, 1);

        // Troca para Scroll
        st.select_type(6);
        st.scroll_direction_index = 0; // Up
        st.scroll_amount = 3;
        assert_eq!(
            st.build_action().ok(),
            Some(ActionType::Scroll {
                direction: ScrollDirection::Up,
                amount: 3
            })
        );

        // Quantidade fora de 1..=10 falha
        st.scroll_amount = 15;
        assert!(st.build_action().is_err());

        // Cancelar fecha o popover
        st.cancel();
        assert!(!st.is_open);
        assert_eq!(st.target_slot, None);
    }

    #[test]
    fn builds_and_edits_macro_steps() {
        let mut st = ActionEditorState::default();
        st.open(ControlSlot::Key2, &ActionType::None);
        st.select_type(7);

        // Macro vazia deve falhar
        assert!(st.build_action().is_err());

        st.add_macro_key_step("Ctrl+C").expect("passo 1");
        st.add_macro_wait_step("100").expect("passo 2");
        st.add_macro_key_step("Ctrl+V").expect("passo 3");

        assert_eq!(
            st.build_action().ok(),
            Some(ActionType::Macro(vec![
                MacroStep::Keys(vec!["Ctrl".into(), "C".into()]),
                MacroStep::WaitMs(100),
                MacroStep::Keys(vec!["Ctrl".into(), "V".into()]),
            ]))
        );

        st.remove_macro_step(1);
        assert_eq!(st.macro_steps.len(), 2);
    }

    #[test]
    fn filters_desktop_apps_by_name_or_filename() {
        let mut st = ActionEditorState::default();
        let apps = vec![
            DesktopApp {
                desktop_file: "firefox.desktop".into(),
                name: "Mozilla Firefox".into(),
                exec: "firefox".into(),
                icon: None,
            },
            DesktopApp {
                desktop_file: "gimp.desktop".into(),
                name: "GNU Image Manipulation Program".into(),
                exec: "gimp".into(),
                icon: None,
            },
        ];

        st.app_search = "fire".into();
        let res = st.filtered_apps(&apps);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].desktop_file, "firefox.desktop");
    }
}
