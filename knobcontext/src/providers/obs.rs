use crate::config::ObsProviderConfig;
use crate::providers::{ArgsSchema, Provider, ProviderArgs, ProviderCommand, ProviderError};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::{client, Message, WebSocket};

const OBS_CONNECT_TIMEOUT: Duration = Duration::from_millis(1_500);
const OBS_IO_TIMEOUT: Duration = Duration::from_millis(1_500);
const OBS_PROBE_TIMEOUT: Duration = Duration::from_millis(150);
const OBS_BACKOFF_INITIAL_MS: u64 = 250;
const OBS_BACKOFF_MAX_MS: u64 = 2_000;

/// Calcula a string de autenticação challenge-response conforme a spec oficial do obs-websocket 5.x:
/// `base64(sha256(base64(sha256(password + salt)) + challenge))`.
pub fn compute_obs_auth_response(password: &str, salt: &str, challenge: &str) -> String {
    let secret_hash = Sha256::digest(format!("{password}{salt}").as_bytes());
    let base64_secret = BASE64_STANDARD.encode(secret_hash);
    let auth_hash = Sha256::digest(format!("{base64_secret}{challenge}").as_bytes());
    BASE64_STANDARD.encode(auth_hash)
}

fn map_obs_request(
    command_id: &str,
    args: &ProviderArgs,
) -> Result<(&'static str, Option<Value>), ProviderError> {
    match command_id.trim().to_ascii_lowercase().as_str() {
        "toggle_record" => Ok(("ToggleRecord", None)),
        "start_record" => Ok(("StartRecord", None)),
        "stop_record" => Ok(("StopRecord", None)),
        "toggle_stream" => Ok(("ToggleStream", None)),
        "start_stream" => Ok(("StartStream", None)),
        "stop_stream" => Ok(("StopStream", None)),
        "toggle_input_mute" => {
            let input_name = args
                .get("input_name")
                .ok_or_else(|| ProviderError::MissingArg {
                    provider: "obs".to_string(),
                    command: "toggle_input_mute".to_string(),
                    arg: "input_name".to_string(),
                })?;
            Ok(("ToggleInputMute", Some(json!({ "inputName": input_name }))))
        }
        "set_scene" => {
            let scene_name = args
                .get("scene_name")
                .ok_or_else(|| ProviderError::MissingArg {
                    provider: "obs".to_string(),
                    command: "set_scene".to_string(),
                    arg: "scene_name".to_string(),
                })?;
            Ok((
                "SetCurrentProgramScene",
                Some(json!({ "sceneName": scene_name })),
            ))
        }
        other => Err(ProviderError::UnknownCommand {
            provider: "obs".to_string(),
            command: other.to_string(),
        }),
    }
}

struct ObsConnectionState {
    socket: Option<WebSocket<TcpStream>>,
    next_request_id: u64,
    consecutive_failures: u32,
    retry_after: Option<Instant>,
}

impl ObsConnectionState {
    fn new() -> Self {
        Self {
            socket: None,
            next_request_id: 1,
            consecutive_failures: 0,
            retry_after: None,
        }
    }

    fn record_failure(&mut self, now: Instant) {
        self.socket = None;
        let shift = self.consecutive_failures.min(4);
        let delay_ms = (OBS_BACKOFF_INITIAL_MS << shift).min(OBS_BACKOFF_MAX_MS);
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.retry_after = Some(now + Duration::from_millis(delay_ms));
    }

    fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.retry_after = None;
    }
}

/// Provider de integração direta com o **OBS Studio** via obs-websocket 5.x (`ws://127.0.0.1:<port>`).
///
/// Conexão estritamente preguiçosa (nenhum socket é aberto até o primeiro comando ou teste)
/// com reconexão automática e backoff exponencial quando o OBS está fechado.
pub struct ObsProvider {
    config: ObsProviderConfig,
    state: Mutex<ObsConnectionState>,
}

