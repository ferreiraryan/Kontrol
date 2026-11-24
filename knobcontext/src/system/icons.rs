use crate::actions_catalog::DesktopApp;
use std::env;
use std::path::{Path, PathBuf};

/// Diretórios padrão de temas de ícones no Linux (prioridade para PNG de boa resolução e SVG).
pub fn default_icon_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(home) = env::var("HOME") {
        let h = PathBuf::from(home);
        dirs.push(h.join(".local/share/icons/hicolor/128x128/apps"));
        dirs.push(h.join(".local/share/icons/hicolor/64x64/apps"));
        dirs.push(h.join(".local/share/icons/hicolor/scalable/apps"));
        dirs.push(h.join(".local/share/icons"));
    }
    for base in [
        "/usr/share/icons/hicolor/128x128/apps",
        "/usr/share/icons/hicolor/64x64/apps",
        "/usr/share/icons/hicolor/48x48/apps",
        "/usr/share/icons/hicolor/256x256/apps",
        "/usr/share/icons/Papirus/64x64/apps",
        "/usr/share/icons/Papirus-Dark/64x64/apps",
        "/usr/share/icons/Adwaita/scalable/apps",
        "/usr/share/icons/hicolor/scalable/apps",
        "/usr/share/pixmaps",
    ] {
        dirs.push(PathBuf::from(base));
    }
    dirs
}

/// Resolve um ícone de aplicativo (nome, `.desktop`, `WINDOWCLASS` ou caminho direto) para um arquivo real.
/// Retorna `None` silenciosamente quando não encontrado (imagens são 100% opcionais).
pub fn resolve_app_icon_path(hint: &str, apps: &[DesktopApp]) -> Option<PathBuf> {
    resolve_app_icon_in_dirs(hint, apps, &default_icon_search_dirs())
}

pub fn resolve_app_icon_in_dirs(
    hint: &str,
    apps: &[DesktopApp],
    icon_dirs: &[PathBuf],
) -> Option<PathBuf> {
    let trimmed = hint.trim();
    if trimmed.is_empty() {
        return None;
    }

    // 1. Caminho direto para imagem
    let direct = Path::new(trimmed);
    if direct.is_absolute() && direct.is_file() {
        return Some(direct.to_path_buf());
    }

    // 2. Tenta encontrar entrada `.desktop` correspondente ao nome/arquivo/classe
    let lower = trimmed.to_ascii_lowercase();
    let stem_hint = lower.trim_end_matches(".desktop");
    let mut icon_candidates: Vec<String> = Vec::new();

    if let Some(app) = apps.iter().find(|a| {
        let df = a.desktop_file.to_ascii_lowercase();
        let df_stem = df.trim_end_matches(".desktop");
        let name_lower = a.name.to_ascii_lowercase();
        df == lower
            || df_stem == stem_hint
            || name_lower == lower
            || df_stem.ends_with(&format!(".{stem_hint}"))
    }) {
        if let Some(ref ic) = app.icon {
            icon_candidates.push(ic.clone());
        }
    }

    // Fallback: usa o próprio hint (normalizado) como nome de ícone
    icon_candidates.push(stem_hint.to_string());
    if stem_hint.contains(' ') {
        icon_candidates.push(stem_hint.replace(' ', "-"));
    }

    for candidate in icon_candidates {
        let cand_path = Path::new(&candidate);
        if cand_path.is_absolute() && cand_path.is_file() {
            return Some(cand_path.to_path_buf());
        }
        if let Some(found) = find_icon_file(&candidate, icon_dirs) {
            return Some(found);
        }
    }

    None
}

fn find_icon_file(icon_name: &str, icon_dirs: &[PathBuf]) -> Option<PathBuf> {
    let has_ext = matches!(
        Path::new(icon_name)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "svg" | "jpg" | "jpeg" | "webp")
    );

    for dir in icon_dirs {
        if !dir.is_dir() {
            continue;
        }
        if has_ext {
            let p = dir.join(icon_name);
            if p.is_file() {
                return Some(p);
            }
            continue;
        }
        for ext in ["png", "svg", "webp", "jpg"] {
            let p = dir.join(format!("{icon_name}.{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::resolve_app_icon_in_dirs;
    use crate::actions_catalog::DesktopApp;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn resolves_icon_from_desktop_entry_and_returns_none_when_missing() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "knobcontext_icons_{}_{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&dir).expect("criar dir");
        let icon_file = dir.join("firefox.png");
        fs::write(&icon_file, b"fake").expect("escrever ícone");

        let apps = vec![DesktopApp {
            desktop_file: "firefox.desktop".into(),
            name: "Mozilla Firefox".into(),
            exec: "firefox".into(),
            icon: Some("firefox".into()),
        }];

        let found = resolve_app_icon_in_dirs("Mozilla Firefox", &apps, std::slice::from_ref(&dir));
        assert_eq!(found.as_deref(), Some(icon_file.as_path()));

        let by_df = resolve_app_icon_in_dirs("firefox.desktop", &apps, std::slice::from_ref(&dir));
        assert_eq!(by_df.as_deref(), Some(icon_file.as_path()));

        let missing =
            resolve_app_icon_in_dirs("inexistente_app_xyz", &apps, std::slice::from_ref(&dir));
        assert_eq!(missing, None);

        let _ = fs::remove_dir_all(&dir);
    }
}
