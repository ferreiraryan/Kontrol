use crate::action::parse_key_name;
use crate::error::KnobError;
use crate::providers::validate_builtin_provider_action;
use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Títulos amigáveis exibidos no popover do construtor visual de ações (ordem fixa 0..10).
pub const ACTION_TYPE_TITLES: [&str; 10] = [
    "Atalho de teclado",
    "Controle de mídia",
    "Abrir aplicativo",
    "Abrir URL",
    "Abrir arquivo",
    "Executar comando",
    "Scroll",
    "Sequência de teclas (macro)",
    "Nenhuma ação",
    "Integração direta",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaAction {
    VolumeUp,
    VolumeDown,
    Mute,
    PlayPause,
    Prev,
    Next,
}

impl MediaAction {
    pub const ALL: [Self; 6] = [
        Self::VolumeUp,
        Self::VolumeDown,
        Self::Mute,
        Self::PlayPause,
        Self::Prev,
        Self::Next,
    ];

    pub fn canonical_name(self) -> &'static str {
        match self {
            Self::VolumeUp => "VolumeUp",
            Self::VolumeDown => "VolumeDown",
            Self::Mute => "Mute",
            Self::PlayPause => "PlayPause",
            Self::Prev => "Prev",
            Self::Next => "Next",
        }
    }

    pub fn friendly_label(self) -> &'static str {
        match self {
            Self::VolumeUp => "Volume +",
            Self::VolumeDown => "Volume -",
            Self::Mute => "Mudo",
            Self::PlayPause => "Play / Pause",
            Self::Prev => "Faixa anterior",
            Self::Next => "Próxima faixa",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

impl ScrollDirection {
    pub const ALL: [Self; 4] = [Self::Up, Self::Down, Self::Left, Self::Right];

    pub fn canonical_name(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    pub fn friendly_label(self) -> &'static str {
        match self {
            Self::Up => "Cima",
            Self::Down => "Baixo",
            Self::Left => "Esquerda",
            Self::Right => "Direita",
        }
    }

    pub fn from_name(input: &str) -> Result<Self, KnobError> {
        match input.trim().to_ascii_lowercase().as_str() {
            "up" | "cima" => Ok(Self::Up),
            "down" | "baixo" => Ok(Self::Down),
            "left" | "esquerda" => Ok(Self::Left),
            "right" | "direita" => Ok(Self::Right),
            other => Err(KnobError::InvalidAction(format!(
                "direção de scroll inválida: '{other}' (use up, down, left ou right)"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MacroStep {
    Keys(Vec<String>),
    WaitMs(u64),
}

impl MacroStep {
    pub fn to_token(&self) -> String {
        match self {
            Self::Keys(keys) => keys.join("+"),
            Self::WaitMs(ms) => format!("Wait({ms})"),
        }
    }
}

/// Catálogo completo dos 10 tipos de ação suportados pelo construtor visual.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionType {
    Shortcut(Vec<String>),
    Media(MediaAction),
    LaunchApp(String),
    OpenUrl(String),
    OpenFile(String),
    ExecCommand(String),
    Scroll {
        direction: ScrollDirection,
        amount: u8,
    },
    Macro(Vec<MacroStep>),
    Provider {
        provider_id: String,
        command_id: String,
        args: BTreeMap<String, String>,
    },
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopApp {
    pub desktop_file: String,
    pub name: String,
    pub exec: String,
    pub icon: Option<String>,
}

pub fn action_type_title(action: &ActionType) -> &'static str {
    match action {
        ActionType::Shortcut(_) => ACTION_TYPE_TITLES[0],
        ActionType::Media(_) => ACTION_TYPE_TITLES[1],
        ActionType::LaunchApp(_) => ACTION_TYPE_TITLES[2],
        ActionType::OpenUrl(_) => ACTION_TYPE_TITLES[3],
        ActionType::OpenFile(_) => ACTION_TYPE_TITLES[4],
        ActionType::ExecCommand(_) => ACTION_TYPE_TITLES[5],
        ActionType::Scroll { .. } => ACTION_TYPE_TITLES[6],
        ActionType::Macro(_) => ACTION_TYPE_TITLES[7],
        ActionType::None => ACTION_TYPE_TITLES[8],
        ActionType::Provider { .. } => ACTION_TYPE_TITLES[9],
    }
}

pub fn action_type_index(action: &ActionType) -> usize {
    match action {
        ActionType::Shortcut(_) => 0,
        ActionType::Media(_) => 1,
        ActionType::LaunchApp(_) => 2,
        ActionType::OpenUrl(_) => 3,
        ActionType::OpenFile(_) => 4,
        ActionType::ExecCommand(_) => 5,
        ActionType::Scroll { .. } => 6,
        ActionType::Macro(_) => 7,
        ActionType::None => 8,
        ActionType::Provider { .. } => 9,
    }
}

pub fn action_summary(action: &ActionType, apps: &[DesktopApp]) -> String {
    match action {
        ActionType::Shortcut(keys) => keys.join("+"),
        ActionType::Media(m) => m.friendly_label().to_string(),
        ActionType::LaunchApp(file) => apps
            .iter()
            .find(|a| a.desktop_file == *file)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| {
                Path::new(file)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| file.trim_end_matches(".desktop").to_string())
            }),
        ActionType::OpenUrl(url) => {
            let short = url
                .trim_start_matches("https://")
                .trim_start_matches("http://");
            if short.len() > 24 {
                format!("{}...", &short[..21])
            } else {
                short.to_string()
            }
        }
        ActionType::OpenFile(uri) => {
            let path = uri.strip_prefix("file://").unwrap_or(uri);
            Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(path)
                .to_string()
        }
        ActionType::ExecCommand(cmd) => {
            if cmd.len() > 24 {
                format!("$ {}...", &cmd[..20])
            } else {
                format!("$ {cmd}")
            }
        }
        ActionType::Scroll { direction, amount } => {
            format!("Scroll {} ×{amount}", direction.friendly_label())
        }
        ActionType::Macro(steps) => {
            let n = steps.len();
            if n == 1 {
                "1 passo".to_string()
            } else {
                format!("{n} passos")
            }
        }
        ActionType::Provider {
            provider_id,
            command_id,
            args,
        } => {
            if args.is_empty() {
                format!("{provider_id} → {command_id}")
            } else {
                let first_val = args.values().next().map(String::as_str).unwrap_or("");
                format!("{provider_id} → {command_id} ({first_val})")
            }
        }
        ActionType::None => "Nenhuma".to_string(),
    }
}

pub fn action_to_string(action: &ActionType) -> String {
    match action {
        ActionType::Shortcut(keys) => keys.join("+"),
        ActionType::Media(media) => media.canonical_name().to_string(),
        ActionType::LaunchApp(file) => format!("Launch:{file}"),
        ActionType::OpenUrl(url) => format!("Open:{url}"),
        ActionType::OpenFile(uri) => {
            if uri.starts_with("file://") {
                format!("Open:{uri}")
            } else {
                format!("Open:file://{uri}")
            }
        }
        ActionType::ExecCommand(cmd) => format!("Exec:{cmd}"),
        ActionType::Scroll { direction, amount } => {
            format!("Scroll:{}:{amount}", direction.canonical_name())
        }
        ActionType::Macro(steps) => {
            let joined = steps
                .iter()
                .map(MacroStep::to_token)
                .collect::<Vec<_>>()
                .join(",");
            format!("Macro:{joined}")
        }
        ActionType::Provider {
            provider_id,
            command_id,
            args,
        } => {
            if args.is_empty() {
                format!("Provider:{provider_id}:{command_id}")
            } else {
                let encoded = args
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join(",");
                format!("Provider:{provider_id}:{command_id}:{encoded}")
            }
        }
        ActionType::None => "None".to_string(),
    }
}

pub fn parse_action_string(input: &str) -> Result<ActionType, KnobError> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        return Ok(ActionType::None);
    }

    // Prefixos ricos (case-insensitive no prefixo, preserva conteúdo)
    if let Some(rest) = strip_prefix_ignore_case(trimmed, "launch:") {
        let desktop_file = rest.trim();
        if desktop_file.is_empty() {
            return Err(KnobError::InvalidAction(
                "Launch: requer o nome do arquivo .desktop (ex.: Launch:firefox.desktop)"
                    .to_string(),
            ));
        }
        return Ok(ActionType::LaunchApp(desktop_file.to_string()));
    }

    if let Some(rest) = strip_prefix_ignore_case(trimmed, "open:") {
        let target = rest.trim();
        if target.starts_with("http://") || target.starts_with("https://") {
            let without_scheme = target
                .strip_prefix("https://")
                .or_else(|| target.strip_prefix("http://"))
                .unwrap_or("");
            if without_scheme.is_empty()
                || without_scheme.starts_with('/')
                || without_scheme.contains(char::is_whitespace)
            {
                return Err(KnobError::InvalidAction(format!(
                    "URL inválida em Open: '{target}'"
                )));
            }
            return Ok(ActionType::OpenUrl(target.to_string()));
        }
        if let Some(path) = target.strip_prefix("file://") {
            if path.trim().is_empty() {
                return Err(KnobError::InvalidAction(
                    "Open:file:// requer um caminho de arquivo válido".to_string(),
                ));
            }
            return Ok(ActionType::OpenFile(target.to_string()));
        }
        return Err(KnobError::InvalidAction(format!(
            "Open: requer prefixo http://, https:// ou file:// (recebido: '{target}')"
        )));
    }

    if let Some(rest) = strip_prefix_ignore_case(trimmed, "exec:") {
        let cmd = rest.trim();
        if cmd.is_empty() {
            return Err(KnobError::InvalidAction(
                "Exec: não pode ter comando vazio".to_string(),
            ));
        }
        return Ok(ActionType::ExecCommand(cmd.to_string()));
    }

    if let Some(rest) = strip_prefix_ignore_case(trimmed, "scroll:") {
        let mut parts = rest.split(':');
        let dir_str = parts.next().unwrap_or("");
        let amount_str = parts.next().unwrap_or("");
        if parts.next().is_some() || dir_str.trim().is_empty() || amount_str.trim().is_empty() {
            return Err(KnobError::InvalidAction(format!(
                "formato Scroll inválido: '{trimmed}' (esperado Scroll:<dir>:<1-10>)"
            )));
        }
        let direction = ScrollDirection::from_name(dir_str)?;
        let amount = amount_str.trim().parse::<u8>().map_err(|_| {
            KnobError::InvalidAction(format!(
                "quantidade de Scroll inválida: '{amount_str}' (use 1 a 10)"
            ))
        })?;
        if !(1..=10).contains(&amount) {
            return Err(KnobError::InvalidAction(format!(
                "quantidade de Scroll fora do intervalo 1..=10: {amount}"
            )));
        }
        return Ok(ActionType::Scroll { direction, amount });
    }

    if let Some(rest) = strip_prefix_ignore_case(trimmed, "provider:") {
        let mut parts = rest.splitn(3, ':');
        let provider_id = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        let command_id = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        let raw_args = parts.next().map(str::trim).unwrap_or("");

        if provider_id.is_empty() || command_id.is_empty() {
            return Err(KnobError::InvalidAction(format!(
                "formato Provider inválido: '{trimmed}' (esperado Provider:<id>:<comando>[:chave=valor,...])"
            )));
        }

        let mut args = BTreeMap::new();
        if !raw_args.is_empty() {
            for pair in raw_args.split(',') {
                let entry = pair.trim();
                if entry.is_empty() {
                    continue;
                }
                let Some((k, v)) = entry.split_once('=') else {
                    return Err(KnobError::InvalidAction(format!(
                        "argumento inválido em Provider: '{entry}' (use chave=valor)"
                    )));
                };
                let key = k.trim();
                let val = v.trim();
                if key.is_empty() || val.is_empty() {
                    return Err(KnobError::InvalidAction(format!(
                        "argumento vazio em Provider: '{entry}'"
                    )));
                }
                args.insert(key.to_string(), val.to_string());
            }
        }

        validate_builtin_provider_action(&provider_id, &command_id, &args)
            .map_err(|err| KnobError::InvalidAction(err.to_string()))?;

        return Ok(ActionType::Provider {
            provider_id,
            command_id,
            args,
        });
    }

    if let Some(rest) = strip_prefix_ignore_case(trimmed, "macro:") {
        let body = rest.trim();
        if body.is_empty() {
            return Err(KnobError::InvalidAction(
                "Macro: precisa de ao menos um passo (ex.: Macro:Ctrl+C,Wait(100),Ctrl+V)"
                    .to_string(),
            ));
        }
        let mut steps = Vec::new();
        for raw_step in body.split(',') {
            let step_str = raw_step.trim();
            if step_str.is_empty() {
                return Err(KnobError::InvalidAction(
                    "Macro: contém passo vazio".to_string(),
                ));
            }
            if let Some(inner) =
                strip_prefix_ignore_case(step_str, "wait(").and_then(|s| s.strip_suffix(')'))
            {
                let ms = inner.trim().parse::<u64>().map_err(|_| {
                    KnobError::InvalidAction(format!(
                        "tempo de espera inválido em Macro: '{step_str}'"
                    ))
                })?;
                if ms == 0 || ms > 60_000 {
                    return Err(KnobError::InvalidAction(format!(
                        "Wait(ms) em Macro deve estar entre 1 e 60000 ms (recebido: {ms})"
                    )));
                }
                steps.push(MacroStep::WaitMs(ms));
            } else {
                let keys = parse_shortcut_keys(step_str)?;
                steps.push(MacroStep::Keys(keys));
            }
        }
        return Ok(ActionType::Macro(steps));
    }

    // Mídia e aliases legados
    let lower = trimmed.to_ascii_lowercase();
    match lower.as_str() {
        "volumeup" | "volume_up" => return Ok(ActionType::Media(MediaAction::VolumeUp)),
        "volumedown" | "volume_down" => return Ok(ActionType::Media(MediaAction::VolumeDown)),
        "volumemute" | "volume_mute" | "mute" => return Ok(ActionType::Media(MediaAction::Mute)),
        "mediaplaypause" | "media_play_pause" | "playpause" => {
            return Ok(ActionType::Media(MediaAction::PlayPause));
        }
        "mediaprev" | "media_prev" | "prev" => return Ok(ActionType::Media(MediaAction::Prev)),
        "medianext" | "media_next" | "next" => return Ok(ActionType::Media(MediaAction::Next)),
        "next_tab" | "nexttab" => {
            return Ok(ActionType::Shortcut(vec![
                "Ctrl".to_string(),
                "Tab".to_string(),
            ]));
        }
        "prev_tab" | "prevtab" => {
            return Ok(ActionType::Shortcut(vec![
                "Ctrl".to_string(),
                "Shift".to_string(),
                "Tab".to_string(),
            ]));
        }
        _ => {}
    }

    // Compatibilidade com formato legado Scroll(N) / HScroll(N)
    if let Some(inner) = lower
        .strip_prefix("scroll(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let delta = inner.trim().parse::<i32>().map_err(|_| {
            KnobError::InvalidAction(format!("delta inválido em Scroll: '{trimmed}'"))
        })?;
        let amount = delta.unsigned_abs().clamp(1, 10) as u8;
        let direction = if delta >= 0 {
            ScrollDirection::Down
        } else {
            ScrollDirection::Up
        };
        return Ok(ActionType::Scroll { direction, amount });
    }
    if let Some(inner) = lower
        .strip_prefix("hscroll(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let delta = inner.trim().parse::<i32>().map_err(|_| {
            KnobError::InvalidAction(format!("delta inválido em HScroll: '{trimmed}'"))
        })?;
        let amount = delta.unsigned_abs().clamp(1, 10) as u8;
        let direction = if delta >= 0 {
            ScrollDirection::Right
        } else {
            ScrollDirection::Left
        };
        return Ok(ActionType::Scroll { direction, amount });
    }

    // Atalho de teclado
    let keys = parse_shortcut_keys(trimmed)?;
    Ok(ActionType::Shortcut(keys))
}

