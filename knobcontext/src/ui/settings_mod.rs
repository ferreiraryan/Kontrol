use crate::actions_catalog::{
    action_summary, action_type_title, list_desktop_apps, ActionType, DesktopApp,
    ACTION_TYPE_TITLES,
};
use crate::config::{
    builtin_profile_presets, default_overlay_items, ActionConfig, Config, CustomGroupEntry,
    OverlayItemConfig, OverlayItemKind, OverlayKey3Behavior, ProfileConfig, GLOBAL_PROFILE_NAME,
};
use crate::context::ActiveWindowInfo;
use crate::error::KnobError;
use crate::ui::action_editor_mod::{format_captured_key, pick_file_uri, sync_editor_to_ui};
use crate::ui::action_editor_state::{ActionEditorState, ControlSlot};
use crate::ui::SettingsWindow;
use slint::{CloseRequestResponse, ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorTarget {
    ProfileSlot(ControlSlot),
    HubQuickAction(usize),
    HubGroupEntry { item_idx: usize, entry_idx: usize },
}

struct SettingsDraft {
    base_config: Config,
    active_tab: usize,
    selected_index: usize,
    hub_selected_index: usize,
    hub_group_selected_entry: usize,
    dirty: bool,
    preset_picker_open: bool,
    editor: ActionEditorState,
    editor_target: Option<EditorTarget>,
    desktop_apps: Vec<DesktopApp>,
}

type OnSavedCallback = Rc<dyn Fn(Config)>;
type OnNotifyCallback = Rc<dyn Fn()>;
type OnCaptureWindowCallback = Rc<dyn Fn() -> Option<ActiveWindowInfo>>;

/// Controlador da janela de configurações Slint (Perfis Contextuais + Hub Interativo + TOML).
///
/// A instância `SettingsWindow` é recriada a cada abertura para contornar o reset de
/// `WindowAttributes` do backend `winit` do Slint após `hide()` no Wayland.
pub struct SettingsController {
    window: RefCell<Option<SettingsWindow>>,
    draft: Rc<RefCell<SettingsDraft>>,
    is_open: Rc<RefCell<bool>>,
    on_saved: OnSavedCallback,
    on_closed: OnNotifyCallback,
    on_release_modifiers: OnNotifyCallback,
    on_capture_window: OnCaptureWindowCallback,
}

impl SettingsController {
    pub fn new(
        initial_config: Config,
        on_saved: impl Fn(Config) + 'static,
        on_closed: impl Fn() + 'static,
        on_release_modifiers: impl Fn() + 'static,
        on_capture_window: impl Fn() -> Option<ActiveWindowInfo> + 'static,
    ) -> Result<Self, KnobError> {
        let draft = Rc::new(RefCell::new(SettingsDraft {
            base_config: initial_config,
            active_tab: 0,
            selected_index: 0,
            hub_selected_index: 0,
            hub_group_selected_entry: 0,
            dirty: false,
            preset_picker_open: false,
            editor: ActionEditorState::default(),
            editor_target: None,
            desktop_apps: list_desktop_apps(),
        }));
        let is_open = Rc::new(RefCell::new(false));

        Ok(Self {
            window: RefCell::new(None),
            draft,
            is_open,
            on_saved: Rc::new(on_saved),
            on_closed: Rc::new(on_closed),
            on_release_modifiers: Rc::new(on_release_modifiers),
            on_capture_window: Rc::new(on_capture_window),
        })
    }

    fn create_wired_window(&self) -> Result<SettingsWindow, KnobError> {
        let window = SettingsWindow::new().map_err(|err| KnobError::UiInit(err.to_string()))?;

        let titles: Vec<SharedString> = ACTION_TYPE_TITLES
            .iter()
            .map(|t| SharedString::from(*t))
            .collect();
        window.set_editor_type_titles(ModelRc::from(Rc::new(VecModel::from(titles))));

        let preset_labels: Vec<SharedString> = builtin_profile_presets()
            .iter()
            .map(|p| SharedString::from(p.label))
            .collect();
        window.set_preset_labels(ModelRc::from(Rc::new(VecModel::from(preset_labels))));

        let draft = Rc::clone(&self.draft);
        let is_open = Rc::clone(&self.is_open);
        let on_saved = Rc::clone(&self.on_saved);
        let on_closed = Rc::clone(&self.on_closed);
        let on_release_modifiers = Rc::clone(&self.on_release_modifiers);
        let on_capture_window = Rc::clone(&self.on_capture_window);

        // Alternar aba (0 = Perfis & Contexto, 1 = Overlay / Hub)
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            window.on_select_tab(move |tab| {
                let Some(win) = weak.upgrade() else { return };
                let mut d = draft_ref.borrow_mut();
                if d.active_tab == 0 {
                    if let Err(err) = apply_profile_header_to_draft(&win, &mut d) {
                        win.set_error_message(SharedString::from(err.to_string()));
                        return;
                    }
                } else {
                    apply_hub_fields_to_draft(&win, &mut d);
                }
                d.active_tab = if tab == 1 { 1 } else { 0 };
                win.set_error_message(SharedString::from(""));
                sync_all_to_ui(&win, &d);
            });
        }

        // Selecionar perfil na lista lateral
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            window.on_select_profile(move |idx| {
                let Some(win) = weak.upgrade() else { return };
                let mut d = draft_ref.borrow_mut();
                if let Err(err) = apply_profile_header_to_draft(&win, &mut d) {
                    win.set_error_message(SharedString::from(err.to_string()));
                    return;
                }
                let target =
                    (idx.max(0) as usize).min(d.base_config.profiles.len().saturating_sub(1));
                d.selected_index = target;
                win.set_error_message(SharedString::from(""));
                sync_all_to_ui(&win, &d);
            });
        }

        // Adicionar novo perfil
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            window.on_add_profile(move || {
                let Some(win) = weak.upgrade() else { return };
                let mut d = draft_ref.borrow_mut();
                if let Err(err) = apply_profile_header_to_draft(&win, &mut d) {
                    win.set_error_message(SharedString::from(err.to_string()));
                    return;
                }

                let new_name = next_unique_profile_name(&d.base_config.profiles, "novo_perfil");
                d.base_config.profiles.push(ProfileConfig {
                    name: new_name,
                    window_class: None,
                    window_title: None,
                    fullscreen: None,
                    priority: 0,
                    inherits: Some(GLOBAL_PROFILE_NAME.to_string()),
                    knob_cw: ActionConfig::VolumeUp,
                    knob_ccw: ActionConfig::VolumeDown,
                    knob_press: None,
                    key1: ActionConfig::None,
                    key2: ActionConfig::None,
                    key3: ActionConfig::None,
                });
                d.selected_index = d.base_config.profiles.len() - 1;
                d.dirty = true;
                win.set_error_message(SharedString::from(""));
                sync_all_to_ui(&win, &d);
            });
        }

        // Abrir/fechar seletor de presets contextuais
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            window.on_toggle_preset_picker(move || {
                let Some(win) = weak.upgrade() else { return };
                let mut d = draft_ref.borrow_mut();
                d.preset_picker_open = !d.preset_picker_open;
                win.set_preset_picker_open(d.preset_picker_open);
            });
        }

        // Adicionar preset contextual selecionado
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            window.on_add_preset(move |idx| {
                let Some(win) = weak.upgrade() else { return };
                let presets = builtin_profile_presets();
                let Some(preset) = presets.get(idx.max(0) as usize) else {
                    return;
                };
                let mut d = draft_ref.borrow_mut();
                if let Err(err) = apply_profile_header_to_draft(&win, &mut d) {
                    win.set_error_message(SharedString::from(err.to_string()));
                    return;
                }
                let mut profile = preset.profile.clone();
                profile.name = next_unique_profile_name(&d.base_config.profiles, &profile.name);
                d.base_config.profiles.push(profile);
                d.selected_index = d.base_config.profiles.len() - 1;
                d.dirty = true;
                d.preset_picker_open = false;
                win.set_preset_picker_open(false);
                win.set_error_message(SharedString::from(""));
                sync_all_to_ui(&win, &d);
            });
        }

        // Capturar última janela ativa do Hyprland
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            let capture_cb = Rc::clone(&on_capture_window);
            window.on_capture_active_window(move || {
                let Some(win) = weak.upgrade() else { return };
                let Some(captured) = capture_cb() else {
                    win.set_captured_window_hint(SharedString::from(
                        "Nenhuma janela anterior detectada no Hyprland.",
                    ));
                    return;
                };
                let mut d = draft_ref.borrow_mut();
                let idx = d.selected_index;
                if let Some(profile) = d.base_config.profiles.get_mut(idx) {
                    if !captured.class.is_empty() {
                        profile.window_class = Some(captured.class.clone());
                        d.dirty = true;
                    }
                }
                win.set_captured_window_hint(SharedString::from(format!(
                    "Capturado: {} ({})",
                    captured.class,
                    if captured.title.is_empty() {
                        "sem título"
                    } else {
                        &captured.title
                    }
                )));
                sync_all_to_ui(&win, &d);
            });
        }

        // Alterar prioridade do perfil selecionado
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            window.on_change_priority(move |delta| {
                let Some(win) = weak.upgrade() else { return };
                let mut d = draft_ref.borrow_mut();
                let _ = apply_profile_header_to_draft(&win, &mut d);
                let idx = d.selected_index;
                if let Some(profile) = d.base_config.profiles.get_mut(idx) {
                    profile.priority = (profile.priority + delta).clamp(-100, 100);
                    d.dirty = true;
                }
                sync_all_to_ui(&win, &d);
            });
        }

        // Alternar regra "Somente tela cheia"
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            window.on_toggle_fullscreen_only(move || {
                let Some(win) = weak.upgrade() else { return };
                let mut d = draft_ref.borrow_mut();
                let _ = apply_profile_header_to_draft(&win, &mut d);
                let idx = d.selected_index;
                if let Some(profile) = d.base_config.profiles.get_mut(idx) {
                    profile.fullscreen = if profile.fullscreen == Some(true) {
                        None
                    } else {
                        Some(true)
                    };
                    d.dirty = true;
                }
                sync_all_to_ui(&win, &d);
            });
        }

        // Remover perfil selecionado (protege "global")
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            window.on_remove_profile(move || {
                let Some(win) = weak.upgrade() else { return };
                let mut d = draft_ref.borrow_mut();
                if let Some(profile) = d.base_config.profiles.get(d.selected_index) {
                    if profile.name.eq_ignore_ascii_case(GLOBAL_PROFILE_NAME) {
                        win.set_error_message(SharedString::from(
                            "O perfil 'global' não pode ser removido.",
                        ));
                        return;
                    }
                }
                if d.base_config.profiles.len() <= 1 {
                    win.set_error_message(SharedString::from(
                        "É necessário manter ao menos um perfil.",
                    ));
                    return;
                }
                let idx = d.selected_index;
                d.base_config.profiles.remove(idx);
                if d.selected_index >= d.base_config.profiles.len() {
                    d.selected_index = d.base_config.profiles.len() - 1;
                }
                d.dirty = true;
                win.set_error_message(SharedString::from(""));
                sync_all_to_ui(&win, &d);
            });
        }

        // Abrir popover visual de edição de ação (slot de perfil)
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            window.on_open_action_editor(move |slot_idx| {
                let Some(win) = weak.upgrade() else { return };
                let Some(slot) = ControlSlot::from_index(slot_idx.max(0) as usize) else {
                    return;
                };
                let mut d = draft_ref.borrow_mut();
                if let Err(err) = apply_profile_header_to_draft(&win, &mut d) {
                    win.set_error_message(SharedString::from(err.to_string()));
                    return;
                }
                let current_action = d
                    .base_config
                    .profiles
                    .get(d.selected_index)
                    .map(|p| get_slot_action(p, slot).to_action_type())
                    .unwrap_or(ActionType::None);
                d.editor_target = Some(EditorTarget::ProfileSlot(slot));
                d.editor.open(slot, &current_action);
                win.set_editor_error_message(SharedString::from(""));
                sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
            });
        }

        wire_hub_tab_callbacks(&window, Rc::clone(&draft));
        wire_action_editor_callbacks(&window, Rc::clone(&draft), Rc::clone(&on_release_modifiers));

        // Salvar e persistir em ~/.config/knobcontext/config.toml
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            let open_ref = Rc::clone(&is_open);
            let on_saved_cb = Rc::clone(&on_saved);
            let on_closed_cb = Rc::clone(&on_closed);
            window.on_save_requested(move || {
                let Some(win) = weak.upgrade() else { return };
                let mut d = draft_ref.borrow_mut();
                if d.active_tab == 0 {
                    if let Err(err) = apply_profile_header_to_draft(&win, &mut d) {
                        win.set_error_message(SharedString::from(err.to_string()));
                        return;
                    }
                } else {
                    apply_hub_fields_to_draft(&win, &mut d);
                }

                if !d
                    .base_config
                    .profiles
                    .iter()
                    .any(|p| p.name == d.base_config.active_profile)
                {
                    d.base_config.active_profile = d.base_config.profiles[0].name.clone();
                }

                match d.base_config.save_default_location() {
                    Ok(_) => {
                        d.dirty = false;
                        let saved = d.base_config.clone();
                        drop(d);
                        win.set_error_message(SharedString::from(""));
                        win.set_has_unsaved_changes(false);
                        *open_ref.borrow_mut() = false;
                        let _ = win.hide();
                        on_saved_cb(saved);
                        on_closed_cb();
                    }
                    Err(err) => {
                        win.set_error_message(SharedString::from(err.to_string()));
                    }
                }
            });
        }

        // Cancelar edição
        {
            let weak = window.as_weak();
            let open_ref = Rc::clone(&is_open);
            let on_closed_cb = Rc::clone(&on_closed);
            let on_release_cb = Rc::clone(&on_release_modifiers);
            window.on_cancel_requested(move || {
                *open_ref.borrow_mut() = false;
                if let Some(win) = weak.upgrade() {
                    win.set_editor_open(false);
                    let _ = win.hide();
                }
                on_release_cb();
                on_closed_cb();
            });
        }

        // Fechamento via botão X do compositor
        {
            let weak = window.as_weak();
            let open_ref = Rc::clone(&is_open);
            let on_closed_cb = Rc::clone(&on_closed);
            let on_release_cb = Rc::clone(&on_release_modifiers);
            window.window().on_close_requested(move || {
                *open_ref.borrow_mut() = false;
                if let Some(win) = weak.upgrade() {
                    win.set_editor_open(false);
                    let _ = win.hide();
                }
                on_release_cb();
                on_closed_cb();
                CloseRequestResponse::KeepWindowShown
            });
        }

        Ok(window)
    }

    pub fn open(&self, current_config: &Config) -> Result<(), KnobError> {
        if *self.is_open.borrow() {
            if let Some(window) = self.window.borrow().as_ref() {
                return window
                    .show()
                    .map_err(|err| KnobError::UiRun(err.to_string()));
            }
        }

        self.window.borrow_mut().take();
        let window = self.create_wired_window()?;

        {
            let mut d = self.draft.borrow_mut();
            d.base_config = current_config.clone();
            d.active_tab = 0;
            d.selected_index = current_config.active_profile_index();
            d.hub_selected_index = 0;
            d.hub_group_selected_entry = 0;
            d.dirty = false;
            d.preset_picker_open = false;
            d.editor.cancel();
            d.editor_target = None;
            sync_all_to_ui(&window, &d);
            sync_editor_to_ui(&window, &d.editor, &d.desktop_apps);
        }
        window.set_preset_picker_open(false);
        window.set_captured_window_hint(SharedString::from(""));
        window.set_error_message(SharedString::from(""));
        *self.is_open.borrow_mut() = true;
        window
            .show()
            .map_err(|err| KnobError::UiRun(err.to_string()))?;
        *self.window.borrow_mut() = Some(window);
        Ok(())
    }

    pub fn hide(&self) -> Result<(), KnobError> {
        *self.is_open.borrow_mut() = false;
        if let Some(window) = self.window.borrow_mut().take() {
            window.set_editor_open(false);
            window
                .hide()
                .map_err(|err| KnobError::UiRun(err.to_string()))?;
        }
        Ok(())
    }
}

