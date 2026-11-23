use std::io;
use std::path::PathBuf;
use thiserror::Error;

/// Erros centralizados do aplicativo KnobContext.
#[derive(Debug, Error)]
pub enum KnobError {
    #[error("falha ao ler arquivo de configuração '{path}': {source}")]
    ConfigRead {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("falha ao salvar arquivo de configuração '{path}': {source}")]
    ConfigWrite {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("arquivo de configuração inválido '{path}': {message}")]
    ConfigParse { path: PathBuf, message: String },

    #[error("configuração inválida: {0}")]
    ConfigValidation(String),

    #[error("falha ao abrir dispositivo de entrada '{path}': {source}")]
    DeviceOpen {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("falha ao capturar exclusivamente (grab) '{path}': {source}")]
    DeviceGrab {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("falha ao ler eventos de '{path}': {source}")]
    DeviceRead {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("falha ao inicializar simulador de entrada: {0}")]
    InputSimulationInit(String),

    #[error("falha ao executar ação de entrada: {0}")]
    InputSimulation(String),

    #[error("ação inválida na configuração: {0}")]
    InvalidAction(String),

    #[error("falha ao inicializar interface overlay (Slint): {0}")]
    UiInit(String),

    #[error("falha no loop de eventos da interface (Slint): {0}")]
    UiRun(String),
}

impl KnobError {
    /// Retorna `true` se o erro for causado por falta de permissão do SO (`PermissionDenied`).
    pub fn is_permission_denied(&self) -> bool {
        match self {
            Self::DeviceOpen { source, .. }
            | Self::DeviceGrab { source, .. }
            | Self::DeviceRead { source, .. }
            | Self::ConfigRead { source, .. } => source.kind() == io::ErrorKind::PermissionDenied,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::KnobError;
    use std::io;
    use std::path::PathBuf;

    #[test]
    fn detects_permission_denied_on_grab_and_open() {
        let err = KnobError::DeviceGrab {
            path: PathBuf::from("/dev/input/by-id/usb-1189_8890-event-kbd"),
            source: io::Error::new(io::ErrorKind::PermissionDenied, "permission denied"),
        };
        assert!(err.is_permission_denied());
        assert!(err.to_string().contains("grab"));

        let parse_err = KnobError::ConfigParse {
            path: PathBuf::from("config.toml"),
            message: "expected '='".to_string(),
        };
        assert!(!parse_err.is_permission_denied());
    }
}
