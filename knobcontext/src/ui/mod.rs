pub mod action_editor_mod;
pub mod action_editor_state;
pub mod settings_mod;
pub mod state;

use crate::error::KnobError;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::rc::Rc;

slint::include_modules!();

/// Gerencia o ciclo de vida da janela Overlay em Slint.
pub struct OverlayWindow {
    window: Overlay,
    profile_names: Vec<String>,
}

impl OverlayWindow {
    pub fn new(profile_names: Vec<String>, active_index: usize) -> Result<Self, KnobError> {
        let window = Overlay::new().map_err(|err| KnobError::UiInit(err.to_string()))?;
        let mut controller = Self {
            window,
            profile_names: Vec::new(),
        };
        controller.update_profiles(profile_names, active_index);
        Ok(controller)
    }

    pub fn update_profiles(&mut self, profile_names: Vec<String>, active_index: usize) {
        let model: Vec<SharedString> = profile_names.iter().map(SharedString::from).collect();
        self.window
            .set_profiles(ModelRc::from(Rc::new(VecModel::from(model))));
        self.profile_names = profile_names;
        self.update_selection(active_index, active_index);
    }

    pub fn update_selection(&self, active_index: usize, highlighted_index: usize) {
        let label = self
            .profile_names
            .get(highlighted_index)
            .map(String::as_str)
            .unwrap_or("Perfil");
        self.window
            .set_active_profile_name(SharedString::from(label));
        self.window.set_active_index(active_index as i32);
        self.window.set_highlighted_index(highlighted_index as i32);
    }

    pub fn show(&self, active_index: usize, highlighted_index: usize) -> Result<(), KnobError> {
        self.update_selection(active_index, highlighted_index);
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
