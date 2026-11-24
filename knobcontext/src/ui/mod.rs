pub mod action_editor_mod;
pub mod action_editor_state;
pub mod settings_mod;

use crate::error::KnobError;
use crate::overlay_hub::{
    HubCardView, HubViewModel, THUMB_COVER_HEIGHT, THUMB_COVER_WIDTH, THUMB_ICON_SIZE,
};
use crate::system::thumbs::{is_svg_path, DecodedThumbnail, ThumbnailMode, ThumbnailRequest};
use slint::{
    ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel,
};
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::rc::Rc;

pub const OVERLAY_WIDTH_PX: i32 = 960;
pub const OVERLAY_HEIGHT_PX: i32 = 256;
const MAX_RAM_THUMBNAILS: usize = 32;

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

/// Gerencia o ciclo de vida da janela Overlay (Carrossel Horizontal Caelestia) em Slint.
///
/// Descarta a instância no `hide()` e cria um `Overlay` novo ao abrir para preservar
/// o estado `floating` no Wayland/Hyprland (contornando o reset de `WindowAttributes`).
pub struct OverlayWindow {
    window: Option<Overlay>,
    last_vm: Option<HubViewModel>,
    thumb_cache: ThumbnailCache,
}

impl OverlayWindow {
    pub fn new() -> Self {
        Self {
            window: None,
            last_vm: None,
            thumb_cache: ThumbnailCache::new(),
        }
    }

    pub fn render(&mut self, vm: &HubViewModel) -> Result<RenderOutcome, KnobError> {
        let just_opened = self.window.is_none();
        if self.window.is_none() {
            let window = Overlay::new().map_err(|err| KnobError::UiInit(err.to_string()))?;
            self.window = Some(window);
        }

        self.last_vm = Some(vm.clone());
        let missing_thumbnails = self.apply_current_vm()?;

        if just_opened {
            if let Some(window) = &self.window {
                window
                    .show()
                    .map_err(|err| KnobError::UiRun(err.to_string()))?;
            }
        }

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
        self.last_vm = None;
        if let Some(window) = self.window.take() {
            window
                .hide()
                .map_err(|err| KnobError::UiRun(err.to_string()))?;
        }
        Ok(())
    }

    fn resolve_card_image(
        &mut self,
        card: &HubCardView,
        missing: &mut Vec<ThumbnailRequest>,
    ) -> (bool, Image) {
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

        // SVGs de ícones pequenos podem ser carregados diretamente pelo Slint (resvg embutido)
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
            });
        }

        // Pre-fetch dos vizinhos imediatos do carrossel que ainda não estão no cache
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
        }

        Ok(missing)
    }
}
