//! Image decoding, downscaling mip chains, transparency checkerboard, and the
//! shared zoomable/pannable image canvas used by image tabs and image Diffs (#1460).

use std::cell::Cell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::rc::Rc;
use std::sync::{Arc, LazyLock, OnceLock};

use gpui::prelude::*;
use gpui::{
    canvas, div, hsla, img, point, px, size, svg, AnyElement, App, Bounds, BoxShadow, Context,
    Corners, FocusHandle, FontWeight, Global, ImageSource, MouseButton, MouseDownEvent,
    MouseMoveEvent, PinchEvent, Pixels, Point, RenderImage, ScrollWheelEvent, SharedString, Task,
    Window,
};
use image::codecs::gif::GifDecoder;
use image::codecs::webp::WebPDecoder;
use image::metadata::Orientation;
use image::{
    AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat as ImgFmt, Limits, Rgba, RgbaImage,
};
use resvg::{tiny_skia, usvg};
use smallvec::SmallVec;

use crate::app_settings::DiffViewMode;
use crate::surfaces::side_panel::PanelCallback;
use crate::theme;

/// Maximum width or height accepted for decoding in the viewer.
pub(crate) const MAX_IMAGE_SIDE: u32 = 16_384;
/// Maximum total pixel count accepted for a still image or all frames of a GIF.
pub(crate) const MAX_IMAGE_PIXELS: u64 = 64_000_000;
/// Maximum texture side ever handed to GPUI's sprite atlas.
pub(crate) const MAX_TEXTURE_SIDE: u32 = 8_192;
/// Finite allocation budget during image header/metadata parsing (e.g. PNG ICC/zTXt chunks).
const MAX_HEADER_ALLOC_BYTES: u64 = 16 * 1024 * 1024;
/// Upper bound on total codec working memory for any accepted still image up to 64 MP.
const MAX_DECODE_ALLOC_BYTES: u64 = 512 * 1024 * 1024;
/// Upper bound on codec working memory for animated GIF/WebP compositing up to 64 MP
/// (two logical-screen RGBA buffers plus one active subframe buffer and codec overhead).
const MAX_ANIMATED_DECODE_ALLOC_BYTES: u64 = MAX_IMAGE_PIXELS * 16 + MAX_HEADER_ALLOC_BYTES;
/// Maximum encoded size for a single data-URI image embedded inside an SVG.
const MAX_EMBEDDED_IMAGE_BYTES: u64 = runner_core::protocol::files::MAX_READ_BYTES;

/// Margin around a fitted image inside the canvas.
pub(crate) const CANVAS_MARGIN: f32 = 16.;
/// Bottom clearance reserved for the floating zoom stepper (`12 + 24 + 12`).
pub(crate) const STEPPER_CLEARANCE: f32 = 48.;
/// Maximum zoom factor (800%).
pub(crate) const MAX_ZOOM: f32 = 8.0;

const ZOOM_STEPS: &[f32] = &[
    0.05, 0.10, 0.15, 0.25, 0.33, 0.50, 0.67, 0.75, 1.0, 1.25, 1.50, 2.0, 3.0, 4.0, 6.0, 8.0,
];
const MIN_MIP_SIDE: u32 = 128;
const CHECKER_SQUARE_PX: f32 = 8.;
const CHECKER_SQUARES_PER_TILE: u32 = 16;
const DRAG_THRESHOLD_PX: f32 = 3.;

/// Deterministic in-process content hash for detecting same-size image changes.
pub(crate) fn bytes_hash(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.len().hash(&mut hasher);
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// Checks whether raw bytes begin with a supported raster image header, so a
/// valid raster file lacking NUL bytes in its first 8 KB still opens as an
/// image while plain text (such as a Git LFS pointer) stays text.
pub(crate) fn detect_raster_magic(bytes: &[u8]) -> Option<gpui::ImageFormat> {
    match image::guess_format(bytes).ok()? {
        ImgFmt::Png => Some(gpui::ImageFormat::Png),
        ImgFmt::Jpeg => Some(gpui::ImageFormat::Jpeg),
        ImgFmt::Gif => Some(gpui::ImageFormat::Gif),
        ImgFmt::WebP => Some(gpui::ImageFormat::Webp),
        ImgFmt::Bmp => Some(gpui::ImageFormat::Bmp),
        _ => None,
    }
}

fn to_image_crate_format(format: gpui::ImageFormat) -> Option<ImgFmt> {
    match format {
        gpui::ImageFormat::Png => Some(ImgFmt::Png),
        gpui::ImageFormat::Jpeg => Some(ImgFmt::Jpeg),
        gpui::ImageFormat::Gif => Some(ImgFmt::Gif),
        gpui::ImageFormat::Webp => Some(ImgFmt::WebP),
        gpui::ImageFormat::Bmp => Some(ImgFmt::Bmp),
        _ => None,
    }
}

fn oriented_dimensions(width: u32, height: u32, orientation: Orientation) -> (u32, u32) {
    match orientation {
        Orientation::Rotate90
        | Orientation::Rotate270
        | Orientation::Rotate90FlipH
        | Orientation::Rotate270FlipH => (height, width),
        _ => (width, height),
    }
}

fn exceeds_pixel_limits(width: u32, height: u32) -> bool {
    width > MAX_IMAGE_SIDE
        || height > MAX_IMAGE_SIDE
        || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
}

/// Premultiplied-alpha 2×2 box filter so downscaled mip levels stay crisp and
/// transparent edges never pick up dark halos.
fn halve_rgba(src: &RgbaImage) -> RgbaImage {
    let (sw, sh) = src.dimensions();
    let dw = (sw / 2).max(1);
    let dh = (sh / 2).max(1);
    let src_raw = src.as_raw();
    let mut dst = vec![0u8; (dw as usize) * (dh as usize) * 4];
    for dy in 0..dh {
        let sy0 = (dy * 2).min(sh - 1);
        let sy1 = (sy0 + 1).min(sh - 1);
        for dx in 0..dw {
            let sx0 = (dx * 2).min(sw - 1);
            let sx1 = (sx0 + 1).min(sw - 1);
            let mut sum_ra = 0u64;
            let mut sum_ga = 0u64;
            let mut sum_ba = 0u64;
            let mut sum_a = 0u64;
            for (sx, sy) in [(sx0, sy0), (sx1, sy0), (sx0, sy1), (sx1, sy1)] {
                let i = ((sy as usize) * (sw as usize) + (sx as usize)) * 4;
                let a = u64::from(src_raw[i + 3]);
                sum_ra += u64::from(src_raw[i]) * a;
                sum_ga += u64::from(src_raw[i + 1]) * a;
                sum_ba += u64::from(src_raw[i + 2]) * a;
                sum_a += a;
            }
            let out_i = ((dy as usize) * (dw as usize) + (dx as usize)) * 4;
            let a = ((sum_a + 2) / 4) as u8;
            let half = sum_a / 2;
            if let (Some(r), Some(g), Some(b)) = (
                (sum_ra + half).checked_div(sum_a),
                (sum_ga + half).checked_div(sum_a),
                (sum_ba + half).checked_div(sum_a),
            ) {
                dst[out_i] = r.min(255) as u8;
                dst[out_i + 1] = g.min(255) as u8;
                dst[out_i + 2] = b.min(255) as u8;
                dst[out_i + 3] = a;
            }
        }
    }
    RgbaImage::from_raw(dw, dh, dst).expect("halved RGBA buffer matches dimensions")
}

fn rgba_to_render_image(mut rgba: RgbaImage) -> Arc<RenderImage> {
    for pixel in rgba.as_mut().as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Arc::new(RenderImage::new(SmallVec::from_const([image::Frame::new(
        rgba,
    )])))
}

fn clamp_frame_to_texture_limit(mut rgba: RgbaImage) -> RgbaImage {
    while rgba.width() > MAX_TEXTURE_SIDE || rgba.height() > MAX_TEXTURE_SIDE {
        rgba = halve_rgba(&rgba);
    }
    rgba
}

/// Builds successively halved BGRA `RenderImage` copies (largest first),
/// omitting any level over [`MAX_TEXTURE_SIDE`] on a side.
fn build_mip_chain(mut current: RgbaImage) -> Vec<Arc<RenderImage>> {
    let mut levels = Vec::new();
    loop {
        let (w, h) = current.dimensions();
        let can_halve = w > 1 || h > 1;
        let keep_halving =
            can_halve && (w.max(h) > MIN_MIP_SIDE || w > MAX_TEXTURE_SIDE || h > MAX_TEXTURE_SIDE);
        let next = keep_halving.then(|| halve_rgba(&current));
        if w <= MAX_TEXTURE_SIDE && h <= MAX_TEXTURE_SIDE {
            levels.push(rgba_to_render_image(current));
        }
        match next {
            Some(halved) => current = halved,
            None => break,
        }
    }
    levels
}

enum DecodedImageKind {
    /// Largest to smallest mip levels, each `<= MAX_TEXTURE_SIDE` on a side.
    /// An animated image stores a single multi-frame `RenderImage`.
    Raster(Vec<Arc<RenderImage>>),
    /// Parsed SVG tree plus its latest rasterized texture and target device size.
    Svg {
        tree: Arc<usvg::Tree>,
        raster: Arc<RenderImage>,
        raster_size: (u32, u32),
    },
}

/// Decoded image ready to render on [`ImageCanvas`].
pub(crate) struct DecodedImage {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) size: u64,
    kind: DecodedImageKind,
}

impl DecodedImage {
    pub(crate) fn is_animated(&self) -> bool {
        matches!(
            &self.kind,
            DecodedImageKind::Raster(levels)
                if levels.first().is_some_and(|img| img.frame_count() > 1)
        )
    }

    pub(crate) fn best_level(&self, target_dev_w: f32, target_dev_h: f32) -> Arc<RenderImage> {
        match &self.kind {
            DecodedImageKind::Raster(levels) => {
                if levels.len() <= 1 || self.is_animated() {
                    return levels[0].clone();
                }
                // `levels` is ordered largest -> smallest; walk from smallest upward
                // and pick the smallest level at least the drawn size in device pixels.
                for level in levels.iter().rev() {
                    let sz = level.size(0);
                    if (sz.width.0 as f32) >= target_dev_w && (sz.height.0 as f32) >= target_dev_h {
                        return level.clone();
                    }
                }
                levels[0].clone()
            }
            DecodedImageKind::Svg { raster, .. } => raster.clone(),
        }
    }

    pub(crate) fn svg_reraster_request(
        &self,
        target_dev_w: f32,
        target_dev_h: f32,
    ) -> Option<(Arc<usvg::Tree>, (u32, u32))> {
        let DecodedImageKind::Svg {
            tree, raster_size, ..
        } = &self.kind
        else {
            return None;
        };
        let desired = svg_target_device_size(self.width, self.height, target_dev_w, target_dev_h);
        (desired != *raster_size).then(|| (tree.clone(), desired))
    }

    pub(crate) fn replace_svg_raster(
        &mut self,
        next: Arc<RenderImage>,
        raster_size: (u32, u32),
    ) -> Option<Arc<RenderImage>> {
        if let DecodedImageKind::Svg {
            raster,
            raster_size: current_size,
            ..
        } = &mut self.kind
        {
            let old = std::mem::replace(raster, next);
            *current_size = raster_size;
            Some(old)
        } else {
            None
        }
    }

    pub(crate) fn drop_atlas_textures(&self, window: Option<&mut Window>, cx: &mut App) {
        match &self.kind {
            DecodedImageKind::Raster(levels) => {
                release_render_images(levels.iter().cloned(), window, cx);
            }
            DecodedImageKind::Svg { raster, .. } => {
                release_render_images([raster.clone()], window, cx);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn levels(&self) -> &[Arc<RenderImage>] {
        match &self.kind {
            DecodedImageKind::Raster(levels) => levels,
            DecodedImageKind::Svg { raster, .. } => std::slice::from_ref(raster),
        }
    }

    #[cfg(test)]
    pub(crate) fn frame_count(&self) -> usize {
        match &self.kind {
            DecodedImageKind::Raster(levels) => levels.first().map_or(0, |img| img.frame_count()),
            DecodedImageKind::Svg { raster, .. } => raster.frame_count(),
        }
    }
}

fn header_limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = None;
    limits.max_image_height = None;
    limits.max_alloc = Some(MAX_HEADER_ALLOC_BYTES);
    limits
}

fn decode_limits(width: u32, height: u32) -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    let pixel_budget = u64::from(width)
        .saturating_mul(u64::from(height))
        .saturating_mul(8)
        .saturating_add(MAX_HEADER_ALLOC_BYTES)
        .min(MAX_DECODE_ALLOC_BYTES);
    limits.max_alloc = Some(pixel_budget);
    limits
}

/// Finite budget for animated GIF/WebP compositing (`image`'s GIF iterator
/// retains two full RGBA logical screens plus the active subframe buffer).
fn animated_decode_limits(width: u32, height: u32) -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    let pixel_budget = u64::from(width)
        .saturating_mul(u64::from(height))
        .saturating_mul(16)
        .saturating_add(MAX_HEADER_ALLOC_BYTES)
        .min(MAX_ANIMATED_DECODE_ALLOC_BYTES);
    limits.max_alloc = Some(pixel_budget);
    limits
}

fn png_has_ancillary_chunk(data: &[u8], target: &[u8; 4]) -> bool {
    if data.len() < 8 || &data[..8] != b"\x89PNG\r\n\x1a\n" {
        return false;
    }
    let mut offset = 8usize;
    while offset + 12 <= data.len() {
        let len = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        let kind = &data[offset + 4..offset + 8];
        if kind == target {
            return true;
        }
        if kind == b"IDAT" || kind == b"IEND" {
            break;
        }
        let Some(next) = offset.checked_add(12).and_then(|n| n.checked_add(len)) else {
            break;
        };
        offset = next;
    }
    false
}

