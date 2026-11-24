use crate::providers::{ArgsSchema, Provider, ProviderArgs, ProviderCommand, ProviderError};
use std::sync::Mutex;
use zbus::blocking::{Connection, Proxy};

const MPRIS_BUS_PREFIX: &str = "org.mpris.MediaPlayer2.";
const MPRIS_OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";
const MPRIS_PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";
const DEFAULT_MPRIS_VOLUME_STEP_PERCENT: f64 = 5.0;

/// Converte o argumento opcional `step` (1..=50%, padrão 5%) em fração `0.01..=0.50`.
pub fn parse_volume_step_fraction(
    args: &ProviderArgs,
    command: &str,
) -> Result<f64, ProviderError> {
    let Some(raw) = args.get("step") else {
        return Ok(DEFAULT_MPRIS_VOLUME_STEP_PERCENT / 100.0);
    };
    let step = raw.parse::<f64>().map_err(|_| ProviderError::InvalidArg {
        provider: "mpris".to_string(),
        command: command.to_string(),
        arg: "step".to_string(),
        reason: format!("esperado número em % (ex.: 5), recebido '{raw}'"),
    })?;
    if !step.is_finite() || step <= 0.0 || step > 100.0 {
        return Err(ProviderError::InvalidArg {
            provider: "mpris".to_string(),
            command: command.to_string(),
            arg: "step".to_string(),
            reason: format!("passo deve estar entre 1 e 100% (recebido '{raw}')"),
        });
    }
    Ok(step / 100.0)
}

/// Converte o argumento obrigatório `percent` (0..=100) em fração MPRIS `0.0..=1.0`.
pub fn parse_volume_percent_fraction(args: &ProviderArgs) -> Result<f64, ProviderError> {
    let raw = args
        .get("percent")
        .ok_or_else(|| ProviderError::MissingArg {
            provider: "mpris".to_string(),
            command: "set_volume".to_string(),
            arg: "percent".to_string(),
        })?;
    let pct = raw.parse::<f64>().map_err(|_| ProviderError::InvalidArg {
        provider: "mpris".to_string(),
        command: "set_volume".to_string(),
        arg: "percent".to_string(),
        reason: format!("esperado número de 0 a 100, recebido '{raw}'"),
    })?;
    if !pct.is_finite() || !(0.0..=100.0).contains(&pct) {
        return Err(ProviderError::InvalidArg {
            provider: "mpris".to_string(),
            command: "set_volume".to_string(),
            arg: "percent".to_string(),
            reason: format!("volume deve estar entre 0 e 100% (recebido '{raw}')"),
        });
    }
    Ok((pct / 100.0).clamp(0.0, 1.0))
}

/// Converte o argumento `seconds` (ex.: `"10"`, `"-5"`, `"2.5"`) para microssegundos (`i64`)
/// conforme a especificação D-Bus `org.mpris.MediaPlayer2.Player.Seek(x: Offset)`.
pub fn parse_seek_microseconds(args: &ProviderArgs) -> Result<i64, ProviderError> {
    let raw = args
        .get("seconds")
        .ok_or_else(|| ProviderError::MissingArg {
            provider: "mpris".to_string(),
            command: "seek".to_string(),
            arg: "seconds".to_string(),
        })?;
    let seconds = raw.parse::<f64>().map_err(|_| ProviderError::InvalidArg {
        provider: "mpris".to_string(),
        command: "seek".to_string(),
        arg: "seconds".to_string(),
        reason: format!("esperado número em segundos (ex.: 10 ou -5), recebido '{raw}'"),
    })?;
    if !seconds.is_finite() || seconds.abs() > 86_400.0 {
        return Err(ProviderError::InvalidArg {
            provider: "mpris".to_string(),
            command: "seek".to_string(),
            arg: "seconds".to_string(),
            reason: format!("valor fora do intervalo permitido: '{raw}'"),
        });
    }
    Ok((seconds * 1_000_000.0).round() as i64)
}

/// Extrai o nome amigável do player a partir do bus name MPRIS
/// (ex.: `"org.mpris.MediaPlayer2.firefox.instance_1_2"` → `"firefox"`).
pub fn short_player_name(bus_name: &str) -> String {
    let trimmed = bus_name.strip_prefix(MPRIS_BUS_PREFIX).unwrap_or(bus_name);
    trimmed
        .split('.')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(trimmed)
        .to_string()
}

