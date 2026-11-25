#[cfg(target_os = "linux")]
use crate::context::hyprland_socket_paths;
use crate::error::KnobError;
use serde::Deserialize;
#[cfg(target_os = "linux")]
use std::io::{Read, Write};
#[cfg(target_os = "linux")]
use std::os::unix::net::UnixStream;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceItem {
    pub id: i32,
    pub name: String,
    pub monitor: String,
    pub windows: u32,
    pub is_active: bool,
    pub is_new_slot: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WorkspacesSnapshot {
    pub workspaces: Vec<WorkspaceItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MonitorInfo {
    pub name: String,
    pub model: String,
    pub focused: bool,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub scale: f32,
    pub transform: i32,
}

#[derive(Deserialize)]
struct HyprWorkspaceJson {
    id: i32,
    #[serde(default)]
    name: String,
    #[serde(default)]
    monitor: String,
    #[serde(default)]
    windows: u32,
}

#[derive(Deserialize)]
struct HyprMonitorJson {
    #[serde(default)]
    name: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    focused: bool,
    #[serde(default)]
    x: i32,
    #[serde(default)]
    y: i32,
    #[serde(default)]
    width: i32,
    #[serde(default)]
    height: i32,
    #[serde(default = "default_scale")]
    scale: f32,
    #[serde(default)]
    transform: i32,
}

fn default_scale() -> f32 {
    1.0
}

#[cfg(target_os = "linux")]
fn hypr_socket1_command(cmd: &str) -> Result<String, KnobError> {
    let Some((socket1_path, _)) = hyprland_socket_paths() else {
        return Err(KnobError::InputSimulation(
            "HYPRLAND_INSTANCE_SIGNATURE ausente; Hyprland IPC indisponível.".to_string(),
        ));
    };
    let mut stream = UnixStream::connect(&socket1_path).map_err(|err| {
        KnobError::InputSimulation(format!(
            "falha ao conectar em {}: {err}",
            socket1_path.display()
        ))
    })?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(400)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(400)));
    stream.write_all(cmd.as_bytes()).map_err(|err| {
        KnobError::InputSimulation(format!("falha ao enviar comando Hyprland '{cmd}': {err}"))
    })?;
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    Ok(response)
}

#[cfg(not(target_os = "linux"))]
fn hypr_socket1_command(_cmd: &str) -> Result<String, KnobError> {
    Err(KnobError::InputSimulation(
        "Hyprland IPC disponível apenas no Linux.".to_string(),
    ))
}

pub fn parse_workspaces_json(
    workspaces_raw: &str,
    active_raw: &str,
) -> Result<WorkspacesSnapshot, KnobError> {
    let parsed: Vec<HyprWorkspaceJson> = serde_json::from_str(workspaces_raw).map_err(|err| {
        KnobError::InputSimulation(format!("falha ao parsear JSON de workspaces: {err}"))
    })?;
    let active_id = serde_json::from_str::<HyprWorkspaceJson>(active_raw)
        .map(|w| w.id)
        .unwrap_or(-1);

    let mut workspaces: Vec<WorkspaceItem> = parsed
        .into_iter()
        .filter(|w| w.id > 0)
        .map(|w| {
            let name = if w.name.trim().is_empty() {
                w.id.to_string()
            } else {
                w.name.trim().to_string()
            };
            WorkspaceItem {
                id: w.id,
                name,
                monitor: w.monitor,
                windows: w.windows,
                is_active: w.id == active_id,
                is_new_slot: false,
            }
        })
        .collect();

    workspaces.sort_by_key(|w| w.id);

    // Adiciona o primeiro ID livre (1..=10) para facilitar abrir um workspace novo pelo knob
    let next_free_id = (1..=20)
        .find(|cand| !workspaces.iter().any(|w| w.id == *cand))
        .unwrap_or(workspaces.last().map(|w| w.id + 1).unwrap_or(1));
    workspaces.push(WorkspaceItem {
        id: next_free_id,
        name: format!("Novo ({next_free_id})"),
        monitor: "Workspace livre".to_string(),
        windows: 0,
        is_active: false,
        is_new_slot: true,
    });

    Ok(WorkspacesSnapshot { workspaces })
}

