use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    canvas, div, point, px, rems, svg, AnyElement, App, Bounds, BoxShadow, ClipboardItem, Context,
    CursorStyle, ElementInputHandler, Entity, EntityInputHandler, FocusHandle, Focusable,
    FontWeight, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, Render, RenderOnce, ScrollHandle, SharedString, UTF16Selection,
    Window, WrappedLine,
};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::text_util;
use crate::theme;
use crate::ui::button::{Button, PressHandler};
use crate::ui::scrollbar::Scrollbar;
use crate::ui::tooltip::Tooltip;
use crate::{Copy, Cut, Paste, SelectAll};

pub type KeyDownInterceptor = Rc<dyn Fn(&KeyDownEvent, &mut Window, &mut App) -> bool>;

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
    multiline: bool,
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
        let new_text = normalize_input_text(new_text, self.multiline);
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
        let new_text = normalize_input_text(new_text, self.multiline);
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

    fn select_word_at(&mut self, position: usize) {
        let position = position.min(self.text.len());
        let range = self
            .text
            .split_word_bound_indices()
            .find_map(|(start, segment)| {
                let end = start + segment.len();
                (start <= position && (position < end || position == self.text.len()))
                    .then_some(start..end)
            })
            .unwrap_or(position..position);
        self.selection = Selection {
            anchor: range.start,
            caret: range.end,
        };
        self.marked = None;
    }

    fn select_line_at(&mut self, position: usize) {
        let position = position.min(self.text.len());
        let start = self.text[..position]
            .rfind('\n')
            .map_or(0, |offset| offset + 1);
        let end = self.text[position..]
            .find('\n')
            .map_or(self.text.len(), |offset| position + offset + 1);
        self.selection = Selection {
            anchor: start,
            caret: end,
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

struct TextFieldLayoutLine {
    start: usize,
    top: Pixels,
    layout: WrappedLine,
}

struct TextFieldLayout {
    origin: Point<Pixels>,
    line_height: Pixels,
    lines: Vec<TextFieldLayoutLine>,
    text_len: usize,
    multiline: bool,
}

impl TextFieldLayout {
    fn new(
        text: &str,
        kind: TextFieldKind,
        bare: bool,
        right_padding: f32,
        scroll_offset: Point<Pixels>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
    ) -> Self {
        let rem_size = window.rem_size();
        let left_padding = if bare { px(0.) } else { rem_size * (10. / 16.) };
        let right_padding = if bare {
            px(0.)
        } else {
            rem_size * (right_padding / 16.)
        };
        let top_padding = if bare || !kind.multiline() {
            px(0.)
        } else {
            rem_size * (6. / 16.)
        };
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(rem_size);
        let line_height = if kind.multiline() {
            rem_size * (20. / 16.)
        } else {
            text_style.line_height_in_pixels(rem_size)
        };
        let content_width = (bounds.size.width - left_padding - right_padding).max(px(0.));
        let shaped = window
            .text_system()
            .shape_text(
                text.to_owned().into(),
                font_size,
                &[text_style.to_run(text.len())],
                kind.multiline().then_some(content_width),
                None,
            )
            .unwrap_or_default();
        let mut start = 0;
        let mut top = px(0.);
        let mut lines = Vec::with_capacity(shaped.len());
        for (source, layout) in text.split('\n').zip(shaped) {
            let row_count = layout.wrap_boundaries().len() + 1;
            lines.push(TextFieldLayoutLine { start, top, layout });
            start += source.len() + usize::from(start + source.len() < text.len());
            top += line_height * row_count as f32;
        }
        Self {
            origin: bounds.origin + point(left_padding, top_padding) + scroll_offset,
            line_height,
            lines,
            text_len: text.len(),
            multiline: kind.multiline(),
        }
    }

    fn index_for_point(&self, screen_point: Point<Pixels>) -> usize {
        let local = screen_point - self.origin;
        if !self.multiline {
            return self.lines.first().map_or(0, |line| {
                line.start
                    + line
                        .layout
                        .closest_index_for_position(point(local.x, px(0.)), self.line_height)
                        .unwrap_or_else(|index| index)
            });
        }
        if local.y < px(0.) {
            return 0;
        }
        for line in &self.lines {
            let height = self.line_height * (line.layout.wrap_boundaries().len() + 1) as f32;
            if local.y < line.top + height {
                let relative = point(local.x, local.y - line.top);
                return line.start
                    + line
                        .layout
                        .closest_index_for_position(relative, self.line_height)
                        .unwrap_or_else(|index| index);
            }
        }
        self.text_len
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
    kind: TextFieldKind,
    disabled: bool,
    validation: FieldValidation,
    bare: bool,
    text_size: f32,
    right_padding: f32,
    hover_border: bool,
    disabled_cursor_not_allowed: bool,
    scroll_handle: ScrollHandle,
    scrollbar: Option<Entity<Scrollbar>>,
    selecting: bool,
    text_layout: Rc<RefCell<Option<TextFieldLayout>>>,
    auto_grow_rows: Option<u8>,
    key_interceptor: Option<KeyDownInterceptor>,
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
            kind: TextFieldKind::Input,
            disabled: false,
            validation: FieldValidation::Valid,
            bare: false,
            text_size: 14.,
            right_padding: 10.,
            hover_border: false,
            disabled_cursor_not_allowed: false,
            scroll_handle: ScrollHandle::new(),
            scrollbar: None,
            selecting: false,
            text_layout: Rc::new(RefCell::new(None)),
            auto_grow_rows: None,
            key_interceptor: None,
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

    pub fn key_interceptor(mut self, interceptor: KeyDownInterceptor) -> Self {
        self.key_interceptor = Some(interceptor);
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

    pub fn text_size(mut self, text_size: f32) -> Self {
        self.text_size = text_size;
        self
    }

    pub fn right_padding(mut self, right_padding: f32) -> Self {
        self.right_padding = right_padding;
        self
    }

    pub fn reset(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.buffer.reset(text);
        cx.notify();
    }

    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.buffer.reset(text);
        self.buffer.edited = true;
        cx.notify();
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.buffer.select_all();
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
                    cx.stop_propagation();
                    cx.notify();
                }
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
        if self.disabled {
            return;
        }
        if let Some(text) = self.buffer.selected_text().map(str::to_owned) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.buffer.delete_left(Boundary::Grapheme);
            cx.notify();
        }
    }

    fn on_paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
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
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        self.focus_handle.focus(window);
        let position = self
            .text_layout
            .borrow()
            .as_ref()
            .map_or(self.buffer.text.len(), |layout| {
                layout.index_for_point(event.position)
            });
        self.selecting = true;
        match event.click_count {
            2 => self.buffer.select_word_at(position),
            count if count >= 3 => self.buffer.select_line_at(position),
            _ => self.buffer.move_to(position, event.modifiers.shift),
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selecting || !event.dragging() {
            return;
        }
        let Some(position) = self
            .text_layout
            .borrow()
            .as_ref()
            .map(|layout| layout.index_for_point(event.position))
        else {
            return;
        };
        self.buffer.move_to(position, true);
        cx.stop_propagation();
        cx.notify();
    }

    fn on_mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.selecting = false;
    }

    fn render_text(&self, focused: bool) -> AnyElement {
        if self.buffer.text.is_empty() {
            return div()
                .flex()
                .items_center()
                .min_h(rems(1.))
                .min_w(px(0.))
                .text_color(if self.placeholder_as_value {
                    theme::text()
                } else {
                    theme::faint()
                })
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
        if self.kind == TextFieldKind::Input {
            return self.render_text_line(0, &self.buffer.text, focused, &selection, &marked);
        }

        let mut content = div().flex().flex_col().min_w(px(0.)).w_full();
        let mut offset = 0;
        for segment in self.buffer.text.split_inclusive('\n') {
            let line = segment.strip_suffix('\n').unwrap_or(segment);
            content =
                content.child(self.render_text_line(offset, line, focused, &selection, &marked));
            offset += segment.len();
        }
        if self.buffer.text.ends_with('\n') {
            content = content.child(self.render_text_line(
                self.buffer.text.len(),
                "",
                focused,
                &selection,
                &marked,
            ));
        }
        content.into_any_element()
    }

    fn render_text_line(
        &self,
        line_start: usize,
        line: &str,
        focused: bool,
        selection: &Option<Range<usize>>,
        marked: &Option<Range<usize>>,
    ) -> AnyElement {
        let caret = self.buffer.selection.caret;
        let mut content = div()
            .flex()
            .items_center()
            .min_w(px(0.))
            .min_h(rems(1.))
            .when(self.kind == TextFieldKind::Input, |line| {
                line.whitespace_nowrap()
            })
            .when(self.kind != TextFieldKind::Input, |line| line.flex_wrap());
        for (relative_start, grapheme) in line.grapheme_indices(true) {
            let start = line_start + relative_start;
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
        if focused && caret == line_start + line.len() {
            content = content.child(input_caret());
        }
        content.into_any_element()
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
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        self.text_layout.borrow().as_ref().map_or_else(
            || Some(self.buffer.character_index_utf16()),
            |layout| {
                Some(text_util::offset_to_utf16(
                    &self.buffer.text,
                    layout.index_for_point(point),
                ))
            },
        )
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
        let input_entity = cx.entity();
        if self.kind != TextFieldKind::Input && self.scrollbar.is_none() {
            let owner = cx.entity_id();
            let scroll_handle = self.scroll_handle.clone();
            self.scrollbar = Some(cx.new(|_| Scrollbar::app(scroll_handle, owner)));
        }
        let multiline = self.kind != TextFieldKind::Input;
        let scrollbar = self.scrollbar.clone();
        let text_layout = Rc::clone(&self.text_layout);
        let layout_text = self.buffer.text.clone();
        let layout_kind = self.kind;
        let layout_bare = self.bare;
        let layout_right_padding = self.right_padding;
        let layout_scroll_handle = self.scroll_handle.clone();
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
            .when(auto_grow.is_none(), |input| input.h(rems(height / 16.)))
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
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .text_size(rems(self.text_size / 16.))
            .when(multiline, |input| input.line_height(rems(20. / 16.)))
            .text_color(theme::text())
            .when(self.monospace, |input| input.font_family("JetBrains Mono"))
            .when(self.kind == TextFieldKind::Input, |input| {
                input.child(self.render_text(focused))
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
                        .child(self.render_text(focused)),
                )
            })
            .when(!self.disabled, |input| {
                input.child(
                    canvas(
                        move |bounds, window, _| {
                            text_layout.replace(Some(TextFieldLayout::new(
                                &layout_text,
                                layout_kind,
                                layout_bare,
                                layout_right_padding,
                                if layout_kind.multiline() {
                                    layout_scroll_handle.offset()
                                } else {
                                    Point::default()
                                },
                                bounds,
                                window,
                            )));
                        },
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
            .text_size(rems(12. / 16.))
            .text_color(if self.emphasized {
                theme::text()
            } else {
                theme::muted()
            })
            .when_some(focus_target, |label, focus| {
                label.cursor(CursorStyle::PointingHand).on_mouse_down(
                    MouseButton::Left,
                    move |_, window, _| {
                        focus.focus(window);
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
                            }])
                        })
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            hint_focus.focus(window);
                            cx.stop_propagation();
                        })
                        .child(
                            svg()
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
            .text_size(rems(12. / 16.))
            .text_color(theme::danger())
            .child(self.message)
    }
}

#[derive(IntoElement)]
pub struct Field {
    id: SharedString,
    label: SharedString,
    hint: Option<(SharedString, FocusHandle)>,
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
            .child(label)
            .child(self.child)
            .children(self.subtitle.map(|subtitle| {
                div()
                    .text_size(rems(11. / 16.))
                    .text_color(theme::faint())
                    .child(subtitle)
            }))
            .children(self.error.map(FieldError::new))
    }
}

#[derive(IntoElement)]
pub struct WorkingDirField {
    input: Entity<TextField>,
    disabled: bool,
    browse_focus: Option<FocusHandle>,
    on_browse: PressHandler,
}

impl WorkingDirField {
    pub fn new(input: Entity<TextField>, disabled: bool, on_browse: PressHandler) -> Self {
        Self {
            input,
            disabled,
            browse_focus: None,
            on_browse,
        }
    }

    pub fn browse_focus(mut self, browse_focus: FocusHandle) -> Self {
        self.browse_focus = Some(browse_focus);
        self
    }
}

impl RenderOnce for WorkingDirField {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let browse = Rc::clone(&self.on_browse);
        div()
            .flex()
            .items_center()
            .gap(rems(8. / 16.))
            .child(div().flex_1().min_w(px(0.)).child(self.input))
            .child(
                Button::new("working-dir-browse", "Browse")
                    .when_some(self.browse_focus, |button, focus| {
                        button.focus_handle(focus)
                    })
                    .disabled(self.disabled)
                    .on_press(move |window, cx| browse(window, cx)),
            )
    }
}

pub fn working_dir_placeholder(owner_path: Option<&str>, default_path: &str) -> String {
    owner_path
        .filter(|path| !path.trim().is_empty())
        .or_else(|| (!default_path.trim().is_empty()).then_some(default_path.trim()))
        .unwrap_or("(no working directory)")
        .to_owned()
}

pub fn working_dir_text_field(
    focus_handle: FocusHandle,
    text: impl Into<String>,
    placeholder: impl Into<SharedString>,
) -> TextField {
    TextField::new(focus_handle, text, placeholder, true)
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
    (!default_path.is_empty()).then(|| default_path.to_owned())
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

fn input_caret() -> impl IntoElement {
    div()
        .flex_none()
        .w(rems(1. / 16.))
        .h(rems(1.))
        .bg(theme::accent())
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

    #[test]
    fn field_validation_exposes_only_error_messages() {
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
        assert_eq!(working_dir_placeholder(None, ""), "(no working directory)");

        assert_eq!(
            effective_working_dir(" /typed ", true, "/default"),
            Some("/typed".into())
        );
        assert_eq!(effective_working_dir("", true, "/default"), None);
        assert_eq!(
            effective_working_dir("", false, "/default"),
            Some("/default".into())
        );
        assert_eq!(effective_working_dir("", false, ""), None);
    }
}