pub fn parse_shortcut_keys(raw: &str) -> Result<Vec<String>, KnobError> {
    let keys: Vec<String> = raw
        .split('+')
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect();
    if keys.is_empty() {
        return Err(KnobError::InvalidAction(
            "atalho de teclado não pode estar vazio".to_string(),
        ));
    }
    for key in &keys {
        parse_key_name(key)?;
    }
    Ok(keys)
}

fn strip_prefix_ignore_case<'a>(input: &'a str, prefix: &str) -> Option<&'a str> {
    if input.len() < prefix.len() {
        return None;
    }
    let (head, tail) = input.split_at(prefix.len());
    if head.eq_ignore_ascii_case(prefix) {
        Some(tail)
    } else {
        None
    }
}

/// Diretórios padrão de arquivos `.desktop` no Linux (usuário primeiro para prioridade).
#[cfg_attr(target_os = "windows", allow(dead_code))]
pub fn default_desktop_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(home) = env::var("HOME") {
        if !home.trim().is_empty() {
            dirs.push(PathBuf::from(home).join(".local/share/applications"));
        }
    }
    dirs.push(PathBuf::from("/usr/share/applications"));
    dirs
}

/// Diretórios padrão de atalhos do Menu Iniciar no Windows.
#[cfg(any(target_os = "windows", test))]
pub fn default_windows_start_menu_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(appdata) = env::var("APPDATA") {
        if !appdata.trim().is_empty() {
            dirs.push(
                PathBuf::from(appdata)
                    .join("Microsoft")
                    .join("Windows")
                    .join("Start Menu")
                    .join("Programs"),
            );
        }
    }
    if let Ok(progdata) = env::var("ProgramData") {
        if !progdata.trim().is_empty() {
            dirs.push(
                PathBuf::from(progdata)
                    .join("Microsoft")
                    .join("Windows")
                    .join("Start Menu")
                    .join("Programs"),
            );
        }
    }
    dirs
}