fn wire_hub_tab_callbacks(window: &SettingsWindow, draft: Rc<RefCell<SettingsDraft>>) {
    // Selecionar item do Hub
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_hub_select_item(move |idx| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            apply_hub_fields_to_draft(&win, &mut d);
            let len = d.base_config.overlay.items.len();
            if len > 0 {
                d.hub_selected_index = (idx.max(0) as usize).min(len - 1);
                d.hub_group_selected_entry = 0;
            }
            sync_all_to_ui(&win, &d);
        });
    }

    // Reordenar item do Hub (-1 sobe, +1 desce)
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_hub_move_item(move |dir| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            apply_hub_fields_to_draft(&win, &mut d);
            let len = d.base_config.overlay.items.len();
            let idx = d.hub_selected_index;
            if dir < 0 && idx > 0 && idx < len {
                d.base_config.overlay.items.swap(idx, idx - 1);
                d.hub_selected_index = idx - 1;
                d.dirty = true;
            } else if dir > 0 && idx + 1 < len {
                d.base_config.overlay.items.swap(idx, idx + 1);
                d.hub_selected_index = idx + 1;
                d.dirty = true;
            }
            sync_all_to_ui(&win, &d);
        });
    }

    // Habilitar/desabilitar item do Hub
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_hub_toggle_enabled(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            apply_hub_fields_to_draft(&win, &mut d);
            let idx = d.hub_selected_index;
            if let Some(item) = d.base_config.overlay.items.get_mut(idx) {
                item.enabled = !item.enabled;
                d.dirty = true;
            }
            sync_all_to_ui(&win, &d);
        });
    }

    // Adicionar Ação Rápida ao Hub
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_hub_add_quick_action(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            apply_hub_fields_to_draft(&win, &mut d);
            let id = next_unique_hub_id(&d.base_config.overlay.items, "quick_action");
            d.base_config.overlay.items.push(OverlayItemConfig {
                id,
                title: "Ação Rápida".to_string(),
                enabled: true,
                icon: None,
                kind: OverlayItemKind::QuickAction {
                    action: ActionConfig::MediaPlayPause,
                },
            });
            d.hub_selected_index = d.base_config.overlay.items.len() - 1;
            d.dirty = true;
            sync_all_to_ui(&win, &d);
        });
    }

    // Adicionar Slider Customizado ao Hub
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_hub_add_custom_slider(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            apply_hub_fields_to_draft(&win, &mut d);
            let id = next_unique_hub_id(&d.base_config.overlay.items, "custom_slider");
            d.base_config.overlay.items.push(OverlayItemConfig {
                id,
                title: "Slider Customizado".to_string(),
                enabled: true,
                icon: None,
                kind: OverlayItemKind::CustomSlider {
                    step: 5,
                    up_command: String::new(),
                    down_command: String::new(),
                    toggle_command: None,
                    read_command: None,
                },
            });
            d.hub_selected_index = d.base_config.overlay.items.len() - 1;
            d.dirty = true;
            sync_all_to_ui(&win, &d);
        });
    }

    // Adicionar Submenu Customizado ao Hub
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_hub_add_custom_group(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            apply_hub_fields_to_draft(&win, &mut d);
            let id = next_unique_hub_id(&d.base_config.overlay.items, "custom_group");
            d.base_config.overlay.items.push(OverlayItemConfig {
                id,
                title: "Submenu Customizado".to_string(),
                enabled: true,
                icon: None,
                kind: OverlayItemKind::CustomGroup {
                    entries: vec![CustomGroupEntry {
                        label: "Nova ação".to_string(),
                        action: ActionConfig::None,
                        icon: None,
                    }],
                },
            });
            d.hub_selected_index = d.base_config.overlay.items.len() - 1;
            d.hub_group_selected_entry = 0;
            d.dirty = true;
            sync_all_to_ui(&win, &d);
        });
    }

    // Remover item do Hub
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_hub_remove_item(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            if d.base_config.overlay.items.len() <= 1 {
                win.set_error_message(SharedString::from(
                    "É necessário manter ao menos um item no Hub.",
                ));
                return;
            }
            let idx = d.hub_selected_index;
            if idx < d.base_config.overlay.items.len() {
                d.base_config.overlay.items.remove(idx);
                if d.hub_selected_index >= d.base_config.overlay.items.len() {
                    d.hub_selected_index = d.base_config.overlay.items.len() - 1;
                }
                d.hub_group_selected_entry = 0;
                d.dirty = true;
            }
            win.set_error_message(SharedString::from(""));
            sync_all_to_ui(&win, &d);
        });
    }

    // Restaurar itens padrão do Hub
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_hub_reset_defaults(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            d.base_config.overlay.items = default_overlay_items();
            d.hub_selected_index = 0;
            d.hub_group_selected_entry = 0;
            d.dirty = true;
            sync_all_to_ui(&win, &d);
        });
    }

    // Editar ação de QuickAction via ActionEditorPopover
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_hub_edit_quick_action(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            apply_hub_fields_to_draft(&win, &mut d);
            let idx = d.hub_selected_index;
            let info = d
                .base_config
                .overlay
                .items
                .get(idx)
                .and_then(|item| match &item.kind {
                    OverlayItemKind::QuickAction { action } => {
                        Some((item.title.clone(), action.to_action_type()))
                    }
                    _ => None,
                });
            if let Some((title, act)) = info {
                d.editor_target = Some(EditorTarget::HubQuickAction(idx));
                d.editor.open_custom(format!("Hub: {title}"), &act);
                win.set_editor_error_message(SharedString::from(""));
                sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
            }
        });
    }

    // Passo do Slider Customizado
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_hub_change_slider_step(move |delta| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            apply_hub_fields_to_draft(&win, &mut d);
            let idx = d.hub_selected_index;
            if let Some(OverlayItemConfig {
                kind: OverlayItemKind::CustomSlider { step, .. },
                ..
            }) = d.base_config.overlay.items.get_mut(idx)
            {
                *step = (*step + delta).clamp(1, 50);
                d.dirty = true;
            }
            sync_all_to_ui(&win, &d);
        });
    }

    // Selecionar entrada de CustomGroup
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_hub_group_select_entry(move |eidx| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            apply_hub_fields_to_draft(&win, &mut d);
            d.hub_group_selected_entry = eidx.max(0) as usize;
            sync_all_to_ui(&win, &d);
        });
    }

    // Adicionar entrada em CustomGroup
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_hub_group_add_entry(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            apply_hub_fields_to_draft(&win, &mut d);
            let idx = d.hub_selected_index;
            if let Some(OverlayItemConfig {
                kind: OverlayItemKind::CustomGroup { entries },
                ..
            }) = d.base_config.overlay.items.get_mut(idx)
            {
                let n = entries.len() + 1;
                entries.push(CustomGroupEntry {
                    label: format!("Ação {n}"),
                    action: ActionConfig::None,
                    icon: None,
                });
                d.hub_group_selected_entry = entries.len() - 1;
                d.dirty = true;
            }
            sync_all_to_ui(&win, &d);
        });
    }

    // Remover entrada de CustomGroup
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_hub_group_remove_entry(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            apply_hub_fields_to_draft(&win, &mut d);
            let idx = d.hub_selected_index;
            let eidx = d.hub_group_selected_entry;
            if let Some(OverlayItemConfig {
                kind: OverlayItemKind::CustomGroup { entries },
                ..
            }) = d.base_config.overlay.items.get_mut(idx)
            {
                if eidx < entries.len() {
                    entries.remove(eidx);
                    d.hub_group_selected_entry = eidx.min(entries.len().saturating_sub(1));
                    d.dirty = true;
                }
            }
            sync_all_to_ui(&win, &d);
        });
    }

    // Editar ação da entrada selecionada de CustomGroup via ActionEditorPopover
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_hub_group_edit_entry_action(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            apply_hub_fields_to_draft(&win, &mut d);
            let idx = d.hub_selected_index;
            let eidx = d.hub_group_selected_entry;
            let info = d
                .base_config
                .overlay
                .items
                .get(idx)
                .and_then(|item| match &item.kind {
                    OverlayItemKind::CustomGroup { entries } => entries
                        .get(eidx)
                        .map(|e| (e.label.clone(), e.action.to_action_type())),
                    _ => None,
                });
            if let Some((label, act)) = info {
                d.editor_target = Some(EditorTarget::HubGroupEntry {
                    item_idx: idx,
                    entry_idx: eidx,
                });
                d.editor.open_custom(format!("Submenu: {label}"), &act);
                win.set_editor_error_message(SharedString::from(""));
                sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
            }
        });
    }

    // Ajustes globais do Hub (passos, timeouts e Key3)
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_hub_change_volume_step(move |delta| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            d.base_config.overlay.volume_step =
                (d.base_config.overlay.volume_step + delta).clamp(1, 50);
            d.dirty = true;
            sync_all_to_ui(&win, &d);
        });
    }
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_hub_change_brightness_step(move |delta| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            d.base_config.overlay.brightness_step =
                (d.base_config.overlay.brightness_step + delta).clamp(1, 50);
            d.dirty = true;
            sync_all_to_ui(&win, &d);
        });
    }
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_hub_change_browse_timeout(move |delta_sec| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            let cur = (d.base_config.overlay.browse_timeout_ms / 1_000) as i64;
            let next = (cur + i64::from(delta_sec)).clamp(2, 30) as u64;
            d.base_config.overlay.browse_timeout_ms = next * 1_000;
            d.dirty = true;
            sync_all_to_ui(&win, &d);
        });
    }
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_hub_change_adjust_timeout(move |delta_sec| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            let cur = (d.base_config.overlay.adjust_timeout_ms / 1_000) as i64;
            let next = (cur + i64::from(delta_sec)).clamp(3, 60) as u64;
            d.base_config.overlay.adjust_timeout_ms = next * 1_000;
            d.dirty = true;
            sync_all_to_ui(&win, &d);
        });
    }
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_hub_toggle_key3_behavior(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            d.base_config.overlay.key3_behavior = match d.base_config.overlay.key3_behavior {
                OverlayKey3Behavior::Confirm => OverlayKey3Behavior::Close,
                OverlayKey3Behavior::Close => OverlayKey3Behavior::Confirm,
            };
            d.dirty = true;
            sync_all_to_ui(&win, &d);
        });
    }
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_hub_toggle_show_images(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            d.base_config.overlay.show_images = !d.base_config.overlay.show_images;
            d.dirty = true;
            sync_all_to_ui(&win, &d);
        });
    }
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_hub_toggle_animations(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            d.base_config.overlay.animations_enabled = !d.base_config.overlay.animations_enabled;
            d.dirty = true;
            sync_all_to_ui(&win, &d);
        });
    }
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_hub_pick_item_icon(move || {
            let Some(win) = weak.upgrade() else { return };
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Imagens", &["png", "jpg", "jpeg", "webp", "svg"])
                .pick_file()
            {
                let mut d = draft_ref.borrow_mut();
                apply_hub_fields_to_draft(&win, &mut d);
                let idx = d.hub_selected_index;
                if let Some(item) = d.base_config.overlay.items.get_mut(idx) {
                    item.icon = Some(path.to_string_lossy().into_owned());
                    d.dirty = true;
                }
                sync_all_to_ui(&win, &d);
            }
        });
    }
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_hub_clear_item_icon(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            apply_hub_fields_to_draft(&win, &mut d);
            let idx = d.hub_selected_index;
            if let Some(item) = d.base_config.overlay.items.get_mut(idx) {
                if item.icon.take().is_some() {
                    d.dirty = true;
                }
            }
            sync_all_to_ui(&win, &d);
        });
    }
}

