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

use buffer::EditIntent;
use layout::LineGeometry;
use working_dir::home_dir;

mod buffer;
mod element;
mod form;
mod input;
mod layout;
mod pointer;
mod text_field;
mod working_dir;

pub use form::{BrowseField, Field, FieldError, Label};
pub use text_field::{FieldValidation, TextFieldKind};
pub(crate) use working_dir::{compact_path_candidates, pick_compact_path};
pub use working_dir::{
    effective_working_dir, working_dir_placeholder, working_dir_text_field, WorkingDirField,
};

#[cfg(test)]
mod tests;

pub type KeyDownInterceptor = Rc<dyn Fn(&KeyDownEvent, &mut Window, &mut App) -> bool>;

const AUTO_SCROLL_TICK: Duration = Duration::from_millis(16);

const MAX_UNDO_STEPS: usize = 1000;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Selection {
    anchor: usize,
    caret: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MarkedText {
    range: Range<usize>,
    original: String,
    /// The selection from before the composition, which undo restores.
    selection_before: Selection,
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct TextBuffer {
    text: String,
    selection: Selection,
    marked: Option<MarkedText>,
    edited: bool,
    multiline: bool,
    history: History,
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

/// One hard line of a field's text, shaped and wrapped.
struct FieldLine {
    start: usize,
    top: Pixels,
    layout: WrappedLine,
    geometry: LineGeometry,
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

/// Draws a text field's editable text from one shaped layout: glyphs,
/// selection, IME underline and caret, with the field's input handler and
/// its window-wide drag listener.
struct TextFieldElement {
    field: Entity<TextField>,
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
            .track_focus(&self.focus_handle.clone().tab_stop(!self.disabled))
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