fn embedded_raster_within_limits(format: ImgFmt, data: &[u8]) -> bool {
    if data.len() as u64 > MAX_EMBEDDED_IMAGE_BYTES {
        return false;
    }
    match format {
        ImgFmt::Gif => {
            let Ok(mut decoder) = GifDecoder::new(Cursor::new(data)) else {
                return false;
            };
            let (w, h) = decoder.dimensions();
            if w == 0 || h == 0 || exceeds_pixel_limits(w, h) {
                return false;
            }
            decoder.set_limits(decode_limits(w, h)).is_ok()
        }
        ImgFmt::WebP => {
            let Ok(mut decoder) = WebPDecoder::new(Cursor::new(data)) else {
                return false;
            };
            let (raw_w, raw_h) = decoder.dimensions();
            if raw_w == 0 || raw_h == 0 {
                return false;
            }
            let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
            let (w, h) = oriented_dimensions(raw_w, raw_h, orientation);
            if exceeds_pixel_limits(w, h) {
                return false;
            }
            decoder.set_limits(decode_limits(raw_w, raw_h)).is_ok()
        }
        ImgFmt::Png | ImgFmt::Jpeg => {
            let mut probe = image::ImageReader::with_format(Cursor::new(data), format);
            probe.limits(header_limits());
            let Ok(mut decoder) = probe.into_decoder() else {
                return false;
            };
            let (raw_w, raw_h) = decoder.dimensions();
            if raw_w == 0 || raw_h == 0 {
                return false;
            }
            let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
            let (w, h) = oriented_dimensions(raw_w, raw_h, orientation);
            if exceeds_pixel_limits(w, h) {
                return false;
            }
            // Reject embedded PNGs whose iCCP chunk exceeded MAX_HEADER_ALLOC_BYTES
            // (png 0.18 drops the chunk silently, whereas resvg's tiny-skia decoder
            // uses a larger default budget).
            if format == ImgFmt::Png
                && png_has_ancillary_chunk(data, b"iCCP")
                && decoder.icc_profile().ok().flatten().is_none()
            {
                return false;
            }
            true
        }
        _ => false,
    }
}

fn svg_resolved_dimensions(tree: &usvg::Tree) -> Result<(u32, u32), Option<(u32, u32)>> {
    let sz = tree.size();
    let raw_w = sz.width();
    let raw_h = sz.height();
    if !raw_w.is_finite() || !raw_h.is_finite() || raw_w <= 0. || raw_h <= 0. {
        return Err(None);
    }
    let w_f = raw_w.ceil();
    let h_f = raw_h.ceil();
    if w_f > MAX_IMAGE_SIDE as f32
        || h_f > MAX_IMAGE_SIDE as f32
        || (w_f as f64) * (h_f as f64) > MAX_IMAGE_PIXELS as f64
    {
        return Err(Some((
            w_f.clamp(1.0, u32::MAX as f32) as u32,
            h_f.clamp(1.0, u32::MAX as f32) as u32,
        )));
    }
    let width = (w_f as u32).max(1);
    let height = (h_f as u32).max(1);
    if exceeds_pixel_limits(width, height) {
        return Err(Some((width, height)));
    }
    Ok((width, height))
}

fn resolve_safe_svg_data(
    mime: &str,
    data: Arc<Vec<u8>>,
    opts: &usvg::Options,
) -> Option<usvg::ImageKind> {
    if data.len() as u64 > MAX_EMBEDDED_IMAGE_BYTES {
        return None;
    }
    let load_sub_svg = |bytes: &[u8]| -> Option<usvg::ImageKind> {
        let text = std::str::from_utf8(bytes).ok()?;
        let sub = usvg::Tree::from_str(text, opts).ok()?;
        svg_resolved_dimensions(&sub).ok()?;
        Some(usvg::ImageKind::SVG(sub))
    };
    let check_and_wrap = |format: ImgFmt, raw: Arc<Vec<u8>>| -> Option<usvg::ImageKind> {
        if !embedded_raster_within_limits(format, &raw) {
            return None;
        }
        match format {
            ImgFmt::Jpeg => Some(usvg::ImageKind::JPEG(raw)),
            ImgFmt::Png => Some(usvg::ImageKind::PNG(raw)),
            ImgFmt::Gif => Some(usvg::ImageKind::GIF(raw)),
            ImgFmt::WebP => Some(usvg::ImageKind::WEBP(raw)),
            _ => None,
        }
    };
    match mime {
        "image/jpg" | "image/jpeg" => check_and_wrap(ImgFmt::Jpeg, data),
        "image/png" => check_and_wrap(ImgFmt::Png, data),
        "image/gif" => check_and_wrap(ImgFmt::Gif, data),
        "image/webp" => check_and_wrap(ImgFmt::WebP, data),
        "image/svg+xml" => load_sub_svg(&data),
        "text/plain" => match detect_raster_magic(&data).and_then(to_image_crate_format) {
            Some(fmt @ (ImgFmt::Jpeg | ImgFmt::Png | ImgFmt::Gif | ImgFmt::WebP)) => {
                check_and_wrap(fmt, data)
            }
            _ => load_sub_svg(&data),
        },
        _ => None,
    }
}

fn safe_svg_options() -> &'static usvg::Options<'static> {
    static SYSTEM_FONT_DB: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    });
    static OPTIONS: OnceLock<usvg::Options<'static>> = OnceLock::new();
    OPTIONS.get_or_init(|| {
        let default_font_resolver = usvg::FontResolver::default_font_selector();
        let select_font = Box::new(
            move |font: &usvg::Font, db: &mut Arc<usvg::fontdb::Database>| {
                if db.is_empty() {
                    *db = SYSTEM_FONT_DB.clone();
                }
                if let Some(id) = default_font_resolver(font, db) {
                    return Some(id);
                }
                let sans_query = usvg::fontdb::Query {
                    families: &[usvg::fontdb::Family::SansSerif],
                    ..Default::default()
                };
                db.query(&sans_query)
                    .or_else(|| db.faces().next().map(|face| face.id))
            },
        );
        usvg::Options {
            font_resolver: usvg::FontResolver {
                select_font,
                select_fallback: usvg::FontResolver::default_fallback_selector(),
            },
            image_href_resolver: usvg::ImageHrefResolver {
                resolve_data: Box::new(resolve_safe_svg_data),
                resolve_string: Box::new(|_, _| None),
            },
            ..Default::default()
        }
    })
}

fn rasterize_svg_tree(tree: &usvg::Tree, target_w: u32, target_h: u32) -> Option<Arc<RenderImage>> {
    let svg_size = tree.size();
    let w = target_w.clamp(1, MAX_TEXTURE_SIDE);
    let h = target_h.clamp(1, MAX_TEXTURE_SIDE);
    let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
    let transform =
        tiny_skia::Transform::from_scale(w as f32 / svg_size.width(), h as f32 / svg_size.height());
    resvg::render(tree, transform, &mut pixmap.as_mut());
    let mut buffer = RgbaImage::from_raw(pixmap.width(), pixmap.height(), pixmap.take())?;
    for pixel in buffer.as_chunks_mut::<4>().0 {
        gpui::swap_rgba_pa_to_bgra(pixel);
    }
    Some(Arc::new(RenderImage::new(SmallVec::from_const([
        image::Frame::new(buffer),
    ]))))
}

/// Computes the rasterization target in device pixels for an SVG, preserving
/// aspect ratio and staying within [`MAX_TEXTURE_SIDE`].
pub(crate) fn svg_target_device_size(
    intrinsic_w: u32,
    intrinsic_h: u32,
    target_dev_w: f32,
    target_dev_h: f32,
) -> (u32, u32) {
    let iw = intrinsic_w.max(1) as f32;
    let ih = intrinsic_h.max(1) as f32;
    let mut scale = (target_dev_w / iw)
        .max(target_dev_h / ih)
        .max(1.0 / iw.max(ih));
    let max_side = MAX_TEXTURE_SIDE as f32;
    if iw * scale > max_side || ih * scale > max_side {
        scale = (max_side / iw).min(max_side / ih);
    }
    let w = (iw * scale).round().clamp(1.0, max_side) as u32;
    let h = (ih * scale).round().clamp(1.0, max_side) as u32;
    (w, h)
}

/// Parses an SVG off the UI thread with host filesystem references disabled,
/// checks resolved intrinsic dimensions before allocating any pixels, and
/// produces an initial bounded raster texture.
pub(crate) fn decode_svg_image(bytes: &[u8], file_size: u64) -> DecodeOutcome {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return DecodeOutcome::Failed;
    };
    let Ok(tree) = usvg::Tree::from_str(text, safe_svg_options()) else {
        return DecodeOutcome::Failed;
    };
    let (width, height) = match svg_resolved_dimensions(&tree) {
        Ok(dims) => dims,
        Err(Some((width, height))) => {
            return DecodeOutcome::TooManyPixels {
                width,
                height,
                size: file_size,
            };
        }
        Err(None) => return DecodeOutcome::Failed,
    };
    let (tex_w, tex_h) = svg_target_device_size(width, height, width as f32, height as f32);
    let Some(raster) = rasterize_svg_tree(&tree, tex_w, tex_h) else {
        return DecodeOutcome::Failed;
    };
    DecodeOutcome::Ready(DecodedImage {
        width,
        height,
        size: file_size,
        kind: DecodedImageKind::Svg {
            tree: Arc::new(tree),
            raster,
            raster_size: (tex_w, tex_h),
        },
    })
}

pub(crate) enum DecodeOutcome {
    Ready(DecodedImage),
    TooManyPixels { width: u32, height: u32, size: u64 },
    Failed,
}

fn decode_animated_frames(
    frames_iter: image::Frames<'_>,
    fallback_w: u32,
    fallback_h: u32,
    file_size: u64,
) -> DecodeOutcome {
    decode_animated_frames_with_budget(
        frames_iter,
        fallback_w,
        fallback_h,
        file_size,
        MAX_IMAGE_PIXELS,
    )
}

fn decode_animated_frames_with_budget(
    frames_iter: image::Frames<'_>,
    fallback_w: u32,
    fallback_h: u32,
    file_size: u64,
    max_total_pixels: u64,
) -> DecodeOutcome {
    let mut frames: SmallVec<[image::Frame; 1]> = SmallVec::new();
    let mut total_pixels: u64 = 0;
    let mut truncated_to_still = false;

    for frame in frames_iter {
        let Ok(frame) = frame else {
            return DecodeOutcome::Failed;
        };
        let (fw, fh) = frame.buffer().dimensions();
        if fw == 0 || fh == 0 {
            continue;
        }
        let frame_pixels = u64::from(fw) * u64::from(fh);
        if !frames.is_empty() && total_pixels.saturating_add(frame_pixels) > max_total_pixels {
            frames.truncate(1);
            truncated_to_still = true;
            break;
        }
        total_pixels = total_pixels.saturating_add(frame_pixels);
        frames.push(frame);
        // Every composited frame covers the full logical screen; if a single frame
        // already exceeds half the pixel budget, a second frame could only fall back
        // to the same first-frame still, so stop before pulling frame 2.
        if frames.len() == 1 && total_pixels.saturating_mul(2) > max_total_pixels {
            truncated_to_still = true;
            break;
        }
    }

    if frames.is_empty() {
        return DecodeOutcome::Failed;
    }

    let (width, height) = frames
        .first()
        .map(|f| f.buffer().dimensions())
        .unwrap_or((fallback_w, fallback_h));

    if frames.len() == 1 || truncated_to_still {
        let first = frames.into_iter().next().unwrap().into_buffer();
        let levels = build_mip_chain(first);
        if levels.is_empty() {
            return DecodeOutcome::Failed;
        }
        return DecodeOutcome::Ready(DecodedImage {
            width,
            height,
            size: file_size,
            kind: DecodedImageKind::Raster(levels),
        });
    }

    let mut bgra_frames: SmallVec<[image::Frame; 1]> = SmallVec::with_capacity(frames.len());
    for frame in frames {
        let delay = frame.delay();
        let left = frame.left();
        let top = frame.top();
        let mut buf = clamp_frame_to_texture_limit(frame.into_buffer());
        for pixel in buf.as_mut().as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        bgra_frames.push(image::Frame::from_parts(buf, left, top, delay));
    }
    DecodeOutcome::Ready(DecodedImage {
        width,
        height,
        size: file_size,
        kind: DecodedImageKind::Raster(vec![Arc::new(RenderImage::new(bgra_frames))]),
    })
}

/// Decodes a raster image off the UI thread. Reads header dimensions under a
/// finite metadata allocation budget first so oversized images never allocate
/// pixel buffers and hostile metadata chunks cannot blow memory.
pub(crate) fn decode_raster_image(
    extension_format: gpui::ImageFormat,
    bytes: &[u8],
    file_size: u64,
) -> DecodeOutcome {
    let detected = detect_raster_magic(bytes).and_then(to_image_crate_format);
    let Some(format) = detected.or_else(|| to_image_crate_format(extension_format)) else {
        return DecodeOutcome::Failed;
    };

    if format == ImgFmt::Gif {
        let Ok(mut decoder) = GifDecoder::new(Cursor::new(bytes)) else {
            return DecodeOutcome::Failed;
        };
        let (w, h) = decoder.dimensions();
        if w == 0 || h == 0 {
            return DecodeOutcome::Failed;
        }
        if exceeds_pixel_limits(w, h) {
            return DecodeOutcome::TooManyPixels {
                width: w,
                height: h,
                size: file_size,
            };
        }
        if decoder.set_limits(animated_decode_limits(w, h)).is_err() {
            return DecodeOutcome::Failed;
        }
        return decode_animated_frames(decoder.into_frames(), w, h, file_size);
    }

    if format == ImgFmt::WebP {
        let Ok(mut decoder) = WebPDecoder::new(Cursor::new(bytes)) else {
            return DecodeOutcome::Failed;
        };
        let (raw_w, raw_h) = decoder.dimensions();
        if raw_w == 0 || raw_h == 0 {
            return DecodeOutcome::Failed;
        }
        let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
        let (w, h) = oriented_dimensions(raw_w, raw_h, orientation);
        if exceeds_pixel_limits(w, h) {
            return DecodeOutcome::TooManyPixels {
                width: w,
                height: h,
                size: file_size,
            };
        }
        let limits = if decoder.has_animation() {
            animated_decode_limits(raw_w, raw_h)
        } else {
            decode_limits(raw_w, raw_h)
        };
        if decoder.set_limits(limits).is_err() {
            return DecodeOutcome::Failed;
        }
        if decoder.has_animation() {
            let _ = decoder.set_background_color(Rgba([0, 0, 0, 0]));
            return decode_animated_frames(decoder.into_frames(), w, h, file_size);
        }
        let Ok(mut img) = DynamicImage::from_decoder(decoder) else {
            return DecodeOutcome::Failed;
        };
        img.apply_orientation(orientation);
        let rgba = img.into_rgba8();
        let (w, h) = rgba.dimensions();
        let levels = build_mip_chain(rgba);
        if levels.is_empty() {
            return DecodeOutcome::Failed;
        }
        return DecodeOutcome::Ready(DecodedImage {
            width: w,
            height: h,
            size: file_size,
            kind: DecodedImageKind::Raster(levels),
        });
    }

    let mut probe = image::ImageReader::with_format(Cursor::new(bytes), format);
    probe.limits(header_limits());
    let Ok(mut probe_decoder) = probe.into_decoder() else {
        return DecodeOutcome::Failed;
    };
    let (raw_w, raw_h) = probe_decoder.dimensions();
    if raw_w == 0 || raw_h == 0 {
        return DecodeOutcome::Failed;
    }
    let orientation = probe_decoder
        .orientation()
        .unwrap_or(Orientation::NoTransforms);
    let (w, h) = oriented_dimensions(raw_w, raw_h, orientation);
    if exceeds_pixel_limits(w, h) {
        return DecodeOutcome::TooManyPixels {
            width: w,
            height: h,
            size: file_size,
        };
    }

    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(decode_limits(raw_w, raw_h));
    let Ok(decoder) = reader.into_decoder() else {
        return DecodeOutcome::Failed;
    };
    let Ok(mut img) = DynamicImage::from_decoder(decoder) else {
        return DecodeOutcome::Failed;
    };
    img.apply_orientation(orientation);
    let rgba = img.into_rgba8();
    let (w, h) = rgba.dimensions();
    let levels = build_mip_chain(rgba);
    if levels.is_empty() {
        return DecodeOutcome::Failed;
    }
    DecodeOutcome::Ready(DecodedImage {
        width: w,
        height: h,
        size: file_size,
        kind: DecodedImageKind::Raster(levels),
    })
}

