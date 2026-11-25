pub mod action_editor_mod;
pub mod action_editor_state;
pub mod animator;
pub mod hud_mod;
pub mod learn_mod;
pub mod settings_mod;

use crate::error::KnobError;
use crate::overlay_hub::{
    HubCardView, HubViewModel, THUMB_COVER_HEIGHT, THUMB_COVER_WIDTH, THUMB_ICON_SIZE,
};
use crate::system::thumbs::{is_svg_path, DecodedThumbnail, ThumbnailMode, ThumbnailRequest};
use crate::ui::animator::CarouselAnimator;
use slint::{
    ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer, TimerMode,
    VecModel,
};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// No Windows, garante que popups (`KnobContext HUD`, `KnobContext Overlay`) exibidos via
/// Slint/winit não roubem o foco da janela ativa do usuário (aplicando `WS_EX_NOACTIVATE` +
/// `WS_EX_TOOLWINDOW` e restaurando o `HWND` anterior caso o foco tenha sido deslocado).
#[cfg(target_os = "windows")]
pub fn show_popup_without_stealing_focus<F>(title: &str, show_fn: F) -> Result<(), KnobError>
where
    F: FnOnce() -> Result<(), KnobError>,
{
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetForegroundWindow, GetWindowLongPtrW, SetForegroundWindow,
        SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };

    // SAFETY: chamadas Win32 somente leitura/configuração de estilo de janela própria.
    let prev_fg = unsafe { GetForegroundWindow() };
    show_fn()?;

    let title_wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), title_wide.as_ptr());
        if !hwnd.is_null() {
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let target = ex | (WS_EX_NOACTIVATE as isize) | (WS_EX_TOOLWINDOW as isize);
            if ex != target {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, target);
            }
            if !prev_fg.is_null() && prev_fg != hwnd && GetForegroundWindow() == hwnd {
                SetForegroundWindow(prev_fg);
            }
        }
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn show_popup_without_stealing_focus<F>(_title: &str, show_fn: F) -> Result<(), KnobError>
where
    F: FnOnce() -> Result<(), KnobError>,
{
    show_fn()
}

pub const OVERLAY_WIDTH_PX: i32 = 960;
pub const OVERLAY_HEIGHT_PX: i32 = 256;
const MAX_RAM_THUMBNAILS: usize = 32;
const ANIM_FRAME_MS: u64 = 16;

slint::include_modules!();

/// Cache LRU em memória para thumbnails decodificados (máx. 32 imagens ≈ ~4 MB de RAM).
struct ThumbnailCache {
    map: HashMap<ThumbnailRequest, Image>,
    order: VecDeque<ThumbnailRequest>,
}

impl ThumbnailCache {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get(&mut self, req: &ThumbnailRequest) -> Option<Image> {
        let img = self.map.get(req)?.clone();
        if let Some(pos) = self.order.iter().position(|k| k == req) {
            self.order.remove(pos);
            self.order.push_back(req.clone());
        }
        Some(img)
    }

    fn insert(&mut self, req: ThumbnailRequest, img: Image) {
        if self.map.contains_key(&req) {
            if let Some(pos) = self.order.iter().position(|k| k == &req) {
                self.order.remove(pos);
            }
        } else if self.order.len() >= MAX_RAM_THUMBNAILS {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
        self.order.push_back(req.clone());
        self.map.insert(req, img);
    }
}

/// Resultado da renderização do overlay informando se acabou de abrir e quais thumbnails faltam.
pub struct RenderOutcome {
    pub just_opened: bool,
    pub missing_thumbnails: Vec<ThumbnailRequest>,
}

struct AnimRuntime {
    animator: CarouselAnimator,
    last_tick: Instant,
}

/// Gerencia o ciclo de vida da janela Overlay (Carrossel Horizontal Caelestia) em Slint.
///
/// Descarta a instância no `hide()` e cria um `Overlay` novo ao abrir para preservar
/// o estado `floating` no Wayland/Hyprland, e dirige animações a 60fps apenas enquanto
/// há transição ativa (0% de CPU em repouso).
pub struct OverlayWindow {
    window: Option<Overlay>,
    last_vm: Option<HubViewModel>,
    thumb_cache: ThumbnailCache,
    anim: Rc<RefCell<AnimRuntime>>,
    anim_timer: Rc<Timer>,
    use_backdrop_blur: bool,
}

impl OverlayWindow {
    pub fn new() -> Self {
        Self {
            window: None,
            last_vm: None,
            thumb_cache: ThumbnailCache::new(),
            anim: Rc::new(RefCell::new(AnimRuntime {
                animator: CarouselAnimator::new(),
                last_tick: Instant::now(),
            })),
            anim_timer: Rc::new(Timer::default()),
            use_backdrop_blur: false,
        }
    }