#[cfg(any(target_os = "windows", test))]
fn is_ignored_windows_shortcut_name(stem_lower: &str) -> bool {
    const IGNORED_KEYWORDS: &[&str] = &[
        "uninstall",
        "desinstalar",
        "readme",
        "license",
        "licença",
        "manual",
        "documentation",
        "help",
        "website",
        "release notes",
    ];
    IGNORED_KEYWORDS.iter().any(|kw| stem_lower.contains(kw))
}

#[cfg(any(target_os = "windows", test))]
fn collect_lnk_shortcuts_recursive(
    dir: &Path,
    depth: usize,
    seen_names: &mut HashSet<String>,
    out: &mut Vec<DesktopApp>,
) {
    if depth > 3 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_lnk_shortcuts_recursive(&path, depth + 1, seen_names, out);
            continue;
        }
        let is_lnk = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("lnk"));
        if !is_lnk {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let name = stem.trim();
        let lower = name.to_ascii_lowercase();
        if name.is_empty() || is_ignored_windows_shortcut_name(&lower) || !seen_names.insert(lower)
        {
            continue;
        }
        let full_path = path.to_string_lossy().into_owned();
        out.push(DesktopApp {
            desktop_file: full_path.clone(),
            name: name.to_string(),
            exec: full_path,
            icon: None,
        });
    }
}

