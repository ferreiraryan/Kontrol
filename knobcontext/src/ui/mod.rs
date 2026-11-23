pub mod action_editor_mod;
pub mod action_editor_state;
pub mod settings_mod;
pub mod state;

use crate::error::KnobError;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::rc::Rc;

pub const AUTO_ITEM_LABEL: &str = "Automático";

slint::include_modules!();

/// Gerencia o ciclo de vida da janela Overlay em Slint.
///
/// O índice `0` é sempre `"Automático"`, seguido pelos perfis configurados em `1..=N`.
pub struct OverlayWindow {
    window: Overlay,
    items: Vec<String>,
}

impl OverlayWindow {
    pub fn new(profile_names: Vec<String>, active_index: usize) -> Result<Self, KnobError> {
        let window = Overlay::new().map_err(|err| KnobError::UiInit(err.to_string()))?;
        let mut controller = Self {
            window,
            items: Vec::new(),
        };
        controller.update_profiles(profile_names, active_index, "Automático (global)");
        Ok(controller)
    }

    pub fn update_profiles(
        &mut self,
        profile_names: Vec<String>,
        active_index: usize,
        subtitle: &str,
    ) {
        let mut items = Vec::with_capacity(profile_names.len() + 1);
        items.push(AUTO_ITEM_LABEL.to_string());
        items.extend(profile_names);

        let model: Vec<SharedString> = items.iter().map(SharedString::from).collect();
        self.window
            .set_profiles(ModelRc::from(Rc::new(VecModel::from(model))));
        self.items = items;
        self.update_selection(active_index, active_index, subtitle);
    }

    pub fn update_selection(&self, active_index: usize, highlighted_index: usize, subtitle: &str) {
        let label = self
            .items
            .get(highlighted_index)
            .map(String::as_str)
            .unwrap_or(AUTO_ITEM_LABEL);
        self.window
            .set_active_profile_name(SharedString::from(label));
        self.window.set_mode_subtitle(SharedString::from(subtitle));
        self.window.set_active_index(active_index as i32);
        self.window.set_highlighted_index(highlighted_index as i32);
    }

    pub fn show(
        &self,
        active_index: usize,
        highlighted_index: usize,
        subtitle: &str,
    ) -> Result<(), KnobError> {
        self.update_selection(active_index, highlighted_index, subtitle);
        self.window
            .show()
            .map_err(|err| KnobError::UiRun(err.to_string()))
    }

    pub fn hide(&self) -> Result<(), KnobError> {
        self.window
            .hide()
            .map_err(|err| KnobError::UiRun(err.to_string()))
    }
}
