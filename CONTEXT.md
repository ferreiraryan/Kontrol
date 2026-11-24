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
**v2 (Fases A, B, C e D)** concluída e validada: Modo Automático Adaptativo por contexto (`class` + `title` + `fullscreen` + `priority` + `inherits` + captura de janela + presets), Hub Interativo Hierárquico no overlay com modo `Adjusting` travado, `SystemWorker` assíncrono com coalescing, módulos nativos de **Áudio (Global + por App)**, **Saída de Som**, **Workspaces Hyprland**, **Brilho (Caelestia IPC + fallback `brightnessctl`)**, **Wallpapers & Controles Caelestia**, e aba **Overlay / Hub** totalmente customizável nas Configurações.

## Concluído
- **Fase 0**: probes `evdev` e `hidapi`, descoberta real do dispositivo `1189:8890`.
- **Fase 1 (núcleo sem UI)**: `error.rs`, `config.rs`, `input.rs` (`evdev::grab()` + `ungrab()` + ordem física confirmada `Key1`/`Key2`/`Key3`), `action.rs` (`enigo` Wayland).
- **Fase 2 (UI Overlay Slint)**: overlay minimalista escuro e ciclo de vida seguro no Wayland.
- **Fase 3 (Contexto Hyprland + Settings Slint)**: `context.rs` (listener `socket2` em `UnixStream`), `ui/settings.slint`, `ui/settings_mod.rs` (hot-reload sem liberar `grab()`).
- **Fase 4C + 4C-fix**: construtor visual de ações (9 tipos, captura ao vivo, apps `.desktop`, `rfd`, macros), `KeyReleaseGuard` (RAII), `device.scroll_step`, `KnobPress` reservado ao overlay, e modo `Automático` / `Manual`.
- **Fase 5 (Distribuição, Tray SNI e Persistência)**:
  - Persistência de `ProfileMode` (`mode = "auto"` ou `"manual:<perfil>"`) no `config.toml`.
  - Tray SNI (`src/tray.rs` via `ksni 0.3`), AppImage (`packaging/`), aviso udev e recriação de janela Slint ao abrir (garantindo floating consistente no Hyprland).
- **v2 — Fase A (Contexto Adaptativo)**:
  - `ActiveWindowInfo { class, title, fullscreen }` via `j/activewindow` + eventos `activewindow>>` e `fullscreen>>` do Hyprland.
  - Regras compostas (`window_class` com múltiplos padrões `|`/wildcards `*`, `window_title`, `fullscreen`, `priority`, `inherits` com detecção de ciclo), botão *"Capturar"* última janela ativa e biblioteca de presets contextuais prontos.
- **v2 — Fase B (Hub Interativo + Áudio PipeWire/`pactl`)**:
  - `src/overlay_hub.rs` (`HubController`: navegação em camadas + modo `Adjusting` travado com atualização otimista e timeout adaptativo) e novo `ui/overlay.slint` com breadcrumb, barras de progresso e rodapé contextual de teclas (`Knob/K3` confirmar/travar, `K1` voltar/fechar, `K2` ação secundária).
  - `src/system/mod.rs` (`SystemWorker` assíncrono com coalescing de rajadas) e `src/system/audio.rs` (`pactl -f json`: Volume Global + por App + Mute + troca de saída padrão com migração de streams).
- **v2 — Fase C (Workspaces, Brilho e Caelestia Dots)**:
  - `src/system/hyprland.rs`: consulta `j/workspaces`/`j/monitors` e troca direta via `.socket.sock` + `K2` special workspace.
  - `src/system/brightness.rs`: brilho por monitor (`DP-3`, `HDMI-A-1`, *Todos*) via `qs -c caelestia ipc call brightness` (com fallback `brightnessctl`).
  - `src/system/caelestia.rs`: lista/seleção de wallpapers (aplicado apenas ao confirmar) + aleatório (`K2` / `caelestia wallpaper -r`) + toggles DND, Game Mode, Inibir Idle e Ciclar Saída.
- **v2 — Fase D (Customização do Hub nas Configurações)**:
  - Nova aba **Overlay / Hub** em `settings.slint` / `settings_mod.rs` para reordenar, renomear, ocultar/exibir módulos, configurar passos/timeouts/`Key3`, toggle `Imagens: SIM/NÃO`, escolher imagem opcional por item (`rfd`), e criar **Ações Rápidas**, **Sliders Customizados** e **Submenus Customizados** com hot-reload e persistência TOML.
- **v2.1 — Overlay Carrossel Horizontal Estilo Caelestia & Thumbnails Opcionais**:
  - Novo `ui/overlay.slint` panorâmico (`960×256px`, 5 cards horizontais com foco central elevado em moldura translúcida, pílulas centralizadas no topo e rodapé discreto).
  - Posicionamento automático na parte inferior central do monitor focado (respeitando `scale` e `transform` para monitor vertical) via `.socket.sock` do Hyprland (`position_overlay_bottom_center`, margem flutuante padrão `20px`).
  - Pipeline assíncrono leve de imagens/ícones opcionais: `system/icons.rs` (resolução `.desktop` → tema de ícones Linux), `system/thumbs.rs` (`image 0.25` redimensionando para `240×140` / `96×96` + cache em disco `~/.cache/knobcontext/thumbs/`), e cache LRU em RAM (`ui/mod.rs`, máx. 32 imagens ≈ ~4 MB) + pre-fetch dos vizinhos do carrossel.

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
1. Instalar `appimagetool` (ou `linuxdeploy`) e rodar `./packaging/build-appimage.sh` para gerar o `KnobContext-x86_64.AppImage`.
2. Validar manualmente o ícone no tray do Caelestia dots, a persistência de `mode = "manual:global"` entre reinicializações e o script `packaging/appimage/install.sh`.
3. *(Versões futuras)* Serviço `systemd --user`, suporte multiplataforma Windows/X11 (`focus-tracker` + `hidapi`) e (opcional) escrita de firmware pela Interface 1 (`EP 2 OUT`).
