#[cfg(target_os = "windows")]
pub mod windows;

/// Evento de tecla capturado globalmente pelo backend da plataforma (ex.: `WH_KEYBOARD_LL` no Windows).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedKeyEvent {
    pub text: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
    pub pressed: bool,
    pub repeat: bool,
}

/// Contrato multiplataforma para captura global temporária de teclas durante o Modo Aprender.
///
/// Deve ser ativado **exclusivamente** enquanto a janela `LearnWindow` estiver aberta,
/// nunca em background.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub trait KeyCapture {
    fn poll_events(&self) -> Vec<CapturedKeyEvent>;
    fn stop(&mut self);
}

/// Plataforma operacional detectada em tempo de execução/compilação.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformKind {
    Windows,
    Linux,
    Other,
}

impl PlatformKind {
    /// Detecta a plataforma atual usando `cfg!`.
    pub const fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Other
        }
    }

    pub const fn is_windows(self) -> bool {
        matches!(self, Self::Windows)
    }

    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub const fn is_linux(self) -> bool {
        matches!(self, Self::Linux)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::Linux => "Linux",
            Self::Other => "Outro",
        }
    }

    /// Identificador do provider nativo de mídia nesta plataforma (`"smtc"` no Windows, `"mpris"` no Linux).
    pub const fn media_provider_id(self) -> &'static str {
        match self {
            Self::Windows => "smtc",
            Self::Linux => "mpris",
            Self::Other => "mpris",
        }
    }

    /// Indica se um provider específico está disponível nesta plataforma.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn supports_provider(self, provider_id: &str) -> bool {
        match provider_id.trim().to_ascii_lowercase().as_str() {
            "obs" => true,
            "smtc" => matches!(self, Self::Windows),
            "mpris" => matches!(self, Self::Linux),
            _ => false,
        }
    }

    /// Indica se um provider pertence a outra plataforma suportada (usado para ignorar
    /// sem erro ações/seções sincronizadas via dual-boot no mesmo `config.toml`).
    pub fn is_other_platform_builtin_provider(self, provider_id: &str) -> bool {
        match provider_id.trim().to_ascii_lowercase().as_str() {
            "smtc" => !matches!(self, Self::Windows),
            "mpris" => !matches!(self, Self::Linux),
            _ => false,
        }
    }
}

/// Consulta a preferência do sistema operacional para "Reduzir movimento" / desativar animações.
///
/// No Windows consulta `SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION)`.
/// Quando o usuário desativa efeitos de animação nas configurações de Acessibilidade do Windows,
/// retorna `true` para que todas as transições do Overlay/Hub sejam instantâneas.
pub fn prefers_reduced_motion() -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::ffi::c_void;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION,
        };

        let mut client_area_animations_enabled: i32 = 1;
        // SAFETY: ponteiro válido para `BOOL` (`i32`) conforme documentação de `SPI_GETCLIENTAREAANIMATION`.
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                (&mut client_area_animations_enabled as *mut i32).cast::<c_void>(),
                0,
            )
        };
        ok != 0 && client_area_animations_enabled == 0
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

/// Resolve se animações devem rodar considerando a configuração do usuário e o Reduced Motion do SO.
pub const fn effective_animations_enabled(
    config_enabled: bool,
    system_reduced_motion: bool,
) -> bool {
    config_enabled && !system_reduced_motion
}

#[cfg(test)]
mod tests {
    use super::{effective_animations_enabled, PlatformKind};

    #[test]
    fn detects_current_platform_and_provider_support() {
        let current = PlatformKind::current();
        #[cfg(target_os = "windows")]
        assert_eq!(current, PlatformKind::Windows);
        #[cfg(target_os = "linux")]
        assert_eq!(current, PlatformKind::Linux);

        assert!(PlatformKind::Windows.supports_provider("smtc"));
        assert!(PlatformKind::Windows.supports_provider("obs"));
        assert!(!PlatformKind::Windows.supports_provider("mpris"));
        assert!(PlatformKind::Windows.is_other_platform_builtin_provider("mpris"));
        assert!(!PlatformKind::Windows.is_other_platform_builtin_provider("smtc"));
        assert_eq!(PlatformKind::Windows.media_provider_id(), "smtc");
        assert_eq!(PlatformKind::Windows.display_name(), "Windows");

        assert!(PlatformKind::Linux.supports_provider("mpris"));
        assert!(PlatformKind::Linux.supports_provider("obs"));
        assert!(!PlatformKind::Linux.supports_provider("smtc"));
        assert!(PlatformKind::Linux.is_other_platform_builtin_provider("smtc"));
        assert!(!PlatformKind::Linux.is_other_platform_builtin_provider("mpris"));
        assert_eq!(PlatformKind::Linux.media_provider_id(), "mpris");
        assert_eq!(PlatformKind::Linux.display_name(), "Linux");
    }

    #[test]
    fn reduced_motion_forces_instant_animations() {
        assert!(effective_animations_enabled(true, false));
        assert!(!effective_animations_enabled(true, true));
        assert!(!effective_animations_enabled(false, false));
        assert!(!effective_animations_enabled(false, true));
    }
}