fn wire_action_editor_callbacks(
    window: &SettingsWindow,
    draft: Rc<RefCell<SettingsDraft>>,
    on_release_modifiers: Rc<dyn Fn()>,
) {
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        let release_cb = Rc::clone(&on_release_modifiers);
        window.on_editor_select_type(move |idx| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            let was_capturing = d.editor.capturing_shortcut || d.editor.capturing_macro_key;
            pull_editor_text_fields(&win, &mut d.editor);
            d.editor.select_type(idx.max(0) as usize);
            win.set_editor_error_message(SharedString::from(""));
            sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
            if was_capturing {
                release_cb();
            }
        });
    }

    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_editor_toggle_shortcut_capture(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            d.editor.capturing_shortcut = !d.editor.capturing_shortcut;
            sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
        });
    }

    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_editor_shortcut_key_captured(move |text, ctrl, alt, shift, meta| {
            let Some(win) = weak.upgrade() else { return };
            if let Some(combo) = format_captured_key(text.as_str(), ctrl, alt, shift, meta) {
                let mut d = draft_ref.borrow_mut();
                d.editor.shortcut_text = combo;
                d.editor.capturing_shortcut = false;
                win.set_editor_error_message(SharedString::from(""));
                sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
            }
        });
    }

    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_editor_select_media(move |idx| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            d.editor.media_index = (idx.max(0) as usize).min(5);
            sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
        });
    }

    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_editor_app_search_changed(move |query| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            d.editor.app_search = query.to_string();
            sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
        });
    }

    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_editor_select_app(move |filtered_idx| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            let chosen = d
                .editor
                .filtered_apps(&d.desktop_apps)
                .get(filtered_idx.max(0) as usize)
                .map(|app| app.desktop_file.clone());
            if let Some(desktop_file) = chosen {
                d.editor.selected_desktop_file = desktop_file;
                win.set_editor_error_message(SharedString::from(""));
                sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
            }
        });
    }

    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_editor_pick_file(move || {
            let Some(win) = weak.upgrade() else { return };
            if let Some(uri) = pick_file_uri() {
                let mut d = draft_ref.borrow_mut();
                d.editor.file_uri = uri;
                win.set_editor_error_message(SharedString::from(""));
                sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
            }
        });
    }

    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_editor_select_scroll_direction(move |idx| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            d.editor.scroll_direction_index = (idx.max(0) as usize).min(3);
            sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
        });
    }

    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_editor_change_scroll_amount(move |delta| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            let next = (i32::from(d.editor.scroll_amount) + delta).clamp(1, 10) as u8;
            d.editor.scroll_amount = next;
            sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
        });
    }

    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_editor_toggle_macro_capture(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            d.editor.capturing_macro_key = !d.editor.capturing_macro_key;
            sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
        });
    }

    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_editor_macro_key_captured(move |text, ctrl, alt, shift, meta| {
            let Some(win) = weak.upgrade() else { return };
            if let Some(combo) = format_captured_key(text.as_str(), ctrl, alt, shift, meta) {
                let mut d = draft_ref.borrow_mut();
                d.editor.macro_key_input = combo;
                d.editor.capturing_macro_key = false;
                win.set_editor_error_message(SharedString::from(""));
                sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
            }
        });
    }

    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_editor_add_macro_key(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            let raw = win.get_editor_macro_key_text().to_string();
            match d.editor.add_macro_key_step(&raw) {
                Ok(()) => {
                    win.set_editor_error_message(SharedString::from(""));
                    sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
                }
                Err(err) => win.set_editor_error_message(SharedString::from(err.to_string())),
            }
        });
    }

    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_editor_add_macro_wait(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            let raw = win.get_editor_macro_wait_text().to_string();
            match d.editor.add_macro_wait_step(&raw) {
                Ok(()) => {
                    win.set_editor_error_message(SharedString::from(""));
                    sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
                }
                Err(err) => win.set_editor_error_message(SharedString::from(err.to_string())),
            }
        });
    }

    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_editor_remove_macro_step(move |idx| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            d.editor.remove_macro_step(idx.max(0) as usize);
            sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
        });
    }

    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        let release_cb = Rc::clone(&on_release_modifiers);
        window.on_editor_save(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            pull_editor_text_fields(&win, &mut d.editor);
            match d.editor.build_action() {
                Ok(action_type) => {
                    log::debug!(
                        "Ação configurada ({}): {:?}",
                        action_type_title(&action_type),
                        d.editor_target
                    );
                    let action_cfg = ActionConfig::from_action_type(action_type);
                    match d.editor_target {
                        Some(EditorTarget::ProfileSlot(slot)) => {
                            let idx = d.selected_index;
                            if let Some(profile) = d.base_config.profiles.get_mut(idx) {
                                set_slot_action(profile, slot, action_cfg);
                                d.dirty = true;
                            }
                        }
                        Some(EditorTarget::HubQuickAction(item_idx)) => {
                            if let Some(OverlayItemConfig {
                                kind: OverlayItemKind::QuickAction { action },
                                ..
                            }) = d.base_config.overlay.items.get_mut(item_idx)
                            {
                                *action = action_cfg;
                                d.dirty = true;
                            }
                        }
                        Some(EditorTarget::HubGroupEntry {
                            item_idx,
                            entry_idx,
                        }) => {
                            if let Some(OverlayItemConfig {
                                kind: OverlayItemKind::CustomGroup { entries },
                                ..
                            }) = d.base_config.overlay.items.get_mut(item_idx)
                            {
                                if let Some(entry) = entries.get_mut(entry_idx) {
                                    entry.action = action_cfg;
                                    d.dirty = true;
                                }
                            }
                        }
                        None => {}
                    }
                    d.editor.cancel();
                    d.editor_target = None;
                    win.set_editor_error_message(SharedString::from(""));
                    sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
                    sync_all_to_ui(&win, &d);
                    release_cb();
                }
                Err(err) => win.set_editor_error_message(SharedString::from(err.to_string())),
            }
        });
    }

    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        let release_cb = Rc::clone(&on_release_modifiers);
        window.on_editor_cancel(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            d.editor.cancel();
            d.editor_target = None;
            win.set_editor_error_message(SharedString::from(""));
            sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
            release_cb();
        });
    }
}

