use std::cell::RefCell;
use std::collections::VecDeque;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    canvas, div, fill, point, px, relative, rems, size, svg, AnyElement, App, AvailableSpace,
    Bounds, BoxShadow, ClipboardItem, ContentMask, Context, CursorStyle, DispatchPhase, Element,
    ElementId, ElementInputHandler, Entity, EntityInputHandler, FocusHandle, Focusable, Font,
    FontWeight, GlobalElementId, Hsla, InspectorElementId, IntoElement, KeyDownEvent, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Rems, Render,
    RenderOnce, ScrollHandle, SharedString, Size, Style, Task, TextAlign, TextRun, UTF16Selection,
    Window, WrappedLine,
};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::text_util;
use crate::theme;
use crate::ui::button::{Button, PressHandler};
use crate::ui::scrollbar::Scrollbar;
use crate::ui::select::rerender_after_draw;
use crate::ui::tooltip::Tooltip;
use crate::{Copy, Cut, Paste, Redo, SelectAll, Undo};

pub type KeyDownInterceptor = Rc<dyn Fn(&KeyDownEvent, &mut Window, &mut App) -> bool>;

const AUTO_SCROLL_TICK: Duration = Duration::from_millis(16);
const MAX_UNDO_STEPS: usize = 1000;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Selection {
    anchor: usize,
    caret: usize,
}

impl Selection {
    fn range(self) -> Range<usize> {
        self.anchor.min(self.caret)..self.anchor.max(self.caret)
    }

    fn is_empty(self) -> bool {
        self.anchor == self.caret
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MarkedText {
    range: Range<usize>,
    original: String,
    /// The selection from before the composition, which undo restores.
    selection_before: Selection,
}

/// What an edit does, which decides whether it merges into the step before
/// it. The history follows gpui-kit's undo manager.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EditIntent {
    Typing,
    Backspace,
    DeleteForward,
    Atomic,
}

/// One replacement, at a byte offset of the text as it stood before it.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Change {
    offset: usize,
    old_text: String,
    new_text: String,
}

/// One undo step: its changes in the order they were made, and the selection
/// from before the first and after the last.
#[derive(Clone, Debug, Eq, PartialEq)]
struct UndoStep {
    intent: EditIntent,
    changes: Vec<Change>,
    selection_before: Selection,
    selection_after: Selection,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct History {
    undo: VecDeque<UndoStep>,
    redo: Vec<UndoStep>,
    /// The last step can take an adjacent edit of the same intent.
    run_open: bool,
    /// Nothing but its own edits has happened since the last step.
    just_recorded: bool,
}

impl History {
    fn record(&mut self, change: Change, intent: EditIntent, before: Selection, after: Selection) {
        self.redo.clear();
        let merge = self.run_open
            && self.undo.back().is_some_and(|step| {
                step.intent == intent
                    && step
                        .changes
                        .last()
                        .is_some_and(|previous| is_adjacent(intent, previous, &change))
            });
        match self.undo.back_mut() {
            Some(step) if merge => {
                step.changes.push(change);
                step.selection_after = after;
            }
            _ => {
                if self.undo.len() == MAX_UNDO_STEPS {
                    self.undo.pop_front();
                }
                self.undo.push_back(UndoStep {
                    intent,
                    changes: vec![change],
                    selection_before: before,
                    selection_after: after,
                });
            }
        }
        self.run_open = intent != EditIntent::Atomic;
        self.just_recorded = true;
    }

    /// Adds `change` to the last step, as part of the edit it records.
    fn amend(&mut self, change: Change, after: Selection) {
        self.redo.clear();
        if let Some(step) = self.undo.back_mut() {
            step.changes.push(change);
            step.selection_after = after;
        }
    }