/// Drops `images` from every window's GPUI sprite atlas. When called without a
/// `&mut Window` (e.g. from `apply_read`, `set_shown(false)` or `on_release`),
/// schedules one extra foreground drop so a window currently checked out of
/// `App::windows` is also cleaned up once its update finishes.
pub(crate) fn release_render_images(
    images: impl IntoIterator<Item = Arc<RenderImage>>,
    window: Option<&mut Window>,
    cx: &mut App,
) {
    let list: Vec<Arc<RenderImage>> = images.into_iter().collect();
    if list.is_empty() {
        return;
    }
    if let Some(window) = window {
        for image in list {
            cx.drop_image(image, Some(window));
        }
        return;
    }
    for image in &list {
        cx.drop_image(image.clone(), None);
    }
    cx.spawn(async move |cx| {
        cx.update(|cx| {
            for image in list {
                cx.drop_image(image, None);
            }
        });
    })
    .detach();
}

/// Formats image dimensions and byte size for subbars and diff captions:
/// `3368 × 2196 · 886 KB`.
pub(crate) fn format_dimensions_and_size(width: u32, height: u32, size: u64) -> String {
    format!("{width} × {height} · {}", super::format_size(size))
}

// ── Checkerboard tile cache ────────────────────────────────────────────────

#[derive(Default)]
struct CheckerboardCache {
    dark: Option<(u32, Arc<RenderImage>)>,
    light: Option<(u32, Arc<RenderImage>)>,
}

impl Global for CheckerboardCache {}

fn checkerboard_tile(
    is_light: bool,
    scale_factor: f32,
    window: &mut Window,
    cx: &mut App,
) -> (f32, Arc<RenderImage>) {
    let sq = (CHECKER_SQUARE_PX * scale_factor).round().max(1.) as u32;
    let tile_logical = (CHECKER_SQUARES_PER_TILE * sq) as f32 / scale_factor.max(0.5);
    let cache = cx.default_global::<CheckerboardCache>();
    let slot = if is_light {
        &mut cache.light
    } else {
        &mut cache.dark
    };
    if let Some((cached_sq, tile)) = slot
        && *cached_sq == sq
    {
        return (tile_logical, tile.clone());
    }
    let old = slot.take().map(|(_, tile)| tile);

    let side = CHECKER_SQUARES_PER_TILE * sq;
    // BGRA colors matching the signed-off checkerboard pairs:
    // Dark: #1b1c21 / #26282f; Light: #e6e8ec / #ffffff.
    let (a, b): ([u8; 4], [u8; 4]) = if is_light {
        ([0xec, 0xe8, 0xe6, 0xff], [0xff, 0xff, 0xff, 0xff])
    } else {
        ([0x21, 0x1c, 0x1b, 0xff], [0x2f, 0x28, 0x26, 0xff])
    };
    let mut raw = vec![0u8; (side as usize) * (side as usize) * 4];
    for y in 0..side {
        let row_parity = (y / sq) & 1;
        for x in 0..side {
            let color = if ((x / sq) & 1) == row_parity { a } else { b };
            let i = ((y as usize) * (side as usize) + (x as usize)) * 4;
            raw[i..i + 4].copy_from_slice(&color);
        }
    }
    let buf = RgbaImage::from_raw(side, side, raw).expect("checkerboard buffer size");
    let tile = Arc::new(RenderImage::new(SmallVec::from_const([image::Frame::new(
        buf,
    )])));
    *slot = Some((sq, tile.clone()));
    if let Some(old) = old {
        cx.drop_image(old, Some(window));
    }
    (tile_logical, tile)
}

// ── Shared Zoom/Pan Image Canvas ───────────────────────────────────────────

/// Height of each side's caption bar in image Diff mode (`− Before ...` / `+ After ...`).
const CAPTION_HEIGHT: f32 = 28.;
/// Vertical margin around stacked panes in Unified image Diff mode.
const UNIFIED_IMAGE_GAP_MARGIN: f32 = 8.;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DiffCaptionKind {
    Before,
    After,
    Added,
    Deleted,
}

impl DiffCaptionKind {
    fn sign(self) -> (&'static str, gpui::Hsla) {
        match self {
            Self::Before | Self::Deleted => ("−", theme::danger()),
            Self::After | Self::Added => ("+", theme::accent()),
        }
    }

    fn label_key(self) -> &'static str {
        match self {
            Self::Before => "viewer.diff_before",
            Self::After => "viewer.diff_after",
            Self::Added => "viewer.diff_added",
            Self::Deleted => "viewer.diff_deleted",
        }
    }

    #[cfg(test)]
    fn selector(self) -> &'static str {
        match self {
            Self::Before => "IMAGE_DIFF_CAPTION_BEFORE",
            Self::After => "IMAGE_DIFF_CAPTION_AFTER",
            Self::Added => "IMAGE_DIFF_CAPTION_ADDED",
            Self::Deleted => "IMAGE_DIFF_CAPTION_DELETED",
        }
    }
}

pub(crate) enum DiffSideState {
    Ready(DecodedImage),
    TooManyPixels { width: u32, height: u32, size: u64 },
    Failed,
}

impl DiffSideState {
    fn image(&self) -> Option<&DecodedImage> {
        match self {
            Self::Ready(img) => Some(img),
            _ => None,
        }
    }

    fn image_mut(&mut self) -> Option<&mut DecodedImage> {
        match self {
            Self::Ready(img) => Some(img),
            _ => None,
        }
    }

    fn dimensions(&self) -> Option<(u32, u32)> {
        self.image().map(|img| (img.width, img.height))
    }

    fn meta_text(&self) -> Option<String> {
        match self {
            Self::Ready(img) => Some(format_dimensions_and_size(img.width, img.height, img.size)),
            Self::TooManyPixels {
                width,
                height,
                size,
            } => Some(format_dimensions_and_size(*width, *height, *size)),
            Self::Failed => None,
        }
    }

    fn drop_atlas_textures(&self, window: Option<&mut Window>, cx: &mut App) {
        if let Self::Ready(img) = self {
            img.drop_atlas_textures(window, cx);
        }
    }
}

pub(crate) struct DiffSide {
    pub(crate) caption: DiffCaptionKind,
    pub(crate) file_name: String,
    pub(crate) state: DiffSideState,
}

pub(crate) enum DiffSides {
    One(Box<DiffSide>),
    Two {
        before: Box<DiffSide>,
        after: Box<DiffSide>,
    },
}

impl DiffSides {
    pub(crate) fn is_one_sided(&self) -> bool {
        matches!(self, Self::One(_))
    }

    pub(crate) fn has_drawable(&self) -> bool {
        match self {
            Self::One(side) => side.state.image().is_some(),
            Self::Two { before, after } => {
                before.state.image().is_some() || after.state.image().is_some()
            }
        }
    }

    pub(crate) fn has_too_many_pixels(&self) -> bool {
        match self {
            Self::One(side) => matches!(side.state, DiffSideState::TooManyPixels { .. }),
            Self::Two { before, after } => {
                matches!(before.state, DiffSideState::TooManyPixels { .. })
                    || matches!(after.state, DiffSideState::TooManyPixels { .. })
            }
        }
    }

    fn slot(&self, idx: usize) -> Option<&DiffSide> {
        match (self, idx) {
            (Self::One(side), 0) => Some(side),
            (Self::Two { before, .. }, 0) => Some(before),
            (Self::Two { after, .. }, 1) => Some(after),
            _ => None,
        }
    }

    fn slot_mut(&mut self, idx: usize) -> Option<&mut DiffSide> {
        match (self, idx) {
            (Self::One(side), 0) => Some(side),
            (Self::Two { before, .. }, 0) => Some(before),
            (Self::Two { after, .. }, 1) => Some(after),
            _ => None,
        }
    }

    fn dimension_signature(&self) -> [Option<(u32, u32)>; 2] {
        [
            self.slot(0).and_then(|s| s.state.dimensions()),
            self.slot(1).and_then(|s| s.state.dimensions()),
        ]
    }

    fn drop_atlas_textures(&self, mut window: Option<&mut Window>, cx: &mut App) {
        match self {
            Self::One(side) => side.state.drop_atlas_textures(window, cx),
            Self::Two { before, after } => {
                before.state.drop_atlas_textures(window.as_deref_mut(), cx);
                after.state.drop_atlas_textures(window, cx);
            }
        }
    }
}

enum CanvasContent {
    Single(DecodedImage),
    Diff {
        layout: DiffViewMode,
        sides: DiffSides,
    },
}

impl CanvasContent {
    fn image_at(&self, slot: usize) -> Option<&DecodedImage> {
        match (self, slot) {
            (Self::Single(img), 0) => Some(img),
            (Self::Diff { sides, .. }, slot) => sides.slot(slot).and_then(|s| s.state.image()),
            _ => None,
        }
    }

    fn image_at_mut(&mut self, slot: usize) -> Option<&mut DecodedImage> {
        match (self, slot) {
            (Self::Single(img), 0) => Some(img),
            (Self::Diff { sides, .. }, slot) => {
                sides.slot_mut(slot).and_then(|s| s.state.image_mut())
            }
            _ => None,
        }
    }

