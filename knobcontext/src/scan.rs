use std::collections::BTreeSet;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtcSessionSummary {
    pub app_id: String,
    pub playback_status: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub has_thumbnail: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedApp {
    pub category: &'static str,
    pub name: String,
    pub source: String,
    pub running: bool,
    pub known_api: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanReport {
    pub os: String,
    pub running_processes: Vec<String>,
    pub installed_dirs: Vec<(String, Vec<String>)>,
    pub highlighted_apps: Vec<DetectedApp>,
    pub smtc_sessions: Vec<SmtcSessionSummary>,
    pub smtc_error: Option<String>,
    pub obs_websocket_open: bool,
    pub discord_rpc_pipe: Option<String>,
}

/// Faz o parsing da saída CSV de `tasklist /FO CSV /NH` retornando nomes únicos ordenados.
pub fn parse_tasklist_csv(raw: &str) -> Vec<String> {
    let mut set = BTreeSet::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let first_col = if let Some(rest) = trimmed.strip_prefix('"') {
            rest.split('"').next().unwrap_or("")
        } else {
            trimmed.split(',').next().unwrap_or("")
        };
        let name = first_col.trim();
        if !name.is_empty() && !name.eq_ignore_ascii_case("Image Name") {
            set.insert(name.to_string());
        }
    }
    let mut list: Vec<String> = set.into_iter().collect();
    list.sort_by_key(|s| s.to_ascii_lowercase());
    list
}

fn list_running_processes() -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        if let Ok(output) = Command::new("tasklist")
            .args(["/FO", "CSV", "/NH"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            if output.status.success() {
                let raw = String::from_utf8_lossy(&output.stdout);
                return parse_tasklist_csv(&raw);
            }
        }
        Vec::new()
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(output) = Command::new("ps").args(["-eo", "comm="]).output() {
            if output.status.success() {
                let raw = String::from_utf8_lossy(&output.stdout);
                let mut set = BTreeSet::new();
                for line in raw.lines() {
                    let name = line.trim();
                    if !name.is_empty() {
                        set.insert(name.to_string());
                    }
                }
                return set.into_iter().collect();
            }
        }
        Vec::new()
    }
}

fn read_dir_entry_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if !name.starts_with('.') {
                    names.push(name.to_string());
                }
            }
        }
    }
    names.sort_by_key(|s| s.to_ascii_lowercase());
    names
}

fn scan_installed_directories() -> Vec<(String, Vec<String>)> {
    let mut dirs_to_scan: Vec<(String, PathBuf)> = Vec::new();

    #[cfg(target_os = "windows")]
    {
        dirs_to_scan.push((
            "C:\\Program Files".to_string(),
            PathBuf::from("C:\\Program Files"),
        ));
        dirs_to_scan.push((
            "C:\\Program Files (x86)".to_string(),
            PathBuf::from("C:\\Program Files (x86)"),
        ));
        if let Ok(local) = env::var("LOCALAPPDATA") {
            let local_path = PathBuf::from(&local);
            let programs = local_path.join("Programs");
            if programs.is_dir() {
                dirs_to_scan.push(("%LOCALAPPDATA%\\Programs".to_string(), programs));
            }
            dirs_to_scan.push(("%LOCALAPPDATA%".to_string(), local_path));
        }
        if let Ok(roaming) = env::var("APPDATA") {
            let spotify = PathBuf::from(&roaming).join("Spotify");
            if spotify.is_dir() {
                dirs_to_scan.push(("%APPDATA% (mídia)".to_string(), PathBuf::from(&roaming)));
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        dirs_to_scan.push((
            "/usr/share/applications".to_string(),
            PathBuf::from("/usr/share/applications"),
        ));
    }

    let mut out = Vec::new();
    for (label, path) in dirs_to_scan {
        let entries = read_dir_entry_names(&path);
        if !entries.is_empty() {
            out.push((label, entries));
        }
    }
    out
}

pub fn probe_obs_websocket(port: u16) -> bool {
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok()
}

pub fn probe_discord_rpc() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        for i in 0..10 {
            let pipe = format!(r"\\.\pipe\discord-ipc-{i}");
            match fs::OpenOptions::new().read(true).write(true).open(&pipe) {
                Ok(_) => return Some(pipe),
                Err(err) if err.raw_os_error() == Some(231) => {
                    // ERROR_PIPE_BUSY (231): pipe existe e está ocupado por outro cliente.
                    return Some(pipe);
                }
                Err(_) => continue,
            }
        }
        None
    }
    #[cfg(not(target_os = "windows"))]
    {
        let runtime = env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
        for i in 0..10 {
            let path = PathBuf::from(&runtime).join(format!("discord-ipc-{i}"));
            if path.exists() {
                return Some(path.display().to_string());
            }
        }
        None
    }
}