    fn break_run(&mut self) {
        self.run_open = false;
        self.just_recorded = false;
    }
}

/// Whether `current` continues `previous` as one gesture, as gpui-kit's
/// `is_adjacent` decides.
fn is_adjacent(intent: EditIntent, previous: &Change, current: &Change) -> bool {
    match intent {
        EditIntent::Typing => {
            previous.old_text.is_empty()
                && current.old_text.is_empty()
                && !previous.new_text.contains('\n')
                && !current.new_text.contains('\n')
                && previous.offset + previous.new_text.len() == current.offset
        }
        EditIntent::Backspace => {
            previous.new_text.is_empty()
                && current.new_text.is_empty()
                && current.offset + current.old_text.len() == previous.offset
        }
        EditIntent::DeleteForward => {
            previous.new_text.is_empty()
                && current.new_text.is_empty()
                && current.offset == previous.offset
        }
        EditIntent::Atomic => false,
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct TextBuffer {
    text: String,
    selection: Selection,
    marked: Option<MarkedText>,
    edited: bool,
    multiline: bool,
    history: History,
}

impl TextBuffer {
    fn reset(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.move_to_end();
        self.marked = None;
        self.edited = false;
        self.history = History::default();
    }

    /// Replaces the whole text as one step. Straight after an edit, the text
    /// lowercased is the handle fields correcting that edit, so it joins the
    /// edit's step and one undo takes back both.
    fn set_text(&mut self, text: &str) -> bool {
        self.end_composition();
        let correction = self.history.just_recorded && text == self.text.to_lowercase();
        let changed = if correction {
            let change = Change {
                offset: 0,
                old_text: self.text.clone(),
                new_text: text.to_owned(),
            };
            let changed = self.splice(0..self.text.len(), text);
            if changed {
                self.history.amend(change, self.selection);
            }
            changed
        } else {
            self.edit(0..self.text.len(), text, EditIntent::Atomic)
        };
        self.history.break_run();
        changed
    }

    /// Writes `new_text` over `range` and leaves the caret after it; the
    /// caller records the change.
    fn splice(&mut self, range: Range<usize>, new_text: &str) -> bool {
        let changed = self.text[range.clone()] != *new_text;
        self.text.replace_range(range.clone(), new_text);
        let end = range.start + new_text.len();
        self.selection = Selection {
            anchor: end,
            caret: end,
        };
        self.edited |= changed;
        changed
    }

    /// Replaces `range` with `new_text`, leaving the caret after it, and
    /// records the change. Every edit but an IME composition's goes through
    /// here.
    fn edit(&mut self, range: Range<usize>, new_text: &str, intent: EditIntent) -> bool {
        self.end_composition();
        let before = self.selection;
        let change = Change {
            offset: range.start,
            old_text: self.text[range.clone()].to_owned(),
            new_text: new_text.to_owned(),
        };
        let changed = self.splice(range, new_text);
        if changed {
            self.history.record(change, intent, before, self.selection);
        } else if self.selection != before {
            self.history.break_run();
        }
        changed
    }

    /// Ends an IME composition, recording what it left as one step from the
    /// text it replaced; a composition that leaves that text records nothing.
    fn end_composition(&mut self) {
        let Some(marked) = self.marked.take() else {
            return;
        };
        let composed = &self.text[marked.range.clone()];
        if composed != marked.original {
            let change = Change {
                offset: marked.range.start,
                old_text: marked.original,
                new_text: composed.to_owned(),
            };
            self.history.record(
                change,
                EditIntent::Atomic,
                marked.selection_before,
                self.selection,
            );
        }
    }

    /// Ends any composition and run before the caret or selection moves.
    fn leave_run(&mut self) {
        self.end_composition();
        self.history.break_run();
    }

    fn undo(&mut self) -> bool {
        if self.marked.is_some() {
            return false;
        }
        let Some(step) = self.history.undo.pop_back() else {
            return false;
        };
        for change in step.changes.iter().rev() {
            let end = change.offset + change.new_text.len();
            self.splice(change.offset..end, &change.old_text);
        }
        self.selection = step.selection_before;
        self.edited = true;
        self.history.redo.push(step);
        self.history.break_run();
        true
    }

    fn redo(&mut self) -> bool {
        if self.marked.is_some() {
            return false;
        }
        let Some(step) = self.history.redo.pop() else {
            return false;
        };
        for change in &step.changes {
            let end = change.offset + change.old_text.len();
            self.splice(change.offset..end, &change.new_text);
        }
        self.selection = step.selection_after;
        self.edited = true;
        self.history.undo.push_back(step);
        self.history.break_run();
        true
    }

    fn move_to_end(&mut self) {
        let end = self.text.len();
        self.selection = Selection {
            anchor: end,
            caret: end,
        };
    }

    fn unmark_text(&mut self) {
        self.end_composition();
    }

    fn text_for_range(
        &self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
    ) -> String {
        let range = text_util::range_from_utf16(&self.text, &range_utf16);
        adjusted_range.replace(text_util::range_to_utf16(&self.text, &range));
        self.text[range].to_string()
    }

    fn selected_text_range(&self) -> UTF16Selection {
        UTF16Selection {
            range: text_util::range_to_utf16(&self.text, &self.selection.range()),
            reversed: self.selection.caret < self.selection.anchor,
        }
    }

    fn marked_text_range(&self) -> Option<Range<usize>> {
        self.marked
            .as_ref()
            .map(|marked| text_util::range_to_utf16(&self.text, &marked.range))
    }

    fn replace_text_in_range(&mut self, range_utf16: Option<Range<usize>>, new_text: &str) -> bool {
        let range = self.resolve_range(range_utf16);
        let new_text = normalize_input_text(new_text, self.multiline);
        let intent = if range.is_empty() && !new_text.is_empty() && !new_text.contains('\n') {
            EditIntent::Typing
        } else {
            EditIntent::Atomic
        };
        self.replace(range, &new_text, intent)
    }

    /// Replaces `range`; over the composed text, it commits the composition.
    fn replace(&mut self, range: Range<usize>, new_text: &str, intent: EditIntent) -> bool {
        let Some(marked) = self.marked.as_mut().filter(|marked| marked.range == range) else {
            return self.edit(range, new_text, intent);
        };
        marked.range = range.start..range.start + new_text.len();
        let changed = self.splice(range, new_text);
        self.end_composition();
        changed
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
    ) -> bool {
        let range = self.resolve_range(range_utf16);
        if self
            .marked
            .as_ref()
            .is_some_and(|marked| marked.range != range)
        {
            self.end_composition();
        }
        let (original, selection_before) = match self.marked.take() {
            Some(marked) => (marked.original, marked.selection_before),
            None => (self.text[range.clone()].to_string(), self.selection),
        };
        let new_text = normalize_input_text(new_text, self.multiline);
        let changed = self.splice(range.clone(), &new_text);
        let marked_range = range.start..range.start + new_text.len();
        self.marked = Some(MarkedText {
            range: marked_range.clone(),
            original,
            selection_before,
        });
        self.selection = new_selected_range_utf16
            .map(|selection| {
                let selection = text_util::range_from_utf16(&new_text, &selection);
                Selection {
                    anchor: range.start + selection.start,
                    caret: range.start + selection.end,
                }
            })
            .unwrap_or_else(|| Selection {
                anchor: marked_range.end,
                caret: marked_range.end,
            });
        if new_text.is_empty() {
            self.end_composition();
        }
        changed
    }

    fn character_index_utf16(&self) -> usize {
        text_util::offset_to_utf16(&self.text, self.selection.caret)
    }

    fn resolve_range(&self, range_utf16: Option<Range<usize>>) -> Range<usize> {
        range_utf16
            .map(|range| text_util::range_from_utf16(&self.text, &range))
            .or_else(|| self.marked.as_ref().map(|marked| marked.range.clone()))
            .unwrap_or_else(|| self.selection.range())
    }

    fn selected_text(&self) -> Option<&str> {
        let range = self.selection.range();
        (!range.is_empty()).then(|| &self.text[range])
    }

    fn replace_selection(&mut self, new_text: &str) -> bool {
        let range = self.resolve_range(None);
        let new_text = normalize_input_text(new_text, self.multiline);
        self.replace(range, &new_text, EditIntent::Atomic)
    }

    fn select_all(&mut self) {
        self.leave_run();
        self.selection = Selection {
            anchor: 0,
            caret: self.text.len(),
        };
    }

    fn select_word_at(&mut self, position: usize) {
        let position = position.min(self.text.len());
        let range = self
            .text
            .split_word_bound_indices()
            .find_map(|(start, segment)| {
                let end = start + segment.len();
                (start <= position && (position < end || end == self.text.len()))
                    .then_some(start..end)
            })
            .unwrap_or(position..position);
        self.leave_run();
        self.selection = Selection {
            anchor: range.start,
            caret: range.end,
        };
    }

    fn select_line_at(&mut self, position: usize) {
        let position = position.min(self.text.len());
        let start = self.text[..position]
            .rfind('\n')
            .map_or(0, |offset| offset + 1);
        let end = self.text[position..]
            .find('\n')
            .map_or(self.text.len(), |offset| position + offset + 1);
        self.leave_run();
        self.selection = Selection {
            anchor: start,
            caret: end,
        };
    }

    fn move_left(&mut self, boundary: Boundary, extend: bool) {
        if !extend && !self.selection.is_empty() {
            let start = self.selection.range().start;
            self.leave_run();
            self.selection = Selection {
                anchor: start,
                caret: start,
            };
            return;
        }
        let target = match boundary {
            Boundary::Grapheme => {
                text_util::prev_grapheme_boundary(&self.text, self.selection.caret)
            }
            Boundary::Word => previous_word_boundary(&self.text, self.selection.caret),
            Boundary::Line => 0,
        };
        self.move_to(target, extend);
    }

    fn move_right(&mut self, boundary: Boundary, extend: bool) {
        if !extend && !self.selection.is_empty() {
            let end = self.selection.range().end;
            self.leave_run();
            self.selection = Selection {
                anchor: end,
                caret: end,
            };
            return;
        }
        let target = match boundary {
            Boundary::Grapheme => {
                text_util::next_grapheme_boundary(&self.text, self.selection.caret)
            }
            Boundary::Word => next_word_boundary(&self.text, self.selection.caret),
            Boundary::Line => self.text.len(),
        };
        self.move_to(target, extend);
    }

    fn move_to(&mut self, target: usize, extend: bool) {
        let target = target.min(self.text.len());
        self.leave_run();
        if extend {
            self.selection.caret = target;
        } else {
            self.selection = Selection {
                anchor: target,
                caret: target,
            };
        }
    }

    fn delete_left(&mut self, boundary: Boundary) -> bool {
        let selection = self.selection.range();
        let (range, intent) = if selection.is_empty() {
            let start = match boundary {
                Boundary::Grapheme => {
                    text_util::prev_grapheme_boundary(&self.text, selection.start)
                }
                Boundary::Word => previous_word_boundary(&self.text, selection.start),
                Boundary::Line => 0,
            };
            (start..selection.start, boundary.delete_intent(true))
        } else {
            (selection, EditIntent::Atomic)
        };
        self.delete_range(range, intent)
    }

    fn delete_right(&mut self, boundary: Boundary) -> bool {
        let selection = self.selection.range();
        let (range, intent) = if selection.is_empty() {
            let end = match boundary {
                Boundary::Grapheme => text_util::next_grapheme_boundary(&self.text, selection.end),
                Boundary::Word => next_word_boundary(&self.text, selection.end),
                Boundary::Line => self.text.len(),
            };
            (selection.end..end, boundary.delete_intent(false))
        } else {
            (selection, EditIntent::Atomic)
        };
        self.delete_range(range, intent)
    }

    fn delete_range(&mut self, range: Range<usize>, intent: EditIntent) -> bool {
        if range.is_empty() {
            return false;
        }
        self.edit(range, "", intent)
    }
}

#[derive(Clone, Copy)]
enum Boundary {
    Grapheme,
    Word,
    Line,
}

impl Boundary {
    /// Only single-grapheme deletes merge; word and line deletes are steps
    /// of their own.
    fn delete_intent(self, backward: bool) -> EditIntent {
        match (self, backward) {
            (Boundary::Grapheme, true) => EditIntent::Backspace,
            (Boundary::Grapheme, false) => EditIntent::DeleteForward,
            _ => EditIntent::Atomic,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextFieldKind {
    #[default]
    Input,
    Textarea {
        rows: u8,
    },
}

impl TextFieldKind {
    fn multiline(self) -> bool {
        matches!(self, Self::Textarea { .. })
    }

    fn rows(self) -> u8 {
        match self {
            Self::Input => 1,
            Self::Textarea { rows } => rows.max(1),
        }
    }
}

/// What a field's text is shaped from: the text shown (the value, or the
/// placeholder while the value is empty) and the style it inherits.
#[derive(Clone, Debug, PartialEq)]
struct FieldTextKey {
    text: SharedString,
    placeholder: bool,
    multiline: bool,
    font: Font,
    color: Hsla,
    font_size: Pixels,
    line_height: Pixels,
}

/// Where an index shows on a line: its visual row and its x on that row.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Spot {
    row: usize,
    x: Pixels,
}

/// A glyph's source index and its box on its visual row.
#[derive(Clone, Copy, Debug)]
struct GlyphBox {
    index: usize,
    row: usize,
    left: Pixels,
    right: Pixels,
}

/// A grapheme boundary of a line and where it shows.
#[derive(Clone, Copy, Debug)]
struct Stop {
    index: usize,
    /// Where it shows as the start of the text after it.
    after: Spot,
    /// Where it shows as the end of the text before it, which is the end of
    /// the previous row where a soft wrap breaks the line.
    before: Spot,
}

/// A line's glyphs where they are painted, split into rows at the same
/// glyphs as the painter splits them, and where each boundary shows. It
/// lays text out left to right; glyphs out of source order, as right-to-left
/// runs shape, stay in bounds and map only to grapheme boundaries.
#[derive(Debug)]
struct LineGeometry {
    /// The right end of each visual row.
    row_ends: Vec<Pixels>,
    /// In visual order.
    glyphs: Vec<GlyphBox>,
    /// In source order.
    stops: Vec<Stop>,
}

impl LineGeometry {
    /// `glyphs` are each glyph's source index and x in the unwrapped line,
    /// in visual order; `wraps` are the glyphs that start a new row.
    fn new(source: &str, glyphs: &[(usize, Pixels)], wraps: &[usize], width: Pixels) -> Self {
        let mut wraps = wraps.iter().peekable();
        let mut row_ends = vec![px(0.)];
        let mut row_x = px(0.);
        let mut boxes = Vec::with_capacity(glyphs.len());
        for (ordinal, &(index, x)) in glyphs.iter().enumerate() {
            if wraps.next_if(|wrap| **wrap == ordinal).is_some() {
                row_ends.push(px(0.));
                row_x = x;
            }
            let left = x - row_x;
            let right =
                (glyphs.get(ordinal + 1).map_or(width, |(_, next)| *next) - row_x).max(left);
            let row = row_ends.len() - 1;
            row_ends[row] = row_ends[row].max(right);
            boxes.push(GlyphBox {
                index,
                row,
                left,
                right,
            });
        }
        let mut by_index = (0..boxes.len()).collect::<Vec<_>>();
        by_index.sort_by_key(|ordinal| (boxes[*ordinal].index, *ordinal));
        let stops = source
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .chain([source.len()])
            .map(|index| {
                let next = by_index.partition_point(|ordinal| boxes[*ordinal].index < index);
                let after = by_index.get(next).map(|ordinal| {
                    let glyph = boxes[*ordinal];
                    Spot {
                        row: glyph.row,
                        x: glyph.left,
                    }
                });
                let before = next.checked_sub(1).map(|previous| {
                    let glyph = boxes[by_index[previous]];
                    Spot {
                        row: glyph.row,
                        x: glyph.right,
                    }
                });
                Stop {
                    index,
                    after: after.or(before).unwrap_or_default(),
                    before: before.or(after).unwrap_or_default(),
                }
            })
            .collect();
        Self {
            row_ends,
            glyphs: boxes,
            stops,
        }
    }

    fn len(&self) -> usize {
        self.stops.last().map_or(0, |stop| stop.index)
    }

    fn rows(&self) -> usize {
        self.row_ends.len()
    }

    /// Where `index` shows: after the text before it with `upstream`, else
    /// before the text after it.
    fn spot(&self, index: usize, upstream: bool) -> Spot {
        let stop = self
            .stops
            .partition_point(|stop| stop.index < index)
            .min(self.stops.len() - 1);
        let stop = self.stops[stop];
        if upstream {
            stop.before
        } else {
            stop.after
        }
    }

    /// The index at `x` on a row: the boundary closest to it, or with
    /// `under`, the start of the grapheme it falls on. Also whether it shows
    /// there as the end of the text before it.
    fn index_at(&self, row: usize, x: Pixels, under: bool) -> (usize, bool) {
        if !under {
            let mut closest: Option<(Pixels, usize, bool)> = None;
            for stop in &self.stops {
                for (spot, upstream) in [(stop.after, false), (stop.before, true)] {
                    if spot.row != row || (upstream && stop.before == stop.after) {
                        continue;
                    }
                    let distance = (spot.x - x).abs();
                    if closest.is_none_or(|(closest, ..)| distance < closest) {
                        closest = Some((distance, stop.index, upstream));
                    }
                }
            }
            if let Some((_, index, upstream)) = closest {
                return (index, upstream);
            }
        }
        // The grapheme holding the glyph nearest `x`; also where no boundary
        // shows on the row, as when a wrap falls inside a grapheme.
        let distance = |glyph: &GlyphBox| {
            if x < glyph.left {
                glyph.left - x
            } else if x >= glyph.right {
                x - glyph.right
            } else {
                px(0.)
            }
        };
        let index = self
            .glyphs
            .iter()
            .filter(|glyph| glyph.row == row)
            .min_by(|a, b| {
                distance(a)
                    .partial_cmp(&distance(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map_or(0, |glyph| glyph.index);
        let stop = self
            .stops
            .partition_point(|stop| stop.index <= index)
            .max(1)
            - 1;
        (self.stops[stop].index, false)
    }

    /// The spans a range covers on each row, as `(row, left, right)`, with a
    /// block of `newline_width` at the end of the last row for a selected
    /// line break.
    fn spans(
        &self,
        range: Range<usize>,
        newline_width: Option<Pixels>,
    ) -> Vec<(usize, Pixels, Pixels)> {
        let mut boxes = self
            .glyphs
            .iter()
            .filter(|glyph| range.contains(&glyph.index))
            .map(|glyph| (glyph.row, glyph.left, glyph.right))
            .collect::<Vec<_>>();
        if let Some(width) = newline_width {
            let row = self.rows() - 1;
            boxes.push((row, self.row_ends[row], self.row_ends[row] + width));
        }
        boxes.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        });
        let mut spans: Vec<(usize, Pixels, Pixels)> = Vec::with_capacity(boxes.len());
        for (row, left, right) in boxes {
            match spans.last_mut() {
                Some(span) if span.0 == row && left <= span.2 => span.2 = span.2.max(right),
                _ => spans.push((row, left, right)),
            }
        }
        spans
    }
}

/// One hard line of a field's text, shaped and wrapped.
struct FieldLine {
    start: usize,
    top: Pixels,
    layout: WrappedLine,
    geometry: LineGeometry,
}

impl FieldLine {
    fn new(source: &str, start: usize, top: Pixels, layout: WrappedLine) -> Self {
        let unwrapped = &layout.unwrapped_layout;
        let mut run_starts = Vec::with_capacity(unwrapped.runs.len());
        let mut glyphs = Vec::new();
        for run in &unwrapped.runs {
            run_starts.push(glyphs.len());
            glyphs.extend(
                run.glyphs
                    .iter()
                    .map(|glyph| (glyph.index, glyph.position.x)),
            );
        }
        let wraps = layout
            .wrap_boundaries()
            .iter()
            .map(|boundary| run_starts[boundary.run_ix] + boundary.glyph_ix)
            .collect::<Vec<_>>();
        let geometry = LineGeometry::new(source, &glyphs, &wraps, unwrapped.width);
        Self {
            start,
            top,
            layout,
            geometry,
        }
    }

    fn end(&self) -> usize {
        self.start + self.geometry.len()
    }

    fn row_top(&self, row: usize, line_height: Pixels) -> Pixels {
        self.top + line_height * row as f32
    }
}

/// A field's text shaped once, at the width the field gives it. Painting,
/// hit-testing, caret movement and IME all read this one layout.
struct FieldText {
    key: FieldTextKey,
    wrap_width: Option<Pixels>,
    lines: Vec<FieldLine>,
    len: usize,
    /// The width of the block a selected line break paints.
    newline_width: Pixels,
    size: Size<Pixels>,
}

impl FieldText {
    fn new(key: FieldTextKey, wrap_width: Option<Pixels>, window: &Window) -> Self {
        let text_system = window.text_system();
        let run = TextRun {
            len: key.text.len(),
            font: key.font.clone(),
            color: key.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let mut shaped = text_system
            .shape_text(key.text.clone(), key.font_size, &[run], wrap_width, None)
            .unwrap_or_default()
            .into_iter();
        let mut lines = Vec::new();
        let mut start = 0;
        let mut top = px(0.);
        let mut width = px(0.);
        for source in key.text.split('\n') {
            let line = FieldLine::new(source, start, top, shaped.next().unwrap_or_default());
            start += source.len() + 1;
            top += key.line_height * line.geometry.rows() as f32;
            width = width.max(line.layout.width());
            lines.push(line);
        }
        let font_id = text_system.resolve_font(&key.font);
        let newline_width = text_system
            .advance(font_id, key.font_size, ' ')
            .map_or(key.font_size / 2., |advance| advance.width);
        Self {
            len: key.text.len(),
            size: size(wrap_width.unwrap_or(width), top),
            key,
            wrap_width,
            lines,
            newline_width,
        }
    }

    fn line_height(&self) -> Pixels {
        self.key.line_height
    }

    fn line_index(&self, index: usize) -> usize {
        self.lines
            .partition_point(|line| line.start <= index)
            .max(1)
            - 1
    }

    /// The top left of the caret at `index`, relative to the text's origin.
    fn caret_position(&self, index: usize, upstream: bool) -> Point<Pixels> {
        if self.key.placeholder {
            return Point::default();
        }
        let line = &self.lines[self.line_index(index)];
        let spot = line
            .geometry
            .spot(index.saturating_sub(line.start), upstream);
        point(spot.x, line.row_top(spot.row, self.line_height()))
    }

    /// The index at a point relative to the text's origin, and whether it
    /// shows there as the end of the text before it. Above the text is its
    /// start, below it its end; a single-line field reads only `x`.
    fn index_for_position(&self, position: Point<Pixels>, under: bool) -> (usize, bool) {
        if self.key.placeholder {
            return (0, false);
        }
        let (line, row) = if self.key.multiline {
            if position.y < px(0.) {
                return (0, false);
            }
            let line = &self.lines[self
                .lines
                .partition_point(|line| line.top <= position.y)
                .max(1)
                - 1];
            let row = ((position.y - line.top) / self.line_height()).floor() as usize;
            if row >= line.geometry.rows() {
                return (self.len, false);
            }
            (line, row)
        } else {
            (&self.lines[0], 0)
        };
        let (index, upstream) = line.geometry.index_at(row, position.x, under);
        (line.start + index, upstream)
    }

    /// Where Up or Down from `index` lands, keeping `goal_x` (or the caret's
    /// own x), and whether it shows as the end of the text before it. From
    /// the first row Up goes to the start, from the last Down to the end.
    fn vertical_target(
        &self,
        index: usize,
        upstream: bool,
        up: bool,
        goal_x: Option<Pixels>,
    ) -> (usize, bool, Pixels) {
        if self.key.placeholder {
            return (0, false, px(0.));
        }
        let line_ix = self.line_index(index);
        let line = &self.lines[line_ix];
        let spot = line
            .geometry
            .spot(index.saturating_sub(line.start), upstream);
        let x = goal_x.unwrap_or(spot.x);
        let target = if up {
            if spot.row > 0 {
                Some((line_ix, spot.row - 1))
            } else {
                line_ix
                    .checked_sub(1)
                    .map(|previous| (previous, self.lines[previous].geometry.rows() - 1))
            }
        } else if spot.row + 1 < line.geometry.rows() {
            Some((line_ix, spot.row + 1))
        } else {
            (line_ix + 1 < self.lines.len()).then_some((line_ix + 1, 0))
        };
        match target {
            Some((line_ix, row)) => {
                let line = &self.lines[line_ix];
                let (index, upstream) = line.geometry.index_at(row, x, false);
                (line.start + index, upstream, x)
            }
            None => (if up { 0 } else { self.len }, false, x),
        }
    }

    /// The rectangles a range covers, one per run of glyphs on a visual row,
    /// relative to the text's origin. With `newline_blocks`, a selected line
    /// break adds a block at the end of its line, so empty lines show as
    /// selected.
    fn range_rects(&self, range: Range<usize>, newline_blocks: bool) -> Vec<Bounds<Pixels>> {
        let mut rects = Vec::new();
        if self.key.placeholder || range.is_empty() {
            return rects;
        }
        let line_height = self.line_height();
        for (line_ix, line) in self
            .lines
            .iter()
            .enumerate()
            .skip(self.line_index(range.start))
        {
            if line.start >= range.end {
                break;
            }
            let local = range.start.saturating_sub(line.start)..range.end - line.start;
            let newline =
                newline_blocks && range.end > line.end() && line_ix + 1 < self.lines.len();
            for (row, left, right) in line
                .geometry
                .spans(local, newline.then_some(self.newline_width))
            {
                let top = line.row_top(row, line_height);
                rects.push(Bounds::from_corners(
                    point(left, top),
                    point(right, top + line_height),
                ));
            }
        }
        rects
    }

    /// The bounds of a range's first span, or of the caret for an empty
    /// range.
    fn range_bounds(&self, range: Range<usize>, upstream: bool) -> Bounds<Pixels> {
        self.range_rects(range.clone(), false)
            .into_iter()
            .next()
            .unwrap_or_else(|| {
                Bounds::new(
                    self.caret_position(range.start, upstream),
                    size(px(0.), self.line_height()),
                )
            })
    }
}

/// What a text field shares with the element that draws it.
#[derive(Default)]
struct FieldTextState {
    shaped: Option<FieldText>,
    /// Where the text was last painted, in window coordinates, and the scroll
    /// offset it was painted at; `None` while a read-only display shows.
    origin: Option<Point<Pixels>>,
    scroll_offset: Point<Pixels>,
    /// Scroll the caret into view on the next prepaint.
    reveal_caret: bool,
}

impl FieldTextState {
    fn shape(
        &mut self,
        key: &FieldTextKey,
        wrap_width: Option<Pixels>,
        window: &Window,
    ) -> &FieldText {
        let reusable = self.shaped.as_ref().is_some_and(|shaped| {
            shaped.key == *key
                && match (shaped.wrap_width, wrap_width) {
                    // Layout rounds to device pixels, so a shape measured at
                    // the unrounded width is the one the bounds carry.
                    (Some(shaped), Some(width)) => (shaped - width).abs() < px(1.),
                    (shaped, width) => shaped == width,
                }
        });
        if !reusable {
            self.shaped = Some(FieldText::new(key.clone(), wrap_width, window));
        }
        self.shaped.as_ref().expect("shaped above")
    }
}

/// Draws a text field's editable text from one shaped layout: glyphs,
/// selection, IME underline and caret, with the field's input handler and
/// its window-wide drag listener.
struct TextFieldElement {
    field: Entity<TextField>,
}

impl IntoElement for TextFieldElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextFieldElement {
    type RequestLayoutState = FieldTextKey;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let field = self.field.read(cx);
        let key = field.text_key(window);
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        if !key.multiline {
            style.size.height = key.line_height.into();
            return (window.request_layout(style, [], cx), key);
        }
        let state = Rc::clone(&field.text_state);
        let measured = key.clone();
        let layout_id =
            window.request_measured_layout(style, move |known, available, window, _| {
                let width = known.width.or(match available.width {
                    AvailableSpace::Definite(width) => Some(width),
                    _ => None,
                });
                match width {
                    Some(width) => {
                        state
                            .borrow_mut()
                            .shape(&measured, Some(width), window)
                            .size
                    }
                    None => size(
                        px(0.),
                        measured.line_height * (measured.text.matches('\n').count() + 1) as f32,
                    ),
                }
            });
        (layout_id, key)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        key: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let field = self.field.read(cx);
        let caret = field.buffer.selection.caret;
        let row_end = field.caret_row_end == Some(caret);
        let scroll_handle = field.scroll_handle.clone();
        let mut state = field.text_state.borrow_mut();
        let caret_top = state
            .shape(key, key.multiline.then_some(bounds.size.width), window)
            .caret_position(caret, row_end)
            .y;
        let mut origin = bounds.origin;
        let mut offset = if key.multiline {
            scroll_handle.offset()
        } else {
            Point::default()
        };
        if key.multiline && std::mem::take(&mut state.reveal_caret) {
            let top = origin.y + caret_top;
            let bottom = top + key.line_height;
            let viewport = scroll_handle.bounds();
            let delta = if top < viewport.top() {
                viewport.top() - top
            } else if bottom > viewport.bottom() {
                viewport.bottom() - bottom
            } else {
                px(0.)
            };
            if delta != px(0.) {
                let max = scroll_handle.max_offset().y;
                let y = (offset.y + delta).clamp(-max, px(0.));
                origin.y += y - offset.y;
                offset.y = y;
                scroll_handle.set_offset(offset);
            }
        }
        state.origin = Some(origin);
        state.scroll_offset = offset;
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _key: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let field = self.field.read(cx);
        let focus_handle = field.focus_handle.clone();
        let focused = focus_handle.is_focused(window);
        let disabled = field.disabled;
        let selection = field.buffer.selection;
        let row_end = field.caret_row_end == Some(selection.caret);
        let marked = field
            .buffer
            .marked
            .as_ref()
            .map(|marked| marked.range.clone());
        let text_state = Rc::clone(&field.text_state);
        let state = text_state.borrow();
        let (Some(shaped), Some(origin)) = (state.shaped.as_ref(), state.origin) else {
            return;
        };
        let content = Bounds::new(origin, bounds.size);
        if !disabled {
            window.handle_input(
                &focus_handle,
                ElementInputHandler::new(content, self.field.clone()),
                cx,
            );
            let field = self.field.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture && field.read(cx).selecting {
                    field.update(cx, |field, cx| field.on_drag_move(event, window, cx));
                }
            });
        }

        let line_height = shaped.line_height();
        if focused {
            for rect in shaped.range_rects(selection.range(), true) {
                window.paint_quad(fill(
                    rect + origin,
                    theme::with_alpha(theme::accent(), 0.267),
                ));
            }
        }
        let visible = window.content_mask().bounds;
        // A one-line placeholder clips at the content edge, clear of a
        // trailing control such as a suggestions chevron.
        let clip =
            (!shaped.key.multiline && shaped.key.placeholder).then_some(ContentMask { bounds });
        window.with_content_mask(clip, |window| {
            for line in &shaped.lines {
                let top = origin.y + line.top;
                let bottom = top + line_height * line.geometry.rows() as f32;
                if bottom < visible.top() || top > visible.bottom() {
                    continue;
                }
                let _ = line.layout.paint(
                    point(origin.x, top),
                    line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
        });
        if !focused {
            return;
        }
        if let Some(marked) = marked {
            for rect in shaped.range_rects(marked, false) {
                window.paint_quad(fill(
                    Bounds::from_corners(
                        point(rect.left(), rect.bottom() - px(1.)),
                        rect.bottom_right(),
                    ) + origin,
                    theme::accent(),
                ));
            }
        }
        let caret = shaped.caret_position(selection.caret, row_end);
        let caret_height = window.rem_size().min(line_height);
        window.paint_quad(fill(
            Bounds::new(
                origin + caret + point(px(0.), (line_height - caret_height) / 2.),
                size(px(1.), caret_height),
            ),
            theme::accent(),
        ));
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum FieldValidation {
    #[default]
    Valid,
    Error(SharedString),
}

impl FieldValidation {
    pub fn error(message: impl Into<SharedString>) -> Self {
        Self::Error(message.into())
    }

    pub fn message(&self) -> Option<&SharedString> {
        match self {
            Self::Valid => None,
            Self::Error(message) => Some(message),
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }
}

pub struct TextField {
    focus_handle: FocusHandle,
    buffer: TextBuffer,
    placeholder: SharedString,
    placeholder_as_value: bool,
    monospace: bool,
    fill_height: bool,
    kind: TextFieldKind,
    disabled: bool,
    validation: FieldValidation,
    bare: bool,
    text_size: Rems,
    right_padding: f32,
    hover_border: bool,
    disabled_cursor_not_allowed: bool,
    scroll_handle: ScrollHandle,
    scrollbar: Option<Entity<Scrollbar>>,
    selecting: bool,
    /// Where a selection drag last saw the pointer.
    drag_position: Option<Point<Pixels>>,
    /// Scrolls a textarea toward a drag that left it, while the button is down.
    auto_scroll: Option<Task<()>>,
    text_state: Rc<RefCell<FieldTextState>>,
    /// The caret index that shows at the end of the row a soft wrap broke,
    /// rather than at the start of the next one.
    caret_row_end: Option<usize>,
    /// The x that consecutive Ups and Downs keep, from the index the last one
    /// left the caret at; anything else that moves the caret clears it.
    vertical_goal: Option<(usize, Pixels)>,
    auto_grow_rows: Option<u8>,
    key_interceptor: Option<KeyDownInterceptor>,
    truncate_unfocused: bool,
    /// Unfocused, show the path compacted to fit, keeping its last folder.
    compact_path: bool,
    /// The compacted path last chosen for this text: `(text, shown)`.
    compact_shown: Rc<RefCell<Option<(String, String)>>>,
}

impl TextField {
    pub fn new(
        focus_handle: FocusHandle,
        text: impl Into<String>,
        placeholder: impl Into<SharedString>,
        monospace: bool,
    ) -> Self {
        let mut buffer = TextBuffer::default();
        buffer.reset(text);
        Self {
            focus_handle,
            buffer,
            placeholder: placeholder.into(),
            placeholder_as_value: false,
            monospace,
            fill_height: false,
            kind: TextFieldKind::Input,
            disabled: false,
            validation: FieldValidation::Valid,
            bare: false,
            text_size: theme::text_title(),
            right_padding: 10.,
            hover_border: false,
            disabled_cursor_not_allowed: false,
            scroll_handle: ScrollHandle::new(),
            scrollbar: None,
            selecting: false,
            drag_position: None,
            auto_scroll: None,
            text_state: Rc::default(),
            caret_row_end: None,
            vertical_goal: None,
            auto_grow_rows: None,
            key_interceptor: None,
            truncate_unfocused: false,
            compact_path: false,
            compact_shown: Rc::new(RefCell::new(None)),
        }
    }

    pub fn textarea(
        focus_handle: FocusHandle,
        text: impl Into<String>,
        placeholder: impl Into<SharedString>,
        rows: u8,
        monospace: bool,
    ) -> Self {
        let mut field = Self::new(focus_handle, text, placeholder, monospace);
        field.kind = TextFieldKind::Textarea { rows: rows.max(1) };
        field.buffer.multiline = true;
        field
    }

    /// Let a textarea grow with its wrapped content, from its base `rows` up
    /// to `max_rows`; longer content scrolls as before.
    pub fn auto_grow(mut self, max_rows: u8) -> Self {
        self.auto_grow_rows = Some(max_rows);
        self
    }

    pub fn fill_height(mut self) -> Self {
        self.fill_height = true;
        self
    }

    pub fn with_scrollbar(mut self, cx: &mut Context<Self>) -> Self {
        let owner = cx.entity_id();
        let handle = self.scroll_handle.clone();
        self.scrollbar = Some(cx.new(|_| Scrollbar::app(handle, owner)));
        self
    }

    pub fn key_interceptor(mut self, interceptor: KeyDownInterceptor) -> Self {
        self.key_interceptor = Some(interceptor);
        self
    }

    pub fn truncate_unfocused(mut self) -> Self {
        self.truncate_unfocused = true;
        self
    }

    /// While unfocused, show the text as a path compacted to fit: home as
    /// `~`, then middle folders folded into `…`, always keeping the last
    /// folder. Focused, the field edits the full path.
    pub fn compact_path(mut self) -> Self {
        self.compact_path = true;
        self
    }

    pub fn text(&self) -> &str {
        &self.buffer.text
    }

    pub fn edited(&self) -> bool {
        self.buffer.edited
    }

    pub fn mark_clean(&mut self) {
        self.buffer.edited = false;
    }

    pub fn is_composing(&self) -> bool {
        self.buffer.marked.is_some()
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    pub fn placeholder_as_value(mut self, placeholder_as_value: bool) -> Self {
        self.placeholder_as_value = placeholder_as_value;
        self
    }

    pub fn set_placeholder_as_value(&mut self, placeholder_as_value: bool, cx: &mut Context<Self>) {
        if self.placeholder_as_value != placeholder_as_value {
            self.placeholder_as_value = placeholder_as_value;
            cx.notify();
        }
    }

    pub fn text_size(mut self, text_size: Rems) -> Self {
        self.text_size = text_size;
        self
    }

    pub fn right_padding(mut self, right_padding: f32) -> Self {
        self.right_padding = right_padding;
        self
    }

    pub fn reset(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.buffer.reset(text);
        self.vertical_goal = None;
        cx.notify();
    }

    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.buffer.set_text(&text.into());
        self.vertical_goal = None;
        self.buffer.edited = true;
        cx.notify();
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.buffer.select_all();
        self.vertical_goal = None;
        cx.notify();
    }

    pub fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        if self.disabled != disabled {
            self.disabled = disabled;
            cx.notify();
        }
    }

    pub fn set_validation(&mut self, validation: FieldValidation, cx: &mut Context<Self>) {
        if self.validation != validation {
            self.validation = validation;
            cx.notify();
        }
    }

    pub fn set_bare(&mut self, bare: bool, cx: &mut Context<Self>) {
        if self.bare != bare {
            self.bare = bare;
            cx.notify();
        }
    }

    pub fn placeholder_uses_value_style(&self) -> bool {
        self.placeholder_as_value
    }

    #[cfg(test)]
    pub(crate) fn text_right_padding(&self) -> f32 {
        self.right_padding
    }

    pub fn set_right_padding(&mut self, right_padding: f32, cx: &mut Context<Self>) {
        if self.right_padding != right_padding {
            self.right_padding = right_padding;
            cx.notify();
        }
    }

    pub fn set_hover_border(&mut self, hover_border: bool, cx: &mut Context<Self>) {
        if self.hover_border != hover_border {
            self.hover_border = hover_border;
            cx.notify();
        }
    }

    pub fn set_disabled_cursor_not_allowed(
        &mut self,
        disabled_cursor_not_allowed: bool,
        cx: &mut Context<Self>,
    ) {
        if self.disabled_cursor_not_allowed != disabled_cursor_not_allowed {
            self.disabled_cursor_not_allowed = disabled_cursor_not_allowed;
            cx.notify();
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        let vertical_goal = self.vertical_goal.take();
        if !self.is_composing()
            && self
                .key_interceptor
                .clone()
                .is_some_and(|interceptor| interceptor(event, window, cx))
        {
            cx.stop_propagation();
            return;
        }
        if event.keystroke.key == "enter" {
            match enter_behavior(self.kind, self.is_composing()) {
                EnterBehavior::Submit => return,
                EnterBehavior::Block => cx.stop_propagation(),
                EnterBehavior::InsertNewline => {
                    self.buffer.replace_selection("\n");
                    self.reveal_caret();
                    cx.stop_propagation();
                    cx.notify();
                }
            }
            return;
        }
        let modifiers = event.keystroke.modifiers;
        if self.kind.multiline()
            && !self.is_composing()
            && matches!(event.keystroke.key.as_str(), "up" | "down")
            && !(modifiers.platform || modifiers.control || modifiers.alt || modifiers.function)
        {
            self.move_vertically(
                event.keystroke.key == "up",
                modifiers.shift,
                vertical_goal,
                window,
            );
            cx.stop_propagation();
            cx.notify();
            return;
        }
        // An undo or redo moves a step even where it leaves the length and
        // selection as they were.
        let state = |buffer: &TextBuffer| {
            (
                buffer.selection,
                buffer.text.len(),
                buffer.history.undo.len(),
            )
        };
        let before = state(&self.buffer);
        let handled = handle_key_down(&mut self.buffer, event, cx);
        if handled {
            if state(&self.buffer) != before {
                self.reveal_caret();
            }
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn on_copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.buffer.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
        }
    }

    fn on_cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        if let Some(text) = self.buffer.selected_text().map(str::to_owned) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.buffer.delete_left(Boundary::Grapheme);
            self.vertical_goal = None;
            self.reveal_caret();
            cx.notify();
        }
    }

    fn on_paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.buffer.replace_selection(&text);
            self.vertical_goal = None;
            self.reveal_caret();
            cx.notify();
        }
    }

    fn on_select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.buffer.select_all();
        self.vertical_goal = None;
        cx.notify();
    }

    fn on_undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if !self.disabled && self.buffer.undo() {
            self.vertical_goal = None;
            self.reveal_caret();
            cx.notify();
        }
    }

    fn on_redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if !self.disabled && self.buffer.redo() {
            self.vertical_goal = None;
            self.reveal_caret();
            cx.notify();
        }
    }

    fn reveal_caret(&self) {
        self.text_state.borrow_mut().reveal_caret = true;
    }

    /// The shaped text for the current value, reshaped at the last painted
    /// width and style when an edit landed since, and where it was painted
    /// at the current scroll offset.
    fn with_text<R>(
        &self,
        window: &Window,
        f: impl FnOnce(&FieldText, Option<Point<Pixels>>) -> R,
    ) -> Option<R> {
        let mut state = self.text_state.borrow_mut();
        let shaped = state.shaped.as_ref()?;
        let current = if self.buffer.text.is_empty() {
            shaped.key.placeholder && shaped.key.text == self.placeholder
        } else {
            !shaped.key.placeholder && shaped.key.text.as_ref() == self.buffer.text
        };
        if !current {
            let (text, placeholder) = self.display_text();
            let key = FieldTextKey {
                text,
                placeholder,
                ..shaped.key.clone()
            };
            let wrap_width = shaped.wrap_width;
            state.shaped = Some(FieldText::new(key, wrap_width, window));
        }
        let origin = state
            .origin
            .map(|origin| origin + (self.scroll_offset() - state.scroll_offset));
        state.shaped.as_ref().map(|shaped| f(shaped, origin))
    }

    fn scroll_offset(&self) -> Point<Pixels> {
        if self.kind.multiline() {
            self.scroll_handle.offset()
        } else {
            Point::default()
        }
    }

    fn display_text(&self) -> (SharedString, bool) {
        if self.buffer.text.is_empty() {
            (self.placeholder.clone(), true)
        } else {
            (SharedString::from(self.buffer.text.clone()), false)
        }
    }

    fn text_key(&self, window: &Window) -> FieldTextKey {
        let style = window.text_style();
        let rem_size = window.rem_size();
        let (text, placeholder) = self.display_text();
        FieldTextKey {
            text,
            placeholder,
            multiline: self.kind.multiline(),
            font: style.font(),
            color: if placeholder && !self.placeholder_as_value {
                theme::faint()
            } else {
                style.color
            },
            font_size: style.font_size.to_pixels(rem_size),
            line_height: style.line_height_in_pixels(rem_size),
        }
    }

    /// The index under a window point, and whether it is the end of a
    /// soft-wrapped row; `None` until the text has been painted.
    fn index_for_point(
        &self,
        position: Point<Pixels>,
        under: bool,
        window: &Window,
    ) -> Option<(usize, bool)> {
        self.with_text(window, |text, origin| {
            origin.map(|origin| text.index_for_position(position - origin, under))
        })
        .flatten()
    }

    fn move_vertically(
        &mut self,
        up: bool,
        extend: bool,
        vertical_goal: Option<(usize, Pixels)>,
        window: &Window,
    ) {
        let selection = self.buffer.selection;
        let from = if extend || selection.is_empty() {
            selection.caret
        } else if up {
            selection.range().start
        } else {
            selection.range().end
        };
        let row_end = self.caret_row_end == Some(from);
        let goal_x = vertical_goal
            .filter(|(index, _)| *index == from)
            .map(|(_, x)| x);
        let (target, row_end, x) = self
            .with_text(window, |text, _| {
                text.vertical_target(from, row_end, up, goal_x)
            })
            .unwrap_or_else(|| {
                let end = if up { 0 } else { self.buffer.text.len() };
                (end, false, px(0.))
            });
        self.buffer.move_to(target, extend);
        self.caret_row_end = row_end.then_some(target);
        self.vertical_goal = Some((target, x));
        self.reveal_caret();
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        self.focus_handle.focus(window, cx);
        let (position, row_end) = self
            .index_for_point(event.position, event.click_count >= 2, window)
            .unwrap_or((self.buffer.text.len(), false));
        self.selecting = true;
        self.drag_position = None;
        self.caret_row_end = None;
        self.vertical_goal = None;
        match event.click_count {
            2 => self.buffer.select_word_at(position),
            count if count >= 3 => self.buffer.select_line_at(position),
            _ => {
                self.buffer.move_to(position, event.modifiers.shift);
                self.caret_row_end = row_end.then_some(position);
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// A window-wide mouse move while this field's selection drag is in
    /// progress, so the selection follows the pointer outside the field too.
    fn on_drag_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.pressed_button != Some(MouseButton::Left) {
            self.stop_selecting();
            return;
        }
        self.drag_position = Some(event.position);
        self.select_to_drag_position(window, cx);
        if self.auto_scroll.is_none() && self.auto_scroll_step().is_some() {
            self.auto_scroll = Some(cx.spawn_in(window, async move |field, cx| {
                loop {
                    cx.background_executor().timer(AUTO_SCROLL_TICK).await;
                    let scrolled = field
                        .update_in(cx, |field, window, cx| field.auto_scroll_tick(window, cx))
                        .unwrap_or(false);
                    if !scrolled {
                        break;
                    }
                }
                field.update(cx, |field, _| field.auto_scroll = None).ok();
            }));
        }
    }

    fn select_to_drag_position(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some((index, row_end)) = self
            .drag_position
            .and_then(|position| self.index_for_point(position, false, window))
        else {
            return;
        };
        self.buffer.move_to(index, true);
        self.caret_row_end = row_end.then_some(index);
        self.vertical_goal = None;
        cx.notify();
    }

    /// How far a textarea scrolls per tick toward a drag above or below it.
    fn auto_scroll_step(&self) -> Option<Pixels> {
        if !self.kind.multiline() {
            return None;
        }
        let position = self.drag_position?;
        let line_height = self.text_state.borrow().shaped.as_ref()?.line_height();
        let viewport = self.scroll_handle.bounds();
        let distance = if position.y < viewport.top() {
            viewport.top() - position.y
        } else if position.y > viewport.bottom() {
            viewport.bottom() - position.y
        } else {
            return None;
        };
        let speed = (distance.abs() / 2.).clamp(line_height / 2., line_height * 3.);
        Some(if distance > px(0.) { speed } else { -speed })
    }

    fn auto_scroll_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(step) = self.selecting.then(|| self.auto_scroll_step()).flatten() else {
            return false;
        };
        let offset = self.scroll_handle.offset();
        let max = self.scroll_handle.max_offset().y;
        let y = (offset.y + step).clamp(-max, px(0.));
        if y == offset.y {
            return false;
        }
        self.scroll_handle.set_offset(point(offset.x, y));
        self.select_to_drag_position(window, cx);
        cx.notify();
        true
    }

    fn stop_selecting(&mut self) {
        self.selecting = false;
        self.drag_position = None;
        self.auto_scroll = None;
    }

    fn on_mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.stop_selecting();
    }

    /// The compacted path to show, or `None` while the field edits it.
    fn compact_path_shown(&self, focused: bool) -> Option<String> {
        if !self.compact_path || focused || self.kind != TextFieldKind::Input {
            return None;
        }
        let text = &self.buffer.text;
        if text.is_empty() {
            return None;
        }
        let chosen = self
            .compact_shown
            .borrow()
            .as_ref()
            .filter(|(source, _)| source == text)
            .map(|(_, shown)| shown.clone());
        chosen.or_else(|| {
            compact_path_candidates(text, home_dir().as_deref())
                .into_iter()
                .next()
        })
    }

    fn render_text(&self, focused: bool, field: Entity<Self>) -> AnyElement {
        let read_only = if let Some(shown) = self.compact_path_shown(focused) {
            let rendered = shown.clone();
            Some(
                div()
                    .debug_selector(move || format!("TEXT_FIELD_COMPACT {rendered}"))
                    .min_w(px(0.))
                    .w_full()
                    .truncate()
                    .child(shown),
            )
        } else if self.kind == TextFieldKind::Input
            && self.truncate_unfocused
            && !focused
            && !self.buffer.text.is_empty()
        {
            Some(
                div()
                    .min_w(px(0.))
                    .w_full()
                    .truncate()
                    .child(self.buffer.text.clone()),
            )
        } else {
            None
        };
        match read_only {
            Some(display) => {
                self.text_state.borrow_mut().origin = None;
                display.into_any_element()
            }
            None => TextFieldElement { field }.into_any_element(),
        }
    }
}

impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        Some(self.buffer.text_for_range(range, adjusted_range))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(self.buffer.selected_text_range())
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.buffer.marked_text_range()
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.buffer.unmark_text();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        self.buffer.replace_text_in_range(range, text);
        self.vertical_goal = None;
        self.reveal_caret();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        self.buffer
            .replace_and_mark_text_in_range(range, new_text, new_selected_range);
        self.vertical_goal = None;
        self.reveal_caret();
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = text_util::range_from_utf16(&self.buffer.text, &range_utf16);
        let upstream = self.caret_row_end == Some(range.start);
        self.with_text(window, |text, origin| {
            origin.map(|origin| text.range_bounds(range, upstream) + origin)
        })
        .flatten()
        .or(Some(element_bounds))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.index_for_point(point, false, window).map_or_else(
            || self.buffer.character_index_utf16(),
            |(index, _)| text_util::offset_to_utf16(&self.buffer.text, index),
        ))
    }
}

