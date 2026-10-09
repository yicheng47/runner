use super::*;

use super::input::next_word_boundary;
use super::input::normalize_input_text;
use super::input::previous_word_boundary;

impl Selection {
    pub(super) fn range(self) -> Range<usize> {
        self.anchor.min(self.caret)..self.anchor.max(self.caret)
    }

    pub(super) fn is_empty(self) -> bool {
        self.anchor == self.caret
    }
}

/// What an edit does, which decides whether it merges into the step before
/// it. The history follows gpui-kit's undo manager.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EditIntent {
    Typing,
    Backspace,
    DeleteForward,
    Atomic,
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

impl TextBuffer {
    pub(super) fn reset(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.move_to_end();
        self.marked = None;
        self.edited = false;
        self.history = History::default();
    }

    /// Replaces the whole text as one step. Straight after an edit, the text
    /// lowercased is the handle fields correcting that edit, so it joins the
    /// edit's step and one undo takes back both.
    pub(super) fn set_text(&mut self, text: &str) -> bool {
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

    pub(super) fn replace_range(&mut self, range: Range<usize>, text: &str) -> bool {
        if self.marked.is_some() || self.text.get(range.clone()).is_none() {
            return false;
        }
        let text = normalize_input_text(text, self.multiline);
        self.edit(range, &text, EditIntent::Atomic);
        true
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

    pub(super) fn undo(&mut self) -> bool {
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

    pub(super) fn redo(&mut self) -> bool {
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

    pub(super) fn unmark_text(&mut self) {
        self.end_composition();
    }

    pub(super) fn text_for_range(
        &self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
    ) -> String {
        let range = text_util::range_from_utf16(&self.text, &range_utf16);
        adjusted_range.replace(text_util::range_to_utf16(&self.text, &range));
        self.text[range].to_string()
    }

    pub(super) fn selected_text_range(&self) -> UTF16Selection {
        UTF16Selection {
            range: text_util::range_to_utf16(&self.text, &self.selection.range()),
            reversed: self.selection.caret < self.selection.anchor,
        }
    }

    pub(super) fn marked_text_range(&self) -> Option<Range<usize>> {
        self.marked
            .as_ref()
            .map(|marked| text_util::range_to_utf16(&self.text, &marked.range))
    }

    pub(super) fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
    ) -> bool {
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

    pub(super) fn replace_and_mark_text_in_range(
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

    pub(super) fn character_index_utf16(&self) -> usize {
        text_util::offset_to_utf16(&self.text, self.selection.caret)
    }

    fn resolve_range(&self, range_utf16: Option<Range<usize>>) -> Range<usize> {
        range_utf16
            .map(|range| text_util::range_from_utf16(&self.text, &range))
            .or_else(|| self.marked.as_ref().map(|marked| marked.range.clone()))
            .unwrap_or_else(|| self.selection.range())
    }

    pub(super) fn selected_text(&self) -> Option<&str> {
        let range = self.selection.range();
        (!range.is_empty()).then(|| &self.text[range])
    }

    pub(super) fn replace_selection(&mut self, new_text: &str) -> bool {
        let range = self.resolve_range(None);
        let new_text = normalize_input_text(new_text, self.multiline);
        self.replace(range, &new_text, EditIntent::Atomic)
    }

    pub(super) fn select_all(&mut self) {
        self.leave_run();
        self.selection = Selection {
            anchor: 0,
            caret: self.text.len(),
        };
    }

    pub(super) fn select_word_at(&mut self, position: usize) {
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

    pub(super) fn select_line_at(&mut self, position: usize) {
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

    pub(super) fn move_left(&mut self, boundary: Boundary, extend: bool) {
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

    pub(super) fn move_right(&mut self, boundary: Boundary, extend: bool) {
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

    pub(super) fn move_to(&mut self, target: usize, extend: bool) {
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

    pub(super) fn delete_left(&mut self, boundary: Boundary) -> bool {
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

    pub(super) fn delete_right(&mut self, boundary: Boundary) -> bool {
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
pub(super) enum Boundary {
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
