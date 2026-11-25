# KnobContext

Mapeador global leve em Rust para dispositivos knob USB (3 teclas + botão rotativo), com overlay minimalista escuro em Slint.

## Recursos principais (Linux Wayland / Hyprland + Caelestia Dots)

- **Captura exclusiva (`evdev::grab()`)** das interfaces do knob em `/dev/input/by-id/` (sem precisar regravar o firmware).
- **Modo Automático Adaptativo ao Contexto (Hyprland)**:
  - Monitora `WINDOWCLASS`, **título da janela** e **estado fullscreen** em tempo real via sockets do Hyprland.
  - Regras compostas com múltiplos padrões (`firefox|zen|chromium`), filtro por título (`YouTube|Twitch|Netflix`), prioridade explícita, herança de perfil (`inherits = "global"`), botão **Capturar janela** e biblioteca de presets prontos.
  - O modo escolhido (`mode = "auto"` ou `mode = "manual:<perfil>"`) é **persistido** no `config.toml` entre sessões.
- **Hub Interativo Hierárquico no Overlay (`KnobPress`)**:
  - **Navegação pelo hardware**: girar navega, **`KnobPress` ou `Key3`** entra/confirma/trava ajuste (`AJUSTANDO`), **`Key1`** volta/solta/fecha e **`Key2`** dispara a ação secundária contextual (*Mudo* no áudio, *Wallpaper aleatório* no Caelestia, *Sincronizar monitores* no brilho, *Special Workspace* nos workspaces).
  - **Módulos nativos**: **Perfis**, **Sons & Apps** (volume global + individual por aplicativo via `pactl`), **Saída de Som** (troca sink padrão + migra streams), **Workspaces** (Hyprland `.socket.sock`), **Brilho por Monitor** (Caelestia IPC + fallback `brightnessctl`), **Wallpapers** (Caelestia IPC, aplicado ao confirmar + aleatório), **Controles Caelestia** (DND, Game Mode, Inibir Idle) e **6 módulos contextuais**: **Abas do Navegador** (`browser_tabs`), **Controle de Mídia** (`media_control` via `playerctl`), **Zoom** (`zoom`), **Navegação de Página** (`scroll_page`), **Tamanho do Pincel** (`brush_size`) e **Desfazer / Refazer** (`undo_redo`).
  - **Hub Contextual por perfil**: cada perfil pode definir sua própria lista de `hub_modules` (na sub-aba **Módulos do Hub** em *Perfis & Contexto* ou via `[[profiles.hub_modules]]` no TOML). Sem `hub_modules`, o perfil herda automaticamente do pai (`inherits` → `global`); uma lista vazia explícita (`hub_modules = []`) oculta o Hub naquele perfil.
  - **Customizável pela UI**: aba **Overlay / Hub** na janela de Configurações para reordenar/ocultar módulos globais e criar **Ações Rápidas**, **Sliders Customizados** e **Submenus Customizados**.
- **Modo Aprender (captura rápida de teclas)**:
  - Botão **Aprender** ao lado de cada controle do perfil e no editor de atalhos abre uma janela flutuante sempre no topo (`KnobContext — Modo Aprender`) que captura a combinação pressionada em tempo real e confirma automaticamente após `200ms` ao soltar as teclas (ou com `Enter`; `Esc` cancela).
  - **Limitação conhecida no Wayland**: por segurança do protocolo Wayland, a captura só funciona enquanto a janela do Modo Aprender tem foco de teclado (`wl_keyboard`). Atalhos interceptados diretamente pelo compositor Hyprland (como `Super+E`) podem não chegar à janela — nesses casos, use o campo de texto manual no editor de atalhos como fallback.
- **Mini-HUD ao girar fora do Hub**:
  - Ao girar o knob com o Hub fechado, exibe um Mini-HUD discreto (`236×64px`) no canto inferior direito por `500ms`, sem roubar foco, mostrando o valor atual (ex.: `Volume 45%` + barra fina) ou o atalho executado (acumulando giros consecutivos).
