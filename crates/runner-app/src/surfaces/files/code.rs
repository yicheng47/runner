//! The read-only code view: a file's lines with their numbers, drawn by one
//! element that shapes only the cells in view, scrolls both ways without
//! wrapping, and keeps its selection in the text so it survives scrolling.
//! Characters sit on the document's grid of monospace cells, so where a
//! character is drawn, hit and highlighted is one column times the advance.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    div, fill, point, px, relative, size, App, Bounds, ClipboardItem, ContentMask, Context,
    CursorStyle, DispatchPhase, Element, Entity, FocusHandle, Font, GlobalElementId, Hitbox,
    HitboxBehavior, Hsla, InspectorElementId, KeyDownEvent, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollWheelEvent, ShapedLine, SharedString, Style,
    TextAlign, TextRun, Window,
};

use super::document::{Document, FileMatch, Position};
use super::highlight::{self, Grammar, Highlights};
use super::syntax::SyntaxTheme;
use crate::theme;
use crate::{Copy, SelectAll};

const GUTTER_PAD_LEFT: f32 = 12.;
const GUTTER_PAD_RIGHT: f32 = 16.;
const TEXT_PAD_RIGHT: f32 = 24.;
const TOP_PAD: f32 = 8.;
/// How long the scrollbars stay after the last scroll.
const SCROLLBAR_SHOWN: Duration = Duration::from_millis(1200);

/// ⌘-click (Ctrl-click) on a line: opens the file at that line, 1-based.
pub(crate) type OpenLine = Rc<dyn Fn(u32, &mut Window, &mut App)>;

#[derive(Clone, Copy, Debug)]
struct Metrics {
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    char_width: Pixels,
    /// From the element's left edge to the text's.
    text_left: Pixels,
}

impl Metrics {
    fn text_width(&self) -> Pixels {
        (self.bounds.size.width - self.text_left).max(px(0.))
    }
}

pub(crate) struct CodeView {
    document: Arc<Document>,
    /// The language the file is highlighted as; `None` stays plain.
    grammar: Option<&'static Grammar>,
    /// The spans of `document`, or of the text before a reload until the
    /// new ones arrive.
    highlights: Option<Arc<Highlights>>,
    /// Set to stop the highlighting under way, when the text it reads is
    /// replaced or the view goes.
    cancel: Arc<AtomicUsize>,
    focus: FocusHandle,
    scroll: Point<Pixels>,
    /// The anchor and the head, which moves.
    selection: Option<(Position, Position)>,
    dragging: bool,
    matches: Arc<Vec<FileMatch>>,
    current: Option<usize>,
    /// The file was deleted: the text is dimmed and ⌘-click opens nothing.
    deleted: bool,
    metrics: Cell<Option<Metrics>>,
    reveal: Option<Position>,
    scrolled_at: Option<Instant>,
    open_line: Option<OpenLine>,
    /// The bytes shaped in the last frame.
    #[cfg(test)]
    pub(crate) shaped_bytes: Cell<usize>,
    /// The runs drawn in the last frame, by line: each run's text and colour.
    #[cfg(test)]
    pub(crate) painted: std::cell::RefCell<std::collections::BTreeMap<usize, Vec<(String, Hsla)>>>,
}

