use crate::error::KnobError;
use serde::Deserialize;
use std::collections::HashMap;
use std::process::Command;

/// Um canal controlável no Mixer de Áudio (Master ou aplicativo específico).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioStreamItem {
    pub id: String,
    pub is_master: bool,
    pub sink_name: Option<String>,
    pub sink_input_index: Option<u32>,
    pub title: String,
    pub subtitle: String,
    pub volume_percent: i32,
    pub muted: bool,
}

/// Dispositivo de saída de áudio (sink) do PipeWire/PulseAudio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioOutputItem {
    pub name: String,
    pub description: String,
    pub is_default: bool,
    pub volume_percent: i32,
    pub muted: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AudioMixerSnapshot {
    pub streams: Vec<AudioStreamItem>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AudioOutputsSnapshot {
    pub outputs: Vec<AudioOutputItem>,
}

#[derive(Deserialize)]
struct PactlVolumeChannel {
    #[serde(default)]
    value_percent: String,
}

#[derive(Deserialize)]
struct PactlSinkJson {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    mute: bool,
    #[serde(default)]
    volume: HashMap<String, PactlVolumeChannel>,
}

#[derive(Deserialize)]
struct PactlSinkInputJson {
    #[serde(default)]
    index: u32,
    #[serde(default)]
    mute: bool,
    #[serde(default)]
    volume: HashMap<String, PactlVolumeChannel>,
    #[serde(default)]
    properties: HashMap<String, serde_json::Value>,
}

fn parse_average_volume_percent(volume: &HashMap<String, PactlVolumeChannel>) -> i32 {
    if volume.is_empty() {
        return 0;
    }
    let mut sum = 0i32;
    let mut count = 0i32;
    for ch in volume.values() {
        let digits = ch.value_percent.trim().trim_end_matches('%').trim();
        if let Ok(val) = digits.parse::<i32>() {
            sum += val;
            count += 1;
        }
    }
    if count == 0 {
        0
    } else {
        (sum + count / 2) / count
    }
}

fn json_prop_str<'a>(map: &'a HashMap<String, serde_json::Value>, key: &str) -> Option<&'a str> {
    map.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

pub fn parse_pactl_sinks_json(
    raw: &str,
    default_sink: &str,
) -> Result<Vec<AudioOutputItem>, KnobError> {
    let sinks: Vec<PactlSinkJson> = serde_json::from_str(raw).map_err(|err| {
        KnobError::InputSimulation(format!(
            "falha ao parsear JSON de `pactl list sinks`: {err}"
        ))
    })?;
    let default_trimmed = default_sink.trim();
    let mut items: Vec<AudioOutputItem> = sinks
        .into_iter()
        .filter(|s| !s.name.trim().is_empty())
        .map(|s| {
            let desc = if s.description.trim().is_empty() {
                s.name.clone()
            } else {
                s.description.trim().to_string()
            };
            let is_default = !default_trimmed.is_empty() && s.name == default_trimmed;
            AudioOutputItem {
                name: s.name,
                description: desc,
                is_default,
                volume_percent: parse_average_volume_percent(&s.volume),
                muted: s.mute,
            }
        })
        .collect();

    // Saída padrão primeiro, depois ordem alfabética pela descrição
    items.sort_by(|a, b| {
        b.is_default.cmp(&a.is_default).then_with(|| {
            a.description
                .to_lowercase()
                .cmp(&b.description.to_lowercase())
        })
    });
    Ok(items)
}

