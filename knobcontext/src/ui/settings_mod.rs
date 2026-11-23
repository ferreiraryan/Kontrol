use crate::actions_catalog::{
    action_summary, action_type_title, list_desktop_apps, DesktopApp, ACTION_TYPE_TITLES,
};
use crate::config::{ActionConfig, Config, ProfileConfig, GLOBAL_PROFILE_NAME};
use crate::error::KnobError;
use crate::ui::action_editor_mod::{format_captured_key, pick_file_uri, sync_editor_to_ui};
use crate::ui::action_editor_state::{ActionEditorState, ControlSlot};
use crate::ui::SettingsWindow;
use slint::{CloseRequestResponse, ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

struct SettingsDraft {
    base_config: Config,
    selected_index: usize,
    dirty: bool,
    editor: ActionEditorState,
    desktop_apps: Vec<DesktopApp>,
}

/// Controlador da janela de configurações Slint (edição visual de perfis e persistência TOML).
pub struct SettingsController {
    window: SettingsWindow,
    draft: Rc<RefCell<SettingsDraft>>,
    is_open: Rc<RefCell<bool>>,
}

impl SettingsController {
    pub fn new(
        initial_config: Config,
        on_saved: impl Fn(Config) + 'static,
        on_closed: impl Fn() + 'static,
    ) -> Result<Self, KnobError> {
        let window = SettingsWindow::new().map_err(|err| KnobError::UiInit(err.to_string()))?;

        let titles: Vec<SharedString> = ACTION_TYPE_TITLES
            .iter()
            .map(|t| SharedString::from(*t))
            .collect();
        window.set_editor_type_titles(ModelRc::from(Rc::new(VecModel::from(titles))));

        let draft = Rc::new(RefCell::new(SettingsDraft {
            base_config: initial_config,
            selected_index: 0,
            dirty: false,
            editor: ActionEditorState::default(),
            desktop_apps: list_desktop_apps(),
        }));
        let is_open = Rc::new(RefCell::new(false));

        let on_saved = Rc::new(on_saved);
        let on_closed = Rc::new(on_closed);

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
                sync_draft_to_ui(&win, &d);
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

                let new_name = next_unique_profile_name(&d.base_config.profiles);
                d.base_config.profiles.push(ProfileConfig {
                    name: new_name,
                    window_class: None,
                    knob_cw: ActionConfig::VolumeUp,
                    knob_ccw: ActionConfig::VolumeDown,
                    knob_press: ActionConfig::None,
                    key1: ActionConfig::None,
                    key2: ActionConfig::None,
                    key3: ActionConfig::None,
                });
                d.selected_index = d.base_config.profiles.len() - 1;
                d.dirty = true;
                win.set_error_message(SharedString::from(""));
                sync_draft_to_ui(&win, &d);
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
                sync_draft_to_ui(&win, &d);
            });
        }

        // Abrir popover visual de edição de ação
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
                    .unwrap_or(crate::actions_catalog::ActionType::None);
                d.editor.open(slot, &current_action);
                win.set_editor_error_message(SharedString::from(""));
                sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
            });
        }

        wire_action_editor_callbacks(&window, Rc::clone(&draft));

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
                if let Err(err) = apply_profile_header_to_draft(&win, &mut d) {
                    win.set_error_message(SharedString::from(err.to_string()));
                    return;
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
            window.on_cancel_requested(move || {
                *open_ref.borrow_mut() = false;
                if let Some(win) = weak.upgrade() {
                    win.set_editor_open(false);
                    let _ = win.hide();
                }
                on_closed_cb();
            });
        }

        // Fechamento via botão X do compositor
        {
            let weak = window.as_weak();
            let open_ref = Rc::clone(&is_open);
            let on_closed_cb = Rc::clone(&on_closed);
            window.window().on_close_requested(move || {
                *open_ref.borrow_mut() = false;
                if let Some(win) = weak.upgrade() {
                    win.set_editor_open(false);
                    let _ = win.hide();
                }
                on_closed_cb();
                CloseRequestResponse::KeepWindowShown
            });
        }

        Ok(Self {
            window,
            draft,
            is_open,
        })
    }

    pub fn open(&self, current_config: &Config) -> Result<(), KnobError> {
        {
            let mut d = self.draft.borrow_mut();
            d.base_config = current_config.clone();
            d.selected_index = current_config.active_profile_index();
            d.dirty = false;
            d.editor.cancel();
            sync_draft_to_ui(&self.window, &d);
            sync_editor_to_ui(&self.window, &d.editor, &d.desktop_apps);
        }
        self.window.set_error_message(SharedString::from(""));
        *self.is_open.borrow_mut() = true;
        self.window
            .show()
            .map_err(|err| KnobError::UiRun(err.to_string()))
    }

    pub fn hide(&self) -> Result<(), KnobError> {
        *self.is_open.borrow_mut() = false;
        self.window.set_editor_open(false);
        self.window
            .hide()
            .map_err(|err| KnobError::UiRun(err.to_string()))
    }
}