pub fn parse_monitors_json(raw: &str) -> Result<Vec<MonitorInfo>, KnobError> {
    let parsed: Vec<HyprMonitorJson> = serde_json::from_str(raw).map_err(|err| {
        KnobError::InputSimulation(format!("falha ao parsear JSON de monitors: {err}"))
    })?;
    let mut monitors: Vec<MonitorInfo> = parsed
        .into_iter()
        .filter(|m| !m.name.trim().is_empty())
        .map(|m| MonitorInfo {
            name: m.name.trim().to_string(),
            model: m.model.trim().to_string(),
            focused: m.focused,
            x: m.x,
            y: m.y,
            width: m.width,
            height: m.height,
            scale: if m.scale > 0.1 { m.scale } else { 1.0 },
            transform: m.transform,
        })
        .collect();
    monitors.sort_by(|a, b| b.focused.cmp(&a.focused).then_with(|| a.name.cmp(&b.name)));
    Ok(monitors)
}

/// Calcula a posição `(x, y)` na parte inferior central do monitor, respeitando `scale` e `transform`.
#[cfg(any(target_os = "linux", test))]
pub fn compute_bottom_center_position(
    mon: &MonitorInfo,
    win_w: i32,
    win_h: i32,
    bottom_margin: i32,
) -> (i32, i32) {
    let rotated = matches!(mon.transform % 4, 1 | 3);
    let raw_w = if rotated { mon.height } else { mon.width };
    let raw_h = if rotated { mon.width } else { mon.height };
    let scale = if mon.scale > 0.1 { mon.scale } else { 1.0 };
    let logical_w = ((raw_w as f32) / scale).round() as i32;
    let logical_h = ((raw_h as f32) / scale).round() as i32;

    let x = mon.x + ((logical_w - win_w) / 2).max(0);
    let y = mon.y + (logical_h - win_h - bottom_margin.max(0)).max(0);
    (x, y)
}

#[cfg(target_os = "linux")]
#[derive(Deserialize)]
struct HyprClientJson {
    #[serde(default)]
    address: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    at: [i32; 2],
}

#[cfg(target_os = "linux")]
fn find_client_by_title(title: &str) -> Option<HyprClientJson> {
    let raw = hypr_socket1_command("j/clients").ok()?;
    let clients: Vec<HyprClientJson> = serde_json::from_str(&raw).ok()?;
    clients
        .into_iter()
        .find(|c| c.title == title && !c.address.trim().is_empty())
}

/// Calcula a posição `(x, y)` no canto inferior direito do monitor, respeitando `scale` e `transform`.
#[cfg(target_os = "linux")]
pub fn compute_bottom_right_position(
    mon: &MonitorInfo,
    win_w: i32,
    win_h: i32,
    margin: i32,
) -> (i32, i32) {
    let rotated = matches!(mon.transform % 4, 1 | 3);
    let raw_w = if rotated { mon.height } else { mon.width };
    let raw_h = if rotated { mon.width } else { mon.height };
    let scale = if mon.scale > 0.1 { mon.scale } else { 1.0 };
    let logical_w = ((raw_w as f32) / scale).round() as i32;
    let logical_h = ((raw_h as f32) / scale).round() as i32;

    let m = margin.max(0);
    let x = mon.x + (logical_w - win_w - m).max(0);
    let y = mon.y + (logical_h - win_h - m).max(0);
    (x, y)
}

/// Registra dinamicamente a regra de janela do Overlay no Hyprland 0.56+ (Lua) para que
/// ele já abra na parte inferior central do monitor focado desde o 1º frame, sem bordas,
/// sem sombra, fixado (`pin`) e sem roubar foco (`no_initial_focus`), sobrescrevendo regras
/// globais como `center = true`.
pub fn register_overlay_window_rule(bottom_margin: i32) {
    let margin = bottom_margin.max(0);
    let lua_rule = format!(
        "eval hl.window_rule({{ name = \"knobcontext-overlay\", match = {{ title = \"^(KnobContext Overlay)$\" }}, float = true, pin = true, center = false, move = \"((monitor_w-window_w)/2) (monitor_h-window_h-{margin})\", no_initial_focus = true, border_size = 0, no_shadow = true, opacity = \"1 override\" }})"
    );
    let _ = hypr_socket1_command(&lua_rule);
}

/// Registra dinamicamente a regra de janela do Mini-HUD (`KnobContext HUD`) no canto inferior direito,
/// sem borda/sombra e sem roubar o foco do aplicativo ativo.
pub fn register_hud_window_rule(margin: i32) {
    let m = margin.max(0);
    let lua_rule = format!(
        "eval hl.window_rule({{ name = \"knobcontext-hud\", match = {{ title = \"^(KnobContext HUD)$\" }}, float = true, pin = true, center = false, move = \"(monitor_w-window_w-{m}) (monitor_h-window_h-{m})\", no_initial_focus = true, border_size = 0, no_shadow = true, opacity = \"1 override\" }})"
    );
    let _ = hypr_socket1_command(&lua_rule);
}