/// Lista aplicativos instalados a partir de diretórios de atalhos `.lnk` do Menu Iniciar (Windows).
#[cfg(any(target_os = "windows", test))]
pub fn list_windows_apps_in_dirs(dirs: &[PathBuf]) -> Vec<DesktopApp> {
    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for dir in dirs {
        collect_lnk_shortcuts_recursive(dir, 0, &mut seen, &mut apps);
    }
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

/// Lista aplicativos disponíveis na plataforma atual:
/// - Windows: atalhos `.lnk` do Menu Iniciar (`Start Menu\Programs`).
/// - Linux: arquivos `.desktop` em `/usr/share/applications` e `~/.local/share/applications`.
pub fn list_desktop_apps() -> Vec<DesktopApp> {
    #[cfg(target_os = "windows")]
    {
        list_windows_apps_in_dirs(&default_windows_start_menu_dirs())
    }
    #[cfg(not(target_os = "windows"))]
    {
        list_desktop_apps_in_dirs(&default_desktop_dirs())
    }
}

/// Lista aplicativos `.desktop` a partir de uma lista explícita de diretórios (testável).
pub fn list_desktop_apps_in_dirs(dirs: &[PathBuf]) -> Vec<DesktopApp> {
    let mut seen = HashSet::new();
    let mut apps = Vec::new();

    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !seen.insert(file_name.to_string()) {
                continue;
            }
            if let Ok(content) = fs::read_to_string(&path) {
                if let Some(app) = parse_desktop_entry(file_name, &content) {
                    apps.push(app);
                }
            }
        }
    }

    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