impl CodeView {
    pub(crate) fn new(
        document: Arc<Document>,
        grammar: Option<&'static Grammar>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self {
            document,
            grammar,
            highlights: None,
            cancel: Arc::default(),
            focus: cx.focus_handle(),
            scroll: Point::default(),
            selection: None,
            dragging: false,
            matches: Arc::default(),
            current: None,
            deleted: false,
            metrics: Cell::new(None),
            reveal: None,
            scrolled_at: None,
            open_line: None,
            #[cfg(test)]
            shaped_bytes: Cell::new(0),
            #[cfg(test)]
            painted: Default::default(),
        };
        view.highlight(cx);
        view
    }

    /// Highlights the text on the background executor; the spans arrive
    /// for the text they were computed from, or not at all.
    fn highlight(&mut self, cx: &mut Context<Self>) {
        self.cancel.store(1, Ordering::Relaxed);
        let Some(grammar) = self.grammar else {
            return;
        };
        let cancel = Arc::new(AtomicUsize::new(0));
        self.cancel = cancel.clone();
        let document = self.document.clone();
        let task = cx.background_spawn(async move {
            highlight::highlight(&document, grammar, Some(&cancel)).map(|found| (document, found))
        });
        cx.spawn(async move |this, cx| {
            let Some((document, found)) = task.await else {
                return;
            };
            let _ = this.update(cx, |view, cx| {
                if Arc::ptr_eq(&view.document, &document) {
                    view.highlights = Some(Arc::new(found));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    #[cfg(test)]
    pub(crate) fn highlights(&self) -> Option<&Arc<Highlights>> {
        self.highlights.as_ref()
    }

    pub(crate) fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    pub(crate) fn document(&self) -> &Arc<Document> {
        &self.document
    }

    /// The file read again: the scroll position stays, the selection only
    /// while it still lands in the text, and find's matches, which name
    /// places in the old text, go until find searches the new one.
    pub(crate) fn set_document(&mut self, document: Arc<Document>, cx: &mut Context<Self>) {
        self.document = document;
        self.highlight(cx);
        self.selection = self
            .selection
            .map(|(anchor, head)| (self.document.clamp(anchor), self.document.clamp(head)));
        self.matches = Arc::default();
        self.current = None;
        self.reveal = None;
        cx.notify();
    }

    pub(crate) fn set_open_line(&mut self, open_line: OpenLine) {
        self.open_line = Some(open_line);
    }

    pub(crate) fn set_deleted(&mut self, deleted: bool, cx: &mut Context<Self>) {
        if self.deleted != deleted {
            self.deleted = deleted;
            cx.notify();
        }
    }

    pub(crate) fn set_find(
        &mut self,
        matches: Arc<Vec<FileMatch>>,
        current: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        if let Some(found) = current.and_then(|index| matches.get(index)) {
            self.reveal = Some(Position {
                line: found.line,
                offset: found.range.start,
            });
        }
        self.matches = matches;
        self.current = current;
        cx.notify();
    }

    /// The first line in view, 1-based, for opening the file there.
    pub(crate) fn line_in_view(&self) -> u32 {
        let line = match self.metrics.get() {
            Some(metrics) => (f32::from(self.scroll.y) / f32::from(metrics.line_height)) as usize,
            None => 0,
        };
        line as u32 + 1
    }

    #[cfg(test)]
    pub(crate) fn scroll_offset(&self) -> Point<Pixels> {
        self.scroll
    }

    pub(crate) fn selected_text(&self) -> Option<String> {
        let (anchor, head) = self.selection?;
        (anchor != head).then(|| self.document.text_between(anchor, head))
    }

    /// The advance, the text's left edge and the element's bounds.
    #[cfg(test)]
    pub(crate) fn geometry(&self) -> (Pixels, Pixels, Bounds<Pixels>) {
        let metrics = self.metrics.get().expect("drawn");
        (metrics.char_width, metrics.text_left, metrics.bounds)
    }

    /// The middle of `line`'s row on screen.
    #[cfg(test)]
    pub(crate) fn row_y(&self, line: usize) -> Pixels {
        let metrics = self.metrics.get().expect("drawn");
        metrics.bounds.top() + px(TOP_PAD) - self.scroll.y
            + metrics.line_height * (line as f32 + 0.5)
    }

    #[cfg(test)]
    pub(crate) fn find_highlights(&self) -> (usize, Option<usize>) {
        (self.matches.len(), self.current)
    }

    #[cfg(test)]
    pub(crate) fn scroll_to(&mut self, offset: Point<Pixels>, cx: &mut Context<Self>) {
        self.scroll = offset;
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selection = Some((Position::default(), self.document.end()));
        cx.notify();
    }

    fn content_size(&self, metrics: &Metrics) -> (Pixels, Pixels) {
        let width = metrics.char_width * self.document.columns() as f32 + px(TEXT_PAD_RIGHT);
        let height = metrics.line_height * self.document.line_count() as f32 + px(TOP_PAD * 2.);
        (width, height)
    }

    fn clamp_scroll(&mut self, metrics: &Metrics) {
        let (width, height) = self.content_size(metrics);
        let max_x = (width - metrics.text_width()).max(px(0.));
        let max_y = (height - metrics.bounds.size.height).max(px(0.));
        self.scroll.x = self.scroll.x.clamp(px(0.), max_x);
        self.scroll.y = self.scroll.y.clamp(px(0.), max_y);
    }

    /// Scrolls the least that brings `position` into view.
    fn apply_reveal(&mut self, metrics: &Metrics) {
        let Some(position) = self.reveal.take() else {
            return;
        };
        let top = metrics.line_height * position.line as f32 + px(TOP_PAD);
        let height = metrics.bounds.size.height;
        if top < self.scroll.y || top + metrics.line_height > self.scroll.y + height {
            self.scroll.y = top - (height - metrics.line_height) / 2.;
        }
        let column = self.document.column_at(position.line, position.offset) as f32;
        let x = metrics.char_width * column;
        let width = metrics.text_width();
        if x < self.scroll.x || x + metrics.char_width * 4. > self.scroll.x + width {
            self.scroll.x = (x - width / 3.).max(px(0.));
        }
    }

    fn scroll_by(&mut self, delta: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(metrics) = self.metrics.get() else {
            return;
        };
        let before = self.scroll;
        self.scroll.x -= delta.x;
        self.scroll.y -= delta.y;
        self.clamp_scroll(&metrics);
        if self.scroll != before {
            self.scrolled_at = Some(Instant::now());
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(SCROLLBAR_SHOWN).await;
                let _ = this.update(cx, |_, cx| cx.notify());
            })
            .detach();
            cx.notify();
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(metrics) = self.metrics.get() else {
            return;
        };
        let modifiers = event.keystroke.modifiers;
        if modifiers.platform || modifiers.control || modifiers.alt {
            return;
        }
        let page = metrics.bounds.size.height - metrics.line_height;
        let delta = match event.keystroke.key.as_str() {
            "up" => point(px(0.), metrics.line_height),
            "down" => point(px(0.), -metrics.line_height),
            "left" => point(metrics.char_width * 4., px(0.)),
            "right" => point(-metrics.char_width * 4., px(0.)),
            "pageup" => point(px(0.), page),
            "pagedown" => point(px(0.), -page),
            "home" => point(px(0.), self.scroll.y),
            "end" => point(px(0.), self.scroll.y - px(f32::MAX / 4.)),
            _ => return,
        };
        cx.stop_propagation();
        self.scroll_by(delta, cx);
    }

    fn font(cx: &App) -> (Font, Pixels) {
        match cx.try_global::<crate::app_store::GlobalAppStore>() {
            Some(store) => {
                let settings = &store.0.read(cx).settings;
                (
                    settings.terminal_font_family.font(cx),
                    px(settings.terminal_font_size as f32),
                )
            }
            None => (gpui::font(theme::UI_MONOSPACE_FONT), px(13.)),
        }
    }
}

impl Drop for CodeView {
    fn drop(&mut self) {
        self.cancel.store(1, Ordering::Relaxed);
    }
}

impl Render for CodeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("file-code")
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILE_CODE".into())
            })
            .track_focus(&self.focus)
            .size_full()
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::select_all))
            .on_key_down(cx.listener(Self::on_key))
            .child(CodeElement { view: cx.entity() })
    }
}