#[cfg(target_os = "windows")]
fn probe_smtc_sessions() -> (Vec<SmtcSessionSummary>, Option<String>) {
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSessionManager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
    };
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};

    // SAFETY: inicialização COM STA compatível com WinRT e threads de UI.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };

    let manager = match GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
        .and_then(|op| op.join())
    {
        Ok(m) => m,
        Err(err) => return (Vec::new(), Some(format!("Falha ao abrir SMTC: {err}"))),
    };

    let sessions = match manager.GetSessions() {
        Ok(list) => list,
        Err(err) => {
            return (
                Vec::new(),
                Some(format!("Falha ao listar sessões SMTC: {err}")),
            )
        }
    };

    let mut out = Vec::new();
    let count = sessions.Size().unwrap_or(0);
    for idx in 0..count {
        let Ok(session) = sessions.GetAt(idx) else {
            continue;
        };
        let app_id = session
            .SourceAppUserModelId()
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "<desconhecido>".to_string());

        let playback_status = session
            .GetPlaybackInfo()
            .and_then(|info| info.PlaybackStatus())
            .map(|st| match st {
                Status::Playing => "Playing",
                Status::Paused => "Paused",
                Status::Stopped => "Stopped",
                Status::Closed => "Closed",
                Status::Opened => "Opened",
                Status::Changing => "Changing",
                _ => "Unknown",
            })
            .unwrap_or("Unknown")
            .to_string();

        let (title, artist, album, has_thumbnail) = session
            .TryGetMediaPropertiesAsync()
            .and_then(|op| op.join())
            .map(|props| {
                (
                    props.Title().map(|s| s.to_string()).unwrap_or_default(),
                    props.Artist().map(|s| s.to_string()).unwrap_or_default(),
                    props
                        .AlbumTitle()
                        .map(|s| s.to_string())
                        .unwrap_or_default(),
                    props.Thumbnail().is_ok(),
                )
            })
            .unwrap_or_default();

        out.push(SmtcSessionSummary {
            app_id,
            playback_status,
            title,
            artist,
            album,
            has_thumbnail,
        });
    }

    (out, None)
}

#[cfg(not(target_os = "windows"))]
fn probe_smtc_sessions() -> (Vec<SmtcSessionSummary>, Option<String>) {
    (
        Vec::new(),
        Some("SMTC disponível apenas no Windows (use MPRIS no Linux).".to_string()),
    )
}

struct AppSignature {
    category: &'static str,
    display_name: &'static str,
    dir_needles: &'static [&'static str],
    proc_needles: &'static [&'static str],
    known_api: &'static str,
}

