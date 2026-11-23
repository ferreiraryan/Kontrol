# CONTEXT.md — KnobContext

## Objetivo do Projeto
**KnobContext** é um aplicativo leve em Rust para ler eventos HID brutos de um dispositivo knob USB (3 teclas + botão rotativo), mapear ações (como volume, tamanho do pincel, atalhos) e abrir um overlay minimalista ao pressionar o botão. Também deve suportar a redefinição/gerenciamento da configuração de atalhos do dispositivo.

### Escopo da v1.0 (Linux Wayland)
- **Mapeador global de knob**: no Linux Wayland, a v1.0 **não implementa detecção de janela em primeiro plano** (todos os eventos respondem globalmente, sem distinção de aplicativo).
- A detecção de aplicativo em primeiro plano será ativada em versão subsequente (Windows / X11).

## Stack Tecnológico
- **Linguagem**: Rust (edição 2021+)
- **UI**: Slint (estilo minimalista escuro, cantos arredondados, animações suaves)
- **HID / Entrada bruta**: `evdev` (Linux) e `hidapi` (Windows / investigação de Feature Reports)
- **Simulação de entrada**: `enigo`
- **Configuração**: TOML
- **Meta de memória**: < 50MB

## Limitações Conhecidas
- **Wayland**: a crate `focus-tracker` não suporta detecção de janela em foco no Wayland. Por isso, a v1.0 opera em modo global no Linux Wayland.

## Status Atual
Fase 0 em andamento: probes compilados, testados e executados para descoberta de interfaces/descritores; falta capturar os eventos reais de giro/clique com interação física do usuário.

## Concluído
- Criação de `AGENTS.md` e `CONTEXT.md` na raiz do projeto.
- Criação do pacote Rust `knobcontext/` com dependências `evdev` (0.12.2) e `hidapi` (2.6.7).
- Implementação de `knobcontext/src/bin/probe.rs` (descoberta via `/dev/input/by-id/` e captura de eventos `evdev`) e `knobcontext/src/bin/hid_probe.rs` (enumeração HID e inspeção somente-leitura via `hidapi`).
- Validação de build e qualidade: `cargo test` (3/3), `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`.
- Inspeção real de interfaces USB (`lsusb -d 1189:8890 -v`), `hid_probe` e Report Descriptors (`sysfs`).

## Hardware e Descobertas HID
- **VID:PID**: `1189:8890` (4 interfaces USB HID):
  - **Interface 0** (`hidraw8`, `/dev/input/by-id/usb-1189_8890-event-kbd`): Boot Keyboard (`EP 1 IN`, 64B), `evdev` suporta `EV_KEY`.
  - **Interface 1** (sem `hidraw` no kernel): HID genérico (`SubClass=0, Protocol=0`) com **`EP 2 OUT` (64B)** — canal de saída/escrita para configuração do firmware.
  - **Interface 2** (`hidraw9`, `/dev/input/by-id/usb-1189_8890-if02-event-kbd`): Keyboard (Report ID `0x01`) + Consumer Control (`usage_page=0x000c`, Report ID `0x02`), `evdev` suporta `EV_KEY + EV_REL`.
  - **Interface 3** (`hidraw10`, `/dev/input/by-id/usb-1189_8890-if03-event-mouse`): Boot Mouse (3 botões + X/Y/Wheel), `evdev` suporta `EV_KEY + EV_REL`.
- **Suporte a escrita HID / Feature Reports**:
  - Os Report Descriptors de `hidraw8..10` não declaram Feature Reports (`0xB1`).
  - A escrita/reprogramação do firmware ocorre pela **Interface 1 (`EP 2 OUT`, 64B)** (requer permissão root/regra `udev` e protocolo específico de escrita, ainda não investigado/enviado).
  - Alternativa no nível do SO: `evdev::Device::grab()` (`EVIOCGRAB`) intercepta exclusivamente as interfaces `event*` do `1189:8890` sem deixar os atalhos atuais chegarem ao Wayland — desde que cada tecla/rotação emita eventos distintos.
- **Permissões no ambiente**:
  - `/dev/input/event23..25`: `root:input` (leitura `evdev` liberada para o usuário atual).
  - `/dev/hidraw*`: `root:root` (`0600`) — acesso `hidapi` requer `sudo` ou regra `udev`.
- **Formato de eventos `evdev` (rotação/teclas)**: pendente de captura interativa (girar/clicar no knob com `probe`).

## Próximos Passos
1. Capturar eventos reais do knob com `probe` (giro horário, anti-horário, clique do knob e 3 teclas) e registrar os códigos/valores exatos neste arquivo.
2. Confirmar com o usuário se cada controle já emite eventos distintos (permitindo usar `evdev` + `grab()` sem tocar no firmware) ou se precisamos implementar reset/reprogramação pela Interface 1 (`EP 2 OUT`).
3. Após concluir a Fase 0, propor a arquitetura da Fase 1 (mapeador global Wayland com TOML + `enigo` + overlay Slint).