pub fn parse_pactl_sink_inputs_json(raw: &str) -> Result<Vec<AudioStreamItem>, KnobError> {
    let inputs: Vec<PactlSinkInputJson> = serde_json::from_str(raw).map_err(|err| {
        KnobError::InputSimulation(format!(
            "falha ao parsear JSON de `pactl list sink-inputs`: {err}"
        ))
    })?;

    let mut items: Vec<AudioStreamItem> = inputs
        .into_iter()
        .map(|inp| {
            let app_name = json_prop_str(&inp.properties, "application.name")
                .or_else(|| json_prop_str(&inp.properties, "node.name"))
                .unwrap_or("Aplicativo")
                .to_string();
            let media_name = json_prop_str(&inp.properties, "media.name")
                .filter(|m| !m.eq_ignore_ascii_case(&app_name))
                .unwrap_or("Áudio do aplicativo")
                .to_string();
            AudioStreamItem {
                id: format!("app:{}", inp.index),
                is_master: false,
                sink_name: None,
                sink_input_index: Some(inp.index),
                title: app_name,
                subtitle: media_name,
                volume_percent: parse_average_volume_percent(&inp.volume),
                muted: inp.mute,
            }
        })
        .collect();

    items.sort_by(|a, b| {
        a.title
            .to_lowercase()
            .cmp(&b.title.to_lowercase())
            .then_with(|| a.sink_input_index.cmp(&b.sink_input_index))
    });
    Ok(items)
}

pub fn build_mixer_snapshot(
    outputs: &[AudioOutputItem],
    app_streams: Vec<AudioStreamItem>,
) -> AudioMixerSnapshot {
    let mut streams = Vec::with_capacity(app_streams.len() + 1);
    if let Some(master) = outputs
        .iter()
        .find(|o| o.is_default)
        .or_else(|| outputs.first())
    {
        streams.push(AudioStreamItem {
            id: format!("master:{}", master.name),
            is_master: true,
            sink_name: Some(master.name.clone()),
            sink_input_index: None,
            title: "Volume Global".to_string(),
            subtitle: master.description.clone(),
            volume_percent: master.volume_percent,
            muted: master.muted,
        });
    }
    streams.extend(app_streams);
    AudioMixerSnapshot { streams }
}