    fn drop_atlas_textures(&self, window: Option<&mut Window>, cx: &mut App) {
        match self {
            Self::Single(img) => img.drop_atlas_textures(window, cx),
            Self::Diff { sides, .. } => sides.drop_atlas_textures(window, cx),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum ZoomMode {
    #[default]
    Fit,
    Custom(f32),
}

#[derive(Clone, Copy, Debug)]
struct PaneGeometry {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    drawn_w: f32,
    drawn_h: f32,
    base_center_x: f32,
    base_center_y: f32,
    max_pan_x: f32,
    max_pan_y: f32,
    has_image: bool,
}

impl PaneGeometry {
    fn clamp_pan(&self, pan: Point<f32>) -> Point<f32> {
        point(
            pan.x.clamp(-self.max_pan_x, self.max_pan_x),
            pan.y.clamp(-self.max_pan_y, self.max_pan_y),
        )
    }

    fn image_origin(&self, pan: Point<f32>) -> Point<f32> {
        let pan = self.clamp_pan(pan);
        point(
            (self.base_center_x + pan.x - self.drawn_w * 0.5).round(),
            (self.base_center_y + pan.y - self.drawn_h * 0.5).round(),
        )
    }
}

#[derive(Clone, Copy, Debug)]
struct CanvasGeometry {
    fit_scale: f32,
    scale: f32,
    max_pan_x: f32,
    max_pan_y: f32,
    has_image: bool,
    panes: [Option<PaneGeometry>; 2],
    divider_x: Option<f32>,
    captions: [Option<(f32, f32, f32)>; 2],
}

#[derive(Clone, Copy, Debug)]
struct PaneRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl CanvasGeometry {
    fn build_pane(
        rect: PaneRect,
        safe_w: f32,
        safe_h: f32,
        safe_top: f32,
        dims: Option<(u32, u32)>,
        scale: f32,
    ) -> PaneGeometry {
        let PaneRect { x, y, w, h } = rect;
        let Some((img_w, img_h)) = dims else {
            return PaneGeometry {
                x,
                y,
                w,
                h,
                drawn_w: 0.,
                drawn_h: 0.,
                base_center_x: w * 0.5,
                base_center_y: h * 0.5,
                max_pan_x: 0.,
                max_pan_y: 0.,
                has_image: false,
            };
        };
        let drawn_w = img_w.max(1) as f32 * scale;
        let drawn_h = img_h.max(1) as f32 * scale;
        let max_pan_x = ((drawn_w - w) * 0.5).max(0.);
        let max_pan_y = ((drawn_h - h) * 0.5).max(0.);
        let base_center_x = w * 0.5;
        let base_center_y = if drawn_h <= safe_h + 0.5 && drawn_w <= safe_w + 0.5 {
            safe_top + safe_h * 0.5
        } else {
            h * 0.5
        };
        PaneGeometry {
            x,
            y,
            w,
            h,
            drawn_w,
            drawn_h,
            base_center_x,
            base_center_y,
            max_pan_x,
            max_pan_y,
            has_image: true,
        }
    }

    fn compute_single(vp_w: f32, vp_h: f32, img_w: u32, img_h: u32, zoom: ZoomMode) -> Self {
        let safe_w = (vp_w - CANVAS_MARGIN * 2.).max(1.);
        let safe_h = (vp_h - CANVAS_MARGIN - STEPPER_CLEARANCE).max(1.);
        let fit_scale = (safe_w / img_w.max(1) as f32)
            .min(safe_h / img_h.max(1) as f32)
            .min(1.0);
        let scale = match zoom {
            ZoomMode::Fit => fit_scale,
            ZoomMode::Custom(s) => s.clamp(fit_scale, MAX_ZOOM),
        };
        let pane = Self::build_pane(
            PaneRect {
                x: 0.,
                y: 0.,
                w: vp_w,
                h: vp_h,
            },
            safe_w,
            safe_h,
            CANVAS_MARGIN,
            Some((img_w, img_h)),
            scale,
        );
        Self {
            fit_scale,
            scale,
            max_pan_x: pane.max_pan_x,
            max_pan_y: pane.max_pan_y,
            has_image: true,
            panes: [Some(pane), None],
            divider_x: None,
            captions: [None, None],
        }
    }

    fn compute_diff(
        vp_w: f32,
        vp_h: f32,
        layout: DiffViewMode,
        sides: &DiffSides,
        zoom: ZoomMode,
    ) -> Self {
        match sides {
            DiffSides::One(side) => {
                let pane_h = (vp_h - CAPTION_HEIGHT).max(1.);
                let safe_w = (vp_w - CANVAS_MARGIN * 2.).max(1.);
                let safe_h = (pane_h - CANVAS_MARGIN - STEPPER_CLEARANCE).max(1.);
                let dims = side.state.dimensions();
                let fit_scale = dims.map_or(1.0, |(w, h)| {
                    (safe_w / w.max(1) as f32)
                        .min(safe_h / h.max(1) as f32)
                        .min(1.0)
                });
                let scale = match zoom {
                    ZoomMode::Fit => fit_scale,
                    ZoomMode::Custom(s) => s.clamp(fit_scale, MAX_ZOOM),
                };
                let pane = Self::build_pane(
                    PaneRect {
                        x: 0.,
                        y: CAPTION_HEIGHT,
                        w: vp_w,
                        h: pane_h,
                    },
                    safe_w,
                    safe_h,
                    CANVAS_MARGIN,
                    dims,
                    scale,
                );
                Self {
                    fit_scale,
                    scale,
                    max_pan_x: pane.max_pan_x,
                    max_pan_y: pane.max_pan_y,
                    has_image: pane.has_image,
                    panes: [Some(pane), None],
                    divider_x: None,
                    captions: [Some((0., 0., vp_w)), None],
                }
            }
            DiffSides::Two { before, after } => {
                let d0 = before.state.dimensions();
                let d1 = after.state.dimensions();
                match layout {
                    DiffViewMode::Split => {
                        let left_w = ((vp_w - 1.0) * 0.5).floor().max(1.);
                        let right_x = left_w + 1.0;
                        let right_w = (vp_w - right_x).max(1.);
                        let pane_h = (vp_h - CAPTION_HEIGHT).max(1.);
                        let safe_w0 = (left_w - CANVAS_MARGIN * 2.).max(1.);
                        let safe_w1 = (right_w - CANVAS_MARGIN * 2.).max(1.);
                        let safe_h = (pane_h - CANVAS_MARGIN - STEPPER_CLEARANCE).max(1.);
                        let fit0 = d0.map(|(w, h)| {
                            (safe_w0 / w.max(1) as f32)
                                .min(safe_h / h.max(1) as f32)
                                .min(1.0)
                        });
                        let fit1 = d1.map(|(w, h)| {
                            (safe_w1 / w.max(1) as f32)
                                .min(safe_h / h.max(1) as f32)
                                .min(1.0)
                        });
                        let fit_scale = match (fit0, fit1) {
                            (Some(a), Some(b)) => a.min(b),
                            (Some(a), None) => a,
                            (None, Some(b)) => b,
                            (None, None) => 1.0,
                        };
                        let scale = match zoom {
                            ZoomMode::Fit => fit_scale,
                            ZoomMode::Custom(s) => s.clamp(fit_scale, MAX_ZOOM),
                        };
                        let p0 = Self::build_pane(
                            PaneRect {
                                x: 0.,
                                y: CAPTION_HEIGHT,
                                w: left_w,
                                h: pane_h,
                            },
                            safe_w0,
                            safe_h,
                            CANVAS_MARGIN,
                            d0,
                            scale,
                        );
                        let p1 = Self::build_pane(
                            PaneRect {
                                x: right_x,
                                y: CAPTION_HEIGHT,
                                w: right_w,
                                h: pane_h,
                            },
                            safe_w1,
                            safe_h,
                            CANVAS_MARGIN,
                            d1,
                            scale,
                        );
                        Self {
                            fit_scale,
                            scale,
                            max_pan_x: p0.max_pan_x.max(p1.max_pan_x),
                            max_pan_y: p0.max_pan_y.max(p1.max_pan_y),
                            has_image: p0.has_image || p1.has_image,
                            panes: [Some(p0), Some(p1)],
                            divider_x: Some(left_w),
                            captions: [Some((0., 0., left_w)), Some((right_x, 0., right_w))],
                        }
                    }
                    DiffViewMode::Unified => {
                        let avail_h = (vp_h - 2.0 * CAPTION_HEIGHT).max(2.0);
                        let margin_top = UNIFIED_IMAGE_GAP_MARGIN * 2.0;
                        let margin_bot = UNIFIED_IMAGE_GAP_MARGIN + STEPPER_CLEARANCE;
                        let total_safe_h = (avail_h - margin_top - margin_bot).max(2.0);
                        let safe_w = (vp_w - CANVAS_MARGIN * 2.).max(1.);
                        let (pane_h0, pane_h1, safe_h0, safe_h1, fit_scale) = match (d0, d1) {
                            (Some((w0, h0)), Some((w1, h1))) => {
                                let wf0 = w0.max(1) as f32;
                                let hf0 = h0.max(1) as f32;
                                let wf1 = w1.max(1) as f32;
                                let hf1 = h1.max(1) as f32;
                                let fit = (safe_w / wf0.max(wf1))
                                    .min(total_safe_h / (hf0 + hf1))
                                    .min(1.0);
                                let sh0 = (total_safe_h * hf0 / (hf0 + hf1)).max(1.0);
                                let ph0 = (sh0 + margin_top).min(avail_h - 1.0).max(1.0);
                                let ph1 = (avail_h - ph0).max(1.0);
                                let sh1 = (ph1 - margin_bot).max(1.0);
                                (ph0, ph1, sh0, sh1, fit)
                            }
                            (Some((w0, h0)), None) => {
                                let ph0 = (avail_h * 0.5).floor().max(1.0);
                                let ph1 = (avail_h - ph0).max(1.0);
                                let sh0 = (ph0 - margin_top).max(1.0);
                                let sh1 = (ph1 - margin_bot).max(1.0);
                                let fit = (safe_w / w0.max(1) as f32)
                                    .min(sh0 / h0.max(1) as f32)
                                    .min(1.0);
                                (ph0, ph1, sh0, sh1, fit)
                            }
                            (None, Some((w1, h1))) => {
                                let ph0 = (avail_h * 0.5).floor().max(1.0);
                                let ph1 = (avail_h - ph0).max(1.0);
                                let sh0 = (ph0 - margin_top).max(1.0);
                                let sh1 = (ph1 - margin_bot).max(1.0);
                                let fit = (safe_w / w1.max(1) as f32)
                                    .min(sh1 / h1.max(1) as f32)
                                    .min(1.0);
                                (ph0, ph1, sh0, sh1, fit)
                            }
                            (None, None) => {
                                let ph0 = (avail_h * 0.5).floor().max(1.0);
                                let ph1 = (avail_h - ph0).max(1.0);
                                (ph0, ph1, 1.0, 1.0, 1.0)
                            }
                        };
                        let scale = match zoom {
                            ZoomMode::Fit => fit_scale,
                            ZoomMode::Custom(s) => s.clamp(fit_scale, MAX_ZOOM),
                        };
                        let p0 = Self::build_pane(
                            PaneRect {
                                x: 0.,
                                y: CAPTION_HEIGHT,
                                w: vp_w,
                                h: pane_h0,
                            },
                            safe_w,
                            safe_h0,
                            UNIFIED_IMAGE_GAP_MARGIN,
                            d0,
                            scale,
                        );
                        let cap1_y = CAPTION_HEIGHT + pane_h0;
                        let p1 = Self::build_pane(
                            PaneRect {
                                x: 0.,
                                y: cap1_y + CAPTION_HEIGHT,
                                w: vp_w,
                                h: pane_h1,
                            },
                            safe_w,
                            safe_h1,
                            UNIFIED_IMAGE_GAP_MARGIN,
                            d1,
                            scale,
                        );
                        Self {
                            fit_scale,
                            scale,
                            max_pan_x: p0.max_pan_x.max(p1.max_pan_x),
                            max_pan_y: p0.max_pan_y.max(p1.max_pan_y),
                            has_image: p0.has_image || p1.has_image,
                            panes: [Some(p0), Some(p1)],
                            divider_x: None,
                            captions: [Some((0., 0., vp_w)), Some((0., cap1_y, vp_w))],
                        }
                    }
                }
            }
        }
    }

    fn clamp_pan(&self, pan: Point<f32>) -> Point<f32> {
        point(
            pan.x.clamp(-self.max_pan_x, self.max_pan_x),
            pan.y.clamp(-self.max_pan_y, self.max_pan_y),
        )
    }

    fn pane_for_point(&self, pt: Point<f32>) -> Option<(usize, PaneGeometry)> {
        for (idx, pane) in self.panes.into_iter().enumerate() {
            if let Some(p) = pane
                && p.has_image
                && pt.x >= p.x
                && pt.x <= p.x + p.w
                && pt.y >= p.y
                && pt.y <= p.y + p.h
            {
                return Some((idx, p));
            }
        }
        None
    }
}

type SvgRasterTask = Option<((u32, u32), Task<()>)>;

pub(crate) struct ImageCanvas {
    focus: FocusHandle,
    content: CanvasContent,
    zoom: ZoomMode,
    pan: Point<f32>,
    viewport: Rc<Cell<Bounds<Pixels>>>,
    deleted: bool,
    drag: Option<(Point<Pixels>, Point<f32>, bool)>,
    svg_generation: u64,
    svg_raster_tasks: [SvgRasterTask; 2],
    on_focus_in: Option<PanelCallback>,
    #[cfg(test)]
    last_image_bounds: [Cell<Option<Bounds<Pixels>>>; 2],
    #[cfg(test)]
    last_stepper_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    #[cfg(test)]
    last_level_size: Cell<Option<(u32, u32)>>,
}

impl ImageCanvas {
    fn new_with_content(content: CanvasContent, cx: &mut Context<Self>) -> Self {
        cx.on_release(|this: &mut Self, cx| {
            this.content.drop_atlas_textures(None, cx);
        })
        .detach();
        Self {
            focus: cx.focus_handle(),
            content,
            zoom: ZoomMode::Fit,
            pan: point(0., 0.),
            viewport: Rc::default(),
            deleted: false,
            drag: None,
            svg_generation: 0,
            svg_raster_tasks: [None, None],
            on_focus_in: None,
            #[cfg(test)]
            last_image_bounds: [Cell::new(None), Cell::new(None)],
            #[cfg(test)]
            last_stepper_bounds: Rc::default(),
            #[cfg(test)]
            last_level_size: Cell::new(None),
        }
    }

    pub(crate) fn new_single(image: DecodedImage, cx: &mut Context<Self>) -> Self {
        Self::new_with_content(CanvasContent::Single(image), cx)
    }

    pub(crate) fn new_diff(layout: DiffViewMode, sides: DiffSides, cx: &mut Context<Self>) -> Self {
        Self::new_with_content(CanvasContent::Diff { layout, sides }, cx)
    }

    pub(crate) fn set_on_focus_in(&mut self, handler: Option<PanelCallback>) {
        self.on_focus_in = handler;
    }

    pub(crate) fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    pub(crate) fn set_deleted(&mut self, deleted: bool, cx: &mut Context<Self>) {
        if self.deleted != deleted {
            self.deleted = deleted;
            cx.notify();
        }
    }

    pub(crate) fn set_diff_mode(&mut self, layout: DiffViewMode, cx: &mut Context<Self>) {
        if let CanvasContent::Diff {
            layout: current, ..
        } = &mut self.content
            && *current != layout
        {
            *current = layout;
            if let Some(geom) = self.current_geometry() {
                if let ZoomMode::Custom(s) = self.zoom
                    && s <= geom.fit_scale + 0.001
                {
                    self.zoom = ZoomMode::Fit;
                    self.pan = point(0., 0.);
                } else {
                    self.pan = geom.clamp_pan(self.pan);
                }
            }
            cx.notify();
        }
    }

    /// Replaces the single image. Keeps zoom and pan when dimensions are
    /// unchanged; resets to fit otherwise. Releases the replaced image's atlas
    /// textures.
    pub(crate) fn set_single_image(&mut self, next: DecodedImage, cx: &mut Context<Self>) {
        let same_dimensions = matches!(
            &self.content,
            CanvasContent::Single(cur) if cur.width == next.width && cur.height == next.height
        );
        self.svg_generation = self.svg_generation.wrapping_add(1);
        self.svg_raster_tasks = [None, None];
        let old = std::mem::replace(&mut self.content, CanvasContent::Single(next));
        old.drop_atlas_textures(None, cx);
        if !same_dimensions {
            self.zoom = ZoomMode::Fit;
            self.pan = point(0., 0.);
        } else if let Some(geom) = self.current_geometry() {
            self.pan = geom.clamp_pan(self.pan);
        }
        cx.notify();
    }

    /// Replaces Diff sides. Keeps zoom and pan when every side's dimensions are
    /// unchanged; resets to fit otherwise. Releases the replaced textures.
    pub(crate) fn set_diff_sides(
        &mut self,
        layout: DiffViewMode,
        next: DiffSides,
        cx: &mut Context<Self>,
    ) {
        let same_dimensions = matches!(
            &self.content,
            CanvasContent::Diff { sides, .. } if sides.dimension_signature() == next.dimension_signature()
        );
        self.svg_generation = self.svg_generation.wrapping_add(1);
        self.svg_raster_tasks = [None, None];
        let old = std::mem::replace(
            &mut self.content,
            CanvasContent::Diff {
                layout,
                sides: next,
            },
        );
        old.drop_atlas_textures(None, cx);
        if !same_dimensions {
            self.zoom = ZoomMode::Fit;
            self.pan = point(0., 0.);
        } else if let Some(geom) = self.current_geometry() {
            self.pan = geom.clamp_pan(self.pan);
        }
        cx.notify();
    }

    fn schedule_svg_reraster(
        &mut self,
        slot: usize,
        tree: Arc<usvg::Tree>,
        desired: (u32, u32),
        cx: &mut Context<Self>,
    ) {
        if self.svg_raster_tasks[slot]
            .as_ref()
            .is_some_and(|(in_flight, _)| *in_flight == desired)
        {
            return;
        }
        let generation = self.svg_generation;
        let task =
            cx.background_spawn(async move { rasterize_svg_tree(&tree, desired.0, desired.1) });
        let handle = cx.spawn(async move |this, cx| {
            let rendered = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.svg_generation != generation {
                    return;
                }
                this.svg_raster_tasks[slot] = None;
                if let Some(next_raster) = rendered
                    && let Some(img) = this.content.image_at_mut(slot)
                    && let Some(old_raster) = img.replace_svg_raster(next_raster, desired)
                {
                    release_render_images([old_raster], None, cx);
                    cx.notify();
                }
            });
        });
        self.svg_raster_tasks[slot] = Some((desired, handle));
    }

    pub(crate) fn drop_atlas_textures(&self, window: Option<&mut Window>, cx: &mut App) {
        self.content.drop_atlas_textures(window, cx);
    }

    fn compute_geometry(&self, vp_w: f32, vp_h: f32, zoom: ZoomMode) -> CanvasGeometry {
        match &self.content {
            CanvasContent::Single(img) => {
                CanvasGeometry::compute_single(vp_w, vp_h, img.width, img.height, zoom)
            }
            CanvasContent::Diff { layout, sides } => {
                CanvasGeometry::compute_diff(vp_w, vp_h, *layout, sides, zoom)
            }
        }
    }

    fn current_geometry(&self) -> Option<CanvasGeometry> {
        let bounds = self.viewport.get();
        let vp_w = f32::from(bounds.size.width);
        let vp_h = f32::from(bounds.size.height);
        if vp_w <= 0. || vp_h <= 0. {
            return None;
        }
        Some(self.compute_geometry(vp_w, vp_h, self.zoom))
    }

    fn on_viewport_resized(&mut self, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        self.viewport.set(bounds);
        if let Some(geom) = self.current_geometry() {
            if let ZoomMode::Custom(s) = self.zoom
                && s <= geom.fit_scale + 0.001
            {
                self.zoom = ZoomMode::Fit;
                self.pan = point(0., 0.);
            } else {
                self.pan = geom.clamp_pan(self.pan);
            }
        }
        cx.notify();
    }

    /// Zooms around each pane's visual center, preserving shared pan per axis
    /// on any overflowing pane and compensating for vertical `base_center_y`
    /// shifts when crossing the reserved stepper clearance.
    fn zoom_to_center(&mut self, target: ZoomMode, cx: &mut Context<Self>) {
        let Some(old) = self.current_geometry() else {
            self.zoom = target;
            if matches!(target, ZoomMode::Fit) {
                self.pan = point(0., 0.);
            }
            cx.notify();
            return;
        };
        if !old.has_image {
            return;
        }
        // Pick the drawable pane with the widest pan range on each axis so
        // unequal Diff sides (where one fits and the other overflows) never
        // zero out the shared pan offset on zoom.
        let pick_axis = |key: fn(&PaneGeometry) -> f32| {
            old.panes
                .into_iter()
                .enumerate()
                .filter_map(|(idx, p)| p.filter(|p| p.has_image).map(|p| (idx, p)))
                .max_by(|(_, a), (_, b)| key(a).total_cmp(&key(b)))
        };
        let Some((pane_x_idx, old_pane_x)) = pick_axis(|p| p.max_pan_x) else {
            return;
        };
        let Some((pane_y_idx, old_pane_y)) = pick_axis(|p| p.max_pan_y) else {
            return;
        };
        let old_pan_x = old_pane_x.clamp_pan(self.pan).x;
        let old_pan_y = old_pane_y.clamp_pan(self.pan).y;
        let img_pt_x = (old_pane_x.w * 0.5 - (old_pane_x.base_center_x + old_pan_x)) / old.scale;
        let img_pt_y = (old_pane_y.h * 0.5 - (old_pane_y.base_center_y + old_pan_y)) / old.scale;

        let bounds = self.viewport.get();
        let next = self.compute_geometry(
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
            target,
        );
        self.zoom = match target {
            ZoomMode::Fit => ZoomMode::Fit,
            ZoomMode::Custom(_) if (next.scale - next.fit_scale).abs() < 0.0005 => ZoomMode::Fit,
            ZoomMode::Custom(_) => ZoomMode::Custom(next.scale),
        };
        if matches!(self.zoom, ZoomMode::Fit) {
            self.pan = point(0., 0.);
        } else {
            let next_pan_x = next.panes[pane_x_idx].map_or(0., |next_pane| {
                (next_pane.w * 0.5 - img_pt_x * next.scale) - next_pane.base_center_x
            });
            let next_pan_y = next.panes[pane_y_idx].map_or(0., |next_pane| {
                (next_pane.h * 0.5 - img_pt_y * next.scale) - next_pane.base_center_y
            });
            self.pan = next.clamp_pan(point(next_pan_x, next_pan_y));
        }
        cx.notify();
    }

    /// Zooms to `target` keeping `anchor_in_viewport` (relative to the canvas
    /// viewport origin) fixed on the same image pixel.
    fn zoom_to_at(
        &mut self,
        target: ZoomMode,
        anchor_in_viewport: Point<f32>,
        cx: &mut Context<Self>,
    ) {
        let Some(old) = self.current_geometry() else {
            self.zoom_to_center(target, cx);
            return;
        };
        if !old.has_image {
            return;
        }
        let Some((pane_idx, old_pane)) = old.pane_for_point(anchor_in_viewport) else {
            self.zoom_to_center(target, cx);
            return;
        };
        let local_anchor = point(
            anchor_in_viewport.x - old_pane.x,
            anchor_in_viewport.y - old_pane.y,
        );
        let old_pan = old_pane.clamp_pan(self.pan);
        let old_center_x = old_pane.base_center_x + old_pan.x;
        let old_center_y = old_pane.base_center_y + old_pan.y;
        let img_pt_x = (local_anchor.x - old_center_x) / old.scale;
        let img_pt_y = (local_anchor.y - old_center_y) / old.scale;

        let bounds = self.viewport.get();
        let vp_w = f32::from(bounds.size.width);
        let vp_h = f32::from(bounds.size.height);
        let next = self.compute_geometry(vp_w, vp_h, target);
        self.zoom = match target {
            ZoomMode::Fit => ZoomMode::Fit,
            ZoomMode::Custom(_) if (next.scale - next.fit_scale).abs() < 0.0005 => ZoomMode::Fit,
            ZoomMode::Custom(_) => ZoomMode::Custom(next.scale),
        };
        if matches!(self.zoom, ZoomMode::Fit) {
            self.pan = point(0., 0.);
        } else if let Some(next_pane) = next.panes[pane_idx] {
            let desired_center_x = local_anchor.x - img_pt_x * next.scale;
            let desired_center_y = local_anchor.y - img_pt_y * next.scale;
            self.pan = next.clamp_pan(point(
                desired_center_x - next_pane.base_center_x,
                desired_center_y - next_pane.base_center_y,
            ));
        }
        cx.notify();
    }

    pub(crate) fn step_zoom(&mut self, zoom_in: bool, cx: &mut Context<Self>) {
        let Some(geom) = self.current_geometry() else {
            return;
        };
        let current = geom.scale;
        let target = if zoom_in {
            let next = ZOOM_STEPS
                .iter()
                .copied()
                .find(|&step| step > current * 1.01)
                .unwrap_or(MAX_ZOOM)
                .min(MAX_ZOOM);
            if next <= current + 0.0005 {
                return;
            }
            ZoomMode::Custom(next)
        } else {
            if matches!(self.zoom, ZoomMode::Fit) || current <= geom.fit_scale * 1.01 {
                return;
            }
            match ZOOM_STEPS
                .iter()
                .copied()
                .rev()
                .find(|&step| step < current / 1.01 && step > geom.fit_scale * 1.01)
            {
                Some(prev) => ZoomMode::Custom(prev),
                None => ZoomMode::Fit,
            }
        };
        self.zoom_to_center(target, cx);
    }

    pub(crate) fn reset_fit(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.zoom, ZoomMode::Fit) || self.pan != point(0., 0.) {
            self.zoom = ZoomMode::Fit;
            self.pan = point(0., 0.);
            cx.notify();
        }
    }