const KNOWN_APPS: &[AppSignature] = &[
    AppSignature {
        category: "Navegador",
        display_name: "Google Chrome",
        dir_needles: &["google"],
        proc_needles: &["chrome.exe"],
        known_api: "SMTC (mídia/vídeo) + atalhos",
    },
    AppSignature {
        category: "Navegador",
        display_name: "Microsoft Edge",
        dir_needles: &["edge"],
        proc_needles: &["msedge.exe"],
        known_api: "SMTC (mídia/vídeo) + atalhos",
    },
    AppSignature {
        category: "Navegador",
        display_name: "Mozilla Firefox",
        dir_needles: &["mozilla firefox", "firefox"],
        proc_needles: &["firefox.exe"],
        known_api: "SMTC (mídia/vídeo) + atalhos",
    },
    AppSignature {
        category: "Navegador",
        display_name: "Brave / Opera / Vivaldi / Zen",
        dir_needles: &["bravesoftware", "opera", "vivaldi", "zen browser"],
        proc_needles: &["brave.exe", "opera.exe", "vivaldi.exe", "zen.exe"],
        known_api: "SMTC (mídia/vídeo) + atalhos",
    },
    AppSignature {
        category: "Editor / IDE",
        display_name: "Visual Studio Code",
        dir_needles: &["microsoft vs code", "vscode"],
        proc_needles: &["code.exe"],
        known_api: "Atalhos / Paleta de comandos",
    },
    AppSignature {
        category: "Editor / IDE",
        display_name: "Cursor / Windsurf / Antigravity",
        dir_needles: &["cursor", "windsurf", "antigravity"],
        proc_needles: &["cursor.exe", "windsurf.exe", "antigravity.exe"],
        known_api: "Atalhos / Paleta de comandos",
    },
    AppSignature {
        category: "Editor / IDE",
        display_name: "Visual Studio",
        dir_needles: &["microsoft visual studio"],
        proc_needles: &["devenv.exe"],
        known_api: "Atalhos",
    },
    AppSignature {
        category: "Editor / IDE",
        display_name: "JetBrains IDEs",
        dir_needles: &["jetbrains"],
        proc_needles: &[
            "idea64.exe",
            "pycharm64.exe",
            "clion64.exe",
            "rustrover64.exe",
        ],
        known_api: "Atalhos",
    },
    AppSignature {
        category: "Mídia",
        display_name: "Spotify",
        dir_needles: &["spotify"],
        proc_needles: &["spotify.exe"],
        known_api: "SMTC (nativo + capa)",
    },
    AppSignature {
        category: "Mídia",
        display_name: "VLC Media Player",
        dir_needles: &["videolan", "vlc"],
        proc_needles: &["vlc.exe"],
        known_api: "SMTC / HTTP API",
    },
    AppSignature {
        category: "Mídia",
        display_name: "YouTube Music / Tidal / Apple Music / foobar2000",
        dir_needles: &[
            "youtube music",
            "tidal",
            "apple music",
            "foobar2000",
            "musicbee",
        ],
        proc_needles: &[
            "youtube music.exe",
            "tidal.exe",
            "applemusic.exe",
            "foobar2000.exe",
            "musicbee.exe",
        ],
        known_api: "SMTC (nativo)",
    },
    AppSignature {
        category: "Streaming / Áudio",
        display_name: "OBS Studio",
        dir_needles: &["obs-studio"],
        proc_needles: &["obs64.exe", "obs32.exe"],
        known_api: "OBS WebSocket 5.x (porta 4455)",
    },
    AppSignature {
        category: "Comunicação",
        display_name: "Discord",
        dir_needles: &["discord"],
        proc_needles: &["discord.exe", "discordptb.exe", "discordcanary.exe"],
        known_api: "Discord RPC (named pipe) / Push-to-Mute",
    },
    AppSignature {
        category: "Criativo",
        display_name: "Adobe Photoshop / Premiere / After Effects / Illustrator",
        dir_needles: &["adobe"],
        proc_needles: &[
            "photoshop.exe",
            "adobe premiere pro.exe",
            "afterfx.exe",
            "illustrator.exe",
        ],
        known_api: "Atalhos (brush/timeline/zoom)",
    },
    AppSignature {
        category: "Criativo",
        display_name: "GIMP / Krita / Blender / DaVinci Resolve / Figma",
        dir_needles: &[
            "gimp",
            "krita",
            "blender foundation",
            "blackmagic design",
            "figma",
        ],
        proc_needles: &[
            "gimp-2.10.exe",
            "gimp-3.exe",
            "krita.exe",
            "blender.exe",
            "resolve.exe",
            "figma.exe",
        ],
        known_api: "Atalhos (brush/timeline/zoom)",
    },
    AppSignature {
        category: "Jogos / Launcher",
        display_name: "Steam / Epic Games / Riot / Xbox / Battle.net",
        dir_needles: &["steam", "epic games", "riot games", "battle.net"],
        proc_needles: &[
            "steam.exe",
            "epicgameslauncher.exe",
            "riotclientservices.exe",
            "battle.net.exe",
        ],
        known_api: "Perfis de jogos (scroll/atalhos)",
    },
    AppSignature {
        category: "Terminal",
        display_name: "Windows Terminal / PowerShell / Git Bash",
        dir_needles: &["git", "wezterm", "alacritty"],
        proc_needles: &[
            "windowsterminal.exe",
            "pwsh.exe",
            "powershell.exe",
            "wezterm-gui.exe",
        ],
        known_api: "Atalhos de terminal",
    },
];

