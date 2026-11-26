//! Per-screen Kitty graphics storage: images, placements, byte accounting,
//! daemon-wide atomic budget, eviction, delete selectors, scroll/margin/resize
//! updates, and relative placement chains.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use super::command::DeleteAction;
use super::image::{ImageError, LoadingImage, StoredImage, PER_SCREEN_LIMIT};

/// Default daemon-wide graphics memory cap across all sessions (1 GiB).
pub const DEFAULT_GLOBAL_BUDGET: usize = 1024 * 1024 * 1024;

/// Maximum parent links in a relative placement chain (Kitty and Ghostty).
pub const PARENT_CHAIN_LIMIT: usize = 8;

static GENERATION_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn next_generation() -> u64 {
    GENERATION_COUNTER.fetch_add(1, Ordering::Relaxed) + 1
}

/// Session cell size in device pixels (0 means unknown).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CellSize {
    pub width: u16,
    pub height: u16,
}

impl CellSize {
    pub const ZERO: Self = Self {
        width: 0,
        height: 0,
    };

    /// Clamp a non-zero cell size to the spec's 1..=1024 px range per side.
    pub fn clamped(self) -> Self {
        Self {
            width: self.width.clamp(1, 1024),
            height: self.height.clamp(1, 1024),
        }
    }
}

/// Daemon-wide atomic byte budget shared across all authoritative session
/// models. Sessions reserve and release bytes without taking another session's
/// lock; each session evicts only from its own active screen.
#[derive(Debug)]
pub struct GlobalImageBudget {
    used: AtomicUsize,
    limit: usize,
}

impl Default for GlobalImageBudget {
    fn default() -> Self {
        Self::new(DEFAULT_GLOBAL_BUDGET)
    }
}

impl GlobalImageBudget {
    pub fn new(limit: usize) -> Self {
        Self {
            used: AtomicUsize::new(0),
            limit,
        }
    }

