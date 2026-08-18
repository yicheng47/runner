use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    canvas, div, px, rems, AnyElement, App, Bounds, ClipboardItem, Context, CursorStyle,
    ElementInputHandler, EntityInputHandler, FocusHandle, Focusable, KeyDownEvent, MouseButton,
    Pixels, Point, Render, SharedString, UTF16Selection, Window,
};
use unicode_segmentation::UnicodeSegmentation as _;

use runner_app::text_util;

use crate::theme;
use crate::{Copy, Cut, Paste, SelectAll};

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
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct TextBuffer {
    text: String,
    selection: Selection,
    marked: Option<MarkedText>,
    edited: bool,
}

impl TextBuffer {
    fn reset(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.move_to_end();
        self.marked = None;
        self.edited = false;
    }

    fn move_to_end(&mut self) {
        let end = self.text.len();
        self.selection = Selection {
            anchor: end,
            caret: end,
        };
    }

    fn unmark_text(&mut self) {
        self.marked = None;
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
        let new_text = single_line(new_text);
        let changed = self.text[range.clone()] != new_text;
        self.text.replace_range(range.clone(), &new_text);
        let end = range.start + new_text.len();
        self.selection = Selection {
            anchor: end,
            caret: end,
        };
        self.marked = None;
        self.edited |= changed;
        changed
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
    ) -> bool {
        let range = self.resolve_range(range_utf16);
        let original = self
            .marked
            .as_ref()
            .filter(|marked| marked.range == range)
            .map(|marked| marked.original.clone())
            .unwrap_or_else(|| self.text[range.clone()].to_string());
        let new_text = single_line(new_text);
        let changed = self.text[range.clone()] != new_text;
        self.text.replace_range(range.clone(), &new_text);
        let marked_range = range.start..range.start + new_text.len();
        self.marked = (!new_text.is_empty()).then_some(MarkedText {
            range: marked_range.clone(),
            original,
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
        self.edited |= changed;
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
        self.replace_text_in_range(None, new_text)
    }

    fn select_all(&mut self) {
        self.selection = Selection {
            anchor: 0,
            caret: self.text.len(),
        };
        self.marked = None;
    }

    fn move_left(&mut self, boundary: Boundary, extend: bool) {
        if !extend && !self.selection.is_empty() {
            let start = self.selection.range().start;
            self.selection = Selection {
                anchor: start,
                caret: start,
            };
            self.marked = None;
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
            self.selection = Selection {
                anchor: end,
                caret: end,
            };
            self.marked = None;
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
        if extend {
            self.selection.caret = target;
        } else {
            self.selection = Selection {
                anchor: target,
                caret: target,
            };
        }
        self.marked = None;
    }

    fn delete_left(&mut self, boundary: Boundary) -> bool {
        let selection = self.selection.range();
        let range = if selection.is_empty() {
            let start = match boundary {
                Boundary::Grapheme => {
                    text_util::prev_grapheme_boundary(&self.text, selection.start)
                }
                Boundary::Word => previous_word_boundary(&self.text, selection.start),
                Boundary::Line => 0,
            };
            start..selection.start
        } else {
            selection
        };
        self.delete_range(range)
    }

    fn delete_right(&mut self, boundary: Boundary) -> bool {
        let selection = self.selection.range();
        let range = if selection.is_empty() {
            let end = match boundary {
                Boundary::Grapheme => text_util::next_grapheme_boundary(&self.text, selection.end),
                Boundary::Word => next_word_boundary(&self.text, selection.end),
                Boundary::Line => self.text.len(),
            };
            selection.end..end
        } else {
            selection
        };
        self.delete_range(range)
    }

    fn delete_range(&mut self, range: Range<usize>) -> bool {
        if range.is_empty() {
            return false;
        }
        self.text.replace_range(range.clone(), "");
        self.selection = Selection {
            anchor: range.start,
            caret: range.start,
        };
        self.marked = None;
        self.edited = true;
        true
    }
}

#[derive(Clone, Copy)]
enum Boundary {
    Grapheme,
    Word,
    Line,
}

pub(crate) struct ModalTextInput {
    focus_handle: FocusHandle,
    buffer: TextBuffer,
    placeholder: SharedString,
    monospace: bool,
}

impl ModalTextInput {
    pub(crate) fn new(
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
            monospace,
        }
    }

    pub(crate) fn text(&self) -> &str {
        &self.buffer.text
    }

    pub(crate) fn edited(&self) -> bool {
        self.buffer.edited
    }

    pub(crate) fn is_composing(&self) -> bool {
        self.buffer.marked.is_some()
    }

    pub(crate) fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    pub(crate) fn reset(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.buffer.reset(text);
        cx.notify();
    }

    pub(crate) fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "enter" {
            if !enter_should_submit(self.is_composing()) {
                cx.stop_propagation();
            }
            return;
        }
        let handled = handle_key_down(&mut self.buffer, event, cx);
        if handled {
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
        if let Some(text) = self.buffer.selected_text().map(str::to_owned) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.buffer.delete_left(Boundary::Grapheme);
            cx.notify();
        }
    }

    fn on_paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.buffer.replace_selection(&text);
            cx.notify();
        }
    }

    fn on_select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.buffer.select_all();
        cx.notify();
    }

    fn on_mouse_down(
        &mut self,
        _: &gpui::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window);
        self.buffer.move_to_end();
        cx.notify();
    }

    fn render_text(&self, focused: bool) -> AnyElement {
        if self.buffer.text.is_empty() {
            return div()
                .flex()
                .items_center()
                .min_w(px(0.))
                .text_color(theme::muted())
                .when(focused, |text| text.child(input_caret()))
                .child(self.placeholder.clone())
                .into_any_element();
        }

        let selection = focused.then_some(self.buffer.selection.range());
        let marked = focused
            .then(|| {
                self.buffer
                    .marked
                    .as_ref()
                    .map(|marked| marked.range.clone())
            })
            .flatten();
        let caret = self.buffer.selection.caret;
        let mut content = div()
            .flex()
            .items_center()
            .min_w(px(0.))
            .whitespace_nowrap();
        for (start, grapheme) in self.buffer.text.grapheme_indices(true) {
            if focused && caret == start {
                content = content.child(input_caret());
            }
            let end = start + grapheme.len();
            content = content.child(
                div()
                    .when(
                        selection
                            .as_ref()
                            .is_some_and(|range| range.start < end && start < range.end),
                        |text| text.bg(theme::with_alpha(theme::accent(), 0.267)),
                    )
                    .when(
                        marked
                            .as_ref()
                            .is_some_and(|range| range.start < end && start < range.end),
                        |text| text.border_b_1().border_color(theme::accent()),
                    )
                    .child(grapheme.to_string()),
            );
        }
        if focused && caret == self.buffer.text.len() {
            content = content.child(input_caret());
        }
        content.into_any_element()
    }
}

