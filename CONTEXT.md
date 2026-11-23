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
Fase 2 (overlay minimalista escuro em Slint) implementada, testada e validada no Linux Wayland (Hyprland). Ordem física das teclas corrigida e documentada. Pronto para teste interativo do usuário e preparação da Fase 3 (detecção de janela em primeiro plano no Windows/X11).

## Concluído
- **Fase 0**:
  - Criação de `AGENTS.md` e `CONTEXT.md`.
  - Criação do pacote Rust `knobcontext/` e binários `probe` (`evdev`) e `hid_probe` (`hidapi`).
  - Mapeamento real das 4 interfaces USB e eventos `EV_KEY` do dispositivo `1189:8890`.
- **Fase 1 (núcleo sem UI)**:
  - `src/error.rs`: enum `KnobError` centralizado com `thiserror` e helper `is_permission_denied()`.
  - `src/config.rs`: configuração TOML (`serde` + `toml`) carregada de `~/.config/knobcontext/config.toml` com fallback para `Config::default()` (`Volume`, `Pincel`, `Scroll`) e validação.
  - `src/input.rs`: abertura e `grab()` exclusivo via `evdev` nas 2 interfaces do knob (`event-kbd` e `if02-event-kbd`), multiplexação via `poll`, `Drop` com `ungrab()` garantido, e máquina de estados `EventTranslator` com janela de 200ms para combos. Ordem física confirmada: `Key1` = esquerda (`Ctrl+F1`), `Key2` = meio (`Ctrl+F4`), `Key3` = direita (`KEY_PLAYPAUSE`).
  - `src/action.rs`: trait `ActionExecutor`, `ActionDispatcher` e `EnigoExecutor` usando `enigo 0.6` com backend nativo `wayland`.
- **Fase 2 (UI Overlay Slint)**:
  - `src/ui/state.rs`: máquina de estados pura `OverlayController` (Normal ↔ Overlay, navegação circular, confirmação por clique e timeout de 3s) coberta por testes unitários sem dependência do Slint.
  - `src/ui/overlay.slint` + `build.rs` + `src/ui/mod.rs`: overlay minimalista escuro (`rgba(20, 20, 20, 0.85)`, borda `rgba(255, 255, 255, 0.1)`, cantos arredondados `12px`, `no-frame`, `always-on-top`, `background: transparent`, animações suaves de 150ms) usando `slint 1.19` com `backend-winit-wayland` + `renderer-software` para manter consumo de memória baixo.
  - `src/main.rs`: integração do event loop Slint na thread principal com thread dedicada de `evdev`, timer de 20ms para drenar eventos e verificar timeout de 3s, e shutdown gracioso em `Ctrl+C` fechando o overlay e executando `ungrab()`.
  - `README.md`: documentação de uso, permissões `input`, exemplo TOML e regras `windowrulev2` do Hyprland (`float`, `pin`, `center`, `nofocus`, `noborder`, `noshadow`) para contornar a limitação do `winit` (`xdg_toplevel`) no Wayland.
  - Validação: `cargo test` (19/19), `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, e smoke test real no Hyprland Wayland.

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
1. Validar interativamente o overlay Slint no Hyprland (aplicando as `windowrulev2` do `README.md` para `float`, `pin`, `center` e `nofocus`).
2. **Fase 3 (Detecção de janela em primeiro plano — Windows / X11)**:
   - Abstrair backend de foco (`focus_tracker` ou equivalente em Windows/X11, com no-op global no Wayland).
   - Associar perfis no TOML a aplicativos específicos (`app_match`) mantendo fallback global.
   - Suporte a leitura HID no Windows via `hidapi`.
3. *(Opcional futuro)* Utilitário de reprogramação do firmware via Interface 1 (`EP 2 OUT`).