#[cfg(target_os = "linux")]
fn move_titled_window_to(title: &str, x: i32, y: i32) {
    for attempt in 0..12 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(20));
        }
        if let Some(client) = find_client_by_title(title) {
            if (client.at[0] - x).abs() <= 2 && (client.at[1] - y).abs() <= 2 {
                return;
            }
            let lua_cmd = format!(
                "eval hl.dispatch(hl.dsp.window.move({{ x = {x}, y = {y}, relative = false, window = \"address:{}\" }}))",
                client.address
            );
            if let Ok(resp) = hypr_socket1_command(&lua_cmd) {
                if resp.trim().eq_ignore_ascii_case("ok") {
                    return;
                }
            }
            let legacy_cmd = format!(
                "dispatch movewindowpixel exact {x} {y},address:{}",
                client.address
            );
            let _ = hypr_socket1_command(&legacy_cmd);
            return;
        }
    }
}

/// Posiciona a janela `KnobContext Overlay` na parte inferior central do monitor focado via IPC.
#[cfg(target_os = "linux")]
pub fn position_overlay_bottom_center(
    win_w: i32,
    win_h: i32,
    bottom_margin: i32,
) -> Result<(), KnobError> {
    let monitors = fetch_monitors()?;
    let Some(target) = monitors
        .iter()
        .find(|m| m.focused)
        .or_else(|| monitors.first())
    else {
        return Ok(());
    };
    let (x, y) = compute_bottom_center_position(target, win_w, win_h, bottom_margin);
    move_titled_window_to("KnobContext Overlay", x, y);
    Ok(())
}

