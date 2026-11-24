pub mod audio;
pub mod brightness;
pub mod caelestia;
pub mod hyprland;
pub mod icons;
pub mod thumbs;

use crate::system::audio::{
    fetch_audio_mixer, fetch_audio_outputs, set_app_volume, set_default_audio_output,
    set_master_volume, toggle_app_mute, toggle_master_mute, AudioMixerSnapshot,
    AudioOutputsSnapshot,
};
use crate::system::brightness::{fetch_brightness, set_brightness, BrightnessSnapshot};
use crate::system::caelestia::{
    fetch_caelestia_toggles, fetch_wallpapers, set_random_wallpaper, set_wallpaper,
    trigger_caelestia_toggle, CaelestiaToggleKind, CaelestiaTogglesSnapshot, WallpapersSnapshot,
};
use crate::system::hyprland::{
    fetch_workspaces, position_hud_bottom_right, position_overlay_bottom_center, switch_workspace,
    toggle_special_workspace, WorkspacesSnapshot,
};
use crate::system::thumbs::{load_or_generate_thumbnail, DecodedThumbnail, ThumbnailRequest};
use log::warn;
use std::collections::VecDeque;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

/// Requisições assíncronas enviadas pela thread principal (UI) ao `SystemWorker`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemRequest {
    FetchAudioMixer,
    FetchAudioOutputs,
    FetchWorkspaces,
    FetchBrightness,
    FetchWallpapers,
    FetchCaelestiaToggles,
    SetMasterVolume {
        sink_name: String,
        percent: i32,
        max_percent: i32,
    },
    ToggleMasterMute {
        sink_name: String,
    },
    SetAppVolume {
        index: u32,
        percent: i32,
        max_percent: i32,
    },
    ToggleAppMute {
        index: u32,
    },
    SetDefaultAudioOutput {
        sink_name: String,
    },
    SwitchWorkspace {
        id: i32,
    },
    ToggleSpecialWorkspace,
    SetBrightness {
        monitor_name: Option<String>,
        percent: i32,
    },
    SetWallpaper {
        path: String,
    },
    SetRandomWallpaper,
    TriggerCaelestiaToggle {
        kind: CaelestiaToggleKind,
    },
    RunShellCommand {
        command: String,
    },
    ReadCustomSlider {
        item_id: String,
        read_command: String,
    },
    LoadThumbnail(ThumbnailRequest),
    PositionOverlay {
        width: i32,
        height: i32,
        bottom_margin: i32,
    },
    PositionHud {
        width: i32,
        height: i32,
        margin: i32,
    },
    Shutdown,
}

/// Respostas assíncronas enviadas pelo `SystemWorker` para a thread principal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemResponse {
    AudioMixer(AudioMixerSnapshot),
    AudioOutputs(AudioOutputsSnapshot),
    Workspaces(WorkspacesSnapshot),
    Brightness(BrightnessSnapshot),
    Wallpapers(WallpapersSnapshot),
    CaelestiaToggles(CaelestiaTogglesSnapshot),
    CustomSliderValue { item_id: String, percent: i32 },
    ThumbnailReady(DecodedThumbnail),
    Error(String),
}