fn run_pactl(args: &[&str]) -> Result<String, KnobError> {
    let output = Command::new("pactl").args(args).output().map_err(|err| {
        KnobError::InputSimulation(format!(
            "falha ao executar `pactl {}`: {err}",
            args.join(" ")
        ))
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(KnobError::InputSimulation(format!(
            "`pactl {}` falhou: {}",
            args.join(" "),
            stderr.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn fetch_audio_outputs() -> Result<AudioOutputsSnapshot, KnobError> {
    let default_sink = run_pactl(&["get-default-sink"]).unwrap_or_default();
    let sinks_json = run_pactl(&["-f", "json", "list", "sinks"])?;
    let outputs = parse_pactl_sinks_json(&sinks_json, &default_sink)?;
    Ok(AudioOutputsSnapshot { outputs })
}

pub fn fetch_audio_mixer() -> Result<AudioMixerSnapshot, KnobError> {
    let outputs = fetch_audio_outputs()?.outputs;
    let inputs_json =
        run_pactl(&["-f", "json", "list", "sink-inputs"]).unwrap_or_else(|_| "[]".to_string());
    let apps = parse_pactl_sink_inputs_json(&inputs_json)?;
    Ok(build_mixer_snapshot(&outputs, apps))
}

pub fn set_master_volume(sink_name: &str, percent: i32, max_percent: i32) -> Result<(), KnobError> {
    let clamped = percent.clamp(0, max_percent.max(100));
    let target = if sink_name.trim().is_empty() {
        "@DEFAULT_SINK@"
    } else {
        sink_name
    };
    let val = format!("{clamped}%");
    let _ = run_pactl(&["set-sink-volume", target, &val])?;
    Ok(())
}

pub fn toggle_master_mute(sink_name: &str) -> Result<(), KnobError> {
    let target = if sink_name.trim().is_empty() {
        "@DEFAULT_SINK@"
    } else {
        sink_name
    };
    let _ = run_pactl(&["set-sink-mute", target, "toggle"])?;
    Ok(())
}

pub fn set_app_volume(index: u32, percent: i32, max_percent: i32) -> Result<(), KnobError> {
    let clamped = percent.clamp(0, max_percent.max(100));
    let idx_str = index.to_string();
    let val = format!("{clamped}%");
    let _ = run_pactl(&["set-sink-input-volume", &idx_str, &val])?;
    Ok(())
}

pub fn toggle_app_mute(index: u32) -> Result<(), KnobError> {
    let idx_str = index.to_string();
    let _ = run_pactl(&["set-sink-input-mute", &idx_str, "toggle"])?;
    Ok(())
}

pub fn set_default_audio_output(sink_name: &str) -> Result<(), KnobError> {
    let _ = run_pactl(&["set-default-sink", sink_name])?;
    // Move streams ativos para a nova saída para o som migrar imediatamente
    if let Ok(inputs_json) = run_pactl(&["-f", "json", "list", "sink-inputs"]) {
        if let Ok(apps) = parse_pactl_sink_inputs_json(&inputs_json) {
            for app in apps {
                if let Some(idx) = app.sink_input_index {
                    let idx_str = idx.to_string();
                    let _ = run_pactl(&["move-sink-input", &idx_str, sink_name]);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{build_mixer_snapshot, parse_pactl_sink_inputs_json, parse_pactl_sinks_json};

    #[test]
    fn parses_pactl_sinks_and_sorts_default_first() {
        let json = r#"[
            {
                "name": "alsa_output.usb-headset",
                "description": "Headset USB",
                "mute": true,
                "volume": {
                    "front-left": { "value_percent": "52%" },
                    "front-right": { "value_percent": "52%" }
                }
            },
            {
                "name": "alsa_output.hdmi",
                "description": "Monitor HDMI GA271",
                "mute": false,
                "volume": {
                    "front-left": { "value_percent": "100%" },
                    "front-right": { "value_percent": "100%" }
                }
            }
        ]"#;

        let outputs = parse_pactl_sinks_json(json, "alsa_output.hdmi").expect("sinks válidos");
        assert_eq!(outputs.len(), 2);
        assert!(outputs[0].is_default);
        assert_eq!(outputs[0].description, "Monitor HDMI GA271");
        assert_eq!(outputs[0].volume_percent, 100);
        assert!(!outputs[0].muted);
        assert!(!outputs[1].is_default);
        assert_eq!(outputs[1].volume_percent, 52);
        assert!(outputs[1].muted);
    }

    #[test]
    fn parses_pactl_sink_inputs_and_builds_mixer_snapshot() {
        let sinks_json = r#"[{
            "name": "alsa_output.hdmi",
            "description": "Monitor HDMI",
            "mute": false,
            "volume": { "front-left": { "value_percent": "80%" } }
        }]"#;
        let inputs_json = r#"[
            {
                "index": 42,
                "mute": false,
                "volume": { "front-left": { "value_percent": "65%" }, "front-right": { "value_percent": "65%" } },
                "properties": {
                    "application.name": "Firefox",
                    "media.name": "YouTube"
                }
            },
            {
                "index": 18,
                "mute": true,
                "volume": { "front-left": { "value_percent": "40%" } },
                "properties": {
                    "application.name": "Discord",
                    "media.name": "Discord"
                }
            }
        ]"#;

        let outputs = parse_pactl_sinks_json(sinks_json, "alsa_output.hdmi").expect("sinks");
        let apps = parse_pactl_sink_inputs_json(inputs_json).expect("sink-inputs");
        let mixer = build_mixer_snapshot(&outputs, apps);

        assert_eq!(mixer.streams.len(), 3);
        assert!(mixer.streams[0].is_master);
        assert_eq!(mixer.streams[0].volume_percent, 80);
        assert_eq!(mixer.streams[1].title, "Discord");
        assert!(mixer.streams[1].muted);
        assert_eq!(mixer.streams[2].title, "Firefox");
        assert_eq!(mixer.streams[2].subtitle, "YouTube");
        assert_eq!(mixer.streams[2].volume_percent, 65);
    }
}
