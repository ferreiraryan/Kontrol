/// Preset estático de atalho de teclado nomeado para um aplicativo conhecido.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortcutPreset {
    pub app: &'static str,
    pub name: &'static str,
    pub combo: &'static str,
}

const SHORTCUT_PRESETS: &[ShortcutPreset] = &[
    // Firefox
    ShortcutPreset {
        app: "Firefox",
        name: "Nova aba",
        combo: "Ctrl+T",
    },
    ShortcutPreset {
        app: "Firefox",
        name: "Reabrir última aba fechada",
        combo: "Ctrl+Shift+T",
    },
    ShortcutPreset {
        app: "Firefox",
        name: "Próxima aba",
        combo: "Ctrl+Tab",
    },
    ShortcutPreset {
        app: "Firefox",
        name: "Aba anterior",
        combo: "Ctrl+Shift+Tab",
    },
    ShortcutPreset {
        app: "Firefox",
        name: "Fechar aba",
        combo: "Ctrl+W",
    },
    ShortcutPreset {
        app: "Firefox",
        name: "Ir para endereço",
        combo: "Ctrl+L",
    },
    ShortcutPreset {
        app: "Firefox",
        name: "Atualizar página",
        combo: "F5",
    },
    // VSCode
    ShortcutPreset {
        app: "VSCode",
        name: "Paleta de comandos",
        combo: "Ctrl+Shift+P",
    },
    ShortcutPreset {
        app: "VSCode",
        name: "Terminal integrado",
        combo: "Ctrl+`",
    },
    ShortcutPreset {
        app: "VSCode",
        name: "Salvar",
        combo: "Ctrl+S",
    },
    ShortcutPreset {
        app: "VSCode",
        name: "Buscar em arquivos",
        combo: "Ctrl+Shift+F",
    },
    ShortcutPreset {
        app: "VSCode",
        name: "Ir para linha",
        combo: "Ctrl+G",
    },
    ShortcutPreset {
        app: "VSCode",
        name: "Comentar linha",
        combo: "Ctrl+/",
    },
    // Terminal
    ShortcutPreset {
        app: "Terminal",
        name: "Limpar tela",
        combo: "Ctrl+L",
    },
    ShortcutPreset {
        app: "Terminal",
        name: "Interromper processo",
        combo: "Ctrl+C",
    },
    ShortcutPreset {
        app: "Terminal",
        name: "Suspender processo",
        combo: "Ctrl+Z",
    },
    ShortcutPreset {
        app: "Terminal",
        name: "Buscar histórico",
        combo: "Ctrl+R",
    },
];

/// Retorna todos os presets estáticos embutidos.
pub fn presets() -> &'static [ShortcutPreset] {
    SHORTCUT_PRESETS
}

/// Retorna todos os presets de um aplicativo (comparação case-insensitive).
pub fn presets_for_app(app: &str) -> Vec<&'static ShortcutPreset> {
    let target = app.trim();
    presets()
        .iter()
        .filter(|p| p.app.eq_ignore_ascii_case(target))
        .collect()
}

/// Retorna a lista única de aplicativos disponíveis, preservando a ordem de declaração.
pub fn apps() -> Vec<&'static str> {
    let mut list = Vec::new();
    for preset in presets() {
        if !list.contains(&preset.app) {
            list.push(preset.app);
        }
    }
    list
}

/// Sugere o índice inicial do app na lista `apps()` com base no nome/classe do perfil atual.
pub fn suggested_app_index(profile_hint: &str) -> usize {
    let hint = profile_hint.trim().to_ascii_lowercase();
    if hint.is_empty() {
        return 0;
    }
    let app_list = apps();
    if let Some(pos) = app_list
        .iter()
        .position(|app| hint.contains(&app.to_ascii_lowercase()))
    {
        return pos;
    }
    if hint.contains("code") || hint.contains("codium") {
        if let Some(pos) = app_list.iter().position(|app| *app == "VSCode") {
            return pos;
        }
    }
    if hint.contains("kitty")
        || hint.contains("alacritty")
        || hint.contains("foot")
        || hint.contains("wezterm")
        || hint.contains("ghostty")
    {
        if let Some(pos) = app_list.iter().position(|app| *app == "Terminal") {
            return pos;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::{apps, presets, presets_for_app, suggested_app_index};
    use crate::actions_catalog::{parse_action_string, parse_shortcut_keys, ActionType};

    #[test]
    fn apps_returns_unique_list_in_order() {
        assert_eq!(apps(), vec!["Firefox", "VSCode", "Terminal"]);
    }

    #[test]
    fn presets_for_app_returns_expected_entries() {
        let firefox = presets_for_app("firefox");
        assert_eq!(firefox.len(), 7);
        assert!(firefox
            .iter()
            .any(|p| p.name == "Nova aba" && p.combo == "Ctrl+T"));

        let vscode = presets_for_app("VSCode");
        assert_eq!(vscode.len(), 6);
        assert!(vscode
            .iter()
            .any(|p| p.name == "Terminal integrado" && p.combo == "Ctrl+`"));
        assert!(vscode
            .iter()
            .any(|p| p.name == "Comentar linha" && p.combo == "Ctrl+/"));

        let terminal = presets_for_app("terminal");
        assert_eq!(terminal.len(), 4);
        assert!(terminal
            .iter()
            .any(|p| p.name == "Buscar histórico" && p.combo == "Ctrl+R"));

        assert!(presets_for_app("inexistente").is_empty());
    }

    #[test]
    fn all_presets_are_parseable_by_existing_action_parser() {
        assert_eq!(presets().len(), 17);
        for preset in presets() {
            let keys = parse_shortcut_keys(preset.combo).unwrap_or_else(|err| {
                panic!(
                    "combo inválido no preset '{} — {}' ({}): {err}",
                    preset.app, preset.name, preset.combo
                )
            });
            assert!(!keys.is_empty());

            let parsed = parse_action_string(preset.combo).unwrap_or_else(|err| {
                panic!(
                    "parse_action_string falhou no preset '{} — {}' ({}): {err}",
                    preset.app, preset.name, preset.combo
                )
            });
            assert!(
                matches!(parsed, ActionType::Shortcut(_)),
                "esperava ActionType::Shortcut para '{}'",
                preset.combo
            );
        }
    }

    #[test]
    fn suggests_matching_app_from_profile_hint() {
        assert_eq!(suggested_app_index("firefox"), 0);
        assert_eq!(suggested_app_index("vscode"), 1);
        assert_eq!(suggested_app_index("code-oss"), 1);
        assert_eq!(suggested_app_index("kitty"), 2);
        assert_eq!(suggested_app_index("global"), 0);
    }
}
