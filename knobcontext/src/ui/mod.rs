pub mod action_editor_mod;
pub mod action_editor_state;
pub mod settings_mod;

use crate::error::KnobError;
use crate::overlay_hub::HubViewModel;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::rc::Rc;

slint::include_modules!();

/// Gerencia o ciclo de vida da janela Overlay (Hub Interativo) em Slint.
///
/// No Wayland, `slint::Window::hide()` suspende/destrói a janela `winit` subjacente e
/// reinicia `WindowAttributes` para o padrão (`resizable = true`, sem `min/max_inner_size`),
/// fazendo compositores tiling (como Hyprland) abrirem reaberturas em modo tile.
/// Por isso, descartamos a instância ao fechar e criamos um `Overlay` novo a cada abertura.
pub struct OverlayWindow {
    window: Option<Overlay>,
}

impl OverlayWindow {
    pub fn new() -> Self {
        Self { window: None }
    }

    pub fn render(&mut self, vm: &HubViewModel) -> Result<(), KnobError> {
        let needs_show = self.window.is_none();
        if self.window.is_none() {
            let window = Overlay::new().map_err(|err| KnobError::UiInit(err.to_string()))?;
            self.window = Some(window);
        }

        if let Some(window) = &self.window {
            Self::apply_view_model(window, vm);
            if needs_show {
                window
                    .show()
                    .map_err(|err| KnobError::UiRun(err.to_string()))?;
            }
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

    fn apply_view_model(window: &Overlay, vm: &HubViewModel) {
        let rows: Vec<OverlayRowData> = vm
            .rows
            .iter()
            .map(|r| OverlayRowData {
                title: SharedString::from(r.title.as_str()),
                subtitle: SharedString::from(r.subtitle.as_str()),
                badge: SharedString::from(r.badge.as_str()),
                value_percent: r.value_percent,
                max_percent: r.max_percent,
                is_slider: r.is_slider,
                is_active: r.is_active,
                is_muted: r.is_muted,
            })
            .collect();

        window.set_breadcrumb(SharedString::from(vm.breadcrumb.as_str()));
        window.set_context_badge(SharedString::from(vm.context_badge.as_str()));
        window.set_adjusting(vm.adjusting);
        window.set_loading(vm.loading);
        window.set_empty_message(SharedString::from(vm.empty_message.as_str()));
        window.set_rows(ModelRc::from(Rc::new(VecModel::from(rows))));
        window.set_highlighted_index(vm.highlighted_visible_index as i32);
        window.set_scroll_indicator(SharedString::from(vm.scroll_indicator.as_str()));
        window.set_footer_hint(SharedString::from(vm.footer_hint.as_str()));
    }
}
