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
Fase 3 (perfis contextuais via Hyprland `socket2` + janela de configurações Slint) implementada, testada e validada no Hyprland Wayland. Pronto para teste interativo do usuário e preparação da Fase 4 (ações ricas + empacotamento).

## Concluído
- **Fase 0**:
  - Criação de `AGENTS.md` e `CONTEXT.md`.
  - Criação do pacote Rust `knobcontext/` e binários `probe` (`evdev`) e `hid_probe` (`hidapi`).
  - Mapeamento real das 4 interfaces USB e eventos `EV_KEY` do dispositivo `1189:8890`.
- **Fase 1 (núcleo sem UI)**:
  - `src/error.rs`: enum `KnobError` centralizado com `thiserror`.
  - `src/config.rs`: configuração TOML (`serde` + `toml`) carregada de `~/.config/knobcontext/config.toml`, com perfis padrão (`global`, `firefox`, `gimp`), `window_class`, parser bidirecional de ações em texto e persistência `save_to_path` / `save_default_location`.
  - `src/input.rs`: `evdev::Device::grab()` + `Drop` com `ungrab()`, multiplexação via `poll`, máquina de estados `EventTranslator` com janela de 200ms (`Key1` esquerda = `Ctrl+F1`, `Key2` meio = `Ctrl+F4`, `Key3` direita = `KEY_PLAYPAUSE`, `Key1+Key2` simultâneos = `OpenSettings`).
  - `src/action.rs`: trait `ActionExecutor`, `ActionDispatcher::dispatch(event, profile_name)` e `EnigoExecutor` (`enigo 0.6` Wayland).
- **Fase 2 (UI Overlay Slint)**:
  - `src/ui/state.rs`: máquina de estados pura `OverlayController` (Normal ↔ Overlay, timeout de 3s, bloqueio de abertura quando Settings está aberta).
  - `src/ui/overlay.slint` + `src/ui/mod.rs`: overlay minimalista escuro em `slint 1.19` (`backend-winit-wayland` + `renderer-software`).
- **Fase 3 (Contexto Hyprland + Configurações Slint)**:
  - **3A (`src/context.rs`)**: listener dedicado em `UnixStream` no `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket2.sock` (zero dependências extras, reconexão automática, consulta inicial via `.socket.sock` e fallback silencioso para `"global"` quando fora do Hyprland). `ContextState::profile_for_window` com testes unitários.
  - **3B (`src/ui/settings.slint`, `src/ui/settings_mod.rs`, `src/ui/app.slint`)**: janela de configurações Slint no mesmo tema escuro, com lista de perfis, edição de nome/`window_class`/6 ações, botões Adicionar/Remover/Salvar/Cancelar, validação antes de salvar, persistência via `toml::to_string_pretty` em `~/.config/knobcontext/config.toml` e hot-reload em tempo real sem liberar `grab()` e sem conflitar com o overlay.
  - Abertura via CLI `--settings` ou `Key1 + Key2` simultâneos.
  - Validação: `cargo test` (29/29), `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, e smoke test real no Hyprland Wayland com `--settings` + `.socket2.sock` + `grab()` + `SIGINT`.

## Hardware e Descobertas HID
- **VID:PID**: `1189:8890` (4 interfaces USB HID):
  - **Interface 0** (`hidraw8`, `/dev/input/by-id/usb-1189_8890-event-kbd`): Boot Keyboard (`EP 1 IN`, 64B), `evdev` suporta `EV_KEY`. Emite os atalhos de teclado das teclas configuradas.
  - **Interface 1** (sem `hidraw` no kernel): HID genérico (`SubClass=0, Protocol=0`) com **`EP 2 OUT` (64B)** — canal de escrita para reprogramação do firmware.
  - **Interface 2** (`hidraw9`, `/dev/input/by-id/usb-1189_8890-if02-event-kbd`): Keyboard (Report ID `0x01`) + Consumer Control (`usage_page=0x000c`, Report ID `0x02`), `evdev` suporta `EV_KEY + EV_REL`. Emite os eventos de mídia do knob e tecla de mídia.
  - **Interface 3** (`hidraw10`, `/dev/input/by-id/usb-1189_8890-if03-event-mouse`): Boot Mouse (3 botões + X/Y/Wheel), não emite eventos com o keymap atual.
- **Formato real dos eventos capturados via `evdev`** (importante: o knob envia `EV_KEY`, não `EV_REL`!):
  - **Clique do knob** (`usb-1189_8890-if02-event-kbd`): `KEY_MUTE` (`code=113`, `value=1`/`0`, `MSC_SCAN=786658`)
  - **Giro do knob (sentido +)** (`usb-1189_8890-if02-event-kbd`): `KEY_VOLUMEUP` (`code=115`, `value=1`/`0`, `MSC_SCAN=786665`)
  - **Giro do knob (sentido -)** (`usb-1189_8890-if02-event-kbd`): `KEY_VOLUMEDOWN` (`code=114`, `value=1`/`0`, `MSC_SCAN=786666`)
  - **Tecla 1 (esquerda)** (`usb-1189_8890-event-kbd`): `KEY_LEFTCTRL` (`code=29`) + `KEY_F1` (`code=59`) (`value=1`/`0`)
  - **Tecla 2 (meio)** (`usb-1189_8890-event-kbd`): `KEY_LEFTCTRL` (`code=29`) + `KEY_F4` (`code=62`) (`value=1`/`0`)
  - **Tecla 3 (direita)** (`usb-1189_8890-if02-event-kbd`): `KEY_PLAYPAUSE` (`code=164`, `value=1`/`0`, `MSC_SCAN=786637`)
- **Decisão sobre reset de atalhos**:
  - Os Report Descriptors de `hidraw8..10` não declaram Feature Reports (`0xB1`); a escrita no firmware ocorre apenas pela Interface 1 (`EP 2 OUT`, 64B).
  - Como todas as 6 ações físicas já emitem eventos `EV_KEY` distintos, usar **`evdev::Device::grab()` (`EVIOCGRAB`)** em `usb-1189_8890-event-kbd` e `usb-1189_8890-if02-event-kbd` intercepta e bloqueia os atalhos atuais no nível do SO (evitando vazamento de `Ctrl+F4`/`Ctrl+F1`/volume para o Wayland), sem precisar regravar o firmware na v1.0.
- **Permissões no ambiente**:
  - `/dev/input/event23..25`: `root:input` (leitura e `grab()` via `evdev` liberados para o usuário atual).
  - `/dev/hidraw*`: `root:root` (`0600`).

## Próximos Passos
1. Validar interativamente a troca contextual de perfis (Firefox, GIMP, fallback `global`) e a janela de configurações (`--settings` ou `Key1+Key2`).
2. **Fase 4 (Ações ricas + Empacotamento)**:
   - Ações avançadas (comandos shell/D-Bus, controle direto PipeWire/WirePlumber, macros sequenciais).
   - Serviço `systemd --user` + empacotamento (PKGBUILD / `.desktop` / ícone).
   - Suporte multiplataforma Windows/X11 (`focus-tracker` + `hidapi`).
