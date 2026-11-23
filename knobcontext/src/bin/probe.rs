use evdev::{Device, EventType};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread;

const BY_ID_DIR: &str = "/dev/input/by-id";

#[derive(Debug, Clone)]
struct InputDeviceInfo {
    by_id_path: PathBuf,
    name: String,
    vendor: u16,
    product: u16,
    has_key: bool,
    has_rel: bool,
}

fn is_event_symlink_target(target: &Path) -> bool {
    target
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("event"))
}

fn discover_by_id_devices(dir: &Path) -> io::Result<Vec<InputDeviceInfo>> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    entries.sort();

    let mut devices = Vec::new();
    for path in entries {
        let Ok(target) = fs::read_link(&path) else {
            continue;
        };
        if !is_event_symlink_target(&target) {
            continue;
        }

        match Device::open(&path) {
            Ok(dev) => {
                let events = dev.supported_events();
                let id = dev.input_id();
                devices.push(InputDeviceInfo {
                    by_id_path: path,
                    name: dev.name().unwrap_or("<sem nome>").to_string(),
                    vendor: id.vendor(),
                    product: id.product(),
                    has_key: events.contains(EventType::KEY),
                    has_rel: events.contains(EventType::RELATIVE),
                });
            }
            Err(err) => {
                eprintln!("[aviso] Falha ao abrir {}: {err}", path.display());
            }
        }
    }

    Ok(devices)
}

fn print_devices(devices: &[InputDeviceInfo]) {
    println!("=== Dispositivos em {BY_ID_DIR} ===");
    if devices.is_empty() {
        println!("Nenhum dispositivo de evento encontrado.");
        return;
    }

    for dev in devices {
        let caps = match (dev.has_key, dev.has_rel) {
            (true, true) => "EV_KEY + EV_REL  [CANDIDATO KNOB/MOUSE]",
            (true, false) => "EV_KEY",
            (false, true) => "EV_REL",
            (false, false) => "(outros)",
        };
        println!(
            "- {}\n    Nome: {} | VID:PID {:04x}:{:04x} | Suporte: {}",
            dev.by_id_path.display(),
            dev.name,
            dev.vendor,
            dev.product,
            caps
        );
    }
}

fn monitor_device(path: PathBuf) {
    let mut device = match Device::open(&path) {
        Ok(dev) => dev,
        Err(err) => {
            eprintln!("[erro] Não foi possível abrir {}: {err}", path.display());
            return;
        }
    };

    let label = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("device")
        .to_string();

    println!(
        "[escutando] {} ({})",
        path.display(),
        device.name().unwrap_or("<sem nome>")
    );

    loop {
        match device.fetch_events() {
            Ok(events) => {
                for ev in events {
                    if ev.event_type() == EventType::SYNCHRONIZATION {
                        continue;
                    }
                    println!(
                        "[{label}] type={:?} code={} kind={:?} value={}",
                        ev.event_type(),
                        ev.code(),
                        ev.kind(),
                        ev.value()
                    );
                }
            }
            Err(err) => {
                eprintln!("[erro] Leitura encerrada em {label}: {err}");
                break;
            }
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    let devices = match discover_by_id_devices(Path::new(BY_ID_DIR)) {
        Ok(list) => list,
        Err(err) => {
            eprintln!("Erro ao listar {BY_ID_DIR}: {err}");
            std::process::exit(1);
        }
    };

    print_devices(&devices);

    let targets: Vec<PathBuf> = if !args.is_empty() {
        args.into_iter().map(PathBuf::from).collect()
    } else {
        devices
            .iter()
            .filter(|d| d.has_key && d.has_rel)
            .map(|d| d.by_id_path.clone())
            .collect()
    };

    if targets.is_empty() {
        println!("\nNenhum dispositivo com EV_KEY + EV_REL selecionado automaticamente.");
        println!("Passe um ou mais caminhos /dev/input/by-id/... como argumento para monitorar.");
        return;
    }

    println!(
        "\nMonitorando {} dispositivo(s). Gire/pressione o knob (Ctrl+C para sair)...",
        targets.len()
    );

    let mut handles = Vec::with_capacity(targets.len());
    for target in targets {
        handles.push(thread::spawn(move || monitor_device(target)));
    }

    for handle in handles {
        let _ = handle.join();
    }
}

#[cfg(test)]
mod tests {
    use super::is_event_symlink_target;
    use std::path::Path;

    #[test]
    fn recognizes_event_targets_only() {
        assert!(is_event_symlink_target(Path::new("../event23")));
        assert!(is_event_symlink_target(Path::new("../event0")));
        assert!(!is_event_symlink_target(Path::new("../../hidraw8")));
        assert!(!is_event_symlink_target(Path::new("../mouse3")));
    }
}
