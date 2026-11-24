pub mod audio;

use crate::system::audio::{
    fetch_audio_mixer, fetch_audio_outputs, set_app_volume, set_default_audio_output,
    set_master_volume, toggle_app_mute, toggle_master_mute, AudioMixerSnapshot,
    AudioOutputsSnapshot,
};
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
    RunShellCommand {
        command: String,
    },
    ReadCustomSlider {
        item_id: String,
        read_command: String,
    },
    Shutdown,
}

/// Respostas assíncronas enviadas pelo `SystemWorker` para a thread principal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemResponse {
    AudioMixer(AudioMixerSnapshot),
    AudioOutputs(AudioOutputsSnapshot),
    CustomSliderValue { item_id: String, percent: i32 },
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
        SystemRequest::Shutdown => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{coalesce_request, SystemRequest};
    use std::collections::VecDeque;

    #[test]
    fn coalesces_repeated_volume_updates_for_same_target() {
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
                percent: 60,
                max_percent: 150,
            },
        );
        coalesce_request(
            &mut queue,
            SystemRequest::SetAppVolume {
                index: 20,
                percent: 80,
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

        assert_eq!(queue.len(), 2);
        assert_eq!(
            queue[0],
            SystemRequest::SetAppVolume {
                index: 20,
                percent: 80,
                max_percent: 150,
            }
        );
        assert_eq!(
            queue[1],
            SystemRequest::SetAppVolume {
                index: 10,
                percent: 70,
                max_percent: 150,
            }
        );
    }
}
