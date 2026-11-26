//! Kitty graphics texture cache, background decoding, and paint-band layout.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures::channel::mpsc;
use futures::StreamExt as _;
use gpui::{
    point, px, size, App, AppContext as _, Bounds, ContentMask, Corners, Entity, Global, Pixels,
    RenderImage, Task, Window,
};
use runner_app::terminal_resize::device_cell_size;
use runner_terminal::kitty::{
    CellSize, PlacementId, StoredImage, TerminalGraphics, PER_SCREEN_LIMIT,
};
use runner_terminal::terminal::{MirrorGraphicsObserver, TerminalMirror};

use super::element::TerminalInteraction;

/// Threshold separating images painted behind cell backgrounds (`z < Z_BELOW_CELL_BG`)
/// from images painted between cell backgrounds and text (`Z_BELOW_CELL_BG <= z < 0`).
pub(crate) const Z_BELOW_CELL_BG: i32 = -1_073_741_824;

pub(crate) type TextureKey = (u32, u64);

#[derive(Clone)]
pub(crate) struct PaintedImage {
    pub(crate) z: i32,
    pub(crate) image_id: u32,
    pub(crate) placement_order: (u8, u32),
    pub(crate) dest_bounds: Bounds<Pixels>,
    pub(crate) image_bounds: Bounds<Pixels>,
    pub(crate) image: Arc<RenderImage>,
}

#[derive(Default)]
struct MirrorCache {
    last_sync_seq: u64,
    textures: HashMap<TextureKey, Arc<RenderImage>>,
    live_keys: HashSet<TextureKey>,
    in_flight: HashSet<TextureKey>,
    failed: HashSet<TextureKey>,
}

impl MirrorCache {
    fn drain_textures(&mut self) -> Vec<Arc<RenderImage>> {
        self.in_flight.clear();
        self.failed.clear();
        self.textures.drain().map(|(_, image)| image).collect()
    }
}

pub(crate) struct TextureStore {
    mirrors: Mutex<HashMap<u64, MirrorCache>>,
    release_tx: mpsc::UnboundedSender<Vec<Arc<RenderImage>>>,
}

impl TextureStore {
    fn new(release_tx: mpsc::UnboundedSender<Vec<Arc<RenderImage>>>) -> Self {
        Self {
            mirrors: Mutex::default(),
            release_tx,
        }
    }

    fn enqueue_release(&self, images: Vec<Arc<RenderImage>>) {
        if !images.is_empty() {
            let _ = self.release_tx.unbounded_send(images);
        }
    }

