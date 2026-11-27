//! Kitty graphics command execution (`exec.rs`), following Ghostty
//! `graphics_exec.zig`: replies, normalized stream output, placement creation,
//! cursor advancement, and deletes.

use alacritty_terminal::grid::Dimensions as _;
use alacritty_terminal::index::Column;
use alacritty_terminal::term::Term;

use super::command::{
    encode_delete_image_by_id, encode_delete_placements_by_id, encode_normalized_delete,
    encode_normalized_display, encode_normalized_transmission, Command, Control, CursorMovement,
    DeleteAction, Display, Medium, Quiet, Response,
};
use super::image::{ImageError, LoadingImage, StoredImage};
use super::media::{medium_supported, PreparedCommand};
use super::storage::{
    CellSize, EvictedImage, ImageStorage, PinLocation, Placement, PlacementId, PlacementKey,
    PlacementLocation, RelativeLocation,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecMode {
    /// Authoritative model in `runnerd`: emits replies and normalized commands.
    Authoritative,
    /// Mirror in `runner-app`: applies normalized commands, emits no replies or
    /// normalized output.
    Mirror,
}

#[derive(Debug, Default)]
pub struct ExecResult {
    /// Reply to send back to the PTY, if any.
    pub reply: Option<String>,
    /// Normalized commands to publish to subscribers (each entry is one complete
    /// `ESC _ G ... ESC \` sequence so the frame splitter can place boundaries
    /// between commands).
    pub normalized: Vec<Vec<u8>>,
}

/// Host interface implemented by the terminal stream handler so `execute` can
/// mutate the active screen's storage and advance the cursor via `linefeed()`
/// (which scrolls both the grid and placements consistently).
pub trait ExecuteHost {
    fn storage(&self) -> &ImageStorage;
    fn storage_mut(&mut self) -> &mut ImageStorage;
    fn term_cols(&self) -> usize;
    fn term_rows(&self) -> usize;
    fn term_cursor(&self) -> (usize, i32);
    fn term_scroll_region(&self) -> (i32, i32);
    fn term_linefeed(&mut self);
    fn term_set_cursor_col(&mut self, col: usize);
}

/// Execute one prepared Kitty graphics command against the active screen's
/// storage and cursor host.
pub fn execute<H: ExecuteHost>(
    host: &mut H,
    cell: CellSize,
    prepared: &PreparedCommand,
    mode: ExecMode,
) -> ExecResult {
    let cmd = &prepared.command;
    let mut result = ExecResult::default();
    let mut quiet = cmd.quiet;

    // `i` and `I` are mutually exclusive for every action, checked before any
    // storage mutation.
    let ids = cmd.control.identifiers();
    if ids.image_id > 0 && ids.image_number > 0 {
        let resp = Response {
            id: ids.image_id,
            image_number: ids.image_number,
            placement_id: ids.placement_id,
            message: "EINVAL: image ID and number are mutually exclusive",
        };
        if mode == ExecMode::Authoritative {
            result.reply = apply_quiet(resp, quiet);
        }
        return result;
    }

    let response = match &cmd.control {
        Control::Query(_) => execute_query(host.storage(), prepared),
        Control::Display(d) => {
            let (resp, normalized) = execute_display(host, cell, d, mode);
            if let Some(norm) = normalized {
                result.normalized.push(norm);
            }
            Some(resp)
        }
        Control::Delete(d) => {
            execute_delete(host, cell, &d.action, mode, &mut result.normalized);
            None
        }
        Control::Transmit(_) | Control::TransmitAndDisplay { .. } => {
            let t = cmd.control.transmission().unwrap();
            // Non-direct transmissions abandon any in-progress chunked load.
            if host.storage().loading.is_some() && t.medium != Medium::Direct {
                host.storage_mut().loading = None;
            }
            // Continuation chunks inherit `q` from the first chunk unless they
            // explicitly set `q >= 1`.
            if let Some(loading) = host.storage_mut().loading.as_mut() {
                match cmd.quiet {
                    Quiet::No => quiet = loading.quiet,
                    Quiet::Ok | Quiet::Failures => {
                        loading.quiet = cmd.quiet;
                    }
                }
            }
            execute_transmit(host, cell, prepared, mode, &mut result.normalized)
        }
        Control::TransmitAnimationFrame(_)
        | Control::ControlAnimation(_)
        | Control::ComposeAnimation(_) => {
            // Animation commands are Phase 2; parsed cleanly and ignored in Phase 1.
            None
        }
    };

    if mode == ExecMode::Authoritative {
        if let Some(resp) = response {
            result.reply = apply_quiet(resp, quiet);
        }
    }
    result
}

fn apply_quiet(resp: Response, quiet: Quiet) -> Option<String> {
    if resp.empty() {
        return None;
    }
    match quiet {
        Quiet::No => resp.encode(),
        Quiet::Ok => {
            if resp.ok() {
                None
            } else {
                resp.encode()
            }
        }
        Quiet::Failures => None,
    }
}

fn execute_query(storage: &ImageStorage, prepared: &PreparedCommand) -> Option<Response> {
    let cmd = &prepared.command;
    let t = match &cmd.control {
        Control::Query(t) => t,
        _ => unreachable!(),
    };
    if t.image_id == 0 {
        return Some(Response {
            message: "EINVAL: image ID required",
            ..Default::default()
        });
    }
    let mut resp = Response {
        id: t.image_id,
        image_number: t.image_number,
        placement_id: t.placement_id,
        message: "OK",
    };
    let payload = match resolve_initial_payload(cmd, prepared.media.as_ref()) {
        Ok(bytes) => bytes,
        Err(err) => {
            resp.message = err.protocol_message();
            return Some(resp);
        }
    };
    let loading = match LoadingImage::new(cmd, payload) {
        Ok(loading) => loading,
        Err(err) => {
            resp.message = err.protocol_message();
            return Some(resp);
        }
    };
    if let Err(err) = loading.complete(storage.total_limit()) {
        resp.message = err.protocol_message();
    }
    Some(resp)
}

fn execute_delete<H: ExecuteHost>(
    host: &mut H,
    cell: CellSize,
    action: &DeleteAction,
    mode: ExecMode,
    normalized: &mut Vec<Vec<u8>>,
) {
    // Every delete command aborts any incomplete chunked upload.
    host.storage_mut().loading = None;

    // Resolve `n`/`N` (newest by image number) to concrete `i`/`I` before
    // deleting so mirrors never see image numbers.
    let norm_action = match *action {
        DeleteAction::Newest {
            delete_images,
            image_number,
            placement_id,
        } => host
            .storage()
            .image_by_number(image_number)
            .map(|img| DeleteAction::Id {
                delete_images,
                image_id: img.id,
                placement_id,
            }),
        DeleteAction::AnimationFrames { .. } => None,
        other => Some(other),
    };

    let cursor = host.term_cursor();
    let rows = host.term_rows();
    let cols = host.term_cols();
    host.storage_mut().delete(*action, cursor, rows, cols, cell);

    if mode == ExecMode::Authoritative {
        if let Some(norm) = norm_action {
            let mut buf = Vec::new();
            encode_normalized_delete(&mut buf, &norm);
            normalized.push(buf);
        }
    }
}

fn execute_transmit<H: ExecuteHost>(
    host: &mut H,
    cell: CellSize,
    prepared: &PreparedCommand,
    mode: ExecMode,
    normalized: &mut Vec<Vec<u8>>,
) -> Option<Response> {
    let cmd = &prepared.command;
    let t = cmd.control.transmission().unwrap();

    let mut resp = if let Some(loading) = &host.storage().loading {
        loading.response
    } else {
        Response {
            id: t.image_id,
            image_number: t.image_number,
            placement_id: t.placement_id,
            message: "OK",
        }
    };

    let (stored, display_opt) = match load_and_add_image(host, prepared, mode, normalized) {
        Ok(LoadOutcome::More) => return None,
        Ok(LoadOutcome::Complete { image, display }) => (image, display),
        Err(err) => {
            resp.message = err.protocol_message();
            return Some(resp);
        }
    };

    let implicit_id = stored.implicit_id;
    let stored_id = stored.id;
    let stored_format = stored.wire_format;
    let stored_width = stored.width;
    let stored_height = stored.height;
    let stored_compression = stored.compression;
    let stored_transient = stored.transient;
    let stored_data = stored.encoded_data;

    let display_succeeded = if let Some(mut d) = display_opt {
        let saved_number = d.image_number;
        d.image_id = stored_id;
        d.image_number = 0;
        let (mut disp_resp, _) = execute_display(host, cell, &d, ExecMode::Mirror);
        disp_resp.image_number = saved_number;
        let ok = disp_resp.ok();
        resp = disp_resp;
        ok
    } else {
        false
    };

    if mode == ExecMode::Authoritative {
        let mut buf = Vec::new();
        let norm_display = if display_succeeded {
            display_opt.as_ref()
        } else {
            None
        };
        encode_normalized_transmission(
            &mut buf,
            stored_id,
            stored_format,
            stored_width,
            stored_height,
            stored_compression,
            stored_transient,
            norm_display,
            &stored_data,
        );
        normalized.push(buf);
    }

    if implicit_id {
        return None;
    }

    resp.id = stored_id;
    Some(resp)
}

enum LoadOutcome {
    More,
    Complete {
        image: StoredImage,
        display: Option<Display>,
    },
}

fn load_and_add_image<H: ExecuteHost>(
    host: &mut H,
    prepared: &PreparedCommand,
    mode: ExecMode,
    normalized: &mut Vec<Vec<u8>>,
) -> Result<LoadOutcome, ImageError> {
    let cmd = &prepared.command;
    let t = cmd.control.transmission().unwrap();
    let storage = host.storage_mut();

    let mut deleted_existing: Option<u32> = None;
    let mut self_deleted_other = Vec::new();
    let mut self_survived_other = Vec::new();
    let mut loading = if let Some(mut loading) = storage.loading.take() {
        loading.add_data(&cmd.data)?;
        if t.more_chunks {
            storage.loading = Some(loading);
            return Ok(LoadOutcome::More);
        }
        loading
    } else {
        // Reusing an explicit image ID deletes the old image and all its
        // placements when the new transmission begins, before it completes or fails.
        // Succeeding single-command replacements never emit a separate
        // `a=d,d=I,i=<self>` because the mirror's own replacement deletes the
        // old image atomically; failed replacements and chunked starts (`m=1`)
        // still emit it.
        if t.image_id > 0 {
            let had_existing = storage.contains_image(t.image_id);
            let (deleted_other, survived_other) = storage.delete_image_for_replacement(t.image_id);
            if mode == ExecMode::Authoritative && had_existing {
                deleted_existing = Some(t.image_id);
                self_deleted_other = deleted_other;
                self_survived_other = survived_other;
            }
        }

        let start = (|| -> Result<LoadingImage, ImageError> {
            let payload = resolve_initial_payload(cmd, prepared.media.as_ref())?;
            let mut loading = LoadingImage::new(cmd, payload)?;
            if loading.id == 0 {
                if loading.number > 0 {
                    loading.id = storage.next_image_id(false);
                } else {
                    loading.id = storage.next_image_id(true);
                    loading.implicit_id = true;
                }
            }
            Ok(loading)
        })();
        let loading = match start {
            Ok(loading) => loading,
            Err(err) => {
                if let Some(id) = deleted_existing {
                    let mut buf = Vec::new();
                    encode_delete_image_by_id(&mut buf, id);
                    normalized.push(buf);
                }
                return Err(err);
            }
        };

        if t.more_chunks {
            if let Some(id) = deleted_existing {
                let mut buf = Vec::new();
                encode_delete_image_by_id(&mut buf, id);
                normalized.push(buf);
            }
            storage.loading = Some(loading);
            return Ok(LoadOutcome::More);
        }
        loading
    };

    let finish = (|| -> Result<LoadOutcome, ImageError> {
        let display = loading.display.take();
        let image = match mode {
            ExecMode::Authoritative => loading.complete(storage.total_limit())?,
            ExecMode::Mirror => loading.complete_normalized_mirror(storage.total_limit())?,
        };
        let evicted = storage.add_image(image.clone())?;
        if mode == ExecMode::Authoritative && !evicted.is_empty() {
            // Preserve authoritative orphan-image outcomes without ever
            // emitting a self pre-delete for `t.image_id`:
            // 1. Any child image that survived the replacement delete only
            //    because of placements under an evicted parent (and now sits
            //    retained with 0 placements) has its placements stripped
            //    lowercase first so the mirror's later replacement delete
            //    cannot free its image bytes.
            let retained_zero_placed: Vec<u32> = self_survived_other
                .into_iter()
                .filter(|&id| {
                    storage
                        .image_by_id(id)
                        .is_some_and(|img| img.placement_count == 0)
                })
                .collect();
            for &id in &retained_zero_placed {
                let mut lower = Vec::new();
                encode_delete_placements_by_id(&mut lower, id);
                normalized.push(lower);
            }
            emit_evictions(&evicted, normalized);
            // 2. If any lowercase delete above could orphan placements of a
            //    child image that runnerd already freed during the replacement
            //    delete, free that child image explicitly so the mirror does
            //    not retain it.
            if !retained_zero_placed.is_empty() || evicted.iter().any(|ev| ev.orphaned_other_image)
            {
                for id in self_deleted_other {
                    let mut buf = Vec::new();
                    encode_delete_image_by_id(&mut buf, id);
                    normalized.push(buf);
                }
            }
        }
        Ok(LoadOutcome::Complete { image, display })
    })();

    if finish.is_err() {
        if let Some(id) = deleted_existing {
            let mut buf = Vec::new();
            encode_delete_image_by_id(&mut buf, id);
            normalized.push(buf);
        }
    }
    finish
}

fn emit_evictions(evicted: &[EvictedImage], normalized: &mut Vec<Vec<u8>>) {
    for ev in evicted {
        if ev.orphaned_other_image {
            // Remove placements lowercase first so the mirror does not delete
            // another image whose relative placement was orphaned by eviction.
            let mut lower = Vec::new();
            encode_delete_placements_by_id(&mut lower, ev.id);
            normalized.push(lower);
        }
        let mut buf = Vec::new();
        encode_delete_image_by_id(&mut buf, ev.id);
        normalized.push(buf);
    }
}

fn resolve_initial_payload<'a>(
    cmd: &'a Command,
    media: Option<&'a Result<Vec<u8>, ImageError>>,
) -> Result<&'a [u8], ImageError> {
    let t = cmd.control.transmission().ok_or(ImageError::InvalidData)?;
    if t.format_unknown {
        return Err(ImageError::UnsupportedFormat);
    }
    if t.medium == Medium::Direct {
        return Ok(&cmd.data);
    }
    if !medium_supported(t.medium) {
        return Err(ImageError::UnsupportedMedium);
    }
    match media {
        Some(Ok(bytes)) => Ok(bytes.as_slice()),
        Some(Err(err)) => Err(*err),
        // No forwarder resolved the local medium (e.g. direct model feed without
        // media resolution).
        None => Err(ImageError::UnsupportedMedium),
    }
}