    pub(crate) fn toggle_fit_100(&mut self, cx: &mut Context<Self>) {
        let Some(geom) = self.current_geometry() else {
            return;
        };
        if matches!(self.zoom, ZoomMode::Fit) && geom.fit_scale < 0.999 {
            self.zoom_to_center(ZoomMode::Custom(1.0), cx);
        } else {
            self.reset_fit(cx);
        }
    }

    pub(crate) fn apply_pinch(
        &mut self,
        delta: f32,
        window_position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(geom) = self.current_geometry() else {
            return;
        };
        let factor = (1.0 + delta).clamp(0.5, 2.0);
        let next = (geom.scale * factor).clamp(geom.fit_scale, MAX_ZOOM);
        let bounds = self.viewport.get();
        let anchor = point(
            f32::from(window_position.x - bounds.origin.x),
            f32::from(window_position.y - bounds.origin.y),
        );
        let target = if delta < 0. && (next - geom.fit_scale).abs() < 0.001 {
            ZoomMode::Fit
        } else {
            ZoomMode::Custom(next)
        };
        self.zoom_to_at(target, anchor, cx);
    }

    pub(crate) fn pan_by(&mut self, dx: f32, dy: f32, cx: &mut Context<Self>) -> bool {
        let Some(geom) = self.current_geometry() else {
            return false;
        };
        if geom.max_pan_x <= 0. && geom.max_pan_y <= 0. {
            return false;
        }
        let next = geom.clamp_pan(point(self.pan.x + dx, self.pan.y + dy));
        if (next.x - self.pan.x).abs() > 0.01 || (next.y - self.pan.y).abs() > 0.01 {
            self.pan = next;
            cx.notify();
        }
        true
    }

    fn render_stepper(&self, geom: Option<CanvasGeometry>, cx: &mut Context<Self>) -> AnyElement {
        let (scale, at_min, at_max) = match geom {
            Some(g) => (
                g.scale,
                matches!(self.zoom, ZoomMode::Fit) || g.scale <= g.fit_scale * 1.005,
                g.scale >= MAX_ZOOM - 0.005,
            ),
            None => (1.0, true, false),
        };
        let pct_label = format!("{}%", (scale * 100.0).round().max(1.0) as u32);
        let shadow = BoxShadow {
            color: hsla(0., 0., 0., 0.33),
            offset: point(px(0.), px(2.)),
            blur_radius: px(8.),
            spread_radius: px(0.),
            inset: false,
        };
        #[cfg(test)]
        let stepper_bounds = self.last_stepper_bounds.clone();

        div()
            .id("image-zoom-stepper")
            .when(cfg!(test), |el| {
                el.debug_selector(|| "IMAGE_ZOOM_STEPPER".into())
            })
            .absolute()
            .bottom(px(12.))
            .right(px(12.))
            .h(px(24.))
            .rounded(px(6.))
            .border_1()
            .border_color(theme::border_strong())
            .bg(theme::panel())
            .shadow(vec![shadow])
            .flex()
            .items_center()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .on_pinch(|_, _, cx| cx.stop_propagation())
            .when(cfg!(test), move |el| {
                #[cfg(test)]
                {
                    el.child(
                        canvas(
                            move |bounds, _, _| stepper_bounds.set(Some(bounds)),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    )
                }
                #[cfg(not(test))]
                {
                    el
                }
            })
            .child(
                div()
                    .id("image-zoom-out")
                    .when(cfg!(test), move |el| {
                        el.debug_selector(move || {
                            if at_min {
                                "IMAGE_ZOOM_OUT disabled".into()
                            } else {
                                "IMAGE_ZOOM_OUT".into()
                            }
                        })
                    })
                    .h_full()
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .when(at_min, |el| el.opacity(0.4).cursor_default())
                    .when(!at_min, |el| {
                        el.cursor_pointer()
                            .hover(|h| h.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                            .tooltip(|_, cx| {
                                runner_app::ui::tooltip::tooltip_view(
                                    crate::i18n::t("viewer.zoom_out"),
                                    cx,
                                )
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.step_zoom(false, cx);
                            }))
                    })
                    .child(
                        svg()
                            .size(px(11.))
                            .path("minus.svg")
                            .text_color(theme::muted()),
                    ),
            )
            .child(
                div()
                    .id("image-zoom-fit")
                    .when(cfg!(test), {
                        let label = pct_label.clone();
                        move |el| el.debug_selector(move || format!("IMAGE_ZOOM_FIT {label}"))
                    })
                    .h_full()
                    .px(px(4.))
                    .min_w(px(36.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(theme::text_meta())
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::text())
                    .cursor_pointer()
                    .hover(|h| h.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                    .tooltip(|_, cx| {
                        runner_app::ui::tooltip::tooltip_view(crate::i18n::t("viewer.zoom_fit"), cx)
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.reset_fit(cx);
                    }))
                    .child(pct_label),
            )
            .child(
                div()
                    .id("image-zoom-in")
                    .when(cfg!(test), move |el| {
                        el.debug_selector(move || {
                            if at_max {
                                "IMAGE_ZOOM_IN disabled".into()
                            } else {
                                "IMAGE_ZOOM_IN".into()
                            }
                        })
                    })
                    .h_full()
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .when(at_max, |el| el.opacity(0.4).cursor_default())
                    .when(!at_max, |el| {
                        el.cursor_pointer()
                            .hover(|h| h.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
                            .tooltip(|_, cx| {
                                runner_app::ui::tooltip::tooltip_view(
                                    crate::i18n::t("viewer.zoom_in"),
                                    cx,
                                )
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.step_zoom(true, cx);
                            }))
                    })
                    .child(
                        svg()
                            .size(px(11.))
                            .path("plus.svg")
                            .text_color(theme::muted()),
                    ),
            )
            .into_any_element()
    }
}

fn render_checkerboard(clip_bounds: Bounds<Pixels>, is_light: bool) -> AnyElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            let visible = if clip_bounds.size.width > px(0.) && clip_bounds.size.height > px(0.) {
                bounds.intersect(&clip_bounds)
            } else {
                bounds
            };
            if visible.size.width <= px(0.) || visible.size.height <= px(0.) {
                return;
            }
            let scale = window.scale_factor();
            let (tile_logical, tile) = checkerboard_tile(is_light, scale, window, cx);
            if tile_logical <= 0. {
                return;
            }
            let start_col = ((f32::from(visible.origin.x - bounds.origin.x)) / tile_logical)
                .floor()
                .max(0.) as i32;
            let end_col = ((f32::from(visible.right() - bounds.origin.x)) / tile_logical)
                .ceil()
                .max(0.) as i32;
            let start_row = ((f32::from(visible.origin.y - bounds.origin.y)) / tile_logical)
                .floor()
                .max(0.) as i32;
            let end_row = ((f32::from(visible.bottom() - bounds.origin.y)) / tile_logical)
                .ceil()
                .max(0.) as i32;
            let tile_px = px(tile_logical);
            for row in start_row..end_row {
                let y = bounds.origin.y + px(row as f32 * tile_logical);
                for col in start_col..end_col {
                    let x = bounds.origin.x + px(col as f32 * tile_logical);
                    let tile_bounds = Bounds::new(point(x, y), size(tile_px, tile_px));
                    let _ = window.paint_image(
                        visible,
                        tile_bounds,
                        Corners::default(),
                        tile.clone(),
                        0,
                        false,
                    );
                }
            }
        },
    )
    .absolute()
    .inset_0()
    .into_any_element()
}

