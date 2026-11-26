//! Serializing Kitty graphics images and placements into VT snapshots within
//! the `wire::MAX_FRAME` budget.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use super::command::{
    encode_normalized_transmission, normalized_transmission_len, write_display_keys,
    CursorMovement, Display,
};
use super::storage::{
    CellSize, ImageStorage, Placement, PlacementKey, PlacementLocation, TerminalGraphics,
    PARENT_CHAIN_LIMIT,
};

/// Upper bound on bytes graphics may occupy inside a snapshot (48 MiB).
pub const MAX_SNAPSHOT_GRAPHICS_BYTES: usize = 48 * 1024 * 1024;

/// Safety reserve kept below `wire::MAX_FRAME` when computing the graphics budget.
pub const SNAPSHOT_FRAME_HEADROOM: usize = 1024 * 1024;

/// Bytes written once per occupied row in `emit_row_placements`: `\x1b7` + `\x1b8`.
const ROW_WRAPPER_BYTES: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ScreenKind {
    Primary,
    Alt,
}

/// Precomputed plan of which images and placements to emit in a snapshot so
/// total serialized bytes stay within `wire::MAX_FRAME` without base64-encoding
/// dropped images.
#[derive(Default)]
pub struct SnapshotGraphicsPlan {
    primary: ScreenPlan,
    alt: ScreenPlan,
}

#[derive(Default)]
struct ScreenPlan {
    /// Image IDs to transmit at the start of the screen, oldest generation first.
    image_ids: Vec<u32>,
    /// Virtual placements (and their relative subtrees) to emit right after
    /// transmitting images at the start of the screen.
    virtual_roots: Vec<PlacementKey>,
    /// Pin placements anchored on each replayed line `line` (after clipping any
    /// placement anchored above `first_line` down to `first_line`).
    by_line: HashMap<i32, Vec<(PlacementKey, Placement)>>,
    /// Relative children keyed by their parent placement key.
    children: HashMap<PlacementKey, Vec<(PlacementKey, Placement)>>,
}

