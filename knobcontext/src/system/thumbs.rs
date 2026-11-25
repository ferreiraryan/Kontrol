use crate::error::KnobError;
use image::imageops::FilterType;
use image::GenericImageView;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Modo de redimensionamento do thumbnail para manter consumo de RAM mínimo (< 150 KB por imagem).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThumbnailMode {
    /// Recorte preenchido 16:10 para wallpapers e fotos no carrossel (ex.: 240×140).
    Cover { width: u32, height: u32 },
    /// Contido preservando transparência/proporção para ícones de aplicativos (ex.: 96×96).
    Contain { size: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ThumbnailRequest {
    pub path: String,
    pub mode: ThumbnailMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedThumbnail {
    pub path: String,
    pub mode: ThumbnailMode,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn default_thumb_cache_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Ok(local_appdata) = env::var("LOCALAPPDATA") {
            if !local_appdata.trim().is_empty() {
                return Some(
                    PathBuf::from(local_appdata)
                        .join("knobcontext")
                        .join("thumbs"),
                );
            }
        }
        if let Ok(profile) = env::var("USERPROFILE") {
            if !profile.trim().is_empty() {
                return Some(
                    PathBuf::from(profile)
                        .join(".cache")
                        .join("knobcontext")
                        .join("thumbs"),
                );
            }
        }
    }
    if let Ok(xdg) = env::var("XDG_CACHE_HOME") {
        if !xdg.trim().is_empty() {
            return Some(PathBuf::from(xdg).join("knobcontext").join("thumbs"));
        }
    }
    env::var("HOME")
        .ok()
        .filter(|h| !h.trim().is_empty())
        .map(|h| {
            PathBuf::from(h)
                .join(".cache")
                .join("knobcontext")
                .join("thumbs")
        })
}

fn fnv1a_64(input: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in input {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn cache_file_path(cache_dir: &Path, src: &Path, mode: ThumbnailMode) -> PathBuf {
    let mtime_secs = fs::metadata(src)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mode_tag = match mode {
        ThumbnailMode::Cover { width, height } => format!("cov_{width}x{height}"),
        ThumbnailMode::Contain { size } => format!("con_{size}"),
    };
    let key_str = format!("{}|{}|{}", src.display(), mtime_secs, mode_tag);
    let hash = fnv1a_64(key_str.as_bytes());
    cache_dir.join(format!("{hash:016x}_{mode_tag}.png"))
}

pub fn load_or_generate_thumbnail(req: &ThumbnailRequest) -> Result<DecodedThumbnail, KnobError> {
    let cache_dir = default_thumb_cache_dir();
    load_or_generate_thumbnail_in(req, cache_dir.as_deref())
}

pub fn load_or_generate_thumbnail_in(
    req: &ThumbnailRequest,
    cache_dir: Option<&Path>,
) -> Result<DecodedThumbnail, KnobError> {
    let src = Path::new(&req.path);
    if !src.is_file() {
        return Err(KnobError::InputSimulation(format!(
            "imagem não encontrada: '{}'",
            req.path
        )));
    }

    // 1. Tenta cache em disco (PNG já redimensionado ~15-40 KB, decodifica em ~1-2ms)
    let disk_path = cache_dir.map(|dir| cache_file_path(dir, src, req.mode));
    if let Some(ref cached) = disk_path {
        if cached.is_file() {
            if let Ok(img) = image::open(cached) {
                let (w, h) = img.dimensions();
                return Ok(DecodedThumbnail {
                    path: req.path.clone(),
                    mode: req.mode,
                    width: w,
                    height: h,
                    rgba: img.to_rgba8().into_raw(),
                });
            }
        }
    }

    // 2. Decodifica original e redimensiona
    let img = image::open(src).map_err(|err| {
        KnobError::InputSimulation(format!("falha ao decodificar '{}': {err}", req.path))
    })?;
    let resized = match req.mode {
        ThumbnailMode::Cover { width, height } => {
            img.resize_to_fill(width, height, FilterType::Triangle)
        }
        ThumbnailMode::Contain { size } => img.thumbnail(size, size),
    };

    if let (Some(dir), Some(cached)) = (cache_dir, disk_path) {
        if fs::create_dir_all(dir).is_ok() {
            let _ = resized.save(&cached);
        }
    }

    let rgba_img = resized.to_rgba8();
    let (width, height) = rgba_img.dimensions();
    Ok(DecodedThumbnail {
        path: req.path.clone(),
        mode: req.mode,
        width,
        height,
        rgba: rgba_img.into_raw(),
    })
}

pub fn is_svg_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
}

#[cfg(test)]
mod tests {
    use super::{load_or_generate_thumbnail_in, ThumbnailMode, ThumbnailRequest};
    use image::{Rgba, RgbaImage};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn resizes_png_to_cover_and_reuses_disk_cache() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "knobcontext_thumbs_{}_{}",
            std::process::id(),
            nanos
        ));
        let cache = dir.join("cache");
        fs::create_dir_all(&dir).expect("dir temp");

        let src = dir.join("wall.png");
        let img = RgbaImage::from_pixel(640, 480, Rgba([40, 90, 200, 255]));
        img.save(&src).expect("salvar png sintético");

        let req = ThumbnailRequest {
            path: src.to_string_lossy().into_owned(),
            mode: ThumbnailMode::Cover {
                width: 240,
                height: 140,
            },
        };

        let first = load_or_generate_thumbnail_in(&req, Some(&cache)).expect("gerar thumb");
        assert_eq!(first.width, 240);
        assert_eq!(first.height, 140);
        assert_eq!(first.rgba.len(), (240 * 140 * 4) as usize);

        // Cache em disco criado
        let cached_count = fs::read_dir(&cache).expect("ler cache").count();
        assert_eq!(cached_count, 1);

        // Segunda chamada lê do cache com mesmo resultado
        let second = load_or_generate_thumbnail_in(&req, Some(&cache)).expect("hit cache");
        assert_eq!(second.width, 240);
        assert_eq!(second.height, 140);

        let _ = fs::remove_dir_all(&dir);
    }
}
