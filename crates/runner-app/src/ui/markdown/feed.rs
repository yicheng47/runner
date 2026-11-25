use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    canvas, div, px, AnyElement, App, Bounds, DispatchPhase, Element, ElementId, EntityId,
    GlobalElementId, Hsla, InspectorElementId, LayoutId, MouseExitEvent, MouseMoveEvent, Pixels,
    SharedString, Window,
};
use runner_terminal::palette::{self, TerminalPalette};

use super::code::CompactCodeStates;
use super::draw::{
    note_compact_exit, note_compact_move, note_compact_press, note_compact_release, Draw, Surface,
};
use super::layout::{selected_range, Row, RowKind};
use super::view::Position;
use super::Page;
use crate::surfaces::files::syntax::SyntaxTheme;

#[cfg(test)]
thread_local! {
    pub(crate) static PARSE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static RENDER_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static LINE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static HIGHLIGHT_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static SYNTAX_THEME_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Callback invoked when a Compact code block's height changes (toggling Wrap
/// or gaining/losing its horizontal scrollbar), used by virtualized hosts to
/// remeasure their list items without retaining global state.
pub(crate) type CompactHeightHandler = Rc<dyn Fn(&mut App)>;

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
        on_height_change: Option<CompactHeightHandler>,
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
            on_height_change.as_ref(),
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
            None,
            cx,
        )
    }
}

thread_local! {
    /// One-entry palette cache so visible highlighted code chunks reuse a
    /// single [`SyntaxTheme`] instead of rebuilding it on every chunk.
    static COMPACT_SYNTAX_THEME: RefCell<Option<(TerminalPalette, Arc<SyntaxTheme>)>> =
        const { RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn clear_compact_syntax_theme_cache() {
    COMPACT_SYNTAX_THEME.with(|cache| *cache.borrow_mut() = None);
    SYNTAX_THEME_BUILDS.set(0);
}

fn compact_syntax_theme(cx: &App) -> Arc<SyntaxTheme> {
    let palette = match cx.try_global::<crate::app_store::GlobalAppStore>() {
        Some(store) => crate::app_settings::terminal_palette(
            &store.0.read(cx).settings,
            crate::theme::active_variant(),
        ),
        None => palette::RUNNER,
    };
    COMPACT_SYNTAX_THEME.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((cached_palette, theme)) = cache.as_ref()
            && *cached_palette == palette
        {
            return Arc::clone(theme);
        }
        #[cfg(test)]
        SYNTAX_THEME_BUILDS.set(SYNTAX_THEME_BUILDS.get() + 1);
        let theme = Arc::new(SyntaxTheme::for_palette(&palette));
        *cache = Some((palette, Arc::clone(&theme)));
        theme
    })
}

/// Element-scoped parsed page for read-only Compact previews. GPUI retains this
/// state across redraws while the preview stays mounted (preserving code block
/// hover, Wrap, Copy and async highlights) and drops it as soon as the preview
/// closes or navigates away.
struct CompactPreviewState {
    text: String,
    /// Stable per-code-block interaction tokens shared across collapsed and
    /// expanded presentations while this preview stays mounted.
    code_tokens: Vec<Arc<()>>,
    preview_len: Option<usize>,
    page: Arc<Page>,
}

struct CompactPreviewElement {
    id: SharedString,
    text: String,
    preview_len: Option<usize>,
    view: EntityId,
}

impl Element for CompactPreviewElement {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(ElementId::Name(format!("compact-md-{}", self.id).into()))
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, AnyElement) {
        let (page, replaced) =
            window.with_element_state::<CompactPreviewState, _>(id.unwrap(), |state, _| {
                let (state, replaced) = match state {
                    Some(prev)
                        if prev.text == self.text && prev.preview_len == self.preview_len =>
                    {
                        (prev, false)
                    }
                    Some(mut prev) if prev.text == self.text => {
                        let page = match self.preview_len {
                            Some(prefix_len) => Page::parse_compact_clamped(
                                &self.text,
                                prefix_len,
                                &mut prev.code_tokens,
                            ),
                            None => {
                                Page::parse_compact_with_tokens(&self.text, &mut prev.code_tokens)
                            }
                        };
                        prev.preview_len = self.preview_len;
                        prev.page = Arc::new(page);
                        (prev, false)
                    }
                    _ => {
                        let mut code_tokens = Vec::new();
                        let page = match self.preview_len {
                            Some(prefix_len) => Page::parse_compact_clamped(
                                &self.text,
                                prefix_len,
                                &mut code_tokens,
                            ),
                            None => Page::parse_compact_with_tokens(&self.text, &mut code_tokens),
                        };
                        (
                            CompactPreviewState {
                                text: self.text.clone(),
                                code_tokens,
                                preview_len: self.preview_len,
                                page: Arc::new(page),
                            },
                            true,
                        )
                    }
                };
                ((Arc::clone(&state.page), replaced), state)
            });
        if replaced && cx.has_global::<CompactCodeStates>() {
            cx.global_mut::<CompactCodeStates>().prune_empty_views();
        }
        let mut child = render_compact_rows(
            &page,
            0..page.rows.len(),
            &self.id,
            self.view,
            None,
            crate::theme::accent(),
            None,
            None,
            cx,
        );
        let layout_id = child.request_layout(window, cx);
        (layout_id, child)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) {
        child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut AnyElement,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        child.paint(window, cx);
    }
}

/// Renders short read-only Markdown in the Compact look (role and crew prompts
/// and previews, Skills preview, Info popover System prompt).
pub(crate) fn render_compact(id: &str, text: &str, view: EntityId, _cx: &App) -> AnyElement {
    render_compact_clamped(id, text, None, view, _cx)
}

/// Renders read-only Markdown in the Compact look, optionally clamping the
/// rendered rows to match the first `preview_len` bytes of `text` while keeping
/// the full parsed [`CodeBlock`]s backing Copy.
pub(crate) fn render_compact_clamped(
    id: &str,
    text: &str,
    preview_len: Option<usize>,
    view: EntityId,
    _cx: &App,
) -> AnyElement {
    CompactPreviewElement {
        id: SharedString::from(id.to_owned()),
        text: text.to_owned(),
        preview_len,
        view,
    }
    .into_any_element()
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
    on_height_change: Option<&CompactHeightHandler>,
    cx: &App,
) -> AnyElement {
    let is_end = range.end == page.rows.len();
    let has_highlighted_code = page.rows[range.clone()]
        .iter()
        .any(|row| matches!(&row.kind, RowKind::Code { code, .. } if code.highlights().is_some()));
    let syntax = has_highlighted_code.then(|| compact_syntax_theme(cx));
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
                    syntax: syntax.as_deref(),
                    selection_color,
                    selection_handler,
                    on_height_change,
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