impl Focusable for TextField {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);
        let compact_shown = self
            .compact_path_shown(focused)
            .map(|shown| (self.buffer.text.clone(), shown));
        let field = cx.entity();
        if self.kind != TextFieldKind::Input && self.scrollbar.is_none() {
            let owner = cx.entity_id();
            let scroll_handle = self.scroll_handle.clone();
            self.scrollbar = Some(cx.new(|_| Scrollbar::app(scroll_handle, owner)));
        }
        let multiline = self.kind != TextFieldKind::Input;
        let scrollbar = self.scrollbar.clone();
        // Auto-grow textareas take their height from the wrapped content between
        // the base row count and `auto_grow_rows`; everything else is fixed-height.
        let auto_grow = self
            .kind
            .multiline()
            .then_some(self.auto_grow_rows)
            .flatten()
            .map(|max_rows| {
                let min_rows = self.kind.rows();
                (min_rows as f32 * 20., max_rows.max(min_rows) as f32 * 20.)
            });
        let height = if self.bare {
            20.
        } else {
            match self.kind {
                TextFieldKind::Input => 34.,
                TextFieldKind::Textarea { .. } => self.kind.rows() as f32 * 20. + 14.,
            }
        };
        div()
            .relative()
            .flex()
            .when(self.kind == TextFieldKind::Input, |input| {
                input.items_center()
            })
            .when(self.kind != TextFieldKind::Input, |input| {
                input.items_start()
            })
            .min_w(px(0.))
            .w_full()
            .when(self.fill_height, |input| input.h_full())
            .when(auto_grow.is_none() && !self.fill_height, |input| {
                input.h(rems(height / 16.))
            })
            .when(!self.bare, |input| {
                input.pl(rems(10. / 16.)).pr(rems(self.right_padding / 16.))
            })
            .when(!self.bare && self.kind != TextFieldKind::Input, |input| {
                input.py(rems(6. / 16.))
            })
            .overflow_hidden()
            .when(!self.bare, |input| {
                input
                    .rounded(rems(4. / 16.))
                    .border_1()
                    .border_color(input_border_color(&self.validation, focused))
                    .bg(theme::bg())
            })
            .track_focus(&self.focus_handle)
            .key_context("TextInput")
            .tab_index(0)
            .tab_stop(!self.disabled)
            .cursor(if self.disabled {
                if self.disabled_cursor_not_allowed {
                    CursorStyle::OperationNotAllowed
                } else {
                    CursorStyle::Arrow
                }
            } else {
                CursorStyle::IBeam
            })
            .when(!self.bare && !self.disabled && self.hover_border, |input| {
                input.hover(|input| input.border_color(theme::faint()))
            })
            .opacity(if self.disabled { 0.6 } else { 1. })
            .on_key_down(cx.listener(Self::on_key_down))
            .on_action(cx.listener(Self::on_cut))
            .on_action(cx.listener(Self::on_copy))
            .on_action(cx.listener(Self::on_paste))
            .on_action(cx.listener(Self::on_select_all))
            .on_action(cx.listener(Self::on_undo))
            .on_action(cx.listener(Self::on_redo))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .text_size(self.text_size)
            .when(multiline, |input| input.line_height(rems(20. / 16.)))
            .text_color(theme::text())
            .when(self.monospace, |input| {
                input.font_family(theme::UI_MONOSPACE_FONT)
            })
            .when(self.kind == TextFieldKind::Input, |input| {
                input.child(self.render_text(focused, field.clone()))
            })
            .when(multiline, |input| {
                input.child(
                    div()
                        .id("text-field-scroll")
                        .relative()
                        .w_full()
                        .map(|scroll| match auto_grow {
                            Some((min_px, max_px)) => {
                                scroll.min_h(rems(min_px / 16.)).max_h(rems(max_px / 16.))
                            }
                            None => scroll.h_full(),
                        })
                        .overflow_y_scroll()
                        .scrollbar_width(px(0.))
                        .track_scroll(&self.scroll_handle)
                        .child(self.render_text(focused, field)),
                )
            })
            .when(compact_shown.is_some(), |input| {
                let (text, rendered) = compact_shown.unwrap_or_default();
                let cell = Rc::clone(&self.compact_shown);
                let bare = self.bare;
                let right_padding = self.right_padding;
                input.child(
                    canvas(
                        move |bounds, window, cx| {
                            let rem_size = window.rem_size();
                            let padding = if bare {
                                px(0.)
                            } else {
                                rem_size * ((10. + right_padding) / 16.)
                            };
                            let width = bounds.size.width - padding;
                            let style = window.text_style();
                            let font_size = style.font_size.to_pixels(rem_size);
                            let shown = pick_compact_path(
                                compact_path_candidates(&text, home_dir().as_deref()),
                                |candidate| {
                                    window
                                        .text_system()
                                        .shape_line(
                                            candidate.to_owned().into(),
                                            font_size,
                                            &[style.to_run(candidate.len())],
                                            None,
                                        )
                                        .width
                                        <= width
                                },
                            );
                            let changed = shown != rendered;
                            cell.replace(Some((text.clone(), shown)));
                            // The pick depends on this frame's width, so a new
                            // pick renders once more with it.
                            if changed {
                                rerender_after_draw(window, cx);
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .inset_0(),
                )
            })
            .when(multiline, |input| input.children(scrollbar))
    }
}

fn input_border_color(validation: &FieldValidation, focused: bool) -> gpui::Hsla {
    if validation.is_error() {
        theme::danger()
    } else if focused {
        theme::faint()
    } else {
        theme::border_strong()
    }
}

#[derive(IntoElement)]
pub struct Label {
    id: SharedString,
    label: SharedString,
    hint: Option<(SharedString, FocusHandle)>,
    focus_target: Option<FocusHandle>,
    emphasized: bool,
}

impl Label {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            hint: None,
            focus_target: None,
            emphasized: false,
        }
    }

    pub fn hint(mut self, hint: impl Into<SharedString>, focus: FocusHandle) -> Self {
        self.hint = Some((hint.into(), focus));
        self
    }

    pub fn focus_target(mut self, focus_target: FocusHandle) -> Self {
        self.focus_target = Some(focus_target);
        self
    }

    pub fn emphasized(mut self, emphasized: bool) -> Self {
        self.emphasized = emphasized;
        self
    }
}