impl ObsProvider {
    pub fn new(config: ObsProviderConfig) -> Self {
        Self {
            config,
            state: Mutex::new(ObsConnectionState::new()),
        }
    }

    fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.config.port)
    }

    fn connect_and_identify(&self) -> Result<(WebSocket<TcpStream>, String), ProviderError> {
        let addr = self.socket_addr();
        let stream = TcpStream::connect_timeout(&addr, OBS_CONNECT_TIMEOUT).map_err(|err| {
            ProviderError::Unavailable(
                "obs".to_string(),
                format!("OBS WebSocket não respondeu em {addr}: {err}"),
            )
        })?;
        let _ = stream.set_read_timeout(Some(OBS_IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(OBS_IO_TIMEOUT));

        let url = format!("ws://127.0.0.1:{}", self.config.port);
        let (mut ws, _resp) = client(&url, stream).map_err(|err| {
            ProviderError::Unavailable("obs".to_string(), format!("handshake WebSocket: {err}"))
        })?;

        // 1. Aguardar Hello (op = 0)
        let hello = read_obs_json(&mut ws)?;
        let op = hello.get("op").and_then(Value::as_u64).unwrap_or(999);
        if op != 0 {
            return Err(ProviderError::Unavailable(
                "obs".to_string(),
                format!("esperava Hello (op=0), recebido op={op}"),
            ));
        }

        let obs_version = hello
            .pointer("/d/obsWebSocketVersion")
            .and_then(Value::as_str)
            .unwrap_or("5.x")
            .to_string();

        // 2. Montar Identify (op = 1), assinando challenge se necessário
        let mut identify_data = json!({
            "rpcVersion": 1,
            "eventSubscriptions": 0,
        });

        if let Some(auth_obj) = hello.pointer("/d/authentication") {
            let challenge = auth_obj
                .get("challenge")
                .and_then(Value::as_str)
                .unwrap_or("");
            let salt = auth_obj.get("salt").and_then(Value::as_str).unwrap_or("");
            if !challenge.is_empty() && !salt.is_empty() {
                let password = self.config.password.as_deref().unwrap_or("");
                if password.is_empty() {
                    return Err(ProviderError::AuthFailed(
                        "obs".to_string(),
                        "OBS exige senha WebSocket, mas nenhuma senha foi configurada".to_string(),
                    ));
                }
                let auth_response = compute_obs_auth_response(password, salt, challenge);
                identify_data["authentication"] = Value::String(auth_response);
            }
        }

        let identify_msg = json!({
            "op": 1,
            "d": identify_data,
        });
        ws.send(Message::Text(identify_msg.to_string().into()))
            .map_err(|err| {
                ProviderError::Unavailable(
                    "obs".to_string(),
                    format!("erro ao enviar Identify: {err}"),
                )
            })?;

        // 3. Aguardar Identified (op = 2)
        let identified = read_obs_json(&mut ws)?;
        let id_op = identified.get("op").and_then(Value::as_u64).unwrap_or(999);
        if id_op != 2 {
            return Err(ProviderError::AuthFailed(
                "obs".to_string(),
                format!("OBS rejeitou identificação (op={id_op})"),
            ));
        }

        Ok((ws, obs_version))
    }

    fn send_request_on_socket(
        ws: &mut WebSocket<TcpStream>,
        request_id: &str,
        request_type: &str,
        request_data: Option<Value>,
        command_id: &str,
    ) -> Result<(), ProviderError> {
        let mut data = json!({
            "requestType": request_type,
            "requestId": request_id,
        });
        if let Some(req_data) = request_data {
            data["requestData"] = req_data;
        }
        let payload = json!({
            "op": 6,
            "d": data,
        });

        ws.send(Message::Text(payload.to_string().into()))
            .map_err(|err| ProviderError::Unavailable("obs".to_string(), err.to_string()))?;

        // Lê frames até encontrar RequestResponse (op = 7) com o mesmo requestId
        for _ in 0..16 {
            let msg = read_obs_json(ws)?;
            let op = msg.get("op").and_then(Value::as_u64).unwrap_or(999);
            if op != 7 {
                continue;
            }
            let resp_id = msg.pointer("/d/requestId").and_then(Value::as_str);
            if resp_id != Some(request_id) {
                continue;
            }
            let ok = msg
                .pointer("/d/requestStatus/result")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if ok {
                return Ok(());
            }
            let comment = msg
                .pointer("/d/requestStatus/comment")
                .and_then(Value::as_str)
                .unwrap_or("erro retornado pelo OBS");
            let code = msg
                .pointer("/d/requestStatus/code")
                .and_then(Value::as_i64)
                .unwrap_or(-1);
            return Err(ProviderError::CommandFailed {
                provider: "obs".to_string(),
                command: command_id.to_string(),
                reason: format!("{comment} (código {code})"),
            });
        }

        Err(ProviderError::CommandFailed {
            provider: "obs".to_string(),
            command: command_id.to_string(),
            reason: "sem resposta RequestResponse (op=7) do OBS".to_string(),
        })
    }
}

