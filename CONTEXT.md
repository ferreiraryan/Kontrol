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
**Fase 7 + Reformulação UX/UI das Configurações e Seletor de Ações** concluídas e validadas: design system unificado (`ui/theme.slint`) alinhado à paleta *cool slate* do Overlay/Mini-HUD, `SettingsWindow` (`920×680`) com mapa físico dos controles (Botão Rotativo + Teclas Físicas K1–K3), preview explícito de ações herdadas, regras de ativação recolhíveis (ocultas em `global`), aba `Overlay / Hub` separada em `Itens Padrão do Hub` e `Aparência e Comportamento`, e `ActionEditorPopover` (`720×500`) redesenhado com keycaps, chips de modificadores/presets, grid 2×3 de mídia em PT-BR e timeline de macro com reordenação e Modo Aprender. AppImage (`KnobContext-x86_64.AppImage`, 10.9 MB) regerado e 73/73 testes unitários passando.

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
- **v2.2 — Animações Suaves 60fps no Overlay (`ui/animator.rs`)**:
  - Carrossel contínuo de **7 slots** (centro fixo em `3`, bordas `0` e `6` entrando/saindo via `clip: true` com espaçamento constante de `12px`), `CarouselAnimator` exponencial (`~175ms` scroll com acúmulo de inércia em rajadas, `~155ms` entrada do dock, `~140ms` troca de tela, `animate width` nos sliders), `slint::Timer` 16ms sob demanda (0% CPU em repouso) e toggle `overlay.animations_enabled` nas Configurações.
- **Fase 6 — Fechar o ciclo de usabilidade**:
  - **Mini-HUD (`ui/hud.slint`, `ui/hud_mod.rs`)**: janela independente `236×64px` no canto inferior direito (`KnobContext HUD`, `no_initial_focus = true` via Hyprland Lua IPC), exibida por `500ms` ao girar o knob fora do Hub, acumulando giros consecutivos, sincronizando volume real via `SystemWorker` e exibindo ícone + ação para atalhos sem barra.
  - **Editor visual de `inherits` (`ui/settings.slint`, `ui/settings_mod.rs`)**: seletor `"Herda de:"` (com `"Nenhum"` padrão e exclusão do próprio perfil), validação de ciclo imediata na UI sem salvar, cadeia de herança clicável (`global → base → firefox`) e limpeza/atualização automática ao renomear ou remover perfis.
  - **Serviço `systemd --user` (`packaging/systemd/knobcontext.service`)**: `WantedBy=default.target` + `After=graphical-session.target hyprland-session.target` + `ExecStartPre=/bin/sleep 2` (confirmado no ambiente que apenas `default.target` está ativo, enquanto `WAYLAND_DISPLAY` e `HYPRLAND_INSTANCE_SIGNATURE` são importados pelo Hyprland).
  - **AppImage validado (`KnobContext-x86_64.AppImage`, 12 MB)**: gerado com `appimagetool` (`~/.local/bin/appimagetool`) + `fuse2`, validado em execução real (RSS ~19.5 MB, warning udev ausente, conexão Hyprland, registro Tray SNI e `ungrab()` gracioso). `install.sh` aprimorado com verificação do grupo `input`, regra udev (`evdev` + `hidraw`), instalação em `~/.local/bin/knobcontext` e habilitação do `systemd --user`.
  - Validação: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (64/64).
