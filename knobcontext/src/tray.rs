use crate::context::ProfileMode;

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

#[cfg(any(target_os = "windows", test))]
fn argb_to_rgba(argb: &[u8]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(argb.len());
    for px in argb.as_chunks::<4>().0 {
        rgba.push(px[1]);
        rgba.push(px[2]);
        rgba.push(px[3]);
        rgba.push(px[0]);
    }
    rgba
}

#[cfg(target_os = "linux")]
mod platform {
    use super::{
        format_tooltip, is_auto_checked, is_manual_profile_checked, TrayCommand, TRAY_ICON_ARGB,
    };
    use crate::context::ProfileMode;
    use ksni::blocking::{Handle, TrayMethods};
    use ksni::menu::{CheckmarkItem, StandardItem, SubMenu};
    use ksni::{Icon, MenuItem, ToolTip, Tray};
    use log::{info, warn};
    use std::sync::mpsc::Sender;

    pub struct KnobTray {
        pub(super) mode: ProfileMode,
        pub(super) contextual_profile: String,
        pub(super) profile_names: Vec<String>,
        pub(super) tx: Sender<TrayCommand>,
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

        pub fn shutdown(&mut self) {
            if let Some(handle) = self.handle.take() {
                handle.shutdown().wait();
            }
        }
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::{
        argb_to_rgba, format_tooltip, is_auto_checked, is_manual_profile_checked, TrayCommand,
        TRAY_ICON_ARGB,
    };
    use crate::context::ProfileMode;
    use log::{info, warn};
    use std::sync::mpsc::Sender;
    use tray_icon::menu::{
        CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu,
    };
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

    const ID_OPEN_SETTINGS: &str = "knobcontext.open_settings";
    const ID_MODE_AUTO: &str = "knobcontext.mode_auto";
    const ID_QUIT: &str = "knobcontext.quit";
    const ID_MANUAL_PREFIX: &str = "knobcontext.manual:";

    fn build_menu(mode: &ProfileMode, profile_names: &[String]) -> Menu {
        let menu = Menu::new();
        let open_item = MenuItem::with_id(
            MenuId::new(ID_OPEN_SETTINGS),
            "Abrir configurações",
            true,
            None,
        );
        let auto_item = CheckMenuItem::with_id(
            MenuId::new(ID_MODE_AUTO),
            "Modo Automático",
            true,
            is_auto_checked(mode),
            None,
        );
        let manual_submenu = Submenu::new("Modo Manual", true);
        for name in profile_names {
            let item = CheckMenuItem::with_id(
                MenuId::new(format!("{ID_MANUAL_PREFIX}{name}")),
                name,
                true,
                is_manual_profile_checked(mode, name),
                None,
            );
            let _ = manual_submenu.append(&item);
        }
        let sep = PredefinedMenuItem::separator();
        let quit_item = MenuItem::with_id(MenuId::new(ID_QUIT), "Sair", true, None);

        let _ = menu.append(&open_item);
        let _ = menu.append(&auto_item);
        let _ = menu.append(&manual_submenu);
        let _ = menu.append(&sep);
        let _ = menu.append(&quit_item);
        menu
    }

    pub struct TrayController {
        tray: Option<TrayIcon>,
        tx: Sender<TrayCommand>,
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
            let rgba = argb_to_rgba(TRAY_ICON_ARGB);
            let icon = Icon::from_rgba(rgba, 24, 24);
            let menu = build_menu(&mode, &profile_names);
            let tooltip = format_tooltip(&mode, &contextual_profile);

            let tray = match icon.map_err(|err| err.to_string()).and_then(|ic| {
                TrayIconBuilder::new()
                    .with_menu(Box::new(menu))
                    .with_tooltip(&tooltip)
                    .with_icon(ic)
                    .build()
                    .map_err(|err| err.to_string())
            }) {
                Ok(t) => {
                    info!("Ícone de tray nativo do Windows registrado.");
                    Some(t)
                }
                Err(err) => {
                    warn!("Não foi possível iniciar o ícone de tray no Windows: {err}. Continuando sem tray.");
                    None
                }
            };

            Self {
                tray,
                tx,
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
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                let id = event.id.0.as_str();
                if id == ID_OPEN_SETTINGS {
                    let _ = self.tx.send(TrayCommand::OpenSettings);
                } else if id == ID_MODE_AUTO {
                    let _ = self.tx.send(TrayCommand::SetMode(ProfileMode::Auto));
                } else if id == ID_QUIT {
                    let _ = self.tx.send(TrayCommand::Quit);
                } else if let Some(profile_name) = id.strip_prefix(ID_MANUAL_PREFIX) {
                    let _ = self.tx.send(TrayCommand::SetMode(ProfileMode::Manual(
                        profile_name.to_string(),
                    )));
                }
            }

            if self.last_mode == *mode
                && self.last_contextual == contextual_profile
                && self.last_profiles == profile_names
            {
                return;
            }

            self.last_mode = mode.clone();
            self.last_contextual = contextual_profile.to_string();
            self.last_profiles = profile_names.to_vec();

            if let Some(tray) = &self.tray {
                let tooltip = format_tooltip(mode, contextual_profile);
                let _ = tray.set_tooltip(Some(&tooltip));
                let menu = build_menu(mode, profile_names);
                tray.set_menu(Some(Box::new(menu)));
            }
        }

        pub fn shutdown(&mut self) {
            self.tray.take();
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod platform {
    use super::TrayCommand;
    use crate::context::ProfileMode;
    use std::sync::mpsc::Sender;

    pub struct TrayController;

    impl TrayController {
        pub fn spawn(
            _mode: ProfileMode,
            _contextual_profile: String,
            _profile_names: Vec<String>,
            _tx: Sender<TrayCommand>,
        ) -> Self {
            Self
        }

        pub fn sync_if_changed(
            &mut self,
            _mode: &ProfileMode,
            _contextual_profile: &str,
            _profile_names: &[String],
        ) {
        }

        pub fn shutdown(&mut self) {}
    }
}

pub use platform::TrayController;

#[cfg(test)]
mod tests {
    use super::{
        argb_to_rgba, format_tooltip, is_auto_checked, is_manual_profile_checked, TRAY_ICON_ARGB,
    };
    use crate::context::ProfileMode;

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
    fn converts_argb_icon_to_rgba() {
        assert_eq!(TRAY_ICON_ARGB.len(), 24 * 24 * 4);
        assert_eq!(
            argb_to_rgba(&[0xFF, 0x11, 0x22, 0x33]),
            vec![0x11, 0x22, 0x33, 0xFF]
        );
    }

    #[test]
    fn mode_checkmarks_match_expected_state() {
        assert!(is_auto_checked(&ProfileMode::Auto));
        assert!(!is_manual_profile_checked(&ProfileMode::Auto, "global"));

        let manual = ProfileMode::Manual("firefox".to_string());
        assert!(!is_auto_checked(&manual));
        assert!(is_manual_profile_checked(&manual, "firefox"));
        assert!(!is_manual_profile_checked(&manual, "global"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn menu_reflects_profile_mode_checkmarks() {
        use super::platform::KnobTray;
        use super::TrayCommand;
        use ksni::{MenuItem, Tray};
        use std::sync::mpsc;

        let manual = ProfileMode::Manual("firefox".to_string());
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
    }
}