    pub fn used(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    fn release(&self, bytes: usize) {
        if bytes > 0 {
            self.used.fetch_sub(bytes, Ordering::AcqRel);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlacementId {
    Internal(u32),
    External(u32),
}

impl PlacementId {
    /// External IDs win over internal ones; lower numeric IDs win within a tag,
    /// matching Ghostty `PlacementId.preferredOver`.
    pub fn preferred_over(self, other: Self) -> bool {
        match (self, other) {
            (Self::External(_), Self::Internal(_)) => true,
            (Self::Internal(_), Self::External(_)) => false,
            (Self::External(a), Self::External(b)) | (Self::Internal(a), Self::Internal(b)) => {
                a < b
            }
        }
    }

    pub fn external_id(self) -> Option<u32> {
        match self {
            Self::External(id) => Some(id),
            Self::Internal(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlacementKey {
    pub image_id: u32,
    pub placement_id: PlacementId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinLocation {
    /// Viewport line (`0..rows` on visible screen, negative in scrollback history).
    pub line: i32,
    pub column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelativeLocation {
    pub parent: PlacementKey,
    pub horizontal_offset: i32,
    pub vertical_offset: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementLocation {
    Pin(PinLocation),
    Virtual,
    Relative(RelativeLocation),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRect {
    pub top_line: i32,
    pub bottom_line: i32,
    pub left_col: usize,
    pub right_col: usize,
}

impl CellRect {
    pub fn contains(&self, col: usize, line: i32) -> bool {
        col >= self.left_col
            && col <= self.right_col
            && line >= self.top_line
            && line <= self.bottom_line
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub location: PlacementLocation,
    pub x_offset: u32,
    pub y_offset: u32,
    pub source_x: u32,
    pub source_y: u32,
    pub source_width: u32,
    pub source_height: u32,
    pub columns: u32,
    pub rows: u32,
    pub z: i32,
}

impl Placement {
    pub fn source_rect(&self, image_width: u32, image_height: u32) -> SourceRect {
        let x = self.source_x.min(image_width);
        let y = self.source_y.min(image_height);
        let width = if self.source_width > 0 {
            self.source_width
        } else {
            image_width
        }
        .min(image_width - x);
        let height = if self.source_height > 0 {
            self.source_height
        } else {
            image_height
        }
        .min(image_height - y);
        SourceRect {
            x,
            y,
            width,
            height,
        }
    }

    pub fn cell_offset(&self, cell: CellSize) -> (u32, u32) {
        let cw = u32::from(cell.width);
        let ch = u32::from(cell.height);
        (
            if cw > 0 { self.x_offset.min(cw - 1) } else { 0 },
            if ch > 0 { self.y_offset.min(ch - 1) } else { 0 },
        )
    }

    pub fn pixel_size(&self, image_width: u32, image_height: u32, cell: CellSize) -> (u32, u32) {
        let src = self.source_rect(image_width, image_height);
        if self.columns == 0 && self.rows == 0 {
            return (src.width, src.height);
        }
        let cw = u32::from(cell.width);
        let ch = u32::from(cell.height);
        let (ox, oy) = self.cell_offset(cell);

        if self.columns > 0 && self.rows > 0 {
            let w = cw.saturating_mul(self.columns).saturating_sub(ox);
            let h = ch.saturating_mul(self.rows).saturating_sub(oy);
            return (w, h);
        }
        if self.columns > 0 {
            let w = cw.saturating_mul(self.columns).saturating_sub(ox);
            let h = scale_dimension(w, src.height, src.width);
            return (w, h);
        }
        let h = ch.saturating_mul(self.rows).saturating_sub(oy);
        let w = scale_dimension(h, src.width, src.height);
        (w, h)
    }

    pub fn grid_size(&self, image_width: u32, image_height: u32, cell: CellSize) -> (u32, u32) {
        if self.columns > 0 && self.rows > 0 {
            return (self.columns, self.rows);
        }
        let (pw, ph) = self.pixel_size(image_width, image_height, cell);
        let (ox, oy) = self.cell_offset(cell);
        let cw = u32::from(cell.width);
        let ch = u32::from(cell.height);
        let cols = if cw == 0 {
            0
        } else {
            pw.saturating_add(ox).div_ceil(cw)
        };
        let rows = if ch == 0 {
            0
        } else {
            ph.saturating_add(oy).div_ceil(ch)
        };
        (cols, rows)
    }

    pub fn clip_top(
        &mut self,
        image_width: u32,
        image_height: u32,
        cell: CellSize,
        count: u32,
        span: u32,
    ) -> bool {
        debug_assert!(count > 0 && count < span);
        let src = self.source_rect(image_width, image_height);
        let crop = if self.rows > 0 {
            ((u64::from(src.height) * u64::from(count)) / u64::from(span)) as u32
        } else {
            u32::from(cell.height).saturating_mul(count)
        };
        if crop >= src.height {
            return false;
        }
        self.source_x = src.x;
        self.source_y = src.y + crop;
        self.source_width = src.width;
        self.source_height = src.height - crop;
        if self.rows > 0 {
            self.rows -= count;
        }
        true
    }

    pub fn clip_bottom(
        &mut self,
        image_width: u32,
        image_height: u32,
        cell: CellSize,
        count: u32,
        span: u32,
    ) -> bool {
        debug_assert!(count > 0 && count < span);
        let src = self.source_rect(image_width, image_height);
        if src.height == 0 {
            return false;
        }
        if self.rows > 0 {
            let crop = ((u64::from(src.height) * u64::from(count)) / u64::from(span)) as u32;
            if crop >= src.height {
                return false;
            }
            self.source_height = src.height - crop;
            self.rows -= count;
        } else {
            let visible = u32::from(cell.height).saturating_mul(span - count);
            let offset = self.cell_offset(cell).1;
            if visible <= offset {
                return false;
            }
            self.source_height = src.height.min(visible - offset);
        }
        self.source_x = src.x;
        self.source_y = src.y;
        self.source_width = src.width;
        true
    }

    pub fn cell_rect(
        &self,
        image_width: u32,
        image_height: u32,
        cell: CellSize,
        screen_cols: usize,
    ) -> Option<CellRect> {
        let pin = match self.location {
            PlacementLocation::Pin(pin) => pin,
            PlacementLocation::Virtual | PlacementLocation::Relative(_) => return None,
        };
        let (cols, rows) = self.grid_size(image_width, image_height, cell);
        if cols == 0 || rows == 0 {
            return None;
        }
        let bottom_line = last_row_i64(pin.line, rows).min(i64::from(i32::MAX)) as i32;
        let right_col = pin
            .column
            .saturating_add((cols - 1) as usize)
            .min(screen_cols.saturating_sub(1));
        Some(CellRect {
            top_line: pin.line,
            bottom_line,
            left_col: pin.column,
            right_col,
        })
    }
}

pub(crate) fn last_row_i64(top_line: i32, rows: u32) -> i64 {
    i64::from(top_line) + i64::from(rows.saturating_sub(1))
}

fn scale_dimension(value: u32, numerator: u32, denominator: u32) -> u32 {
    if denominator == 0 {
        return 0;
    }
    let rounded = (u64::from(value) * u64::from(numerator) + u64::from(denominator) / 2)
        / u64::from(denominator);
    rounded.min(u64::from(u32::MAX)) as u32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentError {
    ParentImageNotFound,
    ParentPlacementNotFound,
    SelfParent,
    Cycle,
    TooDeep,
    AncestorNotFound,
}

impl ParentError {
    pub fn protocol_message(self) -> &'static str {
        match self {
            Self::ParentImageNotFound => "ENOPARENT: parent image not found",
            Self::ParentPlacementNotFound => "ENOPARENT: parent placement not found",
            Self::SelfParent => "EINVAL: placement cannot be its own parent",
            Self::Cycle => "ECYCLE: parent chain creates a cycle",
            Self::TooDeep => "ETOODEEP: parent chain too deep",
            Self::AncestorNotFound => "ENOENT: parent chain ancestor not found",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedChain {
    pub root_key: PlacementKey,
    pub root: Placement,
    pub horizontal_offset: i32,
    pub vertical_offset: i32,
}

#[derive(Debug, Clone, Default)]
pub struct EvictedImage {
    pub id: u32,
    /// True when evicting this image orphaned a relative placement belonging to
    /// another image; mirrors must receive lowercase `d=i` before `d=I` so they
    /// do not free that other image.
    pub orphaned_other_image: bool,
}

#[derive(Debug)]
pub struct ImageStorage {
    pub dirty: u64,
    pub generation: u64,
    next_image_id: u32,
    next_internal_placement_id: u32,
    images: HashMap<u32, StoredImage>,
    placements: HashMap<PlacementKey, Placement>,
    pub loading: Option<LoadingImage>,
    /// Decoded RGB/RGBA byte count enforced against the per-screen limit.
    total_bytes: usize,
    /// Actual retained encoded payload bytes enforced against `GlobalImageBudget`.
    encoded_bytes: usize,
    total_limit: usize,
    budget: Option<Arc<GlobalImageBudget>>,
    /// Mirror mode applies stream deletes and only evicts at the safety cap
    /// (`2 * PER_SCREEN_LIMIT`).
    mirror_mode: bool,
}

impl Drop for ImageStorage {
    fn drop(&mut self) {
        if let Some(budget) = &self.budget {
            budget.release(self.encoded_bytes);
        }
        self.total_bytes = 0;
        self.encoded_bytes = 0;
    }
}

impl ImageStorage {
    pub fn new(total_limit: usize, budget: Option<Arc<GlobalImageBudget>>) -> Self {
        Self {
            dirty: 0,
            generation: 0,
            next_image_id: 2_147_483_647,
            next_internal_placement_id: 0,
            images: HashMap::new(),
            placements: HashMap::new(),
            loading: None,
            total_bytes: 0,
            encoded_bytes: 0,
            total_limit,
            budget,
            mirror_mode: false,
        }
    }

    pub fn authoritative(budget: Option<Arc<GlobalImageBudget>>) -> Self {
        Self::new(PER_SCREEN_LIMIT, budget)
    }

    pub fn mirror() -> Self {
        let mut storage = Self::new(PER_SCREEN_LIMIT * 2, None);
        storage.mirror_mode = true;
        storage
    }

    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    pub fn encoded_bytes(&self) -> usize {
        self.encoded_bytes
    }

    pub fn total_limit(&self) -> usize {
        self.total_limit
    }

    pub fn set_limit(&mut self, limit: usize) {
        self.total_limit = limit;
    }

    pub fn is_mirror(&self) -> bool {
        self.mirror_mode
    }

    pub fn is_empty(&self) -> bool {
        self.images.is_empty() && self.placements.is_empty()
    }

    pub fn images(&self) -> &HashMap<u32, StoredImage> {
        &self.images
    }

    pub fn placements(&self) -> &HashMap<PlacementKey, Placement> {
        &self.placements
    }

    pub fn image_by_id(&self, id: u32) -> Option<&StoredImage> {
        self.images.get(&id)
    }

    pub fn image_by_number(&self, number: u32) -> Option<&StoredImage> {
        if number == 0 {
            return None;
        }
        let mut newest: Option<&StoredImage> = None;
        for img in self.images.values() {
            if img.number == number && newest.is_none_or(|n| img.generation > n.generation) {
                newest = Some(img);
            }
        }
        newest
    }

    pub fn contains_image(&self, id: u32) -> bool {
        self.images.contains_key(&id)
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = self.dirty.wrapping_add(1);
    }

    pub fn mark_mutated(&mut self) {
        self.mark_dirty();
        self.generation = next_generation();
    }

    pub fn reset(&mut self) {
        self.loading = None;
        let had = !self.images.is_empty() || !self.placements.is_empty();
        self.placements.clear();
        self.images.clear();
        if let Some(budget) = &self.budget {
            budget.release(self.encoded_bytes);
        }
        self.total_bytes = 0;
        self.encoded_bytes = 0;
        if had {
            self.mark_mutated();
        }
    }

    pub fn next_image_id(&mut self, implicit: bool) -> u32 {
        let mut id = if implicit { self.next_image_id } else { 1 };
        let count = self.images.len();
        for _ in 0..(count + 2) {
            if id != 0 && !self.images.contains_key(&id) {
                break;
            }
            id = id.wrapping_add(1);
        }
        if id == 0 {
            id = 1;
        }
        if implicit {
            self.next_image_id = id.wrapping_add(1);
        }
        id
    }

    /// Add or replace an image, enforcing both the per-screen decoded limit and
    /// the optional daemon-wide encoded `GlobalImageBudget`. Returns the
    /// evicted image IDs in eviction order so `exec` can emit normalized
    /// deletes before the new transmission.
    pub fn add_image(&mut self, mut image: StoredImage) -> Result<Vec<EvictedImage>, ImageError> {
        let new_decoded = image.storage_bytes();
        let new_encoded = image.encoded_bytes();
        if new_decoded > self.total_limit {
            return Err(ImageError::OutOfMemory);
        }
        let (old_decoded, old_encoded) = self
            .images
            .get(&image.id)
            .map_or((0, 0), |img| (img.storage_bytes(), img.encoded_bytes()));
        let evictable_decoded = self.total_bytes.saturating_sub(old_decoded);
        let evictable_encoded = self.encoded_bytes.saturating_sub(old_encoded);
        let screen_excess = (self.total_bytes - old_decoded)
            .saturating_add(new_decoded)
            .saturating_sub(self.total_limit);

        let victims = if let Some(budget) = self.budget.clone() {
            if new_encoded > budget.limit {
                return Err(ImageError::OutOfMemory);
            }
            loop {
                let current = budget.used.load(Ordering::Acquire);
                let global_excess = current
                    .saturating_sub(old_encoded)
                    .saturating_add(new_encoded)
                    .saturating_sub(budget.limit);
                if screen_excess > evictable_decoded || global_excess > evictable_encoded {
                    return Err(ImageError::OutOfMemory);
                }
                let (plan, evicted_encoded) = if screen_excess == 0 && global_excess == 0 {
                    (Vec::new(), 0)
                } else {
                    self.plan_evictions(screen_excess, global_excess, Some(image.id))
                        .ok_or(ImageError::OutOfMemory)?
                };
                let freed_encoded = old_encoded + evicted_encoded;
                if new_encoded > freed_encoded {
                    let net_add = new_encoded - freed_encoded;
                    if current.saturating_add(net_add) > budget.limit {
                        continue;
                    }
                    if budget
                        .used
                        .compare_exchange_weak(
                            current,
                            current + net_add,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_ok()
                    {
                        break plan;
                    }
                } else {
                    let net_sub = freed_encoded - new_encoded;
                    budget.used.fetch_sub(net_sub, Ordering::AcqRel);
                    break plan;
                }
            }
        } else if screen_excess > 0 {
            let (plan, _) = self
                .plan_evictions(screen_excess, 0, Some(image.id))
                .ok_or(ImageError::OutOfMemory)?;
            plan
        } else {
            Vec::new()
        };

        let mut evicted_report = Vec::with_capacity(victims.len());
        for victim_id in victims {
            self.remove_placements_by_image_id(victim_id);
            let orphaned_other = self.remove_orphans_tracking_other(victim_id, false);
            if let Some(removed) = self.images.remove(&victim_id) {
                self.total_bytes -= removed.storage_bytes();
                self.encoded_bytes -= removed.encoded_bytes();
            }
            evicted_report.push(EvictedImage {
                id: victim_id,
                orphaned_other_image: orphaned_other,
            });
        }

        if let Some(old) = self.images.remove(&image.id) {
            self.remove_placements_by_image_id(image.id);
            self.remove_orphans(false);
            self.total_bytes -= old.storage_bytes();
            self.encoded_bytes -= old.encoded_bytes();
        }

        self.mark_mutated();
        image.placement_count = 0;
        image.generation = self.generation;
        image.transmission = self.generation;
        self.total_bytes += new_decoded;
        self.encoded_bytes += new_encoded;
        self.images.insert(image.id, image);

        Ok(evicted_report)
    }

    /// Read-only eviction plan matching Ghostty `evictImageExcept`:
    /// unused before placed, transient before non-transient, oldest generation
    /// first, then lowest image ID. Selects victims until both per-screen
    /// decoded excess and daemon-wide encoded excess are satisfied.
    fn plan_evictions(
        &self,
        req_decoded: usize,
        req_encoded: usize,
        exclude_id: Option<u32>,
    ) -> Option<(Vec<u32>, usize)> {
        if req_decoded == 0 && req_encoded == 0 {
            return Some((Vec::new(), 0));
        }
        #[derive(Clone, Copy)]
        struct Candidate {
            priority: u8,
            generation: u64,
            id: u32,
            decoded_bytes: usize,
            encoded_bytes: usize,
        }
        let mut candidates: Vec<Candidate> = self
            .images
            .values()
            .filter(|img| exclude_id != Some(img.id))
            .map(|img| {
                let priority =
                    u8::from(!img.transient) + if img.placement_count > 0 { 2 } else { 0 };
                Candidate {
                    priority,
                    generation: img.generation,
                    id: img.id,
                    decoded_bytes: img.storage_bytes(),
                    encoded_bytes: img.encoded_bytes(),
                }
            })
            .collect();
        candidates.sort_by_key(|c| (c.priority, c.generation, c.id));
        let mut chosen = Vec::new();
        let mut sum_decoded = 0usize;
        let mut sum_encoded = 0usize;
        for c in candidates {
            chosen.push(c.id);
            sum_decoded += c.decoded_bytes;
            sum_encoded += c.encoded_bytes;
            if sum_decoded >= req_decoded && sum_encoded >= req_encoded {
                return Some((chosen, sum_encoded));
            }
        }
        None
    }

    pub fn add_placement(
        &mut self,
        image_id: u32,
        placement_id: u32,
        placement: Placement,
    ) -> PlacementKey {
        debug_assert!(self.images.contains_key(&image_id));
        let key = PlacementKey {
            image_id,
            placement_id: if placement_id == 0 {
                let id = self.next_internal_placement_id;
                self.next_internal_placement_id = self.next_internal_placement_id.wrapping_add(1);
                PlacementId::Internal(id)
            } else {
                PlacementId::External(placement_id)
            },
        };
        let existed = self.placements.insert(key, placement).is_some();
        if !existed {
            if let Some(img) = self.images.get_mut(&image_id) {
                img.placement_count = img.placement_count.saturating_add(1);
            }
        }
        self.mark_mutated();
        key
    }

    pub fn resolve_parent(
        &self,
        child: Option<PlacementKey>,
        parent_image_id: u32,
        parent_placement_id: u32,
    ) -> Result<PlacementKey, ParentError> {
        if !self.images.contains_key(&parent_image_id) {
            return Err(ParentError::ParentImageNotFound);
        }
        let parent = if parent_placement_id > 0 {
            let key = PlacementKey {
                image_id: parent_image_id,
                placement_id: PlacementId::External(parent_placement_id),
            };
            if !self.placements.contains_key(&key) {
                return Err(ParentError::ParentPlacementNotFound);
            }
            key
        } else {
            let mut best: Option<PlacementId> = None;
            for key in self.placements.keys() {
                if key.image_id != parent_image_id {
                    continue;
                }
                if best.is_none_or(|b| key.placement_id.preferred_over(b)) {
                    best = Some(key.placement_id);
                }
            }
            let pid = best.ok_or(ParentError::ParentPlacementNotFound)?;
            PlacementKey {
                image_id: parent_image_id,
                placement_id: pid,
            }
        };

        if child == Some(parent) {
            return Err(ParentError::SelfParent);
        }

        let mut depth = 1usize;
        let mut key = parent;
        loop {
            if child == Some(key) {
                return Err(ParentError::Cycle);
            }
            let p = self
                .placements
                .get(&key)
                .ok_or(ParentError::AncestorNotFound)?;
            let rel = match p.location {
                PlacementLocation::Relative(rel) => rel,
                PlacementLocation::Pin(_) | PlacementLocation::Virtual => break,
            };
            if depth >= PARENT_CHAIN_LIMIT {
                return Err(ParentError::TooDeep);
            }
            depth += 1;
            key = rel.parent;
        }
        if let Some(child_key) = child {
            if self.placements.contains_key(&child_key) {
                let descendant_depth = self.max_descendant_depth(child_key);
                if depth.saturating_add(descendant_depth) > PARENT_CHAIN_LIMIT {
                    return Err(ParentError::TooDeep);
                }
            }
        }
        Ok(parent)
    }

    fn max_descendant_depth(&self, root: PlacementKey) -> usize {
        let mut max_depth = 0usize;
        let mut frontier = vec![(root, 0usize)];
        while let Some((node, depth)) = frontier.pop() {
            if depth > PARENT_CHAIN_LIMIT {
                return depth;
            }
            for (&key, placement) in &self.placements {
                if let PlacementLocation::Relative(rel) = placement.location {
                    if rel.parent == node && key != root {
                        let next_depth = depth + 1;
                        max_depth = max_depth.max(next_depth);
                        frontier.push((key, next_depth));
                    }
                }
            }
        }
        max_depth
    }

    pub fn resolve_chain(&self, rel: RelativeLocation) -> Option<ResolvedChain> {
        let mut horizontal = rel.horizontal_offset;
        let mut vertical = rel.vertical_offset;
        let mut key = rel.parent;
        let mut depth = 1usize;
        loop {
            let p = *self.placements.get(&key)?;
            match p.location {
                PlacementLocation::Relative(parent_rel) => {
                    if depth >= PARENT_CHAIN_LIMIT {
                        return None;
                    }
                    depth += 1;
                    horizontal = horizontal.saturating_add(parent_rel.horizontal_offset);
                    vertical = vertical.saturating_add(parent_rel.vertical_offset);
                    key = parent_rel.parent;
                }
                PlacementLocation::Pin(_) | PlacementLocation::Virtual => {
                    return Some(ResolvedChain {
                        root_key: key,
                        root: p,
                        horizontal_offset: horizontal,
                        vertical_offset: vertical,
                    });
                }
            }
        }
    }

    /// Resolve a placement's effective anchor `(line, column)` on the grid.
    /// Returns `None` for virtual placements or broken relative chains.
    pub fn resolved_anchor(&self, placement: &Placement) -> Option<PinLocation> {
        match placement.location {
            PlacementLocation::Pin(pin) => Some(pin),
            PlacementLocation::Virtual => None,
            PlacementLocation::Relative(rel) => {
                let chain = self.resolve_chain(rel)?;
                let root_pin = match chain.root.location {
                    PlacementLocation::Pin(pin) => pin,
                    _ => return None,
                };
                let col = i64::try_from(root_pin.column)
                    .ok()?
                    .saturating_add(i64::from(chain.horizontal_offset));
                if col < 0 {
                    return None;
                }
                Some(PinLocation {
                    line: root_pin.line.saturating_add(chain.vertical_offset),
                    column: col as usize,
                })
            }
        }
    }

    fn remove_placement_by_key(&mut self, key: PlacementKey) -> bool {
        if self.placements.remove(&key).is_none() {
            return false;
        }
        if let Some(img) = self.images.get_mut(&key.image_id) {
            img.placement_count = img.placement_count.saturating_sub(1);
        }
        true
    }

    fn remove_placements_by_image_id(&mut self, image_id: u32) {
        let keys: Vec<PlacementKey> = self
            .placements
            .keys()
            .copied()
            .filter(|k| k.image_id == image_id)
            .collect();
        for key in keys {
            self.remove_placement_by_key(key);
        }
    }

    fn delete_if_unused(&mut self, image_id: u32) {
        if self
            .images
            .get(&image_id)
            .is_none_or(|img| img.placement_count > 0)
        {
            return;
        }
        if let Some(img) = self.images.remove(&image_id) {
            let decoded = img.storage_bytes();
            let encoded = img.encoded_bytes();
            self.total_bytes -= decoded;
            self.encoded_bytes -= encoded;
            if let Some(budget) = &self.budget {
                budget.release(encoded);
            }
        }
    }

    fn remove_orphans(&mut self, delete_unused: bool) -> bool {
        self.remove_orphans_tracking_other(0, delete_unused)
    }

    fn remove_orphans_tracking_other(&mut self, source_image_id: u32, delete_unused: bool) -> bool {
        let mut removed_other = false;
        loop {
            let orphans: Vec<PlacementKey> = self
                .placements
                .iter()
                .filter_map(|(&key, p)| match p.location {
                    PlacementLocation::Relative(rel)
                        if !self.placements.contains_key(&rel.parent) =>
                    {
                        Some(key)
                    }
                    _ => None,
                })
                .collect();
            if orphans.is_empty() {
                break;
            }
            for key in orphans {
                self.remove_placement_by_key(key);
                if key.image_id != source_image_id {
                    removed_other = true;
                }
                if delete_unused {
                    self.delete_if_unused(key.image_id);
                }
            }
        }
        removed_other
    }

    fn placement_intersects_visible(
        &self,
        key: PlacementKey,
        placement: &Placement,
        screen_lines: usize,
        screen_cols: usize,
        cell: CellSize,
    ) -> bool {
        let pin = match placement.location {
            PlacementLocation::Pin(pin) => pin,
            PlacementLocation::Virtual | PlacementLocation::Relative(_) => return false,
        };
        if pin.line >= 0 && pin.line < screen_lines as i32 {
            return true;
        }
        if pin.line >= screen_lines as i32 {
            return false;
        }
        let Some(img) = self.images.get(&key.image_id) else {
            return false;
        };
        let Some(rect) = placement.cell_rect(img.width, img.height, cell, screen_cols) else {
            return false;
        };
        rect.bottom_line >= 0
    }

    fn delete_visible_placements(
        &mut self,
        screen_lines: usize,
        screen_cols: usize,
        cell: CellSize,
        delete_unused: bool,
    ) {
        let doomed: Vec<PlacementKey> = self
            .placements
            .iter()
            .filter_map(|(&key, p)| {
                self.placement_intersects_visible(key, p, screen_lines, screen_cols, cell)
                    .then_some(key)
            })
            .collect();
        for key in doomed {
            self.remove_placement_by_key(key);
            if delete_unused {
                self.delete_if_unused(key.image_id);
            }
        }
    }

    /// ED 2 (`clear_screen(All)`) and entering mode 1049: deletes every
    /// placement intersecting the visible screen, orphaned relative placements,
    /// and every image left without a placement (including images never placed).
    pub fn clear_screen(&mut self, screen_lines: usize, screen_cols: usize, cell: CellSize) {
        let placements_before = self.placements.len();
        let images_before = self.images.len();

        self.delete_visible_placements(screen_lines, screen_cols, cell, true);
        self.remove_orphans(false);
        let unused_ids: Vec<u32> = self
            .images
            .values()
            .filter(|img| img.placement_count == 0)
            .map(|img| img.id)
            .collect();
        for id in unused_ids {
            self.delete_if_unused(id);
        }

        if self.placements.len() != placements_before || self.images.len() != images_before {
            self.mark_mutated();
        }
    }

    /// ED 3 (`clear_screen(Saved)`): delete all pin placements anchored in history.
    pub fn erase_history(&mut self) {
        let doomed: Vec<PlacementKey> = self
            .placements
            .iter()
            .filter_map(|(&key, p)| match p.location {
                PlacementLocation::Pin(pin) if pin.line < 0 => Some(key),
                _ => None,
            })
            .collect();
        if doomed.is_empty() {
            return;
        }
        for key in doomed {
            self.remove_placement_by_key(key);
        }
        self.remove_orphans(false);
        self.mark_mutated();
    }

    /// Execute a protocol delete action (`a=d`).
    pub fn delete(
        &mut self,
        action: DeleteAction,
        cursor: (usize, i32),
        screen_lines: usize,
        screen_cols: usize,
        cell: CellSize,
    ) {
        let placements_before = self.placements.len();
        let images_before = self.images.len();

        match action {
            DeleteAction::All { delete_images } => {
                self.delete_visible_placements(screen_lines, screen_cols, cell, delete_images);
            }
            DeleteAction::Id {
                delete_images,
                image_id,
                placement_id,
            } => {
                self.delete_by_id(image_id, placement_id, delete_images);
            }
            DeleteAction::Newest {
                delete_images,
                image_number,
                placement_id,
            } => {
                if let Some(id) = self.image_by_number(image_number).map(|img| img.id) {
                    self.delete_by_id(id, placement_id, delete_images);
                }
            }
            DeleteAction::IntersectCursor { delete_images } => {
                self.delete_intersecting(
                    cursor.0,
                    cursor.1,
                    screen_cols,
                    cell,
                    delete_images,
                    None,
                );
            }
            DeleteAction::IntersectCell {
                delete_images,
                x,
                y,
            } => {
                if x > 0 && y > 0 {
                    let col = (x - 1) as usize;
                    let line = (y - 1) as usize;
                    if col < screen_cols && line < screen_lines {
                        self.delete_intersecting(
                            col,
                            line as i32,
                            screen_cols,
                            cell,
                            delete_images,
                            None,
                        );
                    }
                }
            }
            DeleteAction::IntersectCellZ {
                delete_images,
                x,
                y,
                z,
            } => {
                if x > 0 && y > 0 {
                    let col = (x - 1) as usize;
                    let line = (y - 1) as usize;
                    if col < screen_cols && line < screen_lines {
                        self.delete_intersecting(
                            col,
                            line as i32,
                            screen_cols,
                            cell,
                            delete_images,
                            Some(z),
                        );
                    }
                }
            }
            DeleteAction::Column { delete_images, x } => {
                if x > 0 {
                    let col = (x - 1) as usize;
                    let doomed: Vec<PlacementKey> = self
                        .placements
                        .iter()
                        .filter_map(|(&key, p)| {
                            let img = self.images.get(&key.image_id)?;
                            let rect = p.cell_rect(img.width, img.height, cell, screen_cols)?;
                            (rect.left_col <= col && col <= rect.right_col).then_some(key)
                        })
                        .collect();
                    for key in doomed {
                        self.remove_placement_by_key(key);
                        if delete_images {
                            self.delete_if_unused(key.image_id);
                        }
                    }
                }
            }
            DeleteAction::Row { delete_images, y } => {
                if y > 0 {
                    let row = (y - 1) as usize;
                    if row < screen_lines {
                        let line = row as i32;
                        let doomed: Vec<PlacementKey> = self
                            .placements
                            .iter()
                            .filter_map(|(&key, p)| {
                                let img = self.images.get(&key.image_id)?;
                                let rect = p.cell_rect(img.width, img.height, cell, screen_cols)?;
                                (rect.top_line <= line && line <= rect.bottom_line).then_some(key)
                            })
                            .collect();
                        for key in doomed {
                            self.remove_placement_by_key(key);
                            if delete_images {
                                self.delete_if_unused(key.image_id);
                            }
                        }
                    }
                }
            }
            DeleteAction::Z { delete_images, z } => {
                let doomed: Vec<PlacementKey> = self
                    .placements
                    .iter()
                    .filter_map(|(&key, p)| match p.location {
                        PlacementLocation::Pin(_) | PlacementLocation::Relative(_) if p.z == z => {
                            Some(key)
                        }
                        _ => None,
                    })
                    .collect();
                for key in doomed {
                    self.remove_placement_by_key(key);
                    if delete_images {
                        self.delete_if_unused(key.image_id);
                    }
                }
            }
            DeleteAction::Range {
                delete_images,
                first,
                last,
            } => {
                if last > 0 && first <= last {
                    let doomed: Vec<PlacementKey> = self
                        .placements
                        .keys()
                        .copied()
                        .filter(|k| k.image_id >= first && k.image_id <= last)
                        .collect();
                    for key in doomed {
                        self.remove_placement_by_key(key);
                    }
                    if delete_images {
                        let ids: Vec<u32> = self
                            .images
                            .keys()
                            .copied()
                            .filter(|&id| id >= first && id <= last)
                            .collect();
                        for id in ids {
                            self.delete_if_unused(id);
                        }
                    }
                }
            }
            DeleteAction::AnimationFrames { .. } => {
                // Parsed in Phase 1; acts once Phase 2 adds animation frames.
            }
        }

        self.remove_orphans(action.delete_images());

        if self.placements.len() != placements_before || self.images.len() != images_before {
            self.mark_mutated();
        }
    }

    fn delete_by_id(&mut self, image_id: u32, placement_id: u32, delete_unused: bool) {
        let mut matched = placement_id == 0;
        if placement_id == 0 {
            self.remove_placements_by_image_id(image_id);
        } else {
            let key = PlacementKey {
                image_id,
                placement_id: PlacementId::External(placement_id),
            };
            if self.remove_placement_by_key(key) {
                matched = true;
            }
        }
        if delete_unused && matched {
            self.delete_if_unused(image_id);
        }
    }

    fn delete_intersecting(
        &mut self,
        col: usize,
        line: i32,
        screen_cols: usize,
        cell: CellSize,
        delete_unused: bool,
        z_filter: Option<i32>,
    ) {
        let doomed: Vec<PlacementKey> = self
            .placements
            .iter()
            .filter_map(|(&key, p)| {
                if let Some(z) = z_filter {
                    if p.z != z {
                        return None;
                    }
                }
                let img = self.images.get(&key.image_id)?;
                let rect = p.cell_rect(img.width, img.height, cell, screen_cols)?;
                rect.contains(col, line).then_some(key)
            })
            .collect();
        for key in doomed {
            self.remove_placement_by_key(key);
            if delete_unused {
                self.delete_if_unused(key.image_id);
            }
        }
    }

    /// Move all pin placements up by `lines` (full-screen scroll up) and prune
    /// placements whose last row scrolled above `topmost_line`.
    pub fn scroll_full_up(&mut self, lines: usize, topmost_line: i32, cell: CellSize) {
        if lines == 0 || self.placements.is_empty() {
            return;
        }
        let delta = i32::try_from(lines).unwrap_or(i32::MAX);
        for placement in self.placements.values_mut() {
            if let PlacementLocation::Pin(pin) = &mut placement.location {
                pin.line = pin.line.saturating_sub(delta);
            }
        }
        self.mark_dirty();
        self.prune_above(topmost_line, cell);
    }

    /// Move only history-anchored pin placements (`pin.line < 0`) up by `lines`
    /// and prune any whose last row scrolled above `topmost_line`. Used for DL
    /// at row 0 and partial regions starting at row 0.
    pub fn scroll_history_only_up(&mut self, lines: usize, topmost_line: i32, cell: CellSize) {
        if lines == 0 || self.placements.is_empty() {
            return;
        }
        let delta = i32::try_from(lines).unwrap_or(i32::MAX);
        let mut moved = false;
        for placement in self.placements.values_mut() {
            if let PlacementLocation::Pin(pin) = &mut placement.location {
                if pin.line < 0 {
                    pin.line = pin.line.saturating_sub(delta);
                    moved = true;
                }
            }
        }
        if moved {
            self.mark_dirty();
            self.prune_above(topmost_line, cell);
        }
    }

    /// Full-screen scroll down (SD / RI at top): moves screen placements down
    /// by `lines`, clipping at the bottom of the screen and deleting wholly
    /// clipped ones. History placements do not move.
    pub fn scroll_full_down(&mut self, lines: usize, screen_lines: usize, cell: CellSize) {
        if lines == 0 || self.placements.is_empty() {
            return;
        }
        let delta = i64::try_from(lines).unwrap_or(i64::from(i32::MAX));
        let bottom = screen_lines as i64 - 1;
        let mut doomed = Vec::new();
        let mut mutated = false;
        let mut moved = false;

        for (&key, placement) in &mut self.placements {
            let PlacementLocation::Pin(pin) = placement.location else {
                continue;
            };
            let pin_line = i64::from(pin.line);
            if pin_line < 0 || pin_line > bottom {
                continue;
            }
            let new_y = pin_line + delta;
            if new_y > bottom {
                doomed.push(key);
                mutated = true;
                continue;
            }
            let new_y_i32 = new_y as i32;
            let Some(img) = self.images.get(&key.image_id) else {
                placement.location = PlacementLocation::Pin(PinLocation {
                    line: new_y_i32,
                    column: pin.column,
                });
                moved = true;
                continue;
            };
            let (_, rows) = placement.grid_size(img.width, img.height, cell);
            if rows == 0 {
                placement.location = PlacementLocation::Pin(PinLocation {
                    line: new_y_i32,
                    column: pin.column,
                });
                moved = true;
                continue;
            }
            let bottom_clip = (new_y + i64::from(rows - 1) - bottom).max(0);
            if bottom_clip >= i64::from(rows) {
                doomed.push(key);
                mutated = true;
            } else if bottom_clip > 0 {
                if !placement.clip_bottom(img.width, img.height, cell, bottom_clip as u32, rows) {
                    doomed.push(key);
                } else {
                    placement.location = PlacementLocation::Pin(PinLocation {
                        line: new_y_i32,
                        column: pin.column,
                    });
                }
                mutated = true;
            } else {
                placement.location = PlacementLocation::Pin(PinLocation {
                    line: new_y_i32,
                    column: pin.column,
                });
                moved = true;
            }
        }

        for key in doomed {
            self.remove_placement_by_key(key);
        }
        if mutated {
            self.remove_orphans(false);
            self.mark_mutated();
        } else if moved {
            self.mark_dirty();
        }
    }

    /// Partial-region scroll (`top..=bottom` inclusive) by `delta` rows
    /// (negative moves content up, positive moves content down), following
    /// Ghostty `ImageStorage.scrollMarginsBegin`: only placements wholly inside
    /// the region move and clip at its margins; straddling, outside, and
    /// history placements do not move.
    pub fn scroll_margins(&mut self, top: i32, bottom: i32, delta: i32, cell: CellSize) {
        if delta == 0 || self.placements.is_empty() {
            return;
        }
        let top64 = i64::from(top);
        let bottom64 = i64::from(bottom);
        let delta64 = i64::from(delta);
        let mut doomed = Vec::new();
        let mut mutated = false;
        let mut moved = false;

        for (&key, placement) in &mut self.placements {
            let PlacementLocation::Pin(pin) = placement.location else {
                continue;
            };
            if pin.line < top || pin.line > bottom {
                continue;
            }
            let Some(img) = self.images.get(&key.image_id) else {
                continue;
            };
            let (cols, rows) = placement.grid_size(img.width, img.height, cell);
            if cols == 0 || rows == 0 {
                continue;
            }
            let end_line = last_row_i64(pin.line, rows);
            if end_line > bottom64 {
                // Straddles the bottom margin: do not move.
                continue;
            }
            let new_y = i64::from(pin.line) + delta64;
            let top_clip = (top64 - new_y).max(0);
            let bottom_clip = (new_y + i64::from(rows - 1) - bottom64).max(0);
            if top_clip >= i64::from(rows) || bottom_clip >= i64::from(rows) {
                doomed.push(key);
                mutated = true;
            } else if top_clip > 0 {
                if !placement.clip_top(img.width, img.height, cell, top_clip as u32, rows) {
                    doomed.push(key);
                } else {
                    placement.location = PlacementLocation::Pin(PinLocation {
                        line: top,
                        column: pin.column,
                    });
                }
                mutated = true;
            } else if bottom_clip > 0 {
                if !placement.clip_bottom(img.width, img.height, cell, bottom_clip as u32, rows) {
                    doomed.push(key);
                } else {
                    placement.location = PlacementLocation::Pin(PinLocation {
                        line: new_y as i32,
                        column: pin.column,
                    });
                }
                mutated = true;
            } else {
                placement.location = PlacementLocation::Pin(PinLocation {
                    line: new_y as i32,
                    column: pin.column,
                });
                moved = true;
            }
        }

        for key in doomed {
            self.remove_placement_by_key(key);
        }
        if mutated {
            self.remove_orphans(false);
            self.mark_mutated();
        } else if moved {
            self.mark_dirty();
        }
    }

    /// Prune pin placements whose last row is above `topmost_line`, along with
    /// any relative children they orphan.
    pub fn prune_above(&mut self, topmost_line: i32, cell: CellSize) {
        if self.placements.is_empty() {
            return;
        }
        let topmost64 = i64::from(topmost_line);
        let doomed: Vec<PlacementKey> = self
            .placements
            .iter()
            .filter_map(|(&key, p)| {
                let PlacementLocation::Pin(pin) = p.location else {
                    return None;
                };
                let rows = self
                    .images
                    .get(&key.image_id)
                    .map(|img| p.grid_size(img.width, img.height, cell).1)
                    .unwrap_or(0);
                let last_row = last_row_i64(pin.line, rows);
                (last_row < topmost64).then_some(key)
            })
            .collect();
        if doomed.is_empty() {
            return;
        }
        for key in doomed {
            self.remove_placement_by_key(key);
        }
        self.remove_orphans(false);
        self.mark_mutated();
    }

    /// Apply a resize's line shift (`+` when rows pulled from history, `-` when
    /// shrinking scrolled rows up) and delete placements outside retained lines
    /// or past `new_cols`.
    pub fn apply_resize(
        &mut self,
        line_shift: i32,
        topmost_line: i32,
        new_rows: usize,
        new_cols: usize,
        cell: CellSize,
    ) {
        if self.placements.is_empty() {
            return;
        }
        let shift64 = i64::from(line_shift);
        let topmost64 = i64::from(topmost_line);
        let new_rows64 = new_rows as i64;
        let mut doomed = Vec::new();
        for (&key, p) in &mut self.placements {
            let PlacementLocation::Pin(pin) = p.location else {
                continue;
            };
            let new_line = i64::from(pin.line) + shift64;
            let clamped_line = new_line.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
            p.location = PlacementLocation::Pin(PinLocation {
                line: clamped_line,
                column: pin.column,
            });
            let rows = self
                .images
                .get(&key.image_id)
                .map(|img| p.grid_size(img.width, img.height, cell).1)
                .unwrap_or(0);
            let last_row = new_line + i64::from(rows.saturating_sub(1));
            if pin.column >= new_cols || new_line >= new_rows64 || last_row < topmost64 {
                doomed.push(key);
            }
        }
        if !doomed.is_empty() {
            for key in doomed {
                self.remove_placement_by_key(key);
            }
            self.remove_orphans(false);
            self.mark_mutated();
        } else if line_shift != 0 {
            self.mark_dirty();
        }
    }
}

/// Both screens' graphics storage for one terminal (`Term`).
#[derive(Debug)]
pub struct TerminalGraphics {
    pub primary: ImageStorage,
    pub alt: ImageStorage,
}

impl TerminalGraphics {
    pub fn authoritative(budget: Option<Arc<GlobalImageBudget>>) -> Self {
        Self {
            primary: ImageStorage::authoritative(budget.clone()),
            alt: ImageStorage::authoritative(budget),
        }
    }

    pub fn mirror() -> Self {
        Self {
            primary: ImageStorage::mirror(),
            alt: ImageStorage::mirror(),
        }
    }

    pub fn active(&self, is_alt: bool) -> &ImageStorage {
        if is_alt {
            &self.alt
        } else {
            &self.primary
        }
    }

    pub fn active_mut(&mut self, is_alt: bool) -> &mut ImageStorage {
        if is_alt {
            &mut self.alt
        } else {
            &mut self.primary
        }
    }

    pub fn reset(&mut self) {
        self.primary.reset();
        self.alt.reset();
    }

    pub fn abort_loading(&mut self) {
        self.primary.loading = None;
        self.alt.loading = None;
    }

    pub fn is_empty(&self) -> bool {
        self.primary.is_empty() && self.alt.is_empty()
    }
}