fn pull_editor_text_fields(win: &SettingsWindow, editor: &mut ActionEditorState) {
    editor.shortcut_text = win.get_editor_shortcut_text().to_string();
    editor.app_search = win.get_editor_app_search().to_string();
    editor.url_text = win.get_editor_url_text().to_string();
    editor.file_uri = win.get_editor_file_text().to_string();
    editor.exec_command = win.get_editor_exec_text().to_string();
    editor.macro_key_input = win.get_editor_macro_key_text().to_string();
    editor.macro_wait_ms_input = win.get_editor_macro_wait_text().to_string();
}

fn get_slot_action(profile: &ProfileConfig, slot: ControlSlot) -> &ActionConfig {
    match slot {
        ControlSlot::RotateCw => &profile.knob_cw,
        ControlSlot::RotateCcw => &profile.knob_ccw,
        ControlSlot::Key1 => &profile.key1,
        ControlSlot::Key2 => &profile.key2,
        ControlSlot::Key3 => &profile.key3,
    }
}

fn set_slot_action(profile: &mut ProfileConfig, slot: ControlSlot, action: ActionConfig) {
    match slot {
        ControlSlot::RotateCw => profile.knob_cw = action,
        ControlSlot::RotateCcw => profile.knob_ccw = action,
        ControlSlot::Key1 => profile.key1 = action,
        ControlSlot::Key2 => profile.key2 = action,
        ControlSlot::Key3 => profile.key3 = action,
    }
}

