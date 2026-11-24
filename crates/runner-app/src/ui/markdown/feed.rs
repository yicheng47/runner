use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    canvas, div, px, AnyElement, App, Bounds, DispatchPhase, EntityId, Hsla, MouseExitEvent,
    MouseMoveEvent, Pixels, Window,
};

use super::draw::{
    note_compact_exit, note_compact_move, note_compact_press, note_compact_release, Draw, Surface,
};
use super::layout::{selected_range, Row};
use super::view::Position;
use super::Page;

#[cfg(test)]
thread_local! {
    pub(crate) static PARSE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static RENDER_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static LINE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FeedSelection {
    pub(crate) event_id: String,
    pub(crate) anchor: Position,
    pub(crate) head: Position,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FeedSelectionPhase {
    Begin,
    Extend,
    End,
}

type FeedSelectionCallback = dyn Fn(FeedSelectionPhase, &str, Position, &mut Window, &mut App);

#[derive(Clone)]
pub(crate) struct FeedSelectionHandler(Rc<FeedSelectionCallback>);

impl FeedSelectionHandler {
    pub(crate) fn new(
        callback: impl Fn(FeedSelectionPhase, &str, Position, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self(Rc::new(callback))
    }

    pub(super) fn handle(
        &self,
        phase: FeedSelectionPhase,
        event_id: &str,
        position: Position,
        window: &mut Window,
        cx: &mut App,
    ) {
        (self.0)(phase, event_id, position, window, cx);
    }
}

/// A feed message or question card parsed once in the Compact look and sliced
/// into chunks of up to [`Self::ROWS_PER_ENTRY`] rows for the virtualized feed.
pub(crate) struct FeedMarkdown {
    page: Page,
}

impl FeedMarkdown {
    const ROWS_PER_ENTRY: usize = 4;

    pub(crate) fn new(text: &str) -> Self {
        Self {
            page: Page::parse_compact(text),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.page.rows.is_empty()
    }

    pub(crate) fn row_count(&self) -> usize {
        self.page.rows.len().div_ceil(Self::ROWS_PER_ENTRY).max(1)
    }

    pub(crate) fn selected_text(&self, selection: &FeedSelection) -> Option<String> {
        self.page
            .selected_text(selection.anchor, selection.head)
            .filter(|text| !text.is_empty())
    }

    #[cfg(test)]
    pub(crate) fn page(&self) -> &Page {
        &self.page
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_row(
        &self,
        chunk: usize,
        event_id: &str,
        view: EntityId,
        selection: Option<&FeedSelection>,
        selection_color: Hsla,
        selection_handler: &FeedSelectionHandler,
        cx: &App,
    ) -> AnyElement {
        let start = chunk * Self::ROWS_PER_ENTRY;
        let end = (start + Self::ROWS_PER_ENTRY).min(self.page.rows.len());
        render_compact_rows(
            &self.page,
            start..end,
            event_id,
            view,
            selection,
            selection_color,
            Some(selection_handler),
            cx,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_all(
        &self,
        event_id: &str,
        view: EntityId,
        selection: Option<&FeedSelection>,
        selection_color: Hsla,
        selection_handler: Option<&FeedSelectionHandler>,
        cx: &App,
    ) -> AnyElement {
        render_compact_rows(
            &self.page,
            0..self.page.rows.len(),
            event_id,
            view,
            selection,
            selection_color,
            selection_handler,
            cx,
        )
    }
}

/// Renders short read-only Markdown in the Compact look (role and crew prompts
/// and previews, Skills preview, Info popover System prompt).
pub(crate) fn render_compact(id: &str, text: &str, view: EntityId, cx: &App) -> AnyElement {
    let page = Page::parse_compact(text);
    render_compact_rows(
        &page,
        0..page.rows.len(),
        id,
        view,
        None,
        crate::theme::accent(),
        None,
        cx,
    )
}

fn selection_for_row(
    selection: Option<&FeedSelection>,
    event_id: &str,
    row: &Row,
    index: usize,
) -> Range<usize> {
    let Some(selection) = selection.filter(|s| s.event_id == event_id && s.anchor != s.head) else {
        return 0..0;
    };
    let (start, end) = (
        selection.anchor.min(selection.head),
        selection.anchor.max(selection.head),
    );
    if !(start.row..=end.row).contains(&index) {
        return 0..0;
    }
    let text = row.text();
    selected_range(row, index, start, end, &text)
}

#[allow(clippy::too_many_arguments)]
fn render_compact_rows(
    page: &Page,
    range: Range<usize>,
    doc_id: &str,
    view: EntityId,
    selection: Option<&FeedSelection>,
    selection_color: Hsla,
    selection_handler: Option<&FeedSelectionHandler>,
    cx: &App,
) -> AnyElement {
    let is_end = range.end == page.rows.len();
    let bounds_cell = Rc::new(Cell::new(Bounds::<Pixels>::default()));
    let down_bounds = Rc::clone(&bounds_cell);
    let paint_bounds = Rc::clone(&bounds_cell);
    let tracker = canvas(
        move |bounds, _, _| paint_bounds.set(bounds),
        move |_, _, window, _| {
            // Track window-wide pointer moves and window exits so a press that
            // leaves this preview's hitbox and returns to its origin still
            // cancels link activation.
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture {
                    note_compact_move(event.position, event.pressed_button, cx);
                }
            });
            window.on_mouse_event(move |_: &MouseExitEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture {
                    note_compact_exit(cx);
                }
            });
        },
    )
    .absolute()
    .inset_0();
    div()
        .relative()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .when(is_end && page.bottom > 0., |body| body.pb(px(page.bottom)))
        .capture_any_mouse_down(move |event, _, cx| {
            note_compact_press(event.button, event.position, down_bounds.get(), cx);
        })
        .capture_any_mouse_up(|event, _, cx| {
            note_compact_release(event.button, event.position, cx);
        })
        .children(range.map(|index| {
            let row = &page.rows[index];
            let selected = selection_for_row(selection, doc_id, row, index);
            let draw = Draw {
                surface: Surface::Compact {
                    doc_id,
                    view,
                    selection_color,
                    selection_handler,
                    cx,
                },
                index,
                selected,
            };
            draw.row(row)
        }))
        .child(tracker)
        .into_any_element()
}
