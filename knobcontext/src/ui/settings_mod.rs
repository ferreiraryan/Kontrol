use crate::config::{ActionConfig, Config, ProfileConfig, GLOBAL_PROFILE_NAME};
use crate::error::KnobError;
use crate::ui::SettingsWindow;
use slint::{CloseRequestResponse, ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

struct SettingsDraft {
    base_config: Config,
    selected_index: usize,
}

/// Controlador da janela de configurações Slint (edição de perfis e persistência TOML).
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
        let draft = Rc::new(RefCell::new(SettingsDraft {
            base_config: initial_config,
            selected_index: 0,
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
                if let Err(err) = apply_fields_to_draft(&win, &mut d) {
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
                if let Err(err) = apply_fields_to_draft(&win, &mut d) {
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
                win.set_error_message(SharedString::from(""));
                sync_draft_to_ui(&win, &d);
            });
        }

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
                if let Err(err) = apply_fields_to_draft(&win, &mut d) {
                    win.set_error_message(SharedString::from(err.to_string()));
                    return;
                }

                // Garante que active_profile aponte para um perfil válido.
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
                        let saved = d.base_config.clone();
                        drop(d);
                        win.set_error_message(SharedString::from(""));
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
                    let _ = win.hide();
                }
                on_closed_cb();
            });
        }

        // Fechamento via botão X do compositor (oculta sem encerrar o daemon)
        {
            let weak = window.as_weak();
            let open_ref = Rc::clone(&is_open);
            let on_closed_cb = Rc::clone(&on_closed);
            window.window().on_close_requested(move || {
                *open_ref.borrow_mut() = false;
                if let Some(win) = weak.upgrade() {
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
            sync_draft_to_ui(&self.window, &d);
        }
        self.window.set_error_message(SharedString::from(""));
        *self.is_open.borrow_mut() = true;
        self.window
            .show()
            .map_err(|err| KnobError::UiRun(err.to_string()))
    }

    pub fn hide(&self) -> Result<(), KnobError> {
        *self.is_open.borrow_mut() = false;
        self.window
            .hide()
            .map_err(|err| KnobError::UiRun(err.to_string()))
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

fn sync_draft_to_ui(win: &SettingsWindow, draft: &SettingsDraft) {
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
        win.set_field_knob_press(SharedString::from(profile.knob_press.to_display_string()));
        win.set_field_rotate_cw(SharedString::from(profile.knob_cw.to_display_string()));
        win.set_field_rotate_ccw(SharedString::from(profile.knob_ccw.to_display_string()));
        win.set_field_key1(SharedString::from(profile.key1.to_display_string()));
        win.set_field_key2(SharedString::from(profile.key2.to_display_string()));
        win.set_field_key3(SharedString::from(profile.key3.to_display_string()));
    }
}

fn apply_fields_to_draft(win: &SettingsWindow, draft: &mut SettingsDraft) -> Result<(), KnobError> {
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

    let knob_press = ActionConfig::parse_from_str(win.get_field_knob_press().as_str())?;
    let knob_cw = ActionConfig::parse_from_str(win.get_field_rotate_cw().as_str())?;
    let knob_ccw = ActionConfig::parse_from_str(win.get_field_rotate_ccw().as_str())?;
    let key1 = ActionConfig::parse_from_str(win.get_field_key1().as_str())?;
    let key2 = ActionConfig::parse_from_str(win.get_field_key2().as_str())?;
    let key3 = ActionConfig::parse_from_str(win.get_field_key3().as_str())?;

    profile.name = name;
    profile.window_class = window_class;
    profile.knob_press = knob_press;
    profile.knob_cw = knob_cw;
    profile.knob_ccw = knob_ccw;
    profile.key1 = key1;
    profile.key2 = key2;
    profile.key3 = key3;

    Ok(())
}