fn parse_desktop_entry(desktop_file: &str, content: &str) -> Option<DesktopApp> {
    let mut in_desktop_entry = false;
    let mut name: Option<String> = None;
    let mut exec: Option<String> = None;
    let mut icon: Option<String> = None;
    let mut no_display = false;
    let mut hidden = false;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "Name" if name.is_none() => name = Some(value.to_string()),
            "Exec" if exec.is_none() => exec = Some(value.to_string()),
            "Icon" if icon.is_none() && !value.is_empty() => icon = Some(value.to_string()),
            "NoDisplay" => no_display = value.eq_ignore_ascii_case("true"),
            "Hidden" => hidden = value.eq_ignore_ascii_case("true"),
            _ => {}
        }
    }

    if no_display || hidden {
        return None;
    }

    Some(DesktopApp {
        desktop_file: desktop_file.to_string(),
        name: name?,
        exec: exec?,
        icon,
    })
}

/// Resolve o campo `Exec=` de um arquivo `.desktop` em argumentos seguros para `Command` (sem shell).
#[cfg_attr(target_os = "windows", allow(dead_code))]
pub fn resolve_desktop_exec(desktop_file: &str) -> Result<Vec<String>, KnobError> {
    resolve_desktop_exec_in_dirs(desktop_file, &default_desktop_dirs())
}