fn read_obs_json(ws: &mut WebSocket<TcpStream>) -> Result<Value, ProviderError> {
    loop {
        match ws.read() {
            Ok(Message::Text(text)) => {
                return serde_json::from_str::<Value>(text.as_str()).map_err(|err| {
                    ProviderError::Unavailable(
                        "obs".to_string(),
                        format!("JSON inválido do OBS: {err}"),
                    )
                });
            }
            Ok(Message::Ping(payload)) => {
                let _ = ws.send(Message::Pong(payload));
            }
            Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {}
            Ok(Message::Close(frame)) => {
                if let Some(cf) = frame {
                    let is_auth = matches!(cf.code, CloseCode::Library(4009));
                    let reason = cf.reason.to_string();
                    if is_auth {
                        return Err(ProviderError::AuthFailed(
                            "obs".to_string(),
                            if reason.is_empty() {
                                "senha incorreta (código 4009)".to_string()
                            } else {
                                reason
                            },
                        ));
                    }
                    return Err(ProviderError::Unavailable(
                        "obs".to_string(),
                        format!("conexão fechada pelo OBS ({:?}): {reason}", cf.code),
                    ));
                }
                return Err(ProviderError::Unavailable(
                    "obs".to_string(),
                    "conexão fechada pelo OBS".to_string(),
                ));
            }
            Ok(Message::Binary(_)) => {}
            Err(err) => {
                return Err(ProviderError::Unavailable(
                    "obs".to_string(),
                    err.to_string(),
                ));
            }
        }
    }
}

