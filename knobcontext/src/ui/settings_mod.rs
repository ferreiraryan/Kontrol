use crate::action_presets::{
    apps as shortcut_preset_apps, presets_for_app as shortcut_presets_for_app, suggested_app_index,
};
use crate::actions_catalog::{
    action_summary, action_type_index, action_type_title, list_desktop_apps, ActionType, DesktopApp,
};
use crate::config::{
    builtin_hub_module_templates, builtin_profile_presets, default_overlay_items, ActionConfig,
    Config, CustomGroupEntry, OverlayItemConfig, OverlayItemKind, OverlayKey3Behavior,
    ProfileConfig, GLOBAL_PROFILE_NAME,
};
use crate::context::ActiveWindowInfo;
use crate::error::KnobError;
use crate::ui::action_editor_mod::{pick_file_uri, sync_editor_to_ui};
use crate::ui::action_editor_state::{ActionEditorState, ControlSlot};
use crate::ui::learn_mod::LearnController;
use crate::ui::{ControlSlotView, ProfileListItem, SettingsWindow};
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
    hub_subtab: usize,
    profile_subtab: usize,
    context_rules_expanded: bool,
    selected_index: usize,
    profile_hub_selected_index: usize,
    profile_hub_picker_open: bool,
    hub_selected_index: usize,
    hub_group_selected_entry: usize,
    dirty: bool,
    preset_picker_open: bool,
    inherits_picker_open: bool,
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
    learn: Rc<LearnController>,
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
            hub_subtab: 0,
            profile_subtab: 0,
            context_rules_expanded: true,
            selected_index: 0,
            profile_hub_selected_index: 0,
            profile_hub_picker_open: false,
            hub_selected_index: 0,
            hub_group_selected_entry: 0,
            dirty: false,
            preset_picker_open: false,
            inherits_picker_open: false,
            editor: ActionEditorState::default(),
            editor_target: None,
            desktop_apps: list_desktop_apps(),
        }));
        let is_open = Rc::new(RefCell::new(false));

        Ok(Self {
            window: RefCell::new(None),
            learn: Rc::new(LearnController::new()),
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

        let preset_labels: Vec<SharedString> = builtin_profile_presets()
            .iter()
            .map(|p| SharedString::from(p.label))
            .collect();
        window.set_preset_labels(ModelRc::from(Rc::new(VecModel::from(preset_labels))));

        let template_labels: Vec<SharedString> = builtin_hub_module_templates()
            .iter()
            .map(|t| SharedString::from(format!("{} — {}", t.title, t.kind.kind_label())))
            .collect();
        window.set_profile_hub_template_labels(ModelRc::from(Rc::new(VecModel::from(
            template_labels,
        ))));

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

        // Alternar sub-aba de Overlay / Hub (0 = Itens Padrão, 1 = Aparência e Comportamento)
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            window.on_select_hub_subtab(move |sub| {
                let Some(win) = weak.upgrade() else { return };
                let mut d = draft_ref.borrow_mut();
                if d.hub_subtab == 0 {
                    apply_hub_fields_to_draft(&win, &mut d);
                }
                d.hub_subtab = if sub == 1 { 1 } else { 0 };
                sync_all_to_ui(&win, &d);
            });
        }

        // Recolher / expandir card "Regras de ativação"
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            window.on_toggle_context_rules(move || {
                let Some(win) = weak.upgrade() else { return };
                let mut d = draft_ref.borrow_mut();
                let _ = apply_profile_header_to_draft(&win, &mut d);
                d.context_rules_expanded = !d.context_rules_expanded;
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
                d.profile_hub_selected_index = 0;
                d.profile_hub_picker_open = false;
                // Abre automaticamente as regras se o perfil contextual ainda não tiver window_class
                d.context_rules_expanded = d
                    .base_config
                    .profiles
                    .get(target)
                    .is_some_and(|p| p.window_class.is_none());
                win.set_error_message(SharedString::from(""));
                sync_all_to_ui(&win, &d);
            });
        }

        // Alternar sub-aba do perfil (0 = Ações do Knob, 1 = Módulos do Hub)
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            window.on_select_profile_subtab(move |sub| {
                let Some(win) = weak.upgrade() else { return };
                let mut d = draft_ref.borrow_mut();
                if let Err(err) = apply_profile_header_to_draft(&win, &mut d) {
                    win.set_error_message(SharedString::from(err.to_string()));
                    return;
                }
                d.profile_subtab = if sub == 1 { 1 } else { 0 };
                win.set_error_message(SharedString::from(""));
                sync_all_to_ui(&win, &d);
            });
        }

        // Customizar módulos do Hub para o perfil selecionado (materializa lista herdada)
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            window.on_profile_hub_customize(move || {
                let Some(win) = weak.upgrade() else { return };
                let mut d = draft_ref.borrow_mut();
                if materialize_profile_hub_modules(&mut d) {
                    d.dirty = true;
                }
                sync_all_to_ui(&win, &d);
            });
        }

        // Restaurar herança de módulos do Hub (`hub_modules = None`)
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            window.on_profile_hub_reset_inherit(move || {
                let Some(win) = weak.upgrade() else { return };
                let mut d = draft_ref.borrow_mut();
                let idx = d.selected_index;
                if let Some(profile) = d.base_config.profiles.get_mut(idx) {
                    if profile.hub_modules.is_some() {
                        profile.hub_modules = None;
                        d.profile_hub_selected_index = 0;
                        d.dirty = true;
                    }
                }
                sync_all_to_ui(&win, &d);
            });
        }

        // Selecionar módulo na lista do perfil
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            window.on_profile_hub_select_module(move |midx| {
                let Some(win) = weak.upgrade() else { return };
                let mut d = draft_ref.borrow_mut();
                apply_profile_hub_title_to_draft(&win, &mut d);
                d.profile_hub_selected_index = midx.max(0) as usize;
                sync_all_to_ui(&win, &d);
            });
        }

        // Reordenar módulo do Hub do perfil (↑/↓)
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            window.on_profile_hub_move_module(move |delta| {
                let Some(win) = weak.upgrade() else { return };
                let mut d = draft_ref.borrow_mut();
                apply_profile_hub_title_to_draft(&win, &mut d);
                materialize_profile_hub_modules(&mut d);
                let pidx = d.selected_index;
                let cur = d.profile_hub_selected_index;
                if let Some(mods) = d
                    .base_config
                    .profiles
                    .get_mut(pidx)
                    .and_then(|p| p.hub_modules.as_mut())
                {
                    let len = mods.len();
                    if cur < len {
                        let target = if delta < 0 {
                            cur.saturating_sub(1)
                        } else {
                            (cur + 1).min(len.saturating_sub(1))
                        };
                        if target != cur {
                            mods.swap(cur, target);
                            d.profile_hub_selected_index = target;
                            d.dirty = true;
                        }
                    }
                }
                sync_all_to_ui(&win, &d);
            });
        }

        // Remover módulo do Hub do perfil
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            window.on_profile_hub_remove_module(move || {
                let Some(win) = weak.upgrade() else { return };
                let mut d = draft_ref.borrow_mut();
                materialize_profile_hub_modules(&mut d);
                let pidx = d.selected_index;
                let cur = d.profile_hub_selected_index;
                if let Some(mods) = d
                    .base_config
                    .profiles
                    .get_mut(pidx)
                    .and_then(|p| p.hub_modules.as_mut())
                {
                    if cur < mods.len() {
                        mods.remove(cur);
                        d.profile_hub_selected_index = cur.min(mods.len().saturating_sub(1));
                        d.dirty = true;
                    }
                }
                sync_all_to_ui(&win, &d);
            });
        }

        // Abrir/fechar seletor "+ Adicionar módulo" do perfil
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            window.on_profile_hub_toggle_picker(move || {
                let Some(win) = weak.upgrade() else { return };
                let mut d = draft_ref.borrow_mut();
                d.profile_hub_picker_open = !d.profile_hub_picker_open;
                win.set_profile_hub_picker_open(d.profile_hub_picker_open);
            });
        }

        // Adicionar template de módulo escolhido ao perfil
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            window.on_profile_hub_add_template(move |tidx| {
                let Some(win) = weak.upgrade() else { return };
                let templates = builtin_hub_module_templates();
                let Some(mut item) = templates.get(tidx.max(0) as usize).cloned() else {
                    return;
                };
                let mut d = draft_ref.borrow_mut();
                apply_profile_hub_title_to_draft(&win, &mut d);
                materialize_profile_hub_modules(&mut d);
                let pidx = d.selected_index;
                if let Some(mods) = d
                    .base_config
                    .profiles
                    .get_mut(pidx)
                    .and_then(|p| p.hub_modules.as_mut())
                {
                    if mods.iter().any(|existing| existing.id == item.id) {
                        item.id = next_unique_hub_id(mods, &item.id);
                    }
                    mods.push(item);
                    d.profile_hub_selected_index = mods.len() - 1;
                    d.dirty = true;
                }
                d.profile_hub_picker_open = false;
                win.set_profile_hub_picker_open(false);
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
                    hub_modules: None,
                });
                d.selected_index = d.base_config.profiles.len() - 1;
                d.context_rules_expanded = true;
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

        // Abrir/fechar seletor de herança (`Herda de:`)
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            window.on_toggle_inherits_picker(move || {
                let Some(win) = weak.upgrade() else { return };
                let mut d = draft_ref.borrow_mut();
                if !d.inherits_picker_open {
                    if let Err(err) = apply_profile_header_to_draft(&win, &mut d) {
                        win.set_error_message(SharedString::from(err.to_string()));
                        return;
                    }
                }
                d.inherits_picker_open = !d.inherits_picker_open;
                win.set_inherits_picker_open(d.inherits_picker_open);
                if d.inherits_picker_open {
                    sync_all_to_ui(&win, &d);
                }
            });
        }

        // Selecionar opção no seletor de herança (com validação de ciclo imediata)
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            window.on_select_inherits_option(move |opt_idx| {
                let Some(win) = weak.upgrade() else { return };
                let mut d = draft_ref.borrow_mut();
                let sel = d.selected_index;
                match apply_inherits_selection(&mut d.base_config, sel, opt_idx.max(0) as usize) {
                    Ok(changed) => {
                        if changed {
                            d.dirty = true;
                        }
                        d.inherits_picker_open = false;
                        win.set_inherits_picker_open(false);
                        win.set_error_message(SharedString::from(""));
                        sync_all_to_ui(&win, &d);
                    }
                    Err(err) => {
                        d.inherits_picker_open = false;
                        win.set_inherits_picker_open(false);
                        win.set_error_message(SharedString::from(err.to_string()));
                    }
                }
            });
        }

        // Navegar clicando em um perfil na cadeia de herança (ex.: global → base → firefox)
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            window.on_navigate_inherits_chain(move |target_name| {
                let Some(win) = weak.upgrade() else { return };
                let mut d = draft_ref.borrow_mut();
                if let Err(err) = apply_profile_header_to_draft(&win, &mut d) {
                    win.set_error_message(SharedString::from(err.to_string()));
                    return;
                }
                if let Some(pos) = d
                    .base_config
                    .profiles
                    .iter()
                    .position(|p| p.name.eq_ignore_ascii_case(target_name.as_str()))
                {
                    d.selected_index = pos;
                    win.set_error_message(SharedString::from(""));
                    sync_all_to_ui(&win, &d);
                }
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
                let removed_name = d.base_config.profiles.remove(idx).name;
                // Limpa referências `inherits` que apontavam para o perfil removido
                for p in &mut d.base_config.profiles {
                    if p.inherits
                        .as_deref()
                        .is_some_and(|parent| parent.eq_ignore_ascii_case(&removed_name))
                    {
                        p.inherits = None;
                    }
                }
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
                let (profile_name, inherited_hint, current_action) = d
                    .base_config
                    .profiles
                    .get(d.selected_index)
                    .map(|p| {
                        let hint = p
                            .inherits
                            .as_deref()
                            .filter(|s| !s.trim().is_empty())
                            .map(|parent| {
                                format!(
                                    "Com 'Nenhuma ação', este controle herdará automaticamente do perfil '{parent}'."
                                )
                            })
                            .unwrap_or_default();
                        (p.name.clone(), hint, get_slot_action(p, slot).to_action_type())
                    })
                    .unwrap_or_else(|| (String::new(), String::new(), ActionType::None));
                d.editor_target = Some(EditorTarget::ProfileSlot(slot));
                d.editor.open(slot, &current_action);
                d.editor.preset_app_index = suggested_app_index(&profile_name);
                win.set_editor_profile_context_label(SharedString::from(profile_name));
                win.set_editor_inherited_hint(SharedString::from(inherited_hint));
                win.set_editor_error_message(SharedString::from(""));
                sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
            });
        }

        // Limpar rapidamente um slot de ação para None (revertendo para herança se houver pai)
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            window.on_clear_action_slot(move |slot_idx| {
                let Some(win) = weak.upgrade() else { return };
                let Some(slot) = ControlSlot::from_index(slot_idx.max(0) as usize) else {
                    return;
                };
                let mut d = draft_ref.borrow_mut();
                let pidx = d.selected_index;
                if let Some(profile) = d.base_config.profiles.get_mut(pidx) {
                    if *get_slot_action(profile, slot) != ActionConfig::None {
                        set_slot_action(profile, slot, ActionConfig::None);
                        d.dirty = true;
                    }
                }
                sync_all_to_ui(&win, &d);
            });
        }

        // Modo Aprender direto em um slot de ação do perfil
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            let learn_ref = Rc::clone(&self.learn);
            let release_cb = Rc::clone(&on_release_modifiers);
            window.on_learn_action_slot(move |slot_idx| {
                let Some(win) = weak.upgrade() else { return };
                let Some(slot) = ControlSlot::from_index(slot_idx.max(0) as usize) else {
                    return;
                };
                {
                    let mut d = draft_ref.borrow_mut();
                    if let Err(err) = apply_profile_header_to_draft(&win, &mut d) {
                        win.set_error_message(SharedString::from(err.to_string()));
                        return;
                    }
                }
                let weak_inner = win.as_weak();
                let draft_inner = Rc::clone(&draft_ref);
                let rel_ok = Rc::clone(&release_cb);
                let rel_cancel = Rc::clone(&release_cb);
                if let Err(err) = learn_ref.open(
                    slot.display_name(),
                    move |combo| {
                        let Some(w) = weak_inner.upgrade() else {
                            return;
                        };
                        match ActionConfig::parse_from_str(&combo) {
                            Ok(action) => {
                                let mut d = draft_inner.borrow_mut();
                                let pidx = d.selected_index;
                                if let Some(profile) = d.base_config.profiles.get_mut(pidx) {
                                    set_slot_action(profile, slot, action);
                                    d.dirty = true;
                                }
                                w.set_error_message(SharedString::from(""));
                                sync_all_to_ui(&w, &d);
                            }
                            Err(e) => {
                                w.set_error_message(SharedString::from(e.to_string()));
                            }
                        }
                        rel_ok();
                    },
                    move || {
                        rel_cancel();
                    },
                ) {
                    win.set_error_message(SharedString::from(format!(
                        "Falha ao abrir Modo Aprender: {err}"
                    )));
                }
            });
        }

        // Modo Aprender a partir do popover de Atalho de Teclado
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            let learn_ref = Rc::clone(&self.learn);
            let release_cb = Rc::clone(&on_release_modifiers);
            window.on_editor_open_learn_shortcut(move || {
                let Some(win) = weak.upgrade() else { return };
                let title = win.get_editor_control_title().to_string();
                let weak_inner = win.as_weak();
                let draft_inner = Rc::clone(&draft_ref);
                let rel_ok = Rc::clone(&release_cb);
                let rel_cancel = Rc::clone(&release_cb);
                if let Err(err) = learn_ref.open(
                    &title,
                    move |combo| {
                        let Some(w) = weak_inner.upgrade() else {
                            return;
                        };
                        let mut d = draft_inner.borrow_mut();
                        d.editor.set_manual_shortcut(combo);
                        d.editor.capturing_shortcut = false;
                        w.set_editor_error_message(SharedString::from(""));
                        sync_editor_to_ui(&w, &d.editor, &d.desktop_apps);
                        rel_ok();
                    },
                    move || {
                        rel_cancel();
                    },
                ) {
                    win.set_editor_error_message(SharedString::from(format!(
                        "Falha ao abrir Modo Aprender: {err}"
                    )));
                }
            });
        }

        // Modo Aprender a partir da aba Macro do popover
        {
            let weak = window.as_weak();
            let draft_ref = Rc::clone(&draft);
            let learn_ref = Rc::clone(&self.learn);
            let release_cb = Rc::clone(&on_release_modifiers);
            window.on_editor_open_learn_macro(move || {
                let Some(win) = weak.upgrade() else { return };
                let title = format!("Macro • {}", win.get_editor_control_title());
                let weak_inner = win.as_weak();
                let draft_inner = Rc::clone(&draft_ref);
                let rel_ok = Rc::clone(&release_cb);
                let rel_cancel = Rc::clone(&release_cb);
                if let Err(err) = learn_ref.open(
                    &title,
                    move |combo| {
                        let Some(w) = weak_inner.upgrade() else {
                            return;
                        };
                        let mut d = draft_inner.borrow_mut();
                        match d.editor.add_macro_key_step(&combo) {
                            Ok(()) => {
                                w.set_editor_error_message(SharedString::from(""));
                                sync_editor_to_ui(&w, &d.editor, &d.desktop_apps);
                            }
                            Err(e) => {
                                w.set_editor_error_message(SharedString::from(e.to_string()));
                            }
                        }
                        rel_ok();
                    },
                    move || {
                        rel_cancel();
                    },
                ) {
                    win.set_editor_error_message(SharedString::from(format!(
                        "Falha ao abrir Modo Aprender: {err}"
                    )));
                }
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
            let learn_ref = Rc::clone(&self.learn);
            window.on_cancel_requested(move || {
                learn_ref.close();
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
            let learn_ref = Rc::clone(&self.learn);
            window.window().on_close_requested(move || {
                learn_ref.close();
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
            d.hub_subtab = 0;
            d.selected_index = current_config.active_profile_index();
            d.context_rules_expanded = d
                .base_config
                .profiles
                .get(d.selected_index)
                .is_some_and(|p| p.window_class.is_none());
            d.hub_selected_index = 0;
            d.hub_group_selected_entry = 0;
            d.dirty = false;
            d.preset_picker_open = false;
            d.inherits_picker_open = false;
            d.editor.cancel();
            d.editor_target = None;
            sync_all_to_ui(&window, &d);
            sync_editor_to_ui(&window, &d.editor, &d.desktop_apps);
        }
        window.set_preset_picker_open(false);
        window.set_inherits_picker_open(false);
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
                win.set_editor_profile_context_label(SharedString::from(""));
                win.set_editor_inherited_hint(SharedString::from(""));
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
                win.set_editor_profile_context_label(SharedString::from(""));
                win.set_editor_inherited_hint(SharedString::from(""));
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
        window.on_editor_shortcut_text_edited(move |val| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            d.editor.set_manual_shortcut(val.to_string());
            sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
        });
    }

    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_editor_toggle_shortcut_modifier(move |modifier| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            d.editor.shortcut_text = win.get_editor_shortcut_text().to_string();
            d.editor.toggle_shortcut_modifier(modifier.as_str());
            win.set_editor_error_message(SharedString::from(""));
            sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
        });
    }

    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_editor_apply_shortcut_preset(move |preset| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            d.editor.set_manual_shortcut(preset.to_string());
            win.set_editor_error_message(SharedString::from(""));
            sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
        });
    }

    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_editor_toggle_preset_dropdown(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            d.editor.preset_dropdown_open = !d.editor.preset_dropdown_open;
            sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
        });
    }

    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_editor_select_preset_app(move |app_idx| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            let max_idx = shortcut_preset_apps().len().saturating_sub(1);
            d.editor.preset_app_index = (app_idx.max(0) as usize).min(max_idx);
            sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
        });
    }

    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_editor_select_preset_item(move |preset_idx| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            let apps = shortcut_preset_apps();
            if let Some(app_name) = apps.get(d.editor.preset_app_index) {
                let items = shortcut_presets_for_app(app_name);
                if let Some(preset) = items.get(preset_idx.max(0) as usize) {
                    d.editor
                        .apply_named_preset(preset.app, preset.name, preset.combo);
                    win.set_editor_error_message(SharedString::from(""));
                    sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
                }
            }
        });
    }

    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_editor_clear_shortcut(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            d.editor.shortcut_text.clear();
            d.editor.active_preset_label.clear();
            win.set_editor_error_message(SharedString::from(""));
            sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
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
        window.on_editor_set_scroll_amount(move |val| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            d.editor.scroll_amount = val.clamp(1, 10) as u8;
            sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
        });
    }

    {
        let weak = window.as_weak();
        let draft_ref = Rc::clone(&draft);
        window.on_editor_add_macro_wait_preset(move |ms| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            let raw = ms.max(1).to_string();
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
        window.on_editor_move_macro_step(move |idx, delta| {
            let Some(win) = weak.upgrade() else { return };
            let mut d = draft_ref.borrow_mut();
            d.editor
                .move_macro_step(idx.max(0) as usize, delta as isize);
            sync_editor_to_ui(&win, &d.editor, &d.desktop_apps);
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
    win.set_hub_subtab(draft.hub_subtab as i32);
    win.set_has_unsaved_changes(draft.dirty);
    sync_profile_tab_to_ui(win, draft);
    sync_hub_tab_to_ui(win, draft);
}

/// Resolve a visualização de um controle físico (ação própria vs. herdada pela cadeia `inherits`).
pub fn resolve_slot_view(
    profiles: &[ProfileConfig],
    profile_idx: usize,
    slot: ControlSlot,
    apps: &[DesktopApp],
) -> ControlSlotView {
    let Some(profile) = profiles.get(profile_idx) else {
        return ControlSlotView {
            summary: SharedString::from("Nenhuma"),
            category: 8,
            is_inherited: false,
            inherited_from: SharedString::from(""),
            can_clear: false,
        };
    };

    let own = get_slot_action(profile, slot);
    if *own != ActionConfig::None {
        let act_type = own.to_action_type();
        return ControlSlotView {
            summary: SharedString::from(action_summary(&act_type, apps)),
            category: action_type_index(&act_type) as i32,
            is_inherited: false,
            inherited_from: SharedString::from(""),
            can_clear: true,
        };
    }

    // Percorre cadeia `inherits` protegida contra ciclos para mostrar o valor efetivo herdado
    let mut visited = vec![profile.name.to_ascii_lowercase()];
    let mut parent_name = profile.inherits.as_deref();
    while let Some(pname) = parent_name {
        let trimmed = pname.trim();
        let key = trimmed.to_ascii_lowercase();
        if trimmed.is_empty() || visited.contains(&key) {
            break;
        }
        visited.push(key);
        let Some(parent) = profiles
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(trimmed))
        else {
            break;
        };
        let p_act = get_slot_action(parent, slot);
        if *p_act != ActionConfig::None {
            let act_type = p_act.to_action_type();
            return ControlSlotView {
                summary: SharedString::from(action_summary(&act_type, apps)),
                category: action_type_index(&act_type) as i32,
                is_inherited: true,
                inherited_from: SharedString::from(parent.name.as_str()),
                can_clear: false,
            };
        }
        parent_name = parent.inherits.as_deref();
    }

    ControlSlotView {
        summary: SharedString::from("Nenhuma"),
        category: 8,
        is_inherited: false,
        inherited_from: SharedString::from(""),
        can_clear: false,
    }
}

fn format_context_rules_summary(profile: &ProfileConfig) -> String {
    let mut parts = Vec::new();
    if let Some(ref wc) = profile.window_class {
        parts.push(wc.clone());
    } else {
        parts.push("Sem classe".to_string());
    }
    if let Some(ref wt) = profile.window_title {
        parts.push(format!("Título: {wt}"));
    }
    parts.push(format!("Prio {}", profile.priority));
    if profile.fullscreen == Some(true) {
        parts.push("Tela cheia".to_string());
    }
    parts.join(" • ")
}

const INHERITS_NONE_LABEL: &str = "Nenhum";

/// Retorna as opções do seletor `Herda de:`: `"Nenhum"` na posição 0, seguido pelos demais
/// perfis configurados (excluindo o próprio perfil selecionado para evitar auto-herança).
pub fn inherits_options_for_profile(
    profiles: &[ProfileConfig],
    selected_idx: usize,
) -> Vec<String> {
    let mut options = Vec::with_capacity(profiles.len());
    options.push(INHERITS_NONE_LABEL.to_string());
    for (idx, profile) in profiles.iter().enumerate() {
        if idx != selected_idx {
            options.push(profile.name.clone());
        }
    }
    options
}

/// Constrói a cadeia de herança em ordem ancestral -> perfil atual (ex.: `["global", "base", "firefox"]`).
/// Protegida contra ciclos.
pub fn compute_inheritance_chain(profiles: &[ProfileConfig], selected_idx: usize) -> Vec<String> {
    let Some(start) = profiles.get(selected_idx) else {
        return Vec::new();
    };
    let mut chain_rev = Vec::new();
    let mut visited = Vec::new();
    let mut current: Option<&ProfileConfig> = Some(start);

    while let Some(prof) = current {
        let key = prof.name.to_ascii_lowercase();
        if visited.contains(&key) {
            break;
        }
        visited.push(key);
        chain_rev.push(prof.name.clone());
        current = prof.inherits.as_deref().and_then(|parent_name| {
            profiles
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case(parent_name.trim()))
        });
    }

    chain_rev.reverse();
    chain_rev
}

/// Aplica a escolha do seletor `Herda de:` validando contra ciclos via `Config::validate()`.
/// Em caso de ciclo, restaura o valor anterior e retorna erro claro sem corromper a configuração.
pub fn apply_inherits_selection(
    config: &mut Config,
    selected_idx: usize,
    option_idx: usize,
) -> Result<bool, KnobError> {
    let options = inherits_options_for_profile(&config.profiles, selected_idx);
    let Some(chosen) = options.get(option_idx) else {
        return Ok(false);
    };
    let new_inherits = if option_idx == 0 || chosen == INHERITS_NONE_LABEL {
        None
    } else {
        Some(chosen.clone())
    };

    let Some(profile) = config.profiles.get_mut(selected_idx) else {
        return Ok(false);
    };
    if profile.inherits == new_inherits {
        return Ok(false);
    }

    let previous = profile.inherits.clone();
    profile.inherits = new_inherits;
    if let Err(err) = config.validate() {
        if let Some(p) = config.profiles.get_mut(selected_idx) {
            p.inherits = previous;
        }
        return Err(err);
    }
    Ok(true)
}

fn sync_profile_tab_to_ui(win: &SettingsWindow, draft: &SettingsDraft) {
    let items: Vec<ProfileListItem> = draft
        .base_config
        .profiles
        .iter()
        .map(|p| {
            let is_global = p.name.eq_ignore_ascii_case(GLOBAL_PROFILE_NAME);
            let subtitle = if is_global {
                String::new()
            } else {
                p.window_class
                    .clone()
                    .unwrap_or_else(|| "sem janela".to_string())
            };
            ProfileListItem {
                name: SharedString::from(p.name.as_str()),
                subtitle: SharedString::from(subtitle),
                is_global,
                has_custom_hub: p.hub_modules.is_some(),
            }
        })
        .collect();
    win.set_profile_items(ModelRc::from(Rc::new(VecModel::from(items))));
    win.set_selected_index(draft.selected_index as i32);

    let options: Vec<SharedString> =
        inherits_options_for_profile(&draft.base_config.profiles, draft.selected_index)
            .into_iter()
            .map(SharedString::from)
            .collect();
    win.set_inherits_options(ModelRc::from(Rc::new(VecModel::from(options))));

    let chain: Vec<SharedString> =
        compute_inheritance_chain(&draft.base_config.profiles, draft.selected_index)
            .into_iter()
            .map(SharedString::from)
            .collect();
    win.set_inherits_chain(ModelRc::from(Rc::new(VecModel::from(chain))));

    if let Some(profile) = draft.base_config.profiles.get(draft.selected_index) {
        let is_global = profile.name.eq_ignore_ascii_case(GLOBAL_PROFILE_NAME);
        win.set_selected_is_global(is_global);
        win.set_context_rules_expanded(draft.context_rules_expanded);
        win.set_context_rules_summary(SharedString::from(format_context_rules_summary(profile)));

        win.set_field_name(SharedString::from(profile.name.as_str()));
        win.set_field_window_class(SharedString::from(
            profile.window_class.as_deref().unwrap_or(""),
        ));
        win.set_field_window_title(SharedString::from(
            profile.window_title.as_deref().unwrap_or(""),
        ));
        win.set_field_inherits_label(SharedString::from(
            profile
                .inherits
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(INHERITS_NONE_LABEL),
        ));
        win.set_field_priority(profile.priority);
        win.set_field_fullscreen_only(profile.fullscreen == Some(true));

        let profiles = &draft.base_config.profiles;
        let idx = draft.selected_index;
        win.set_slot_rotate_cw(resolve_slot_view(
            profiles,
            idx,
            ControlSlot::RotateCw,
            &draft.desktop_apps,
        ));
        win.set_slot_rotate_ccw(resolve_slot_view(
            profiles,
            idx,
            ControlSlot::RotateCcw,
            &draft.desktop_apps,
        ));
        win.set_slot_key1(resolve_slot_view(
            profiles,
            idx,
            ControlSlot::Key1,
            &draft.desktop_apps,
        ));
        win.set_slot_key2(resolve_slot_view(
            profiles,
            idx,
            ControlSlot::Key2,
            &draft.desktop_apps,
        ));
        win.set_slot_key3(resolve_slot_view(
            profiles,
            idx,
            ControlSlot::Key3,
            &draft.desktop_apps,
        ));

        // Módulos do Hub deste perfil (próprios ou herdados)
        win.set_profile_subtab(draft.profile_subtab as i32);
        win.set_profile_hub_picker_open(draft.profile_hub_picker_open);
        let (effective_mods, inherited_from) = draft.base_config.resolve_hub_modules(&profile.name);
        let is_inherited = profile.hub_modules.is_none();
        win.set_profile_hub_is_inherited(is_inherited);
        let subtab_label = if is_inherited {
            "Módulos do Hub • herdado".to_string()
        } else {
            format!("Módulos do Hub ({})", effective_mods.len())
        };
        win.set_profile_hub_subtab_label(SharedString::from(subtab_label));
        let status_text = if is_inherited {
            let parent = inherited_from.as_deref().unwrap_or(GLOBAL_PROFILE_NAME);
            format!("Herdado de {parent} — clique para customizar")
        } else {
            format!("Definido neste perfil ({} módulos)", effective_mods.len())
        };
        win.set_profile_hub_status_text(SharedString::from(status_text));

        let mod_labels: Vec<SharedString> = effective_mods
            .iter()
            .map(|m| SharedString::from(format!("{}  ({})", m.title, m.kind.kind_label())))
            .collect();
        win.set_profile_hub_module_labels(ModelRc::from(Rc::new(VecModel::from(mod_labels))));
        let clamped_mod_idx = draft
            .profile_hub_selected_index
            .min(effective_mods.len().saturating_sub(1));
        win.set_profile_hub_selected_index(clamped_mod_idx as i32);
        if let Some(sel) = effective_mods.get(clamped_mod_idx) {
            win.set_profile_hub_selected_title(SharedString::from(sel.title.as_str()));
        } else {
            win.set_profile_hub_selected_title(SharedString::from(""));
        }
    }
}

fn materialize_profile_hub_modules(draft: &mut SettingsDraft) -> bool {
    let idx = draft.selected_index;
    let Some(profile_name) = draft.base_config.profiles.get(idx).map(|p| p.name.clone()) else {
        return false;
    };
    if draft
        .base_config
        .profiles
        .get(idx)
        .is_some_and(|p| p.hub_modules.is_some())
    {
        return false;
    }
    let (resolved, _) = draft.base_config.resolve_hub_modules(&profile_name);
    if let Some(profile) = draft.base_config.profiles.get_mut(idx) {
        profile.hub_modules = Some(resolved);
        return true;
    }
    false
}

fn apply_profile_hub_title_to_draft(win: &SettingsWindow, draft: &mut SettingsDraft) {
    let pidx = draft.selected_index;
    let midx = draft.profile_hub_selected_index;
    let new_title = win.get_profile_hub_selected_title().trim().to_string();
    if new_title.is_empty() {
        return;
    }
    if let Some(item) = draft
        .base_config
        .profiles
        .get_mut(pidx)
        .and_then(|p| p.hub_modules.as_mut())
        .and_then(|mods| mods.get_mut(midx))
    {
        if item.title != new_title {
            item.title = new_title;
            draft.dirty = true;
        }
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
        .map(|i| SharedString::from(i.title.as_str()))
        .collect();
    let enabled_flags: Vec<bool> = overlay.items.iter().map(|i| i.enabled).collect();
    win.set_hub_item_labels(ModelRc::from(Rc::new(VecModel::from(labels))));
    win.set_hub_item_enabled_flags(ModelRc::from(Rc::new(VecModel::from(enabled_flags))));
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
    let idx = draft.selected_index;
    let Some(old_name) = draft.base_config.profiles.get(idx).map(|p| p.name.clone()) else {
        return Ok(());
    };

    // O perfil `global` oculta campos de regra contextual na UI; apenas aplica título de módulo do Hub
    if old_name.eq_ignore_ascii_case(GLOBAL_PROFILE_NAME) {
        apply_profile_hub_title_to_draft(win, draft);
        return Ok(());
    }

    let name = win.get_field_name().trim().to_string();
    if name.is_empty() {
        return Err(KnobError::ConfigValidation(
            "nome do perfil não pode ficar vazio".to_string(),
        ));
    }

    let window_class = normalize_optional_field(win.get_field_window_class().as_str());
    let window_title = normalize_optional_field(win.get_field_window_title().as_str());

    let mut renamed = false;
    if let Some(profile) = draft.base_config.profiles.get_mut(idx) {
        if profile.name != name
            || profile.window_class != window_class
            || profile.window_title != window_title
        {
            renamed = profile.name != name;
            profile.name = name.clone();
            profile.window_class = window_class;
            profile.window_title = window_title;
            draft.dirty = true;
        }
    }

    // Atualiza referências `inherits` caso o perfil tenha sido renomeado.
    if renamed {
        for p in &mut draft.base_config.profiles {
            if p.inherits
                .as_deref()
                .is_some_and(|parent| parent.eq_ignore_ascii_case(&old_name))
            {
                p.inherits = Some(name.clone());
            }
        }
    }

    apply_profile_hub_title_to_draft(win, draft);
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

#[cfg(test)]
mod tests {
    use super::{
        apply_inherits_selection, compute_inheritance_chain, inherits_options_for_profile,
        resolve_slot_view, INHERITS_NONE_LABEL,
    };
    use crate::config::{ActionConfig, Config, ProfileConfig};
    use crate::error::KnobError;
    use crate::ui::action_editor_state::ControlSlot;

    #[test]
    fn resolve_slot_view_displays_own_inherited_and_empty_actions() {
        let mut cfg = Config::default();
        // global tem Key3 = MediaPlayPause
        cfg.profiles[0].key3 = ActionConfig::MediaPlayPause;
        // firefox herda de global e deixa Key3 = None
        cfg.profiles[1].inherits = Some("global".to_string());
        cfg.profiles[1].key3 = ActionConfig::None;
        cfg.profiles[1].key1 = ActionConfig::Shortcut {
            keys: vec!["Ctrl".into(), "T".into()],
        };

        // Ação própria em firefox (Key1)
        let own = resolve_slot_view(&cfg.profiles, 1, ControlSlot::Key1, &[]);
        assert_eq!(own.summary.as_str(), "Ctrl+T");
        assert!(!own.is_inherited);
        assert!(own.can_clear);
        assert_eq!(own.category, 0);

        // Ação herdada de global em firefox (Key3)
        let inherited = resolve_slot_view(&cfg.profiles, 1, ControlSlot::Key3, &[]);
        assert_eq!(inherited.summary.as_str(), "Play / Pause");
        assert!(inherited.is_inherited);
        assert_eq!(inherited.inherited_from.as_str(), "global");
        assert!(!inherited.can_clear);
        assert_eq!(inherited.category, 1);

        // Sem herança e sem ação -> Vazio
        cfg.profiles[0].key2 = ActionConfig::None;
        let empty = resolve_slot_view(&cfg.profiles, 0, ControlSlot::Key2, &[]);
        assert_eq!(empty.summary.as_str(), "Nenhuma");
        assert!(!empty.is_inherited);
        assert!(!empty.can_clear);
        assert_eq!(empty.category, 8);
    }

    #[test]
    fn inherits_editor_validates_chain_rejects_cycles_and_clears_on_none() {
        let mut cfg = Config::default();
        // Adiciona perfil intermediário "base" herdando de "global"
        cfg.profiles.push(ProfileConfig {
            name: "base".to_string(),
            window_class: None,
            window_title: None,
            fullscreen: None,
            priority: 0,
            inherits: Some("global".to_string()),
            knob_cw: ActionConfig::VolumeUp,
            knob_ccw: ActionConfig::VolumeDown,
            knob_press: None,
            key1: ActionConfig::None,
            key2: ActionConfig::None,
            key3: ActionConfig::None,
            hub_modules: None,
        });

        // Opções para "firefox" (índice 1): exclui o próprio "firefox" e inicia com "Nenhum"
        let opts = inherits_options_for_profile(&cfg.profiles, 1);
        assert_eq!(opts[0], INHERITS_NONE_LABEL);
        assert!(!opts.iter().any(|o| o == "firefox"));
        assert!(opts.iter().any(|o| o == "base"));

        // Seleciona "base" como pai de "firefox" -> cadeia global -> base -> firefox
        let base_opt_idx = opts.iter().position(|o| o == "base").expect("opção base");
        assert_eq!(
            apply_inherits_selection(&mut cfg, 1, base_opt_idx).ok(),
            Some(true)
        );
        assert_eq!(cfg.profiles[1].inherits.as_deref(), Some("base"));
        assert_eq!(
            compute_inheritance_chain(&cfg.profiles, 1),
            vec!["global", "base", "firefox"]
        );

        // Tentar fazer "base" (índice 3) herdar de "firefox" criaria ciclo -> deve rejeitar sem alterar!
        let base_opts = inherits_options_for_profile(&cfg.profiles, 3);
        let firefox_opt_idx = base_opts
            .iter()
            .position(|o| o == "firefox")
            .expect("opção firefox");
        let err = apply_inherits_selection(&mut cfg, 3, firefox_opt_idx)
            .expect_err("ciclo deve ser rejeitado");
        assert!(matches!(err, KnobError::ConfigValidation(_)));
        assert_eq!(cfg.profiles[3].inherits.as_deref(), Some("global"));

        // Selecionar "Nenhum" (índice 0) remove herança de "firefox"
        assert_eq!(apply_inherits_selection(&mut cfg, 1, 0).ok(), Some(true));
        assert_eq!(cfg.profiles[1].inherits, None);
        assert_eq!(compute_inheritance_chain(&cfg.profiles, 1), vec!["firefox"]);
    }
}
