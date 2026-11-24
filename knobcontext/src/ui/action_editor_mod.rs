use crate::action_presets::{apps as preset_apps, presets_for_app};
use crate::actions_catalog::{DesktopApp, MacroStep};
use crate::ui::action_editor_state::ActionEditorState;
use crate::ui::{AppListItem, MacroStepItem, PresetListItem, SettingsWindow};
use slint::{ModelRc, SharedString, VecModel};
use std::rc::Rc;

/// Identifica se o `text` enviado pelo Slint corresponde a uma tecla modificadora pura
/// (`Shift`, `ShiftR`, `Control`, `ControlR`, `Alt`, `AltGr`, `Meta`, `MetaR`, `CapsLock`).
pub fn slint_modifier_key_name(text: &str) -> Option<&'static str> {
    let mut chars = text.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    match ch {
        '\u{0010}' | '\u{0015}' => Some("Shift"),
        '\u{0011}' | '\u{0016}' => Some("Ctrl"),
        '\u{0012}' | '\u{0013}' => Some("Alt"),
        '\u{0017}' | '\u{0018}' => Some("Super"),
        '\u{0014}' => Some("CapsLock"),
        _ => None,
    }
}

/// Converte um evento de tecla capturado no Slint (`text` + modificadores) em string do parser
/// (ex.: `"Ctrl+Shift+Tab"`, `"Super+E"`, `"Alt+F4"`). Retorna `None` para teclas modificadoras puras.
pub fn format_captured_key(
    text: &str,
    ctrl: bool,
    alt: bool,
    shift: bool,
    meta: bool,
) -> Option<String> {
    let (key_name, implies_shift) = map_slint_key_text(text, ctrl)?;
    let mut parts = Vec::new();
    if ctrl {
        parts.push("Ctrl".to_string());
    }
    if alt {
        parts.push("Alt".to_string());
    }
    if shift || implies_shift {
        parts.push("Shift".to_string());
    }
    if meta {
        parts.push("Super".to_string());
    }
    parts.push(key_name);
    Some(parts.join("+"))
}

fn map_slint_key_text(text: &str, ctrl_held: bool) -> Option<(String, bool)> {
    let mut chars = text.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }

    // Mapeamento dos códigos usados pelo Slint (`i_slint_common::key_codes`)
    let mapped = match ch {
        '\u{0008}' => ("Backspace".to_string(), false),
        '\u{0009}' => ("Tab".to_string(), false),
        '\u{0019}' => ("Tab".to_string(), true), // Backtab (Shift+Tab)
        '\u{000a}' | '\u{000d}' => ("Return".to_string(), false),
        '\u{001b}' => ("Escape".to_string(), false),
        '\u{0020}' => ("Space".to_string(), false),
        '\u{007f}' => ("Delete".to_string(), false),
        '\u{f700}' => ("Up".to_string(), false),
        '\u{f701}' => ("Down".to_string(), false),
        '\u{f702}' => ("Left".to_string(), false),
        '\u{f703}' => ("Right".to_string(), false),
        '\u{f704}' => ("F1".to_string(), false),
        '\u{f705}' => ("F2".to_string(), false),
        '\u{f706}' => ("F3".to_string(), false),
        '\u{f707}' => ("F4".to_string(), false),
        '\u{f708}' => ("F5".to_string(), false),
        '\u{f709}' => ("F6".to_string(), false),
        '\u{f70a}' => ("F7".to_string(), false),
        '\u{f70b}' => ("F8".to_string(), false),
        '\u{f70c}' => ("F9".to_string(), false),
        '\u{f70d}' => ("F10".to_string(), false),
        '\u{f70e}' => ("F11".to_string(), false),
        '\u{f70f}' => ("F12".to_string(), false),
        '\u{f729}' => ("Home".to_string(), false),
        '\u{f72b}' => ("End".to_string(), false),
        '\u{f72c}' => ("PageUp".to_string(), false),
        '\u{f72d}' => ("PageDown".to_string(), false),
        // Ignora teclas modificadoras isoladas enviadas pelo Slint
        // (Shift/Control/Alt/AltGr/CapsLock/ShiftR/ControlR/Meta/MetaR)
        '\u{0010}'..='\u{0018}' => return None,
        // Fallback para caracteres de controle ASCII (Ctrl+A..Ctrl+Z) caso o backend envie
        c @ '\u{0001}'..='\u{001a}' if ctrl_held => {
            let letter = ((c as u8 - 1) + b'A') as char;
            (letter.to_string(), false)
        }
        c if c.is_ascii_graphic() => (c.to_ascii_uppercase().to_string(), false),
        _ => return None,
    };

    Some(mapped)
}

