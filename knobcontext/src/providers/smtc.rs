use crate::config::SmtcProviderConfig;
use crate::providers::{ArgsSchema, Provider, ProviderArgs, ProviderCommand, ProviderError};
use image::imageops::FilterType;
use image::GenericImageView;

const SMTC_THUMBNAIL_SIZE: u32 = 96;

/// Imagem de capa de álbum já decodificada em RGBA8 para exibição direta no Hub / Mini-HUD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtcThumbnail {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Snapshot imutável de uma sessão SMTC do Windows (ex.: Spotify, Firefox/YouTube, VLC).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtcSessionSnapshot {
    pub app_id: String,
    pub app_label: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub is_playing: bool,
    pub playback_status: String,
    /// `None` é tratado como caso normal (ex.: Firefox/YouTube frequentemente não expõe thumbnail).
    pub thumbnail: Option<SmtcThumbnail>,
}

/// Snapshot agregado retornado ao Hub e ao Mini-HUD pelo `SystemWorker`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SmtcHubSnapshot {
    pub session: Option<SmtcSessionSnapshot>,
    pub active_sessions_count: usize,
}

/// Comandos suportados pelo provider SMTC.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SmtcCommand {
    PlayPause,
    Play,
    Pause,
    Next,
    Previous,
    Seek { seconds: f64 },
}

/// Faz o parsing de `command_id` + `args` para `SmtcCommand`.
pub fn parse_smtc_command(
    command_id: &str,
    args: &ProviderArgs,
) -> Result<SmtcCommand, ProviderError> {
    match command_id.trim().to_ascii_lowercase().as_str() {
        "play_pause" => Ok(SmtcCommand::PlayPause),
        "play" => Ok(SmtcCommand::Play),
        "pause" => Ok(SmtcCommand::Pause),
        "next" => Ok(SmtcCommand::Next),
        "previous" => Ok(SmtcCommand::Previous),
        "seek" => {
            let raw = args
                .get("seconds")
                .ok_or_else(|| ProviderError::MissingArg {
                    provider: "smtc".to_string(),
                    command: "seek".to_string(),
                    arg: "seconds".to_string(),
                })?;
            let seconds = raw.parse::<f64>().map_err(|_| ProviderError::InvalidArg {
                provider: "smtc".to_string(),
                command: "seek".to_string(),
                arg: "seconds".to_string(),
                reason: format!("esperado número em segundos (ex.: 10 ou -10), recebido '{raw}'"),
            })?;
            if !seconds.is_finite() || seconds.abs() > 86_400.0 {
                return Err(ProviderError::InvalidArg {
                    provider: "smtc".to_string(),
                    command: "seek".to_string(),
                    arg: "seconds".to_string(),
                    reason: format!("valor fora do intervalo permitido: '{raw}'"),
                });
            }
            Ok(SmtcCommand::Seek { seconds })
        }
        other => Err(ProviderError::UnknownCommand {
            provider: "smtc".to_string(),
            command: other.to_string(),
        }),
    }
}

/// Formata o `SourceAppUserModelId` do Windows em um rótulo amigável para a UI.
pub fn format_smtc_app_label(app_id: &str) -> String {
    let trimmed = app_id.trim();
    if trimmed.is_empty() {
        return "Mídia".to_string();
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower == "308046b0af4a39cb" || lower.contains("firefox") {
        return "Firefox".to_string();
    }
    if lower.contains("spotify") {
        return "Spotify".to_string();
    }
    if lower.contains("chrome") {
        return "Chrome".to_string();
    }
    if lower.contains("msedge") || lower.contains("edge") {
        return "Edge".to_string();
    }
    if lower.contains("vlc") {
        return "VLC".to_string();
    }
    if lower.contains("zunemusic") || lower.contains("mediaplayer") {
        return "Media Player".to_string();
    }

    let candidate = trimmed
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(trimmed)
        .split('!')
        .next()
        .unwrap_or(trimmed);
    let clean = candidate
        .strip_suffix(".exe")
        .or_else(|| candidate.strip_suffix(".EXE"))
        .unwrap_or(candidate)
        .split('_')
        .next()
        .unwrap_or(candidate);

    let mut chars = clean.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
        None => "Mídia".to_string(),
    }
}