/// Consolida requisições contínuas repetidas para o mesmo alvo na fila (evita lag em giros rápidos).
pub fn coalesce_request(queue: &mut VecDeque<SystemRequest>, incoming: SystemRequest) {
    match &incoming {
        SystemRequest::SetMasterVolume { sink_name, .. } => {
            queue.retain(|req| {
                !matches!(req, SystemRequest::SetMasterVolume { sink_name: s, .. } if s == sink_name)
            });
        }
        SystemRequest::SetAppVolume { index, .. } => {
            queue.retain(
                |req| !matches!(req, SystemRequest::SetAppVolume { index: i, .. } if i == index),
            );
        }
        SystemRequest::SetBrightness { monitor_name, .. } => {
            queue.retain(|req| {
                !matches!(req, SystemRequest::SetBrightness { monitor_name: m, .. } if m == monitor_name)
            });
        }
        SystemRequest::SwitchWorkspace { .. } => {
            queue.retain(|req| !matches!(req, SystemRequest::SwitchWorkspace { .. }));
        }
        SystemRequest::FetchAudioMixer
            if queue
                .iter()
                .any(|r| matches!(r, SystemRequest::FetchAudioMixer)) =>
        {
            return;
        }
        SystemRequest::FetchAudioOutputs
            if queue
                .iter()
                .any(|r| matches!(r, SystemRequest::FetchAudioOutputs)) =>
        {
            return;
        }
        SystemRequest::FetchWorkspaces
            if queue
                .iter()
                .any(|r| matches!(r, SystemRequest::FetchWorkspaces)) =>
        {
            return;
        }
        SystemRequest::FetchBrightness
            if queue
                .iter()
                .any(|r| matches!(r, SystemRequest::FetchBrightness)) =>
        {
            return;
        }
        SystemRequest::FetchWallpapers
            if queue
                .iter()
                .any(|r| matches!(r, SystemRequest::FetchWallpapers)) =>
        {
            return;
        }
        SystemRequest::FetchCaelestiaToggles
            if queue
                .iter()
                .any(|r| matches!(r, SystemRequest::FetchCaelestiaToggles)) =>
        {
            return;
        }
        SystemRequest::LoadThumbnail(req)
            if queue.iter().any(
                |r| matches!(r, SystemRequest::LoadThumbnail(existing) if existing == req),
            ) =>
        {
            return;
        }
        SystemRequest::PositionOverlay { .. } => {
            queue.retain(|req| !matches!(req, SystemRequest::PositionOverlay { .. }));
        }
        SystemRequest::PositionHud { .. } => {
            queue.retain(|req| !matches!(req, SystemRequest::PositionHud { .. }));
        }
        _ => {}
    }
    queue.push_back(incoming);
}

/// Worker em thread dedicada para chamadas de sistema sem bloquear o loop de UI do Slint.
pub struct SystemWorker {
    tx: Sender<SystemRequest>,
    rx: Receiver<SystemResponse>,
    handle: Option<JoinHandle<()>>,
}

impl SystemWorker {
    pub fn spawn() -> Self {
        let (req_tx, req_rx) = mpsc::channel::<SystemRequest>();
        let (res_tx, res_rx) = mpsc::channel::<SystemResponse>();

        let handle = thread::spawn(move || {
            let mut queue: VecDeque<SystemRequest> = VecDeque::new();
            loop {
                if queue.is_empty() {
                    match req_rx.recv() {
                        Ok(req) => coalesce_request(&mut queue, req),
                        Err(_) => break,
                    }
                }
                while let Ok(req) = req_rx.try_recv() {
                    coalesce_request(&mut queue, req);
                }

                let Some(next) = queue.pop_front() else {
                    continue;
                };
                if matches!(next, SystemRequest::Shutdown) {
                    break;
                }
                if let Some(resp) = handle_system_request(next) {
                    if res_tx.send(resp).is_err() {
                        break;
                    }
                }
            }
        });

        Self {
            tx: req_tx,
            rx: res_rx,
            handle: Some(handle),
        }
    }

    pub fn send(&self, request: SystemRequest) {
        let _ = self.tx.send(request);
    }

    pub fn try_recv(&self) -> Option<SystemResponse> {
        self.rx.try_recv().ok()
    }