impl RenderOnce for Label {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let hint_id = SharedString::from(format!("field-hint-tooltip-{}", self.id));
        let focus_target = self.focus_target;
        div()
            .id(SharedString::from(format!("field-label-{}", self.id)))
            .flex()
            .items_center()
            .gap(rems(6. / 16.))
            .font_weight(if self.emphasized {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::MEDIUM
            })
            .text_size(theme::text_ui())
            .text_color(if self.emphasized {
                theme::text()
            } else {
                theme::muted()
            })
            .when_some(focus_target, |label, focus| {
                label.cursor(CursorStyle::PointingHand).on_mouse_down(
                    MouseButton::Left,
                    move |_, window, cx| {
                        focus.focus(window, cx);
                    },
                )
            })
            .child(self.label)
            .children(self.hint.map(|(hint, focus)| {
                let hint_focus = focus.clone();
                Tooltip::new(
                    hint_id,
                    hint,
                    div()
                        .track_focus(&focus)
                        .tab_index(0)
                        .tab_stop(true)
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(rems(14. / 16.))
                        .rounded(rems(2. / 16.))
                        .text_color(theme::faint())
                        .focus_visible(|hint| {
                            hint.shadow(vec![BoxShadow {
                                color: theme::faint(),
                                offset: gpui::point(px(0.), px(0.)),
                                blur_radius: px(0.),
                                spread_radius: px(1.),
                                inset: false,
                            }])
                        })
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            hint_focus.focus(window, cx);
                            cx.stop_propagation();
                        })
                        .child(
                            svg()
                                .flex_none()
                                .path("info.svg")
                                .size(rems(14. / 16.))
                                .text_color(theme::faint()),
                        ),
                )
                .focus_handle(focus)
            }))
    }
}