impl SnapshotGraphicsPlan {
    pub fn is_empty(&self) -> bool {
        self.primary.image_ids.is_empty() && self.alt.image_ids.is_empty()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn build(
        graphics: &TerminalGraphics,
        alt_active: bool,
        primary_first_line: i32,
        primary_screen_lines: usize,
        alt_screen_lines: usize,
        screen_cols: usize,
        cell: CellSize,
        budget: usize,
    ) -> Self {
        if budget == 0 || (graphics.primary.is_empty() && (!alt_active || graphics.alt.is_empty()))
        {
            return Self::default();
        }

        // Gather all candidate images across primary (and active alt screen),
        // along with reachability tracking for relative placements so planning
        // and serialization agree to the exact byte.
        // Keep-order: active visible > inactive primary visible > non-visible,
        // newest generation first (so LRU is dropped first, visible last).
        let mut candidates = Vec::new();
        let mut primary_rel = RelativeTracker::default();
        collect_candidates(
            ScreenKind::Primary,
            &graphics.primary,
            !alt_active,
            primary_first_line,
            primary_screen_lines,
            screen_cols,
            cell,
            &mut candidates,
            &mut primary_rel,
        );
        let mut alt_rel = RelativeTracker::default();
        if alt_active {
            collect_candidates(
                ScreenKind::Alt,
                &graphics.alt,
                true,
                0,
                alt_screen_lines,
                screen_cols,
                cell,
                &mut candidates,
                &mut alt_rel,
            );
        }

        // Higher tier kept first; within a tier, higher generation (most recently used) kept first.
        candidates.sort_by(|a, b| {
            b.tier
                .cmp(&a.tier)
                .then_with(|| b.generation.cmp(&a.generation))
                .then_with(|| b.image_id.cmp(&a.image_id))
        });

        let mut kept_primary = HashSet::new();
        let mut kept_alt = HashSet::new();
        let mut occupied_rows: HashSet<(ScreenKind, i32)> = HashSet::new();
        let mut used = 0usize;
        for c in candidates {
            let rel_tracker = match c.screen {
                ScreenKind::Primary => &mut primary_rel,
                ScreenKind::Alt => &mut alt_rel,
            };
            let new_rows = c
                .pin_rows
                .iter()
                .filter(|&&row| !occupied_rows.contains(&(c.screen, row)))
                .count();
            let newly_reachable_rel = rel_tracker.newly_reachable_bytes(c.image_id);
            let delta = c
                .base_cost
                .saturating_add(new_rows.saturating_mul(ROW_WRAPPER_BYTES))
                .saturating_add(newly_reachable_rel);
            if used.saturating_add(delta) <= budget {
                used += delta;
                for row in c.pin_rows {
                    occupied_rows.insert((c.screen, row));
                }
                rel_tracker.mark_image_kept(c.image_id);
                match c.screen {
                    ScreenKind::Primary => {
                        kept_primary.insert(c.image_id);
                    }
                    ScreenKind::Alt => {
                        kept_alt.insert(c.image_id);
                    }
                }
            }
        }

        let primary = build_screen_plan(
            &graphics.primary,
            &kept_primary,
            primary_first_line,
            primary_screen_lines,
            cell,
        );
        let alt = if alt_active {
            build_screen_plan(&graphics.alt, &kept_alt, 0, alt_screen_lines, cell)
        } else {
            ScreenPlan::default()
        };

        Self { primary, alt }
    }

    pub fn emit_screen_images(&self, is_alt: bool, storage: &ImageStorage, out: &mut String) {
        let plan = if is_alt { &self.alt } else { &self.primary };
        let mut raw = Vec::new();
        for &id in &plan.image_ids {
            let Some(img) = storage.image_by_id(id) else {
                continue;
            };
            raw.clear();
            encode_normalized_transmission(
                &mut raw,
                img.id,
                img.wire_format,
                img.width,
                img.height,
                img.compression,
                img.transient,
                None,
                &img.encoded_data,
            );
            // Normalized transmission is pure ASCII.
            out.push_str(std::str::from_utf8(&raw).expect("ASCII kitty sequence"));
        }
        for &root_key in &plan.virtual_roots {
            if let Some(&placement) = storage.placements().get(&root_key) {
                emit_placement_command(out, root_key, &placement);
                emit_children(out, plan, root_key, 0);
            }
        }
    }

    pub fn emit_row_placements(&self, is_alt: bool, line: i32, out: &mut String) {
        let plan = if is_alt { &self.alt } else { &self.primary };
        let Some(entries) = plan.by_line.get(&line) else {
            return;
        };
        if entries.is_empty() {
            return;
        }
        // Save and restore cursor so pending-wrap state survives placement replay.
        out.push_str("\x1b7");
        for &(key, placement) in entries {
            if let PlacementLocation::Pin(pin) = placement.location {
                let _ = write!(out, "\x1b[{}G", pin.column + 1);
                emit_placement_command(out, key, &placement);
                emit_children(out, plan, key, 0);
            }
        }
        out.push_str("\x1b8");
    }
}

struct RelativeEntry {
    bytes: usize,
    missing_images: u8,
}

/// Tracks relative placements that reach an emittable root within
/// `PARENT_CHAIN_LIMIT` links so their exact serialized bytes are charged once
/// when the final missing image in their ancestor chain is retained.
#[derive(Default)]
struct RelativeTracker {
    entries: Vec<RelativeEntry>,
    by_image: HashMap<u32, Vec<usize>>,
}

impl RelativeTracker {
    fn register(&mut self, required_images: &[u32], bytes: usize) {
        let idx = self.entries.len();
        self.entries.push(RelativeEntry {
            bytes,
            missing_images: required_images.len() as u8,
        });
        for &image_id in required_images {
            self.by_image.entry(image_id).or_default().push(idx);
        }
    }

    fn newly_reachable_bytes(&self, image_id: u32) -> usize {
        let Some(indices) = self.by_image.get(&image_id) else {
            return 0;
        };
        let mut sum = 0usize;
        for &idx in indices {
            if self.entries[idx].missing_images == 1 {
                sum = sum.saturating_add(self.entries[idx].bytes);
            }
        }
        sum
    }