    /// Computes the three sorted paint bands for the active screen and returns
    /// any visible images that are not yet decoded in the cache.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_paint_bands(
        &self,
        mirror_id: u64,
        is_alt: bool,
        graphics: &TerminalGraphics,
        display_offset: usize,
        pane_bounds: Bounds<Pixels>,
        cell_width: Pixels,
        line_height: Pixels,
        scale_factor: f32,
        session_cell: CellSize,
    ) -> ([Vec<PaintedImage>; 3], Vec<StoredImage>) {
        let storage = graphics.active(is_alt);
        if storage.placements().is_empty() {
            return (Default::default(), Vec::new());
        }

        let viewer_cw = f32::from(cell_width);
        let viewer_ch = f32::from(line_height);
        if !(viewer_cw > 0.0 && viewer_ch > 0.0 && viewer_cw.is_finite() && viewer_ch.is_finite()) {
            return (Default::default(), Vec::new());
        }
        let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        let (effective_cell, sx, sy) = if session_cell.width > 0 && session_cell.height > 0 {
            (
                session_cell,
                viewer_cw / f32::from(session_cell.width),
                viewer_ch / f32::from(session_cell.height),
            )
        } else {
            let (dev_w, dev_h) = device_cell_size(viewer_cw, viewer_ch, scale);
            (
                CellSize {
                    width: dev_w,
                    height: dev_h,
                },
                1.0 / scale,
                1.0 / scale,
            )
        };

        let pane_left = f32::from(pane_bounds.left());
        let pane_top = f32::from(pane_bounds.top());
        let pane_right = f32::from(pane_bounds.right());
        let pane_bottom = f32::from(pane_bounds.bottom());

        let mut bands: [Vec<PaintedImage>; 3] = Default::default();
        let mut missing: Vec<StoredImage> = Vec::new();
        let mut missing_keys: HashSet<TextureKey> = HashSet::new();

        let mirrors = self.mirrors.lock().unwrap();
        let entry = mirrors.get(&mirror_id);

        for (&key, placement) in storage.placements() {
            let Some(pin) = storage.resolved_signed_anchor(placement) else {
                continue;
            };
            let Some(img) = storage.image_by_id(key.image_id) else {
                continue;
            };
            let src = placement.source_rect(img.width, img.height);
            if src.width == 0 || src.height == 0 {
                continue;
            }
            let (ox, oy) = placement.cell_offset(effective_cell);
            let (pw, ph) = placement.pixel_size(img.width, img.height, effective_cell);
            if pw == 0 || ph == 0 {
                continue;
            }
            let viewport_row = pin.line.saturating_add(display_offset as i64);
            let dest_x = pane_left + (pin.column as f32) * viewer_cw + (ox as f32) * sx;
            let dest_y = pane_top + (viewport_row as f32) * viewer_ch + (oy as f32) * sy;
            let dest_w = (pw as f32) * sx;
            let dest_h = (ph as f32) * sy;
            if !(dest_w > 0.0 && dest_h > 0.0 && dest_x.is_finite() && dest_y.is_finite()) {
                continue;
            }
            if dest_x + dest_w <= pane_left
                || dest_x >= pane_right
                || dest_y + dest_h <= pane_top
                || dest_y >= pane_bottom
            {
                continue;
            }

            let tex_key = (img.id, img.generation);
            let cached = entry.and_then(|cache| cache.textures.get(&tex_key).cloned());
            let Some(render_image) = cached else {
                let already_pending = entry.is_some_and(|cache| {
                    cache.in_flight.contains(&tex_key) || cache.failed.contains(&tex_key)
                });
                if !already_pending && missing_keys.insert(tex_key) {
                    missing.push(img.clone());
                }
                continue;
            };

            let dest_bounds =
                Bounds::new(point(px(dest_x), px(dest_y)), size(px(dest_w), px(dest_h)));
            let image_bounds =
                if src.x == 0 && src.y == 0 && src.width == img.width && src.height == img.height {
                    dest_bounds
                } else {
                    let scale_x = dest_w / (src.width as f32);
                    let scale_y = dest_h / (src.height as f32);
                    Bounds::new(
                        point(
                            px(dest_x - (src.x as f32) * scale_x),
                            px(dest_y - (src.y as f32) * scale_y),
                        ),
                        size(
                            px((img.width as f32) * scale_x),
                            px((img.height as f32) * scale_y),
                        ),
                    )
                };

            let band_index = band_for_z(placement.z);
            let placement_order = match key.placement_id {
                PlacementId::Internal(id) => (0u8, id),
                PlacementId::External(id) => (1u8, id),
            };
            bands[band_index].push(PaintedImage {
                z: placement.z,
                image_id: key.image_id,
                placement_order,
                dest_bounds,
                image_bounds,
                image: render_image,
            });
        }
        drop(mirrors);

        for band in &mut bands {
            band.sort_by_key(|item| (item.z, item.image_id, item.placement_order));
        }

        (bands, missing)
    }

    /// Decodes visible images missing from the cache (for instance when a
    /// previously hidden session becomes visible) on a GPUI background task,
    /// then notifies the terminal so it repaints once textures are ready.
    pub(crate) fn schedule_decode(
        self: &Arc<Self>,
        mirror: &Arc<TerminalMirror>,
        interaction: &Entity<TerminalInteraction>,
        missing: Vec<StoredImage>,
        cx: &mut App,
    ) {
        if missing.is_empty() || mirror.viewer_count() == 0 {
            return;
        }
        let mirror_id = mirror.mirror_id();
        let to_decode: Vec<StoredImage> = {
            let mut mirrors = self.mirrors.lock().unwrap();
            let entry = mirrors.entry(mirror_id).or_default();
            let mut selected = Vec::new();
            for img in missing {
                let key = (img.id, img.generation);
                if entry.textures.contains_key(&key)
                    || entry.in_flight.contains(&key)
                    || entry.failed.contains(&key)
                {
                    continue;
                }
                entry.live_keys.insert(key);
                entry.in_flight.insert(key);
                selected.push(img);
            }
            selected
        };
        if to_decode.is_empty() {
            return;
        }

        let store = Arc::clone(self);
        let weak_mirror = Arc::downgrade(mirror);
        let weak_interaction = interaction.downgrade();
        let work = cx.background_spawn(async move {
            to_decode
                .into_iter()
                .map(|img| {
                    let key = (img.id, img.generation);
                    let decoded = decode_kitty_render_image(&img);
                    (key, decoded)
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |cx| {
            let decoded_batch = work.await;
            // Leave mirror teardown eviction to `Drop for TerminalMirror` ->
            // `on_mirror_dropped` so painted textures are never dropped before
            // passing through `enqueue_release` / `cx.drop_image`.
            let Some(mirror) = weak_mirror.upgrade() else {
                return;
            };
            // Check live keys before locking `store.mirrors` so lock ordering
            // stays strictly `term -> graphics -> store.mirrors`.
            let mirror_live: HashSet<TextureKey> = mirror.live_image_keys().into_iter().collect();
            let mut inserted_any = false;
            {
                let mut mirrors = store.mirrors.lock().unwrap();
                if let Some(entry) = mirrors.get_mut(&mirror_id) {
                    let has_viewers = mirror.viewer_count() > 0;
                    for (key, decoded) in decoded_batch {
                        entry.in_flight.remove(&key);
                        if !mirror_live.contains(&key) {
                            entry.live_keys.remove(&key);
                            continue;
                        }
                        if !has_viewers || !entry.live_keys.contains(&key) {
                            continue;
                        }
                        match decoded {
                            Some(render_image) => {
                                entry.textures.entry(key).or_insert(render_image);
                                inserted_any = true;
                            }
                            None => {
                                entry.failed.insert(key);
                            }
                        }
                    }
                }
            }
            // Respect the mirror's DECSET 2026 synchronized-update wake guard:
            // notifying `TerminalInteraction` wakes `TerminalViewport`, so only
            // notify when `wake_viewers_now()` actually permits a wake.
            if inserted_any && mirror.wake_viewers_now() {
                let _ = weak_interaction.update(cx, |_, cx| cx.notify());
            }
        })
        .detach();
    }

    #[cfg(test)]
    pub(crate) fn cached_image(&self, mirror_id: u64, key: TextureKey) -> Option<Arc<RenderImage>> {
        self.mirrors
            .lock()
            .unwrap()
            .get(&mirror_id)
            .and_then(|entry| entry.textures.get(&key).cloned())
    }

    #[cfg(test)]
    pub(crate) fn cached_count(&self, mirror_id: u64) -> usize {
        self.mirrors
            .lock()
            .unwrap()
            .get(&mirror_id)
            .map_or(0, |entry| entry.textures.len())
    }

    #[cfg(test)]
    pub(crate) fn live_keys_count(&self, mirror_id: u64) -> usize {
        self.mirrors
            .lock()
            .unwrap()
            .get(&mirror_id)
            .map_or(0, |entry| entry.live_keys.len())
    }
}

impl MirrorGraphicsObserver for TextureStore {
    fn on_graphics_updated(
        &self,
        mirror_id: u64,
        sync_seq: u64,
        viewers: &Arc<AtomicUsize>,
        live_keys: &[(u32, u64)],
        decode_candidates: &[StoredImage],
    ) {
        let live_set: HashSet<TextureKey> = live_keys.iter().copied().collect();
        let (to_release, to_decode) = {
            let mut mirrors = self.mirrors.lock().unwrap();
            let entry = mirrors.entry(mirror_id).or_default();
            if sync_seq <= entry.last_sync_seq {
                return;
            }
            entry.last_sync_seq = sync_seq;
            if viewers.load(Ordering::Acquire) == 0 || live_set.is_empty() {
                entry.live_keys.clear();
                (entry.drain_textures(), Vec::new())
            } else {
                let mut released = Vec::new();
                entry.textures.retain(|key, image| {
                    if live_set.contains(key) {
                        true
                    } else {
                        released.push(Arc::clone(image));
                        false
                    }
                });
                entry.in_flight.retain(|key| live_set.contains(key));
                entry.failed.retain(|key| live_set.contains(key));
                entry.live_keys = live_set;
                let mut missing = Vec::new();
                for img in decode_candidates {
                    let key = (img.id, img.generation);
                    if entry.live_keys.contains(&key)
                        && !entry.textures.contains_key(&key)
                        && !entry.in_flight.contains(&key)
                        && !entry.failed.contains(&key)
                    {
                        entry.in_flight.insert(key);
                        missing.push(img.clone());
                    }
                }
                (released, missing)
            }
        };

        self.enqueue_release(to_release);

        if to_decode.is_empty() {
            return;
        }

        let decoded: Vec<(TextureKey, Option<Arc<RenderImage>>)> = to_decode
            .into_iter()
            .map(|img| {
                let key = (img.id, img.generation);
                (key, decode_kitty_render_image(&img))
            })
            .collect();

        let mut mirrors = self.mirrors.lock().unwrap();
        let Some(entry) = mirrors.get_mut(&mirror_id) else {
            return;
        };
        if viewers.load(Ordering::Acquire) == 0 {
            entry.live_keys.clear();
            let released = entry.drain_textures();
            drop(mirrors);
            self.enqueue_release(released);
            return;
        }
        for (key, maybe_image) in decoded {
            entry.in_flight.remove(&key);
            if !entry.live_keys.contains(&key) {
                continue;
            }
            match maybe_image {
                Some(render_image) => {
                    entry.textures.entry(key).or_insert(render_image);
                }
                None => {
                    entry.failed.insert(key);
                }
            }
        }
    }

    fn on_last_viewer_dropped(&self, mirror_id: u64, viewers: &Arc<AtomicUsize>) {
        let released = {
            let mut mirrors = self.mirrors.lock().unwrap();
            if viewers.load(Ordering::Acquire) > 0 {
                return;
            }
            mirrors
                .get_mut(&mirror_id)
                .map(|entry| {
                    entry.live_keys.clear();
                    entry.drain_textures()
                })
                .unwrap_or_default()
        };
        self.enqueue_release(released);
    }

    fn on_mirror_dropped(&self, mirror_id: u64) {
        let released = {
            let mut mirrors = self.mirrors.lock().unwrap();
            mirrors
                .remove(&mirror_id)
                .map(|mut entry| entry.drain_textures())
                .unwrap_or_default()
        };
        self.enqueue_release(released);
    }
}

pub(crate) struct TerminalTextureCache {
    store: Arc<TextureStore>,
    _release_task: Task<()>,
}

impl Global for TerminalTextureCache {}

impl TerminalTextureCache {
    pub(crate) fn store(cx: &mut App) -> Arc<TextureStore> {
        if let Some(cache) = cx.try_global::<Self>() {
            return Arc::clone(&cache.store);
        }
        let (release_tx, mut release_rx) = mpsc::unbounded::<Vec<Arc<RenderImage>>>();
        let store = Arc::new(TextureStore::new(release_tx));
        let release_task = cx.spawn(async move |cx| {
            while let Some(mut batch) = release_rx.next().await {
                while let Ok(more) = release_rx.try_recv() {
                    batch.extend(more);
                }
                if batch.is_empty() {
                    continue;
                }
                cx.update(|cx| {
                    crate::surfaces::files::image::release_render_images(batch, None, cx);
                });
            }
        });
        cx.set_global(Self {
            store: Arc::clone(&store),
            _release_task: release_task,
        });
        store
    }

    pub(crate) fn observer(cx: &mut App) -> Arc<dyn MirrorGraphicsObserver> {
        Self::store(cx)
    }

    pub(crate) fn attach_mirror(mirror: &TerminalMirror, cx: &mut App) -> Arc<TextureStore> {
        let store = Self::store(cx);
        let observer: Arc<dyn MirrorGraphicsObserver> = store.clone();
        mirror.set_graphics_observer(observer);
        store
    }
}

pub(crate) fn band_for_z(z: i32) -> usize {
    if z < Z_BELOW_CELL_BG {
        0
    } else if z < 0 {
        1
    } else {
        2
    }
}

pub(crate) fn decode_kitty_render_image(img: &StoredImage) -> Option<Arc<RenderImage>> {
    let rgba_bytes = img.decode_rgba(PER_SCREEN_LIMIT).ok()?;
    let rgba = image::RgbaImage::from_raw(img.width, img.height, rgba_bytes)?;
    let clamped = crate::surfaces::files::image::clamp_frame_to_texture_limit(rgba);
    Some(crate::surfaces::files::image::rgba_to_render_image(clamped))
}

pub(crate) fn paint_band(band: &[PaintedImage], pane_bounds: Bounds<Pixels>, window: &mut Window) {
    if band.is_empty() {
        return;
    }
    window.with_content_mask(
        Some(ContentMask {
            bounds: pane_bounds,
        }),
        |window| {
            for item in band {
                let _ = window.paint_image(
                    item.dest_bounds,
                    item.image_bounds,
                    Corners::default(),
                    Arc::clone(&item.image),
                    0,
                    false,
                );
            }
        },
    );
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PaintEvent {
    PaneBackground,
    CellBackgrounds,
    Selections,
    Text,
    CursorOrIme,
    Image {
        band: usize,
        z: i32,
        image_id: u32,
        dest_bounds: Bounds<Pixels>,
        image_bounds: Bounds<Pixels>,
    },
}

#[cfg(test)]
#[derive(Default)]
struct PaintTrace(Vec<PaintEvent>);

#[cfg(test)]
impl Global for PaintTrace {}

#[cfg(test)]
pub(crate) fn record_paint_event(event: PaintEvent, cx: &mut App) {
    if cx.has_global::<PaintTrace>() {
        let trace = &mut cx.global_mut::<PaintTrace>().0;
        if event == PaintEvent::PaneBackground {
            trace.clear();
        }
        trace.push(event);
    }
}

#[cfg(test)]
#[derive(Default)]
struct BackgroundTrace {
    pane_background: Option<gpui::Hsla>,
    cell_quads: Vec<(Bounds<Pixels>, gpui::Hsla)>,
}

#[cfg(test)]
impl Global for BackgroundTrace {}

#[cfg(test)]
pub(crate) fn record_pane_background(color: gpui::Hsla, cx: &mut App) {
    if cx.has_global::<BackgroundTrace>() {
        cx.global_mut::<BackgroundTrace>().pane_background = Some(color);
    }
}

#[cfg(test)]
pub(crate) fn record_cell_backgrounds(quads: &[(Bounds<Pixels>, gpui::Hsla)], cx: &mut App) {
    if cx.has_global::<BackgroundTrace>() {
        cx.global_mut::<BackgroundTrace>().cell_quads = quads.to_vec();
    }
}

#[cfg(test)]
pub(crate) fn record_paint_band(band: usize, items: &[PaintedImage], cx: &mut App) {
    if cx.has_global::<PaintTrace>() {
        let trace = &mut cx.global_mut::<PaintTrace>().0;
        for item in items {
            trace.push(PaintEvent::Image {
                band,
                z: item.z,
                image_id: item.image_id,
                dest_bounds: item.dest_bounds,
                image_bounds: item.image_bounds,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
    use alacritty_terminal::selection::SelectionType;
    use base64::Engine as _;
    use gpui::prelude::*;
    use gpui::{
        div, px, Context, Entity, FocusHandle, Render, TestAppContext, VisualTestContext, Window,
    };
    use runner_app::terminal_ime::TerminalInput;
    use runner_core::protocol::terminal::{TerminalController, TerminalFrame};
    use runner_terminal::kitty::command::Compression;
    use runner_terminal::kitty::{
        DecodedFormat, Format, ForwardedItem, GlobalImageBudget, PreparedCommand, StoredImage,
        TerminalGraphics,
    };
    use runner_terminal::terminal::{
        ClipboardTarget, ModelOptions, TerminalHost, TerminalMirror, TerminalModel,
    };

    use super::*;
    use crate::terminal::control::tests::mirror_named;
    use crate::terminal::element::{TerminalElement, TerminalInteraction, TerminalStyle};

    fn kitty_rgba_command(params: &str, rgba: &[u8]) -> Vec<u8> {
        let encoded = base64::engine::general_purpose::STANDARD.encode(rgba);
        format!("\x1b_G{params};{encoded}\x1b\\").into_bytes()
    }

    fn kitty_raw_command(params: &str, rgba: &[u8]) -> Vec<u8> {
        let encoded = base64::engine::general_purpose::STANDARD.encode(rgba);
        format!("{params};{encoded}").into_bytes()
    }

    fn solid_rgba(width: u32, height: u32, pixel: [u8; 4]) -> Vec<u8> {
        pixel
            .into_iter()
            .cycle()
            .take((width as usize) * (height as usize) * 4)
            .collect()
    }

    fn send_and_wait(
        tx: &std::sync::mpsc::Sender<TerminalFrame>,
        ack: &std::sync::mpsc::Receiver<()>,
        frame: TerminalFrame,
    ) {
        tx.send(frame).unwrap();
        ack.recv_timeout(Duration::from_secs(5))
            .expect("frame worker should finish processing frame");
    }

    struct NullHost;

    impl TerminalHost for NullHost {
        fn write_input(&self, _session_id: &str, _bytes: &[u8]) -> anyhow::Result<()> {
            Ok(())
        }
        fn write_reply(&self, _session_id: &str, _bytes: &[u8]) -> anyhow::Result<()> {
            Ok(())
        }
        fn report_input_state(
            &self,
            _session_id: &str,
            _observation: runner_terminal::input_state::InputObservation,
        ) {
        }
        fn set_live_title(&self, _session_id: &str, _title: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn resize(&self, _session_id: &str, _cols: u16, _rows: u16) -> anyhow::Result<()> {
            Ok(())
        }
        fn is_local_host(&self, _host: &str) -> bool {
            true
        }
        fn clipboard_store(&self, _session_id: &str, _target: ClipboardTarget, _text: &str) {}
    }

    struct GraphicsPaneHost {
        terminal: Option<Arc<TerminalMirror>>,
        interaction: Option<Entity<TerminalInteraction>>,
        input: Option<Entity<TerminalInput>>,
        focus: FocusHandle,
        font_size: f32,
        line_height_factor: f32,
    }

    impl Render for GraphicsPaneHost {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let style = TerminalStyle {
                palette: runner_terminal::palette::RUNNER,
                font: crate::app_settings::AppSettings::default()
                    .terminal_font_family
                    .font(cx),
                font_size: self.font_size,
                app_zoom: 1.0,
                line_height_factor: self.line_height_factor,
                copy_on_select: false,
            };
            let mut root = div().size_full().p(px(12.));
            if let (Some(terminal), Some(interaction), Some(input)) =
                (&self.terminal, &self.interaction, &self.input)
            {
                root = root.child(
                    div()
                        .debug_selector(|| "GRAPHICS_PANE".into())
                        .w(px(320.))
                        .h(px(200.))
                        .child(TerminalElement::new(
                            Arc::clone(terminal),
                            interaction.clone(),
                            input.clone(),
                            self.focus.clone(),
                            true,
                            false,
                            style,
                        )),
                );
            }
            root
        }
    }

    fn live_key_for(mirror: &TerminalMirror, image_id: u32) -> TextureKey {
        let graphics = mirror.graphics().lock().unwrap();
        let img = graphics
            .primary
            .image_by_id(image_id)
            .expect("image should exist on primary screen");
        (img.id, img.generation)
    }

    #[test]
    fn texture_lifecycle_drops_atlas_entries_without_prepaint() {
        let mut cx = TestAppContext::single();
        let (_host, mirror, tx, ack) =
            mirror_named("graphics-lifecycle", TerminalController::Desktop);
        let mirror_id = mirror.mirror_id();
        let store = cx.update(|cx| TerminalTextureCache::attach_mirror(&mirror, cx));

        // 1. Hidden session (zero TerminalView leases): decodes nothing on arrival.
        let img_bytes = solid_rgba(4, 4, [255, 0, 0, 255]);
        let mut seq = 1u64;
        send_and_wait(
            &tx,
            &ack,
            TerminalFrame::Output {
                seq,
                bytes: kitty_rgba_command("a=T,f=32,s=4,v=4,i=1,C=1,q=2", &img_bytes).into(),
            },
        );
        assert_eq!(
            store.cached_count(mirror_id),
            0,
            "hidden session with zero leases must not decode or cache textures"
        );

        // 2. Showing the session decodes lazily on a background task and paints into the atlas.
        let view_a = mirror.view();
        let view_b = mirror.view();
        let interaction = cx.new(|_| TerminalInteraction::new(Arc::clone(&mirror)));
        let input = cx.new(|_| TerminalInput::new(Arc::clone(&mirror)));
        let window = cx.add_window(|_, cx| GraphicsPaneHost {
            terminal: Some(Arc::clone(&mirror)),
            interaction: Some(interaction.clone()),
            input: Some(input.clone()),
            focus: cx.focus_handle(),
            font_size: 14.,
            line_height_factor: 1.4,
        });
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.simulate_resize(gpui::size(px(400.), px(260.)));
        visual.refresh().unwrap();
        visual.run_until_parked();
        let key_initial = live_key_for(&mirror, 1);
        let img_initial = store
            .cached_image(mirror_id, key_initial)
            .expect("visible pane should lazily decode existing image");
        visual.refresh().unwrap();
        assert!(
            visual.update(|window, _| window.has_image_atlas_entry(&img_initial)),
            "painted image must be uploaded into GPUI's window sprite atlas"
        );

        // 3. Replacing the same image 200 times releases every superseded texture via
        // `cx.drop_image` WITHOUT another prepaint.
        let mut superseded = Vec::new();
        for step in 0..200u32 {
            seq += 1;
            let bytes = solid_rgba(4, 4, [(step & 0xff) as u8, 40, 200, 255]);
            send_and_wait(
                &tx,
                &ack,
                TerminalFrame::Output {
                    seq,
                    bytes: kitty_rgba_command("a=T,f=32,s=4,v=4,i=1,C=1,q=2", &bytes).into(),
                },
            );
            if step < 199 {
                let key = live_key_for(&mirror, 1);
                if let Some(img) = store.cached_image(mirror_id, key) {
                    superseded.push(img);
                }
            }
        }
        // Drive only foreground tasks (no `visual.refresh()` / no prepaint!).
        visual.run_until_parked();
        assert_eq!(store.cached_count(mirror_id), 1);
        assert!(
            !visual.update(|window, _| window.has_image_atlas_entry(&img_initial)),
            "replaced texture must be evicted from GPUI atlas without another prepaint"
        );
        assert_eq!(
            Arc::strong_count(&img_initial),
            1,
            "initial RenderImage must not be retained by cache or atlas"
        );
        for old in &superseded {
            assert_eq!(
                Arc::strong_count(old),
                1,
                "superseded RenderImage must not be retained by cache or atlas"
            );
        }
        drop(superseded);
        drop(img_initial);

        // Paint the 200th replacement into the atlas, then delete it and verify atlas eviction
        // without another prepaint.
        let key_latest = live_key_for(&mirror, 1);
        let img_latest = store
            .cached_image(mirror_id, key_latest)
            .expect("200th replacement should be decoded on frame worker");
        visual.update(|window, _| window.refresh());
        visual.refresh().unwrap();
        assert!(visual.update(|window, _| window.has_image_atlas_entry(&img_latest)));

        seq += 1;
        send_and_wait(
            &tx,
            &ack,
            TerminalFrame::Output {
                seq,
                bytes: b"\x1b_Ga=d,d=I,i=1,q=2\x1b\\".into(),
            },
        );
        visual.run_until_parked();
        assert_eq!(store.cached_count(mirror_id), 0);
        assert!(
            !visual.update(|window, _| window.has_image_atlas_entry(&img_latest)),
            "deleted texture must be evicted from GPUI atlas without another prepaint"
        );

        // 4. Eviction produced by a real authoritative TerminalModel with a tight budget:
        // image 10 is painted, then image 11 triggers daemon eviction (`d=I,i=10`) and releases
        // image 10 from GPUI's atlas without another prepaint.
        let budget = Arc::new(GlobalImageBudget::new(80));
        let model = TerminalModel::with_graphics(
            Arc::new(NullHost),
            "evict-model".into(),
            80,
            24,
            ModelOptions::default(),
            CellSize::ZERO,
            Some(budget),
        )
        .unwrap();
        let cmd1 = PreparedCommand::direct_only(&kitty_raw_command(
            "a=T,f=32,s=4,v=4,i=10,C=1,q=2",
            &solid_rgba(4, 4, [1, 2, 3, 255]),
        ))
        .unwrap();
        let (_, frames1, next_seq) = model.feed_forwarded(1, &[ForwardedItem::Command(cmd1)]);
        for bytes in frames1 {
            seq += 1;
            send_and_wait(
                &tx,
                &ack,
                TerminalFrame::Output {
                    seq,
                    bytes: bytes.into(),
                },
            );
        }
        let key_evict = live_key_for(&mirror, 10);
        let img_evict = store
            .cached_image(mirror_id, key_evict)
            .expect("image 10 should decode on frame worker");
        visual.update(|window, _| window.refresh());
        visual.refresh().unwrap();
        assert!(visual.update(|window, _| window.has_image_atlas_entry(&img_evict)));

        let cmd2 = PreparedCommand::direct_only(&kitty_raw_command(
            "a=T,f=32,s=4,v=4,i=11,C=1,q=2",
            &solid_rgba(4, 4, [4, 5, 6, 255]),
        ))
        .unwrap();
        let (_, frames2, _) = model.feed_forwarded(next_seq + 1, &[ForwardedItem::Command(cmd2)]);
        for bytes in frames2 {
            seq += 1;
            send_and_wait(
                &tx,
                &ack,
                TerminalFrame::Output {
                    seq,
                    bytes: bytes.into(),
                },
            );
        }
        for handle in model.stop_workers() {
            let _ = handle.join();
        }
        visual.run_until_parked();
        assert!(
            !visual.update(|window, _| window.has_image_atlas_entry(&img_evict)),
            "evicted texture must be evicted from GPUI atlas without another prepaint"
        );

        // Paint surviving image 11 into the atlas.
        let key_kept = live_key_for(&mirror, 11);
        let img_kept = store
            .cached_image(mirror_id, key_kept)
            .expect("image 11 should be cached");
        visual.update(|window, _| window.refresh());
        visual.refresh().unwrap();
        assert!(visual.update(|window, _| window.has_image_atlas_entry(&img_kept)));

        // 5. Hiding one of two panes (dropping view_a while view_b stays alive) keeps textures;
        // dropping the last lease (view_b) releases them without another prepaint.
        drop(view_a);
        visual.run_until_parked();
        assert_eq!(store.cached_count(mirror_id), 1);
        assert!(
            visual.update(|window, _| window.has_image_atlas_entry(&img_kept)),
            "hiding one of two panes showing the session must keep its textures"
        );

        drop(view_b);
        visual.run_until_parked();
        assert_eq!(store.cached_count(mirror_id), 0);
        assert!(
            !visual.update(|window, _| window.has_image_atlas_entry(&img_kept)),
            "dropping the last TerminalView lease must release textures without another prepaint"
        );

        // 6. Dropping the TerminalMirror itself (even while a lease still exists so
        // on_last_viewer_dropped has not fired) releases textures without another prepaint.
        let _leaked_lease = mirror.view();
        visual.update(|window, _| window.refresh());
        visual.refresh().unwrap();
        visual.run_until_parked();
        let img_for_drop = store
            .cached_image(mirror_id, key_kept)
            .expect("re-showing session should re-decode image 11");
        visual.update(|window, _| window.refresh());
        visual.refresh().unwrap();
        assert!(visual.update(|window, _| window.has_image_atlas_entry(&img_for_drop)));

        window
            .update(&mut visual, |host, _, cx| {
                host.terminal = None;
                host.interaction = None;
                host.input = None;
                cx.notify();
            })
            .unwrap();
        // Refresh the now-empty host twice so both of GPUI's double-buffered frames drop the old
        // mouse/input handlers while `mirror` and `_leaked_lease` remain alive.
        visual.refresh().unwrap();
        visual.update(|window, _| window.refresh());
        visual.refresh().unwrap();
        drop(interaction);
        drop(input);
        visual.update(|_, _| {});
        visual.run_until_parked();
        drop(tx);
        mirror.test_stop_frames();
        assert_eq!(
            Arc::strong_count(&mirror),
            1,
            "test should hold the last strong reference to TerminalMirror"
        );
        assert!(visual.update(|window, _| window.has_image_atlas_entry(&img_for_drop)));
        drop(mirror);
        visual.run_until_parked();
        assert_eq!(store.cached_count(mirror_id), 0);
        assert!(
            !visual.update(|window, _| window.has_image_atlas_entry(&img_for_drop)),
            "dropping TerminalMirror must release textures without another prepaint"
        );
    }

    #[test]
    fn oversized_kitty_image_halves_within_texture_limit() {
        let img = StoredImage {
            id: 1,
            number: 0,
            width: 9_000,
            height: 2,
            wire_format: Format::Rgba,
            compression: Compression::None,
            decoded_format: DecodedFormat::Rgba,
            encoded_data: Arc::from(solid_rgba(9_000, 2, [10, 20, 30, 255])),
            transient: false,
            implicit_id: false,
            placement_count: 1,
            generation: 1,
            transmission: 1,
        };
        let render_img = decode_kitty_render_image(&img).expect("oversized image should decode");
        let sz = render_img.size(0);
        assert_eq!(sz.width.0, 4_500);
        assert_eq!(sz.height.0, 1);
        assert!(sz.width.0 as u32 <= crate::surfaces::files::image::MAX_TEXTURE_SIDE);
    }

    #[test]
    fn paint_bands_order_and_placement_bounds_at_1x_and_2x_scale() {
        let mut cx = TestAppContext::single();
        let (_host, mirror, tx, ack) = mirror_named("graphics-render", TerminalController::Desktop);
        let mirror_id = mirror.mirror_id();
        let store = cx.update(|cx| TerminalTextureCache::attach_mirror(&mirror, cx));
        let _view = mirror.view();

        let interaction = cx.new(|_| TerminalInteraction::new(Arc::clone(&mirror)));
        let input = cx.new(|_| TerminalInput::new(Arc::clone(&mirror)));
        let window = cx.add_window(|_, cx| GraphicsPaneHost {
            terminal: Some(Arc::clone(&mirror)),
            interaction: Some(interaction.clone()),
            input: Some(input.clone()),
            focus: cx.focus_handle(),
            font_size: 14.,
            line_height_factor: 1.4,
        });
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.simulate_resize(gpui::size(px(400.), px(260.)));
        visual.update(|window, cx| {
            window.set_scale_factor(1.0);
            window.activate_window();
            let focus = window
                .root::<GraphicsPaneHost>()
                .flatten()
                .unwrap()
                .read(cx)
                .focus
                .clone();
            focus.focus(window, cx);
        });
        visual.refresh().unwrap();

        let (cell_w, line_h) = visual.update(|window, cx| {
            let font = crate::app_settings::AppSettings::default()
                .terminal_font_family
                .font(cx);
            let font_id = window.text_system().resolve_font(&font);
            let cw = window.text_system().em_advance(font_id, px(14.)).unwrap();
            let lh = px(crate::terminal::element::row_height(14., 1.4));
            (cw, lh)
        });
        let (dev_cw_1x, dev_ch_1x) = device_cell_size(f32::from(cell_w), f32::from(line_h), 1.0);

        // Configure 1x session cell size, print text with a colored background, place one image
        // in each of the 3 z-bands, and add a selection + focused block cursor.
        let mut stream = Vec::new();
        stream.extend_from_slice(b"\x1b[41mHELLO\x1b[0m");
        // Band 0: z < -1_073_741_824 (behind cell backgrounds)
        stream.extend_from_slice(&kitty_rgba_command(
            "a=T,f=32,s=16,v=16,i=1,p=1,z=-1073741825,C=1,q=2",
            &solid_rgba(16, 16, [255, 0, 0, 255]),
        ));
        // Band 1: -1_073_741_824 <= z < 0 (between cell backgrounds/selection and text)
        stream.extend_from_slice(&kitty_rgba_command(
            "a=T,f=32,s=24,v=16,i=2,p=1,z=-1,C=1,q=2",
            &solid_rgba(24, 16, [0, 255, 0, 255]),
        ));
        // Band 2: z >= 0 (above text, below cursor)
        stream.extend_from_slice(&kitty_rgba_command(
            "a=T,f=32,s=32,v=16,i=3,p=1,z=0,C=1,q=2",
            &solid_rgba(32, 16, [0, 0, 255, 255]),
        ));
        send_and_wait(
            &tx,
            &ack,
            TerminalFrame::Resized {
                seq: 1,
                cols: 30,
                rows: 10,
                cell_width: dev_cw_1x,
                cell_height: dev_ch_1x,
                controller: TerminalController::Desktop,
            },
        );
        send_and_wait(
            &tx,
            &ack,
            TerminalFrame::Output {
                seq: 2,
                bytes: stream.into(),
            },
        );
        mirror.start_selection(
            SelectionType::Simple,
            GridPoint::new(Line(0), Column(0)),
            Side::Left,
        );
        mirror.update_selection(GridPoint::new(Line(0), Column(2)), Side::Right);

        visual.update(|window, cx| {
            cx.set_global(PaintTrace::default());
            window.refresh();
        });
        visual.run_until_parked();

        // Verify all 3 textures uploaded to GPUI's window sprite atlas.
        for id in [1, 2, 3] {
            let key = live_key_for(&mirror, id);
            let img = store.cached_image(mirror_id, key).unwrap();
            assert!(visual.update(|window, _| window.has_image_atlas_entry(&img)));
        }

        // Verify exact 7-layer paint order and 1x destination bounds.
        let events_1x = visual.update(|_, cx| cx.global::<PaintTrace>().0.clone());
        let expected_sx_1x = f32::from(cell_w) / f32::from(dev_cw_1x);
        let expected_sy_1x = f32::from(line_h) / f32::from(dev_ch_1x);
        assert_eq!(events_1x.len(), 8, "unexpected paint trace: {events_1x:?}");
        assert_eq!(events_1x[0], PaintEvent::PaneBackground);
        match &events_1x[1] {
            PaintEvent::Image {
                band: 0,
                image_id: 1,
                dest_bounds,
                ..
            } => {
                assert!((f32::from(dest_bounds.size.width) - 16.0 * expected_sx_1x).abs() < 1e-3);
                assert!((f32::from(dest_bounds.size.height) - 16.0 * expected_sy_1x).abs() < 1e-3);
            }
            other => panic!("expected band 0 image, got {other:?}"),
        }
        assert_eq!(events_1x[2], PaintEvent::CellBackgrounds);
        assert_eq!(events_1x[3], PaintEvent::Selections);
        match &events_1x[4] {
            PaintEvent::Image {
                band: 1,
                image_id: 2,
                dest_bounds,
                ..
            } => {
                assert!((f32::from(dest_bounds.size.width) - 24.0 * expected_sx_1x).abs() < 1e-3);
                assert!((f32::from(dest_bounds.size.height) - 16.0 * expected_sy_1x).abs() < 1e-3);
            }
            other => panic!("expected band 1 image, got {other:?}"),
        }
        assert_eq!(events_1x[5], PaintEvent::Text);
        match &events_1x[6] {
            PaintEvent::Image {
                band: 2,
                image_id: 3,
                dest_bounds,
                ..
            } => {
                assert!((f32::from(dest_bounds.size.width) - 32.0 * expected_sx_1x).abs() < 1e-3);
                assert!((f32::from(dest_bounds.size.height) - 16.0 * expected_sy_1x).abs() < 1e-3);
            }
            other => panic!("expected band 2 image, got {other:?}"),
        }
        assert_eq!(events_1x[7], PaintEvent::CursorOrIme);

        // Verify 2x Retina scale: same native image sizes map to half the logical size
        // (= 1:1 native device pixels on Retina!).
        let (dev_cw_2x, dev_ch_2x) = device_cell_size(f32::from(cell_w), f32::from(line_h), 2.0);
        send_and_wait(
            &tx,
            &ack,
            TerminalFrame::Resized {
                seq: 3,
                cols: 30,
                rows: 10,
                cell_width: dev_cw_2x,
                cell_height: dev_ch_2x,
                controller: TerminalController::Desktop,
            },
        );
        visual.update(|window, cx| {
            window.set_scale_factor(2.0);
            cx.set_global(PaintTrace::default());
        });
        visual.run_until_parked();
        let events_2x = visual.update(|_, cx| cx.global::<PaintTrace>().0.clone());
        let expected_sx_2x = f32::from(cell_w) / f32::from(dev_cw_2x);
        let expected_sy_2x = f32::from(line_h) / f32::from(dev_ch_2x);
        assert_eq!(
            events_2x.len(),
            8,
            "unexpected 2x paint trace: {events_2x:?}"
        );
        match &events_2x[6] {
            PaintEvent::Image {
                band: 2,
                image_id: 3,
                dest_bounds,
                ..
            } => {
                assert!((f32::from(dest_bounds.size.width) - 32.0 * expected_sx_2x).abs() < 1e-3);
                assert!((f32::from(dest_bounds.size.height) - 16.0 * expected_sy_2x).abs() < 1e-3);
                // Logical width at 2x is ~half of 1x (~16 logical px = 32 device px).
                assert!(f32::from(dest_bounds.size.width) < 20.0);
            }
            other => panic!("expected band 2 image at 2x, got {other:?}"),
        }

        // Verify prepare_paint_bands directly for exact logical coordinates:
        // 1x vs 2x, zero session cell fallback, c/r, X/Y offset, source-rect crop,
        // scrollback display_offset, relative placement, and intra-band sorting.
        let mut graphics = TerminalGraphics::mirror();
        let model = TerminalModel::with_graphics(
            Arc::new(NullHost),
            "layout-model".into(),
            40,
            10,
            ModelOptions::default(),
            CellSize {
                width: 16,
                height: 32,
            },
            None,
        )
        .unwrap();
        let rgba_64x32 = solid_rgba(64, 32, [200, 100, 50, 255]);
        let items = vec![
            ForwardedItem::Command(
                PreparedCommand::direct_only(&kitty_raw_command(
                    "a=T,f=32,s=64,v=32,i=1,p=2,x=16,y=8,w=32,h=16,X=4,Y=8,c=4,r=2,z=0,C=1,q=2",
                    &rgba_64x32,
                ))
                .unwrap(),
            ),
            ForwardedItem::Command(
                PreparedCommand::direct_only(b"a=p,i=1,p=1,z=0,C=1,q=2").unwrap(),
            ),
            ForwardedItem::Command(
                PreparedCommand::direct_only(b"a=p,i=1,p=3,P=1,Q=1,H=1,V=1,z=1,C=1,q=2").unwrap(),
            ),
        ];
        let (_, norm_frames, _) = model.feed_forwarded(1, &items);
        for handle in model.stop_workers() {
            let _ = handle.join();
        }
        let norm: Vec<u8> = norm_frames.into_iter().flatten().collect();
        let test_mirror_id = 999_999u64;
        let test_viewers = Arc::new(AtomicUsize::new(1));
        {
            let mut term = runner_terminal::replay::new_term(40, 10);
            let sgr = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let mut scanner = runner_terminal::kitty::Scanner::default();
            let mut handler = runner_terminal::terminal::stream::StreamHandler::new(
                &mut term,
                &mut graphics,
                &sgr,
                CellSize {
                    width: 16,
                    height: 32,
                },
                None,
            );
            if let runner_terminal::kitty::ScanOutput::Items(items) = scanner.scan(&norm) {
                for item in items {
                    if let runner_terminal::kitty::ScanItem::Command(raw) = item {
                        if let Some(cmd) = PreparedCommand::direct_only(&raw) {
                            handler.execute_kitty(&cmd, runner_terminal::kitty::ExecMode::Mirror);
                        }
                    }
                }
            }
        }
        let img1 = graphics.primary.image_by_id(1).unwrap().clone();
        store.on_graphics_updated(
            test_mirror_id,
            1,
            &test_viewers,
            &[(img1.id, img1.generation)],
            &[img1],
        );

        let bounds = Bounds::new(point(px(10.), px(20.)), size(px(320.), px(160.)));
        // At 2x session cell (16, 32) on viewer cell (8, 16): sx = 0.5, sy = 0.5.
        let (bands_2x, missing) = store.prepare_paint_bands(
            test_mirror_id,
            false,
            &graphics,
            0,
            bounds,
            px(8.),
            px(16.),
            2.0,
            CellSize {
                width: 16,
                height: 32,
            },
        );
        assert!(missing.is_empty());
        assert_eq!(bands_2x[2].len(), 3);
        // Sorted by (z, image_id, placement_order): (0, 1, Ext(1)), (0, 1, Ext(2)), (1, 1, Ext(3)).
        assert_eq!(bands_2x[2][0].placement_order, (1, 1));
        assert_eq!(bands_2x[2][1].placement_order, (1, 2));
        assert_eq!(bands_2x[2][2].placement_order, (1, 3));

        // p=1: native 64x32 at sx=0.5, sy=0.5 => logical 32x16 at (10, 20).
        let p1 = &bands_2x[2][0];
        assert_eq!(
            p1.dest_bounds,
            Bounds::new(point(px(10.), px(20.)), size(px(32.), px(16.)))
        );
        assert_eq!(p1.image_bounds, p1.dest_bounds);

        // p=2: X=4, Y=8 => +2, +4 logical; c=4, r=2 => pixel_size=(4*16-4, 2*32-8)=(60, 56)
        // => logical size (30, 28); source_rect=(16, 8, 32, 16) => scale_x=30/32, scale_y=28/16
        // => image_bounds origin=(12 - 16*30/32, 24 - 8*28/16) = (-3, 10), size=(60, 56).
        let p2 = &bands_2x[2][1];
        assert!((f32::from(p2.dest_bounds.origin.x) - 12.0).abs() < 1e-4);
        assert!((f32::from(p2.dest_bounds.origin.y) - 24.0).abs() < 1e-4);
        assert!((f32::from(p2.dest_bounds.size.width) - 30.0).abs() < 1e-4);
        assert!((f32::from(p2.dest_bounds.size.height) - 28.0).abs() < 1e-4);
        assert!((f32::from(p2.image_bounds.origin.x) - (-3.0)).abs() < 1e-4);
        assert!((f32::from(p2.image_bounds.origin.y) - 10.0).abs() < 1e-4);
        assert!((f32::from(p2.image_bounds.size.width) - 60.0).abs() < 1e-4);
        assert!((f32::from(p2.image_bounds.size.height) - 56.0).abs() < 1e-4);

        // p=3: relative to p=1 with H=1, V=1 => origin at (10 + 8, 20 + 16) = (18, 36).
        let p3 = &bands_2x[2][2];
        assert_eq!(p3.dest_bounds.origin, point(px(18.), px(36.)));

        // Zero session cell size fallback at 1x vs 2x scale:
        let (bands_zero_1x, _) = store.prepare_paint_bands(
            test_mirror_id,
            false,
            &graphics,
            0,
            bounds,
            px(8.),
            px(16.),
            1.0,
            CellSize::ZERO,
        );
        assert_eq!(
            bands_zero_1x[2][0].dest_bounds.size,
            size(px(64.), px(32.)),
            "1x zero-cell fallback maps 1 image pixel to 1 logical pixel"
        );
        let (bands_zero_2x, _) = store.prepare_paint_bands(
            test_mirror_id,
            false,
            &graphics,
            0,
            bounds,
            px(8.),
            px(16.),
            2.0,
            CellSize::ZERO,
        );
        assert_eq!(
            bands_zero_2x[2][0].dest_bounds.size,
            size(px(32.), px(16.)),
            "2x zero-cell fallback maps 2 image pixels to 1 logical pixel"
        );

        // Scrollback culling: scrolling down 20 rows pushes row-0 placements below pane_bottom.
        let (bands_scrolled_off, _) = store.prepare_paint_bands(
            test_mirror_id,
            false,
            &graphics,
            20,
            bounds,
            px(8.),
            px(16.),
            2.0,
            CellSize {
                width: 16,
                height: 32,
            },
        );
        assert!(bands_scrolled_off[2].is_empty());

        store.on_mirror_dropped(test_mirror_id);
        visual.run_until_parked();
    }

    #[test]
    fn configure_interleaving_keeps_cache_ordered_and_avoids_foreground_decode() {
        use alacritty_terminal::vte::ansi::CursorShape;
        use runner_core::protocol::terminal::{
            TerminalAttachment, TerminalMetadata, TerminalSnapshot, TerminalSubscription,
        };
        use runner_core::protocol::{
            ClientError, DaemonClient, EventSubscription, Request, Response, Transport,
        };
        use std::sync::mpsc;

        struct Frames(mpsc::Receiver<TerminalFrame>);
        impl TerminalSubscription for Frames {
            fn recv(&mut self) -> Result<TerminalFrame, ClientError> {
                self.0.recv().map_err(|_| ClientError::msg("closed"))
            }
            fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
                Arc::new(|| {})
            }
        }
        struct InterleavingHost {
            streams: Mutex<std::collections::VecDeque<(Vec<u8>, mpsc::Receiver<TerminalFrame>)>>,
        }
        impl Transport for InterleavingHost {
            fn call(&self, request: Request) -> Result<Response, ClientError> {
                match request {
                    Request::terminal_metadata { .. } => {
                        Ok(Response::terminal_metadata(Ok(TerminalMetadata::default())))
                    }
                    Request::terminal_configure { .. } => Ok(Response::terminal_configure(Ok(()))),
                    _ => Err(ClientError::msg("unexpected request")),
                }
            }
            fn subscribe(&self) -> Box<dyn EventSubscription> {
                unreachable!()
            }
            fn attach(&self, _: &str) -> Result<TerminalAttachment, ClientError> {
                let (snapshot_bytes, rx) = self.streams.lock().unwrap().pop_front().unwrap();
                Ok(TerminalAttachment {
                    subscriber_id: 1,
                    snapshot: TerminalSnapshot {
                        seq: 0,
                        activity: None,
                        cols: 80,
                        rows: 4,
                        cell_width: 8,
                        cell_height: 16,
                        controller: TerminalController::Desktop,
                        bytes: snapshot_bytes,
                        unfinished_len: 0,
                        preceding_char: None,
                    },
                    frames: Box::new(Frames(rx)),
                })
            }
        }

        struct GateObserver {
            inner: Arc<TextureStore>,
            main_thread: std::thread::ThreadId,
            pause_next_main: std::sync::atomic::AtomicBool,
            main_decode_counts: Mutex<Vec<usize>>,
            main_entered: mpsc::Sender<()>,
            main_resume: Mutex<mpsc::Receiver<()>>,
            worker_done: mpsc::Sender<(u64, usize)>,
        }
        impl MirrorGraphicsObserver for GateObserver {
            fn on_graphics_updated(
                &self,
                mirror_id: u64,
                sync_seq: u64,
                viewers: &Arc<AtomicUsize>,
                live_keys: &[(u32, u64)],
                decode_candidates: &[StoredImage],
            ) {
                if std::thread::current().id() == self.main_thread {
                    self.main_decode_counts
                        .lock()
                        .unwrap()
                        .push(decode_candidates.len());
                    if self.pause_next_main.swap(false, Ordering::AcqRel) {
                        self.main_entered.send(()).unwrap();
                        self.main_resume
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(5))
                            .unwrap();
                    }
                    self.inner.on_graphics_updated(
                        mirror_id,
                        sync_seq,
                        viewers,
                        live_keys,
                        decode_candidates,
                    );
                } else {
                    self.inner.on_graphics_updated(
                        mirror_id,
                        sync_seq,
                        viewers,
                        live_keys,
                        decode_candidates,
                    );
                    let _ = self.worker_done.send((sync_seq, live_keys.len()));
                }
            }
            fn on_last_viewer_dropped(&self, mirror_id: u64, viewers: &Arc<AtomicUsize>) {
                self.inner.on_last_viewer_dropped(mirror_id, viewers);
            }
            fn on_mirror_dropped(&self, mirror_id: u64) {
                self.inner.on_mirror_dropped(mirror_id);
            }
        }

        let cx = TestAppContext::single();
        let store = cx.update(TerminalTextureCache::store);
        let (tx1, rx1) = mpsc::channel();
        let (tx2, rx2) = mpsc::channel();
        let reattach_snapshot =
            kitty_rgba_command("a=T,f=32,s=1,v=1,i=2,C=1,q=2", &[0, 255, 0, 255]);
        let host = Arc::new(InterleavingHost {
            streams: Mutex::new(std::collections::VecDeque::from([
                (Vec::new(), rx1),
                (reattach_snapshot, rx2),
            ])),
        });
        let mirror =
            TerminalMirror::attach(DaemonClient::new(host), "c1-order".into(), Arc::new(|| {}))
                .unwrap();
        let mirror_id = mirror.mirror_id();
        let (main_entered_tx, main_entered_rx) = mpsc::channel();
        let (main_resume_tx, main_resume_rx) = mpsc::channel();
        let (worker_done_tx, worker_done_rx) = mpsc::channel();
        let gate = Arc::new(GateObserver {
            inner: Arc::clone(&store),
            main_thread: std::thread::current().id(),
            pause_next_main: std::sync::atomic::AtomicBool::new(true),
            main_decode_counts: Mutex::default(),
            main_entered: main_entered_tx,
            main_resume: Mutex::new(main_resume_rx),
            worker_done: worker_done_tx,
        });
        mirror.set_graphics_observer(gate.clone());

        // 1. Load image 1 while hidden and scroll its placement into history.
        tx1.send(TerminalFrame::Output {
            seq: 1,
            bytes: b"\x1b_Ga=T,f=32,s=1,v=1,i=1,C=1,q=2;/////w==\x1b\\\r\n\r\n\r\n\r\n\r\n\r\n"
                .into(),
        })
        .unwrap();
        let (seq1, count1) = worker_done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!((seq1, count1), (1, 1));
        assert_eq!(store.cached_count(mirror_id), 0);

        // 2. Take a lease, then shrink scrollback on the UI thread while pausing
        //    its observer callback before TextureStore reconciliation. Meanwhile the
        //    frame worker deletes image 1 and delivers the empty set first.
        let _view = mirror.view();
        let helper = std::thread::spawn(move || {
            main_entered_rx
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            tx1.send(TerminalFrame::Output {
                seq: 2,
                bytes: b"\x1b_Ga=d,d=I,i=1,q=2;\x1b\\".into(),
            })
            .unwrap();
            let (seq_del, count_del) = worker_done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(count_del, 0);
            assert!(seq_del > seq1);
            main_resume_tx.send(()).unwrap();
            (tx1, worker_done_rx)
        });
        mirror.configure(0, CursorShape::Block, false);
        let (tx1, worker_done_rx) = helper.join().unwrap();

        assert!(mirror.live_image_keys().is_empty());
        assert_eq!(
            store.cached_count(mirror_id),
            0,
            "stale configure callback must not resurrect deleted image"
        );
        assert_eq!(
            store.live_keys_count(mirror_id),
            0,
            "stale configure callback must not resurrect live_keys"
        );
        assert_eq!(
            *gate.main_decode_counts.lock().unwrap(),
            vec![0],
            "configure on UI thread must never pass decode_candidates"
        );

        // 3. Subsequent frame worker image still reconciles and caches cleanly.
        tx1.send(TerminalFrame::Output {
            seq: 3,
            bytes: kitty_rgba_command(
                "a=T,f=32,s=2,v=2,i=2,C=1,q=2",
                &solid_rgba(2, 2, [9, 8, 7, 255]),
            )
            .into(),
        })
        .unwrap();
        let (_, count2) = worker_done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(count2, 1);
        assert_eq!(store.cached_count(mirror_id), 1);

        // 4. Reattach on caller thread must also pass zero synchronous decode candidates.
        drop(tx1);
        mirror.reattach().unwrap();
        assert_eq!(
            *gate.main_decode_counts.lock().unwrap(),
            vec![0, 0],
            "reattach must not decode synchronously on the caller thread"
        );
        drop(tx2);
        mirror.test_stop_frames();
        drop(mirror);
        cx.run_until_parked();
    }

    #[test]
    fn pending_lazy_decode_does_not_lose_painted_textures_during_mirror_teardown() {
        use std::sync::mpsc;

        struct PausedDropObserver {
            inner: Arc<TextureStore>,
            drop_entered: mpsc::Sender<()>,
            drop_proceed: Mutex<mpsc::Receiver<()>>,
        }
        impl MirrorGraphicsObserver for PausedDropObserver {
            fn on_graphics_updated(
                &self,
                mirror_id: u64,
                sync_seq: u64,
                viewers: &Arc<AtomicUsize>,
                live_keys: &[(u32, u64)],
                decode_candidates: &[StoredImage],
            ) {
                self.inner.on_graphics_updated(
                    mirror_id,
                    sync_seq,
                    viewers,
                    live_keys,
                    decode_candidates,
                );
            }
            fn on_last_viewer_dropped(&self, mirror_id: u64, viewers: &Arc<AtomicUsize>) {
                self.inner.on_last_viewer_dropped(mirror_id, viewers);
            }
            fn on_mirror_dropped(&self, mirror_id: u64) {
                let _ = self.drop_entered.send(());
                let _ = self
                    .drop_proceed
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5));
                self.inner.on_mirror_dropped(mirror_id);
            }
        }

        let mut cx = TestAppContext::single();
        let (_host, mirror, tx, ack) = mirror_named("teardown-race", TerminalController::Desktop);
        let mirror_id = mirror.mirror_id();
        let store = cx.update(|cx| TerminalTextureCache::attach_mirror(&mirror, cx));

        let _leaked_lease = mirror.view();
        send_and_wait(
            &tx,
            &ack,
            TerminalFrame::Output {
                seq: 1,
                bytes: kitty_rgba_command(
                    "a=T,f=32,s=4,v=4,i=1,C=1,q=2",
                    &solid_rgba(4, 4, [255, 80, 20, 255]),
                )
                .into(),
            },
        );
        let key1 = live_key_for(&mirror, 1);
        let img1 = store
            .cached_image(mirror_id, key1)
            .expect("image 1 should be decoded on frame worker");

        let interaction = cx.new(|_| TerminalInteraction::new(Arc::clone(&mirror)));
        let input = cx.new(|_| TerminalInput::new(Arc::clone(&mirror)));
        let window = cx.add_window(|_, cx| GraphicsPaneHost {
            terminal: Some(Arc::clone(&mirror)),
            interaction: Some(interaction.clone()),
            input: Some(input.clone()),
            focus: cx.focus_handle(),
            font_size: 14.,
            line_height_factor: 1.4,
        });
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.simulate_resize(gpui::size(px(400.), px(260.)));
        visual.refresh().unwrap();
        assert!(
            visual.update(|window, _| window.has_image_atlas_entry(&img1)),
            "image 1 must be painted into GPUI atlas before teardown"
        );

        // Queue a pending lazy decode alongside the already-painted texture.
        let pending_img = StoredImage {
            id: 2,
            number: 0,
            width: 4,
            height: 4,
            wire_format: Format::Rgba,
            compression: Compression::None,
            decoded_format: DecodedFormat::Rgba,
            encoded_data: Arc::from(solid_rgba(4, 4, [10, 200, 10, 255])),
            transient: false,
            implicit_id: false,
            placement_count: 1,
            generation: 999_999,
            transmission: 999_999,
        };
        visual.update(|_, cx| {
            store.schedule_decode(&mirror, &interaction, vec![pending_img], cx);
        });

        // Unmount mirror from UI and stop its frame worker so this test owns the last Arc.
        window
            .update(&mut visual, |host, _, cx| {
                host.terminal = None;
                host.interaction = None;
                host.input = None;
                cx.notify();
            })
            .unwrap();
        visual.refresh().unwrap();
        visual.update(|window, _| window.refresh());
        visual.refresh().unwrap();
        drop(interaction);
        drop(input);
        visual.update(|_, _| {});
        drop(tx);
        mirror.test_stop_frames();
        assert_eq!(Arc::strong_count(&mirror), 1);

        // Install PausedDropObserver now (after prepaint can no longer overwrite it) so
        // `Drop for TerminalMirror` pauses inside `on_mirror_dropped` after strong count
        // reaches 0 (`Weak::upgrade() == None`) and before `store.on_mirror_dropped` runs.
        let (drop_entered_tx, drop_entered_rx) = mpsc::channel();
        let (drop_proceed_tx, drop_proceed_rx) = mpsc::channel();
        mirror.set_graphics_observer(Arc::new(PausedDropObserver {
            inner: Arc::clone(&store),
            drop_entered: drop_entered_tx,
            drop_proceed: Mutex::new(drop_proceed_rx),
        }));

        // Drop mirror on a helper thread that pauses inside `on_mirror_dropped` so
        // `Weak::upgrade` is already `None` while the cache entry still holds `img1`.
        let drop_thread = std::thread::spawn(move || drop(mirror));
        drop_entered_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap();

        // Let `schedule_decode` finish while `Weak::upgrade() == None`. It must NOT
        // remove the cache entry before `on_mirror_dropped` can enqueue `img1` for release.
        visual.run_until_parked();
        assert_eq!(
            store.cached_count(mirror_id),
            1,
            "schedule_decode must leave teardown eviction to on_mirror_dropped"
        );
        assert!(visual.update(|window, _| window.has_image_atlas_entry(&img1)));

        // Resume `on_mirror_dropped` and verify `img1` is evicted from GPUI's atlas.
        drop_proceed_tx.send(()).unwrap();
        drop_thread.join().unwrap();
        visual.run_until_parked();
        assert_eq!(store.cached_count(mirror_id), 0);
        assert!(
            !visual.update(|window, _| window.has_image_atlas_entry(&img1)),
            "on_mirror_dropped must release the painted texture via cx.drop_image"
        );
    }

    #[test]
    fn lazy_decode_respects_decset_2026_synchronized_update_guard() {
        let mut cx = TestAppContext::single();
        let wakes = Arc::new(AtomicUsize::new(0));
        let wakes_cb = Arc::clone(&wakes);
        let (_host, mirror, tx, ack) = crate::terminal::control::tests::mirror_waking(
            "graphics-sync-2026",
            TerminalController::Desktop,
            Arc::new(move || {
                wakes_cb.fetch_add(1, Ordering::SeqCst);
            }),
        );
        let mirror_id = mirror.mirror_id();
        let store = cx.update(|cx| TerminalTextureCache::attach_mirror(&mirror, cx));

        // Load image 1 while hidden so it stays undecoded in TextureStore.
        send_and_wait(
            &tx,
            &ack,
            TerminalFrame::Output {
                seq: 1,
                bytes: kitty_rgba_command(
                    "a=T,f=32,s=4,v=4,i=1,C=1,q=2",
                    &solid_rgba(4, 4, [0, 128, 255, 255]),
                )
                .into(),
            },
        );
        drop(tx);
        mirror.test_stop_frames();
        assert_eq!(store.cached_count(mirror_id), 0);

        let _view = mirror.view();
        let interaction = cx.new(|_| TerminalInteraction::new(Arc::clone(&mirror)));
        let input = cx.new(|_| TerminalInput::new(Arc::clone(&mirror)));
        let interaction_notifies = Arc::new(AtomicUsize::new(0));
        let notifies_cb = Arc::clone(&interaction_notifies);
        let _sub = cx.update(|cx| {
            cx.observe(&interaction, move |_, _| {
                notifies_cb.fetch_add(1, Ordering::SeqCst);
            })
        });
        let window = cx.add_window(|_, cx| GraphicsPaneHost {
            terminal: Some(Arc::clone(&mirror)),
            interaction: Some(interaction.clone()),
            input: Some(input.clone()),
            focus: cx.focus_handle(),
            font_size: 14.,
            line_height_factor: 1.4,
        });
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.simulate_resize(gpui::size(px(400.), px(260.)));
        // First refresh schedules lazy decode of image 1 without completing it yet.
        visual.refresh().unwrap();
        assert_eq!(store.cached_count(mirror_id), 0);

        // Open DECSET 2026 synchronized update before the lazy decode finishes.
        mirror.test_feed_unscheduled(2, b"\x1b[?2026h");
        wakes.store(0, Ordering::SeqCst);
        interaction_notifies.store(0, Ordering::SeqCst);

        // Complete background lazy decode while DECSET 2026 is open.
        visual.run_until_parked();
        let key1 = live_key_for(&mirror, 1);
        let img1 = store
            .cached_image(mirror_id, key1)
            .expect("lazy decode should populate cache while DECSET 2026 is open");
        assert_eq!(
            wakes.load(Ordering::SeqCst),
            0,
            "lazy decode must not wake mirror viewers during DECSET 2026"
        );
        assert_eq!(
            interaction_notifies.load(Ordering::SeqCst),
            0,
            "lazy decode must not notify TerminalInteraction during DECSET 2026"
        );

        // Closing DECSET 2026 wakes viewers and the cached image paints cleanly.
        mirror.test_feed_unscheduled(3, b"\x1b[?2026l");
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        visual.update(|window, _| window.refresh());
        visual.refresh().unwrap();
        assert!(visual.update(|window, _| window.has_image_atlas_entry(&img1)));
    }

    #[test]
    fn relative_placement_with_negative_column_keeps_visible_overlap_and_culls_outside() {
        let cx = TestAppContext::single();
        let store = cx.update(TerminalTextureCache::store);
        let cell = CellSize {
            width: 8,
            height: 16,
        };
        let mut graphics = TerminalGraphics::mirror();
        let mut term = runner_terminal::replay::new_term(40, 10);
        let sgr = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut scanner = runner_terminal::kitty::Scanner::default();
        let mut proc: alacritty_terminal::vte::ansi::Processor =
            alacritty_terminal::vte::ansi::Processor::new();

        // Ghostty v1.4.0 vector: parent pinned at (2, 2) (1-based CSI 3;3H).
        // - p=2: 2x2 child with H=-3, V=-3 -> origin (-1, -1); bottom-right cell (0, 0) visible.
        // - p=3: 1x1 child with H=-3, V=-3 -> origin (-1, -1); fully outside.
        // - p=4: 2x1 child with H=-3, V=-3 -> origin (-1, -1); ends at y=0, fully outside.
        // - p=5: 1x2 child with H=-3, V=-3 -> origin (-1, -1); ends at x=0, fully outside.
        let rgba = solid_rgba(16, 16, [120, 40, 200, 255]);
        let mut stream = b"\x1b[3;3H".to_vec();
        stream.extend_from_slice(&kitty_rgba_command(
            "a=T,f=32,s=16,v=16,i=1,p=1,c=1,r=1,z=0,C=1,q=2",
            &rgba,
        ));
        stream.extend_from_slice(b"\x1b_Ga=p,i=1,p=2,P=1,Q=1,H=-3,V=-3,c=2,r=2,z=1,C=1,q=2\x1b\\");
        stream.extend_from_slice(b"\x1b_Ga=p,i=1,p=3,P=1,Q=1,H=-3,V=-3,c=1,r=1,z=2,C=1,q=2\x1b\\");
        stream.extend_from_slice(b"\x1b_Ga=p,i=1,p=4,P=1,Q=1,H=-3,V=-3,c=2,r=1,z=3,C=1,q=2\x1b\\");
        stream.extend_from_slice(b"\x1b_Ga=p,i=1,p=5,P=1,Q=1,H=-3,V=-3,c=1,r=2,z=4,C=1,q=2\x1b\\");
        let mut handler = runner_terminal::terminal::stream::StreamHandler::new(
            &mut term,
            &mut graphics,
            &sgr,
            cell,
            None,
        );
        if let runner_terminal::kitty::ScanOutput::Items(items) = scanner.scan(&stream) {
            for item in items {
                match item {
                    runner_terminal::kitty::ScanItem::Text(text) => {
                        proc.advance(&mut handler, &text);
                    }
                    runner_terminal::kitty::ScanItem::Command(raw) => {
                        if let Some(cmd) = PreparedCommand::direct_only(&raw) {
                            handler.execute_kitty(&cmd, runner_terminal::kitty::ExecMode::Mirror);
                        }
                    }
                    runner_terminal::kitty::ScanItem::Discarded => {}
                }
            }
        }

        let mirror_id = 777_777u64;
        let viewers = Arc::new(AtomicUsize::new(1));
        let img1 = graphics.primary.image_by_id(1).unwrap().clone();
        store.on_graphics_updated(
            mirror_id,
            1,
            &viewers,
            &[(img1.id, img1.generation)],
            &[img1],
        );

        let pane_bounds = Bounds::new(point(px(10.), px(20.)), size(px(320.), px(160.)));
        let (bands, missing) = store.prepare_paint_bands(
            mirror_id,
            false,
            &graphics,
            0,
            pane_bounds,
            px(8.),
            px(16.),
            1.0,
            cell,
        );
        assert!(missing.is_empty());
        let ids: Vec<(u8, u32)> = bands[2].iter().map(|item| item.placement_order).collect();
        assert_eq!(
            ids,
            vec![(1, 1), (1, 2)],
            "only parent p=1 and overlapping 2x2 child p=2 at (-1,-1) should survive culling"
        );
        let child = &bands[2][1];
        assert_eq!(child.dest_bounds.origin, point(px(2.), px(4.)));
        assert_eq!(child.dest_bounds.size, size(px(16.), px(32.)));

        store.on_mirror_dropped(mirror_id);
        cx.run_until_parked();
    }

    #[test]
    fn explicit_and_inverse_cell_backgrounds_matching_pane_rgb_paint_over_band_zero() {
        let mut cx = TestAppContext::single();
        let (_host, mirror, tx, ack) = mirror_named("band-zero-bg", TerminalController::Desktop);
        let _store = cx.update(|cx| TerminalTextureCache::attach_mirror(&mirror, cx));
        let _view = mirror.view();

        let interaction = cx.new(|_| TerminalInteraction::new(Arc::clone(&mirror)));
        let input = cx.new(|_| TerminalInput::new(Arc::clone(&mirror)));
        let window = cx.add_window(|_, cx| GraphicsPaneHost {
            terminal: Some(Arc::clone(&mirror)),
            interaction: Some(interaction.clone()),
            input: Some(input.clone()),
            focus: cx.focus_handle(),
            font_size: 14.,
            line_height_factor: 1.4,
        });
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.simulate_resize(gpui::size(px(400.), px(260.)));
        visual.update(|window, _| window.set_scale_factor(1.0));
        visual.refresh().unwrap();

        let (cell_w, line_h) = visual.update(|window, cx| {
            let font = crate::app_settings::AppSettings::default()
                .terminal_font_family
                .font(cx);
            let font_id = window.text_system().resolve_font(&font);
            let cw = window.text_system().em_advance(font_id, px(14.)).unwrap();
            let lh = px(crate::terminal::element::row_height(14., 1.4));
            (cw, lh)
        });
        let pane_bounds = visual.debug_bounds("GRAPHICS_PANE").unwrap();
        let bg = runner_terminal::palette::RUNNER.background;

        // Col 0: explicit truecolor background equal to RUNNER.background (`E`).
        // Col 1: default uncolored background (`D`) — must stay transparent over Band 0.
        // Col 2: inverse cell whose foreground equals RUNNER.background (`I`) — must paint opaque.
        let mut stream = format!(
            "\x1b[48;2;{};{};{}mE\x1b[0mD\x1b[7;38;2;{};{};{}mI\x1b[0m",
            bg.r, bg.g, bg.b, bg.r, bg.g, bg.b
        )
        .into_bytes();
        stream.extend_from_slice(&kitty_rgba_command(
            "a=T,f=32,s=40,v=20,i=1,p=1,z=-1073741825,C=1,q=2",
            &solid_rgba(40, 20, [255, 0, 0, 255]),
        ));
        send_and_wait(
            &tx,
            &ack,
            TerminalFrame::Output {
                seq: 1,
                bytes: stream.into(),
            },
        );

        visual.update(|window, cx| {
            cx.set_global(PaintTrace::default());
            cx.set_global(BackgroundTrace::default());
            window.refresh();
        });
        visual.run_until_parked();

        let trace = visual.update(|_, cx| cx.global::<PaintTrace>().0.clone());
        let (pane_bg_initial, bg_quads) = visual.update(|_, cx| {
            let b = cx.global::<BackgroundTrace>();
            (b.pane_background, b.cell_quads.clone())
        });
        let band0_pos = trace
            .iter()
            .position(|e| matches!(e, PaintEvent::Image { band: 0, .. }))
            .expect("Band 0 image must paint");
        let cell_bg_pos = trace
            .iter()
            .position(|e| *e == PaintEvent::CellBackgrounds)
            .expect("explicit cell backgrounds must paint over Band 0");
        assert!(
            band0_pos < cell_bg_pos,
            "Band 0 image must paint behind explicit cell backgrounds"
        );
        assert_eq!(
            pane_bg_initial,
            Some(
                gpui::rgba(
                    ((bg.r as u32) << 24) | ((bg.g as u32) << 16) | ((bg.b as u32) << 8) | 0xff
                )
                .into()
            )
        );
        assert_eq!(
            bg_quads.len(),
            2,
            "only explicit col 0 and inverse col 2 should emit quads; col 1 default must stay transparent: {bg_quads:?}"
        );
        let col0_x = pane_bounds.left();
        let col2_x = pane_bounds.left() + cell_w * 2.0;
        assert!((f32::from(bg_quads[0].0.left()) - f32::from(col0_x)).abs() < 1e-2);
        assert!((f32::from(bg_quads[0].0.size.width) - f32::from(cell_w)).abs() < 1e-2);
        assert!((f32::from(bg_quads[0].0.size.height) - f32::from(line_h)).abs() < 1e-2);
        assert!((f32::from(bg_quads[1].0.left()) - f32::from(col2_x)).abs() < 1e-2);
        assert!((f32::from(bg_quads[1].0.size.width) - f32::from(cell_w)).abs() < 1e-2);

        // OSC 11 overrides the terminal's default background to rgb:12/34/56.
        // The pane background underneath Band 0 must adopt rgb(0x12, 0x34, 0x56),
        // default cells must remain transparent so Band 0 stays visible, and
        // explicit/inverse cells matching the OSC 11 RGB must still paint opaque.
        let osc11_hsla: gpui::Hsla = gpui::rgba(0x1234_56ff).into();
        let osc11_stream = concat!(
            "\x1b]11;rgb:12/34/56\x07",
            "\x1b[H\x1b[K",
            "\x1b[48;2;18;52;86mE\x1b[0mD\x1b[7;38;2;18;52;86mI\x1b[0m",
        );
        send_and_wait(
            &tx,
            &ack,
            TerminalFrame::Output {
                seq: 2,
                bytes: osc11_stream.as_bytes().into(),
            },
        );
        visual.update(|window, cx| {
            cx.set_global(PaintTrace::default());
            cx.set_global(BackgroundTrace::default());
            window.refresh();
        });
        visual.run_until_parked();
        let trace_osc11 = visual.update(|_, cx| cx.global::<PaintTrace>().0.clone());
        let (pane_bg_osc11, quads_osc11) = visual.update(|_, cx| {
            let b = cx.global::<BackgroundTrace>();
            (b.pane_background, b.cell_quads.clone())
        });
        assert!(
            trace_osc11
                .iter()
                .any(|e| matches!(e, PaintEvent::Image { band: 0, .. })),
            "Band 0 image must still paint under OSC 11"
        );
        assert_eq!(
            pane_bg_osc11,
            Some(osc11_hsla),
            "OSC 11 effective default background must paint underneath Band 0"
        );
        assert_eq!(
            quads_osc11.len(),
            2,
            "default cells under OSC 11 must stay transparent over Band 0 while explicit col 0 and inverse col 2 paint opaque: {quads_osc11:?}"
        );
        assert!((f32::from(quads_osc11[0].0.left()) - f32::from(col0_x)).abs() < 1e-2);
        assert_eq!(quads_osc11[0].1, osc11_hsla);
        assert!((f32::from(quads_osc11[1].0.left()) - f32::from(col2_x)).abs() < 1e-2);
        assert_eq!(quads_osc11[1].1, osc11_hsla);

        // OSC 111 resets the default background back to the theme color: pane
        // background reverts, default cells remain transparent over Band 0, and
        // explicit/inverse cells matching the theme RGB paint opaque again.
        let reset_stream = format!(
            "\x1b]111\x07\x1b[H\x1b[K\x1b[48;2;{};{};{}mE\x1b[0mD\x1b[7;38;2;{};{};{}mI\x1b[0m",
            bg.r, bg.g, bg.b, bg.r, bg.g, bg.b
        );
        send_and_wait(
            &tx,
            &ack,
            TerminalFrame::Output {
                seq: 3,
                bytes: reset_stream.into_bytes().into(),
            },
        );
        visual.update(|window, cx| {
            cx.set_global(BackgroundTrace::default());
            window.refresh();
        });
        visual.run_until_parked();
        let (pane_bg_reset, quads_reset) = visual.update(|_, cx| {
            let b = cx.global::<BackgroundTrace>();
            (b.pane_background, b.cell_quads.clone())
        });
        assert_eq!(
            pane_bg_reset, pane_bg_initial,
            "OSC 111 must restore theme default pane background underneath Band 0"
        );
        assert_eq!(
            quads_reset.len(),
            2,
            "after OSC 111 reset, only explicit col 0 and inverse col 2 should paint over Band 0: {quads_reset:?}"
        );

        // Once the Band 0 image is deleted, matching-pane-RGB backgrounds emit no redundant quads.
        send_and_wait(
            &tx,
            &ack,
            TerminalFrame::Output {
                seq: 4,
                bytes: b"\x1b_Ga=d,d=I,i=1,q=2\x1b\\".into(),
            },
        );
        visual.update(|window, cx| {
            cx.set_global(BackgroundTrace::default());
            window.refresh();
        });
        visual.run_until_parked();
        let quads_after_delete =
            visual.update(|_, cx| cx.global::<BackgroundTrace>().cell_quads.clone());
        assert!(
            quads_after_delete.is_empty(),
            "without a Band 0 image, matching-pane-RGB cells must not emit extra quads"
        );
    }
}
