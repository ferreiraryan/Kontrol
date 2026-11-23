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
Fase 4C-fix (correções de bugs + ajustes de design) implementada, testada e validada no Hyprland Wayland. Pronto para validação manual final e preparação da Fase 5 (empacotamento e distribuição).

## Concluído
- **Fase 0**: probes `evdev` e `hidapi`, descoberta real do dispositivo `1189:8890`.
- **Fase 1 (núcleo sem UI)**: `error.rs`, `config.rs`, `input.rs` (`evdev::grab()` + `ungrab()` + ordem física confirmada `Key1`/`Key2`/`Key3`), `action.rs` (`enigo` Wayland).
- **Fase 2 (UI Overlay Slint)**: `ui/state.rs`, `ui/overlay.slint`, `ui/mod.rs` (overlay minimalista escuro com timeout de 3s).
- **Fase 3 (Contexto Hyprland + Settings Slint)**: `context.rs` (listener `socket2` em `UnixStream`), `ui/settings.slint`, `ui/settings_mod.rs` (hot-reload sem liberar `grab()`, sem conflito com overlay).
- **Fase 4C (Construtor visual de ações + ações ricas)**: `actions_catalog.rs`, `ui/action_editor.slint`, `ui/action_editor_mod.rs`, `ui/action_editor_state.rs` (9 tipos de ação, captura ao vivo, apps `.desktop`, `rfd` file chooser, macros, TOML em strings canônicas).
- **Fase 4C-fix (Estabilidade e Design)**:
  - **BUG 1 (tecla presa)**: criado `KeyReleaseGuard` (RAII) em `src/action.rs` garantindo `Direction::Release` em ordem reversa mesmo em erros no meio de atalhos/macros; `release_modifiers()` disparado ao cancelar/fechar captura no popover; `EventTranslator` em `src/input.rs` agora emite combos com `Ctrl` somente no `KEY_UP` do modificador (sem estado residual).
  - **BUG 2 (scroll lento no Wayland)**: descoberto que `enigo` envia pixels contínuos no `wlr_virtual_pointer::axis`; adicionado `device.scroll_step` (padrão `20`, configurável 1–200) multiplicando `Scroll:<dir>:<qtd>`.
  - **DESIGN 1 (`KnobPress` reservado)**: `KnobPress` é reservado exclusivamente para abrir/fechar o overlay; removido da tabela de ações nas Settings e ignorado no `ActionDispatcher`. Campos legados `knob_press` em TOMLs antigos geram warning único ao carregar e são omitidos ao salvar.
  - **DESIGN 2 (Modo Automático / Manual)**: adicionado `ProfileMode { Auto, Manual(String) }` volátil em `ContextState`; primeira opção do overlay é `"Automático"` (subtítulo `"Automático (<perfil>)"`), enquanto perfis nomeados ativam modo `"Manual"` que não é sobrescrito por mudanças de janela.
  - Validação: `cargo test` (41/41), `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`.

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
1. Validar interativamente o construtor visual de ações (`--settings` → `"Editar"`): testar `Launch:`, `Macro:`, captura ao vivo (inclusive `Super+E`), `Open:` (URL e arquivo via `rfd`) e `Scroll:`.
2. **Fase 5 (Empacotamento e distribuição)**:
   - Serviço `systemd --user` (`knobcontext.service`) + regra `udev` instalada no pacote.
   - Arquivo `.desktop` e ícone para abrir configurações diretamente do launcher.
   - `PKGBUILD` (Arch Linux) / binário release otimizado (`strip`, `lto`, verificação de RSS < 50MB).
   - Suporte multiplataforma Windows/X11 (`focus-tracker` + `hidapi`).