    pub fn set_use_backdrop_blur(&mut self, enabled: bool) {
        self.use_backdrop_blur = enabled;
    }

    pub fn render(&mut self, vm: &HubViewModel) -> Result<RenderOutcome, KnobError> {
        let just_opened = self.window.is_none();
        if self.window.is_none() {
            let window = Overlay::new().map_err(|err| KnobError::UiInit(err.to_string()))?;
            self.window = Some(window);
        }

        {
            let mut rt = self.anim.borrow_mut();
            rt.animator.set_loading(vm.loading, vm.animations_enabled);
            if just_opened {
                rt.animator.trigger_open(vm.animations_enabled);
            } else if vm.screen_changed {
                rt.animator.trigger_screen_transition(vm.animations_enabled);
            } else if vm.scroll_delta != 0 {
                rt.animator
                    .trigger_scroll(vm.scroll_delta, vm.animations_enabled);
            }
            if vm.press_pulse {
                rt.animator.trigger_press_pulse(vm.animations_enabled);
            }
            rt.last_tick = Instant::now();
        }

        self.last_vm = Some(vm.clone());
        let missing_thumbnails = self.apply_current_vm()?;
        self.sync_anim_properties();

        if let Some(window) = &self.window {
            window.invoke_ensure_focused_visible();
            if just_opened {
                show_popup_without_stealing_focus("KnobContext Overlay", || {
                    window
                        .show()
                        .map_err(|err| KnobError::UiRun(err.to_string()))
                })?;
            }
        }

        self.ensure_anim_timer();

        Ok(RenderOutcome {
            just_opened,
            missing_thumbnails,
        })
    }

    pub fn on_thumbnail_ready(&mut self, decoded: DecodedThumbnail) -> Result<(), KnobError> {
        let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
            &decoded.rgba,
            decoded.width,
            decoded.height,
        );
        let img = Image::from_rgba8(buffer);
        let req = ThumbnailRequest {
            path: decoded.path,
            mode: decoded.mode,
        };
        self.thumb_cache.insert(req, img);

        if self.window.is_some() && self.last_vm.is_some() {
            let _ = self.apply_current_vm()?;
        }
        Ok(())
    }

    pub fn hide(&mut self) -> Result<(), KnobError> {
        self.anim_timer.stop();
        self.anim.borrow_mut().animator.reset();
        self.last_vm = None;
        if let Some(window) = self.window.take() {
            window
                .hide()
                .map_err(|err| KnobError::UiRun(err.to_string()))?;
        }
        Ok(())
    }

    fn sync_anim_properties(&self) {
        if let Some(window) = &self.window {
            let rt = self.anim.borrow();
            window.set_scroll_phase(rt.animator.scroll_phase);
            window.set_open_progress(rt.animator.open_progress);
            window.set_screen_progress(rt.animator.screen_progress);
            window.set_press_pulse(rt.animator.press_pulse);
            window.set_spinner_phase(rt.animator.spinner_phase);
        }
    }

