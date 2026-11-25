use hidapi::HidApi;
use std::env;

struct Filter {
    vid: Option<u16>,
    pid: Option<u16>,
    monitor: bool,
}

fn parse_hex_u16(input: &str) -> Result<u16, String> {
    let trimmed = input
        .trim()
        .trim_start_matches("0x")
        .trim_start_matches("0X");
    u16::from_str_radix(trimmed, 16)
        .map_err(|_| format!("valor hexadecimal inválido para VID/PID: '{input}'"))
}

fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Filter, String> {
    let mut filter = Filter {
        vid: None,
        pid: None,
        monitor: false,
    };
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--vid" => {
                let val = iter
                    .next()
                    .ok_or_else(|| "faltando valor após --vid".to_string())?;
                filter.vid = Some(parse_hex_u16(&val)?);
            }
            "--pid" => {
                let val = iter
                    .next()
                    .ok_or_else(|| "faltando valor após --pid".to_string())?;
                filter.pid = Some(parse_hex_u16(&val)?);
            }
            "--monitor" => {
                filter.monitor = true;
            }
            "--help" | "-h" => {
                println!("Uso: hid_probe [--vid <hex>] [--pid <hex>] [--monitor]");
                println!("  Sem argumentos: lista todos os dispositivos HID.");
                println!("  Com --vid/--pid: filtra e testa leitura (somente leitura) de Feature Reports.");
                println!(
                    "  Com --monitor: escuta relatórios HID (0x000C) e eventos WH_KEYBOARD_LL."
                );
                std::process::exit(0);
            }
            other => return Err(format!("argumento desconhecido: {other}")),
        }
    }
    Ok(filter)
}

fn probe_feature_reports(api: &HidApi, path: &std::ffi::CStr) {
    match api.open_path(path) {
        Ok(device) => {
            println!("    Abertura hidraw: OK (acesso disponível)");
            let mut found_any = false;
            for report_id in 0u8..=8u8 {
                let mut buf = [0u8; 65];
                buf[0] = report_id;
                if let Ok(len) = device.get_feature_report(&mut buf) {
                    found_any = true;
                    let preview_len = len.min(16);
                    println!(
                        "    Feature Report ID 0x{report_id:02x}: OK ({len} bytes) -> {:02x?}",
                        &buf[..preview_len]
                    );
                }
            }
            if !found_any {
                println!("    Feature Reports (IDs 0x00..=0x08): nenhum retornado por get_feature_report.");
            }
        }
        Err(err) => {
            println!("    Abertura hidraw: BLOQUEADA ({err})");
            println!("    Dica: verifique permissões de /dev/hidraw* (regra udev ou sudo).");
        }
    }
}

fn main() {
    let filter = match parse_args(env::args().skip(1)) {
        Ok(f) => f,
        Err(err) => {
            eprintln!("Erro: {err}");
            std::process::exit(1);
        }
    };

    let api = match HidApi::new() {
        Ok(api) => api,
        Err(err) => {
            eprintln!("Erro ao inicializar HidApi: {err}");
            std::process::exit(1);
        }
    };

    println!("=== Dispositivos HID detectados ===");
    let mut matched = 0usize;

    for info in api.device_list() {
        if let Some(vid) = filter.vid {
            if info.vendor_id() != vid {
                continue;
            }
        }
        if let Some(pid) = filter.pid {
            if info.product_id() != pid {
                continue;
            }
        }

        matched += 1;
        println!(
            "- VID:PID {:04x}:{:04x} | iface={} | usage_page=0x{:04x} usage=0x{:04x}",
            info.vendor_id(),
            info.product_id(),
            info.interface_number(),
            info.usage_page(),
            info.usage()
        );
        println!(
            "    Fabricante: {:?} | Produto: {:?} | Serial: {:?}",
            info.manufacturer_string(),
            info.product_string(),
            info.serial_number()
        );
        println!("    Path: {}", info.path().to_string_lossy());

        if filter.vid.is_some() || filter.pid.is_some() {
            probe_feature_reports(&api, info.path());
        }
    }

    if matched == 0 {
        println!("Nenhum dispositivo HID correspondeu ao filtro.");
    } else if filter.vid.is_none() && filter.pid.is_none() {
        println!(
            "\nTotal: {matched} interface(s) HID. Passe --vid <hex> --pid <hex> para testar abertura e leitura de Feature Reports."
        );
    }

    println!("\nNota: nenhuma escrita HID foi enviada (modo somente leitura).");

    if filter.monitor {
        monitor_events(&api, &filter);
    }
}

