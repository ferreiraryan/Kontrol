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
    pub active_preset_label: String,
    pub preset_dropdown_open: bool,
    pub preset_app_index: usize,
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
            active_preset_label: String::new(),
            preset_dropdown_open: false,
            preset_app_index: 0,
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
            ActionType::Provider { .. } | ActionType::None => {}
        }
    }

    pub fn select_type(&mut self, index: usize) {
        self.selected_type_index = index.min(8);
        self.preset_dropdown_open = false;
        self.capturing_shortcut = false;
        self.capturing_macro_key = false;
    }

    pub fn cancel(&mut self) {
        self.is_open = false;
        self.target_slot = None;
        self.preset_dropdown_open = false;
        self.capturing_shortcut = false;
        self.capturing_macro_key = false;
    }

    /// Aplica um preset nomeado de aplicativo (ex.: Firefox — Nova aba -> `Ctrl+T`).
    pub fn apply_named_preset(&mut self, app: &str, name: &str, combo: &str) {
        self.shortcut_text = combo.to_string();
        self.active_preset_label = format!("Preset: {app} — {name}");
        self.preset_dropdown_open = false;
    }

    /// Atualiza manualmente o campo de atalho e limpa o indicador de preset ativo.
    pub fn set_manual_shortcut(&mut self, value: String) {
        if self.shortcut_text != value {
            self.shortcut_text = value;
            self.active_preset_label.clear();
        }
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

    pub fn move_macro_step(&mut self, index: usize, delta: isize) {
        let len = self.macro_steps.len();
        if index >= len {
            return;
        }
        let target = if delta < 0 {
            index.saturating_sub(delta.unsigned_abs())
        } else {
            (index + delta as usize).min(len.saturating_sub(1))
        };
        if target != index {
            self.macro_steps.swap(index, target);
        }
    }

    /// Alterna um modificador (`Ctrl`, `Alt`, `Shift`, `Super`) no campo `shortcut_text`,
    /// mantendo ordem canônica `Ctrl+Alt+Shift+Super+<Tecla(s)>`.
    pub fn toggle_shortcut_modifier(&mut self, modifier: &str) {
        let canonical = match modifier.trim().to_ascii_lowercase().as_str() {
            "ctrl" | "control" => "Ctrl",
            "alt" => "Alt",
            "shift" => "Shift",
            "super" | "meta" | "win" => "Super",
            _ => return,
        };

        let mut has_ctrl = false;
        let mut has_alt = false;
        let mut has_shift = false;
        let mut has_super = false;
        let mut others: Vec<String> = Vec::new();

        for part in self
            .shortcut_text
            .split('+')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => has_ctrl = true,
                "alt" => has_alt = true,
                "shift" => has_shift = true,
                "super" | "meta" | "win" => has_super = true,
                _ => others.push(part.to_string()),
            }
        }

        match canonical {
            "Ctrl" => has_ctrl = !has_ctrl,
            "Alt" => has_alt = !has_alt,
            "Shift" => has_shift = !has_shift,
            "Super" => has_super = !has_super,
            _ => {}
        }

        let mut rebuilt = Vec::new();
        if has_ctrl {
            rebuilt.push("Ctrl".to_string());
        }
        if has_alt {
            rebuilt.push("Alt".to_string());
        }
        if has_shift {
            rebuilt.push("Shift".to_string());
        }
        if has_super {
            rebuilt.push("Super".to_string());
        }
        rebuilt.extend(others);
        self.shortcut_text = rebuilt.join("+");
        self.active_preset_label.clear();
    }

    /// Retorna as teclas individuais do atalho atual para renderização como keycaps visuais.
    pub fn shortcut_keycap_parts(&self) -> Vec<String> {
        self.shortcut_text
            .split('+')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect()
    }

    /// Gera um resumo legível em tempo real para o rodapé do Seletor de Ações.
    pub fn live_preview_summary(&self, apps: &[DesktopApp]) -> String {
        match self.selected_type_index {
            0 => {
                let trimmed = self.shortcut_text.trim();
                if trimmed.is_empty() {
                    "Atalho • nenhum definido".to_string()
                } else {
                    format!("Atalho • {trimmed}")
                }
            }
            1 => {
                let label = MediaAction::ALL
                    .get(self.media_index)
                    .copied()
                    .unwrap_or(MediaAction::VolumeUp)
                    .friendly_label();
                format!("Mídia • {label}")
            }
            2 => {
                if self.selected_desktop_file.trim().is_empty() {
                    "Aplicativo • selecione na lista".to_string()
                } else {
                    let name = apps
                        .iter()
                        .find(|a| a.desktop_file == self.selected_desktop_file)
                        .map(|a| a.name.as_str())
                        .unwrap_or(self.selected_desktop_file.as_str());
                    format!("Aplicativo • {name}")
                }
            }
            3 => {
                let u = self.url_text.trim();
                if u.is_empty() {
                    "URL • informe um endereço".to_string()
                } else {
                    format!("URL • {u}")
                }
            }
            4 => {
                let f = self.file_uri.trim();
                if f.is_empty() {
                    "Arquivo • selecione um arquivo".to_string()
                } else {
                    let clean = f.strip_prefix("file://").unwrap_or(f);
                    let fname = std::path::Path::new(clean)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(clean);
                    format!("Arquivo • {fname}")
                }
            }
            5 => {
                let cmd = self.exec_command.trim();
                if cmd.is_empty() {
                    "Comando • informe um comando shell".to_string()
                } else {
                    format!("Comando • $ {cmd}")
                }
            }
            6 => {
                let dir = ScrollDirection::ALL
                    .get(self.scroll_direction_index)
                    .copied()
                    .unwrap_or(ScrollDirection::Up)
                    .friendly_label();
                format!("Scroll • {dir} ×{}", self.scroll_amount)
            }
            7 => {
                let n = self.macro_steps.len();
                if n == 0 {
                    "Macro • nenhum passo".to_string()
                } else if n == 1 {
                    "Macro • 1 passo".to_string()
                } else {
                    format!("Macro • {n} passos")
                }
            }
            _ => "Nenhuma ação (herda do pai se houver)".to_string(),
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

    #[test]
    fn toggles_shortcut_modifiers_in_canonical_order_and_reorders_macro() {
        let mut st = ActionEditorState::default();
        st.open(ControlSlot::Key1, &ActionType::Shortcut(vec!["T".into()]));
        st.toggle_shortcut_modifier("Shift");
        st.toggle_shortcut_modifier("Ctrl");
        st.toggle_shortcut_modifier("Super");
        assert_eq!(st.shortcut_text, "Ctrl+Shift+Super+T");
        assert_eq!(
            st.shortcut_keycap_parts(),
            vec!["Ctrl", "Shift", "Super", "T"]
        );

        // Remover Shift preserva o resto em ordem canônica
        st.toggle_shortcut_modifier("Shift");
        assert_eq!(st.shortcut_text, "Ctrl+Super+T");

        // Reordenação de macro respeita limites
        st.select_type(7);
        st.add_macro_key_step("Ctrl+C").expect("p1");
        st.add_macro_wait_step("100").expect("p2");
        st.add_macro_key_step("Ctrl+V").expect("p3");
        st.move_macro_step(2, -1);
        assert_eq!(
            st.macro_steps[1],
            MacroStep::Keys(vec!["Ctrl".into(), "V".into()])
        );
        st.move_macro_step(0, -1); // no-op no início
        assert_eq!(
            st.macro_steps[0],
            MacroStep::Keys(vec!["Ctrl".into(), "C".into()])
        );
        assert_eq!(st.live_preview_summary(&[]), "Macro • 3 passos");
    }

    #[test]
    fn named_preset_sets_indicator_and_clears_on_manual_edit() {
        let mut st = ActionEditorState::default();
        st.open(ControlSlot::Key1, &ActionType::None);
        st.select_type(0);
        st.preset_dropdown_open = true;

        st.apply_named_preset("Firefox", "Nova aba", "Ctrl+T");
        assert_eq!(st.shortcut_text, "Ctrl+T");
        assert_eq!(st.active_preset_label, "Preset: Firefox — Nova aba");
        assert!(!st.preset_dropdown_open);

        // Editar manualmente limpa o indicador de preset
        st.set_manual_shortcut("Ctrl+Shift+T".to_string());
        assert_eq!(st.shortcut_text, "Ctrl+Shift+T");
        assert!(st.active_preset_label.is_empty());
    }
}