#[derive(IntoElement)]
pub struct FieldError {
    message: SharedString,
}

impl FieldError {
    pub fn new(message: impl Into<SharedString>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl RenderOnce for FieldError {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .text_size(theme::text_ui())
            .text_color(theme::danger())
            .child(self.message)
    }
}

#[derive(IntoElement)]
pub struct Field {
    id: SharedString,
    label: SharedString,
    hint: Option<(SharedString, FocusHandle)>,
    tag: Option<SharedString>,
    subtitle: Option<SharedString>,
    error: Option<SharedString>,
    focus_target: Option<FocusHandle>,
    child: AnyElement,
    emphasized: bool,
}

impl Field {
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        child: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            hint: None,
            tag: None,
            subtitle: None,
            error: None,
            focus_target: None,
            child: child.into_any_element(),
            emphasized: false,
        }
    }

    pub fn hint(mut self, hint: impl Into<SharedString>, focus: FocusHandle) -> Self {
        self.hint = Some((hint.into(), focus));
        self
    }

    /// A faint word after the label, such as `optional`.
    pub fn tag(mut self, tag: impl Into<SharedString>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    pub fn focus_target(mut self, focus_target: FocusHandle) -> Self {
        self.focus_target = Some(focus_target);
        self
    }

    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    pub fn error(mut self, error: impl Into<SharedString>) -> Self {
        self.error = Some(error.into());
        self
    }

    pub fn emphasized(mut self, emphasized: bool) -> Self {
        self.emphasized = emphasized;
        self
    }
}

impl RenderOnce for Field {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let mut label = Label::new(self.id, self.label).emphasized(self.emphasized);
        if let Some(focus_target) = self.focus_target {
            label = label.focus_target(focus_target);
        }
        if let Some((hint, focus)) = self.hint {
            label = label.hint(hint, focus);
        }
        div()
            .flex()
            .flex_col()
            .gap(rems(if self.emphasized { 6. / 16. } else { 4. / 16. }))
            .child(match self.tag {
                Some(tag) => div()
                    .flex()
                    .items_center()
                    .gap(rems(6. / 16.))
                    .child(label)
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .text_color(theme::faint())
                            .child(tag),
                    )
                    .into_any_element(),
                None => label.into_any_element(),
            })
            .child(self.child)
            .children(self.subtitle.map(|subtitle| {
                div()
                    .text_size(theme::text_meta())
                    .text_color(theme::faint())
                    .child(subtitle)
            }))
            .children(self.error.map(FieldError::new))
    }
}

#[derive(IntoElement)]
pub struct BrowseField {
    input: Entity<TextField>,
    disabled: bool,
    browse_id: ElementId,
    browse_label: SharedString,
    browse_focus: Option<FocusHandle>,
    on_browse: PressHandler,
}

impl BrowseField {
    pub fn new(input: Entity<TextField>, disabled: bool, on_browse: PressHandler) -> Self {
        Self {
            input,
            disabled,
            browse_id: "working-dir-browse".into(),
            browse_label: "Browse".into(),
            browse_focus: None,
            on_browse,
        }
    }

    pub fn browse_id(mut self, browse_id: impl Into<ElementId>) -> Self {
        self.browse_id = browse_id.into();
        self
    }

    pub fn browse_label(mut self, browse_label: impl Into<SharedString>) -> Self {
        self.browse_label = browse_label.into();
        self
    }

    pub fn browse_focus(mut self, browse_focus: FocusHandle) -> Self {
        self.browse_focus = Some(browse_focus);
        self
    }
}

impl RenderOnce for BrowseField {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let browse = Rc::clone(&self.on_browse);
        div()
            .flex()
            .items_center()
            .gap(rems(8. / 16.))
            .child(div().flex_1().min_w(px(0.)).child(self.input))
            .child(
                Button::new(self.browse_id, self.browse_label)
                    .when_some(self.browse_focus, |button, focus| {
                        button.focus_handle(focus)
                    })
                    .disabled(self.disabled)
                    .on_press(move |window, cx| browse(window, cx)),
            )
    }
}

pub type WorkingDirField = BrowseField;

pub fn working_dir_placeholder(owner_path: Option<&str>, default_path: &str) -> String {
    owner_path
        .filter(|path| !path.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| effective_working_dir("", false, default_path))
        .unwrap_or_else(|| "Home directory".to_owned())
}

pub fn working_dir_text_field(
    focus_handle: FocusHandle,
    text: impl Into<String>,
    placeholder: impl Into<SharedString>,
) -> TextField {
    TextField::new(focus_handle, text, placeholder, true).compact_path()
}

fn home_dir() -> Option<String> {
    std::env::home_dir().map(|home| home.to_string_lossy().into_owned())
}

