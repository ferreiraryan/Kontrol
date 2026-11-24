#![allow(dead_code)]

use crate::config::ProvidersConfig;
use std::collections::{BTreeMap, HashMap};
use thiserror::Error;

/// Erros padronizados retornados por qualquer provider de integração direta.
#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("provider desconhecido: '{0}'")]
    NotFound(String),

    #[error("comando '{command}' não suportado pelo provider '{provider}'")]
    UnknownCommand { provider: String, command: String },

    #[error("argumento obrigatório ausente '{arg}' para '{provider}:{command}'")]
    MissingArg {
        provider: String,
        command: String,
        arg: String,
    },

    #[error("argumento inválido '{arg}' para '{provider}:{command}': {reason}")]
    InvalidArg {
        provider: String,
        command: String,
        arg: String,
        reason: String,
    },

    #[error("provider '{0}' indisponível: {1}")]
    Unavailable(String, String),

    #[error("falha de autenticação no provider '{0}': {1}")]
    AuthFailed(String, String),

    #[error("falha ao executar '{provider}:{command}': {reason}")]
    CommandFailed {
        provider: String,
        command: String,
        reason: String,
    },
}

/// Definição de um campo de argumento exibido dinamicamente na UI do editor de ações.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgField {
    pub key: &'static str,
    pub label: &'static str,
    pub placeholder: &'static str,
    pub required: bool,
}

/// Esquema de argumentos aceitos por um `ProviderCommand`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArgsSchema {
    pub fields: Vec<ArgField>,
}

impl ArgsSchema {
    pub fn empty() -> Self {
        Self { fields: Vec::new() }
    }

    pub fn single(
        key: &'static str,
        label: &'static str,
        placeholder: &'static str,
        required: bool,
    ) -> Self {
        Self {
            fields: vec![ArgField {
                key,
                label,
                placeholder,
                required,
            }],
        }
    }
}

/// Comando exposto por um `Provider`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCommand {
    pub id: &'static str,
    pub name: &'static str,
    pub args_schema: ArgsSchema,
}

/// Argumentos chave-valor enviados ao executar um comando de provider.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderArgs {
    pub values: HashMap<String, String>,
}

