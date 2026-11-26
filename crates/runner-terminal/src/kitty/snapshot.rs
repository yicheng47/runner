//! Serializing Kitty graphics images and placements into VT snapshots within
//! the `wire::MAX_FRAME` budget.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use super::command::{
    base64_encoded_len, encode_normalized_transmission, write_display_keys, CursorMovement, Display,
};
use super::storage::{
    CellSize, ImageStorage, Placement, PlacementKey, PlacementLocation, TerminalGraphics,
};

/// Upper bound on bytes graphics may occupy inside a snapshot (48 MiB).
pub const MAX_SNAPSHOT_GRAPHICS_BYTES: usize = 48 * 1024 * 1024;

/// Safety reserve kept below `wire::MAX_FRAME` when computing the graphics budget.
pub const SNAPSHOT_FRAME_HEADROOM: usize = 1024 * 1024;

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

        // Gather all candidate images across primary (and active alt screen).
        // Keep-order: active visible > inactive primary visible > non-visible,
        // newest generation first (so LRU is dropped first, visible last).
        let mut candidates = Vec::new();
        collect_candidates(
            ScreenKind::Primary,
            &graphics.primary,
            !alt_active,
            primary_first_line,
            primary_screen_lines,
            screen_cols,
            cell,
            &mut candidates,
        );
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
        let mut used = 0usize;
        for c in candidates {
            if used.saturating_add(c.cost) <= budget {
                used += c.cost;
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
                emit_children(out, plan, root_key);
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
                emit_children(out, plan, key);
            }
        }
        out.push_str("\x1b8");
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
) {
    let mut has_visible: HashSet<u32> = HashSet::new();
    let mut placement_bytes: HashMap<u32, usize> = HashMap::new();

    for (&key, placement) in storage.placements() {
        let visible = match placement.location {
            PlacementLocation::Pin(pin) => {
                if pin.line >= 0 && pin.line < screen_lines as i32 {
                    true
                } else if pin.line < 0 {
                    storage
                        .image_by_id(key.image_id)
                        .and_then(|img| {
                            placement.cell_rect(img.width, img.height, cell, screen_cols)
                        })
                        .is_some_and(|rect| rect.bottom_line >= 0)
                } else {
                    false
                }
            }
            PlacementLocation::Relative(rel) => {
                storage
                    .resolved_anchor(placement)
                    .map(|anchor| {
                        if anchor.line >= 0 && anchor.line < screen_lines as i32 {
                            true
                        } else if anchor.line < 0 {
                            storage
                                .image_by_id(key.image_id)
                                .map(|img| {
                                    let (_, rows) =
                                        placement.grid_size(img.width, img.height, cell);
                                    anchor.line.saturating_add(rows.saturating_sub(1) as i32) >= 0
                                })
                                .unwrap_or(false)
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false)
                    && storage.resolve_chain(rel).is_some()
            }
            PlacementLocation::Virtual => false,
        };
        if visible {
            has_visible.insert(key.image_id);
        }

        // Only count placements that would actually be emitted.
        let emittable = match placement.location {
            PlacementLocation::Pin(pin) => {
                pin.line < screen_lines as i32
                    && (pin.line >= first_line
                        || clipped_to_first_line(
                            storage,
                            key.image_id,
                            *placement,
                            first_line,
                            cell,
                        )
                        .is_some())
            }
            PlacementLocation::Virtual | PlacementLocation::Relative(_) => true,
        };
        if emittable {
            // Conservative upper bound covering `\x1b7`, `\x1b[colG`, `a=p...`, `\x1b8`.
            let est = estimate_placement_bytes(key, placement);
            *placement_bytes.entry(key.image_id).or_default() += est;
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
        let tx_cost = estimate_transmission_bytes(img);
        let p_cost = placement_bytes.get(&img.id).copied().unwrap_or(0);
        out.push(Candidate {
            screen,
            image_id: img.id,
            tier,
            generation: img.generation,
            cost: tx_cost.saturating_add(p_cost),
        });
    }
}

struct Candidate {
    screen: ScreenKind,
    image_id: u32,
    tier: u8,
    generation: u64,
    cost: usize,
}

fn estimate_transmission_bytes(img: &super::image::StoredImage) -> usize {
    // Header `\x1b_Ga=t,t=d,i=...,f=...,s=...,v=...,o=z,N=1,q=2;` (< 80 bytes) + base64 + `\x1b\\`.
    80 + base64_encoded_len(img.encoded_data.len()) + 2
}

fn estimate_placement_bytes(key: PlacementKey, placement: &Placement) -> usize {
    let mut tmp = String::new();
    emit_placement_command(&mut tmp, key, placement);
    // Add room for `\x1b7\x1b[99999G` and `\x1b8` around row placements.
    tmp.len() + 20
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
    let clip = (first_line - pin.line) as u32;
    if rows == 0 || clip >= rows {
        return None;
    }
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
    let mut emitted_keys: HashSet<PlacementKey> = HashSet::new();
    for (&key, &placement) in storage.placements() {
        if !kept_images.contains(&key.image_id) {
            continue;
        }
        match placement.location {
            PlacementLocation::Pin(pin) => {
                if pin.line >= screen_lines as i32 {
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
                emitted_keys.insert(key);
            }
            PlacementLocation::Virtual => {
                plan.virtual_roots.push(key);
                emitted_keys.insert(key);
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

    // Attach relative placements whose entire ancestor chain was kept.
    loop {
        let mut added = false;
        for (&key, &placement) in storage.placements() {
            if emitted_keys.contains(&key) || !kept_images.contains(&key.image_id) {
                continue;
            }
            if let PlacementLocation::Relative(rel) = placement.location {
                if emitted_keys.contains(&rel.parent) {
                    plan.children
                        .entry(rel.parent)
                        .or_default()
                        .push((key, placement));
                    emitted_keys.insert(key);
                    added = true;
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

fn emit_children(out: &mut String, plan: &ScreenPlan, parent: PlacementKey) {
    let Some(children) = plan.children.get(&parent) else {
        return;
    };
    for &(child_key, child_placement) in children {
        emit_placement_command(out, child_key, &child_placement);
        emit_children(out, plan, child_key);
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