/// Seleciona o player preferencial a partir de pares `(bus_name, playback_status)`:
/// prioriza `"Playing"`, depois `"Paused"`, e por fim o primeiro disponível.
pub fn select_preferred_player(candidates: &[(String, String)]) -> Option<String> {
    candidates
        .iter()
        .find(|(_, st)| st.eq_ignore_ascii_case("Playing"))
        .or_else(|| {
            candidates
                .iter()
                .find(|(_, st)| st.eq_ignore_ascii_case("Paused"))
        })
        .or_else(|| candidates.first())
        .map(|(name, _)| name.clone())
}

/// Provider de integração direta com players de mídia compatíveis com **MPRIS** (D-Bus).
///
/// Funciona com Spotify, Firefox, Chromium, mpv, VLC e qualquer player Linux com
/// `org.mpris.MediaPlayer2.Player`, usando `zbus::blocking` (conexão preguiçosa).
pub struct MprisProvider {
    conn: Mutex<Option<Connection>>,
}

impl Default for MprisProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl MprisProvider {
    pub fn new() -> Self {
        Self {
            conn: Mutex::new(None),
        }
    }

    fn with_connection<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, ProviderError>,
    ) -> Result<T, ProviderError> {
        let mut guard = self.conn.lock().map_err(|_| {
            ProviderError::Unavailable("mpris".to_string(), "lock interno corrompido".to_string())
        })?;
        if guard.is_none() {
            let conn = Connection::session().map_err(|err| {
                ProviderError::Unavailable(
                    "mpris".to_string(),
                    format!("falha ao conectar ao D-Bus de sessão: {err}"),
                )
            })?;
            *guard = Some(conn);
        }
        let Some(ref conn) = *guard else {
            return Err(ProviderError::Unavailable(
                "mpris".to_string(),
                "conexão D-Bus indisponível".to_string(),
            ));
        };
        match f(conn) {
            Ok(val) => Ok(val),
            Err(err) => {
                if matches!(err, ProviderError::Unavailable(_, _)) {
                    *guard = None;
                }
                Err(err)
            }
        }
    }

    fn list_mpris_players(conn: &Connection) -> Result<Vec<String>, ProviderError> {
        let dbus = Proxy::new(
            conn,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
        )
        .map_err(|err| {
            ProviderError::Unavailable("mpris".to_string(), format!("proxy D-Bus: {err}"))
        })?;
        let names: Vec<String> = dbus.call("ListNames", &()).map_err(|err| {
            ProviderError::Unavailable("mpris".to_string(), format!("ListNames falhou: {err}"))
        })?;
        Ok(names
            .into_iter()
            .filter(|n| n.starts_with(MPRIS_BUS_PREFIX))
            .collect())
    }

    fn playback_status(conn: &Connection, bus_name: &str) -> String {
        let Ok(proxy) = Proxy::new(conn, bus_name, MPRIS_OBJECT_PATH, MPRIS_PLAYER_IFACE) else {
            return String::new();
        };
        proxy
            .get_property::<String>("PlaybackStatus")
            .unwrap_or_default()
    }

    fn resolve_target_player(conn: &Connection) -> Result<String, ProviderError> {
        let names = Self::list_mpris_players(conn)?;
        if names.is_empty() {
            return Err(ProviderError::Unavailable(
                "mpris".to_string(),
                "nenhum player de mídia MPRIS em execução".to_string(),
            ));
        }
        let candidates: Vec<(String, String)> = names
            .into_iter()
            .map(|name| {
                let status = Self::playback_status(conn, &name);
                (name, status)
            })
            .collect();
        select_preferred_player(&candidates).ok_or_else(|| {
            ProviderError::Unavailable(
                "mpris".to_string(),
                "nenhum player de mídia MPRIS disponível".to_string(),
            )
        })
    }
}

