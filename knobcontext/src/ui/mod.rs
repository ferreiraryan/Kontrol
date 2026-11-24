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
///
/// No Wayland, `slint::Window::hide()` suspende/destrói a janela `winit` subjacente e
/// reinicia `WindowAttributes` para o padrão (`resizable = true`, sem `min/max_inner_size`),
/// fazendo compositores tiling (como Hyprland) abrirem reaberturas em modo tile.
/// Por isso, descartamos a instância ao fechar e criamos um `Overlay` novo a cada `show()`.
pub struct OverlayWindow {
    window: Option<Overlay>,
    items: Vec<String>,
    active_index: usize,
    highlighted_index: usize,
    subtitle: String,
}

impl OverlayWindow {
    pub fn new(profile_names: Vec<String>, active_index: usize) -> Result<Self, KnobError> {
        let mut controller = Self {
            window: None,
            items: Vec::new(),
            active_index,
            highlighted_index: active_index,
            subtitle: "Automático (global)".to_string(),
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

        self.items = items;
        self.active_index = active_index;
        self.highlighted_index = active_index;
        self.subtitle = subtitle.to_string();

        if let Some(window) = &self.window {
            Self::apply_state_to_window(
                window,
                &self.items,
                self.active_index,
                self.highlighted_index,
                &self.subtitle,
            );
        }
    }

    pub fn update_selection(
        &mut self,
        active_index: usize,
        highlighted_index: usize,
        subtitle: &str,
    ) {
        self.active_index = active_index;
        self.highlighted_index = highlighted_index;
        self.subtitle = subtitle.to_string();

        if let Some(window) = &self.window {
            Self::apply_selection_to_window(
                window,
                &self.items,
                active_index,
                highlighted_index,
                subtitle,
            );
        }
    }

    pub fn show(
        &mut self,
        active_index: usize,
        highlighted_index: usize,
        subtitle: &str,
    ) -> Result<(), KnobError> {
        self.active_index = active_index;
        self.highlighted_index = highlighted_index;
        self.subtitle = subtitle.to_string();

        if self.window.is_none() {
            let window = Overlay::new().map_err(|err| KnobError::UiInit(err.to_string()))?;
            Self::apply_state_to_window(
                &window,
                &self.items,
                self.active_index,
                self.highlighted_index,
                &self.subtitle,
            );
            self.window = Some(window);
        } else if let Some(window) = &self.window {
            Self::apply_selection_to_window(
                window,
                &self.items,
                active_index,
                highlighted_index,
                subtitle,
            );
        }

        if let Some(window) = &self.window {
            window
                .show()
                .map_err(|err| KnobError::UiRun(err.to_string()))?;
        }
        Ok(())
    }

    pub fn hide(&mut self) -> Result<(), KnobError> {
        if let Some(window) = self.window.take() {
            window
                .hide()
                .map_err(|err| KnobError::UiRun(err.to_string()))?;
        }
        Ok(())
    }

    fn apply_state_to_window(
        window: &Overlay,
        items: &[String],
        active_index: usize,
        highlighted_index: usize,
        subtitle: &str,
    ) {
        let model: Vec<SharedString> = items.iter().map(SharedString::from).collect();
        window.set_profiles(ModelRc::from(Rc::new(VecModel::from(model))));
        Self::apply_selection_to_window(window, items, active_index, highlighted_index, subtitle);
    }

    fn apply_selection_to_window(
        window: &Overlay,
        items: &[String],
        active_index: usize,
        highlighted_index: usize,
        subtitle: &str,
    ) {
        let label = items
            .get(highlighted_index)
            .map(String::as_str)
            .unwrap_or(AUTO_ITEM_LABEL);
        window.set_active_profile_name(SharedString::from(label));
        window.set_mode_subtitle(SharedString::from(subtitle));
        window.set_active_index(active_index as i32);
        window.set_highlighted_index(highlighted_index as i32);
    }
}