/// One line's cells in view, as shaped runs each drawn from its column.
struct Row {
    line: usize,
    segments: Vec<(usize, ShapedLine)>,
    /// Each run's text and colour, for tests.
    #[cfg(test)]
    painted: Vec<(String, Hsla)>,
}

/// How a line is set: its font, size and colour, the monospace advance,
/// and the colours of its highlights when it is highlighted.
struct RowStyle<'a> {
    font: &'a Font,
    font_size: Pixels,
    char_width: Pixels,
    color: Hsla,
    syntax: Option<(&'a Highlights, &'a SyntaxTheme)>,
}

/// Shapes the cells of `line` from column `from` to `to`, each run held to
/// its cells so it lands on the grid.
fn shape_row(
    document: &Document,
    line: usize,
    (from, to): (usize, usize),
    style: &RowStyle,
    window: &Window,
) -> Row {
    let spans = style
        .syntax
        .map_or(&[][..], |(highlights, _)| highlights.line(line));
    #[cfg(test)]
    let mut painted = Vec::new();
    let segments = document
        .segments(line, from, to, spans)
        .into_iter()
        .map(|segment| {
            let runs = segment
                .runs
                .iter()
                .map(|&(len, capture)| TextRun {
                    len,
                    font: style.font.clone(),
                    color: capture
                        .zip(style.syntax)
                        .and_then(|(capture, (_, theme))| theme.color(capture))
                        .unwrap_or(style.color),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                })
                .collect::<Vec<_>>();
            #[cfg(test)]
            {
                let mut at = 0;
                for run in &runs {
                    painted.push((segment.text[at..at + run.len].to_owned(), run.color));
                    at += run.len;
                }
            }
            let text: SharedString = segment.text.into();
            let shaped = window.text_system().shape_line(
                text,
                style.font_size,
                &runs,
                Some(style.char_width * segment.cells as f32),
            );
            (segment.column, shaped)
        })
        .collect();
    Row {
        line,
        segments,
        #[cfg(test)]
        painted,
    }
}