fn next_unique_profile_name(profiles: &[ProfileConfig], base: &str) -> String {
    let base_clean = if base.trim().is_empty() {
        "novo_perfil"
    } else {
        base.trim()
    };
    let mut counter = 1usize;
    loop {
        let candidate = if counter == 1 {
            base_clean.to_string()
        } else {
            format!("{base_clean}_{counter}")
        };
        if !profiles
            .iter()
            .any(|p| p.name.eq_ignore_ascii_case(&candidate))
        {
            return candidate;
        }
        counter += 1;
    }
}

fn next_unique_hub_id(items: &[OverlayItemConfig], base: &str) -> String {
    let mut counter = 1usize;
    loop {
        let candidate = format!("{base}_{counter}");
        if !items.iter().any(|i| i.id == candidate) {
            return candidate;
        }
        counter += 1;
    }
}

fn format_summary(action: &ActionConfig, apps: &[DesktopApp]) -> SharedString {
    SharedString::from(action_summary(&action.to_action_type(), apps))
}

fn sync_all_to_ui(win: &SettingsWindow, draft: &SettingsDraft) {
    win.set_active_tab(draft.active_tab as i32);
    win.set_has_unsaved_changes(draft.dirty);
    sync_profile_tab_to_ui(win, draft);
    sync_hub_tab_to_ui(win, draft);
}