impl Provider for MprisProvider {
    fn id(&self) -> &'static str {
        "mpris"
    }

    fn name(&self) -> &'static str {
        "Mídia (MPRIS)"
    }

    fn is_available(&self) -> bool {
        self.with_connection(|conn| {
            let players = Self::list_mpris_players(conn)?;
            Ok(!players.is_empty())
        })
        .unwrap_or(false)
    }

    fn commands(&self) -> Vec<ProviderCommand> {
        vec![
            ProviderCommand {
                id: "play_pause",
                name: "Play / Pause",
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
                id: "stop",
                name: "Parar reprodução",
                args_schema: ArgsSchema::empty(),
            },
            ProviderCommand {
                id: "seek",
                name: "Avançar / retroceder (segundos)",
                args_schema: ArgsSchema::single(
                    "seconds",
                    "Segundos (negativo retrocede)",
                    "ex: 10 ou -10",
                    true,
                ),
            },
            ProviderCommand {
                id: "volume_up",
                name: "Aumentar volume do player",
                args_schema: ArgsSchema::single(
                    "step",
                    "Passo % (opcional, padrão 5)",
                    "ex: 5",
                    false,
                ),
            },
            ProviderCommand {
                id: "volume_down",
                name: "Diminuir volume do player",
                args_schema: ArgsSchema::single(
                    "step",
                    "Passo % (opcional, padrão 5)",
                    "ex: 5",
                    false,
                ),
            },
            ProviderCommand {
                id: "set_volume",
                name: "Definir volume do player (%)",
                args_schema: ArgsSchema::single("percent", "Volume (0 a 100%)", "ex: 50", true),
            },
        ]
    }

    fn execute(&self, command_id: &str, args: &ProviderArgs) -> Result<(), ProviderError> {
        let cmd = command_id.trim().to_ascii_lowercase();
        let seek_offset = if cmd == "seek" {
            Some(parse_seek_microseconds(args)?)
        } else {
            None
        };
        let vol_step = if cmd == "volume_up" || cmd == "volume_down" {
            Some(parse_volume_step_fraction(args, &cmd)?)
        } else {
            None
        };
        let target_vol = if cmd == "set_volume" {
            Some(parse_volume_percent_fraction(args)?)
        } else {
            None
        };

        self.with_connection(|conn| {
            let target = Self::resolve_target_player(conn)?;
            let proxy = Proxy::new(conn, target.as_str(), MPRIS_OBJECT_PATH, MPRIS_PLAYER_IFACE)
                .map_err(|err| ProviderError::CommandFailed {
                    provider: "mpris".to_string(),
                    command: cmd.clone(),
                    reason: err.to_string(),
                })?;

            let call_res: zbus::Result<()> = match cmd.as_str() {
                "play_pause" => proxy.call("PlayPause", &()),
                "next" => proxy.call("Next", &()),
                "previous" => proxy.call("Previous", &()),
                "stop" => proxy.call("Stop", &()),
                "seek" => {
                    let offset = seek_offset.unwrap_or(0);
                    proxy.call("Seek", &(offset,))
                }
                "volume_up" | "volume_down" => {
                    let current = proxy.get_property::<f64>("Volume").map_err(|err| {
                        ProviderError::CommandFailed {
                            provider: "mpris".to_string(),
                            command: cmd.clone(),
                            reason: format!(
                                "player '{target}' não suporta leitura de Volume MPRIS: {err}"
                            ),
                        }
                    })?;
                    let delta = vol_step.unwrap_or(DEFAULT_MPRIS_VOLUME_STEP_PERCENT / 100.0);
                    let next = if cmd == "volume_up" {
                        (current + delta).clamp(0.0, 1.0)
                    } else {
                        (current - delta).clamp(0.0, 1.0)
                    };
                    proxy.set_property("Volume", next).map_err(|err| {
                        ProviderError::CommandFailed {
                            provider: "mpris".to_string(),
                            command: cmd.clone(),
                            reason: format!(
                                "player '{target}' não suporta escrita de Volume MPRIS: {err}"
                            ),
                        }
                    })?;
                    Ok(())
                }
                "set_volume" => {
                    let next = target_vol.unwrap_or(0.5);
                    proxy.set_property("Volume", next).map_err(|err| {
                        ProviderError::CommandFailed {
                            provider: "mpris".to_string(),
                            command: cmd.clone(),
                            reason: format!(
                                "player '{target}' não suporta escrita de Volume MPRIS: {err}"
                            ),
                        }
                    })?;
                    Ok(())
                }
                other => {
                    return Err(ProviderError::UnknownCommand {
                        provider: "mpris".to_string(),
                        command: other.to_string(),
                    });
                }
            };

            call_res.map_err(|err| ProviderError::CommandFailed {
                provider: "mpris".to_string(),
                command: cmd,
                reason: err.to_string(),
            })
        })
    }

    fn test_connection(&self) -> Result<String, ProviderError> {
        self.with_connection(|conn| {
            let players = Self::list_mpris_players(conn)?;
            if players.is_empty() {
                return Err(ProviderError::Unavailable(
                    "mpris".to_string(),
                    "nenhum player MPRIS rodando no momento".to_string(),
                ));
            }
            let names: Vec<String> = players.iter().map(|p| short_player_name(p)).collect();
            Ok(format!("Players detectados: {}", names.join(", ")))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_seek_microseconds, parse_volume_percent_fraction, parse_volume_step_fraction,
        select_preferred_player, short_player_name, MprisProvider,
    };
    use crate::providers::{Provider, ProviderArgs, ProviderError};

    #[test]
    fn parses_mpris_volume_step_and_percent_arguments() {
        // Sem step -> padrão 5% (0.05)
        assert_eq!(
            parse_volume_step_fraction(&ProviderArgs::default(), "volume_up").ok(),
            Some(0.05)
        );
        let mut step_args = ProviderArgs::default();
        step_args.values.insert("step".into(), "10".into());
        assert_eq!(
            parse_volume_step_fraction(&step_args, "volume_down").ok(),
            Some(0.10)
        );

        let mut bad_step = ProviderArgs::default();
        bad_step.values.insert("step".into(), "0".into());
        assert!(matches!(
            parse_volume_step_fraction(&bad_step, "volume_up"),
            Err(ProviderError::InvalidArg { .. })
        ));

        // set_volume exige percent 0..=100
        assert!(matches!(
            parse_volume_percent_fraction(&ProviderArgs::default()),
            Err(ProviderError::MissingArg { .. })
        ));
        let mut pct_args = ProviderArgs::default();
        pct_args.values.insert("percent".into(), "75".into());
        assert_eq!(parse_volume_percent_fraction(&pct_args).ok(), Some(0.75));
    }

    #[test]
    fn parses_seek_seconds_and_rejects_invalid_values() {
        let mut ok_pos = ProviderArgs::default();
        ok_pos.values.insert("seconds".into(), "10".into());
        assert_eq!(parse_seek_microseconds(&ok_pos).ok(), Some(10_000_000));

        let mut ok_neg = ProviderArgs::default();
        ok_neg.values.insert("seconds".into(), "-5.5".into());
        assert_eq!(parse_seek_microseconds(&ok_neg).ok(), Some(-5_500_000));

        assert!(matches!(
            parse_seek_microseconds(&ProviderArgs::default()),
            Err(ProviderError::MissingArg { .. })
        ));

        let mut invalid = ProviderArgs::default();
        invalid.values.insert("seconds".into(), "abc".into());
        assert!(matches!(
            parse_seek_microseconds(&invalid),
            Err(ProviderError::InvalidArg { .. })
        ));
    }

    #[test]
    fn selects_playing_player_before_paused_and_formats_names() {
        let candidates = vec![
            (
                "org.mpris.MediaPlayer2.vlc".to_string(),
                "Paused".to_string(),
            ),
            (
                "org.mpris.MediaPlayer2.firefox.instance_1_2".to_string(),
                "Playing".to_string(),
            ),
        ];
        assert_eq!(
            select_preferred_player(&candidates).as_deref(),
            Some("org.mpris.MediaPlayer2.firefox.instance_1_2")
        );
        assert_eq!(
            short_player_name("org.mpris.MediaPlayer2.firefox.instance_1_2"),
            "firefox"
        );
        assert_eq!(
            short_player_name("org.mpris.MediaPlayer2.spotify"),
            "spotify"
        );
    }

    #[test]
    #[ignore = "requer sessão D-Bus ativa com um player MPRIS em execução"]
    fn integration_mpris_test_connection() {
        let provider = MprisProvider::new();
        let status = provider
            .test_connection()
            .expect("esperava player MPRIS rodando na sessão");
        assert!(!status.is_empty());
    }
}