fn render_diff_caption(side: &DiffSide, x: f32, y: f32, w: f32) -> AnyElement {
    let (sign, sign_color) = side.caption.sign();
    let label = crate::i18n::text(side.caption.label_key());
    let meta = side.state.meta_text();
    #[cfg(test)]
    let selector = side.caption.selector();
    div()
        .when(cfg!(test), move |el| {
            #[cfg(test)]
            {
                el.debug_selector(move || selector.into())
            }
            #[cfg(not(test))]
            {
                el
            }
        })
        .absolute()
        .left(px(x))
        .top(px(y))
        .w(px(w))
        .h(px(CAPTION_HEIGHT))
        .px(px(12.))
        .pt(px(6.))
        .flex()
        .items_center()
        .gap(px(6.))
        .overflow_hidden()
        .child(
            div()
                .flex_none()
                .font_family(theme::UI_MONOSPACE_FONT)
                .text_size(theme::text_meta())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(sign_color)
                .child(sign),
        )
        .child(
            div()
                .flex_none()
                .text_size(theme::text_meta())
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::text())
                .child(label),
        )
        .children(meta.map(|m| {
            div()
                .min_w(px(0.))
                .truncate()
                .text_size(theme::text_meta())
                .text_color(theme::muted())
                .child(m)
        }))
        .into_any_element()
}

fn render_diff_side_placeholder(side: &DiffSide) -> AnyElement {
    let (selector, icon, title, body) = match &side.state {
        DiffSideState::TooManyPixels {
            width,
            height,
            size,
        } => (
            "IMAGE_DIFF_SIDE_TOO_LARGE",
            "triangle-alert.svg",
            format!(
                "{} · {}",
                side.file_name,
                format_dimensions_and_size(*width, *height, *size)
            ),
            crate::i18n::text("viewer.image_too_many_pixels").to_owned(),
        ),
        DiffSideState::Failed => (
            "IMAGE_DIFF_SIDE_FAILED",
            "image-off.svg",
            crate::i18n::text("viewer.image_decode_failed_title").to_owned(),
            crate::i18n::tf(
                "viewer.image_decode_failed_body",
                &[("name", side.file_name.clone())],
            ),
        ),
        DiffSideState::Ready(_) => return div().into_any_element(),
    };
    div()
        .when(cfg!(test), move |el| {
            el.debug_selector(move || selector.into())
        })
        .size_full()
        .px(px(16.))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(4.))
        .text_center()
        .child(svg().size(px(22.)).path(icon).text_color(theme::faint()))
        .child(
            div()
                .pt(px(4.))
                .text_size(theme::text_ui())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text())
                .child(title),
        )
        .child(
            div()
                .text_size(theme::text_meta())
                .text_color(theme::muted())
                .child(body),
        )
        .into_any_element()
}

impl Render for ImageCanvas {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let geom = self.current_geometry();
        let is_light = theme::active_variant().is_light();
        let scale_factor = window.scale_factor();
        let viewport_cell = self.viewport.clone();
        let weak = cx.weak_entity();
        let measure = canvas(
            move |bounds, window, cx| {
                let prev = viewport_cell.get();
                if (f32::from(bounds.size.width) - f32::from(prev.size.width)).abs() > 0.5
                    || (f32::from(bounds.size.height) - f32::from(prev.size.height)).abs() > 0.5
                    || (f32::from(bounds.origin.x) - f32::from(prev.origin.x)).abs() > 0.5
                    || (f32::from(bounds.origin.y) - f32::from(prev.origin.y)).abs() > 0.5
                {
                    let size_changed =
                        (f32::from(bounds.size.width) - f32::from(prev.size.width)).abs() > 0.5
                            || (f32::from(bounds.size.height) - f32::from(prev.size.height)).abs()
                                > 0.5;
                    viewport_cell.set(bounds);
                    if size_changed && bounds.size.width > px(0.) && bounds.size.height > px(0.) {
                        let weak = weak.clone();
                        window.defer(cx, move |_, cx| {
                            let _ =
                                weak.update(cx, |this, cx| this.on_viewport_resized(bounds, cx));
                        });
                    }
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0();

        if let Some(g) = geom {
            for (slot, pane) in g.panes.into_iter().enumerate() {
                if let Some(p) = pane
                    && p.has_image
                    && let Some((parsed, desired)) = self.content.image_at(slot).and_then(|img| {
                        img.svg_reraster_request(p.drawn_w * scale_factor, p.drawn_h * scale_factor)
                    })
                {
                    self.schedule_svg_reraster(slot, parsed, desired, cx);
                }
            }
        }

        let mut layers: Vec<AnyElement> = Vec::new();
        if let Some(g) = geom {
            let vp = self.viewport.get();
            if let CanvasContent::Diff { sides, .. } = &self.content {
                for (slot, cap) in g.captions.into_iter().enumerate() {
                    if let (Some((cx_x, cx_y, cx_w)), Some(side)) = (cap, sides.slot(slot)) {
                        layers.push(render_diff_caption(side, cx_x, cx_y, cx_w));
                    }
                }
            }
            for (slot, pane) in g.panes.into_iter().enumerate() {
                let Some(p) = pane else {
                    #[cfg(test)]
                    self.last_image_bounds[slot].set(None);
                    continue;
                };
                let pane_clip = Bounds::new(
                    point(vp.origin.x + px(p.x), vp.origin.y + px(p.y)),
                    size(px(p.w), px(p.h)),
                );
                let pane_inner = if p.has_image
                    && let Some(decoded) = self.content.image_at(slot)
                {
                    let origin = p.image_origin(self.pan);
                    let level =
                        decoded.best_level(p.drawn_w * scale_factor, p.drawn_h * scale_factor);
                    #[cfg(test)]
                    {
                        self.last_image_bounds[slot].set(Some(Bounds::new(
                            point(
                                pane_clip.origin.x + px(origin.x),
                                pane_clip.origin.y + px(origin.y),
                            ),
                            size(px(p.drawn_w), px(p.drawn_h)),
                        )));
                        if slot == 0 {
                            let lsz = level.size(0);
                            self.last_level_size
                                .set(Some((lsz.width.0 as u32, lsz.height.0 as u32)));
                        }
                    }
                    let img_id =
                        SharedString::from(format!("image-canvas-render-{slot}-{}", level.id.0));
                    div()
                        .when(cfg!(test), move |el| {
                            el.debug_selector(move || {
                                if slot == 0 {
                                    "IMAGE_VIEW_IMG".into()
                                } else {
                                    "IMAGE_VIEW_IMG_AFTER".into()
                                }
                            })
                        })
                        .absolute()
                        .left(px(origin.x))
                        .top(px(origin.y))
                        .w(px(p.drawn_w))
                        .h(px(p.drawn_h))
                        .overflow_hidden()
                        .when(self.deleted, |el| el.opacity(0.5))
                        .child(render_checkerboard(pane_clip, is_light))
                        .child(
                            img(ImageSource::Render(level))
                                .id(img_id)
                                .size_full()
                                .object_fit(gpui::ObjectFit::Fill),
                        )
                        .into_any_element()
                } else {
                    #[cfg(test)]
                    self.last_image_bounds[slot].set(None);
                    match &self.content {
                        CanvasContent::Diff { sides, .. } => sides
                            .slot(slot)
                            .map(render_diff_side_placeholder)
                            .unwrap_or_else(|| div().into_any_element()),
                        CanvasContent::Single(_) => div().into_any_element(),
                    }
                };
                layers.push(
                    div()
                        .absolute()
                        .left(px(p.x))
                        .top(px(p.y))
                        .w(px(p.w))
                        .h(px(p.h))
                        .overflow_hidden()
                        .child(pane_inner)
                        .into_any_element(),
                );
            }
            if let Some(div_x) = g.divider_x {
                layers.push(
                    div()
                        .when(cfg!(test), |el| {
                            el.debug_selector(|| "IMAGE_DIFF_SPLIT_DIVIDER".into())
                        })
                        .absolute()
                        .left(px(div_x))
                        .top_0()
                        .bottom_0()
                        .w(px(1.))
                        .bg(theme::border())
                        .into_any_element(),
                );
            }
        }

        let show_stepper = geom.is_none_or(|g| g.has_image);
        let stepper = show_stepper.then(|| self.render_stepper(geom, cx));
        let pannable = geom.is_some_and(|g| g.max_pan_x > 0. || g.max_pan_y > 0.);
        let dragging = self.drag.is_some_and(|(_, _, moved)| moved);

        div()
            .id("image-canvas")
            .when(cfg!(test), |el| el.debug_selector(|| "IMAGE_CANVAS".into()))
            .track_focus(&self.focus)
            .relative()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .size_full()
            .overflow_hidden()
            .bg(theme::bg())
            .when(pannable && !dragging, |el| el.cursor_grab())
            .when(dragging, |el| el.cursor_grabbing())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.focus.focus(window, cx);
                    if let Some(on_in) = this.on_focus_in.clone() {
                        on_in(window, cx);
                    }
                    if event.click_count >= 2 {
                        this.drag = None;
                        this.toggle_fit_100(cx);
                    } else {
                        this.drag = Some((event.position, this.pan, false));
                    }
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if event.pressed_button != Some(MouseButton::Left) {
                    if this.drag.take().is_some() {
                        cx.notify();
                    }
                    return;
                }
                let Some((start_pos, start_pan, moved)) = this.drag else {
                    return;
                };
                let dx = f32::from(event.position.x - start_pos.x);
                let dy = f32::from(event.position.y - start_pos.y);
                if !moved && dx.abs() < DRAG_THRESHOLD_PX && dy.abs() < DRAG_THRESHOLD_PX {
                    return;
                }
                this.drag = Some((start_pos, start_pan, true));
                if let Some(geom) = this.current_geometry() {
                    let next = geom.clamp_pan(point(start_pan.x + dx, start_pan.y + dy));
                    this.pan = next;
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.drag.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.drag.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                let mut delta = event.delta.pixel_delta(px(24.));
                if event.modifiers.shift && delta.x == px(0.) {
                    delta.x = delta.y;
                    delta.y = px(0.);
                }
                if this.pan_by(f32::from(delta.x), f32::from(delta.y), cx) {
                    cx.stop_propagation();
                }
            }))
            .on_pinch(cx.listener(|this, event: &PinchEvent, _, cx| {
                cx.stop_propagation();
                this.apply_pinch(event.delta, event.position, cx);
            }))
            .child(measure)
            .children(layers)
            .children(stepper)
    }
}

#[cfg(test)]
impl ImageCanvas {
    pub(crate) fn image_dimensions(&self) -> (u32, u32) {
        self.content
            .image_at(0)
            .map_or((0, 0), |img| (img.width, img.height))
    }

    pub(crate) fn diff_layout(&self) -> Option<DiffViewMode> {
        match &self.content {
            CanvasContent::Diff { layout, .. } => Some(*layout),
            CanvasContent::Single(_) => None,
        }
    }

    pub(crate) fn effective_scale(&self) -> Option<f32> {
        self.current_geometry().map(|g| g.scale)
    }

    pub(crate) fn fit_scale(&self) -> Option<f32> {
        self.current_geometry().map(|g| g.fit_scale)
    }

    pub(crate) fn is_fit(&self) -> bool {
        matches!(self.zoom, ZoomMode::Fit)
    }

    pub(crate) fn pan(&self) -> Point<f32> {
        self.pan
    }

    pub(crate) fn max_pan(&self) -> Option<(f32, f32)> {
        self.current_geometry().map(|g| (g.max_pan_x, g.max_pan_y))
    }

    pub(crate) fn is_deleted(&self) -> bool {
        self.deleted
    }

    pub(crate) fn last_image_bounds(&self) -> Option<Bounds<Pixels>> {
        self.last_image_bounds[0].get()
    }

    pub(crate) fn last_after_image_bounds(&self) -> Option<Bounds<Pixels>> {
        self.last_image_bounds[1].get()
    }

    pub(crate) fn last_stepper_bounds(&self) -> Option<Bounds<Pixels>> {
        self.last_stepper_bounds.get()
    }

    pub(crate) fn last_level_size(&self) -> Option<(u32, u32)> {
        self.last_level_size.get()
    }

    pub(crate) fn render_images(&self) -> Vec<Arc<RenderImage>> {
        let mut out = Vec::new();
        for slot in 0..2 {
            if let Some(img) = self.content.image_at(slot) {
                out.extend(img.levels().iter().cloned());
            }
        }
        out
    }

    pub(crate) fn frame_count(&self) -> usize {
        self.content
            .image_at(0)
            .map_or(0, DecodedImage::frame_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::codecs::png::PngEncoder;
    use image::{Delay, ExtendedColorType, ImageEncoder};

    fn make_png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut buf = Vec::new();
        let pixels = vec![rgba[0], rgba[1], rgba[2], rgba[3]]
            .into_iter()
            .cycle()
            .take((w as usize) * (h as usize) * 4)
            .collect::<Vec<_>>();
        PngEncoder::new(&mut buf)
            .write_image(&pixels, w, h, ExtendedColorType::Rgba8)
            .unwrap();
        buf
    }

    fn append_png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let mut crc_input = Vec::with_capacity(4 + data.len());
        crc_input.extend_from_slice(kind);
        crc_input.extend_from_slice(data);
        out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    }