/// Posiciona a janela `KnobContext HUD` no canto inferior direito do monitor focado via IPC.
#[cfg(target_os = "linux")]
pub fn position_hud_bottom_right(win_w: i32, win_h: i32, margin: i32) -> Result<(), KnobError> {
    let monitors = fetch_monitors()?;
    let Some(target) = monitors
        .iter()
        .find(|m| m.focused)
        .or_else(|| monitors.first())
    else {
        return Ok(());
    };
    let (x, y) = compute_bottom_right_position(target, win_w, win_h, margin);
    move_titled_window_to("KnobContext HUD", x, y);
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn position_hud_bottom_right(win_w: i32, win_h: i32, margin: i32) -> Result<(), KnobError> {
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetForegroundWindow, SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOSIZE,
    };

    let title_wide: Vec<u16> = "KnobContext HUD"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    for attempt in 0..12 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(20));
        }
        // SAFETY: `title_wide` é uma string UTF-16 terminada em nulo válida.
        let hwnd = unsafe { FindWindowW(std::ptr::null(), title_wide.as_ptr()) };
        if hwnd.is_null() {
            continue;
        }
        // SAFETY: APIs Win32 padrão com ponteiros/handles válidos.
        unsafe {
            let fg = GetForegroundWindow();
            let ref_hwnd = if fg.is_null() { hwnd } else { fg };
            let hmon = MonitorFromWindow(ref_hwnd, MONITOR_DEFAULTTONEAREST);
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(hmon, &mut mi) != 0 {
                let work = mi.rcWork;
                let mon_w = work.right - work.left;
                let mon_h = work.bottom - work.top;
                let m = margin.max(0);
                let x = work.left + (mon_w - win_w - m).max(0);
                let y = work.top + (mon_h - win_h - m).max(0);
                SetWindowPos(hwnd, HWND_TOPMOST, x, y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
        return Ok(());
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn position_hud_bottom_right(_win_w: i32, _win_h: i32, _margin: i32) -> Result<(), KnobError> {
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn position_overlay_bottom_center(
    win_w: i32,
    win_h: i32,
    bottom_margin: i32,
) -> Result<(), KnobError> {
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetForegroundWindow, SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOSIZE,
    };

    let title_wide: Vec<u16> = "KnobContext Overlay"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    for attempt in 0..12 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(20));
        }
        // SAFETY: `title_wide` é uma string UTF-16 terminada em nulo válida.
        let hwnd = unsafe { FindWindowW(std::ptr::null(), title_wide.as_ptr()) };
        if hwnd.is_null() {
            continue;
        }
        // SAFETY: APIs Win32 padrão com ponteiros/handles válidos.
        unsafe {
            let fg = GetForegroundWindow();
            let ref_hwnd = if fg.is_null() { hwnd } else { fg };
            let hmon = MonitorFromWindow(ref_hwnd, MONITOR_DEFAULTTONEAREST);
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(hmon, &mut mi) != 0 {
                let work = mi.rcWork;
                let mon_w = work.right - work.left;
                let mon_h = work.bottom - work.top;
                let x = work.left + ((mon_w - win_w) / 2).max(0);
                let y = work.top + (mon_h - win_h - bottom_margin.max(0)).max(0);
                SetWindowPos(hwnd, HWND_TOPMOST, x, y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
        return Ok(());
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn position_overlay_bottom_center(
    _win_w: i32,
    _win_h: i32,
    _bottom_margin: i32,
) -> Result<(), KnobError> {
    Ok(())
}

pub fn fetch_workspaces() -> Result<WorkspacesSnapshot, KnobError> {
    let all = hypr_socket1_command("j/workspaces")?;
    let active = hypr_socket1_command("j/activeworkspace").unwrap_or_default();
    parse_workspaces_json(&all, &active)
}

pub fn fetch_monitors() -> Result<Vec<MonitorInfo>, KnobError> {
    let raw = hypr_socket1_command("j/monitors")?;
    parse_monitors_json(&raw)
}

pub fn switch_workspace(id: i32) -> Result<(), KnobError> {
    let lua_cmd = format!("eval hl.dispatch(hl.dsp.focus({{ workspace = \"{id}\" }}))");
    if let Ok(resp) = hypr_socket1_command(&lua_cmd) {
        if resp.trim().eq_ignore_ascii_case("ok") {
            return Ok(());
        }
    }
    let _ = hypr_socket1_command(&format!("dispatch workspace {id}"))?;
    Ok(())
}

pub fn toggle_special_workspace() -> Result<(), KnobError> {
    let lua_cmd = "eval hl.dispatch(hl.dsp.focus({ workspace = \"special:special\" }))";
    if let Ok(resp) = hypr_socket1_command(lua_cmd) {
        if resp.trim().eq_ignore_ascii_case("ok") {
            return Ok(());
        }
    }
    let _ = hypr_socket1_command("dispatch togglespecialworkspace")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_monitors_json, parse_workspaces_json};

    #[test]
    fn parses_workspaces_sorted_and_appends_next_free() {
        let all = r#"[
            {"id": 6, "name": "6", "monitor": "HDMI-A-1", "windows": 2},
            {"id": -99, "name": "special:magic", "monitor": "DP-3", "windows": 1},
            {"id": 1, "name": "1", "monitor": "DP-3", "windows": 1},
            {"id": 2, "name": "2", "monitor": "DP-3", "windows": 3}
        ]"#;
        let active = r#"{"id": 2, "name": "2", "monitor": "DP-3", "windows": 3}"#;

        let snap = parse_workspaces_json(all, active).expect("workspaces válidos");
        assert_eq!(snap.workspaces.len(), 4);
        assert_eq!(snap.workspaces[0].id, 1);
        assert_eq!(snap.workspaces[1].id, 2);
        assert!(snap.workspaces[1].is_active);
        assert_eq!(snap.workspaces[2].id, 6);
        assert_eq!(snap.workspaces[3].id, 3);
        assert!(snap.workspaces[3].is_new_slot);
    }

    #[test]
    fn parses_monitors_focused_first_and_computes_bottom_center() {
        let raw = r#"[
            {"name": "HDMI-A-1", "model": "MCR-VLK24BL01", "focused": false, "x": 2560, "y": 0, "width": 1920, "height": 1080, "scale": 1.0, "transform": 3},
            {"name": "DP-3", "model": "GA271", "focused": true, "x": 0, "y": 900, "width": 2560, "height": 1440, "scale": 1.0, "transform": 0}
        ]"#;
        let monitors = parse_monitors_json(raw).expect("monitors válidos");
        assert_eq!(monitors.len(), 2);
        assert_eq!(monitors[0].name, "DP-3");
        assert!(monitors[0].focused);
        assert_eq!(monitors[1].name, "HDMI-A-1");

        // Monitor horizontal DP-3 (2560x1440 em y=900), overlay 960x256, margem 20
        assert_eq!(
            super::compute_bottom_center_position(&monitors[0], 960, 256, 20),
            (800, 900 + 1440 - 256 - 20)
        );

        // Monitor vertical HDMI-A-1 (transform=3 -> lógico 1080x1920 em x=2560, y=0)
        assert_eq!(
            super::compute_bottom_center_position(&monitors[1], 960, 256, 20),
            (2560 + 60, 1920 - 256 - 20)
        );
    }
}