pub fn match_known_apps(
    running_processes: &[String],
    installed_dirs: &[(String, Vec<String>)],
) -> Vec<DetectedApp> {
    let procs_lower: Vec<String> = running_processes
        .iter()
        .map(|p| p.to_ascii_lowercase())
        .collect();

    let mut found = Vec::new();
    for sig in KNOWN_APPS {
        let running = sig
            .proc_needles
            .iter()
            .any(|needle| procs_lower.iter().any(|p| p == needle));

        let mut sources = Vec::new();
        for (dir_label, entries) in installed_dirs {
            for entry in entries {
                let entry_lower = entry.to_ascii_lowercase();
                if sig
                    .dir_needles
                    .iter()
                    .any(|needle| entry_lower == *needle || entry_lower.contains(needle))
                {
                    sources.push(format!("{dir_label}\\{entry}"));
                    break;
                }
            }
        }

        if running || !sources.is_empty() {
            let source = if sources.is_empty() {
                "em execução (processo)".to_string()
            } else {
                sources.join(", ")
            };
            found.push(DetectedApp {
                category: sig.category,
                name: sig.display_name.to_string(),
                source,
                running,
                known_api: sig.known_api,
            });
        }
    }
    found
}

pub fn collect_scan_report() -> ScanReport {
    let running_processes = list_running_processes();
    let installed_dirs = scan_installed_directories();
    let highlighted_apps = match_known_apps(&running_processes, &installed_dirs);
    let (smtc_sessions, smtc_error) = probe_smtc_sessions();
    let obs_websocket_open = probe_obs_websocket(4455);
    let discord_rpc_pipe = probe_discord_rpc();

    ScanReport {
        os: format!("{} ({})", env::consts::OS, env::consts::ARCH),
        running_processes,
        installed_dirs,
        highlighted_apps,
        smtc_sessions,
        smtc_error,
        obs_websocket_open,
        discord_rpc_pipe,
    }
}

pub fn format_scan_report(report: &ScanReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "=== KnobContext System Scan Report ===");
    let _ = writeln!(out, "SO: {}\n", report.os);

    let _ = writeln!(out, "## 1. APIs e Integrações Detectadas ao Vivo");
    let _ = writeln!(
        out,
        "- OBS WebSocket (127.0.0.1:4455): {}",
        if report.obs_websocket_open {
            "ABERTA (OBS respondendo)"
        } else {
            "fechada / não em execução"
        }
    );
    let _ = writeln!(
        out,
        "- Discord RPC (named pipe): {}",
        report
            .discord_rpc_pipe
            .as_deref()
            .unwrap_or("não detectado")
    );
    if let Some(ref err) = report.smtc_error {
        let _ = writeln!(out, "- SMTC (mídia Windows): erro ({err})");
    } else if report.smtc_sessions.is_empty() {
        let _ = writeln!(
            out,
            "- SMTC (mídia Windows): disponível, nenhuma sessão de mídia ativa no momento"
        );
    } else {
        let _ = writeln!(
            out,
            "- SMTC (mídia Windows): {} sessão(ões) ativa(s):",
            report.smtc_sessions.len()
        );
        for s in &report.smtc_sessions {
            let _ = writeln!(
                out,
                "    * [{}] app='{}' | título='{}' | artista='{}' | álbum='{}' | capa={}",
                s.playback_status,
                s.app_id,
                s.title,
                s.artist,
                s.album,
                if s.has_thumbnail { "sim" } else { "não" }
            );
        }
    }

    let _ = writeln!(out, "\n## 2. Aplicativos Relevantes Detectados");
    if report.highlighted_apps.is_empty() {
        let _ = writeln!(out, "(nenhum aplicativo da lista conhecida detectado)");
    } else {
        for app in &report.highlighted_apps {
            let status = if app.running { "RODANDO" } else { "instalado" };
            let _ = writeln!(
                out,
                "- [{}] {} ({}) | API: {} | Local: {}",
                app.category, app.name, status, app.known_api, app.source
            );
        }
    }

    let _ = writeln!(
        out,
        "\n## 3. Processos em Execução ({} únicos)",
        report.running_processes.len()
    );
    for proc_name in &report.running_processes {
        let _ = writeln!(out, "  - {proc_name}");
    }

    let _ = writeln!(out, "\n## 4. Diretórios de Aplicativos Instalados");
    for (dir_label, entries) in &report.installed_dirs {
        let _ = writeln!(out, "### {dir_label} ({} itens)", entries.len());
        for entry in entries {
            let _ = writeln!(out, "  - {entry}");
        }
    }

    out
}