#[cfg(target_os = "windows")]
fn monitor_events(api: &HidApi, filter: &Filter) {
    use std::ptr;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage,
        UnhookWindowsHookEx, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, WH_KEYBOARD_LL, WM_KEYDOWN,
        WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    static HOOK_TX: std::sync::Mutex<Option<mpsc::Sender<String>>> = std::sync::Mutex::new(None);

    unsafe extern "system" fn diag_kb_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let msg = wparam as u32;
            let state = match msg {
                WM_KEYDOWN | WM_SYSKEYDOWN => Some("DOWN"),
                WM_KEYUP | WM_SYSKEYUP => Some("UP"),
                _ => None,
            };
            if let Some(st) = state {
                let kb = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
                let injected = (kb.flags & LLKHF_INJECTED) != 0;
                if let Ok(guard) = HOOK_TX.lock() {
                    if let Some(tx) = guard.as_ref() {
                        let _ = tx.send(format!(
                            "[WH_KEYBOARD_LL] {st} vk=0x{:02X} scan=0x{:02X} flags=0x{:02X} injected={injected}",
                            kb.vkCode, kb.scanCode, kb.flags
                        ));
                    }
                }
            }
        }
        unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
    }

    let mut hid_devices = Vec::new();
    for info in api.device_list() {
        if let Some(vid) = filter.vid {
            if info.vendor_id() != vid {
                continue;
            }
        }
        if let Some(pid) = filter.pid {
            if info.product_id() != pid {
                continue;
            }
        }
        if info.usage_page() == 0x000c || info.usage_page() == 0xff00 {
            if let Ok(dev) = api.open_path(info.path()) {
                let _ = dev.set_blocking_mode(false);
                println!(
                    "[monitor] HID aberto: iface={} usage_page=0x{:04x} usage=0x{:04x}",
                    info.interface_number(),
                    info.usage_page(),
                    info.usage()
                );
                hid_devices.push((info.interface_number(), info.usage_page(), dev));
            }
        }
    }

    let (tx, rx) = mpsc::channel::<String>();
    if let Ok(mut guard) = HOOK_TX.lock() {
        *guard = Some(tx);
    }

    thread::spawn(move || unsafe {
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(diag_kb_proc), ptr::null_mut(), 0);
        if hook.is_null() {
            eprintln!("[monitor] Falha ao instalar WH_KEYBOARD_LL");
            return;
        }
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        UnhookWindowsHookEx(hook);
    });

    println!("\n=== Monitorando eventos (gire e clique o knob; Ctrl+C para sair) ===");
    let mut buf = [0u8; 64];
    loop {
        for (iface, up, dev) in &hid_devices {
            while let Ok(n) = dev.read(&mut buf) {
                if n == 0 {
                    break;
                }
                println!(
                    "[HID iface={iface} up=0x{up:04x}] {} bytes: {:02x?}",
                    n,
                    &buf[..n]
                );
            }
        }
        while let Ok(line) = rx.try_recv() {
            println!("{line}");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(not(target_os = "windows"))]
fn monitor_events(_api: &HidApi, _filter: &Filter) {
    println!("--monitor está disponível apenas no Windows.");
}

#[cfg(test)]
mod tests {
    use super::{parse_args, parse_hex_u16};

    #[test]
    fn parses_hex_with_or_without_prefix() {
        assert_eq!(parse_hex_u16("1189"), Ok(0x1189));
        assert_eq!(parse_hex_u16("0x8890"), Ok(0x8890));
        assert_eq!(parse_hex_u16("0X00ff"), Ok(0x00ff));
        assert!(parse_hex_u16("zzzz").is_err());
    }

    #[test]
    fn parses_cli_filter_args() {
        let filter = parse_args(vec![
            "--vid".to_string(),
            "1189".to_string(),
            "--pid".to_string(),
            "0x8890".to_string(),
        ])
        .expect("argumentos válidos");
        assert_eq!(filter.vid, Some(0x1189));
        assert_eq!(filter.pid, Some(0x8890));
    }
}