impl EntityInputHandler for ModalTextInput {
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
        self.buffer.replace_text_in_range(range, text);
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
        self.buffer
            .replace_and_mark_text_in_range(range, new_text, new_selected_range);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(element_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.buffer.character_index_utf16())
    }
}

impl Focusable for ModalTextInput {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ModalTextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);
        let input_entity = cx.entity();
        div()
            .relative()
            .flex()
            .items_center()
            .min_w(px(0.))
            .w_full()
            .h(rems(36. / 16.))
            .px_3()
            .overflow_hidden()
            .rounded_md()
            .border_1()
            .border_color(if focused {
                theme::muted()
            } else {
                theme::border()
            })
            .bg(theme::bg())
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_action(cx.listener(Self::on_cut))
            .on_action(cx.listener(Self::on_copy))
            .on_action(cx.listener(Self::on_paste))
            .on_action(cx.listener(Self::on_select_all))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .text_size(rems(13. / 16.))
            .text_color(theme::text())
            .when(self.monospace, |input| input.font_family("Menlo"))
            .child(self.render_text(focused))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        let focus = input_entity.read(cx).focus_handle.clone();
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, input_entity.clone()),
                            cx,
                        );
                    },
                )
                .absolute()
                .top_0()
                .right_0()
                .bottom_0()
                .left_0(),
            )
    }
}

fn handle_key_down<T>(input: &mut TextBuffer, event: &KeyDownEvent, cx: &mut Context<T>) -> bool {
    let key = event.keystroke.key.as_str();
    let modifiers = event.keystroke.modifiers;
    if modifiers.platform {
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
    if modifiers.alt {
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

fn enter_should_submit(composing: bool) -> bool {
    !composing
}

fn input_caret() -> impl IntoElement {
    div()
        .flex_none()
        .w(rems(1. / 16.))
        .h(rems(1.))
        .bg(theme::accent())
}

fn single_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
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
}
