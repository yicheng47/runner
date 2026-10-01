use super::*;

use super::buffer::Boundary;

impl TextField {
    pub(super) fn on_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(super) fn on_copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.buffer.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
        }
    }

    pub(super) fn on_cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn on_paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn on_select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.buffer.select_all();
        self.vertical_goal = None;
        cx.notify();
    }

    pub(super) fn on_undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if !self.disabled && self.buffer.undo() {
            self.vertical_goal = None;
            self.reveal_caret();
            cx.notify();
        }
    }

    pub(super) fn on_redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if !self.disabled && self.buffer.redo() {
            self.vertical_goal = None;
            self.reveal_caret();
            cx.notify();
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

fn handle_key_down<T>(input: &mut TextBuffer, event: &KeyDownEvent, cx: &mut Context<T>) -> bool {
    handle_key_down_for_platform(input, event, cx, cfg!(windows))
}

pub(super) fn handle_key_down_for_platform<T>(
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
pub(super) enum EnterBehavior {
    Submit,
    InsertNewline,
    Block,
}

pub(super) fn enter_behavior(kind: TextFieldKind, composing: bool) -> EnterBehavior {
    if composing {
        EnterBehavior::Block
    } else if kind.multiline() {
        EnterBehavior::InsertNewline
    } else {
        EnterBehavior::Submit
    }
}

#[cfg(test)]
pub(super) fn enter_should_submit(composing: bool) -> bool {
    enter_behavior(TextFieldKind::Input, composing) == EnterBehavior::Submit
}

pub(super) fn normalize_input_text(text: &str, multiline: bool) -> String {
    if multiline {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text.replace("\r\n", " ").replace(['\r', '\n'], " ")
    }
}

pub(super) fn previous_word_boundary(text: &str, position: usize) -> usize {
    text.split_word_bound_indices()
        .take_while(|(start, _)| *start < position)
        .filter(|(_, segment)| is_word(segment))
        .map(|(start, _)| start)
        .fold(0, |_, start| start)
}

pub(super) fn next_word_boundary(text: &str, position: usize) -> usize {
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