fn wire_action_editor_callbacks(window: &SettingsWindow, draft: Rc<RefCell<SettingsDraft>>) {
    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_editor_select_type(move |idx| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            pull_editor_text_fields(&win, &mut d.editor);
            d.editor.select_type(idx.max(0) as usize);
            win.set_editor_error_message(SharedString::from(""));
            sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
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
        window.on_editor_save(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            pull_editor_text_fields(&win, &mut d.editor);
            match d.editor.build_action() {
                Ok(action_type) => {
                    log::debug!(
                        "Ação configurada ({}): {:?}",
                        action_type_title(&action_type),
                        d.editor.target_slot
                    );
                    if let Some(slot) = d.editor.target_slot {
                        let idx = d.selected_index;
                        if let Some(profile) = d.base_config.profiles.get_mut(idx) {
                            set_slot_action(
                                profile,
                                slot,
                                ActionConfig::from_action_type(action_type),
                            );
                        }
                        d.dirty = true;
                    }
                    d.editor.cancel();
                    win.set_editor_error_message(SharedString::from(""));
                    sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
                    sync_draft_to_ui(&win, &d);
                }
                Err(err) => win.set_editor_error_message(SharedString::from(err.to_string())),
            }
        });
    }

    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_editor_cancel(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            d.editor.cancel();
            win.set_editor_error_message(SharedString::from(""));
            sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
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
        ControlSlot::KnobPress => &profile.knob_press,
        ControlSlot::RotateCw => &profile.knob_cw,
        ControlSlot::RotateCcw => &profile.knob_ccw,
        ControlSlot::Key1 => &profile.key1,
        ControlSlot::Key2 => &profile.key2,
        ControlSlot::Key3 => &profile.key3,
    }
}

fn set_slot_action(profile: &mut ProfileConfig, slot: ControlSlot, action: ActionConfig) {
    match slot {
        ControlSlot::KnobPress => profile.knob_press = action,
        ControlSlot::RotateCw => profile.knob_cw = action,
        ControlSlot::RotateCcw => profile.knob_ccw = action,
        ControlSlot::Key1 => profile.key1 = action,
        ControlSlot::Key2 => profile.key2 = action,
        ControlSlot::Key3 => profile.key3 = action,
    }
}

fn next_unique_profile_name(profiles: &[ProfileConfig]) -> String {
    let mut counter = 1usize;
    loop {
        let candidate = if counter == 1 {
            "novo_perfil".to_string()
        } else {
            format!("novo_perfil_{counter}")
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

fn format_summary(action: &ActionConfig, apps: &[DesktopApp]) -> SharedString {
    SharedString::from(action_summary(&action.to_action_type(), apps))
}

fn sync_draft_to_ui(win: &SettingsWindow, draft: &SettingsDraft) {
    let names: Vec<SharedString> = draft
        .base_config
        .profiles
        .iter()
        .map(|p| SharedString::from(p.name.as_str()))
        .collect();
    win.set_profile_names(ModelRc::from(Rc::new(VecModel::from(names))));
    win.set_selected_index(draft.selected_index as i32);
    win.set_has_unsaved_changes(draft.dirty);

    if let Some(profile) = draft.base_config.profiles.get(draft.selected_index) {
        win.set_field_name(SharedString::from(profile.name.as_str()));
        win.set_field_window_class(SharedString::from(
            profile.window_class.as_deref().unwrap_or(""),
        ));
        win.set_summary_knob_press(format_summary(&profile.knob_press, &draft.desktop_apps));
        win.set_summary_rotate_cw(format_summary(&profile.knob_cw, &draft.desktop_apps));
        win.set_summary_rotate_ccw(format_summary(&profile.knob_ccw, &draft.desktop_apps));
        win.set_summary_key1(format_summary(&profile.key1, &draft.desktop_apps));
        win.set_summary_key2(format_summary(&profile.key2, &draft.desktop_apps));
        win.set_summary_key3(format_summary(&profile.key3, &draft.desktop_apps));
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

    let window_class_raw = win.get_field_window_class().trim().to_string();
    let window_class = if window_class_raw.is_empty() {
        None
    } else {
        Some(window_class_raw)
    };

    if profile.name != name || profile.window_class != window_class {
        profile.name = name;
        profile.window_class = window_class;
        draft.dirty = true;
    }

    Ok(())
}