- **Simulação de entrada nativa no Wayland** via `enigo` (backend `wayland`) com proteção RAII contra tecla presa.
- **Ícone no tray via StatusNotifierItem (SNI)**:
  - Exposto via D-Bus (`org.kde.StatusNotifierItem` com `ksni`) — compatível nativamente com **Caelestia dots** (Quickshell), Waybar (`tray` module) e outros hosts SNI no Wayland.
  - Exibe tooltip dinâmico (`KnobContext — Automático (firefox)` ou `KnobContext — Manual: global`) e menu rápido (*Abrir configurações*, *Modo Automático*, *Modo Manual → perfis*, *Sair*).

## Requisitos de permissão no Linux

Seu usuário precisa de permissão de leitura nos nós `evdev` (`/dev/input/event*`):

```bash
sudo usermod -aG input "$USER"
```

*(Faça logout/login após adicionar ao grupo `input`, ou crie uma regra udev em `/etc/udev/rules.d/99-knobcontext.rules`.)*

## Regras recomendadas no Hyprland (Wayland)

O KnobContext posiciona automaticamente o overlay flutuante na **parte inferior central do monitor focado** via IPC do Hyprland (`.socket.sock`).

Se quiser garantir também **sempre no topo (`pin`)**, **sem roubar foco (`nofocus`)**, **sem bordas/sombras** e posicionamento instantâneo desde o 1º frame, adicione as regras abaixo:

**Em `~/.config/hypr/hyprland.conf`:**
```ini
# KnobContext Overlay (dock flutuante inferior central)
windowrulev2 = float, title:^(KnobContext Overlay)$
windowrulev2 = pin, title:^(KnobContext Overlay)$
windowrulev2 = move (monitor_w-window_w)/2 (monitor_h-window_h-20), title:^(KnobContext Overlay)$
windowrulev2 = nofocus, title:^(KnobContext Overlay)$
windowrulev2 = noborder, title:^(KnobContext Overlay)$
windowrulev2 = noshadow, title:^(KnobContext Overlay)$

# KnobContext Mini-HUD (canto inferior direito)
windowrulev2 = float, title:^(KnobContext HUD)$
windowrulev2 = pin, title:^(KnobContext HUD)$
windowrulev2 = move (monitor_w-window_w-20) (monitor_h-window_h-20), title:^(KnobContext HUD)$
windowrulev2 = nofocus, title:^(KnobContext HUD)$
windowrulev2 = noborder, title:^(KnobContext HUD)$
windowrulev2 = noshadow, title:^(KnobContext HUD)$
```

**Ou em `~/.config/caelestia/hypr-user.lua` (setup Caelestia em Lua — também aplicado automaticamente no startup do KnobContext):**
```lua
hl.window_rule({
  name = "knobcontext-overlay",
  match = { title = "^(KnobContext Overlay)$" },
  float = true,
  pin = true,
  center = false,
  move = "((monitor_w-window_w)/2) (monitor_h-window_h-20)",
  no_initial_focus = true,
  border_size = 0,
  no_shadow = true,
  opacity = "1 override",
})

hl.window_rule({
  name = "knobcontext-hud",
  match = { title = "^(KnobContext HUD)$" },
  float = true,
  pin = true,
  center = false,
  move = "(monitor_w-window_w-20) (monitor_h-window_h-20)",
  no_initial_focus = true,
  border_size = 0,
  no_shadow = true,
  opacity = "1 override",
})
```

> **Transparência**: o overlay utiliza `background: transparent` no Slint (`renderer-software`) com cartão interno em `rgba(20, 20, 20, 0.85)` e cantos arredondados de `12px`. No Hyprland, a transparência RGBA funciona nativamente sem configuração adicional; se desejar desfoque de fundo, adicione `windowrulev2 = blur, title:^(KnobContext Overlay)$` ou regra de layer/opacity conforme sua versão do Hyprland.

## Instalação via AppImage e Serviço `systemd --user` (Arch Linux)

> **Importante**: o AppImage **não** instala a regra `udev` nem o serviço `systemd` sozinho (regras em `/etc/udev/rules.d/` exigem `sudo` uma única vez). Use o script `install.sh` incluso.

1. Instale `fuse2` (necessário no Arch para executar AppImages):
   ```bash
   sudo pacman -S fuse2
   ```
2. Garanta que seu usuário pertence ao grupo `input` (e faça logout/login se adicionou agora):
   ```bash
   sudo usermod -aG input "$USER"
   ```