    /// Minimal PNG header (`IHDR` + stub `IDAT`) with arbitrary dimensions, for
    /// testing header-only limit rejection without allocating gigabytes.
    fn fake_png_header(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        append_png_chunk(&mut out, b"IHDR", &ihdr);
        append_png_chunk(&mut out, b"IDAT", &[0]);
        out
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &b in bytes {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if (crc & 1) != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    #[test]
    fn header_limits_reject_oversized_dimensions_without_decoding() {
        let over_side = fake_png_header(MAX_IMAGE_SIDE + 1, 100);
        assert!(matches!(
            decode_raster_image(gpui::ImageFormat::Png, &over_side, over_side.len() as u64),
            DecodeOutcome::TooManyPixels {
                width: 16_385,
                height: 100,
                ..
            }
        ));

        let over_mp = fake_png_header(9_000, 9_000);
        assert!(matches!(
            decode_raster_image(gpui::ImageFormat::Png, &over_mp, over_mp.len() as u64),
            DecodeOutcome::TooManyPixels {
                width: 9_000,
                height: 9_000,
                ..
            }
        ));
    }

    #[test]
    fn mip_chain_selects_smallest_level_covering_drawn_size_and_caps_texture_size() {
        let png = make_png(1024, 512, [10, 20, 30, 255]);
        let DecodeOutcome::Ready(decoded) =
            decode_raster_image(gpui::ImageFormat::Png, &png, png.len() as u64)
        else {
            panic!("expected ready");
        };
        assert_eq!((decoded.width, decoded.height), (1024, 512));
        let sizes: Vec<(i32, i32)> = decoded
            .levels()
            .iter()
            .map(|l| (l.size(0).width.0, l.size(0).height.0))
            .collect();
        assert_eq!(sizes, vec![(1024, 512), (512, 256), (256, 128), (128, 64)]);

        assert_eq!(
            decoded.best_level(200., 100.).size(0),
            size(256.into(), 128.into())
        );
        assert_eq!(
            decoded.best_level(500., 250.).size(0),
            size(512.into(), 256.into())
        );
        assert_eq!(
            decoded.best_level(2000., 1000.).size(0),
            size(1024.into(), 512.into())
        );

        // A wide 10000×100 image (1 MP, <= 16384) must never expose a texture > 8192 px.
        let wide = make_png(10_000, 100, [255, 0, 0, 255]);
        let DecodeOutcome::Ready(wide_decoded) =
            decode_raster_image(gpui::ImageFormat::Png, &wide, wide.len() as u64)
        else {
            panic!("expected ready");
        };
        assert_eq!((wide_decoded.width, wide_decoded.height), (10_000, 100));
        for level in wide_decoded.levels() {
            assert!(level.size(0).width.0 as u32 <= MAX_TEXTURE_SIDE);
            assert!(level.size(0).height.0 as u32 <= MAX_TEXTURE_SIDE);
        }
        assert_eq!(
            wide_decoded.levels()[0].size(0),
            size(5000.into(), 50.into())
        );
    }

    #[test]
    fn gif_within_limit_animates_and_over_64mp_falls_back_to_first_frame_still() {
        let mut small_gif = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut small_gif);
            enc.set_repeat(Repeat::Infinite).unwrap();
            let f1 = image::Frame::from_parts(
                RgbaImage::from_pixel(16, 16, Rgba([255, 0, 0, 255])),
                0,
                0,
                Delay::from_numer_denom_ms(100, 1),
            );
            let f2 = image::Frame::from_parts(
                RgbaImage::from_pixel(16, 16, Rgba([0, 255, 0, 255])),
                0,
                0,
                Delay::from_numer_denom_ms(100, 1),
            );
            enc.encode_frames([f1, f2]).unwrap();
        }
        let DecodeOutcome::Ready(small) =
            decode_raster_image(gpui::ImageFormat::Gif, &small_gif, small_gif.len() as u64)
        else {
            panic!("expected animated gif");
        };
        assert!(small.is_animated());
        assert_eq!(small.frame_count(), 2);

        // 2000 × 2000 = 4 MP per frame × 17 frames = 68 MP > 64 MP -> falls back to 1-frame still.
        let mut huge_gif = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut huge_gif);
            enc.set_repeat(Repeat::Infinite).unwrap();
            let frame_buf = RgbaImage::from_pixel(2000, 2000, Rgba([0, 0, 255, 255]));
            let frames = (0..17).map(|_| {
                image::Frame::from_parts(frame_buf.clone(), 0, 0, Delay::from_numer_denom_ms(50, 1))
            });
            enc.encode_frames(frames).unwrap();
        }
        let DecodeOutcome::Ready(still) =
            decode_raster_image(gpui::ImageFormat::Gif, &huge_gif, huge_gif.len() as u64)
        else {
            panic!("expected first-frame still");
        };
        assert!(!still.is_animated());
        assert_eq!(still.frame_count(), 1);
        assert_eq!((still.width, still.height), (2000, 2000));
    }

    #[test]
    fn single_geometry_fits_never_above_100_percent_and_clears_stepper() {
        let large = CanvasGeometry::compute_single(440., 346., 3368, 2196, ZoomMode::Fit);
        assert!(large.scale < 1.0);
        let pane = large.panes[0].unwrap();
        let origin = pane.image_origin(point(0., 0.));
        assert!(origin.x >= CANVAS_MARGIN - 1.);
        assert!(origin.y >= CANVAS_MARGIN - 1.);
        assert!(origin.y + pane.drawn_h <= 346. - STEPPER_CLEARANCE + 1.);

        let small = CanvasGeometry::compute_single(440., 186., 64, 64, ZoomMode::Fit);
        assert_eq!(small.scale, 1.0);
        let small_pane = small.panes[0].unwrap();
        assert_eq!(small_pane.drawn_w, 64.);
        assert_eq!(small_pane.drawn_h, 64.);
    }

    #[test]
    fn svg_target_device_size_scales_sharply_and_respects_texture_limit() {
        assert_eq!(svg_target_device_size(240, 120, 960., 480.), (960, 480));
        assert_eq!(svg_target_device_size(240, 120, 1920., 960.), (1920, 960));
        // Oversized target preserves aspect ratio while clamping to MAX_TEXTURE_SIDE.
        assert_eq!(
            svg_target_device_size(4000, 2000, 16_000., 8_000.),
            (MAX_TEXTURE_SIDE, MAX_TEXTURE_SIDE / 2)
        );
    }

    fn make_png_with_iccp(w: u32, h: u32, iccp_uncompressed_len: usize) -> Vec<u8> {
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write;

        let mut png = make_png(w, h, [10, 20, 30, 255]);
        // Insert a zlib-compressed iCCP chunk right after the 8-byte signature + 25-byte IHDR.
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::best());
        enc.write_all(&vec![0u8; iccp_uncompressed_len]).unwrap();
        let compressed = enc.finish().unwrap();
        let mut chunk_data = b"bomb\0\0".to_vec();
        chunk_data.extend_from_slice(&compressed);

        let mut crc_in = Vec::with_capacity(4 + chunk_data.len());
        crc_in.extend_from_slice(b"iCCP");
        crc_in.extend_from_slice(&chunk_data);
        let mut chunk = Vec::with_capacity(12 + chunk_data.len());
        chunk.extend_from_slice(&(chunk_data.len() as u32).to_be_bytes());
        chunk.extend_from_slice(b"iCCP");
        chunk.extend_from_slice(&chunk_data);
        chunk.extend_from_slice(&crc32(&crc_in).to_be_bytes());

        let tail = png.split_off(8 + 25);
        png.extend_from_slice(&chunk);
        png.extend_from_slice(&tail);
        png
    }

    #[test]
    fn png_header_bounds_iccp_decompression_budget() {
        let small = make_png_with_iccp(16, 16, 1024);
        let mut small_probe = image::ImageReader::with_format(Cursor::new(&small), ImgFmt::Png);
        small_probe.limits(header_limits());
        let mut small_dec = small_probe.into_decoder().unwrap();
        assert_eq!(
            small_dec.icc_profile().unwrap().map(|v| v.len()),
            Some(1024)
        );

        // An iCCP chunk expanding past MAX_HEADER_ALLOC_BYTES is stopped by the finite
        // header budget (`png` skips the oversized ancillary profile rather than allocating it).
        let bomb = make_png_with_iccp(16, 16, (MAX_HEADER_ALLOC_BYTES as usize) + 2 * 1024 * 1024);
        assert!(
            bomb.len() < 256 * 1024,
            "compressed bomb fixture should stay tiny on disk"
        );
        let mut bomb_probe = image::ImageReader::with_format(Cursor::new(&bomb), ImgFmt::Png);
        bomb_probe.limits(header_limits());
        let mut bomb_dec = bomb_probe.into_decoder().unwrap();
        assert!(
            bomb_dec.icc_profile().unwrap().is_none(),
            "oversized iCCP profile must not decompress past MAX_HEADER_ALLOC_BYTES"
        );
        // The image itself still decodes safely within the finite budget.
        assert!(matches!(
            decode_raster_image(gpui::ImageFormat::Png, &bomb, bomb.len() as u64),
            DecodeOutcome::Ready(_)
        ));
    }

    #[test]
    fn truncated_gif_fails_cleanly_without_looping() {
        let mut two_frame_gif = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut two_frame_gif);
            enc.set_repeat(Repeat::Infinite).unwrap();
            let f1 = image::Frame::from_parts(
                RgbaImage::from_pixel(16, 16, Rgba([255, 0, 0, 255])),
                0,
                0,
                Delay::from_numer_denom_ms(100, 1),
            );
            let f2 = image::Frame::from_parts(
                RgbaImage::from_pixel(16, 16, Rgba([0, 255, 0, 255])),
                0,
                0,
                Delay::from_numer_denom_ms(100, 1),
            );
            enc.encode_frames([f1, f2]).unwrap();
        }
        // Truncate mid-way through frame 2 so frame 1 decodes and frame 2 errors.
        let truncated_mid_second = &two_frame_gif[..two_frame_gif.len() - 6];
        assert!(matches!(
            decode_raster_image(
                gpui::ImageFormat::Gif,
                truncated_mid_second,
                truncated_mid_second.len() as u64
            ),
            DecodeOutcome::Failed
        ));

        // Truncate inside the first frame as well.
        let truncated_first = &two_frame_gif[..24.min(two_frame_gif.len())];
        assert!(matches!(
            decode_raster_image(
                gpui::ImageFormat::Gif,
                truncated_first,
                truncated_first.len() as u64
            ),
            DecodeOutcome::Failed
        ));
    }

    #[test]
    fn svg_decode_enforces_limits_keeps_logical_size_and_blocks_local_files() {
        use base64::Engine;

        let valid = br##"<svg xmlns="http://www.w3.org/2000/svg" width="240" height="120"><rect width="240" height="120" fill="#ff0000"/></svg>"##;
        let DecodeOutcome::Ready(decoded) = decode_svg_image(valid, valid.len() as u64) else {
            panic!("valid SVG should decode");
        };
        assert_eq!((decoded.width, decoded.height), (240, 120));
        assert_eq!(decoded.size, valid.len() as u64);
        assert!(decoded.svg_reraster_request(240., 120.).is_none());
        let (_, desired) = decoded
            .svg_reraster_request(960., 480.)
            .expect("400% zoom requests sharper raster");
        assert_eq!(desired, (960, 480));

        assert!(matches!(
            decode_svg_image(b"<svg><unclosed", 14),
            DecodeOutcome::Failed
        ));

        // Legal SVG over MAX_TEXTURE_SIDE (10000 x 100 = 1 MP <= 64 MP): preserves logical
        // dimensions while keeping the rasterized texture <= 8192 px.
        let wide_svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10000" height="100"><rect width="10000" height="100" fill="#38bdf8"/></svg>"##;
        let DecodeOutcome::Ready(wide) = decode_svg_image(wide_svg, wide_svg.len() as u64) else {
            panic!("10000x100 SVG should be accepted");
        };
        assert_eq!((wide.width, wide.height), (10_000, 100));
        let tex_sz = wide.levels()[0].size(0);
        assert!(tex_sz.width.0 as u32 <= MAX_TEXTURE_SIDE);
        assert!(tex_sz.height.0 as u32 <= MAX_TEXTURE_SIDE);

        // Over-16384 and over-64MP SVGs are rejected before pixel allocation.
        let over_side = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20000" height="100"><rect width="20000" height="100" fill="#f00"/></svg>"##;
        assert!(matches!(
            decode_svg_image(over_side, over_side.len() as u64),
            DecodeOutcome::TooManyPixels {
                width: 20_000,
                height: 100,
                ..
            }
        ));
        let over_mp = br##"<svg xmlns="http://www.w3.org/2000/svg" width="9000" height="9000"><rect width="9000" height="9000" fill="#f00"/></svg>"##;
        assert!(matches!(
            decode_svg_image(over_mp, over_mp.len() as u64),
            DecodeOutcome::TooManyPixels {
                width: 9_000,
                height: 9_000,
                ..
            }
        ));

        // Local filesystem `<image href="...">` must never be read.
        let temp = tempfile::tempdir().unwrap();
        let secret_path = temp.path().join("secret.png");
        std::fs::write(&secret_path, make_png(32, 32, [255, 0, 0, 255])).unwrap();
        let file_ref_svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><image href="{}" width="32" height="32"/></svg>"#,
            secret_path.display()
        );
        let DecodeOutcome::Ready(safe) =
            decode_svg_image(file_ref_svg.as_bytes(), file_ref_svg.len() as u64)
        else {
            panic!("SVG with blocked local href should still parse safely");
        };
        let bytes = safe.levels()[0].as_bytes(0).unwrap();
        assert!(
            bytes.as_chunks::<4>().0.iter().all(|px| px[3] == 0),
            "local filesystem image must never be loaded into SVG raster"
        );

        // Embedded oversized/bomb data-URI PNG inside SVG is dropped by the bounded data resolver.
        let oversized_b64 =
            base64::engine::general_purpose::STANDARD.encode(fake_png_header(16_385, 100));
        let iccp_bomb_b64 = base64::engine::general_purpose::STANDARD.encode(make_png_with_iccp(
            16,
            16,
            (MAX_HEADER_ALLOC_BYTES as usize) + 2 * 1024 * 1024,
        ));
        let embedded_svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><image href="data:image/png;base64,{oversized_b64}" width="32" height="32"/><image href="data:image/png;base64,{iccp_bomb_b64}" width="32" height="32"/></svg>"#
        );
        let DecodeOutcome::Ready(bounded) =
            decode_svg_image(embedded_svg.as_bytes(), embedded_svg.len() as u64)
        else {
            panic!("SVG with rejected embedded data-URI rasters should stay bounded");
        };
        assert!(bounded.levels()[0]
            .as_bytes(0)
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .all(|px| px[3] == 0));

        // Gzip-compressed SVGZ must be rejected directly and when embedded via data URI
        // so usvg never performs unbounded gzip expansion.
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;
        let mut gz = GzEncoder::new(Vec::new(), Compression::best());
        gz.write_all(valid).unwrap();
        let svgz = gz.finish().unwrap();
        assert!(matches!(
            decode_svg_image(&svgz, svgz.len() as u64),
            DecodeOutcome::Failed
        ));
        let svgz_b64 = base64::engine::general_purpose::STANDARD.encode(&svgz);
        let embedded_svgz = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><image href="data:image/svg+xml;base64,{svgz_b64}" width="32" height="32"/></svg>"#
        );
        let DecodeOutcome::Ready(no_svgz) =
            decode_svg_image(embedded_svgz.as_bytes(), embedded_svgz.len() as u64)
        else {
            panic!("outer SVG should ignore embedded SVGZ without decompressing it");
        };
        assert!(no_svgz.levels()[0]
            .as_bytes(0)
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .all(|px| px[3] == 0));
    }

    #[test]
    fn optimized_large_gif_with_partial_subframe_decodes_within_finite_budget() {
        // 3000x2000 logical screen + 2900x1900 partial subframe (total 12 MP <= 64 MP).
        // Needs two screen RGBA buffers plus the partial subframe (~70 MB > 8 B/px budget).
        let mut gif_bytes = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut gif_bytes);
            enc.set_repeat(Repeat::Infinite).unwrap();
            let f1 = image::Frame::from_parts(
                RgbaImage::from_pixel(3000, 2000, Rgba([20, 40, 80, 255])),
                0,
                0,
                Delay::from_numer_denom_ms(100, 1),
            );
            let f2 = image::Frame::from_parts(
                RgbaImage::from_pixel(2900, 1900, Rgba([200, 60, 40, 255])),
                50,
                50,
                Delay::from_numer_denom_ms(100, 1),
            );
            enc.encode_frames([f1, f2]).unwrap();
        }
        let DecodeOutcome::Ready(decoded) =
            decode_raster_image(gpui::ImageFormat::Gif, &gif_bytes, gif_bytes.len() as u64)
        else {
            panic!("valid 3000x2000 optimized GIF with partial subframe should decode");
        };
        assert!(decoded.is_animated());
        assert_eq!(decoded.frame_count(), 2);
        assert_eq!((decoded.width, decoded.height), (3000, 2000));
    }

    #[test]
    fn animated_gif_budget_covers_64mp_partial_frame_and_stops_after_first_frame_over_half() {
        // Budget arithmetic near 64 MP: 8000x7500 (60 MP) with a 6000x6000 partial subframe
        // requires 2 logical screens (480 MB) + partial subframe (144 MB) = 624 MB.
        let near_cap = animated_decode_limits(8000, 7500).max_alloc.unwrap();
        let required_near_cap = 2 * (8000u64 * 7500 * 4) + (6000u64 * 6000 * 4);
        assert!(
            near_cap > required_near_cap && near_cap <= MAX_ANIMATED_DECODE_ALLOC_BYTES,
            "near-64MP budget {near_cap} must cover {required_near_cap} and stay bounded"
        );
        let at_cap = animated_decode_limits(8000, 8000).max_alloc.unwrap();
        assert!(at_cap >= 3 * (8000u64 * 8000 * 4));
        assert_eq!(at_cap, MAX_ANIMATED_DECODE_ALLOC_BYTES);

        // Parameterized iterator check: a 16x16 (256 px) 2-frame GIF whose second frame is
        // truncated stops right after frame 1 when max_total_pixels = 300 (256 > 150),
        // proving frame 2 is never pulled; at budget = 600 it pulls frame 2 and fails.
        let mut two_frame_gif = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut two_frame_gif);
            enc.set_repeat(Repeat::Infinite).unwrap();
            let f1 = image::Frame::from_parts(
                RgbaImage::from_pixel(16, 16, Rgba([255, 0, 0, 255])),
                0,
                0,
                Delay::from_numer_denom_ms(100, 1),
            );
            let f2 = image::Frame::from_parts(
                RgbaImage::from_pixel(16, 16, Rgba([0, 255, 0, 255])),
                0,
                0,
                Delay::from_numer_denom_ms(100, 1),
            );
            enc.encode_frames([f1, f2]).unwrap();
        }
        let truncated_f2 = &two_frame_gif[..two_frame_gif.len() - 6];

        let dec_stop = GifDecoder::new(Cursor::new(truncated_f2)).unwrap();
        let DecodeOutcome::Ready(still) = decode_animated_frames_with_budget(
            dec_stop.into_frames(),
            16,
            16,
            truncated_f2.len() as u64,
            300,
        ) else {
            panic!("logical screen > half budget should stop after valid frame 1");
        };
        assert!(!still.is_animated());
        assert_eq!(still.frame_count(), 1);

        let dec_pull = GifDecoder::new(Cursor::new(truncated_f2)).unwrap();
        assert!(matches!(
            decode_animated_frames_with_budget(
                dec_pull.into_frames(),
                16,
                16,
                truncated_f2.len() as u64,
                600,
            ),
            DecodeOutcome::Failed
        ));
    }

    #[gpui::test]
    fn center_zoom_preserves_pan_on_overflowing_side_of_unequal_split_diff(
        cx: &mut gpui::TestAppContext,
    ) {
        let small_png = make_png(100, 100, [255, 0, 0, 255]);
        let large_png = make_png(800, 400, [0, 255, 0, 255]);
        let DecodeOutcome::Ready(before_img) =
            decode_raster_image(gpui::ImageFormat::Png, &small_png, small_png.len() as u64)
        else {
            panic!("before_img");
        };
        let DecodeOutcome::Ready(after_img) =
            decode_raster_image(gpui::ImageFormat::Png, &large_png, large_png.len() as u64)
        else {
            panic!("after_img");
        };
        let sides = DiffSides::Two {
            before: Box::new(DiffSide {
                caption: DiffCaptionKind::Before,
                file_name: "shot.png".into(),
                state: DiffSideState::Ready(before_img),
            }),
            after: Box::new(DiffSide {
                caption: DiffCaptionKind::After,
                file_name: "shot.png".into(),
                state: DiffSideState::Ready(after_img),
            }),
        };
        let canvas = cx.new(|cx| ImageCanvas::new_diff(DiffViewMode::Split, sides, cx));
        canvas.update(cx, |c, cx| {
            c.on_viewport_resized(
                Bounds::new(point(px(0.), px(0.)), size(px(800.), px(600.))),
                cx,
            );
            // Zoom to 100%: Before (100x100) still fits in its ~400 px pane (max_pan_x == 0),
            // while After (800x400) overflows horizontally (max_pan_x ~= 200).
            c.zoom_to_center(ZoomMode::Custom(1.0), cx);
            assert!(c.pan_by(-80., 0., cx));
        });

        let old_after_img_x = canvas.read_with(cx, |c, _| {
            let geom = c.current_geometry().unwrap();
            let p0 = geom.panes[0].unwrap();
            let p1 = geom.panes[1].unwrap();
            assert_eq!(p0.max_pan_x, 0., "Before side fits horizontally");
            assert!(p1.max_pan_x > 100., "After side overflows horizontally");
            let pan1 = p1.clamp_pan(c.pan);
            assert!((pan1.x + 80.).abs() < 0.5);
            (p1.w * 0.5 - (p1.base_center_x + pan1.x)) / geom.scale
        });

        canvas.update(cx, |c, cx| {
            c.step_zoom(true, cx);
        });
        let (new_after_img_x, new_pan_x) = canvas.read_with(cx, |c, _| {
            let geom = c.current_geometry().unwrap();
            let p1 = geom.panes[1].unwrap();
            let pan1 = p1.clamp_pan(c.pan);
            (
                (p1.w * 0.5 - (p1.base_center_x + pan1.x)) / geom.scale,
                pan1.x,
            )
        });
        assert!(
            new_pan_x < -80.,
            "After pan must grow with zoom rather than resetting to 0 (got {new_pan_x})"
        );
        assert!(
            (new_after_img_x - old_after_img_x).abs() < 0.5,
            "After center shifted from image x={old_after_img_x} to x={new_after_img_x}"
        );
    }

    #[gpui::test]
    fn center_zoom_preserves_image_point_across_stepper_clearance_shift(
        cx: &mut gpui::TestAppContext,
    ) {
        let png = make_png(1000, 1000, [40, 120, 220, 255]);
        let DecodeOutcome::Ready(decoded) =
            decode_raster_image(gpui::ImageFormat::Png, &png, png.len() as u64)
        else {
            panic!("expected decoded image");
        };
        let canvas = cx.new(|cx| ImageCanvas::new_single(decoded, cx));
        canvas.update(cx, |c, cx| {
            c.on_viewport_resized(
                Bounds::new(point(px(0.), px(0.)), size(px(500.), px(500.))),
                cx,
            );
        });

        // At Fit in a 500x500 viewport, base_center_y is shifted up to clear the stepper (234 != 250).
        let (old_img_x, old_img_y) = canvas.read_with(cx, |c, _| {
            let geom = c.current_geometry().unwrap();
            let pane = geom.panes[0].unwrap();
            assert!(
                (pane.base_center_y - pane.h * 0.5).abs() > 1.0,
                "fitted square should sit above raw viewport center to clear stepper"
            );
            let old_pan = pane.clamp_pan(c.pan);
            (
                (pane.w * 0.5 - (pane.base_center_x + old_pan.x)) / geom.scale,
                (pane.h * 0.5 - (pane.base_center_y + old_pan.y)) / geom.scale,
            )
        });

        // Step directly to 75% (750x750 > 500x500 viewport, where base_center_y becomes 250
        // and max_pan_y = 125 can fully compensate without clamping).
        canvas.update(cx, |c, cx| {
            c.zoom_to_center(ZoomMode::Custom(0.75), cx);
        });
        let (new_img_x, new_img_y, pan_y, max_pan_y) = canvas.read_with(cx, |c, _| {
            let geom = c.current_geometry().unwrap();
            let pane = geom.panes[0].unwrap();
            assert!((pane.base_center_y - pane.h * 0.5).abs() < 0.1);
            let new_pan = pane.clamp_pan(c.pan);
            (
                (pane.w * 0.5 - (pane.base_center_x + new_pan.x)) / geom.scale,
                (pane.h * 0.5 - (pane.base_center_y + new_pan.y)) / geom.scale,
                new_pan.y,
                pane.max_pan_y,
            )
        });
        assert!(
            pan_y.abs() < max_pan_y - 1.0,
            "zoom stayed within pan bounds"
        );
        assert!((new_img_x - old_img_x).abs() < 0.5);
        assert!(
            (new_img_y - old_img_y).abs() < 0.5,
            "pane center shifted from image y={old_img_y} to y={new_img_y}"
        );
    }

    #[test]
    fn diff_geometry_uses_one_shared_scale_for_unified_split_and_one_sided() {
        let png_a = make_png(400, 200, [255, 0, 0, 255]);
        let png_b = make_png(800, 600, [0, 255, 0, 255]);
        let DecodeOutcome::Ready(img_a) =
            decode_raster_image(gpui::ImageFormat::Png, &png_a, png_a.len() as u64)
        else {
            panic!("img_a");
        };
        let DecodeOutcome::Ready(img_b) =
            decode_raster_image(gpui::ImageFormat::Png, &png_b, png_b.len() as u64)
        else {
            panic!("img_b");
        };
        let sides = DiffSides::Two {
            before: Box::new(DiffSide {
                caption: DiffCaptionKind::Before,
                file_name: "shot.png".into(),
                state: DiffSideState::Ready(img_a),
            }),
            after: Box::new(DiffSide {
                caption: DiffCaptionKind::After,
                file_name: "shot.png".into(),
                state: DiffSideState::Ready(img_b),
            }),
        };

        let unified =
            CanvasGeometry::compute_diff(460., 600., DiffViewMode::Unified, &sides, ZoomMode::Fit);
        assert!(unified.divider_x.is_none());
        let p0 = unified.panes[0].unwrap();
        let p1 = unified.panes[1].unwrap();
        assert!((p0.drawn_w / 400. - unified.scale).abs() < 0.001);
        assert!((p1.drawn_w / 800. - unified.scale).abs() < 0.001);
        assert!(p0.y + p0.h < p1.y);

        let split =
            CanvasGeometry::compute_diff(900., 600., DiffViewMode::Split, &sides, ZoomMode::Fit);
        assert!(split.divider_x.is_some());
        let sp0 = split.panes[0].unwrap();
        let sp1 = split.panes[1].unwrap();
        assert!((sp0.drawn_w / 400. - split.scale).abs() < 0.001);
        assert!((sp1.drawn_w / 800. - split.scale).abs() < 0.001);
        assert!(sp0.x + sp0.w < sp1.x);

        let DecodeOutcome::Ready(img_one) =
            decode_raster_image(gpui::ImageFormat::Png, &png_a, png_a.len() as u64)
        else {
            panic!("img_one");
        };
        let one_sided = DiffSides::One(Box::new(DiffSide {
            caption: DiffCaptionKind::Added,
            file_name: "added.png".into(),
            state: DiffSideState::Ready(img_one),
        }));
        let one_geom = CanvasGeometry::compute_diff(
            460.,
            600.,
            DiffViewMode::Split,
            &one_sided,
            ZoomMode::Fit,
        );
        assert!(one_geom.divider_x.is_none());
        assert!(one_geom.panes[0].is_some());
        assert!(one_geom.panes[1].is_none());
    }
}