pub fn resolve_desktop_exec_in_dirs(
    desktop_file: &str,
    dirs: &[PathBuf],
) -> Result<Vec<String>, KnobError> {
    let apps = list_desktop_apps_in_dirs(dirs);
    let app = apps
        .iter()
        .find(|a| a.desktop_file == desktop_file)
        .ok_or_else(|| {
            KnobError::InputSimulation(format!(
                "aplicativo '.desktop' não encontrado: '{desktop_file}'"
            ))
        })?;
    parse_desktop_exec_tokens(&app.exec)
}

/// Tokeniza uma linha `Exec=` de `.desktop` respeitando aspas e removendo placeholders `%u/%U/%f/%F/...`.
pub fn parse_desktop_exec_tokens(exec_line: &str) -> Result<Vec<String>, KnobError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes: Option<char> = None;
    let mut escaped = false;

    for ch in exec_line.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' && in_quotes != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(q) = in_quotes {
            if ch == q {
                in_quotes = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        match ch {
            '"' | '\'' => in_quotes = Some(ch),
            c if c.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    let cleaned: Vec<String> = tokens
        .into_iter()
        .filter(|tok| {
            !(tok.len() == 2
                && tok.starts_with('%')
                && matches!(
                    tok.chars().nth(1),
                    Some(
                        'f' | 'F' | 'u' | 'U' | 'd' | 'D' | 'n' | 'N' | 'i' | 'c' | 'k' | 'v' | 'm'
                    )
                ))
        })
        .collect();

    if cleaned.is_empty() {
        return Err(KnobError::InvalidAction(format!(
            "campo Exec= vazio ou inválido: '{exec_line}'"
        )));
    }
    Ok(cleaned)
}

#[cfg(test)]
mod tests {
    use super::{
        action_to_string, action_type_title, list_desktop_apps_in_dirs, parse_action_string,
        parse_desktop_exec_tokens, resolve_desktop_exec_in_dirs, ActionType, MacroStep,
        MediaAction, ScrollDirection,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir_unique(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "knobcontext_desktop_{}_{}_{}",
            std::process::id(),
            nanos,
            tag
        ));
        fs::create_dir_all(&dir).expect("criar diretório temporário");
        dir
    }

    #[test]
    fn round_trips_all_nine_action_types() {
        let samples = vec![
            (
                "Ctrl+Tab",
                ActionType::Shortcut(vec!["Ctrl".into(), "Tab".into()]),
            ),
            (
                "Super+E",
                ActionType::Shortcut(vec!["Super".into(), "E".into()]),
            ),
            ("VolumeUp", ActionType::Media(MediaAction::VolumeUp)),
            ("Mute", ActionType::Media(MediaAction::Mute)),
            ("PlayPause", ActionType::Media(MediaAction::PlayPause)),
            (
                "Launch:firefox.desktop",
                ActionType::LaunchApp("firefox.desktop".into()),
            ),
            (
                "Open:https://example.com",
                ActionType::OpenUrl("https://example.com".into()),
            ),
            (
                "Open:file:///home/user/doc.pdf",
                ActionType::OpenFile("file:///home/user/doc.pdf".into()),
            ),
            (
                "Exec:playerctl play-pause",
                ActionType::ExecCommand("playerctl play-pause".into()),
            ),
            (
                "Scroll:up:3",
                ActionType::Scroll {
                    direction: ScrollDirection::Up,
                    amount: 3,
                },
            ),
            (
                "Macro:Ctrl+C,Wait(100),Ctrl+V",
                ActionType::Macro(vec![
                    MacroStep::Keys(vec!["Ctrl".into(), "C".into()]),
                    MacroStep::WaitMs(100),
                    MacroStep::Keys(vec!["Ctrl".into(), "V".into()]),
                ]),
            ),
            (
                "Provider:obs:toggle_record",
                ActionType::Provider {
                    provider_id: "obs".into(),
                    command_id: "toggle_record".into(),
                    args: std::collections::BTreeMap::new(),
                },
            ),
            (
                "Provider:obs:toggle_input_mute:input_name=Mic/Aux",
                ActionType::Provider {
                    provider_id: "obs".into(),
                    command_id: "toggle_input_mute".into(),
                    args: std::collections::BTreeMap::from([(
                        "input_name".into(),
                        "Mic/Aux".into(),
                    )]),
                },
            ),
            (
                "Provider:mpris:play_pause",
                ActionType::Provider {
                    provider_id: "mpris".into(),
                    command_id: "play_pause".into(),
                    args: std::collections::BTreeMap::new(),
                },
            ),
            (
                "Provider:mpris:seek:seconds=10",
                ActionType::Provider {
                    provider_id: "mpris".into(),
                    command_id: "seek".into(),
                    args: std::collections::BTreeMap::from([("seconds".into(), "10".into())]),
                },
            ),
            (
                "Provider:mpris:volume_up:step=5",
                ActionType::Provider {
                    provider_id: "mpris".into(),
                    command_id: "volume_up".into(),
                    args: std::collections::BTreeMap::from([("step".into(), "5".into())]),
                },
            ),
            (
                "Provider:mpris:volume_down",
                ActionType::Provider {
                    provider_id: "mpris".into(),
                    command_id: "volume_down".into(),
                    args: std::collections::BTreeMap::new(),
                },
            ),
            ("None", ActionType::None),
        ];

        for (raw, expected) in samples {
            let parsed = parse_action_string(raw).expect("parse válido");
            assert_eq!(parsed, expected, "falha ao parsear '{raw}'");
            assert_eq!(action_to_string(&parsed), raw);
            assert!(!action_type_title(&parsed).is_empty());
        }
    }

    #[test]
    fn rejects_invalid_action_strings() {
        let invalid = [
            "Launch:",
            "Open:ftp://invalid",
            "Open:https://",
            "Open:file://",
            "Exec:   ",
            "Scroll:up:0",
            "Scroll:up:11",
            "Scroll:diagonal:2",
            "Macro:",
            "Macro:Wait(0)",
            "Macro:Ctrl+TeclaInvalida",
            "Provider:",
            "Provider:obs",
            "Provider:obs:",
            "Provider:obs:set_scene:sem_igual",
            "Provider:obs:set_scene:scene_name=",
            "Provider:obs:comando_inexistente",
            "Provider:provider_inexistente:toggle_record",
            "TeclaInexistenteXYZ",
        ];

        for raw in invalid {
            assert!(
                parse_action_string(raw).is_err(),
                "esperava erro para '{raw}'"
            );
        }
    }

    #[test]
    fn lists_desktop_apps_and_ignores_nodisplay() {
        let dir = temp_dir_unique("apps");
        fs::write(
            dir.join("firefox.desktop"),
            "[Desktop Entry]\nName=Firefox\nExec=/usr/lib/firefox/firefox %u\nType=Application\n",
        )
        .expect("escrever firefox.desktop");
        fs::write(
            dir.join("hidden.desktop"),
            "[Desktop Entry]\nName=Hidden Tool\nExec=/usr/bin/hidden\nNoDisplay=true\n",
        )
        .expect("escrever hidden.desktop");

        let apps = list_desktop_apps_in_dirs(std::slice::from_ref(&dir));
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].desktop_file, "firefox.desktop");
        assert_eq!(apps[0].name, "Firefox");

        let tokens = resolve_desktop_exec_in_dirs("firefox.desktop", std::slice::from_ref(&dir))
            .expect("resolver Exec=");
        assert_eq!(tokens, vec!["/usr/lib/firefox/firefox"]);

        let quoted =
            parse_desktop_exec_tokens("\"my app\" --flag %F").expect("tokenizar com aspas");
        assert_eq!(quoted, vec!["my app", "--flag"]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn lists_windows_lnk_shortcuts_and_ignores_uninstallers() {
        let dir = temp_dir_unique("win_apps");
        let sub = dir.join("Adobe");
        fs::create_dir_all(&sub).expect("criar subpasta");
        fs::write(dir.join("Spotify.lnk"), b"lnk").expect("spotify");
        fs::write(dir.join("Uninstall Spotify.lnk"), b"lnk").expect("uninstall");
        fs::write(sub.join("Photoshop.lnk"), b"lnk").expect("photoshop");
        fs::write(sub.join("Readme.lnk"), b"lnk").expect("readme");
        fs::write(dir.join("notes.txt"), b"txt").expect("txt");

        let apps = super::list_windows_apps_in_dirs(std::slice::from_ref(&dir));
        let names: Vec<&str> = apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["Photoshop", "Spotify"]);

        let _ = fs::remove_dir_all(&dir);
    }
}