    fn mark_image_kept(&mut self, image_id: u32) {
        let Some(indices) = self.by_image.get(&image_id) else {
            return;
        };
        for &idx in indices {
            self.entries[idx].missing_images = self.entries[idx].missing_images.saturating_sub(1);
        }
    }
}

/// Walk a relative placement's ancestor chain up to `PARENT_CHAIN_LIMIT` links.
/// Returns the deduplicated image IDs required for the placement to be emitted
/// (its own image plus every ancestor's image up to the root) when the chain
/// terminates at an emittable root, or `None` if unreachable.
fn reachable_chain_images(
    storage: &ImageStorage,
    emittable_roots: &HashSet<PlacementKey>,
    key: PlacementKey,
    mut parent: PlacementKey,
) -> Option<([u32; PARENT_CHAIN_LIMIT + 1], usize)> {
    let mut req = [0u32; PARENT_CHAIN_LIMIT + 1];
    req[0] = key.image_id;
    let mut req_len = 1usize;
    let mut depth = 1usize;
    loop {
        let p = storage.placements().get(&parent)?;
        if !req[..req_len].contains(&parent.image_id) {
            req[req_len] = parent.image_id;
            req_len += 1;
        }
        match p.location {
            PlacementLocation::Relative(parent_rel) => {
                if depth >= PARENT_CHAIN_LIMIT {
                    return None;
                }
                depth += 1;
                parent = parent_rel.parent;
            }
            PlacementLocation::Pin(_) | PlacementLocation::Virtual => {
                return emittable_roots.contains(&parent).then_some((req, req_len));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_candidates(
    screen: ScreenKind,
    storage: &ImageStorage,
    is_active_screen: bool,
    first_line: i32,
    screen_lines: usize,
    screen_cols: usize,
    cell: CellSize,
    out: &mut Vec<super::snapshot::Candidate>,
    rel_tracker: &mut RelativeTracker,
) {
    let mut has_visible: HashSet<u32> = HashSet::new();
    let mut placement_bytes: HashMap<u32, usize> = HashMap::new();
    let mut pin_rows: HashMap<u32, HashSet<i32>> = HashMap::new();
    let mut emittable_roots: HashSet<PlacementKey> = HashSet::new();

    // First pass: collect emittable root placements (Pin and Virtual) and
    // measure their exact bytes (clipped crop keys + per-placement `\x1b[colG`).
    // The shared `\x1b7` / `\x1b8` row wrapper is charged once per occupied row
    // during candidate selection in `SnapshotGraphicsPlan::build`.
    for (&key, placement) in storage.placements() {
        match placement.location {
            PlacementLocation::Pin(pin) => {
                if i64::from(pin.line) >= screen_lines as i64 {
                    continue;
                }
                if let Some(adjusted) =
                    clipped_to_first_line(storage, key.image_id, *placement, first_line, cell)
                {
                    emittable_roots.insert(key);
                    let adjusted_line = match adjusted.location {
                        PlacementLocation::Pin(p) => p.line,
                        _ => unreachable!(),
                    };
                    pin_rows
                        .entry(key.image_id)
                        .or_default()
                        .insert(adjusted_line);
                    *placement_bytes.entry(key.image_id).or_default() +=
                        exact_pin_placement_bytes(key, &adjusted);
                }
                let visible = if pin.line >= 0 {
                    true
                } else {
                    storage
                        .image_by_id(key.image_id)
                        .and_then(|img| {
                            placement.cell_rect(img.width, img.height, cell, screen_cols)
                        })
                        .is_some_and(|rect| rect.bottom_line >= 0)
                };
                if visible {
                    has_visible.insert(key.image_id);
                }
            }
            PlacementLocation::Virtual => {
                emittable_roots.insert(key);
                *placement_bytes.entry(key.image_id).or_default() +=
                    exact_placement_command_bytes(key, placement);
            }
            PlacementLocation::Relative(_) => {}
        }
    }

    // Second pass: register only relative placements whose ancestor chain
    // reaches an emittable root within `PARENT_CHAIN_LIMIT` links.
    for (&key, placement) in storage.placements() {
        let PlacementLocation::Relative(rel) = placement.location else {
            continue;
        };
        let Some((req_images, req_len)) =
            reachable_chain_images(storage, &emittable_roots, key, rel.parent)
        else {
            continue;
        };
        let bytes = exact_placement_command_bytes(key, placement);
        rel_tracker.register(&req_images[..req_len], bytes);

        let visible = storage
            .resolved_signed_anchor(placement)
            .map(|anchor| {
                if anchor.column >= screen_cols as i64 || anchor.line >= screen_lines as i64 {
                    return false;
                }
                if anchor.column >= 0 && anchor.line >= 0 {
                    return true;
                }
                storage
                    .image_by_id(key.image_id)
                    .map(|img| {
                        let (cols, rows) = placement.grid_size(img.width, img.height, cell);
                        let horiz = anchor.column >= 0
                            || (cols > 0 && anchor.column.saturating_add(i64::from(cols) - 1) >= 0);
                        let vert = anchor.line >= 0
                            || (rows > 0 && anchor.line.saturating_add(i64::from(rows) - 1) >= 0);
                        horiz && vert
                    })
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if visible {
            has_visible.insert(key.image_id);
        }
    }

    for img in storage.images().values() {
        let visible = has_visible.contains(&img.id);
        let tier = match (is_active_screen, visible) {
            (true, true) => 3,
            (false, true) => 2,
            (true, false) => 1,
            (false, false) => 0,
        };
        let tx_cost = exact_transmission_bytes(img);
        let p_cost = placement_bytes.get(&img.id).copied().unwrap_or(0);
        let rows = pin_rows
            .remove(&img.id)
            .map(|set| set.into_iter().collect())
            .unwrap_or_default();
        out.push(Candidate {
            screen,
            image_id: img.id,
            tier,
            generation: img.generation,
            base_cost: tx_cost.saturating_add(p_cost),
            pin_rows: rows,
        });
    }
}

struct Candidate {
    screen: ScreenKind,
    image_id: u32,
    tier: u8,
    generation: u64,
    base_cost: usize,
    pin_rows: Vec<i32>,
}

fn exact_transmission_bytes(img: &super::image::StoredImage) -> usize {
    normalized_transmission_len(
        img.id,
        img.wire_format,
        img.width,
        img.height,
        img.compression,
        img.transient,
        None,
        img.encoded_data.len(),
    )
}

fn exact_pin_placement_bytes(key: PlacementKey, adjusted: &Placement) -> usize {
    let mut tmp = String::new();
    let col = match adjusted.location {
        PlacementLocation::Pin(pin) => pin.column,
        _ => 0,
    };
    let _ = write!(tmp, "\x1b[{}G", col + 1);
    emit_placement_command(&mut tmp, key, adjusted);
    tmp.len()
}

fn exact_placement_command_bytes(key: PlacementKey, placement: &Placement) -> usize {
    let mut tmp = String::new();
    emit_placement_command(&mut tmp, key, placement);
    tmp.len()
}

fn clipped_to_first_line(
    storage: &ImageStorage,
    image_id: u32,
    mut placement: Placement,
    first_line: i32,
    cell: CellSize,
) -> Option<Placement> {
    let PlacementLocation::Pin(pin) = placement.location else {
        return None;
    };
    if pin.line >= first_line {
        return Some(placement);
    }
    let img = storage.image_by_id(image_id)?;
    let (_, rows) = placement.grid_size(img.width, img.height, cell);
    let clip_i64 = i64::from(first_line) - i64::from(pin.line);
    if rows == 0 || clip_i64 >= i64::from(rows) {
        return None;
    }
    let clip = clip_i64 as u32;
    if !placement.clip_top(img.width, img.height, cell, clip, rows) {
        return None;
    }
    placement.location = PlacementLocation::Pin(super::storage::PinLocation {
        line: first_line,
        column: pin.column,
    });
    Some(placement)
}

fn build_screen_plan(
    storage: &ImageStorage,
    kept_images: &HashSet<u32>,
    first_line: i32,
    screen_lines: usize,
    cell: CellSize,
) -> ScreenPlan {
    let mut plan = ScreenPlan::default();
    if kept_images.is_empty() {
        return plan;
    }

    let mut image_ids: Vec<(u64, u32)> = kept_images
        .iter()
        .filter_map(|&id| storage.image_by_id(id).map(|img| (img.generation, id)))
        .collect();
    image_ids.sort_unstable();
    plan.image_ids = image_ids.into_iter().map(|(_, id)| id).collect();

    // Collect root placements (Pin and Virtual) whose image was kept.
    let mut emitted_depths: HashMap<PlacementKey, usize> = HashMap::new();
    for (&key, &placement) in storage.placements() {
        if !kept_images.contains(&key.image_id) {
            continue;
        }
        match placement.location {
            PlacementLocation::Pin(pin) => {
                if i64::from(pin.line) >= screen_lines as i64 {
                    continue;
                }
                let Some(adjusted) =
                    clipped_to_first_line(storage, key.image_id, placement, first_line, cell)
                else {
                    continue;
                };
                let line = match adjusted.location {
                    PlacementLocation::Pin(p) => p.line,
                    _ => unreachable!(),
                };
                plan.by_line.entry(line).or_default().push((key, adjusted));
                emitted_depths.insert(key, 0);
            }
            PlacementLocation::Virtual => {
                plan.virtual_roots.push(key);
                emitted_depths.insert(key, 0);
            }
            PlacementLocation::Relative(_) => {}
        }
    }

    // Sort root placements deterministically so snapshots are reproducible.
    plan.virtual_roots
        .sort_by_key(|k| (k.image_id, k.placement_id.external_id().unwrap_or(0)));
    for entries in plan.by_line.values_mut() {
        entries.sort_by_key(|(k, p)| {
            (
                p.z,
                k.image_id,
                matches!(k.placement_id, super::storage::PlacementId::Internal(_)),
                match k.placement_id {
                    super::storage::PlacementId::External(id)
                    | super::storage::PlacementId::Internal(id) => id,
                },
            )
        });
    }

    // Attach relative placements whose entire ancestor chain was kept, bounded
    // to `PARENT_CHAIN_LIMIT` links to stay consistent with `resolve_chain`.
    loop {
        let mut added = false;
        for (&key, &placement) in storage.placements() {
            if emitted_depths.contains_key(&key) || !kept_images.contains(&key.image_id) {
                continue;
            }
            if let PlacementLocation::Relative(rel) = placement.location {
                if let Some(&parent_depth) = emitted_depths.get(&rel.parent) {
                    if parent_depth < PARENT_CHAIN_LIMIT {
                        plan.children
                            .entry(rel.parent)
                            .or_default()
                            .push((key, placement));
                        emitted_depths.insert(key, parent_depth + 1);
                        added = true;
                    }
                }
            }
        }
        if !added {
            break;
        }
    }

    for children in plan.children.values_mut() {
        children.sort_by_key(|(k, _)| {
            (
                k.image_id,
                matches!(k.placement_id, super::storage::PlacementId::Internal(_)),
                match k.placement_id {
                    super::storage::PlacementId::External(id)
                    | super::storage::PlacementId::Internal(id) => id,
                },
            )
        });
    }

    plan
}

fn emit_children(out: &mut String, plan: &ScreenPlan, parent: PlacementKey, depth: usize) {
    if depth >= PARENT_CHAIN_LIMIT {
        return;
    }
    let Some(children) = plan.children.get(&parent) else {
        return;
    };
    for &(child_key, child_placement) in children {
        emit_placement_command(out, child_key, &child_placement);
        emit_children(out, plan, child_key, depth + 1);
    }
}

fn emit_placement_command(out: &mut String, key: PlacementKey, placement: &Placement) {
    let (virtual_placement, parent_id, parent_placement_id, horizontal_offset, vertical_offset) =
        match placement.location {
            PlacementLocation::Pin(_) => (false, 0, 0, 0, 0),
            PlacementLocation::Virtual => (true, 0, 0, 0, 0),
            PlacementLocation::Relative(rel) => (
                false,
                rel.parent.image_id,
                rel.parent.placement_id.external_id().unwrap_or(0),
                rel.horizontal_offset,
                rel.vertical_offset,
            ),
        };
    let display = Display {
        image_id: key.image_id,
        image_number: 0,
        placement_id: key.placement_id.external_id().unwrap_or(0),
        x: placement.source_x,
        y: placement.source_y,
        width: placement.source_width,
        height: placement.source_height,
        x_offset: placement.x_offset,
        y_offset: placement.y_offset,
        columns: placement.columns,
        rows: placement.rows,
        cursor_movement: CursorMovement::None,
        virtual_placement,
        parent_id,
        parent_placement_id,
        horizontal_offset,
        vertical_offset,
        z: placement.z,
    };
    let _ = write!(out, "\x1b_Ga=p,i={}", key.image_id);
    write_display_keys(out, &display);
    out.push_str(",q=2\x1b\\");
}