3. Gere o AppImage (caso ainda não tenha o `KnobContext-x86_64.AppImage`; usa `appimagetool` em `~/.local/bin/` ou no `PATH`):
   ```bash
   ./packaging/build-appimage.sh
   chmod +x KnobContext-x86_64.AppImage
   ```
4. Execute o `install.sh` **uma única vez**:
   ```bash
   ./packaging/appimage/install.sh
   ```
   O script:
   - Verifica se o usuário pertence ao grupo `input`.
   - Instala `/etc/udev/rules.d/99-knobcontext.rules` (cobrindo `evdev` e `hidraw`) e recarrega o `udev`.
   - Copia `KnobContext-x86_64.AppImage` para `~/.local/bin/knobcontext`.
   - Instala `~/.config/systemd/user/knobcontext.service`, executa `systemctl --user daemon-reload` e `systemctl --user enable --now knobcontext.service`.
5. **Reconecte o dispositivo knob** após a primeira instalação da regra `udev`.

### Gerenciando o serviço `systemd --user`

```bash
# Ver status do serviço
systemctl --user status knobcontext.service

# Acompanhar logs em tempo real
journalctl --user -u knobcontext.service -f

# Reiniciar / parar / desabilitar início automático
systemctl --user restart knobcontext.service
systemctl --user stop knobcontext.service
systemctl --user disable knobcontext.service
```

> **Nota sobre Hyprland e `systemd --user`**: em sessões Hyprland iniciadas sem `uwsm`, o alvo `graphical-session.target` pode não existir (`systemctl --user list-units --type=target` mostra apenas `default.target`). Por isso, `knobcontext.service` usa `WantedBy=default.target` com `After=graphical-session.target hyprland-session.target` e `ExecStartPre=/bin/sleep 2` para aguardar o Hyprland exportar `WAYLAND_DISPLAY` e `HYPRLAND_INSTANCE_SIGNATURE` via `systemctl --user import-environment`. Se quiser sincronia imediata no login, você também pode adicionar `systemctl --user restart knobcontext.service` logo após `import-environment` na config do Hyprland.

> **Requisito de Tray (SNI)**: o ícone na barra requer um host `StatusNotifierItem` ativo (como **Caelestia dots**, Waybar com módulo `tray`, ou Quickshell). Se nenhum host SNI estiver rodando, o KnobContext registra um aviso e continua funcionando normalmente.

## Como executar (desenvolvimento)

```bash
cd knobcontext
RUST_LOG=info cargo run --bin knobcontext
```

Para abrir a janela de configurações diretamente na inicialização (mantendo o `grab()` ativo para testar ações ao vivo):

```bash
cargo run --bin knobcontext -- --settings
```

Você também pode abrir a janela de configurações a qualquer momento pressionando **`Key1 + Key2`** simultaneamente no dispositivo.

Para encerrar e restaurar imediatamente os atalhos originais do dispositivo (`ungrab`), pressione `Ctrl+C`.

## Exemplo de configuração (`~/.config/knobcontext/config.toml`)

O arquivo usa strings legíveis para cada ação (compatível também com o formato legado de tabelas `{ type = "..." }`):

- **Atalho de teclado**: `"Ctrl+Tab"`, `"Super+E"`, `"Alt+F4"`
- **Controle de mídia**: `"VolumeUp"`, `"VolumeDown"`, `"Mute"`, `"PlayPause"`, `"Prev"`, `"Next"`
- **Abrir aplicativo**: `"Launch:firefox.desktop"` (executado diretamente sem shell)
- **Abrir URL / arquivo**: `"Open:https://example.com"`, `"Open:file:///home/user/doc.pdf"`
- **Scroll**: `"Scroll:up:3"`, `"Scroll:down:1"`, `"Scroll:left:2"`, `"Scroll:right:2"` (quantidade 1–10)
- **Macro**: `"Macro:Ctrl+C,Wait(100),Ctrl+V"`
- **Executar comando**: `"Exec:playerctl play-pause"`
- **Nenhuma ação**: `"None"`

> **Aviso de segurança (`Exec:`)**: ações com o prefixo `Exec:` são executadas via `sh -c` para suportar pipes e redirecionamentos. Isso concede execução arbitrária de comandos com as permissões do seu usuário — use com cuidado e apenas com comandos confiáveis.