impl Provider for ObsProvider {
    fn id(&self) -> &'static str {
        "obs"
    }

    fn name(&self) -> &'static str {
        "OBS Studio"
    }

    fn is_available(&self) -> bool {
        if !self.config.enabled {
            return false;
        }
        if let Ok(guard) = self.state.lock() {
            if guard.socket.is_some() {
                return true;
            }
        }
        TcpStream::connect_timeout(&self.socket_addr(), OBS_PROBE_TIMEOUT).is_ok()
    }

    fn commands(&self) -> Vec<ProviderCommand> {
        vec![
            ProviderCommand {
                id: "toggle_record",
                name: "Alternar gravação",
                args_schema: ArgsSchema::empty(),
            },
            ProviderCommand {
                id: "start_record",
                name: "Iniciar gravação",
                args_schema: ArgsSchema::empty(),
            },
            ProviderCommand {
                id: "stop_record",
                name: "Parar gravação",
                args_schema: ArgsSchema::empty(),
            },
            ProviderCommand {
                id: "toggle_stream",
                name: "Alternar transmissão",
                args_schema: ArgsSchema::empty(),
            },
            ProviderCommand {
                id: "start_stream",
                name: "Iniciar transmissão",
                args_schema: ArgsSchema::empty(),
            },
            ProviderCommand {
                id: "stop_stream",
                name: "Parar transmissão",
                args_schema: ArgsSchema::empty(),
            },
            ProviderCommand {
                id: "toggle_input_mute",
                name: "Silenciar / ativar fonte de áudio",
                args_schema: ArgsSchema::single(
                    "input_name",
                    "Nome da fonte de áudio",
                    "ex: Mic/Aux",
                    true,
                ),
            },
            ProviderCommand {
                id: "set_scene",
                name: "Trocar cena ativa",
                args_schema: ArgsSchema::single(
                    "scene_name",
                    "Nome da cena",
                    "ex: Cena Principal",
                    true,
                ),
            },
        ]
    }

    fn execute(&self, command_id: &str, args: &ProviderArgs) -> Result<(), ProviderError> {
        if !self.config.enabled {
            return Err(ProviderError::Unavailable(
                "obs".to_string(),
                "OBS WebSocket desabilitado em Configurações → Integrações".to_string(),
            ));
        }

        let (request_type, request_data) = map_obs_request(command_id, args)?;

        let mut guard = self.state.lock().map_err(|_| {
            ProviderError::Unavailable("obs".to_string(), "lock interno corrompido".to_string())
        })?;

        let now = Instant::now();
        if guard.socket.is_none() {
            if let Some(retry_at) = guard.retry_after {
                if now < retry_at {
                    return Err(ProviderError::Unavailable(
                        "obs".to_string(),
                        "aguardando backoff de reconexão com o OBS".to_string(),
                    ));
                }
            }
        }

        // Tenta até 2 vezes caso um socket reaproveitado tenha caído (ex.: OBS reiniciado)
        for attempt in 0..2 {
            if guard.socket.is_none() {
                match self.connect_and_identify() {
                    Ok((ws, _ver)) => {
                        guard.socket = Some(ws);
                        guard.record_success();
                    }
                    Err(err) => {
                        guard.record_failure(Instant::now());
                        return Err(err);
                    }
                }
            }

            let req_id = format!("kc-{}", guard.next_request_id);
            guard.next_request_id = guard.next_request_id.wrapping_add(1);

            let res = if let Some(ref mut ws) = guard.socket {
                Self::send_request_on_socket(
                    ws,
                    &req_id,
                    request_type,
                    request_data.clone(),
                    command_id,
                )
            } else {
                Err(ProviderError::Unavailable(
                    "obs".to_string(),
                    "socket ausente".to_string(),
                ))
            };

            match res {
                Ok(()) => {
                    guard.record_success();
                    return Ok(());
                }
                Err(ProviderError::Unavailable(_, _)) if attempt == 0 => {
                    // Socket stale: descarta e reconecta imediatamente nesta mesma tentativa
                    guard.socket = None;
                    continue;
                }
                Err(err) => {
                    if matches!(
                        err,
                        ProviderError::Unavailable(_, _) | ProviderError::AuthFailed(_, _)
                    ) {
                        guard.record_failure(Instant::now());
                    }
                    return Err(err);
                }
            }
        }

        Err(ProviderError::Unavailable(
            "obs".to_string(),
            "falha ao comunicar com o OBS".to_string(),
        ))
    }

    fn test_connection(&self) -> Result<String, ProviderError> {
        if !self.config.enabled {
            return Err(ProviderError::Unavailable(
                "obs".to_string(),
                "habilite o OBS WebSocket antes de testar".to_string(),
            ));
        }

        let mut guard = self.state.lock().map_err(|_| {
            ProviderError::Unavailable("obs".to_string(), "lock interno corrompido".to_string())
        })?;
        // Força handshake real (também valida senha) e zera backoff
        guard.socket = None;
        guard.retry_after = None;

        match self.connect_and_identify() {
            Ok((ws, version)) => {
                guard.socket = Some(ws);
                guard.record_success();
                Ok(format!(
                    "Conectado ao OBS WebSocket v{version} (porta {})",
                    self.config.port
                ))
            }
            Err(err) => {
                guard.record_failure(Instant::now());
                Err(err)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{compute_obs_auth_response, map_obs_request, ObsProvider};
    use crate::config::ObsProviderConfig;
    use crate::providers::{Provider, ProviderArgs, ProviderError};
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use base64::Engine;
    use serde_json::json;
    use sha2::{Digest, Sha256};

    #[test]
    fn computes_obs_v5_challenge_response_matching_official_spec() {
        // Vetor oficial da documentação do protocolo obs-websocket v5:
        // password = "supersecretpassword"
        // salt = "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI="
        // challenge = "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeagNgGY3CDf8="
        let password = "supersecretpassword";
        let salt = "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=";
        let challenge = "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeagNgGY3CDf8=";

        // Verifica também os dois passos intermediários independentemente
        let expected_secret =
            BASE64_STANDARD.encode(Sha256::digest(format!("{password}{salt}").as_bytes()));
        assert_eq!(
            expected_secret,
            "H1IfVz1pSREUQzbFTVnX/Tyb+gMhMik5x7yUBCY0PTs="
        );
        let expected_auth = BASE64_STANDARD.encode(Sha256::digest(
            format!("{expected_secret}{challenge}").as_bytes(),
        ));
        assert_eq!(
            compute_obs_auth_response(password, salt, challenge),
            expected_auth
        );
        assert_eq!(
            compute_obs_auth_response(password, salt, challenge),
            "MSyByQH/oVoCRqijf2Vtog62v4/MQhHWBmtIRyz9mMQ="
        );
    }

    #[test]
    fn maps_all_eight_obs_commands_and_validates_required_args() {
        let empty = ProviderArgs::default();
        assert_eq!(
            map_obs_request("toggle_record", &empty).ok(),
            Some(("ToggleRecord", None))
        );
        assert_eq!(
            map_obs_request("start_record", &empty).ok(),
            Some(("StartRecord", None))
        );
        assert_eq!(
            map_obs_request("stop_record", &empty).ok(),
            Some(("StopRecord", None))
        );
        assert_eq!(
            map_obs_request("toggle_stream", &empty).ok(),
            Some(("ToggleStream", None))
        );
        assert_eq!(
            map_obs_request("start_stream", &empty).ok(),
            Some(("StartStream", None))
        );
        assert_eq!(
            map_obs_request("stop_stream", &empty).ok(),
            Some(("StopStream", None))
        );

        assert!(matches!(
            map_obs_request("toggle_input_mute", &empty),
            Err(ProviderError::MissingArg { .. })
        ));
        let mut mute_args = ProviderArgs::default();
        mute_args
            .values
            .insert("input_name".to_string(), "Mic/Aux".to_string());
        assert_eq!(
            map_obs_request("toggle_input_mute", &mute_args).ok(),
            Some(("ToggleInputMute", Some(json!({ "inputName": "Mic/Aux" }))))
        );

        let mut scene_args = ProviderArgs::default();
        scene_args
            .values
            .insert("scene_name".to_string(), "Live".to_string());
        assert_eq!(
            map_obs_request("set_scene", &scene_args).ok(),
            Some((
                "SetCurrentProgramScene",
                Some(json!({ "sceneName": "Live" }))
            ))
        );
    }

    #[test]
    fn disabled_obs_provider_is_unavailable_without_opening_network() {
        let obs = ObsProvider::new(ObsProviderConfig::default());
        assert!(!obs.is_available());
        assert_eq!(obs.commands().len(), 8);
        let err = obs
            .execute("toggle_record", &ProviderArgs::default())
            .expect_err("OBS desabilitado deve retornar erro claro");
        assert!(matches!(err, ProviderError::Unavailable(_, _)));
    }
}