fn sync_profile_tab_to_ui(win: &SettingsWindow, draft: &SettingsDraft) {
    let names: Vec<SharedString> = draft
        .base_config
        .profiles
        .iter()
        .map(|p| SharedString::from(p.name.as_str()))
        .collect();
    win.set_profile_names(ModelRc::from(Rc::new(VecModel::from(names))));
    win.set_selected_index(draft.selected_index as i32);

    if let Some(profile) = draft.base_config.profiles.get(draft.selected_index) {
        win.set_field_name(SharedString::from(profile.name.as_str()));
        win.set_field_window_class(SharedString::from(
            profile.window_class.as_deref().unwrap_or(""),
        ));
        win.set_field_window_title(SharedString::from(
            profile.window_title.as_deref().unwrap_or(""),
        ));
        win.set_field_inherits(SharedString::from(
            profile.inherits.as_deref().unwrap_or(""),
        ));
        win.set_field_priority(profile.priority);
        win.set_field_fullscreen_only(profile.fullscreen == Some(true));
        win.set_summary_rotate_cw(format_summary(&profile.knob_cw, &draft.desktop_apps));
        win.set_summary_rotate_ccw(format_summary(&profile.knob_ccw, &draft.desktop_apps));
        win.set_summary_key1(format_summary(&profile.key1, &draft.desktop_apps));
        win.set_summary_key2(format_summary(&profile.key2, &draft.desktop_apps));
        win.set_summary_key3(format_summary(&profile.key3, &draft.desktop_apps));
    }
}