pub fn default_scan_report_path() -> PathBuf {
    if Path::new("CONTEXT.md").exists() {
        return PathBuf::from("scan_report.txt");
    }
    if Path::new("../CONTEXT.md").exists() {
        return PathBuf::from("../scan_report.txt");
    }
    PathBuf::from("scan_report.txt")
}

pub fn run_scan_command() -> Result<PathBuf, String> {
    let report = collect_scan_report();
    let text = format_scan_report(&report);
    let path = default_scan_report_path();
    fs::write(&path, &text)
        .map_err(|err| format!("falha ao gravar '{}': {err}", path.display()))?;
    print!("{text}");
    println!("\nRelatório salvo em: {}", path.display());
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::{format_scan_report, match_known_apps, parse_tasklist_csv, ScanReport};

    #[test]
    fn parses_windows_tasklist_csv_deduplicated() {
        let sample = "\"Code.exe\",\"1234\",\"Console\",\"1\",\"120,000 K\"\r\n\
                      \"chrome.exe\",\"5678\",\"Console\",\"1\",\"90,000 K\"\r\n\
                      \"chrome.exe\",\"9999\",\"Console\",\"1\",\"45,000 K\"\r\n\
                      \"Spotify.exe\",\"2222\",\"Console\",\"1\",\"80,000 K\"\r\n";
        let procs = parse_tasklist_csv(sample);
        assert_eq!(procs, vec!["chrome.exe", "Code.exe", "Spotify.exe"]);
    }

    #[test]
    fn highlights_known_apps_from_processes_and_install_dirs() {
        let procs = vec!["chrome.exe".to_string(), "Code.exe".to_string()];
        let dirs = vec![(
            "C:\\Program Files".to_string(),
            vec!["obs-studio".to_string(), "Mozilla Firefox".to_string()],
        )];
        let matched = match_known_apps(&procs, &dirs);
        assert!(matched
            .iter()
            .any(|a| a.name == "Google Chrome" && a.running));
        assert!(matched
            .iter()
            .any(|a| a.name == "Visual Studio Code" && a.running));
        assert!(matched.iter().any(|a| a.name == "OBS Studio" && !a.running));
        assert!(matched
            .iter()
            .any(|a| a.name == "Mozilla Firefox" && !a.running));
    }

    #[test]
    fn formats_scan_report_sections() {
        let report = ScanReport {
            os: "windows (x86_64)".to_string(),
            running_processes: vec!["chrome.exe".to_string()],
            installed_dirs: vec![("C:\\Program Files".to_string(), vec!["Git".to_string()])],
            highlighted_apps: Vec::new(),
            smtc_sessions: Vec::new(),
            smtc_error: None,
            obs_websocket_open: false,
            discord_rpc_pipe: Some(r"\\.\pipe\discord-ipc-0".to_string()),
        };
        let rendered = format_scan_report(&report);
        assert!(rendered.contains("KnobContext System Scan Report"));
        assert!(rendered.contains("OBS WebSocket (127.0.0.1:4455): fechada"));
        assert!(rendered.contains(r"\\.\pipe\discord-ipc-0"));
    }
}
