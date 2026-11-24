use crate::context::hyprland_socket_paths;
use crate::error::KnobError;
use serde::Deserialize;
use std::io::{Read, Write};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorInfo {
    pub name: String,
    pub model: String,
    pub focused: bool,
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
}

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
        })
        .collect();
    monitors.sort_by(|a, b| b.focused.cmp(&a.focused).then_with(|| a.name.cmp(&b.name)));
    Ok(monitors)
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
    let _ = hypr_socket1_command(&format!("dispatch workspace {id}"))?;
    Ok(())
}

pub fn toggle_special_workspace() -> Result<(), KnobError> {
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
    fn parses_monitors_focused_first() {
        let raw = r#"[
            {"name": "HDMI-A-1", "model": "MCR-VLK24BL01", "focused": false},
            {"name": "DP-3", "model": "GA271", "focused": true}
        ]"#;
        let monitors = parse_monitors_json(raw).expect("monitors válidos");
        assert_eq!(monitors.len(), 2);
        assert_eq!(monitors[0].name, "DP-3");
        assert!(monitors[0].focused);
        assert_eq!(monitors[1].name, "HDMI-A-1");
    }
}
