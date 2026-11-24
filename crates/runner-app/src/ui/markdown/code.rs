use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::Duration;

use gpui::{EntityId, Global, Task};

use super::CodeBlock;

/// How long the Copy button shows its check and "Copied" confirmation.
pub(super) const COPIED_DURATION: Duration = Duration::from_secs(2);

/// Minimum horizontal scrollbar thumb length in logical pixels at default scale.
pub(super) const MIN_SCROLLBAR_THUMB: f32 = 24.;

/// Identifies a live [`CodeBlock`] by its interaction token address, paired
/// with a [`Weak`] token on every stored entry so freed addresses are never
/// reused across blocks.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct CodeBlockId(usize);

impl CodeBlockId {
    pub(super) fn of(code: &Arc<CodeBlock>) -> Self {
        Self(Arc::as_ptr(&code.state_token) as usize)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct MetricsChange {
    pub(super) changed: bool,
    pub(super) overflow_toggled: bool,
}

struct UnwrappedEntry {
    weak: Weak<()>,
    scroll_x: f32,
    viewport_width: f32,
    content_width: f32,
}

impl UnwrappedEntry {
    fn is_live(&self) -> bool {
        self.weak.strong_count() > 0
    }

    fn max_scroll(&self) -> f32 {
        (self.content_width - self.viewport_width).max(0.)
    }

    fn has_overflow(&self) -> bool {
        self.viewport_width > 0. && self.content_width - self.viewport_width >= 1.
    }

    fn thumb(&self, track_width: f32, min_thumb: f32) -> Option<(f32, f32)> {
        if !self.has_overflow() || track_width <= 0. {
            return None;
        }
        let thumb_width = (track_width * self.viewport_width / self.content_width)
            .max(min_thumb)
            .min(track_width);
        let max_scroll = self.max_scroll();
        let ratio = if max_scroll == 0. {
            0.
        } else {
            (self.scroll_x / max_scroll).clamp(0., 1.)
        };
        Some(((track_width - thumb_width) * ratio, thumb_width))
    }
}

#[derive(Clone, Copy, Debug)]
struct ScrollbarDrag {
    code_id: CodeBlockId,
    track_left: f32,
    track_width: f32,
    grab_offset: f32,
    min_thumb: f32,
}

struct CopiedEntry {
    weak: Weak<()>,
    generation: u64,
    _timer: Option<Task<()>>,
}

/// Per-view interactive state for rendered code blocks: hovered rows,
/// unwrapped blocks with shared horizontal scroll offsets, and active
/// copy-confirmation timers.
#[derive(Default)]
pub(super) struct CodeBlockStates {
    hovered_rows: Vec<(CodeBlockId, Weak<()>, usize)>,
    unwrapped: HashMap<CodeBlockId, UnwrappedEntry>,
    scrollbar_drag: Option<ScrollbarDrag>,
    copied: HashMap<CodeBlockId, CopiedEntry>,
    next_generation: u64,
}

impl CodeBlockStates {
    pub(super) fn is_hovered(&self, code: &Arc<CodeBlock>) -> bool {
        let id = CodeBlockId::of(code);
        self.hovered_rows
            .iter()
            .any(|(block, weak, _)| *block == id && weak.strong_count() > 0)
    }

    pub(super) fn set_row_hovered(
        &mut self,
        code: &Arc<CodeBlock>,
        row: usize,
        hovered: bool,
    ) -> bool {
        self.hovered_rows
            .retain(|(_, weak, _)| weak.strong_count() > 0);
        let id = CodeBlockId::of(code);
        let was_hovered = self.hovered_rows.iter().any(|(block, _, _)| *block == id);
        let mut other_cleared = false;
        if hovered {
            let before = self.hovered_rows.len();
            self.hovered_rows.retain(|(block, _, _)| *block == id);
            other_cleared = self.hovered_rows.len() != before;
            if !self
                .hovered_rows
                .iter()
                .any(|(block, _, existing)| *block == id && *existing == row)
            {
                self.hovered_rows
                    .push((id, Arc::downgrade(&code.state_token), row));
            }
        } else {
            self.hovered_rows
                .retain(|(block, _, existing)| !(*block == id && *existing == row));
        }
        let now_hovered = self.hovered_rows.iter().any(|(block, _, _)| *block == id);
        other_cleared || was_hovered != now_hovered
    }

    pub(super) fn clear_hover(&mut self) -> bool {
        let had = self
            .hovered_rows
            .iter()
            .any(|(_, weak, _)| weak.strong_count() > 0);
        self.hovered_rows.clear();
        had
    }

    pub(super) fn is_unwrapped(&self, code: &Arc<CodeBlock>) -> bool {
        self.unwrapped
            .get(&CodeBlockId::of(code))
            .is_some_and(UnwrappedEntry::is_live)
    }

    pub(super) fn toggle_wrap(&mut self, code: &Arc<CodeBlock>) -> bool {
        self.prune();
        let id = CodeBlockId::of(code);
        if self.unwrapped.get(&id).is_some_and(UnwrappedEntry::is_live) {
            self.unwrapped.remove(&id);
            if self.scrollbar_drag.is_some_and(|drag| drag.code_id == id) {
                self.scrollbar_drag = None;
            }
            false
        } else {
            self.unwrapped.insert(
                id,
                UnwrappedEntry {
                    weak: Arc::downgrade(&code.state_token),
                    scroll_x: 0.,
                    viewport_width: 0.,
                    content_width: 0.,
                },
            );
            true
        }
    }

    pub(super) fn scroll_x(&self, code: &Arc<CodeBlock>) -> f32 {
        self.unwrapped
            .get(&CodeBlockId::of(code))
            .filter(|entry| entry.is_live())
            .map_or(0., |entry| entry.scroll_x)
    }

    pub(super) fn max_scroll_x(&self, code: &Arc<CodeBlock>) -> f32 {
        self.unwrapped
            .get(&CodeBlockId::of(code))
            .filter(|entry| entry.is_live())
            .map_or(0., UnwrappedEntry::max_scroll)
    }

    pub(super) fn has_overflow(&self, code: &Arc<CodeBlock>) -> bool {
        self.unwrapped
            .get(&CodeBlockId::of(code))
            .is_some_and(|entry| entry.is_live() && entry.has_overflow())
    }

    pub(super) fn metrics(&self, code: &Arc<CodeBlock>) -> (f32, f32) {
        self.unwrapped
            .get(&CodeBlockId::of(code))
            .filter(|entry| entry.is_live())
            .map_or((0., 0.), |entry| {
                (entry.viewport_width, entry.content_width)
            })
    }

    pub(super) fn scrollbar_thumb(
        &self,
        code: &Arc<CodeBlock>,
        track_width: f32,
        min_thumb: f32,
    ) -> Option<(f32, f32)> {
        self.unwrapped
            .get(&CodeBlockId::of(code))
            .filter(|entry| entry.is_live())
            .and_then(|entry| entry.thumb(track_width, min_thumb))
    }

    pub(super) fn update_metrics(
        &mut self,
        code: &Arc<CodeBlock>,
        viewport_width: f32,
        content_width: f32,
    ) -> MetricsChange {
        let Some(entry) = self
            .unwrapped
            .get_mut(&CodeBlockId::of(code))
            .filter(|entry| entry.is_live())
        else {
            return MetricsChange::default();
        };
        let had_overflow = entry.has_overflow();
        let viewport_changed = (entry.viewport_width - viewport_width).abs() > 0.5;
        let content_changed = (entry.content_width - content_width).abs() > 0.5;
        if !viewport_changed && !content_changed {
            return MetricsChange::default();
        }
        entry.viewport_width = viewport_width;
        entry.content_width = content_width;
        entry.scroll_x = entry.scroll_x.clamp(0., entry.max_scroll());
        MetricsChange {
            changed: true,
            overflow_toggled: had_overflow != entry.has_overflow(),
        }
    }

    pub(super) fn scroll_by(&mut self, code: &Arc<CodeBlock>, delta_x: f32) -> bool {
        let Some(entry) = self
            .unwrapped
            .get_mut(&CodeBlockId::of(code))
            .filter(|entry| entry.is_live())
        else {
            return false;
        };
        let next = (entry.scroll_x + delta_x).clamp(0., entry.max_scroll());
        if (next - entry.scroll_x).abs() <= 0.01 {
            return false;
        }
        entry.scroll_x = next;
        true
    }

    pub(super) fn is_dragging_scrollbar(&self, code: &Arc<CodeBlock>) -> bool {
        self.scrollbar_drag
            .is_some_and(|drag| drag.code_id == CodeBlockId::of(code))
    }

    pub(super) fn begin_scrollbar_drag(
        &mut self,
        code: &Arc<CodeBlock>,
        pointer_x: f32,
        track_left: f32,
        track_width: f32,
        min_thumb: f32,
    ) -> bool {
        let id = CodeBlockId::of(code);
        let Some((thumb_left, thumb_width)) = self.scrollbar_thumb(code, track_width, min_thumb)
        else {
            return false;
        };
        let local = pointer_x - track_left;
        let grab_offset = if local >= thumb_left && local <= thumb_left + thumb_width {
            local - thumb_left
        } else {
            thumb_width / 2.
        };
        self.scrollbar_drag = Some(ScrollbarDrag {
            code_id: id,
            track_left,
            track_width,
            grab_offset,
            min_thumb,
        });
        self.update_scrollbar_drag(pointer_x);
        true
    }

    pub(super) fn update_scrollbar_drag(&mut self, pointer_x: f32) -> bool {
        let Some(drag) = self.scrollbar_drag else {
            return false;
        };
        let Some(entry) = self
            .unwrapped
            .get_mut(&drag.code_id)
            .filter(|entry| entry.is_live())
        else {
            self.scrollbar_drag = None;
            return false;
        };
        let Some((_, thumb_width)) = entry.thumb(drag.track_width, drag.min_thumb) else {
            return false;
        };
        let available = (drag.track_width - thumb_width).max(1.);
        let start = pointer_x - drag.track_left - drag.grab_offset;
        let ratio = (start / available).clamp(0., 1.);
        let next = ratio * entry.max_scroll();
        if (next - entry.scroll_x).abs() <= 0.01 {
            return false;
        }
        entry.scroll_x = next;
        true
    }

    pub(super) fn end_scrollbar_drag(&mut self) -> bool {
        self.scrollbar_drag.take().is_some()
    }

    pub(super) fn is_copied(&self, code: &Arc<CodeBlock>) -> bool {
        self.copied
            .get(&CodeBlockId::of(code))
            .is_some_and(|entry| entry.weak.strong_count() > 0)
    }

    pub(super) fn begin_copied(&mut self, code: &Arc<CodeBlock>) -> u64 {
        self.prune();
        self.next_generation = self.next_generation.wrapping_add(1);
        let generation = self.next_generation;
        self.copied.insert(
            CodeBlockId::of(code),
            CopiedEntry {
                weak: Arc::downgrade(&code.state_token),
                generation,
                _timer: None,
            },
        );
        generation
    }

    pub(super) fn set_copied_timer(
        &mut self,
        code_id: CodeBlockId,
        generation: u64,
        task: Task<()>,
    ) {
        if let Some(entry) = self.copied.get_mut(&code_id)
            && entry.generation == generation
        {
            entry._timer = Some(task);
        }
    }

    pub(super) fn finish_copied(&mut self, code_id: CodeBlockId, generation: u64) -> bool {
        if self
            .copied
            .get(&code_id)
            .is_some_and(|entry| entry.generation == generation)
        {
            self.copied.remove(&code_id);
            true
        } else {
            false
        }
    }

    pub(super) fn prune(&mut self) {
        self.hovered_rows
            .retain(|(_, weak, _)| weak.strong_count() > 0);
        self.unwrapped.retain(|_, entry| entry.is_live());
        self.copied.retain(|_, entry| entry.weak.strong_count() > 0);
        if let Some(drag) = self.scrollbar_drag
            && !self.unwrapped.contains_key(&drag.code_id)
        {
            self.scrollbar_drag = None;
        }
    }

    fn is_empty(&self) -> bool {
        self.hovered_rows.is_empty()
            && self.unwrapped.is_empty()
            && self.scrollbar_drag.is_none()
            && self.copied.is_empty()
    }
}

/// App-wide code block interaction state for Compact Markdown surfaces, keyed
/// by their owning view's [`EntityId`].
#[derive(Default)]
pub(super) struct CompactCodeStates {
    views: HashMap<EntityId, CodeBlockStates>,
}

impl Global for CompactCodeStates {}

impl CompactCodeStates {
    pub(super) fn view(&self, view: EntityId) -> Option<&CodeBlockStates> {
        self.views.get(&view)
    }

    pub(super) fn view_mut(&mut self, view: EntityId) -> &mut CodeBlockStates {
        self.views.entry(view).or_default()
    }

    pub(super) fn update_scrollbar_drags(&mut self, pointer_x: f32) -> Vec<EntityId> {
        let mut changed = Vec::new();
        for (&view, states) in &mut self.views {
            if states.update_scrollbar_drag(pointer_x) {
                changed.push(view);
            }
        }
        changed
    }

    pub(super) fn end_scrollbar_drags(&mut self) -> Vec<EntityId> {
        let mut changed = Vec::new();
        for (&view, states) in &mut self.views {
            if states.end_scrollbar_drag() {
                changed.push(view);
            }
        }
        changed
    }

    pub(super) fn clear_all_hovers(&mut self) -> Vec<EntityId> {
        let mut changed = Vec::new();
        for (&view, states) in &mut self.views {
            if states.clear_hover() {
                changed.push(view);
            }
        }
        self.prune_empty_views();
        changed
    }

    pub(super) fn prune_empty_views(&mut self) {
        for states in self.views.values_mut() {
            states.prune();
        }
        self.views.retain(|_, states| !states.is_empty());
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.views.is_empty()
    }
}