/// A path's displays, longest first: the path with home shown as `~`, then
/// with more and more middle folders folded into `…`, down to the root or
/// `~` and the last folder alone.
pub(crate) fn compact_path_candidates(path: &str, home: Option<&str>) -> Vec<String> {
    let separator = if path.contains('\\') && !path.contains('/') {
        '\\'
    } else {
        '/'
    };
    let trimmed = path.trim_end_matches(separator);
    // A bare drive (`C:`) is drive-relative on Windows; the root keeps its
    // separator.
    let path = if trimmed.is_empty() || trimmed.ends_with(':') {
        path
    } else {
        trimmed
    };
    let home = home
        .map(|home| home.trim_end_matches(['/', '\\']))
        .filter(|home| !home.is_empty());
    let abbreviated = match home {
        Some(home) if path == home => "~".to_owned(),
        Some(home)
            if path
                .strip_prefix(home)
                .is_some_and(|rest| rest.starts_with(separator)) =>
        {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_owned(),
    };
    let mut parts = abbreviated.split(separator);
    let head = parts.next().unwrap_or_default().to_owned();
    let folders = parts.filter(|part| !part.is_empty()).collect::<Vec<_>>();
    let mut candidates = vec![abbreviated.clone()];
    for keep in (1..folders.len()).rev() {
        let tail = folders[folders.len() - keep..].join(&separator.to_string());
        candidates.push(format!("{head}{separator}…{separator}{tail}"));
    }
    // A relative path's head is a folder too, and can be long: fold it as
    // well so the last folder still shows. A root, `~` or a drive stays.
    if let Some(last) = folders.last() {
        if !head.is_empty() && head != "~" && !head.ends_with(':') {
            candidates.push(format!("…{separator}{last}"));
        }
    }
    candidates
}

/// The longest display that fits, else the shortest.
pub(crate) fn pick_compact_path(
    candidates: Vec<String>,
    mut fits: impl FnMut(&str) -> bool,
) -> String {
    let last = candidates.last().cloned().unwrap_or_default();
    candidates
        .into_iter()
        .find(|candidate| fits(candidate))
        .unwrap_or(last)
}

pub fn effective_working_dir(
    explicit_path: &str,
    owner_has_working_dir: bool,
    default_path: &str,
) -> Option<String> {
    let explicit = explicit_path.trim();
    if !explicit.is_empty() {
        return Some(explicit.to_owned());
    }
    if owner_has_working_dir {
        return None;
    }
    let default_path = default_path.trim();
    (!default_path.is_empty())
        .then(|| default_path.to_owned())
        .or_else(|| {
            runner_backend::app_paths::home_dir()
                .and_then(|home| home.into_os_string().into_string().ok())
        })
}

fn handle_key_down<T>(input: &mut TextBuffer, event: &KeyDownEvent, cx: &mut Context<T>) -> bool {
    handle_key_down_for_platform(input, event, cx, cfg!(windows))
}

fn handle_key_down_for_platform<T>(
    input: &mut TextBuffer,
    event: &KeyDownEvent,
    cx: &mut Context<T>,
    windows: bool,
) -> bool {
    let key = event.keystroke.key.as_str();
    let modifiers = event.keystroke.modifiers;
    let command = if windows {
        modifiers.control && matches!(key, "a" | "c" | "x" | "v" | "y" | "z")
    } else {
        modifiers.platform
    };
    if command {
        return match key {
            "a" => {
                input.select_all();
                true
            }
            "c" => {
                if let Some(text) = input.selected_text() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
                }
                true
            }
            "x" => {
                if let Some(text) = input.selected_text() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
                    input.delete_left(Boundary::Grapheme);
                }
                true
            }
            "v" => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    input.replace_selection(&text);
                }
                true
            }
            "z" => {
                if modifiers.shift {
                    input.redo();
                } else {
                    input.undo();
                }
                true
            }
            "y" if windows => {
                input.redo();
                true
            }
            "left" => {
                input.move_left(Boundary::Line, modifiers.shift);
                true
            }
            "right" => {
                input.move_right(Boundary::Line, modifiers.shift);
                true
            }
            "backspace" => {
                input.delete_left(Boundary::Line);
                true
            }
            "delete" => {
                input.delete_right(Boundary::Line);
                true
            }
            _ => false,
        };
    }
    let word = if windows {
        modifiers.control
    } else {
        modifiers.alt
    };
    if word {
        return match key {
            "left" => {
                input.move_left(Boundary::Word, modifiers.shift);
                true
            }
            "right" => {
                input.move_right(Boundary::Word, modifiers.shift);
                true
            }
            "backspace" => {
                input.delete_left(Boundary::Word);
                true
            }
            "delete" => {
                input.delete_right(Boundary::Word);
                true
            }
            _ => false,
        };
    }
    if modifiers.control {
        return false;
    }
    if modifiers.function {
        return match key {
            "left" => {
                input.move_left(Boundary::Line, modifiers.shift);
                true
            }
            "right" => {
                input.move_right(Boundary::Line, modifiers.shift);
                true
            }
            "backspace" | "delete" => {
                input.delete_right(Boundary::Grapheme);
                true
            }
            _ => false,
        };
    }
    match key {
        "left" => {
            input.move_left(Boundary::Grapheme, modifiers.shift);
            true
        }
        "right" => {
            input.move_right(Boundary::Grapheme, modifiers.shift);
            true
        }
        "home" => {
            input.move_left(Boundary::Line, modifiers.shift);
            true
        }
        "end" => {
            input.move_right(Boundary::Line, modifiers.shift);
            true
        }
        "backspace" => {
            input.delete_left(Boundary::Grapheme);
            true
        }
        "delete" => {
            input.delete_right(Boundary::Grapheme);
            true
        }
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EnterBehavior {
    Submit,
    InsertNewline,
    Block,
}

fn enter_behavior(kind: TextFieldKind, composing: bool) -> EnterBehavior {
    if composing {
        EnterBehavior::Block
    } else if kind.multiline() {
        EnterBehavior::InsertNewline
    } else {
        EnterBehavior::Submit
    }
}

#[cfg(test)]
fn enter_should_submit(composing: bool) -> bool {
    enter_behavior(TextFieldKind::Input, composing) == EnterBehavior::Submit
}

fn normalize_input_text(text: &str, multiline: bool) -> String {
    if multiline {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text.replace("\r\n", " ").replace(['\r', '\n'], " ")
    }
}

fn previous_word_boundary(text: &str, position: usize) -> usize {
    text.split_word_bound_indices()
        .take_while(|(start, _)| *start < position)
        .filter(|(_, segment)| is_word(segment))
        .map(|(start, _)| start)
        .fold(0, |_, start| start)
}

fn next_word_boundary(text: &str, position: usize) -> usize {
    text.split_word_bound_indices()
        .find(|(start, segment)| *start + segment.len() > position && is_word(segment))
        .map(|(start, segment)| start + segment.len())
        .unwrap_or(text.len())
}

fn is_word(segment: &str) -> bool {
    segment
        .chars()
        .any(|character| character.is_alphanumeric() || character == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test platform's text system advances every character by 0.6 em;
    /// fields default to 14 px text, and textarea rows are 20 px.
    const ADVANCE: f32 = 14. * 0.6;
    const ROW: f32 = 20.;
    /// Wraps a 110 px wide textarea's text after ten characters.
    const WRAPPED: &str = "alpha beta gamma\n\nend";

    struct FieldHost {
        input: Entity<TextField>,
        width: Pixels,
    }

    impl Render for FieldHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            // Room around the field, so a drag can leave it inside the window.
            div().p(px(100.)).child(
                div()
                    .w(self.width)
                    .debug_selector(|| "FIELD_HOST".into())
                    .child(self.input.clone()),
            )
        }
    }

    fn open_field(
        width: f32,
        build: impl FnOnce(FocusHandle) -> TextField + 'static,
    ) -> (gpui::VisualTestContext, Entity<TextField>) {
        let mut cx = gpui::TestAppContext::single();
        let window = cx.add_window(|_, cx| FieldHost {
            input: cx.new(|cx| build(cx.focus_handle())),
            width: px(width),
        });
        cx.run_until_parked();
        let visual = gpui::VisualTestContext::from_window(window.into(), &cx);
        visual.run_until_parked();
        let input = window.read_with(&cx, |host, _| host.input.clone()).unwrap();
        (visual, input)
    }

    fn text_origin(visual: &gpui::VisualTestContext, input: &Entity<TextField>) -> Point<Pixels> {
        input.read_with(visual, |field, _| {
            field
                .text_state
                .borrow()
                .origin
                .expect("the text was painted")
        })
    }

    /// A point `chars` advances along the text's row `row`, a little below
    /// the row's top.
    fn at(origin: Point<Pixels>, chars: f32, row: f32) -> Point<Pixels> {
        origin + point(px(chars * ADVANCE), px(row * ROW + 5.))
    }

    #[track_caller]
    fn assert_near(actual: Point<Pixels>, expected: Point<Pixels>) {
        assert!(
            (actual.x - expected.x).abs() < px(0.01) && (actual.y - expected.y).abs() < px(0.01),
            "{actual:?} is not {expected:?}"
        );
    }

    #[track_caller]
    fn assert_bounds_near(actual: Option<Bounds<Pixels>>, expected: Bounds<Pixels>) {
        let actual = actual.expect("bounds");
        assert_near(actual.origin, expected.origin);
        assert_near(actual.bottom_right(), expected.bottom_right());
    }

    fn click(visual: &mut gpui::VisualTestContext, position: Point<Pixels>, count: usize) {
        click_with(visual, position, count, gpui::Modifiers::none());
    }

    fn click_with(
        visual: &mut gpui::VisualTestContext,
        position: Point<Pixels>,
        click_count: usize,
        modifiers: gpui::Modifiers,
    ) {
        visual.simulate_event(MouseDownEvent {
            position,
            modifiers,
            button: MouseButton::Left,
            click_count,
            first_mouse: false,
        });
        visual.simulate_event(MouseUpEvent {
            position,
            modifiers,
            button: MouseButton::Left,
            click_count,
        });
    }

    fn selection(visual: &gpui::VisualTestContext, input: &Entity<TextField>) -> (usize, usize) {
        input.read_with(visual, |field, _| {
            (field.buffer.selection.anchor, field.buffer.selection.caret)
        })
    }

    fn caret(visual: &gpui::VisualTestContext, input: &Entity<TextField>) -> usize {
        selection(visual, input).1
    }

    /// The caret's top left, relative to the text's origin, as painted.
    fn caret_position(
        visual: &gpui::VisualTestContext,
        input: &Entity<TextField>,
    ) -> Point<Pixels> {
        input.read_with(visual, |field, _| {
            let caret = field.buffer.selection.caret;
            let state = field.text_state.borrow();
            state
                .shaped
                .as_ref()
                .unwrap()
                .caret_position(caret, field.caret_row_end == Some(caret))
        })
    }

    /// Runs the auto-scroll timer `ticks` times; the test clock fires one
    /// timer per advance.
    fn tick(visual: &mut gpui::VisualTestContext, ticks: usize) {
        for _ in 0..ticks {
            visual.executor().advance_clock(AUTO_SCROLL_TICK);
            visual.run_until_parked();
        }
    }

    fn scroll_y(visual: &gpui::VisualTestContext, input: &Entity<TextField>) -> Pixels {
        input.read_with(visual, |field, _| field.scroll_handle.offset().y)
    }

    fn wrapped_field() -> (gpui::VisualTestContext, Entity<TextField>) {
        let (visual, input) = open_field(110., |focus| {
            TextField::textarea(focus, WRAPPED, "", 6, false)
        });
        input.read_with(&visual, |field, _| {
            let state = field.text_state.borrow();
            let lines = &state.shaped.as_ref().unwrap().lines;
            let rows = lines
                .iter()
                .map(|line| line.geometry.rows())
                .collect::<Vec<_>>();
            assert_eq!(rows, [2, 1, 1]);
            assert_eq!(
                lines[0].geometry.spot(6, false).row,
                1,
                "alpha / beta gamma"
            );
        });
        (visual, input)
    }

    #[test]
    fn a_click_lands_where_the_text_is_painted_after_wraps_at_line_ends_and_below() {
        let (mut visual, input) = wrapped_field();
        let origin = text_origin(&visual, &input);

        click(&mut visual, at(origin, 2.3, 1.), 1);
        assert_eq!(caret(&visual, &input), 8, "after the soft wrap");
        assert_near(
            caret_position(&visual, &input),
            point(px(2. * ADVANCE), px(ROW)),
        );

        click(&mut visual, at(origin, 9.6, 1.), 1);
        assert_eq!(caret(&visual, &input), 16, "the end of the wrapped line");

        click(&mut visual, at(origin, 9., 0.), 1);
        assert_eq!(caret(&visual, &input), 6, "past the end of the first row");
        // The caret stays at the end of the row that was clicked.
        assert_near(
            caret_position(&visual, &input),
            point(px(6. * ADVANCE), px(0.)),
        );

        click(&mut visual, at(origin, 0.4, 1.), 1);
        assert_eq!(caret(&visual, &input), 6, "the start of the second row");
        assert_near(caret_position(&visual, &input), point(px(0.), px(ROW)));

        click(&mut visual, at(origin, 3., 2.), 1);
        assert_eq!(caret(&visual, &input), 17, "the empty line");
        assert_near(caret_position(&visual, &input), point(px(0.), px(2. * ROW)));

        click(&mut visual, at(origin, 1.6, 3.), 1);
        assert_eq!(
            caret(&visual, &input),
            20,
            "the closest boundary on the last line"
        );

        click(&mut visual, at(origin, 1., 4.5), 1);
        assert_eq!(caret(&visual, &input), WRAPPED.len(), "below the text");
        assert_near(
            caret_position(&visual, &input),
            point(px(3. * ADVANCE), px(3. * ROW)),
        );

        click(&mut visual, at(origin, 3., 2.), 1);
        click_with(
            &mut visual,
            at(origin, 2.3, 1.),
            1,
            gpui::Modifiers::shift(),
        );
        assert_eq!(selection(&visual, &input), (17, 8), "shift-click extends");
    }

    #[test]
    fn a_long_line_hit_tests_the_painted_glyphs_to_its_end() {
        let text = "The quick brown fox jumps over the lazy dog, then naps.";
        let (mut visual, input) =
            open_field(600., move |focus| TextField::new(focus, text, "", false));
        let origin = text_origin(&visual, &input);
        for index in [0, 17, 40, text.len() - 1, text.len()] {
            click(
                &mut visual,
                origin + point(px((index as f32 + 0.3) * ADVANCE), px(5.)),
                1,
            );
            assert_eq!(caret(&visual, &input), index);
            assert_near(
                caret_position(&visual, &input),
                point(px(index as f32 * ADVANCE), px(0.)),
            );
        }
        click(&mut visual, origin + point(px(-4.), px(5.)), 1);
        assert_eq!(caret(&visual, &input), 0);
    }

    #[test]
    fn double_and_triple_clicks_select_the_word_and_line_under_the_pointer() {
        let (mut visual, input) = wrapped_field();
        let origin = text_origin(&visual, &input);
        let selected = |visual: &gpui::VisualTestContext| {
            input.read_with(visual, |field, _| {
                field.buffer.selected_text().unwrap_or_default().to_owned()
            })
        };

        click(&mut visual, at(origin, 4.8, 0.), 2);
        assert_eq!(
            selected(&visual),
            "alpha",
            "the right half of its last letter"
        );
        click(&mut visual, at(origin, 3.8, 1.), 2);
        assert_eq!(selected(&visual), "beta", "a word after the soft wrap");
        click(&mut visual, at(origin, 3., 2.), 2);
        assert_eq!(selected(&visual), "\n", "an empty line");
        click(&mut visual, at(origin, 1., 4.5), 2);
        assert_eq!(selected(&visual), "end", "below the text, the last word");

        click(&mut visual, at(origin, 1., 1.), 3);
        assert_eq!(
            selected(&visual),
            "alpha beta gamma\n",
            "the whole wrapped line"
        );
        click(&mut visual, at(origin, 3., 2.), 3);
        assert_eq!(selected(&visual), "\n");
    }

    #[test]
    fn a_drag_past_the_field_keeps_selecting_and_scrolls_until_the_button_is_up() {
        let text = (0..12)
            .map(|line| format!("line {line:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (mut visual, input) = open_field(300., move |focus| {
            TextField::textarea(focus, text, "", 3, false)
        });
        let origin = text_origin(&visual, &input);
        let field = visual.debug_bounds("FIELD_HOST").unwrap();
        let left = Some(MouseButton::Left);
        let none = gpui::Modifiers::none();

        visual.simulate_mouse_down(at(origin, 2., 0.), MouseButton::Left, none);
        visual.simulate_mouse_move(
            point(field.right() + px(50.), at(origin, 0., 1.).y),
            left,
            none,
        );
        assert_eq!(
            selection(&visual, &input),
            (2, 15),
            "right of the field, to the row's end"
        );

        visual.simulate_mouse_move(
            point(field.left() + px(20.), field.bottom() + px(30.)),
            left,
            none,
        );
        let below = caret(&visual, &input);
        assert!(
            below > 15,
            "below the field, into the rows under the pointer: {below}"
        );
        assert_eq!(
            scroll_y(&visual, &input),
            px(0.),
            "scrolling waits for the timer"
        );

        tick(&mut visual, 5);
        let scrolled = scroll_y(&visual, &input);
        assert!(scrolled < px(0.), "the field scrolls toward the pointer");
        assert!(caret(&visual, &input) > below, "and the selection follows");

        visual.simulate_mouse_up(
            point(field.left(), field.bottom() + px(30.)),
            MouseButton::Left,
            none,
        );
        let stopped = selection(&visual, &input);
        let stopped_at = scroll_y(&visual, &input);
        tick(&mut visual, 10);
        assert_eq!(
            scroll_y(&visual, &input),
            stopped_at,
            "mouse up stops scrolling"
        );

        visual.simulate_mouse_move(at(origin, 0., 0.), left, none);
        assert_eq!(
            selection(&visual, &input),
            stopped,
            "the listener is inert after mouse up"
        );

        let origin = text_origin(&visual, &input);
        assert_eq!(
            origin.y - field.top(),
            px(7. - 92.5),
            "five ticks scrolled 92.5 px"
        );
        visual.simulate_mouse_down(at(origin, 2., 5.), MouseButton::Left, none);
        visual.simulate_mouse_move(
            point(field.left() + px(20.), field.top() - px(40.)),
            left,
            none,
        );
        assert!(
            caret(&visual, &input) > 0,
            "above the field, a row scrolled out of view"
        );
        tick(&mut visual, 20);
        assert_eq!(
            scroll_y(&visual, &input),
            px(0.),
            "scrolled back to the top"
        );
        assert_eq!(caret(&visual, &input), 0, "above the text, its start");
        visual.simulate_mouse_up(at(origin, 0., 0.), MouseButton::Left, none);
    }

    #[test]
    fn up_and_down_move_by_visual_row_keeping_x() {
        let (mut visual, input) = wrapped_field();
        let origin = text_origin(&visual, &input);
        click(&mut visual, at(origin, 2., 1.), 1);
        assert_eq!(caret(&visual, &input), 8);

        visual.simulate_keystrokes("up");
        assert_eq!(caret(&visual, &input), 2, "across the soft wrap");
        visual.simulate_keystrokes("up");
        assert_eq!(caret(&visual, &input), 0, "from the first row, the start");
        visual.simulate_keystrokes("down");
        assert_eq!(caret(&visual, &input), 8, "back down, keeping x");
        visual.simulate_keystrokes("down");
        assert_eq!(caret(&visual, &input), 17, "onto the empty line");
        visual.simulate_keystrokes("down");
        assert_eq!(caret(&visual, &input), 20, "x kept through the empty line");
        visual.simulate_keystrokes("down");
        assert_eq!(
            caret(&visual, &input),
            WRAPPED.len(),
            "from the last row, the end"
        );

        visual.simulate_keystrokes("shift-up shift-up");
        assert_eq!(
            selection(&visual, &input),
            (WRAPPED.len(), 8),
            "shift extends"
        );
        visual.simulate_keystrokes("up");
        assert_eq!(
            selection(&visual, &input),
            (2, 2),
            "up from the selection's start"
        );

        click(&mut visual, at(origin, 9., 0.), 1);
        visual.simulate_keystrokes("down");
        assert_eq!(caret(&visual, &input), 12, "from the end of a wrapped row");

        click(&mut visual, at(origin, 2., 1.), 1);
        visual.simulate_keystrokes("down");
        assert_eq!(caret(&visual, &input), 17);
        visual.simulate_keystrokes("left right down");
        assert_eq!(
            caret(&visual, &input),
            18,
            "a sideways move drops the kept x, so Down starts from the empty line's own x"
        );
    }

    #[test]
    fn glyphs_out_of_source_order_map_only_to_grapheme_boundaries() {
        let cases = [
            // Shaped right to left, wrapped after two glyphs.
            (
                "abcd",
                vec![(3, px(0.)), (2, px(10.)), (1, px(20.)), (0, px(30.))],
                vec![2],
            ),
            // Wrapped inside a grapheme, before its combining accent.
            (
                "e\u{301}x",
                vec![(0, px(0.)), (1, px(10.)), (3, px(10.))],
                vec![1],
            ),
        ];
        for (source, glyphs, wraps) in cases {
            let geometry = LineGeometry::new(source, &glyphs, &wraps, px(40.));
            let boundaries = source
                .grapheme_indices(true)
                .map(|(index, _)| index)
                .chain([source.len()])
                .collect::<Vec<_>>();
            assert_eq!(geometry.rows(), 2);
            for row in 0..geometry.rows() {
                for x in [-5., 0., 5., 12., 25., 35., 60.] {
                    for under in [false, true] {
                        let (index, _) = geometry.index_at(row, px(x), under);
                        assert!(
                            boundaries.contains(&index),
                            "{source:?} row {row} at {x}: {index}"
                        );
                    }
                }
            }
            for index in 0..=source.len() + 1 {
                for upstream in [false, true] {
                    assert!(geometry.spot(index, upstream).row < geometry.rows());
                }
            }
            for (_, left, right) in geometry.spans(0..source.len(), Some(px(4.))) {
                assert!(left <= right);
            }
        }
    }

    #[test]
    fn a_textarea_scrolls_to_keep_the_caret_in_view() {
        let text = (0..12)
            .map(|line| format!("line {line:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        let len = text.len();
        let (mut visual, input) = open_field(300., move |focus| {
            TextField::textarea(focus, text, "", 3, false)
        });
        let origin = text_origin(&visual, &input);
        let bottom = px(-(12. - 3.) * ROW);
        click(&mut visual, at(origin, 1., 0.), 1);

        visual.simulate_keystrokes("end");
        assert_eq!(scroll_y(&visual, &input), bottom, "moving to the end");
        visual.simulate_keystrokes("home");
        assert_eq!(scroll_y(&visual, &input), px(0.), "moving to the start");
        visual.simulate_keystrokes("down down down down");
        assert_eq!(
            scroll_y(&visual, &input),
            px(-2. * ROW),
            "moving down a row at a time"
        );

        input.update(&mut visual, |field, cx| {
            field.buffer.move_to(len, false);
            cx.notify();
        });
        visual.run_until_parked();
        assert_eq!(
            scroll_y(&visual, &input),
            px(-2. * ROW),
            "only edits and moves scroll"
        );
        visual.simulate_input("x");
        assert_eq!(
            input.read_with(&visual, |field, _| field.text().len()),
            len + 1
        );
        assert_eq!(scroll_y(&visual, &input), bottom, "typing at the end");
    }

    #[test]
    fn auto_grow_follows_the_wrapped_rows_up_to_its_maximum() {
        let (mut visual, input) = open_field(110., |focus| {
            TextField::textarea(focus, "", "", 1, false).auto_grow(6)
        });
        let mut height_for = |text: &str| {
            input.update(&mut visual, |field, cx| field.reset(text.to_owned(), cx));
            visual.run_until_parked();
            visual.debug_bounds("FIELD_HOST").unwrap().size.height
        };
        let chrome = 14.;
        assert_eq!(height_for(""), px(ROW + chrome), "one row");
        assert_eq!(height_for("one"), px(ROW + chrome));
        assert_eq!(height_for("a\nb\nc"), px(3. * ROW + chrome), "three lines");
        assert_eq!(
            height_for("alpha beta gamma delta"),
            px(4. * ROW + chrome),
            "soft wraps grow it too: alpha / beta / gamma / delta"
        );
        assert_eq!(
            height_for(&"line\n".repeat(20)),
            px(6. * ROW + chrome),
            "capped at its maximum"
        );
    }

    #[test]
    fn ime_bounds_and_character_index_come_from_the_painted_layout() {
        let (mut visual, input) = wrapped_field();
        let origin = text_origin(&visual, &input);
        let host = visual.debug_bounds("FIELD_HOST").unwrap();
        let (beta, empty, across, index) = visual.update(|window, cx| {
            input.update(cx, |field, cx| {
                (
                    field.bounds_for_range(8..10, host, window, cx),
                    field.bounds_for_range(0..0, host, window, cx),
                    field.bounds_for_range(3..8, host, window, cx),
                    field.character_index_for_point(at(origin, 2.2, 1.), window, cx),
                )
            })
        });
        assert_bounds_near(
            beta,
            Bounds::from_corners(
                origin + point(px(2. * ADVANCE), px(ROW)),
                origin + point(px(4. * ADVANCE), px(2. * ROW)),
            ),
        );
        assert_eq!(beta.map(|beta| host.contains(&beta.center())), Some(true));
        assert_bounds_near(empty, Bounds::new(origin, size(px(0.), px(ROW))));
        // A range across a wrap reports its first row.
        assert_bounds_near(
            across,
            Bounds::from_corners(
                origin + point(px(3. * ADVANCE), px(0.)),
                origin + point(px(6. * ADVANCE), px(ROW)),
            ),
        );
        assert_eq!(index, Some(8));

        click(&mut visual, at(origin, 9., 0.), 1);
        assert_eq!(caret(&visual, &input), 6);
        let wrap_end = visual.update(|window, cx| {
            input.update(cx, |field, cx| {
                field.bounds_for_range(6..6, host, window, cx)
            })
        });
        assert_bounds_near(
            wrap_end,
            Bounds::new(
                origin + point(px(6. * ADVANCE), px(0.)),
                size(px(0.), px(ROW)),
            ),
        );
    }

    #[test]
    fn an_empty_field_places_the_caret_before_its_placeholder_and_takes_input() {
        let (mut visual, input) = open_field(300., |focus| {
            TextField::new(focus, "", "Search roles", false)
        });
        let origin = text_origin(&visual, &input);
        click(&mut visual, origin + point(px(40.), px(5.)), 1);
        assert_near(caret_position(&visual, &input), Point::default());
        visual.simulate_input("ab");
        visual.simulate_keystrokes("left");
        assert_eq!(
            input.read_with(&visual, |field, _| field.text().to_owned()),
            "ab"
        );
        assert_eq!(caret(&visual, &input), 1);
        visual.simulate_keystrokes("up");
        assert_eq!(caret(&visual, &input), 1, "a single-line field ignores Up");
    }

    #[test]
    fn a_path_compacts_to_home_then_folds_its_middle_keeping_the_last_folder() {
        let home = Some("/Users/jason");
        assert_eq!(
            compact_path_candidates("/Users/jason/repos/yicheng47", home),
            ["~/repos/yicheng47", "~/…/yicheng47"]
        );
        assert_eq!(compact_path_candidates("/Users/jason/", home), ["~"]);
        assert_eq!(
            compact_path_candidates("/Users/jasonx/repo", home),
            ["/Users/jasonx/repo", "/…/jasonx/repo", "/…/repo"],
            "a sibling of home is not under it"
        );
        assert_eq!(
            compact_path_candidates("/opt/work/projects/runner-app", None),
            [
                "/opt/work/projects/runner-app",
                "/…/work/projects/runner-app",
                "/…/projects/runner-app",
                "/…/runner-app",
            ]
        );
        assert_eq!(
            compact_path_candidates(r"C:\Users\jason\repos\runner", Some(r"C:\Users\jason")),
            [r"~\repos\runner", r"~\…\runner"]
        );
        assert_eq!(compact_path_candidates("/", home), ["/"]);
        assert_eq!(
            compact_path_candidates(r"C:\", None),
            [r"C:\"],
            "a drive root keeps its separator"
        );
        assert_eq!(compact_path_candidates(r"C:\Users", None), [r"C:\Users"]);
        assert_eq!(compact_path_candidates("dir", home), ["dir"]);
        assert_eq!(
            compact_path_candidates("very-long-parent-name/final", home),
            ["very-long-parent-name/final", "…/final"],
            "a shallow relative path folds its head to keep the last folder"
        );
        assert_eq!(
            compact_path_candidates("a/b/c", home),
            ["a/b/c", "a/…/c", "…/c"]
        );
    }

    #[test]
    fn the_longest_fitting_display_wins_else_the_shortest() {
        let candidates = || {
            vec![
                "~/repos/yicheng47/runner".to_owned(),
                "~/…/yicheng47/runner".to_owned(),
                "~/…/runner".to_owned(),
            ]
        };
        assert_eq!(
            pick_compact_path(candidates(), |candidate| candidate.chars().count() <= 20),
            "~/…/yicheng47/runner"
        );
        assert_eq!(pick_compact_path(candidates(), |_| false), "~/…/runner");
        assert_eq!(pick_compact_path(Vec::new(), |_| true), "");
        let shallow = compact_path_candidates("very-long-parent-name/final", None);
        assert_eq!(
            pick_compact_path(shallow, |candidate| candidate.chars().count() <= 10),
            "…/final",
            "where only the folded form fits, the last folder shows"
        );
    }

    #[test]
    fn an_unfocused_path_field_shows_what_fits_and_keeps_the_last_folder() {
        let path = "/opt/work/some-long-folder/another-long-folder/runner-app";
        let (mut visual, input) =
            open_field(200., move |focus| working_dir_text_field(focus, path, ""));
        let shown = input.read_with(&visual, |field, _| field.compact_path_shown(false));
        let shown = shown.expect("an unfocused path field compacts");
        assert_ne!(shown, path, "the full path does not fit 200 px");
        assert!(
            shown.starts_with("/…/") && shown.ends_with("/runner-app"),
            "{shown}"
        );
        let selector: &'static str = Box::leak(format!("TEXT_FIELD_COMPACT {shown}").into());
        assert!(
            visual.debug_bounds(selector).is_some(),
            "the field re-rendered with its pick, not only stored it"
        );
        assert_eq!(
            input.read_with(&visual, |field, _| field.compact_path_shown(true)),
            None,
            "focused, the field edits the full path"
        );
    }

    #[test]
    fn editing_shortcuts_use_command_on_macos_and_control_on_windows() {
        let cx = gpui::TestAppContext::single();
        for windows in [false, true] {
            cx.update(|cx| {
                let input = cx.new(|_| TextBuffer::default());
                input.update(cx, |input, cx| {
                    let press =
                        |input: &mut TextBuffer, key: &str, cx: &mut Context<TextBuffer>| {
                            handle_key_down_for_platform(
                                input,
                                &KeyDownEvent {
                                    keystroke: gpui::Keystroke::parse(key).unwrap(),
                                    is_held: false,
                                    prefer_character_input: false,
                                },
                                cx,
                                windows,
                            )
                        };
                    let command = if windows { "ctrl" } else { "cmd" };
                    input.reset("one two");
                    assert!(press(input, &format!("{command}-a"), cx));
                    assert_eq!(input.selected_text(), Some("one two"));
                    assert!(press(input, &format!("{command}-c"), cx));
                    assert_eq!(
                        cx.read_from_clipboard().unwrap().text().as_deref(),
                        Some("one two")
                    );
                    assert!(press(input, &format!("{command}-x"), cx));
                    assert_eq!(input.text, "");
                    assert!(press(input, &format!("{command}-v"), cx));
                    assert_eq!(input.text, "one two");
                    let word = if windows { "ctrl" } else { "alt" };
                    assert!(press(input, &format!("{word}-left"), cx));
                    assert_eq!(input.selection.caret, 4);
                    assert!(press(input, &format!("{word}-shift-right"), cx));
                    assert_eq!(input.selected_text(), Some("two"));
                    assert!(press(input, "right", cx));
                    assert_eq!(input.selection.caret, 7);
                    assert!(press(input, "home", cx));
                    assert_eq!(input.selection.caret, 0);
                    assert!(press(input, "end", cx));
                    assert_eq!(input.selection.caret, 7);
                    if !windows {
                        assert!(!press(input, "ctrl-a", cx));
                        assert!(press(input, "cmd-left", cx));
                        assert_eq!(input.selection.caret, 0);
                        assert!(press(input, "cmd-right", cx));
                        assert_eq!(input.selection.caret, 7);
                    }
                });
            });
        }
    }

    #[test]
    fn marked_text_uses_utf16_offsets_and_blocks_enter_until_committed() {
        let mut input = TextBuffer::default();
        input.reset("王菲");
        input.selection = Selection {
            anchor: "王".len(),
            caret: "王".len(),
        };

        assert!(input.replace_and_mark_text_in_range(None, "bianji", Some(6..6)));
        assert_eq!(input.text, "王bianji菲");
        assert_eq!(input.marked_text_range(), Some(1..7));
        assert!(!enter_should_submit(input.marked.is_some()));

        assert!(input.replace_and_mark_text_in_range(None, "编辑", Some(2..2)));
        assert!(!input.replace_text_in_range(None, "编辑"));
        assert_eq!(input.text, "王编辑菲");
        assert!(enter_should_submit(input.marked.is_some()));
    }

    #[test]
    fn deletion_respects_grapheme_clusters() {
        let mut input = TextBuffer::default();
        input.reset("A👨‍👩‍👧‍👦B");
        input.move_left(Boundary::Grapheme, false);

        assert!(input.delete_left(Boundary::Grapheme));
        assert_eq!(input.text, "AB");
        assert!(input.edited);
    }

    #[test]
    fn marked_text_updates_replace_the_existing_composition() {
        let mut input = TextBuffer::default();
        input.reset("王菲");
        input.selection = Selection {
            anchor: "王".len(),
            caret: "王".len(),
        };

        assert!(input.replace_and_mark_text_in_range(None, "bian", Some(4..4)));
        assert!(input.replace_and_mark_text_in_range(None, "编辑", Some(2..2)));

        assert_eq!(input.text, "王编辑菲");
        assert_eq!(input.marked_text_range(), Some(1..3));
        assert_eq!(input.selected_text_range().range, 3..3);
        assert_eq!(input.character_index_utf16(), 3);
    }

    #[test]
    fn selection_round_trips_through_utf16_for_clipboard_text() {
        let mut input = TextBuffer::default();
        input.reset("A😀王B");
        input.selection = Selection {
            anchor: "A😀王".len(),
            caret: "A".len(),
        };

        assert_eq!(input.selected_text(), Some("😀王"));
        assert_eq!(input.selected_text_range().range, 1..4);
        assert!(input.selected_text_range().reversed);

        let mut adjusted = None;
        assert_eq!(input.text_for_range(1..4, &mut adjusted), "😀王");
        assert_eq!(adjusted, Some(1..4));
    }

    #[test]
    fn pointer_selection_uses_byte_safe_word_and_line_ranges() {
        let mut input = TextBuffer::default();
        input.reset("alpha 王菲\nbeta");

        input.move_to("alpha ".len(), false);
        input.move_to("alpha 王菲".len(), true);
        assert_eq!(input.selected_text(), Some("王菲"));

        input.select_word_at("alpha ".len());
        assert_eq!(input.selected_text(), Some("王"));

        input.select_line_at("alpha 王".len());
        assert_eq!(input.selected_text(), Some("alpha 王菲\n"));
    }

    #[test]
    fn single_line_input_normalizes_commits_and_paste() {
        let mut input = TextBuffer::default();
        input.reset("");

        assert!(input.replace_text_in_range(None, "alpha\r\nbeta\ngamma"));
        assert_eq!(input.text, "alpha beta gamma");
    }

    #[test]
    fn enter_submits_only_after_composition_is_committed() {
        assert!(enter_should_submit(false));
        assert!(!enter_should_submit(true));
    }

    #[test]
    fn multiline_commits_preserve_lines_and_enter_inserts_a_line() {
        let mut input = TextBuffer {
            multiline: true,
            ..TextBuffer::default()
        };
        input.reset("alpha");

        assert!(input.replace_text_in_range(None, "beta\r\ngamma"));
        assert_eq!(input.text, "alphabeta\ngamma");
        assert_eq!(
            enter_behavior(TextFieldKind::Textarea { rows: 3 }, false),
            EnterBehavior::InsertNewline
        );
        assert_eq!(
            enter_behavior(TextFieldKind::Textarea { rows: 3 }, true),
            EnterBehavior::Block
        );
    }

    fn type_text(input: &mut TextBuffer, text: &str) {
        for character in text.chars() {
            input.replace_text_in_range(None, &character.to_string());
        }
    }

    fn caret_at(offset: usize) -> Selection {
        Selection {
            anchor: offset,
            caret: offset,
        }
    }

    #[test]
    fn typing_merges_into_one_step() {
        let mut input = TextBuffer::default();
        input.reset("say ");
        type_text(&mut input, "hello");
        assert_eq!(input.text, "say hello");
        assert_eq!(input.history.undo.len(), 1);

        input.edited = false;
        assert!(input.undo());
        assert_eq!(input.text, "say ");
        assert_eq!(input.selection, caret_at(4));
        assert!(input.edited, "an undo is an edit");
        assert!(!input.undo());
        assert!(input.redo());
        assert_eq!(input.text, "say hello");
        assert_eq!(input.selection, caret_at(9));
        assert!(!input.redo());
    }

    #[test]
    fn a_newline_splits_a_run_and_typing_after_it_is_a_new_step() {
        let mut input = TextBuffer {
            multiline: true,
            ..TextBuffer::default()
        };
        input.reset("");
        type_text(&mut input, "one");
        assert!(input.replace_selection("\n"), "Enter");
        type_text(&mut input, "two");
        type_text(&mut input, "\n");
        type_text(&mut input, "three");
        assert_eq!(input.text, "one\ntwo\nthree");

        for text in ["one\ntwo\n", "one\ntwo", "one\n", "one", ""] {
            assert!(input.undo());
            assert_eq!(input.text, text);
        }
        assert!(!input.undo());
    }

    #[test]
    fn paste_cut_and_delete_selection_are_one_step_each() {
        let mut input = TextBuffer::default();
        input.reset("");
        type_text(&mut input, "ab");
        assert!(input.replace_selection("cd"), "paste");
        type_text(&mut input, "ef");
        assert!(input.replace_selection("gh"), "paste");
        assert!(input.replace_selection("ij"), "paste");
        assert_eq!(input.text, "abcdefghij");
        assert_eq!(input.history.undo.len(), 5);

        input.select_all();
        assert!(input.delete_left(Boundary::Grapheme), "cut");
        assert_eq!(input.text, "");
        assert!(input.undo());
        assert_eq!(input.text, "abcdefghij");
        assert_eq!(
            input.selection,
            Selection {
                anchor: 0,
                caret: 10
            },
            "the cut text is selected again"
        );

        input.move_to(2, false);
        input.move_to(4, true);
        assert!(input.delete_left(Boundary::Grapheme), "Backspace over cd");
        input.move_to(0, false);
        input.move_to(2, true);
        assert!(input.delete_right(Boundary::Grapheme), "Delete over ab");
        assert_eq!(input.text, "efghij");

        for text in [
            "abefghij",
            "abcdefghij",
            "abcdefgh",
            "abcdef",
            "abcd",
            "ab",
            "",
        ] {
            assert!(input.undo());
            assert_eq!(input.text, text);
        }
    }

    #[test]
    fn backspace_and_forward_delete_runs_merge() {
        let mut input = TextBuffer::default();
        input.reset("a王菲bcdef");
        input.move_to("a王菲".len(), false);
        assert!(input.delete_left(Boundary::Grapheme));
        assert!(input.delete_left(Boundary::Grapheme));
        assert_eq!(input.text, "abcdef");
        input.move_to(2, false);
        assert!(input.delete_right(Boundary::Grapheme));
        assert!(input.delete_right(Boundary::Grapheme));
        assert_eq!(input.text, "abef");
        assert!(
            input.delete_left(Boundary::Grapheme),
            "a Backspace after Delete"
        );
        assert_eq!(input.history.undo.len(), 3);

        assert!(input.undo());
        assert_eq!(input.text, "abef");
        assert_eq!(input.selection, caret_at(2));
        assert!(input.undo());
        assert_eq!(input.text, "abcdef");
        assert_eq!(input.selection, caret_at(2));
        assert!(input.undo());
        assert_eq!(input.text, "a王菲bcdef");
        assert_eq!(input.selection, caret_at("a王菲".len()));

        input.reset("one two three");
        assert!(input.delete_left(Boundary::Word));
        assert!(input.delete_left(Boundary::Word));
        assert_eq!(input.history.undo.len(), 2, "word deletes are steps");
        assert!(input.delete_left(Boundary::Line));
        assert_eq!(input.history.undo.len(), 3, "so is a line delete");
    }

    #[test]
    fn a_caret_move_splits_typing() {
        let mut input = TextBuffer::default();
        input.reset("");
        type_text(&mut input, "ab");
        input.move_left(Boundary::Grapheme, false);
        input.move_right(Boundary::Grapheme, false);
        type_text(&mut input, "cd");
        input.select_all();
        input.move_to(4, false);
        type_text(&mut input, "e");
        input.select_word_at(0);
        input.move_to(5, false);
        type_text(&mut input, "f");
        assert_eq!(input.text, "abcdef");
        assert_eq!(input.history.undo.len(), 4);

        assert!(input.undo());
        assert!(input.undo());
        assert!(input.undo());
        assert_eq!(input.text, "ab");
        assert_eq!(input.selection, caret_at(2));
    }

    #[test]
    fn a_new_edit_clears_redo() {
        let mut input = TextBuffer::default();
        input.reset("");
        type_text(&mut input, "ab");
        assert!(input.replace_selection("cd"));
        assert!(input.undo());
        type_text(&mut input, "x");
        assert!(!input.redo());
        assert_eq!(input.text, "abx");
        assert!(input.undo());
        assert_eq!(input.text, "ab", "typing after an undo is a new step");
    }

    #[test]
    fn undo_and_redo_restore_the_selection() {
        let mut input = TextBuffer::default();
        input.reset("alpha beta");
        let beta = Selection {
            anchor: 10,
            caret: 6,
        };
        input.selection = beta;
        type_text(&mut input, "gamma");
        assert_eq!(input.text, "alpha gamma");
        assert_eq!(input.history.undo.len(), 2, "the replacement, then typing");

        assert!(input.undo());
        assert_eq!(input.text, "alpha g");
        assert_eq!(input.selection, caret_at(7));
        assert!(input.undo());
        assert_eq!(input.text, "alpha beta");
        assert_eq!(
            input.selection, beta,
            "the replaced text, as it was selected"
        );
        assert!(input.redo());
        assert_eq!(input.text, "alpha g");
        assert_eq!(input.selection, caret_at(7));
        assert!(input.redo());
        assert_eq!(input.text, "alpha gamma");
        assert_eq!(input.selection, caret_at(11), "the end of the merged run");
    }

    #[test]
    fn an_ime_composition_is_one_step_with_utf8_offsets() {
        let mut input = TextBuffer::default();
        input.reset("王菲");
        input.move_to("王".len(), false);
        for (marked, caret) in [("n", 1), ("ni", 2), ("ni h", 4), ("ni hao", 6)] {
            input.replace_and_mark_text_in_range(None, marked, Some(caret..caret));
            assert_eq!(input.text, format!("王{marked}菲"));
            assert!(input.history.undo.is_empty(), "{marked} is not a step");
            assert!(!input.undo(), "undo waits for the composition");
            assert!(!input.redo());
            assert_eq!(input.text, format!("王{marked}菲"));
        }
        assert!(input.replace_text_in_range(None, "你好"));
        assert_eq!(input.text, "王你好菲");
        assert!(input.marked.is_none());
        assert_eq!(input.history.undo.len(), 1);
        assert_eq!(
            input.history.undo[0].changes,
            [Change {
                offset: "王".len(),
                old_text: String::new(),
                new_text: "你好".into(),
            }]
        );

        assert!(input.undo());
        assert_eq!(input.text, "王菲");
        assert_eq!(input.selection, caret_at("王".len()));
        assert!(input.redo());
        assert_eq!(input.text, "王你好菲");
        assert_eq!(input.selection, caret_at("王你好".len()));

        type_text(&mut input, "!");
        assert_eq!(input.history.undo.len(), 2, "the next keystroke is a step");

        // A composition over a selection undoes to that selection.
        input.move_to("王你好!".len(), false);
        input.move_to("王你好!菲".len(), true);
        input.replace_and_mark_text_in_range(None, "fei", Some(3..3));
        input.replace_text_in_range(None, "飞");
        assert_eq!(input.text, "王你好!飞");
        assert!(input.undo());
        assert_eq!(input.text, "王你好!菲");
        assert_eq!(
            input.selection,
            Selection {
                anchor: "王你好!".len(),
                caret: "王你好!菲".len(),
            }
        );
    }

    #[test]
    fn a_cancelled_composition_records_nothing_and_an_unmarked_one_is_a_step() {
        let mut input = TextBuffer::default();
        input.reset("ab");
        input.replace_and_mark_text_in_range(None, "n", Some(1..1));
        input.replace_and_mark_text_in_range(None, "ni", Some(2..2));
        input.replace_and_mark_text_in_range(None, "", None);
        assert_eq!(input.text, "ab");
        assert!(input.marked.is_none());
        assert!(input.history.undo.is_empty());

        input.replace_and_mark_text_in_range(None, "ni", Some(2..2));
        input.replace_text_in_range(None, "");
        assert_eq!(input.text, "ab");
        assert!(input.history.undo.is_empty());

        input.replace_and_mark_text_in_range(None, "ni", Some(2..2));
        input.unmark_text();
        assert_eq!(input.text, "abni");
        assert_eq!(input.history.undo.len(), 1);

        input.replace_and_mark_text_in_range(None, "hao", Some(3..3));
        input.move_to(0, false);
        assert_eq!(input.history.undo.len(), 2, "a click keeps the composition");
        assert!(input.undo());
        assert_eq!(input.text, "abni");
        assert_eq!(input.selection, caret_at(4));
        assert!(input.undo());
        assert_eq!(input.text, "ab");
    }

    #[test]
    fn reset_clears_the_history() {
        let mut input = TextBuffer::default();
        input.reset("");
        type_text(&mut input, "ab");
        assert!(input.replace_selection("cd"));
        assert!(input.undo());
        input.replace_and_mark_text_in_range(None, "ni", Some(2..2));

        input.reset("fresh");
        assert!(!input.undo());
        assert!(!input.redo());
        assert_eq!(input.text, "fresh");
        type_text(&mut input, "!");
        assert!(input.undo());
        assert_eq!(input.text, "fresh");
        assert!(!input.undo());
    }

    #[test]
    fn set_text_is_one_step_and_corrects_the_edit_it_follows() {
        let mut input = TextBuffer::default();
        input.reset("draft");
        assert!(input.set_text("final copy"));
        assert!(!input.set_text("final copy"));
        assert_eq!(input.history.undo.len(), 1);
        assert!(input.undo());
        assert_eq!(input.text, "draft");
        assert_eq!(input.selection, caret_at(5));
        assert!(input.redo());
        assert_eq!(input.text, "final copy");
        assert_eq!(input.selection, caret_at(10));

        // The handle fields lowercase what was typed through `set_text`.
        type_text(&mut input, "A");
        assert!(input.set_text("final copya"));
        assert!(input.replace_selection("BC"));
        assert!(input.set_text("final copyabc"));
        assert_eq!(input.history.undo.len(), 3);
        assert!(input.undo());
        assert_eq!(input.text, "final copya");
        assert!(input.undo());
        assert_eq!(input.text, "final copy");
        assert_eq!(input.selection, caret_at(10));
        assert!(input.redo());
        assert_eq!(input.text, "final copya");
        assert_eq!(input.selection, caret_at(11));

        type_text(&mut input, "D");
        input.move_to(0, false);
        assert!(input.set_text("final copyad"));
        assert!(input.undo());
        assert_eq!(input.text, "final copyaD", "after a move it is a step");

        type_text(&mut input, "e");
        assert!(input.set_text("FINAL COPYAD"));
        assert!(input.undo());
        assert_eq!(input.text, "efinal copyaD", "so is uppercasing");
        assert!(input.undo());
        assert_eq!(input.text, "final copyaD");

        type_text(&mut input, "x");
        assert!(input.set_text("canonical"));
        assert!(input.undo());
        assert_eq!(
            input.text, "xfinal copyaD",
            "a new value after an edit is a step"
        );
        assert!(input.undo());
        assert_eq!(input.text, "final copyaD");
    }

    #[test]
    fn the_history_keeps_the_last_1000_steps() {
        let mut input = TextBuffer::default();
        input.reset("");
        for _ in 0..1100 {
            input.replace_selection("x");
        }
        assert_eq!(input.history.undo.len(), MAX_UNDO_STEPS);
        type_text(&mut input, "A");
        assert!(input.set_text(&format!("{}a", "x".repeat(1100))));
        assert_eq!(
            input.history.undo.len(),
            MAX_UNDO_STEPS,
            "a correction drops no step"
        );
        assert!(input.undo());
        assert_eq!(input.text, "x".repeat(1100));
        for _ in 1..MAX_UNDO_STEPS {
            assert!(input.undo());
        }
        assert!(!input.undo());
        assert_eq!(input.text, "x".repeat(101));
    }

    #[test]
    fn undo_and_redo_keys_follow_the_platform() {
        let cx = gpui::TestAppContext::single();
        for windows in [false, true] {
            cx.update(|cx| {
                let input = cx.new(|_| TextBuffer::default());
                input.update(cx, |input, cx| {
                    let press =
                        |input: &mut TextBuffer, key: &str, cx: &mut Context<TextBuffer>| {
                            handle_key_down_for_platform(
                                input,
                                &KeyDownEvent {
                                    keystroke: gpui::Keystroke::parse(key).unwrap(),
                                    is_held: false,
                                    prefer_character_input: false,
                                },
                                cx,
                                windows,
                            )
                        };
                    let command = if windows { "ctrl" } else { "cmd" };
                    input.reset("");
                    type_text(input, "one");
                    cx.write_to_clipboard(ClipboardItem::new_string(" two".into()));
                    assert!(press(input, &format!("{command}-v"), cx));
                    assert_eq!(input.text, "one two");
                    assert!(press(input, &format!("{command}-z"), cx));
                    assert_eq!(input.text, "one");
                    assert!(press(input, &format!("{command}-shift-z"), cx));
                    assert_eq!(input.text, "one two");
                    assert!(press(input, &format!("{command}-a"), cx));
                    assert!(press(input, &format!("{command}-x"), cx));
                    assert_eq!(input.text, "");
                    assert!(press(input, &format!("{command}-z"), cx));
                    assert_eq!(input.text, "one two");
                    assert_eq!(input.selected_text(), Some("one two"));
                    assert!(press(input, &format!("{command}-z"), cx));
                    assert!(press(input, &format!("{command}-z"), cx));
                    assert_eq!(input.text, "");
                    assert!(press(input, &format!("{command}-z"), cx), "nothing left");
                    if windows {
                        assert!(press(input, "ctrl-y", cx));
                        assert_eq!(input.text, "one");
                        assert!(!press(input, "cmd-z", cx));
                    } else {
                        assert!(!press(input, "cmd-y", cx));
                        assert!(!press(input, "ctrl-z", cx));
                        assert!(press(input, "cmd-shift-z", cx));
                        assert_eq!(input.text, "one");
                    }
                });
            });
        }
    }

    #[test]
    fn undo_and_redo_reach_an_enabled_field_by_key_and_action() {
        let (mut visual, input) = open_field(300., |focus| TextField::new(focus, "", "", false));
        let origin = text_origin(&visual, &input);
        let text = |visual: &gpui::VisualTestContext| {
            input.read_with(visual, |field, _| field.text().to_owned())
        };
        let undo = if cfg!(windows) { "ctrl-z" } else { "cmd-z" };
        click(&mut visual, origin + point(px(40.), px(5.)), 1);
        visual.simulate_input("one");
        visual.simulate_keystrokes("left");
        visual.simulate_input("x");
        assert_eq!(text(&visual), "onxe");

        input.update(&mut visual, |field, _| field.mark_clean());
        visual.simulate_keystrokes(undo);
        assert_eq!(text(&visual), "one");
        assert_eq!(caret(&visual, &input), 2);
        assert!(input.read_with(&visual, |field, _| field.edited()));
        visual.dispatch_action(Undo);
        assert_eq!(text(&visual), "");
        visual.dispatch_action(Redo);
        assert_eq!(text(&visual), "one");

        input.update(&mut visual, |field, cx| field.set_disabled(true, cx));
        visual.dispatch_action(Undo);
        visual.simulate_keystrokes(undo);
        assert_eq!(text(&visual), "one", "a disabled field keeps its text");
    }

    #[test]
    fn field_validation_exposes_only_error_messages() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        assert!(!FieldValidation::Valid.is_error());
        assert_eq!(FieldValidation::Valid.message(), None);

        let error = FieldValidation::error("Required");
        assert!(error.is_error());
        assert_eq!(error.message().map(SharedString::as_ref), Some("Required"));
        assert_eq!(input_border_color(&error, false), theme::danger());
        assert_eq!(input_border_color(&error, true), theme::danger());
        assert_eq!(
            input_border_color(&FieldValidation::Valid, true),
            theme::faint()
        );
    }

    #[test]
    fn working_directory_precedence_matches_main() {
        assert_eq!(
            working_dir_placeholder(Some("/runner"), "/default"),
            "/runner"
        );
        assert_eq!(working_dir_placeholder(None, "/default"), "/default");
        let home = runner_backend::app_paths::home_dir()
            .expect("home directory")
            .into_os_string()
            .into_string()
            .unwrap();
        assert_eq!(working_dir_placeholder(None, ""), home);

        assert_eq!(
            effective_working_dir(" /typed ", true, "/default"),
            Some("/typed".into())
        );
        assert_eq!(effective_working_dir("", true, "/default"), None);
        assert_eq!(
            effective_working_dir("", false, "/default"),
            Some("/default".into())
        );
        assert_eq!(effective_working_dir("", false, ""), Some(home.clone()));
        assert_eq!(effective_working_dir(" \t", false, " "), Some(home));
    }
}