> **Captura de tecla `Super` no Wayland**: a captura ao vivo na janela de configurações suporta `Ctrl`, `Alt`, `Shift` e `Super`, mas alguns atalhos com `Super` podem ser interceptados pelo compositor (ex.: atalhos globais do Hyprland) antes de chegar à janela. Nesses casos, utilize o campo de texto manual logo abaixo da área de captura para digitar a combinação diretamente (ex.: `Super+E`).

> **Sensibilidade do Scroll no Wayland (`device.scroll_step`)**: no Wayland (`wlr_virtual_pointer::axis`), o `enigo` envia unidades contínuas (pixels), não cliques discretos. O valor de `Scroll:<dir>:<qtd>` é multiplicado por `device.scroll_step` (padrão `20`: `Scroll:up:1` = 20px, `Scroll:up:5` = 100px, `Scroll:up:10` = 200px). Ajuste `scroll_step` em `[device]` (1–200) se quiser rolagem mais lenta ou mais rápida no Hyprland.

> **Teste manual de prevenção de tecla presa (BUG 1)**: abra as configurações (`--settings`), clique em **Editar** em qualquer controle, escolha *Atalho de teclado*, clique na área de captura, pressione `Shift` (ou `Shift+Tab`) e clique em **Cancelar**. Digite texto em outro aplicativo/terminal e confirme que `Shift` não ficou preso.

```toml
active_profile = "global"
mode = "auto" # ou "manual:global"

[device]
kbd_path = "/dev/input/by-id/usb-1189_8890-event-kbd"
media_path = "/dev/input/by-id/usb-1189_8890-if02-event-kbd"
combo_window_ms = 200
scroll_step = 20

[[profiles]]
name = "global"
knob_cw = "VolumeUp"
knob_ccw = "VolumeDown"
key1 = "Prev"
key2 = "Next"
key3 = "PlayPause"

[[profiles]]
name = "firefox"
window_class = "firefox"
knob_cw = "Ctrl+Tab"
knob_ccw = "Ctrl+Shift+Tab"
key1 = "Launch:firefox.desktop"
key2 = "Macro:Ctrl+C,Wait(100),Ctrl+V"
key3 = "Scroll:down:3"
```

## Presets de Atalhos por Aplicativo (Fase 8)

No **Seletor de Ações → Atalho de teclado**, o menu **Presets ▾** permite selecionar ações nomeadas por aplicativo (sugerindo automaticamente o app correspondente ao perfil editado). Ao editar manualmente o campo de atalho, o indicador `Preset: <App> — <Ação>` desaparece automaticamente.