fn sync_hub_tab_to_ui(win: &SettingsWindow, draft: &SettingsDraft) {
    let overlay = &draft.base_config.overlay;
    win.set_hub_volume_step(overlay.volume_step);
    win.set_hub_brightness_step(overlay.brightness_step);
    win.set_hub_browse_timeout_sec((overlay.browse_timeout_ms / 1_000) as i32);
    win.set_hub_adjust_timeout_sec((overlay.adjust_timeout_ms / 1_000) as i32);
    win.set_hub_key3_is_confirm(overlay.key3_behavior == OverlayKey3Behavior::Confirm);
    win.set_hub_show_images(overlay.show_images);
    win.set_hub_animations_enabled(overlay.animations_enabled);

    let labels: Vec<SharedString> = overlay
        .items
        .iter()
        .map(|i| {
            let prefix = if i.enabled { "● " } else { "○ " };
            SharedString::from(format!("{prefix}{}", i.title))
        })
        .collect();
    win.set_hub_item_labels(ModelRc::from(Rc::new(VecModel::from(labels))));
    win.set_hub_selected_index(draft.hub_selected_index as i32);

    if let Some(item) = overlay.items.get(draft.hub_selected_index) {
        win.set_hub_item_title(SharedString::from(item.title.as_str()));
        win.set_hub_item_kind_label(SharedString::from(item.kind.kind_label()));
        win.set_hub_item_enabled(item.enabled);
        let icon_label = item
            .icon
            .as_deref()
            .and_then(|p| std::path::Path::new(p).file_name().and_then(|n| n.to_str()))
            .or(item.icon.as_deref())
            .unwrap_or("Automático / nenhuma");
        win.set_hub_item_icon_label(SharedString::from(icon_label));

        match &item.kind {
            OverlayItemKind::QuickAction { action } => {
                win.set_hub_item_editor_mode(1);
                win.set_hub_quick_action_summary(format_summary(action, &draft.desktop_apps));
            }
            OverlayItemKind::CustomSlider {
                step,
                up_command,
                down_command,
                toggle_command,
                read_command,
            } => {
                win.set_hub_item_editor_mode(2);
                win.set_hub_slider_step(*step);
                win.set_hub_slider_up(SharedString::from(up_command.as_str()));
                win.set_hub_slider_down(SharedString::from(down_command.as_str()));
                win.set_hub_slider_toggle(SharedString::from(
                    toggle_command.as_deref().unwrap_or(""),
                ));
                win.set_hub_slider_read(SharedString::from(read_command.as_deref().unwrap_or("")));
            }
            OverlayItemKind::CustomGroup { entries } => {
                win.set_hub_item_editor_mode(3);
                let elabels: Vec<SharedString> = entries
                    .iter()
                    .map(|e| {
                        SharedString::from(format!(
                            "{} — {}",
                            e.label,
                            action_summary(&e.action.to_action_type(), &draft.desktop_apps)
                        ))
                    })
                    .collect();
                win.set_hub_group_entry_labels(ModelRc::from(Rc::new(VecModel::from(elabels))));
                let eidx = draft
                    .hub_group_selected_entry
                    .min(entries.len().saturating_sub(1));
                win.set_hub_group_selected_entry(eidx as i32);
                if let Some(entry) = entries.get(eidx) {
                    win.set_hub_group_entry_name(SharedString::from(entry.label.as_str()));
                    win.set_hub_group_entry_action_summary(format_summary(
                        &entry.action,
                        &draft.desktop_apps,
                    ));
                } else {
                    win.set_hub_group_entry_name(SharedString::from(""));
                    win.set_hub_group_entry_action_summary(SharedString::from("Nenhuma"));
                }
            }
            _ => {
                win.set_hub_item_editor_mode(0);
            }
        }
    }
}

