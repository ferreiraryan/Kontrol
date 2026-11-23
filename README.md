# KnobContext

Mapeador global leve em Rust para dispositivos knob USB (3 teclas + botão rotativo), com overlay minimalista escuro em Slint.

## Escopo atual (v1.0 — Linux Wayland)

- Captura exclusiva (`evdev::grab()`) das interfaces do knob em `/dev/input/by-id/` (sem precisar regravar o firmware).
- Perfis globais configuráveis em `~/.config/knobcontext/config.toml` (padrão embutido: `Volume`, `Pincel`, `Scroll`).
- Simulação de entrada nativa no Wayland via `enigo` (backend `wayland`).
- Overlay minimalista escuro em Slint (`backend-winit-wayland` + `renderer-software` para baixo consumo de memória):
  - **Pressionar o knob** em modo normal abre o overlay.
  - **Girar o knob** com o overlay aberto alterna entre os perfis.
  - **Pressionar o knob** novamente seleciona o perfil destacado e fecha o overlay.
  - **Inatividade de 3s** fecha o overlay automaticamente sem alterar o perfil ativo.
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

```toml
active_profile = "Volume"

[device]
kbd_path = "/dev/input/by-id/usb-1189_8890-event-kbd"
media_path = "/dev/input/by-id/usb-1189_8890-if02-event-kbd"
combo_window_ms = 200

[[profiles]]
name = "Volume"
knob_cw = { type = "volume_up" }
knob_ccw = { type = "volume_down" }
knob_press = { type = "volume_mute" }
key1 = { type = "media_prev" }
key2 = { type = "media_next" }
key3 = { type = "media_play_pause" }

[[profiles]]
name = "Pincel"
knob_cw = { type = "shortcut", keys = ["]"] }
knob_ccw = { type = "shortcut", keys = ["["] }
knob_press = { type = "none" }
key1 = { type = "shortcut", keys = ["ctrl", "z"] }
key2 = { type = "shortcut", keys = ["ctrl", "shift", "z"] }
key3 = { type = "shortcut", keys = ["b"] }
```