- **Fase 7 — Hub Contextual + Modo Aprender**:
  - **Hub Contextual por perfil (`config.rs`, `overlay_hub.rs`)**: campo `hub_modules: Option<Vec<OverlayItemConfig>>` em `ProfileConfig` com desserialização resiliente (aceita aliases `type`/`label` e ignora módulos desconhecidos com `warn!`).
    - **Semântica de herança de módulos** (diferente da herança campo-a-campo de ações): `None` (omitido no TOML) herda pela cadeia `inherits` até `global` / `overlay.items`; `Some(vec![])` define Hub explicitamente vazio (não herda); `Some(vec![...])` substitui os módulos do pai.
    - **6 novos módulos nativos**: `browser_tabs` (`Ctrl+Tab`/`Ctrl+Shift+Tab`/`Ctrl+T`/`Ctrl+W`), `media_control` (`playerctl play-pause`/`next`/`previous`), `zoom` (`Ctrl+Plus`/`Ctrl+Minus`/`Ctrl+0`), `scroll_page` (`PageUp`/`PageDown`/`Home`/`End`), `brush_size` (`]`/`[`), `undo_redo` (`Ctrl+Z`/`Ctrl+Shift+Z`).
    - **`HubController`**: tira snapshot dos módulos habilitados do perfil ativo na abertura e mantém esse snapshot travado até fechar (evitando troca no meio da interação).
  - **Editor de Módulos por Perfil (`ui/settings.slint`, `ui/settings_mod.rs`)**: sub-aba `Ações do Knob | Módulos do Hub` dentro de `Perfis & Contexto` com indicador de herança (*"Herdado de \<pai\> — clique para customizar"* vs *"Definido neste perfil"* + *Restaurar herança*), reordenação `↑`/`↓`, edição de rótulo, remoção e seletor `+ Adicionar módulo`.
  - **Modo Aprender (`ui/learn.slint`, `ui/learn_mod.rs`)**: janela flutuante sempre no topo (`420×144px`, com foco de teclado via `wl_keyboard` do Slint) acionada pelo botão **Aprender** em cada `ActionRow` e no popover de atalhos. Exibe a combinação em tempo real, confirma automaticamente `200ms` após soltar todas as teclas (ou imediatamente com `Enter`) e cancela com `Esc`.
    - *Limitação no Wayland*: atalhos interceptados pelo compositor antes de chegar à janela (ex.: `Super+E` no Hyprland) não são entregues ao `wl_keyboard` sem `zwp_keyboard_shortcuts_inhibit_manager_v1` (não exposto pelo Slint 1.19 sem crates extras); o fallback é digitar manualmente no campo de texto do construtor visual.
  - Validação: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (70/70), AppImage regerado.
- **UX/UI Overhaul — Configurações, Seletor de Ações e Modo Aprender**:
  - **Design System compartilhado (`ui/theme.slint`)**: paleta *cool slate* (`#11141b` / `#171b24` / `#1e2432` + acento `#3b82f6`) e componentes reutilizáveis (`AppButton`, `AppInput`, `AppField`, `ToggleSwitch`, `SegmentedTwo`, `StepperControl`, `ActionCategoryBadge`), eliminando estilos duplicados.
  - **`SettingsWindow` (`920×680`, `ui/settings.slint`, `ui/settings_mod.rs`)**:
    - Sidebar de perfis enriquecida (badge `Padrão`, subtítulo `window_class`, ponto indicador de Hub customizado e `Remover perfil` desabilitado em `global`).
    - Perfil `global` exibe banner limpo sem campos contextuais irrelevantes; perfis contextuais agrupam regras em card recolhível (`Regras de ativação`) com resumo compacto.
    - Mapa físico em `Ações do Knob` (2 cards lado a lado para `↺ Anti-horário` / `↻ Horário` + 3 linhas enriquecidas `K1`/`K2`/`K3` clicáveis na linha toda, com badge colorido por categoria, preview de herança `resolve_slot_view`, `⌨ Aprender` e `×` para limpar).
    - Aba `Overlay / Hub` dividida em 2 sub-abas via `SegmentedTwo` (`Itens Padrão do Hub` com altura livre para editores + `Aparência e Comportamento` com `ToggleSwitch` e `StepperControl`).
  - **`ActionEditorPopover` (`720×500`, `ui/action_editor.slint`, `ui/action_editor_state.rs`, `ui/action_editor_mod.rs`) & `LearnWindow` (`440×168`)**:
    - Sidebar agrupada por categoria com ícones e rodapé com `Preview:` ao vivo.
    - Atalho de teclado unificado com keycaps visuais, botão principal `⌨ Gravar tecla` (Modo Aprender), chips `+ Ctrl`/`+ Shift`/`+ Alt`/`+ Super`, atalhos comuns e edição manual.
    - Grid 2×3 de Controle de Mídia em português, busca de aplicativos com contador e seleção fixa, Scroll com setas + presets (`1×`/`3×`/`5×`/`10×`), e Macro com badges coloridos, reordenação `↑`/`↓`, `⌨ Aprender` e chips de espera (`+50ms`/`+100ms`/`+250ms`).
    - `LearnWindow` alinhada ao `Theme` com botão clicável `Cancelar (Esc)`.
  - Validação: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (73/73), AppImage regerado (`10.9 MB`).

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
1. Rodar `./packaging/appimage/install.sh` (requer `sudo` interativo) para instalar `/etc/udev/rules.d/99-knobcontext.rules`, copiar `KnobContext-x86_64.AppImage` para `~/.local/bin/knobcontext` e habilitar `knobcontext.service` no `systemd --user`; reconectar o knob e validar após reboot.
2. *(Versões futuras)* Suporte multiplataforma Windows/X11 (`focus-tracker` + `hidapi`) e (opcional) escrita de firmware pela Interface 1 (`EP 2 OUT`, cuja permissão `hidraw` já está incluída em `99-knobcontext.rules`).