    pub fn shutdown(&mut self) {
        let _ = self.tx.send(SystemRequest::Shutdown);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn handle_system_request(req: SystemRequest) -> Option<SystemResponse> {
    match req {
        SystemRequest::FetchAudioMixer => match fetch_audio_mixer() {
            Ok(snap) => Some(SystemResponse::AudioMixer(snap)),
            Err(err) => {
                warn!("Erro ao carregar mixer de áudio: {err}");
                Some(SystemResponse::Error(err.to_string()))
            }
        },
        SystemRequest::FetchAudioOutputs => match fetch_audio_outputs() {
            Ok(snap) => Some(SystemResponse::AudioOutputs(snap)),
            Err(err) => {
                warn!("Erro ao carregar saídas de áudio: {err}");
                Some(SystemResponse::Error(err.to_string()))
            }
        },
        SystemRequest::FetchWorkspaces => match fetch_workspaces() {
            Ok(snap) => Some(SystemResponse::Workspaces(snap)),
            Err(err) => {
                warn!("Erro ao carregar workspaces do Hyprland: {err}");
                Some(SystemResponse::Error(err.to_string()))
            }
        },
        SystemRequest::FetchBrightness => match fetch_brightness() {
            Ok(snap) => Some(SystemResponse::Brightness(snap)),
            Err(err) => {
                warn!("Erro ao carregar brilho dos monitores: {err}");
                Some(SystemResponse::Error(err.to_string()))
            }
        },
        SystemRequest::FetchWallpapers => match fetch_wallpapers() {
            Ok(snap) => Some(SystemResponse::Wallpapers(snap)),
            Err(err) => {
                warn!("Erro ao listar wallpapers do Caelestia: {err}");
                Some(SystemResponse::Error(err.to_string()))
            }
        },
        SystemRequest::FetchCaelestiaToggles => match fetch_caelestia_toggles() {
            Ok(snap) => Some(SystemResponse::CaelestiaToggles(snap)),
            Err(err) => {
                warn!("Erro ao consultar controles do Caelestia: {err}");
                Some(SystemResponse::Error(err.to_string()))
            }
        },
        SystemRequest::SetMasterVolume {
            sink_name,
            percent,
            max_percent,
        } => {
            if let Err(err) = set_master_volume(&sink_name, percent, max_percent) {
                warn!("Erro ao ajustar volume global: {err}");
                return Some(SystemResponse::Error(err.to_string()));
            }
            None
        }
        SystemRequest::ToggleMasterMute { sink_name } => {
            if let Err(err) = toggle_master_mute(&sink_name) {
                warn!("Erro ao alternar mudo global: {err}");
                return Some(SystemResponse::Error(err.to_string()));
            }
            fetch_audio_mixer().ok().map(SystemResponse::AudioMixer)
        }
        SystemRequest::SetAppVolume {
            index,
            percent,
            max_percent,
        } => {
            if let Err(err) = set_app_volume(index, percent, max_percent) {
                warn!("Erro ao ajustar volume do app {index}: {err}");
                return Some(SystemResponse::Error(err.to_string()));
            }
            None
        }
        SystemRequest::ToggleAppMute { index } => {
            if let Err(err) = toggle_app_mute(index) {
                warn!("Erro ao alternar mudo do app {index}: {err}");
                return Some(SystemResponse::Error(err.to_string()));
            }
            fetch_audio_mixer().ok().map(SystemResponse::AudioMixer)
        }
        SystemRequest::SetDefaultAudioOutput { sink_name } => {
            if let Err(err) = set_default_audio_output(&sink_name) {
                warn!("Erro ao trocar saída de som para '{sink_name}': {err}");
                return Some(SystemResponse::Error(err.to_string()));
            }
            fetch_audio_outputs().ok().map(SystemResponse::AudioOutputs)
        }
        SystemRequest::SwitchWorkspace { id } => {
            if let Err(err) = switch_workspace(id) {
                warn!("Erro ao trocar para workspace {id}: {err}");
                return Some(SystemResponse::Error(err.to_string()));
            }
            fetch_workspaces().ok().map(SystemResponse::Workspaces)
        }
        SystemRequest::ToggleSpecialWorkspace => {
            if let Err(err) = toggle_special_workspace() {
                warn!("Erro ao alternar special workspace: {err}");
                return Some(SystemResponse::Error(err.to_string()));
            }
            None
        }
        SystemRequest::SetBrightness {
            monitor_name,
            percent,
        } => {
            if let Err(err) = set_brightness(monitor_name.as_deref(), percent) {
                warn!("Erro ao ajustar brilho: {err}");
                return Some(SystemResponse::Error(err.to_string()));
            }
            None
        }
        SystemRequest::SetWallpaper { path } => {
            if let Err(err) = set_wallpaper(&path) {
                warn!("Erro ao aplicar wallpaper '{path}': {err}");
                return Some(SystemResponse::Error(err.to_string()));
            }
            fetch_wallpapers().ok().map(SystemResponse::Wallpapers)
        }
        SystemRequest::SetRandomWallpaper => {
            if let Err(err) = set_random_wallpaper() {
                warn!("Erro ao aplicar wallpaper aleatório: {err}");
                return Some(SystemResponse::Error(err.to_string()));
            }
            fetch_wallpapers().ok().map(SystemResponse::Wallpapers)
        }
        SystemRequest::TriggerCaelestiaToggle { kind } => {
            if let Err(err) = trigger_caelestia_toggle(kind) {
                warn!("Erro ao alternar controle Caelestia {kind:?}: {err}");
                return Some(SystemResponse::Error(err.to_string()));
            }
            fetch_caelestia_toggles()
                .ok()
                .map(SystemResponse::CaelestiaToggles)
        }
        SystemRequest::RunShellCommand { command } => {
            if let Err(err) = Command::new("sh").arg("-c").arg(&command).status() {
                warn!("Erro ao executar comando customizado '{command}': {err}");
                return Some(SystemResponse::Error(err.to_string()));
            }
            None
        }
        SystemRequest::ReadCustomSlider {
            item_id,
            read_command,
        } => match Command::new("sh").arg("-c").arg(&read_command).output() {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                let digits = text.trim().trim_end_matches('%').trim();
                let percent = digits
                    .parse::<f32>()
                    .map(|v| v.round() as i32)
                    .unwrap_or(0)
                    .clamp(0, 100);
                Some(SystemResponse::CustomSliderValue { item_id, percent })
            }
            Ok(_) | Err(_) => None,
        },
        SystemRequest::LoadThumbnail(req) => match load_or_generate_thumbnail(&req) {
            Ok(decoded) => Some(SystemResponse::ThumbnailReady(decoded)),
            Err(err) => {
                log::debug!("Thumbnail ignorado para '{}': {err}", req.path);
                None
            }
        },
        SystemRequest::PositionOverlay {
            width,
            height,
            bottom_margin,
        } => {
            if let Err(err) = position_overlay_bottom_center(width, height, bottom_margin) {
                log::debug!("Posicionamento automático do overlay no Hyprland ignorado: {err}");
            }
            None
        }
        SystemRequest::PositionHud {
            width,
            height,
            margin,
        } => {
            if let Err(err) = position_hud_bottom_right(width, height, margin) {
                log::debug!("Posicionamento automático do Mini-HUD no Hyprland ignorado: {err}");
            }
            None
        }
        SystemRequest::Shutdown => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{coalesce_request, SystemRequest};
    use std::collections::VecDeque;

    #[test]
    fn coalesces_repeated_volume_and_brightness_updates_for_same_target() {
        let mut queue = VecDeque::new();
        coalesce_request(
            &mut queue,
            SystemRequest::SetAppVolume {
                index: 10,
                percent: 55,
                max_percent: 150,
            },
        );
        coalesce_request(
            &mut queue,
            SystemRequest::SetAppVolume {
                index: 10,
                percent: 70,
                max_percent: 150,
            },
        );
        coalesce_request(
            &mut queue,
            SystemRequest::SetBrightness {
                monitor_name: Some("DP-3".into()),
                percent: 40,
            },
        );
        coalesce_request(
            &mut queue,
            SystemRequest::SetBrightness {
                monitor_name: Some("DP-3".into()),
                percent: 55,
            },
        );

        assert_eq!(queue.len(), 2);
        assert_eq!(
            queue[0],
            SystemRequest::SetAppVolume {
                index: 10,
                percent: 70,
                max_percent: 150,
            }
        );
        assert_eq!(
            queue[1],
            SystemRequest::SetBrightness {
                monitor_name: Some("DP-3".into()),
                percent: 55,
            }
        );
    }
}
