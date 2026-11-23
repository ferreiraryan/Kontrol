use hidapi::HidApi;
use std::env;

struct Filter {
    vid: Option<u16>,
    pid: Option<u16>,
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
            "--help" | "-h" => {
                println!("Uso: hid_probe [--vid <hex>] [--pid <hex>]");
                println!("  Sem argumentos: lista todos os dispositivos HID.");
                println!("  Com --vid/--pid: filtra e testa leitura (somente leitura) de Feature Reports.");
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
