use crate::error::KnobError;
use crate::system::hyprland::fetch_monitors;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorBrightnessItem {
    pub id: String,
    pub is_all: bool,
    pub monitor_name: Option<String>,
    pub title: String,
    pub subtitle: String,
    pub percent: i32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BrightnessSnapshot {
    pub monitors: Vec<MonitorBrightnessItem>,
}

/// Converte resposta do Caelestia IPC (`0.3`, `0.36`, `1.0`) para percentual `0..=100`.
pub fn parse_caelestia_brightness(raw: &str) -> Option<i32> {
    let val = raw.trim().parse::<f32>().ok()?;
    if val <= 1.0 {
        Some((val * 100.0).round().clamp(0.0, 100.0) as i32)
    } else {
        Some(val.round().clamp(0.0, 100.0) as i32)
    }
}

/// Converte saída `brightnessctl -m` (ex.: `intel_backlight,backlight,120,50%,240`) para `0..=100`.
pub fn parse_brightnessctl_machine(raw: &str) -> Option<(String, i32)> {
    let line = raw.lines().find(|l| !l.trim().is_empty())?;
    let parts: Vec<&str> = line.split(',').collect();
    if parts.len() < 4 {
        return None;
    }
    let name = parts[0].trim().to_string();
    let pct = parts[3].trim().trim_end_matches('%').parse::<i32>().ok()?;
    Some((name, pct.clamp(0, 100)))
}

fn run_qs_brightness(args: &[&str]) -> Result<String, KnobError> {
    let mut full_args = vec!["-c", "caelestia", "ipc", "call", "brightness"];
    full_args.extend_from_slice(args);
    let output = Command::new("qs")
        .args(&full_args)
        .output()
        .map_err(|err| {
            KnobError::InputSimulation(format!("falha ao executar `qs` brightness: {err}"))
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(KnobError::InputSimulation(format!(
            "`qs brightness` falhou: {}",
            stderr.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn fetch_brightness() -> Result<BrightnessSnapshot, KnobError> {
    // 1) Tenta Caelestia IPC + lista real de monitores do Hyprland (rápido e suporta DDC/backlight)
    if let Ok(monitors) = fetch_monitors() {
        let mut items = Vec::new();
        for mon in &monitors {
            if let Ok(out) = run_qs_brightness(&["getFor", &mon.name]) {
                if let Some(pct) = parse_caelestia_brightness(&out) {
                    let subtitle = if mon.model.is_empty() {
                        "Monitor (Caelestia)".to_string()
                    } else {
                        mon.model.clone()
                    };
                    items.push(MonitorBrightnessItem {
                        id: format!("mon:{}", mon.name),
                        is_all: false,
                        monitor_name: Some(mon.name.clone()),
                        title: mon.name.clone(),
                        subtitle,
                        percent: pct,
                    });
                }
            }
        }
        if !items.is_empty() {
            if items.len() > 1 {
                let avg = items.iter().map(|m| m.percent).sum::<i32>() / (items.len() as i32);
                items.insert(
                    0,
                    MonitorBrightnessItem {
                        id: "mon:all".to_string(),
                        is_all: true,
                        monitor_name: None,
                        title: "Todos os monitores".to_string(),
                        subtitle: format!("{} monitores conectados", items.len()),
                        percent: avg,
                    },
                );
            }
            return Ok(BrightnessSnapshot { monitors: items });
        }
    }

    // 2) Fallback: Caelestia `brightness get` genérico
    if let Ok(out) = run_qs_brightness(&["get"]) {
        if let Some(pct) = parse_caelestia_brightness(&out) {
            return Ok(BrightnessSnapshot {
                monitors: vec![MonitorBrightnessItem {
                    id: "mon:all".to_string(),
                    is_all: true,
                    monitor_name: None,
                    title: "Brilho Geral".to_string(),
                    subtitle: "Caelestia Shell".to_string(),
                    percent: pct,
                }],
            });
        }
    }

    // 3) Fallback: `brightnessctl -m` (notebooks sem Caelestia)
    let output = Command::new("brightnessctl")
        .arg("-m")
        .output()
        .map_err(|err| {
            KnobError::InputSimulation(format!(
                "falha ao consultar brilho (qs/brightnessctl): {err}"
            ))
        })?;
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some((dev, pct)) = parse_brightnessctl_machine(&stdout) {
            return Ok(BrightnessSnapshot {
                monitors: vec![MonitorBrightnessItem {
                    id: format!("backlight:{dev}"),
                    is_all: true,
                    monitor_name: None,
                    title: "Tela".to_string(),
                    subtitle: dev,
                    percent: pct,
                }],
            });
        }
    }

    Ok(BrightnessSnapshot::default())
}

pub fn set_brightness(monitor_name: Option<&str>, percent: i32) -> Result<(), KnobError> {
    let clamped = percent.clamp(0, 100);
    let val = format!("{clamped}%");

    let qs_res = match monitor_name {
        Some(name) if !name.trim().is_empty() => run_qs_brightness(&["setFor", name, &val]),
        _ => run_qs_brightness(&["set", &val]),
    };
    if qs_res.is_ok() {
        return Ok(());
    }

    // Fallback para brightnessctl
    let status = Command::new("brightnessctl")
        .args(["set", &val])
        .status()
        .map_err(|err| {
            KnobError::InputSimulation(format!("falha ao ajustar brilho para {val}: {err}"))
        })?;
    if !status.success() {
        return Err(KnobError::InputSimulation(format!(
            "`brightnessctl set {val}` retornou erro"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_brightnessctl_machine, parse_caelestia_brightness};

    #[test]
    fn parses_caelestia_float_and_percent_brightness() {
        assert_eq!(parse_caelestia_brightness("0.3\n"), Some(30));
        assert_eq!(parse_caelestia_brightness("0.36"), Some(36));
        assert_eq!(parse_caelestia_brightness("1.0"), Some(100));
        assert_eq!(parse_caelestia_brightness("0"), Some(0));
        assert_eq!(parse_caelestia_brightness("invalid"), None);
    }

    #[test]
    fn parses_brightnessctl_machine_output() {
        let sample = "intel_backlight,backlight,120,50%,240\n";
        assert_eq!(
            parse_brightnessctl_machine(sample),
            Some(("intel_backlight".to_string(), 50))
        );
    }
}