struct CodeElement {
    view: Entity<CodeView>,
}

pub(crate) struct CodePrepaint {
    hitbox: Hitbox,
    metrics: Metrics,
    scroll: Point<Pixels>,
    rows: Vec<Row>,
    numbers: Vec<(Pixels, ShapedLine)>,
    matches: Vec<Bounds<Pixels>>,
    current: Option<Bounds<Pixels>>,
    selections: Vec<Bounds<Pixels>>,
    scrollbars: Vec<Bounds<Pixels>>,
}

impl IntoElement for CodeElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for CodeElement {
    type RequestLayoutState = ();
    type PrepaintState = CodePrepaint;

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
        style.size.height = relative(1.).into();
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
    ) -> CodePrepaint {
        let (font, font_size) = CodeView::font(cx);
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&font);
        let char_width = text_system
            .em_advance(font_id, font_size)
            .unwrap_or(font_size * 0.6);
        let line_height = (font_size * crate::terminal::element::LINE_HEIGHT_FACTOR).round();
        let digits = self
            .view
            .read(cx)
            .document
            .line_count()
            .max(1)
            .to_string()
            .len();
        let text_left = px(GUTTER_PAD_LEFT) + char_width * digits as f32 + px(GUTTER_PAD_RIGHT);
        let metrics = Metrics {
            bounds,
            line_height,
            char_width,
            text_left,
        };
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let text_color = if self.view.read(cx).deleted {
            theme::faint()
        } else {
            theme::text()
        };
        self.view.update(cx, |view, _| {
            view.metrics.set(Some(metrics));
            view.apply_reveal(&metrics);
            view.clamp_scroll(&metrics);
        });
        let view = self.view.read(cx);
        let document = view.document.clone();
        let scroll = view.scroll;
        let top = bounds.top() + px(TOP_PAD) - scroll.y;
        let first = (f32::from(scroll.y - px(TOP_PAD)) / f32::from(line_height))
            .floor()
            .max(0.) as usize;
        let shown_lines =
            (f32::from(bounds.size.height) / f32::from(line_height)).ceil() as usize + 1;
        let last = (first + shown_lines).min(document.line_count());
        let text_x = bounds.left() + text_left - scroll.x;
        let visible = metrics.text_width();
        let from = (f32::from(scroll.x) / f32::from(char_width)).floor() as usize;
        let to = from + (f32::from(visible) / f32::from(char_width)).ceil() as usize + 2;
        let mut rows = Vec::with_capacity(last.saturating_sub(first));
        let mut numbers = Vec::with_capacity(rows.capacity());
        let number_color = theme::faint();
        let highlights = view.highlights.clone().filter(|_| !view.deleted);
        let syntax = highlights.as_ref().map(|_| SyntaxTheme::current(cx));
        let style = RowStyle {
            font: &font,
            font_size,
            char_width,
            color: text_color,
            syntax: highlights.as_deref().zip(syntax.as_ref()),
        };
        for line in first..last {
            #[cfg(test)]
            {
                crate::render_counts::count_render("file-code-lines", cx);
                if style.syntax.is_some() {
                    crate::render_counts::count_render("file-code-styled-lines", cx);
                }
            }
            rows.push(shape_row(&document, line, (from, to), &style, window));
            let number: SharedString = (line + 1).to_string().into();
            let shaped = window.text_system().shape_line(
                number.clone(),
                font_size,
                &[TextRun {
                    len: number.len(),
                    font: font.clone(),
                    color: number_color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            );
            let x = bounds.left() + text_left - px(GUTTER_PAD_RIGHT) - shaped.width;
            numbers.push((x, shaped));
        }
        #[cfg(test)]
        view.painted.replace(
            rows.iter()
                .map(|row| (row.line, row.painted.clone()))
                .collect(),
        );
        #[cfg(test)]
        view.shaped_bytes.set(
            rows.iter()
                .flat_map(|row| &row.segments)
                .map(|(_, shaped)| shaped.text.len())
                .sum(),
        );
        let row_top = |line: usize| top + line_height * line as f32;
        let span = |row: &Row, start: usize, end: usize| {
            let left = text_x + char_width * document.column_at(row.line, start) as f32;
            let right = text_x + char_width * document.column_at(row.line, end) as f32;
            Bounds::new(
                point(left, row_top(row.line)),
                size((right - left).max(px(0.)), line_height),
            )
        };
        let mut matches = Vec::new();
        let mut current = None;
        let start_match = view.matches.partition_point(|found| found.line < first);
        for (index, found) in view.matches[start_match..].iter().enumerate() {
            if found.line >= last {
                break;
            }
            let row = &rows[found.line - first];
            let bounds = span(row, found.range.start, found.range.end);
            if view.current == Some(start_match + index) {
                current = Some(bounds);
            } else {
                matches.push(bounds);
            }
        }
        let mut selections = Vec::new();
        if let Some((anchor, head)) = view.selection {
            let (start, end) = (anchor.min(head), anchor.max(head));
            for row in rows
                .iter()
                .filter(|row| row.line >= start.line && row.line <= end.line)
            {
                let line = document.line(row.line);
                let from = if row.line == start.line {
                    start.offset
                } else {
                    0
                };
                let to = if row.line == end.line {
                    end.offset
                } else {
                    line.len()
                };
                let mut bounds = span(row, from, to);
                if row.line != end.line {
                    bounds.size.width += char_width / 2.;
                }
                selections.push(bounds);
            }
        }
        let mut scrollbars = Vec::new();
        if view
            .scrolled_at
            .is_some_and(|at| at.elapsed() < SCROLLBAR_SHOWN)
        {
            let (width, height) = view.content_size(&metrics);
            let thickness = px(4.);
            if height > bounds.size.height {
                let ratio = bounds.size.height / height;
                let length = (bounds.size.height * ratio).max(px(24.));
                let y = (bounds.size.height - length) * (scroll.y / (height - bounds.size.height));
                scrollbars.push(Bounds::new(
                    point(bounds.right() - thickness - px(2.), bounds.top() + y),
                    size(thickness, length),
                ));
            }
            if width > visible {
                let ratio = visible / width;
                let length = (visible * ratio).max(px(24.));
                let x = (visible - length) * (scroll.x / (width - visible));
                scrollbars.push(Bounds::new(
                    point(
                        bounds.left() + text_left + x,
                        bounds.bottom() - thickness - px(2.),
                    ),
                    size(length, thickness),
                ));
            }
        }
        CodePrepaint {
            hitbox,
            metrics,
            scroll,
            rows,
            numbers,
            matches,
            current,
            selections,
            scrollbars,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut (),
        prepaint: &mut CodePrepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let metrics = prepaint.metrics;
        let text_bounds = Bounds::new(
            point(bounds.left() + metrics.text_left, bounds.top()),
            size(metrics.text_width(), bounds.size.height),
        );
        window.set_cursor_style(CursorStyle::IBeam, &prepaint.hitbox);
        let highlight = theme::find_highlight();
        let top = bounds.top() + px(TOP_PAD) - prepaint.scroll.y;
        window.with_content_mask(
            Some(ContentMask {
                bounds: text_bounds,
            }),
            |window| {
                for bounds in &prepaint.matches {
                    window.paint_quad(fill(*bounds, highlight.tint).corner_radii(px(2.)));
                }
                if let Some(bounds) = prepaint.current {
                    window.paint_quad(fill(bounds, highlight.current).corner_radii(px(2.)));
                }
                let selection = theme::with_alpha(theme::accent(), 0.28);
                for bounds in &prepaint.selections {
                    window.paint_quad(fill(*bounds, selection));
                }
                let text_x = bounds.left() + metrics.text_left - prepaint.scroll.x;
                for row in &prepaint.rows {
                    let y = top + metrics.line_height * row.line as f32;
                    for (column, shaped) in &row.segments {
                        let origin = point(text_x + metrics.char_width * *column as f32, y);
                        let _ = shaped.paint(
                            origin,
                            metrics.line_height,
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    }
                }
            },
        );
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for ((x, number), row) in prepaint.numbers.iter().zip(&prepaint.rows) {
                let origin = point(*x, top + metrics.line_height * row.line as f32);
                let _ = number.paint(
                    origin,
                    metrics.line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
            let thumb = theme::with_alpha(theme::muted(), 0.5);
            for bar in &prepaint.scrollbars {
                window.paint_quad(fill(*bar, thumb).corner_radii(px(2.)));
            }
        });
        register_mouse(self.view.clone(), prepaint.hitbox.clone(), metrics, window);
    }
}

fn word_at(line: &str, offset: usize) -> (usize, usize) {
    let is_word = |character: char| character.is_alphanumeric() || character == '_';
    let start = line[..offset.min(line.len())]
        .char_indices()
        .rev()
        .take_while(|(_, character)| is_word(*character))
        .last()
        .map_or(offset, |(index, _)| index);
    let end = line[offset.min(line.len())..]
        .char_indices()
        .find(|(_, character)| !is_word(*character))
        .map_or(line.len(), |(index, _)| offset + index);
    (start, end)
}

fn register_mouse(view: Entity<CodeView>, hitbox: Hitbox, metrics: Metrics, window: &mut Window) {
    let scroll_view = view.clone();
    let scroll_hitbox = hitbox.clone();
    window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || !scroll_hitbox.should_handle_scroll(window) {
            return;
        }
        let mut delta = event.delta.pixel_delta(metrics.line_height);
        if event.modifiers.shift && delta.x == px(0.) {
            delta = point(delta.y, px(0.));
        }
        scroll_view.update(cx, |view, cx| view.scroll_by(delta, cx));
        cx.stop_propagation();
    });

    let down_view = view.clone();
    let down_hitbox = hitbox.clone();
    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble
            || event.button != MouseButton::Left
            || !down_hitbox.is_hovered(window)
        {
            return;
        }
        down_view.update(cx, |view, cx| {
            view.focus.focus(window, cx);
            let at = position_at(view, &metrics, event.position);
            let open = if cfg!(target_os = "macos") {
                event.modifiers.platform
            } else {
                event.modifiers.control
            };
            if open {
                if let Some(open_line) = view.open_line.clone().filter(|_| !view.deleted) {
                    let line = at.line as u32 + 1;
                    cx.defer_in(window, move |_, window, cx| open_line(line, window, cx));
                }
                return;
            }
            match event.click_count {
                2 => {
                    let (start, end) = word_at(view.document.line(at.line), at.offset);
                    view.selection = Some((
                        Position {
                            line: at.line,
                            offset: start,
                        },
                        Position {
                            line: at.line,
                            offset: end,
                        },
                    ));
                }
                count if count >= 3 => {
                    view.selection = Some((
                        Position {
                            line: at.line,
                            offset: 0,
                        },
                        Position {
                            line: at.line,
                            offset: view.document.line(at.line).len(),
                        },
                    ));
                }
                _ => {
                    let anchor = match view.selection {
                        Some((anchor, _)) if event.modifiers.shift => anchor,
                        _ => at,
                    };
                    view.selection = Some((anchor, at));
                    view.dragging = true;
                }
            }
            cx.notify();
        });
    });

    let move_view = view.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
        if phase != DispatchPhase::Capture
            || event.pressed_button != Some(MouseButton::Left)
            || !move_view.read(cx).dragging
        {
            return;
        }
        move_view.update(cx, |view, cx| {
            // Dragging past an edge scrolls toward it.
            let bounds = metrics.bounds;
            let mut delta = Point::default();
            if event.position.y < bounds.top() {
                delta.y = bounds.top() - event.position.y;
            } else if event.position.y > bounds.bottom() {
                delta.y = bounds.bottom() - event.position.y;
            }
            let text_left = bounds.left() + metrics.text_left;
            if event.position.x < text_left {
                delta.x = text_left - event.position.x;
            } else if event.position.x > bounds.right() {
                delta.x = bounds.right() - event.position.x;
            }
            if delta != Point::default() {
                view.scroll_by(delta, cx);
            }
            let head = position_at(view, &metrics, event.position);
            if let Some((anchor, _)) = view.selection {
                view.selection = Some((anchor, head));
            }
            cx.notify();
        });
    });

    let up_view = view;
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
        if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
            return;
        }
        up_view.update(cx, |view, _| view.dragging = false);
    });
}

/// The text position under `at`, on or off screen.
fn position_at(view: &CodeView, metrics: &Metrics, at: Point<Pixels>) -> Position {
    let document = &view.document;
    let y = at.y - metrics.bounds.top() - px(TOP_PAD) + view.scroll.y;
    let line = (f32::from(y) / f32::from(metrics.line_height)).floor();
    if line < 0. {
        return Position::default();
    }
    let line = line as usize;
    if line >= document.line_count() {
        return document.end();
    }
    let x = at.x - metrics.bounds.left() - metrics.text_left + view.scroll.x;
    let column = f32::from(x.max(px(0.))) / f32::from(metrics.char_width);
    Position {
        line,
        offset: document.offset_near(line, column),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_double_click_selects_the_word_under_it() {
        assert_eq!(word_at("let stream = x;", 6), (4, 10));
        assert_eq!(word_at("let stream = x;", 4), (4, 10));
        assert_eq!(word_at("a b", 1), (0, 1));
        assert_eq!(word_at("a  b", 2), (2, 2));
        assert_eq!(word_at("", 0), (0, 0));
    }
}