| Aplicativo | Ação | Atalho |
|---|---|---|
| **Firefox** | Nova aba | `Ctrl+T` |
| **Firefox** | Reabrir última aba fechada | `Ctrl+Shift+T` |
| **Firefox** | Próxima aba | `Ctrl+Tab` |
| **Firefox** | Aba anterior | `Ctrl+Shift+Tab` |
| **Firefox** | Fechar aba | `Ctrl+W` |
| **Firefox** | Ir para endereço | `Ctrl+L` |
| **Firefox** | Atualizar página | `F5` |
| **VSCode** | Paleta de comandos | `Ctrl+Shift+P` |
| **VSCode** | Terminal integrado | ``Ctrl+` `` |
| **VSCode** | Salvar | `Ctrl+S` |
| **VSCode** | Buscar em arquivos | `Ctrl+Shift+F` |
| **VSCode** | Ir para linha | `Ctrl+G` |
| **VSCode** | Comentar linha | `Ctrl+/` |
| **Terminal** | Limpar tela | `Ctrl+L` |
| **Terminal** | Interromper processo | `Ctrl+C` |
| **Terminal** | Suspender processo | `Ctrl+Z` |
| **Terminal** | Buscar histórico | `Ctrl+R` |

## Integrações Diretas (Providers)

Além da simulação de teclado via `enigo`, o KnobContext suporta **providers de integração direta** (`src/providers/`), permitindo controlar aplicativos via API sem depender de atalhos ou foco de janela.

- **Limitação conhecida**: navegadores como o **Firefox** não expõem API local de controle de abas, portanto continuam usando **presets de atalhos**; **OBS Studio** (WebSocket 5.x) e players **MPRIS** (D-Bus) usam controle direto.
- Todos os providers usam **conexão preguiçosa** (não abrem WebSocket nem D-Bus até serem usados ou testados na aba **Integrações**).

### Providers e Comandos Disponíveis

| Provider (`id`) | Comando (`command`) | Argumentos | Exemplo no TOML |
|---|---|---|---|
| **OBS Studio** (`obs`) | `toggle_record` | — | `Provider:obs:toggle_record` |
| **OBS Studio** (`obs`) | `start_record` | — | `Provider:obs:start_record` |
| **OBS Studio** (`obs`) | `stop_record` | — | `Provider:obs:stop_record` |
| **OBS Studio** (`obs`) | `toggle_stream` | — | `Provider:obs:toggle_stream` |
| **OBS Studio** (`obs`) | `start_stream` | — | `Provider:obs:start_stream` |
| **OBS Studio** (`obs`) | `stop_stream` | — | `Provider:obs:stop_stream` |
| **OBS Studio** (`obs`) | `toggle_input_mute` | `input_name` | `Provider:obs:toggle_input_mute:input_name=Mic/Aux` |
| **OBS Studio** (`obs`) | `set_scene` | `scene_name` | `Provider:obs:set_scene:scene_name=Live` |
| **Mídia MPRIS** (`mpris`) | `play_pause` | — | `Provider:mpris:play_pause` |
| **Mídia MPRIS** (`mpris`) | `next` | — | `Provider:mpris:next` |
| **Mídia MPRIS** (`mpris`) | `previous` | — | `Provider:mpris:previous` |
| **Mídia MPRIS** (`mpris`) | `stop` | — | `Provider:mpris:stop` |
| **Mídia MPRIS** (`mpris`) | `seek` | `seconds` | `Provider:mpris:seek:seconds=10` |
| **Mídia MPRIS** (`mpris`) | `volume_up` | `step` *(opcional, padrão 5)* | `Provider:mpris:volume_up` |
| **Mídia MPRIS** (`mpris`) | `volume_down` | `step` *(opcional, padrão 5)* | `Provider:mpris:volume_down` |
| **Mídia MPRIS** (`mpris`) | `set_volume` | `percent` | `Provider:mpris:set_volume:percent=50` |

### Configuração do OBS WebSocket (`config.toml`)

Habilite em **Configurações → Integrações** (ou edite diretamente `~/.config/knobcontext/config.toml`). No OBS Studio, ative em **Ferramentas → Configurações do Servidor WebSocket**:

```toml
[providers.obs]
enabled = true
port = 4455
password = "sua_senha_opcional" # omita se desabilitar autenticação no OBS
```

O provider **MPRIS** (Linux) não requer configuração e detecta automaticamente qualquer player compatível em execução (Spotify, Firefox, Chromium, mpv, VLC).

---

## Suporte Multiplataforma (Windows e Linux — Fase 9)

O KnobContext detecta automaticamente se está rodando no **Windows** ou no **Linux** (`src/platform/mod.rs`) e exibe nas Configurações apenas os providers disponíveis na plataforma atual. Se você sincronizar o mesmo `config.toml` em dual-boot, seções e ações de providers da outra plataforma são ignoradas sem erro.

### Providers por Plataforma

| Provider (`id`) | Windows | Linux | Comandos | Configuração no TOML |
|---|---|---|---|---|
| **Mídia Windows SMTC** (`smtc`) | Disponível (nativo via `windows 0.62`) | Oculto | `play_pause`, `play`, `pause`, `next`, `previous`, `seek` (`seconds`) | `[providers.smtc]` (`enabled = true`) |
| **Mídia Linux MPRIS** (`mpris`) | Oculto | Disponível (D-Bus) | `play_pause`, `next`, `previous`, `stop`, `seek`, `volume_up`, `volume_down`, `set_volume` | Sem configuração |
| **OBS Studio** (`obs`) | Disponível (WebSocket 5.x) | Disponível (WebSocket 5.x) | `toggle_record`, `start_record`, `stop_record`, `toggle_stream`, `start_stream`, `stop_stream`, `toggle_input_mute`, `set_scene` | `[providers.obs]` (`enabled`, `port`, `password`) |

> **Nota sobre capas de álbum no SMTC**: players nativos como o **Spotify** enviam capa do álbum (exibida no módulo Mídia do Hub e no Mini-HUD). Navegadores como o **Firefox** com YouTube frequentemente não expõem thumbnail via SMTC — o KnobContext trata `thumbnail = None` como caso normal e exibe o ícone genérico de música junto com o título e controles.

### Configuração `[providers.smtc]` e `[ui]` (`%APPDATA%\knobcontext\config.toml`)

```toml
[ui]
# "software" (padrão no Windows — menor uso de RAM, glassmorphism simulado)
# "femtovg"  (ativa backdrop-blur real na GPU; cai automaticamente para software se falhar)
renderer = "software"

