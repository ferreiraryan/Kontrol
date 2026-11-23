# KnobContext

Mapeador global leve em Rust para dispositivos knob USB (3 teclas + botão rotativo), com overlay minimalista escuro em Slint.

## Escopo atual (v1.0 — Linux Wayland)

- Captura exclusiva (`evdev::grab()`) das interfaces do knob em `/dev/input/by-id/` (sem precisar regravar o firmware).
- Perfis globais configuráveis em `~/.config/knobcontext/config.toml` (padrão embutido: `Volume`, `Pincel`, `Scroll`).
- Simulação de entrada nativa no Wayland via `enigo` (backend `wayland`).
- Overlay minimalista escuro em Slint (`backend-winit-wayland` + `renderer-software` para baixo consumo de memória):
  - **Pressionar o knob (`KnobPress`)** é reservado para abrir/fechar o overlay (não é configurável como ação de perfil).
  - A primeira opção do overlay é **Automático** (segue a janela ativa via Hyprland); escolher um perfil nomeado trava em modo **Manual** até voltar para **Automático**. O modo é **volátil** (sempre reinicia em **Automático** ao abrir o app).
  - **Inatividade de 3s** fecha o overlay automaticamente sem alterar o modo/perfil ativo.
- **Limitação conhecida**: a v1.0 no Linux Wayland opera em modo global (sem detecção de janela em primeiro plano). A detecção de aplicativo em foco está prevista para a Fase 3 (Windows/X11).

## Requisitos de permissão no Linux

Seu usuário precisa de permissão de leitura nos nós `evdev` (`/dev/input/event*`):

```bash
sudo usermod -aG input "$USER"
```

*(Faça logout/login após adicionar ao grupo `input`, ou crie uma regra udev em `/etc/udev/rules.d/99-knobcontext.rules`.)*

## Regras recomendadas no Hyprland (Wayland)

No Wayland, o backend `winit` do Slint cria uma janela `xdg_toplevel` (não `wlr-layer-shell`), portanto o compositor controla **posicionamento centralizado**, **sempre no topo (`pin`)** e **não roubar foco (`nofocus`)**.

Adicione as regras abaixo ao seu `~/.config/hypr/hyprland.conf` para garantir que o overlay fique sempre no topo, centralizado, sem bordas do compositor e **sem roubar o foco** do aplicativo ativo:

```ini
# KnobContext Overlay
windowrulev2 = float, title:^(KnobContext Overlay)$
windowrulev2 = pin, title:^(KnobContext Overlay)$
windowrulev2 = center, title:^(KnobContext Overlay)$
windowrulev2 = nofocus, title:^(KnobContext Overlay)$
windowrulev2 = noborder, title:^(KnobContext Overlay)$
windowrulev2 = noshadow, title:^(KnobContext Overlay)$
```

> **Transparência**: o overlay utiliza `background: transparent` no Slint (`renderer-software`) com cartão interno em `rgba(20, 20, 20, 0.85)` e cantos arredondados de `12px`. No Hyprland, a transparência RGBA funciona nativamente sem configuração adicional; se desejar desfoque de fundo, adicione `windowrulev2 = blur, title:^(KnobContext Overlay)$` ou regra de layer/opacity conforme sua versão do Hyprland.

## Como executar

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