/// Decodifica bytes de imagem (PNG/JPEG/WebP/BMP) vindos do thumbnail SMTC em RGBA8 redimensionado.
///
/// Retorna `None` de forma segura caso os bytes estejam vazios ou em formato não suportado.
pub fn decode_thumbnail_bytes(raw: &[u8], target_size: u32) -> Option<SmtcThumbnail> {
    if raw.is_empty() || target_size == 0 {
        return None;
    }
    let img = image::load_from_memory(raw).ok()?;
    let resized = img.resize_to_fill(target_size, target_size, FilterType::Triangle);
    let (width, height) = resized.dimensions();
    Some(SmtcThumbnail {
        width,
        height,
        rgba: resized.to_rgba8().into_raw(),
    })
}

/// Seleciona a sessão SMTC prioritária: `Playing` → `Paused` → primeira disponível.
pub fn select_best_session(sessions: &[SmtcSessionSnapshot]) -> Option<usize> {
    if sessions.is_empty() {
        return None;
    }
    sessions
        .iter()
        .position(|s| s.is_playing)
        .or_else(|| {
            sessions
                .iter()
                .position(|s| s.playback_status.eq_ignore_ascii_case("Paused"))
        })
        .or(Some(0))
}

/// Constrói o subtítulo legível de uma sessão SMTC (priorizando Artista, depois Álbum, depois App).
pub fn format_session_subtitle(session: &SmtcSessionSnapshot) -> String {
    match (session.artist.trim(), session.album.trim()) {
        (artist, album) if !artist.is_empty() && !album.is_empty() => {
            format!("{artist} • {album}")
        }
        (artist, _) if !artist.is_empty() => artist.to_string(),
        (_, album) if !album.is_empty() => album.to_string(),
        _ => session.app_label.clone(),
    }
}

/// Provider nativo Windows para controle de mídia via SMTC (System Media Transport Controls).
pub struct SmtcProvider {
    config: SmtcProviderConfig,
}

impl SmtcProvider {
    pub fn new(config: SmtcProviderConfig) -> Self {
        Self { config }
    }
}

impl Provider for SmtcProvider {
    fn id(&self) -> &'static str {
        "smtc"
    }

    fn name(&self) -> &'static str {
        "Mídia Windows (SMTC)"
    }

    fn is_available(&self) -> bool {
        if !self.config.enabled {
            return false;
        }
        let snap = fetch_smtc_hub_snapshot();
        snap.session.is_some()
    }

    fn commands(&self) -> Vec<ProviderCommand> {
        vec![
            ProviderCommand {
                id: "play_pause",
                name: "Play / Pause",
                args_schema: ArgsSchema::empty(),
            },
            ProviderCommand {
                id: "play",
                name: "Reproduzir",
                args_schema: ArgsSchema::empty(),
            },
            ProviderCommand {
                id: "pause",
                name: "Pausar",
                args_schema: ArgsSchema::empty(),
            },
            ProviderCommand {
                id: "next",
                name: "Próxima faixa",
                args_schema: ArgsSchema::empty(),
            },
            ProviderCommand {
                id: "previous",
                name: "Faixa anterior",
                args_schema: ArgsSchema::empty(),
            },
            ProviderCommand {
                id: "seek",
                name: "Avançar / Retroceder (segundos)",
                args_schema: ArgsSchema::single("seconds", "Segundos (+/-)", "ex: 10 ou -10", true),
            },
        ]
    }

    fn execute(&self, command_id: &str, args: &ProviderArgs) -> Result<(), ProviderError> {
        if !self.config.enabled {
            return Err(ProviderError::Unavailable(
                "smtc".to_string(),
                "provider SMTC desativado em [providers.smtc]".to_string(),
            ));
        }
        let cmd = parse_smtc_command(command_id, args)?;
        execute_platform_smtc_command(cmd)
    }

    fn test_connection(&self) -> Result<String, ProviderError> {
        if !self.config.enabled {
            return Err(ProviderError::Unavailable(
                "smtc".to_string(),
                "desativado nas configurações".to_string(),
            ));
        }
        let snap = fetch_smtc_hub_snapshot();
        match snap.session {
            Some(s) => {
                let title = if s.title.trim().is_empty() {
                    "Sem título"
                } else {
                    s.title.trim()
                };
                let cover = if s.thumbnail.is_some() {
                    "com capa"
                } else {
                    "sem capa"
                };
                Ok(format!(
                    "{} ({}) — {} [{}]",
                    s.app_label, s.playback_status, title, cover
                ))
            }
            None => Err(ProviderError::Unavailable(
                "smtc".to_string(),
                "nenhuma sessão de mídia SMTC ativa no momento".to_string(),
            )),
        }
    }
}