[providers.smtc]
enabled = true # padrão: true (SMTC é local e nativo, sem porta ou senha)
```

> **Reduced Motion**: no Windows, o KnobContext consulta `SPI_GETCLIENTAREAANIMATION`. Se os efeitos de animação estiverem desativados nas configurações de Acessibilidade do Windows, as animações do carrossel, fade de 120ms entre telas e micro-animação de pressão tornam-se instantâneas automaticamente.

### Perfis Padrão no Primeiro Startup (por Plataforma)

Quando nenhum `config.toml` existe ainda, o KnobContext cria automaticamente um arquivo inicial com perfis adaptados à plataforma atual:

| Plataforma | Perfis Pré-configurados | Ações Principais |
|---|---|---|
| **Windows** (`presets_windows.rs`) | `global`, `navegadores` (`firefox\|chrome\|msedge`), `vscode` (`code`), `explorer`, `krita`, `photoshop`, `illustrator`, `resolve`, `spotify` | Abas (`Ctrl+Tab`/`Ctrl+T`/`Ctrl+W`/`Ctrl+Shift+T`), Paleta/Terminal VSCode, Navegação/Copiar/Colar no Explorer, Pincel/Undo no Krita & Photoshop, Zoom/Ferramentas no Illustrator, J/K/L/I/O no DaVinci Resolve e controle direto `Provider:smtc:*` no Spotify |
| **Linux** (`presets_linux.rs`) | `global`, `firefox`, `vscode`, `gimp`, `krita` | Abas no Firefox, Paleta/Terminal no VSCode, Pincel/Undo no GIMP e Krita |

### Limitação em Jogos com Anti-Cheat (CS, Valorant, Fortnite, etc.)

> **Importante**: perfis de jogos funcionam apenas em **jogos single-player ou sem anti-cheat**. Jogos competitivos com anti-cheat em nível de kernel/sistema (**CS / VAC**, **Valorant / Vanguard**, **Fortnite / EAC**, etc.) **bloqueiam simulação de teclado sintético (`enigo` / `SendInput`) por design** — isso não é um bug do KnobContext. Por esse motivo, nenhum preset de jogos é incluído por padrão; você pode criar perfis manuais nas Configurações para jogos single-player que aceitem entrada simulada.

### Modo Aprender com Captura Global (Windows)

No Windows, o Modo Aprender usa um hook `WH_KEYBOARD_LL` temporário (`src/platform/windows/input_capture.rs`) para capturar atalhos do sistema como `Win+E` e `Alt+Tab` sem abrir o Explorador nem trocar de janela.

- **Privacidade e segurança**: a captura global vê todas as teclas do sistema enquanto está ativa, portanto o hook é instalado **exclusivamente** enquanto a janela *Modo Aprender* está aberta e é **desinstalado imediatamente** ao confirmar, cancelar ou fechar a janela — nunca permanece ativo em background.
- Caso alguma combinação reservada do sistema (como `Ctrl+Alt+Del` ou `Win+L`) seja interceptada pelo Windows antes do hook, use o campo de texto manual do editor de atalhos como fallback.

### Diagnóstico do Ambiente (`--scan`)

Para listar processos ativos, aplicativos instalados e APIs detectadas ao vivo (SMTC, OBS WebSocket na porta 4455, Discord RPC) e salvar `scan_report.txt` na raiz do projeto:

```powershell
.\knobcontext\target\debug\knobcontext.exe --scan
```