/// Sincroniza todo o estado do `ActionEditorState` para as propriedades do popover na `SettingsWindow`.
pub fn sync_editor_to_ui(win: &SettingsWindow, editor: &ActionEditorState, apps: &[DesktopApp]) {
    win.set_editor_open(editor.is_open);
    if let Some(ref title) = editor.custom_title {
        win.set_editor_control_title(SharedString::from(title.as_str()));
    } else if let Some(slot) = editor.target_slot {
        win.set_editor_control_title(SharedString::from(slot.display_name()));
    }
    win.set_editor_type_index(editor.selected_type_index as i32);
    win.set_editor_shortcut_text(SharedString::from(editor.shortcut_text.as_str()));

    let keycap_parts: Vec<SharedString> = editor
        .shortcut_keycap_parts()
        .into_iter()
        .map(SharedString::from)
        .collect();
    win.set_editor_shortcut_parts(ModelRc::from(Rc::new(VecModel::from(keycap_parts))));
    win.set_editor_active_preset_label(SharedString::from(editor.active_preset_label.as_str()));
    win.set_editor_preset_dropdown_open(editor.preset_dropdown_open);

    let apps_list = preset_apps();
    let app_idx = editor
        .preset_app_index
        .min(apps_list.len().saturating_sub(1));
    let app_labels: Vec<SharedString> = apps_list.iter().map(|a| SharedString::from(*a)).collect();
    win.set_editor_preset_apps(ModelRc::from(Rc::new(VecModel::from(app_labels))));
    win.set_editor_preset_app_index(app_idx as i32);

    let preset_items: Vec<PresetListItem> = apps_list
        .get(app_idx)
        .map(|app_name| {
            presets_for_app(app_name)
                .into_iter()
                .map(|p| PresetListItem {
                    name: SharedString::from(p.name),
                    combo: SharedString::from(p.combo),
                })
                .collect()
        })
        .unwrap_or_default();
    win.set_editor_preset_items(ModelRc::from(Rc::new(VecModel::from(preset_items))));

    win.set_editor_media_index(editor.media_index as i32);
    win.set_editor_app_search(SharedString::from(editor.app_search.as_str()));

    let filtered = editor.filtered_apps(apps);
    let app_items: Vec<AppListItem> = filtered
        .iter()
        .map(|app| AppListItem {
            name: SharedString::from(app.name.as_str()),
            desktop_file: SharedString::from(app.desktop_file.as_str()),
        })
        .collect();
    let selected_app_idx = filtered
        .iter()
        .position(|app| app.desktop_file == editor.selected_desktop_file)
        .map(|i| i as i32)
        .unwrap_or(-1);
    let selected_app_label = if editor.selected_desktop_file.trim().is_empty() {
        String::new()
    } else {
        apps.iter()
            .find(|a| a.desktop_file == editor.selected_desktop_file)
            .map(|a| format!("{} ({})", a.name, a.desktop_file))
            .unwrap_or_else(|| editor.selected_desktop_file.clone())
    };
    win.set_editor_filtered_apps(ModelRc::from(Rc::new(VecModel::from(app_items))));
    win.set_editor_selected_app_index(selected_app_idx);
    win.set_editor_selected_app_label(SharedString::from(selected_app_label));

    win.set_editor_url_text(SharedString::from(editor.url_text.as_str()));
    win.set_editor_file_text(SharedString::from(editor.file_uri.as_str()));
    win.set_editor_exec_text(SharedString::from(editor.exec_command.as_str()));
    win.set_editor_scroll_direction(editor.scroll_direction_index as i32);
    win.set_editor_scroll_amount(i32::from(editor.scroll_amount));

    let step_items: Vec<MacroStepItem> = editor
        .macro_steps
        .iter()
        .map(|step| match step {
            MacroStep::Keys(keys) => MacroStepItem {
                label: SharedString::from(keys.join("+")),
                is_wait: false,
            },
            MacroStep::WaitMs(ms) => MacroStepItem {
                label: SharedString::from(format!("{ms} ms")),
                is_wait: true,
            },
        })
        .collect();
    win.set_editor_macro_steps(ModelRc::from(Rc::new(VecModel::from(step_items))));
    win.set_editor_macro_key_text(SharedString::from(editor.macro_key_input.as_str()));
    win.set_editor_macro_wait_text(SharedString::from(editor.macro_wait_ms_input.as_str()));
    win.set_editor_preview_text(SharedString::from(editor.live_preview_summary(apps)));
}

/// Abre o diálogo nativo de seleção de arquivo (Wayland via XDG Portal usando `rfd`).
pub fn pick_file_uri() -> Option<String> {
    let path = rfd::FileDialog::new().pick_file()?;
    Some(format!("file://{}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::format_captured_key;

    #[test]
    fn formats_captured_shortcuts_with_modifiers() {
        assert_eq!(
            format_captured_key("\u{0009}", true, false, false, false),
            Some("Ctrl+Tab".to_string())
        );
        assert_eq!(
            format_captured_key("e", false, false, false, true),
            Some("Super+E".to_string())
        );
        assert_eq!(
            format_captured_key("\u{f707}", false, true, false, false),
            Some("Alt+F4".to_string())
        );
        assert_eq!(
            format_captured_key("c", true, false, true, false),
            Some("Ctrl+Shift+C".to_string())
        );
    }

    #[test]
    fn ignores_pure_modifier_codes() {
        assert_eq!(
            format_captured_key("\u{0011}", true, false, false, false),
            None
        );
    }
}