/// Consulta assíncrona (chamada a partir do `SystemWorker`) retornando a sessão SMTC ativa com capa opcional.
pub fn fetch_smtc_hub_snapshot() -> SmtcHubSnapshot {
    #[cfg(target_os = "windows")]
    {
        winrt_impl::fetch_hub_snapshot()
    }
    #[cfg(not(target_os = "windows"))]
    {
        SmtcHubSnapshot::default()
    }
}

#[cfg(target_os = "windows")]
fn execute_platform_smtc_command(cmd: SmtcCommand) -> Result<(), ProviderError> {
    winrt_impl::execute_command(cmd)
}

#[cfg(not(target_os = "windows"))]
fn execute_platform_smtc_command(_cmd: SmtcCommand) -> Result<(), ProviderError> {
    Err(ProviderError::Unavailable(
        "smtc".to_string(),
        "SMTC disponível apenas no Windows".to_string(),
    ))
}

#[cfg(target_os = "windows")]
mod winrt_impl {
    use super::{
        decode_thumbnail_bytes, format_smtc_app_label, select_best_session, SmtcCommand,
        SmtcHubSnapshot, SmtcSessionSnapshot, SmtcThumbnail, SMTC_THUMBNAIL_SIZE,
    };
    use crate::providers::ProviderError;
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession as SmtcSession,
        GlobalSystemMediaTransportControlsSessionManager as SmtcManager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
    };
    use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    fn ensure_com() {
        // SAFETY: inicialização COM padrão para chamadas WinRT na thread atual.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    }

    fn read_thumbnail(thumb_ref: &IRandomAccessStreamReference) -> Option<SmtcThumbnail> {
        let stream = thumb_ref.OpenReadAsync().ok()?.join().ok()?;
        let size = stream.Size().ok()?;
        if size == 0 || size > 10 * 1024 * 1024 {
            return None;
        }
        let reader = DataReader::CreateDataReader(&stream).ok()?;
        let loaded = reader.LoadAsync(size as u32).ok()?.join().ok()?;
        if loaded == 0 {
            return None;
        }
        let mut buf = vec![0u8; loaded as usize];
        reader.ReadBytes(&mut buf).ok()?;
        decode_thumbnail_bytes(&buf, SMTC_THUMBNAIL_SIZE)
    }

    fn snapshot_session(session: &SmtcSession, with_thumbnail: bool) -> SmtcSessionSnapshot {
        let app_id = session
            .SourceAppUserModelId()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let app_label = format_smtc_app_label(&app_id);

        let status = session
            .GetPlaybackInfo()
            .and_then(|info| info.PlaybackStatus())
            .ok();
        let is_playing = matches!(status, Some(PlaybackStatus::Playing));
        let playback_status = match status {
            Some(PlaybackStatus::Playing) => "Playing",
            Some(PlaybackStatus::Paused) => "Paused",
            Some(PlaybackStatus::Stopped) => "Stopped",
            Some(PlaybackStatus::Opened) => "Opened",
            Some(PlaybackStatus::Changing) => "Changing",
            Some(PlaybackStatus::Closed) => "Closed",
            _ => "Unknown",
        }
        .to_string();

        let (title, artist, album, thumbnail) = session
            .TryGetMediaPropertiesAsync()
            .and_then(|op| op.join())
            .map(|props| {
                let title = props.Title().map(|s| s.to_string()).unwrap_or_default();
                let artist = props.Artist().map(|s| s.to_string()).unwrap_or_default();
                let album = props
                    .AlbumTitle()
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                // Thumbnail ausente é caso normal (ex.: Firefox/YouTube) — nunca falha.
                let thumb = if with_thumbnail {
                    props.Thumbnail().ok().and_then(|r| read_thumbnail(&r))
                } else {
                    None
                };
                (title, artist, album, thumb)
            })
            .unwrap_or_default();

        SmtcSessionSnapshot {
            app_id,
            app_label,
            title,
            artist,
            album,
            is_playing,
            playback_status,
            thumbnail,
        }
    }

    fn list_raw_sessions() -> Result<Vec<SmtcSession>, ProviderError> {
        ensure_com();
        let manager = SmtcManager::RequestAsync()
            .and_then(|op| op.join())
            .map_err(|err| ProviderError::Unavailable("smtc".to_string(), err.to_string()))?;
        let sessions = manager
            .GetSessions()
            .map_err(|err| ProviderError::Unavailable("smtc".to_string(), err.to_string()))?;
        let count = sessions.Size().unwrap_or(0);
        let mut out = Vec::with_capacity(count as usize);
        for idx in 0..count {
            if let Ok(s) = sessions.GetAt(idx) {
                out.push(s);
            }
        }
        Ok(out)
    }

    pub(super) fn fetch_hub_snapshot() -> SmtcHubSnapshot {
        let Ok(raw_sessions) = list_raw_sessions() else {
            return SmtcHubSnapshot::default();
        };
        if raw_sessions.is_empty() {
            return SmtcHubSnapshot::default();
        }
        let light: Vec<SmtcSessionSnapshot> = raw_sessions
            .iter()
            .map(|s| snapshot_session(s, false))
            .collect();
        let active_sessions_count = light.len();
        let best_idx = select_best_session(&light);
        let session = best_idx.map(|idx| snapshot_session(&raw_sessions[idx], true));
        SmtcHubSnapshot {
            session,
            active_sessions_count,
        }
    }

    pub(super) fn execute_command(cmd: SmtcCommand) -> Result<(), ProviderError> {
        let raw_sessions = list_raw_sessions()?;
        if raw_sessions.is_empty() {
            return Err(ProviderError::Unavailable(
                "smtc".to_string(),
                "nenhuma sessão SMTC ativa".to_string(),
            ));
        }
        let light: Vec<SmtcSessionSnapshot> = raw_sessions
            .iter()
            .map(|s| snapshot_session(s, false))
            .collect();
        let idx = select_best_session(&light).unwrap_or(0);
        let session = &raw_sessions[idx];

        let map_err = |name: &str, err: windows::core::Error| ProviderError::CommandFailed {
            provider: "smtc".to_string(),
            command: name.to_string(),
            reason: err.to_string(),
        };

        match cmd {
            SmtcCommand::PlayPause => {
                let _ = session
                    .TryTogglePlayPauseAsync()
                    .and_then(|op| op.join())
                    .map_err(|e| map_err("play_pause", e))?;
            }
            SmtcCommand::Play => {
                let _ = session
                    .TryPlayAsync()
                    .and_then(|op| op.join())
                    .map_err(|e| map_err("play", e))?;
            }
            SmtcCommand::Pause => {
                let _ = session
                    .TryPauseAsync()
                    .and_then(|op| op.join())
                    .map_err(|e| map_err("pause", e))?;
            }
            SmtcCommand::Next => {
                let _ = session
                    .TrySkipNextAsync()
                    .and_then(|op| op.join())
                    .map_err(|e| map_err("next", e))?;
            }
            SmtcCommand::Previous => {
                let _ = session
                    .TrySkipPreviousAsync()
                    .and_then(|op| op.join())
                    .map_err(|e| map_err("previous", e))?;
            }
            SmtcCommand::Seek { seconds } => {
                let timeline = session
                    .GetTimelineProperties()
                    .map_err(|e| map_err("seek", e))?;
                let current_ticks = timeline
                    .Position()
                    .map_err(|e| map_err("seek", e))?
                    .Duration;
                let delta_ticks = (seconds * 10_000_000.0).round() as i64;
                let target = current_ticks.saturating_add(delta_ticks).max(0);
                let _ = session
                    .TryChangePlaybackPositionAsync(target)
                    .and_then(|op| op.join())
                    .map_err(|e| map_err("seek", e))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_thumbnail_bytes, format_session_subtitle, format_smtc_app_label, parse_smtc_command,
        select_best_session, SmtcCommand, SmtcProvider, SmtcSessionSnapshot, SmtcThumbnail,
    };
    use crate::config::SmtcProviderConfig;
    use crate::providers::{Provider, ProviderArgs, ProviderError};
    use image::{ImageBuffer, Rgba};
    use std::io::Cursor;

    fn sample_png_bytes() -> Vec<u8> {
        let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
            ImageBuffer::from_pixel(16, 16, Rgba([59, 130, 246, 255]));
        let mut cursor = Cursor::new(Vec::new());
        img.write_to(&mut cursor, image::ImageFormat::Png)
            .expect("png válido");
        cursor.into_inner()
    }

    #[test]
    fn parses_all_smtc_commands_and_validates_seek_args() {
        let empty = ProviderArgs::default();
        assert_eq!(
            parse_smtc_command("play_pause", &empty).expect("play_pause"),
            SmtcCommand::PlayPause
        );
        assert_eq!(
            parse_smtc_command("play", &empty).expect("play"),
            SmtcCommand::Play
        );
        assert_eq!(
            parse_smtc_command("pause", &empty).expect("pause"),
            SmtcCommand::Pause
        );
        assert_eq!(
            parse_smtc_command("next", &empty).expect("next"),
            SmtcCommand::Next
        );
        assert_eq!(
            parse_smtc_command("previous", &empty).expect("previous"),
            SmtcCommand::Previous
        );

        let mut args = ProviderArgs::default();
        args.values.insert("seconds".to_string(), "-15".to_string());
        assert_eq!(
            parse_smtc_command("seek", &args).expect("seek"),
            SmtcCommand::Seek { seconds: -15.0 }
        );

        assert!(matches!(
            parse_smtc_command("seek", &empty),
            Err(ProviderError::MissingArg { .. })
        ));
        let mut bad = ProviderArgs::default();
        bad.values.insert("seconds".to_string(), "abc".to_string());
        assert!(matches!(
            parse_smtc_command("seek", &bad),
            Err(ProviderError::InvalidArg { .. })
        ));
        assert!(matches!(
            parse_smtc_command("desconhecido", &empty),
            Err(ProviderError::UnknownCommand { .. })
        ));
    }

    #[test]
    fn handles_complete_session_and_missing_thumbnail_gracefully() {
        let png = sample_png_bytes();
        let decoded = decode_thumbnail_bytes(&png, 64).expect("deve decodificar capa PNG");
        assert_eq!(decoded.width, 64);
        assert_eq!(decoded.height, 64);
        assert_eq!(decoded.rgba.len(), 64 * 64 * 4);

        // Bytes vazios ou inválidos retornam None sem panic
        assert!(decode_thumbnail_bytes(&[], 64).is_none());
        assert!(decode_thumbnail_bytes(b"nao-e-imagem", 64).is_none());

        let firefox_no_thumb = SmtcSessionSnapshot {
            app_id: "308046B0AF4A39CB".to_string(),
            app_label: format_smtc_app_label("308046B0AF4A39CB"),
            title: "YouTube".to_string(),
            artist: String::new(),
            album: String::new(),
            is_playing: false,
            playback_status: "Paused".to_string(),
            thumbnail: None,
        };
        assert_eq!(firefox_no_thumb.app_label, "Firefox");
        assert_eq!(format_session_subtitle(&firefox_no_thumb), "Firefox");
        assert!(firefox_no_thumb.thumbnail.is_none());

        let spotify_playing = SmtcSessionSnapshot {
            app_id: "Spotify.exe".to_string(),
            app_label: format_smtc_app_label("Spotify.exe"),
            title: "Midnight City".to_string(),
            artist: "M83".to_string(),
            album: "Hurry Up, We're Dreaming".to_string(),
            is_playing: true,
            playback_status: "Playing".to_string(),
            thumbnail: Some(SmtcThumbnail {
                width: 64,
                height: 64,
                rgba: decoded.rgba,
            }),
        };
        assert_eq!(spotify_playing.app_label, "Spotify");
        assert_eq!(
            format_session_subtitle(&spotify_playing),
            "M83 • Hurry Up, We're Dreaming"
        );

        let sessions = vec![firefox_no_thumb, spotify_playing];
        assert_eq!(select_best_session(&sessions), Some(1));
    }

    #[test]
    fn disabled_smtc_provider_reports_unavailable() {
        let provider = SmtcProvider::new(SmtcProviderConfig { enabled: false });
        assert!(!provider.is_available());
        assert!(matches!(
            provider.execute("play_pause", &ProviderArgs::default()),
            Err(ProviderError::Unavailable(_, _))
        ));
    }
}