fn execute_display<H: ExecuteHost>(
    host: &mut H,
    cell: CellSize,
    d: &Display,
    mode: ExecMode,
) -> (Response, Option<Vec<u8>>) {
    if d.image_id == 0 && d.image_number == 0 {
        return (
            Response {
                message: "EINVAL: image ID or number required",
                ..Default::default()
            },
            None,
        );
    }
    let mut resp = Response {
        id: d.image_id,
        image_number: d.image_number,
        placement_id: d.placement_id,
        message: "OK",
    };

    if d.virtual_placement && d.parent_id > 0 {
        resp.message = "EINVAL: virtual placement cannot refer to a parent";
        return (resp, None);
    }

    let img_meta = if d.image_id != 0 {
        host.storage()
            .image_by_id(d.image_id)
            .map(|img| (img.id, img.width, img.height))
    } else {
        host.storage()
            .image_by_number(d.image_number)
            .map(|img| (img.id, img.width, img.height))
    };
    let Some((img_id, img_width, img_height)) = img_meta else {
        resp.message = "ENOENT: image not found";
        return (resp, None);
    };
    resp.id = img_id;

    let location = if d.virtual_placement {
        PlacementLocation::Virtual
    } else if d.parent_id == 0 {
        let (col, line) = host.term_cursor();
        PlacementLocation::Pin(PinLocation { line, column: col })
    } else {
        let child = (d.placement_id > 0).then_some(PlacementKey {
            image_id: img_id,
            placement_id: PlacementId::External(d.placement_id),
        });
        match host
            .storage()
            .resolve_parent(child, d.parent_id, d.parent_placement_id)
        {
            Ok(parent) => PlacementLocation::Relative(RelativeLocation {
                parent,
                horizontal_offset: d.horizontal_offset,
                vertical_offset: d.vertical_offset,
            }),
            Err(err) => {
                resp.message = err.protocol_message();
                return (resp, None);
            }
        }
    };

    let mut placement = Placement {
        location,
        x_offset: d.x_offset,
        y_offset: d.y_offset,
        source_x: d.x,
        source_y: d.y,
        source_width: d.width,
        source_height: d.height,
        columns: d.columns,
        rows: d.rows,
        z: d.z,
    };
    let (clamped_x, clamped_y) = placement.cell_offset(cell);
    if cell.width > 0 {
        placement.x_offset = clamped_x;
    }
    if cell.height > 0 {
        placement.y_offset = clamped_y;
    }

    host.storage_mut()
        .add_placement(img_id, d.placement_id, placement);

    // Move cursor for pin placements with `C=0` (relative and virtual never move it).
    if let PlacementLocation::Pin(pin) = placement.location {
        if d.cursor_movement == CursorMovement::After {
            let (grid_cols, grid_rows) = placement.grid_size(img_width, img_height, cell);
            let screen_cols = host.term_cols();
            let screen_rows = host.term_rows();
            let target_x = pin.column.saturating_add(grid_cols as usize);
            let wraps = target_x >= screen_cols;
            let requested_rows = (grid_rows as usize)
                .saturating_sub(1)
                .saturating_add(usize::from(wraps));

            let (region_top, region_bottom) = host.term_scroll_region();
            let (_, cursor_y) = host.term_cursor();
            let rows_before_scroll = if cursor_y >= region_top && cursor_y <= region_bottom {
                (region_bottom - cursor_y) as usize
            } else {
                0
            };
            let rows_to_move = requested_rows.min(rows_before_scroll.saturating_add(screen_rows));
            for _ in 0..rows_to_move {
                host.term_linefeed();
            }
            let final_col = if wraps {
                0
            } else {
                target_x.min(screen_cols.saturating_sub(1))
            };
            host.term_set_cursor_col(final_col);
        }
    }

    let normalized = if mode == ExecMode::Authoritative {
        let mut buf = Vec::new();
        encode_normalized_display(&mut buf, img_id, d);
        Some(buf)
    } else {
        None
    };

    (resp, normalized)
}

/// Helper to set cursor column on a `Term` without disturbing line under ORIGIN mode.
pub fn set_term_cursor_col<T>(term: &mut Term<T>, col: usize) {
    let clamped = col.min(term.columns().saturating_sub(1));
    let grid = term.grid_mut();
    grid.cursor.point.column = Column(clamped);
    grid.cursor.input_needs_wrap = false;
}