impl ProviderArgs {
    pub fn from_btree(map: &BTreeMap<String, String>) -> Self {
        Self {
            values: map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values
            .get(key)
            .map(String::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
    }
}

/// Contrato genérico para qualquer integração direta (OBS, MPRIS, futuros plugins).
///
/// O núcleo (`ActionDispatcher`) nunca referencia providers concretos — apenas delega
/// para `ProviderRegistry`.
pub trait Provider: Send + Sync {
    /// Identificador curto estável usado no formato `Provider:<id>:<cmd>` (ex.: `"obs"`, `"mpris"`).
    fn id(&self) -> &'static str;

    /// Nome amigável exibido na interface (ex.: `"OBS Studio"`).
    fn name(&self) -> &'static str;

    /// Indica se o serviço/aplicativo alvo está acessível neste momento.
    fn is_available(&self) -> bool;

    /// Lista de comandos suportados pelo provider e seus esquemas de argumentos.
    fn commands(&self) -> Vec<ProviderCommand>;

    /// Executa um comando diretamente na API do provider.
    fn execute(&self, command_id: &str, args: &ProviderArgs) -> Result<(), ProviderError>;

    /// Teste explícito de conectividade acionado pela aba Integrações nas Configurações.
    fn test_connection(&self) -> Result<String, ProviderError> {
        if self.is_available() {
            Ok(format!("{} disponível", self.name()))
        } else {
            Err(ProviderError::Unavailable(
                self.id().to_string(),
                "serviço não detectado".to_string(),
            ))
        }
    }
}

/// Registro central de providers ativos.
#[derive(Default)]
pub struct ProviderRegistry {
    providers: Vec<Box<dyn Provider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// Constrói o registro padrão com todos os providers embutidos a partir de `[providers]`.
    pub fn default_registry(_config: &ProvidersConfig) -> Self {
        Self::new()
    }

    pub fn register(&mut self, provider: Box<dyn Provider>) {
        if let Some(pos) = self
            .providers
            .iter()
            .position(|existing| existing.id().eq_ignore_ascii_case(provider.id()))
        {
            self.providers[pos] = provider;
        } else {
            self.providers.push(provider);
        }
    }

    pub fn providers(&self) -> &[Box<dyn Provider>] {
        &self.providers
    }

    pub fn find(&self, id: &str) -> Option<&dyn Provider> {
        self.providers
            .iter()
            .find(|p| p.id().eq_ignore_ascii_case(id.trim()))
            .map(|p| p.as_ref())
    }

    /// Valida se `provider_id` existe, se `command_id` é suportado e se todos os
    /// argumentos obrigatórios do `ArgsSchema` estão preenchidos.
    pub fn validate_action(
        &self,
        provider_id: &str,
        command_id: &str,
        args: &ProviderArgs,
    ) -> Result<(), ProviderError> {
        let Some(provider) = self.find(provider_id) else {
            return Err(ProviderError::NotFound(provider_id.to_string()));
        };
        let commands = provider.commands();
        let Some(cmd) = commands
            .iter()
            .find(|c| c.id.eq_ignore_ascii_case(command_id.trim()))
        else {
            return Err(ProviderError::UnknownCommand {
                provider: provider.id().to_string(),
                command: command_id.to_string(),
            });
        };
        for field in &cmd.args_schema.fields {
            if field.required && args.get(field.key).is_none() {
                return Err(ProviderError::MissingArg {
                    provider: provider.id().to_string(),
                    command: cmd.id.to_string(),
                    arg: field.key.to_string(),
                });
            }
        }
        Ok(())
    }

    /// Executa `provider_id:command_id` validando previamente existência e argumentos.
    pub fn execute(
        &self,
        provider_id: &str,
        command_id: &str,
        args: &ProviderArgs,
    ) -> Result<(), ProviderError> {
        self.validate_action(provider_id, command_id, args)?;
        let provider = self
            .find(provider_id)
            .ok_or_else(|| ProviderError::NotFound(provider_id.to_string()))?;
        provider.execute(command_id, args)
    }
}

/// Valida uma ação `Provider:<id>:<cmd>` contra o registro padrão de providers embutidos.
///
/// Quando o registro padrão possui providers registrados, exige que `provider_id` e
/// `command_id` existam e que argumentos obrigatórios estejam preenchidos.
pub fn validate_builtin_provider_action(
    provider_id: &str,
    command_id: &str,
    args: &BTreeMap<String, String>,
) -> Result<(), ProviderError> {
    let registry = ProviderRegistry::default_registry(&ProvidersConfig::default());
    if registry.providers().is_empty() {
        return Ok(());
    }
    registry.validate_action(provider_id, command_id, &ProviderArgs::from_btree(args))
}

#[cfg(test)]
mod tests {
    use super::{
        ArgsSchema, Provider, ProviderArgs, ProviderCommand, ProviderError, ProviderRegistry,
    };
    use std::sync::{Arc, Mutex};

    struct MockProvider {
        available: bool,
        executed: Arc<Mutex<Vec<(String, ProviderArgs)>>>,
    }

    impl Provider for MockProvider {
        fn id(&self) -> &'static str {
            "mock"
        }

        fn name(&self) -> &'static str {
            "Mock App"
        }

        fn is_available(&self) -> bool {
            self.available
        }

        fn commands(&self) -> Vec<ProviderCommand> {
            vec![
                ProviderCommand {
                    id: "ping",
                    name: "Ping",
                    args_schema: ArgsSchema::empty(),
                },
                ProviderCommand {
                    id: "set_target",
                    name: "Definir alvo",
                    args_schema: ArgsSchema::single("target", "Alvo", "ex: Cena 1", true),
                },
            ]
        }

        fn execute(&self, command_id: &str, args: &ProviderArgs) -> Result<(), ProviderError> {
            if !self.available {
                return Err(ProviderError::Unavailable(
                    self.id().to_string(),
                    "offline".to_string(),
                ));
            }
            if let Ok(mut guard) = self.executed.lock() {
                guard.push((command_id.to_string(), args.clone()));
            }
            Ok(())
        }
    }

    #[test]
    fn registry_validates_and_executes_registered_mock_provider() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut reg = ProviderRegistry::new();
        reg.register(Box::new(MockProvider {
            available: true,
            executed: Arc::clone(&log),
        }));

        // Comando sem argumentos funciona
        reg.execute("mock", "ping", &ProviderArgs::default())
            .expect("ping válido");

        // Provider inexistente falha
        let err_nf = reg
            .execute("desconhecido", "ping", &ProviderArgs::default())
            .expect_err("provider inexistente deve falhar");
        assert!(matches!(err_nf, ProviderError::NotFound(_)));

        // Comando inexistente falha
        let err_cmd = reg
            .execute("mock", "invalido", &ProviderArgs::default())
            .expect_err("comando inválido deve falhar");
        assert!(matches!(err_cmd, ProviderError::UnknownCommand { .. }));

        // Argumento obrigatório ausente falha
        let err_arg = reg
            .execute("mock", "set_target", &ProviderArgs::default())
            .expect_err("arg obrigatório ausente deve falhar");
        assert!(matches!(err_arg, ProviderError::MissingArg { .. }));

        // Com argumento obrigatório presente executa com sucesso
        let mut args = ProviderArgs::default();
        args.values
            .insert("target".to_string(), "Cena 1".to_string());
        reg.execute("mock", "set_target", &args)
            .expect("set_target válido");

        let recorded = log.lock().expect("lock log");
        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[0].0, "ping");
        assert_eq!(recorded[1].0, "set_target");
        assert_eq!(recorded[1].1.get("target"), Some("Cena 1"));
    }
}