fn apply_profile_header_to_draft(
    win: &SettingsWindow,
    draft: &mut SettingsDraft,
) -> Result<(), KnobError> {
    let Some(profile) = draft.base_config.profiles.get_mut(draft.selected_index) else {
        return Ok(());
    };

    let name = win.get_field_name().trim().to_string();
    if name.is_empty() {
        return Err(KnobError::ConfigValidation(
            "nome do perfil não pode ficar vazio".to_string(),
        ));
    }

    let window_class = normalize_optional_field(win.get_field_window_class().as_str());
    let window_title = normalize_optional_field(win.get_field_window_title().as_str());
    let inherits = normalize_optional_field(win.get_field_inherits().as_str());

    if profile.name != name
        || profile.window_class != window_class
        || profile.window_title != window_title
        || profile.inherits != inherits
    {
        profile.name = name;
        profile.window_class = window_class;
        profile.window_title = window_title;
        profile.inherits = inherits;
        draft.dirty = true;
    }

    Ok(())
}

fn apply_hub_fields_to_draft(win: &SettingsWindow, draft: &mut SettingsDraft) {
    let idx = draft.hub_selected_index;
    let eidx = draft.hub_group_selected_entry;
    let Some(item) = draft.base_config.overlay.items.get_mut(idx) else {
        return;
    };

    let new_title = win.get_hub_item_title().trim().to_string();
    if !new_title.is_empty() && item.title != new_title {
        item.title = new_title;
        draft.dirty = true;
    }

    match &mut item.kind {
        OverlayItemKind::CustomSlider {
            up_command,
            down_command,
            toggle_command,
            read_command,
            ..
        } => {
            let up = win.get_hub_slider_up().trim().to_string();
            let down = win.get_hub_slider_down().trim().to_string();
            let toggle = normalize_optional_field(win.get_hub_slider_toggle().as_str());
            let read = normalize_optional_field(win.get_hub_slider_read().as_str());
            if *up_command != up
                || *down_command != down
                || *toggle_command != toggle
                || *read_command != read
            {
                *up_command = up;
                *down_command = down;
                *toggle_command = toggle;
                *read_command = read;
                draft.dirty = true;
            }
        }
        OverlayItemKind::CustomGroup { entries } => {
            if let Some(entry) = entries.get_mut(eidx) {
                let label = win.get_hub_group_entry_name().trim().to_string();
                if !label.is_empty() && entry.label != label {
                    entry.label = label;
                    draft.dirty = true;
                }
            }
        }
        _ => {}
    }
}

fn normalize_optional_field(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