    fn ensure_anim_timer(&self) {
        if !self.anim.borrow().animator.is_active() {
            self.anim_timer.stop();
            return;
        }
        let Some(window) = &self.window else {
            return;
        };
        let weak = window.as_weak();
        let anim_ref = Rc::clone(&self.anim);
        let timer_ref = Rc::clone(&self.anim_timer);
        self.anim_timer.start(
            TimerMode::Repeated,
            Duration::from_millis(ANIM_FRAME_MS),
            move || {
                let Some(win) = weak.upgrade() else {
                    timer_ref.stop();
                    return;
                };
                let (
                    still_active,
                    scroll_phase,
                    open_progress,
                    screen_progress,
                    press_pulse,
                    spinner_phase,
                ) = {
                    let mut rt = anim_ref.borrow_mut();
                    let now = Instant::now();
                    let dt = now.saturating_duration_since(rt.last_tick).as_secs_f32();
                    rt.last_tick = now;
                    let active = rt.animator.step(dt);
                    (
                        active,
                        rt.animator.scroll_phase,
                        rt.animator.open_progress,
                        rt.animator.screen_progress,
                        rt.animator.press_pulse,
                        rt.animator.spinner_phase,
                    )
                };
                win.set_scroll_phase(scroll_phase);
                win.set_open_progress(open_progress);
                win.set_screen_progress(screen_progress);
                win.set_press_pulse(press_pulse);
                win.set_spinner_phase(spinner_phase);
                if !still_active {
                    timer_ref.stop();
                }
            },
        );
    }

    fn resolve_card_image(
        &mut self,
        card: &HubCardView,
        missing: &mut Vec<ThumbnailRequest>,
    ) -> (bool, Image) {
        if let Some((w, h, ref rgba)) = card.inline_rgba {
            if w > 0 && h > 0 && rgba.len() == (w as usize) * (h as usize) * 4 {
                let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(rgba, w, h);
                return (true, Image::from_rgba8(buffer));
            }
        }
        let Some(ref path) = card.image_path else {
            return (false, Image::default());
        };
        let req = ThumbnailRequest {
            path: path.clone(),
            mode: if card.image_is_cover {
                ThumbnailMode::Cover {
                    width: THUMB_COVER_WIDTH,
                    height: THUMB_COVER_HEIGHT,
                }
            } else {
                ThumbnailMode::Contain {
                    size: THUMB_ICON_SIZE,
                }
            },
        };

        if let Some(cached) = self.thumb_cache.get(&req) {
            return (true, cached);
        }

        if is_svg_path(path) {
            if let Ok(svg_img) = Image::load_from_path(Path::new(path)) {
                self.thumb_cache.insert(req, svg_img.clone());
                return (true, svg_img);
            }
            return (false, Image::default());
        }

        if !missing.contains(&req) {
            missing.push(req);
        }
        (false, Image::default())
    }

    fn apply_current_vm(&mut self) -> Result<Vec<ThumbnailRequest>, KnobError> {
        let Some(vm) = self.last_vm.clone() else {
            return Ok(Vec::new());
        };

        let mut missing = Vec::new();
        let mut cards = Vec::with_capacity(vm.cards.len());
        for c in &vm.cards {
            let (has_image, image_source) = if c.is_empty_slot {
                (false, Image::default())
            } else {
                self.resolve_card_image(c, &mut missing)
            };
            cards.push(OverlayCardData {
                is_empty: c.is_empty_slot,
                title: SharedString::from(c.title.as_str()),
                subtitle: SharedString::from(c.subtitle.as_str()),
                badge: SharedString::from(c.badge.as_str()),
                glyph: SharedString::from(c.glyph.as_str()),
                has_image,
                image_is_cover: c.image_is_cover,
                image_source,
                value_percent: c.value_percent,
                max_percent: c.max_percent,
                is_slider: c.is_slider,
                is_active: c.is_active,
                is_muted: c.is_muted,
                is_playing: c.is_playing,
            });
        }

        for pre in &vm.prefetch_requests {
            if !is_svg_path(&pre.path)
                && self.thumb_cache.get(pre).is_none()
                && !missing.contains(pre)
            {
                missing.push(pre.clone());
            }
        }

        if let Some(window) = &self.window {
            window.set_parent_pill(SharedString::from(vm.parent_pill.as_str()));
            window.set_active_pill(SharedString::from(vm.active_pill.as_str()));
            window.set_status_pill(SharedString::from(vm.status_pill.as_str()));
            window.set_adjusting(vm.adjusting);
            window.set_loading(vm.loading);
            window.set_empty_message(SharedString::from(vm.empty_message.as_str()));
            window.set_cards(ModelRc::from(Rc::new(VecModel::from(cards))));
            window.set_footer_hint(SharedString::from(vm.footer_hint.as_str()));
            window.set_use_backdrop_blur(self.use_backdrop_blur);
            window.set_animations_enabled(vm.animations_enabled);
        }

        Ok(missing)
    }
}
