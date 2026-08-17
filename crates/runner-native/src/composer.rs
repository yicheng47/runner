//! Native IME-capable composer field (Phase 1 exit criterion 3:
//! Chinese input must work). Adapted from gpui's Apache-2.0
//! `examples/input.rs`, trimmed: no drag selection, no cut/copy, no
//! undo. Enter submits the buffer to the PTY.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    actions, div, fill, point, px, relative, size, App, Bounds, Context, CursorStyle, Element,
    ElementInputHandler, Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId,
    InspectorElementId, InteractiveElement as _, IntoElement, LayoutId, MouseButton,
    MouseDownEvent, ParentElement as _, Pixels, Render, ShapedLine, SharedString, Style,
    Styled as _, TextRun, UTF16Selection, UnderlineStyle, Window,
};

use runner_native::text_util;
use runner_terminal::terminal::TerminalSession;

use crate::theme;

actions!(
    composer,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        Paste,
        Submit,
        ShowCharacterPalette,
    ]
);

pub struct Composer {
    pub focus_handle: FocusHandle,
    session: Arc<TerminalSession>,
    content: SharedString,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
}

impl Composer {
    pub fn new(focus_handle: FocusHandle, session: Arc<TerminalSession>) -> Self {
        Self {
            focus_handle,
            session,
            content: "".into(),
            placeholder: "Message this chat — Enter to send".into(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
        }
    }

    fn submit(&mut self, _: &Submit, _window: &mut Window, cx: &mut Context<Self>) {
        // Never submit mid-IME-composition; Enter belongs to the IME.
        if self.marked_range.is_some() || self.content.is_empty() {
            return;
        }
        if let Err(error) = self.session.submit_text(&self.content) {
            self.placeholder = format!("Send failed: {error}").into();
            return;
        }
        self.content = "".into();
        self.selected_range = 0..0;
        self.marked_range = None;
        cx.notify();
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let start = self.previous_boundary(self.cursor_offset());
            self.selected_range = start..self.cursor_offset();
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let end = self.next_boundary(self.cursor_offset());
            self.selected_range = self.cursor_offset()..end;
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn left(&mut self, _: &Left, _window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    fn right(&mut self, _: &Right, _window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx);
    }

    fn home(&mut self, _: &Home, _window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text.replace('\n', " "), window, cx);
        }
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window);
        if let Some(index) = self.index_for_mouse_position(event.position) {
            if event.modifiers.shift {
                self.select_to(index, cx);
            } else {
                self.move_to(index, cx);
            }
        }
    }

    fn index_for_mouse_position(&self, position: gpui::Point<Pixels>) -> Option<usize> {
        let bounds = self.last_bounds?;
        let layout = self.last_layout.as_ref()?;
        if self.content.is_empty() {
            return Some(0);
        }
        Some(layout.closest_index_for_x(position.x - bounds.left()))
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify();
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        text_util::offset_to_utf16(&self.content, offset)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        text_util::range_to_utf16(&self.content, range)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        text_util::range_from_utf16(&self.content, range_utf16)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        text_util::prev_grapheme_boundary(&self.content, offset)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        text_util::next_grapheme_boundary(&self.content, offset)
    }
}

impl EntityInputHandler for Composer {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        adjusted_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range.as_ref().map(|r| self.range_to_utf16(r))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.marked_range.take();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        if !new_text.is_empty() {
            self.marked_range = Some(range.start..range.start + new_text.len());
        } else {
            self.marked_range = None;
        }
        self.selected_range =
            text_util::marked_selection(range.start, new_text, new_selected_range_utf16.as_ref());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        Some(Bounds::from_corners(
            point(
                bounds.left() + layout.x_for_index(range.start),
                bounds.top(),
            ),
            point(
                bounds.left() + layout.x_for_index(range.end),
                bounds.bottom(),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let layout = self.last_layout.as_ref()?;
        let bounds = self.last_bounds?;
        let utf8 = layout.index_for_x(position.x - bounds.left())?;
        Some(self.offset_to_utf16(utf8))
    }
}

impl Focusable for Composer {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

struct ComposerElement {
    composer: Entity<Composer>,
}

struct ComposerPrepaint {
    line: ShapedLine,
    cursor: Option<Bounds<Pixels>>,
    selection: Option<Bounds<Pixels>>,
}

impl Element for ComposerElement {
    type RequestLayoutState = ();
    type PrepaintState = ComposerPrepaint;

    fn id(&self) -> Option<gpui::ElementId> {
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
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> ComposerPrepaint {
        let composer = self.composer.read(cx);
        let content = composer.content.clone();
        let selected_range = composer.selected_range.clone();
        let cursor_offset = composer.cursor_offset();
        let marked_range = composer.marked_range.clone();
        let empty = content.is_empty();

        let display_text: SharedString = if empty {
            composer.placeholder.clone()
        } else {
            content
        };
        let text_color = if empty { theme::muted() } else { theme::text() };
        let base_run = TextRun {
            len: display_text.len(),
            font: gpui::font(crate::terminal_element::FONT_FAMILY),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = match marked_range.as_ref() {
            Some(marked) if !empty => vec![
                TextRun {
                    len: marked.start,
                    ..base_run.clone()
                },
                TextRun {
                    len: marked.end - marked.start,
                    underline: Some(UnderlineStyle {
                        color: Some(base_run.color),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    ..base_run.clone()
                },
                TextRun {
                    len: display_text.len() - marked.end,
                    ..base_run.clone()
                },
            ]
            .into_iter()
            .filter(|r| r.len > 0)
            .collect(),
            _ => vec![base_run],
        };

        let font_size = px(crate::terminal_element::FONT_SIZE);
        let line = window
            .text_system()
            .shape_line(display_text, font_size, &runs, None);

        let (cursor, selection) = if empty {
            (
                Some(Bounds::new(
                    point(bounds.left(), bounds.top()),
                    size(px(2.), bounds.size.height),
                )),
                None,
            )
        } else if selected_range.is_empty() {
            (
                Some(Bounds::new(
                    point(
                        bounds.left() + line.x_for_index(cursor_offset),
                        bounds.top(),
                    ),
                    size(px(2.), bounds.size.height),
                )),
                None,
            )
        } else {
            (
                None,
                Some(Bounds::from_corners(
                    point(
                        bounds.left() + line.x_for_index(selected_range.start),
                        bounds.top(),
                    ),
                    point(
                        bounds.left() + line.x_for_index(selected_range.end),
                        bounds.bottom(),
                    ),
                )),
            )
        };

        ComposerPrepaint {
            line,
            cursor,
            selection,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut (),
        prepaint: &mut ComposerPrepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.composer.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.composer.clone()),
            cx,
        );
        if let Some(selection) = prepaint.selection {
            window.paint_quad(fill(selection, theme::selection()));
        }
        let _ = prepaint
            .line
            .paint(bounds.origin, bounds.size.height, window, cx);
        if focus_handle.is_focused(window) {
            if let Some(cursor) = prepaint.cursor {
                window.paint_quad(fill(cursor, theme::accent()));
            }
        }
        self.composer.update(cx, |composer, _| {
            composer.last_layout = Some(prepaint.line.clone());
            composer.last_bounds = Some(bounds);
        });
    }
}

impl IntoElement for ComposerElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Render for Composer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("composer")
            .key_context("Composer")
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::show_character_palette))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .flex_none()
            .w_full()
            .px_3()
            .py_2()
            .bg(theme::composer_bg())
            .border_t_1()
            .border_color(theme::border())
            .child(ComposerElement {
                composer: cx.entity(),
            })
    }
}
