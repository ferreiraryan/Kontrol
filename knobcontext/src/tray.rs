use crate::context::ProfileMode;
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{CheckmarkItem, StandardItem, SubMenu};
use ksni::{Icon, MenuItem, ToolTip, Tray};
use log::{info, warn};
use std::sync::mpsc::Sender;

const TRAY_ICON_ARGB: &[u8] = include_bytes!("../assets/tray-icon-24.argb");

/// Comandos disparados pelo menu do ícone no tray para o loop principal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayCommand {
    OpenSettings,
    SetMode(ProfileMode),
    Quit,
}

/// Formata o texto de tooltip do tray de acordo com o modo e perfil contextual atual.
pub fn format_tooltip(mode: &ProfileMode, contextual_profile: &str) -> String {
    match mode {
        ProfileMode::Auto => format!("KnobContext — Automático ({contextual_profile})"),
        ProfileMode::Manual(name) => format!("KnobContext — Manual: {name}"),
    }
}

pub fn is_auto_checked(mode: &ProfileMode) -> bool {
    matches!(mode, ProfileMode::Auto)
}

pub fn is_manual_profile_checked(mode: &ProfileMode, profile_name: &str) -> bool {
    matches!(mode, ProfileMode::Manual(current) if current.eq_ignore_ascii_case(profile_name))
}

pub struct KnobTray {
    mode: ProfileMode,
    contextual_profile: String,
    profile_names: Vec<String>,
    tx: Sender<TrayCommand>,
}

impl Tray for KnobTray {
    fn id(&self) -> String {
        "knobcontext".to_string()
    }

    fn title(&self) -> String {
        format_tooltip(&self.mode, &self.contextual_profile)
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        vec![Icon {
            width: 24,
            height: 24,
            data: TRAY_ICON_ARGB.to_vec(),
        }]
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "KnobContext".to_string(),
            description: format_tooltip(&self.mode, &self.contextual_profile),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let manual_items: Vec<MenuItem<Self>> = self
            .profile_names
            .iter()
            .map(|name| {
                let target_name = name.clone();
                let checked = is_manual_profile_checked(&self.mode, name);
                CheckmarkItem {
                    label: name.clone(),
                    checked,
                    activate: Box::new(move |this: &mut Self| {
                        let _ = this.tx.send(TrayCommand::SetMode(ProfileMode::Manual(
                            target_name.clone(),
                        )));
                    }),
                    ..Default::default()
                }
                .into()
            })
            .collect();

        vec![
            StandardItem {
                label: "Abrir configurações".to_string(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(TrayCommand::OpenSettings);
                }),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Modo Automático".to_string(),
                checked: is_auto_checked(&self.mode),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(TrayCommand::SetMode(ProfileMode::Auto));
                }),
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: "Modo Manual".to_string(),
                submenu: manual_items,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Sair".to_string(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(TrayCommand::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Wrapper resiliente sobre o `ksni::blocking::Handle` que só envia updates D-Bus quando há mudança.
pub struct TrayController {
    handle: Option<Handle<KnobTray>>,
    last_mode: ProfileMode,
    last_contextual: String,
    last_profiles: Vec<String>,
}

impl TrayController {
    pub fn spawn(
        mode: ProfileMode,
        contextual_profile: String,
        profile_names: Vec<String>,
        tx: Sender<TrayCommand>,
    ) -> Self {
        let tray = KnobTray {
            mode: mode.clone(),
            contextual_profile: contextual_profile.clone(),
            profile_names: profile_names.clone(),
            tx,
        };

        let handle = match tray.spawn() {
            Ok(h) => {
                info!("Ícone de tray StatusNotifierItem (SNI) registrado via D-Bus.");
                Some(h)
            }
            Err(err) => {
                warn!("Não foi possível iniciar o ícone de tray SNI: {err}. Continuando sem tray.");
                None
            }
        };

        Self {
            handle,
            last_mode: mode,
            last_contextual: contextual_profile,
            last_profiles: profile_names,
        }
    }

    pub fn sync_if_changed(
        &mut self,
        mode: &ProfileMode,
        contextual_profile: &str,
        profile_names: &[String],
    ) {
        if self.last_mode == *mode
            && self.last_contextual == contextual_profile
            && self.last_profiles == profile_names
        {
            return;
        }

        self.last_mode = mode.clone();
        self.last_contextual = contextual_profile.to_string();
        self.last_profiles = profile_names.to_vec();

        if let Some(handle) = &self.handle {
            let new_mode = mode.clone();
            let new_ctx = contextual_profile.to_string();
            let new_profiles = profile_names.to_vec();
            let _ = handle.update(move |tray| {
                tray.mode = new_mode;
                tray.contextual_profile = new_ctx;
                tray.profile_names = new_profiles;
            });
        }
    }

    pub fn shutdown(&self) {
        if let Some(handle) = &self.handle {
            handle.shutdown().wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        format_tooltip, is_auto_checked, is_manual_profile_checked, KnobTray, TrayCommand,
        TRAY_ICON_ARGB,
    };
    use crate::context::ProfileMode;
    use ksni::{MenuItem, Tray};
    use std::sync::mpsc;

    #[test]
    fn formats_tooltip_for_auto_and_manual_modes() {
        assert_eq!(
            format_tooltip(&ProfileMode::Auto, "firefox"),
            "KnobContext — Automático (firefox)"
        );
        assert_eq!(
            format_tooltip(&ProfileMode::Manual("global".to_string()), "firefox"),
            "KnobContext — Manual: global"
        );
    }

    #[test]
    fn menu_reflects_profile_mode_checkmarks() {
        assert!(is_auto_checked(&ProfileMode::Auto));
        assert!(!is_manual_profile_checked(&ProfileMode::Auto, "global"));

        let manual = ProfileMode::Manual("firefox".to_string());
        assert!(!is_auto_checked(&manual));
        assert!(is_manual_profile_checked(&manual, "firefox"));
        assert!(!is_manual_profile_checked(&manual, "global"));

        let (tx, _rx) = mpsc::channel::<TrayCommand>();
        let tray = KnobTray {
            mode: manual,
            contextual_profile: "gimp".to_string(),
            profile_names: vec!["global".into(), "firefox".into(), "gimp".into()],
            tx,
        };
        let items = tray.menu();
        assert_eq!(items.len(), 5);
        assert!(matches!(items[1], MenuItem::Checkmark(ref c) if !c.checked));
        assert!(matches!(items[2], MenuItem::SubMenu(ref sub) if sub.submenu.len() == 3));
        assert_eq!(TRAY_ICON_ARGB.len(), 24 * 24 * 4);
    }
}
