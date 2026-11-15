//! Virtualized Unified diff view with two-version background highlighting,
//! gutter-free text selection/copy, hunk navigation, and end-of-diff viewed box.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, fill, point, px, relative, size, svg, App, Bounds, ClipboardItem, ContentMask, Context,
    CursorStyle, DispatchPhase, Element, Entity, FocusHandle, Font, GlobalElementId, Hitbox,
    HitboxBehavior, Hsla, InspectorElementId, KeyDownEvent, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollWheelEvent, ShapedLine, SharedString, Style,
    TextAlign, TextRun, Window,
};
use runner_app::ui::{app_scrollbar_gutter, Scrollbar, ScrollbarKind, ScrollbarMetrics};
use runner_core::protocol::changes::{DiffHunk, DiffOp};

use crate::surfaces::files::code::{word_at, CodeView};
use crate::surfaces::files::document::Document;
use crate::surfaces::files::highlight::{self, Grammar, Highlights};
use crate::surfaces::files::syntax::SyntaxTheme;
use crate::theme;
use crate::{Copy, SelectAll};

const GUTTER_PAD_LEFT: f32 = 8.;
const COL_GAP: f32 = 8.;
const SIGN_WIDTH: f32 = 10.;
const TEXT_GAP: f32 = 8.;
const TEXT_PAD_RIGHT: f32 = 24.;
const TOP_PAD: f32 = 6.;
const FOOTER_HEIGHT: f32 = 40.;

pub(crate) type DiffCallback = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct DiffPos {
    pub(crate) row: usize,
    pub(crate) offset: usize,
}

#[derive(Clone, Debug)]
pub(crate) enum DiffRow {
    Hunk { header: SharedString },
    Context { old_line: usize, new_line: usize },
    Removed { old_line: usize },
    Added { new_line: usize },
}

#[derive(Clone, Copy, Debug)]
struct Metrics {
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    char_width: Pixels,
    old_num_right: Pixels,
    new_num_right: Pixels,
    sign_left: Pixels,
    text_left: Pixels,
}

impl Metrics {
    fn text_width(&self) -> Pixels {
        (self.bounds.size.width - self.text_left).max(px(0.))
    }
}

pub(crate) struct UnifiedDiffView {
    old_doc: Arc<Document>,
    new_doc: Arc<Document>,
    rows: Arc<Vec<DiffRow>>,
    hunk_rows: Vec<usize>,
    max_columns: usize,
    old_digits: usize,
    new_digits: usize,
    grammar: Option<&'static Grammar>,
    old_highlights: Option<Arc<Highlights>>,
    new_highlights: Option<Arc<Highlights>>,
    cancel: Arc<AtomicUsize>,
    focus: FocusHandle,
    scroll: Point<Pixels>,
    cursor_row: usize,
    reveal_row: Option<usize>,
    selection: Option<(DiffPos, DiffPos)>,
    selects_all: bool,
    dragging: bool,
    file_name: SharedString,
    viewed: bool,
    changed_since_viewed: bool,
    metrics: Cell<Option<Metrics>>,
    bar_metrics: Rc<Cell<(ScrollbarMetrics, ScrollbarMetrics)>>,
    vertical_bar: Entity<Scrollbar>,
    horizontal_bar: Entity<Scrollbar>,
    on_escape: Option<DiffCallback>,
    on_toggle_viewed: Option<DiffCallback>,
    on_focus_in: Option<DiffCallback>,
    #[cfg(test)]
    pub(crate) painted: std::cell::RefCell<std::collections::BTreeMap<usize, Vec<(String, Hsla)>>>,
}

fn build_rows(
    old_doc: &Document,
    new_doc: &Document,
    hunks: &[DiffHunk],
) -> (Vec<DiffRow>, Vec<usize>, usize, usize, usize) {
    let mut rows = Vec::new();
    let mut hunk_rows = Vec::with_capacity(hunks.len());
    let mut max_columns = 0usize;
    let mut max_old_line = 1usize;
    let mut max_new_line = 1usize;

    let old_count = old_doc.line_count();
    let new_count = new_doc.line_count();

    for hunk in hunks {
        hunk_rows.push(rows.len());
        max_columns = max_columns.max(hunk.header.chars().count());
        rows.push(DiffRow::Hunk {
            header: hunk.header.clone().into(),
        });
        let mut old_idx = hunk.old_start.saturating_sub(1) as usize;
        let mut new_idx = hunk.new_start.saturating_sub(1) as usize;
        for op in &hunk.ops {
            match *op {
                DiffOp::Context { count } => {
                    for _ in 0..count {
                        let o = old_idx.min(old_count.saturating_sub(1));
                        let n = new_idx.min(new_count.saturating_sub(1));
                        max_old_line = max_old_line.max(o + 1);
                        max_new_line = max_new_line.max(n + 1);
                        rows.push(DiffRow::Context {
                            old_line: o,
                            new_line: n,
                        });
                        old_idx += 1;
                        new_idx += 1;
                    }
                }
                DiffOp::Removed { count } => {
                    for _ in 0..count {
                        let o = old_idx.min(old_count.saturating_sub(1));
                        max_old_line = max_old_line.max(o + 1);
                        rows.push(DiffRow::Removed { old_line: o });
                        old_idx += 1;
                    }
                }
                DiffOp::Added { count } => {
                    for _ in 0..count {
                        let n = new_idx.min(new_count.saturating_sub(1));
                        max_new_line = max_new_line.max(n + 1);
                        rows.push(DiffRow::Added { new_line: n });
                        new_idx += 1;
                    }
                }
            }
        }
    }
    max_columns = max_columns.max(old_doc.columns()).max(new_doc.columns());
    let old_digits = max_old_line.to_string().len().max(2);
    let new_digits = max_new_line.to_string().len().max(2);
    (rows, hunk_rows, max_columns, old_digits, new_digits)
}

impl UnifiedDiffView {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        old_doc: Arc<Document>,
        new_doc: Arc<Document>,
        hunks: &[DiffHunk],
        grammar: Option<&'static Grammar>,
        file_name: SharedString,
        viewed: bool,
        changed_since_viewed: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let (rows, hunk_rows, max_columns, old_digits, new_digits) =
            build_rows(&old_doc, &new_doc, hunks);
        let initial_cursor = hunk_rows
            .first()
            .map(|&h| (h + 1).min(rows.len().saturating_sub(1)))
            .unwrap_or(0);
        let bar_metrics = Rc::new(Cell::new(Default::default()));
        let vertical_bar = diff_scrollbar(&bar_metrics, false, cx);
        let horizontal_bar = diff_scrollbar(&bar_metrics, true, cx);
        let mut view = Self {
            old_doc,
            new_doc,
            rows: Arc::new(rows),
            hunk_rows,
            max_columns,
            old_digits,
            new_digits,
            grammar,
            old_highlights: None,
            new_highlights: None,
            cancel: Arc::default(),
            focus: cx.focus_handle(),
            scroll: Point::default(),
            cursor_row: initial_cursor,
            reveal_row: None,
            selection: None,
            selects_all: false,
            dragging: false,
            file_name,
            viewed,
            changed_since_viewed,
            metrics: Cell::new(None),
            bar_metrics,
            vertical_bar,
            horizontal_bar,
            on_escape: None,
            on_toggle_viewed: None,
            on_focus_in: None,
            #[cfg(test)]
            painted: Default::default(),
        };
        view.highlight(cx);
        view
    }

    pub(crate) fn set_callbacks(
        &mut self,
        on_escape: DiffCallback,
        on_toggle_viewed: DiffCallback,
        on_focus_in: DiffCallback,
    ) {
        self.on_escape = Some(on_escape);
        self.on_toggle_viewed = Some(on_toggle_viewed);
        self.on_focus_in = Some(on_focus_in);
    }

    pub(crate) fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    pub(crate) fn set_viewed_state(
        &mut self,
        viewed: bool,
        changed_since_viewed: bool,
        cx: &mut Context<Self>,
    ) {
        if self.viewed != viewed || self.changed_since_viewed != changed_since_viewed {
            self.viewed = viewed;
            self.changed_since_viewed = changed_since_viewed;
            cx.notify();
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update_content(
        &mut self,
        old_doc: Arc<Document>,
        new_doc: Arc<Document>,
        hunks: &[DiffHunk],
        grammar: Option<&'static Grammar>,
        file_name: SharedString,
        viewed: bool,
        changed_since_viewed: bool,
        cx: &mut Context<Self>,
    ) {
        let (rows, hunk_rows, max_columns, old_digits, new_digits) =
            build_rows(&old_doc, &new_doc, hunks);
        self.old_doc = old_doc;
        self.new_doc = new_doc;
        self.rows = Arc::new(rows);
        self.hunk_rows = hunk_rows;
        self.max_columns = max_columns;
        self.old_digits = old_digits;
        self.new_digits = new_digits;
        self.grammar = grammar;
        self.file_name = file_name;
        self.viewed = viewed;
        self.changed_since_viewed = changed_since_viewed;
        self.cursor_row = self.cursor_row.min(self.rows.len().saturating_sub(1));
        self.selection = self
            .selection
            .map(|(a, b)| (self.clamp_pos(a), self.clamp_pos(b)));
        self.old_highlights = None;
        self.new_highlights = None;
        self.highlight(cx);
        cx.notify();
    }

    fn highlight(&mut self, cx: &mut Context<Self>) {
        self.cancel.store(1, Ordering::Relaxed);
        let Some(grammar) = self.grammar else {
            self.old_highlights = None;
            self.new_highlights = None;
            return;
        };
        let cancel = Arc::new(AtomicUsize::new(0));
        self.cancel = cancel.clone();
        let old_doc = self.old_doc.clone();
        let new_doc = self.new_doc.clone();
        let task = cx.background_spawn(async move {
            let old_hl = highlight::highlight(&old_doc, grammar, Some(&cancel)).map(Arc::new);
            if cancel.load(Ordering::Relaxed) != 0 {
                return None;
            }
            let new_hl = highlight::highlight(&new_doc, grammar, Some(&cancel)).map(Arc::new);
            if cancel.load(Ordering::Relaxed) != 0 {
                return None;
            }
            Some((old_doc, new_doc, old_hl, new_hl))
        });
        cx.spawn(async move |this, cx| {
            let Some((old_doc, new_doc, old_hl, new_hl)) = task.await else {
                return;
            };
            let _ = this.update(cx, |view, cx| {
                if Arc::ptr_eq(&view.old_doc, &old_doc) && Arc::ptr_eq(&view.new_doc, &new_doc) {
                    view.old_highlights = old_hl;
                    view.new_highlights = new_hl;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn row_line_text(&self, row: usize) -> Option<&str> {
        match self.rows.get(row)? {
            DiffRow::Hunk { .. } => None,
            DiffRow::Context { new_line, .. } | DiffRow::Added { new_line } => {
                Some(self.new_doc.line(*new_line))
            }
            DiffRow::Removed { old_line } => Some(self.old_doc.line(*old_line)),
        }
    }

    fn row_doc_and_line(&self, row: usize) -> Option<(&Document, usize)> {
        match self.rows.get(row)? {
            DiffRow::Hunk { .. } => None,
            DiffRow::Context { new_line, .. } | DiffRow::Added { new_line } => {
                Some((&self.new_doc, *new_line))
            }
            DiffRow::Removed { old_line } => Some((&self.old_doc, *old_line)),
        }
    }

    fn clamp_pos(&self, pos: DiffPos) -> DiffPos {
        if self.rows.is_empty() {
            return DiffPos::default();
        }
        let row = pos.row.min(self.rows.len() - 1);
        let max_offset = self.row_line_text(row).map_or(0, str::len);
        let mut offset = pos.offset.min(max_offset);
        if let Some(line) = self.row_line_text(row) {
            while offset > 0 && !line.is_char_boundary(offset) {
                offset -= 1;
            }
        }
        DiffPos { row, offset }
    }

    pub(crate) fn selected_text(&self) -> Option<String> {
        if self.rows.is_empty() {
            return None;
        }
        if self.selects_all {
            let lines: Vec<&str> = (0..self.rows.len())
                .filter_map(|r| self.row_line_text(r))
                .collect();
            return (!lines.is_empty()).then(|| lines.join("\n"));
        }
        let (anchor, head) = self.selection?;
        if anchor == head {
            return None;
        }
        let (start, end) = (anchor.min(head), anchor.max(head));
        let mut pieces = Vec::new();
        for r in start.row..=end.row.min(self.rows.len().saturating_sub(1)) {
            let Some(line) = self.row_line_text(r) else {
                continue;
            };
            let from = if r == start.row {
                start.offset.min(line.len())
            } else {
                0
            };
            let to = if r == end.row {
                end.offset.min(line.len())
            } else {
                line.len()
            };
            if from <= to && line.is_char_boundary(from) && line.is_char_boundary(to) {
                pieces.push(&line[from..to]);
            }
        }
        (!pieces.is_empty()).then(|| pieces.join("\n"))
    }

    pub(crate) fn jump_hunk(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        if self.hunk_rows.is_empty() || self.rows.is_empty() {
            return false;
        }
        let last = self.rows.len() - 1;
        let targets: Vec<usize> = self.hunk_rows.iter().map(|&h| (h + 1).min(last)).collect();
        let next = if forward {
            targets.iter().copied().find(|&t| t > self.cursor_row)
        } else {
            targets.iter().copied().rev().find(|&t| t < self.cursor_row)
        };
        if let Some(target) = next {
            self.cursor_row = target;
            self.reveal_row = Some(target.saturating_sub(1));
            cx.notify();
            true
        } else {
            false
        }
    }

    pub(crate) fn step_line(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let next = (self.cursor_row as isize + delta)
            .clamp(0, self.rows.len().saturating_sub(1) as isize) as usize;
        if next != self.cursor_row {
            self.cursor_row = next;
            self.reveal_row = Some(next);
            cx.notify();
        }
    }

    #[cfg(test)]
    pub(crate) fn cursor_row(&self) -> usize {
        self.cursor_row
    }

    #[cfg(test)]
    pub(crate) fn scroll_offset(&self) -> Point<Pixels> {
        self.scroll
    }

    #[cfg(test)]
    pub(crate) fn has_highlights(&self) -> (bool, bool) {
        (self.old_highlights.is_some(), self.new_highlights.is_some())
    }

    #[cfg(test)]
    pub(crate) fn set_selection_for_test(
        &mut self,
        start: DiffPos,
        end: DiffPos,
        cx: &mut Context<Self>,
    ) {
        self.selection = Some((self.clamp_pos(start), self.clamp_pos(end)));
        self.selects_all = false;
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() - 1;
        let end_offset = self.row_line_text(last).map_or(0, str::len);
        self.selection = Some((
            DiffPos { row: 0, offset: 0 },
            DiffPos {
                row: last,
                offset: end_offset,
            },
        ));
        self.selects_all = true;
        self.dragging = false;
        cx.notify();
    }

    fn content_size(&self, metrics: &Metrics) -> (Pixels, Pixels) {
        let width = metrics.char_width * self.max_columns as f32 + px(TEXT_PAD_RIGHT);
        let height = metrics.line_height * self.rows.len() as f32 + px(TOP_PAD + FOOTER_HEIGHT);
        (width, height)
    }

    fn measure_scrollbars(&self, metrics: &Metrics) {
        let (width, height) = self.content_size(metrics);
        self.bar_metrics.set((
            ScrollbarMetrics {
                viewport: f32::from(metrics.bounds.size.height),
                content: f32::from(height),
                position: f32::from(self.scroll.y),
            },
            ScrollbarMetrics {
                viewport: f32::from(metrics.text_width()),
                content: f32::from(width),
                position: f32::from(self.scroll.x),
            },
        ));
    }

    fn clamp_scroll(&mut self, metrics: &Metrics) {
        let (width, height) = self.content_size(metrics);
        let max_x = (width - metrics.text_width()).max(px(0.));
        let max_y = (height - metrics.bounds.size.height).max(px(0.));
        self.scroll.x = self.scroll.x.clamp(px(0.), max_x);
        self.scroll.y = self.scroll.y.clamp(px(0.), max_y);
    }

    fn apply_reveal(&mut self, metrics: &Metrics) {
        let Some(row) = self.reveal_row.take() else {
            return;
        };
        let top = metrics.line_height * row as f32 + px(TOP_PAD);
        let height = metrics.bounds.size.height;
        if top < self.scroll.y {
            self.scroll.y = (top - px(TOP_PAD)).max(px(0.));
        } else if top + metrics.line_height * 2. > self.scroll.y + height {
            self.scroll.y = (top + metrics.line_height * 2. - height).max(px(0.));
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
            cx.notify();
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        if modifiers.platform || modifiers.control || modifiers.alt {
            return;
        }
        match event.keystroke.key.as_str() {
            "escape" => {
                self.dragging = false;
                cx.stop_propagation();
                if let Some(on_escape) = self.on_escape.clone() {
                    on_escape(window, cx);
                }
            }
            "j" | "down" => {
                cx.stop_propagation();
                self.step_line(1, cx);
            }
            "k" | "up" => {
                cx.stop_propagation();
                self.step_line(-1, cx);
            }
            "]" => {
                cx.stop_propagation();
                self.jump_hunk(true, cx);
            }
            "[" => {
                cx.stop_propagation();
                self.jump_hunk(false, cx);
            }
            "left" => {
                if let Some(metrics) = self.metrics.get() {
                    cx.stop_propagation();
                    self.scroll_by(point(metrics.char_width * 4., px(0.)), cx);
                }
            }
            "right" => {
                if let Some(metrics) = self.metrics.get() {
                    cx.stop_propagation();
                    self.scroll_by(point(-metrics.char_width * 4., px(0.)), cx);
                }
            }
            "pageup" => {
                if let Some(metrics) = self.metrics.get() {
                    cx.stop_propagation();
                    let page = metrics.bounds.size.height - metrics.line_height;
                    self.scroll_by(point(px(0.), page), cx);
                }
            }
            "pagedown" => {
                if let Some(metrics) = self.metrics.get() {
                    cx.stop_propagation();
                    let page = metrics.bounds.size.height - metrics.line_height;
                    self.scroll_by(point(px(0.), -page), cx);
                }
            }
            _ => {}
        }
    }
}

impl Drop for UnifiedDiffView {
    fn drop(&mut self) {
        self.cancel.store(1, Ordering::Relaxed);
    }
}

fn diff_scrollbar(
    bar_metrics: &Rc<Cell<(ScrollbarMetrics, ScrollbarMetrics)>>,
    horizontal: bool,
    cx: &mut Context<UnifiedDiffView>,
) -> Entity<Scrollbar> {
    let read = bar_metrics.clone();
    let view = cx.weak_entity();
    cx.new(|_| {
        let bar = Scrollbar::external(
            ScrollbarKind::App,
            Rc::new(move || {
                let (vertical, sideways) = read.get();
                if horizontal {
                    sideways
                } else {
                    vertical
                }
            }),
            Rc::new(move |position, cx| {
                let _ = view.update(cx, |view, cx| {
                    if horizontal {
                        view.scroll.x = px(position);
                    } else {
                        view.scroll.y = px(position);
                    }
                    cx.notify();
                });
            }),
        );
        if horizontal {
            bar.horizontal()
        } else {
            bar
        }
    })
}

pub(crate) fn render_viewed_checkbox(viewed: bool, changed_since_viewed: bool) -> gpui::Div {
    let border_color = if viewed {
        theme::accent()
    } else if changed_since_viewed {
        theme::warning()
    } else {
        theme::faint()
    };
    div()
        .flex_none()
        .size(px(13.))
        .rounded(px(3.))
        .flex()
        .items_center()
        .justify_center()
        .when(viewed, |box_el| {
            box_el.bg(theme::accent()).child(
                svg()
                    .size(px(10.))
                    .path("check.svg")
                    .text_color(theme::accent_ink()),
            )
        })
        .when(!viewed, |box_el| {
            box_el.border_1().border_color(border_color)
        })
}

impl Render for UnifiedDiffView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (_, font_size) = CodeView::font(cx);
        let line_height = (font_size * crate::app_settings::TERMINAL_LINE_HEIGHT_DEFAULT).round();
        let text_left = self
            .metrics
            .get()
            .map_or(px(0.), |metrics| metrics.text_left);
        let footer_top = px(TOP_PAD) + line_height * self.rows.len() as f32 - self.scroll.y;
        let show_footer = self.metrics.get().is_none_or(|metrics| {
            footer_top < metrics.bounds.size.height && footer_top + px(FOOTER_HEIGHT) > px(0.)
        });
        let toggle_cb = self.on_toggle_viewed.clone();
        let viewed = self.viewed;
        let changed = self.changed_since_viewed;
        let label = crate::i18n::tf(
            "changes.mark_file_viewed",
            &[("name", self.file_name.to_string())],
        );

        div()
            .id("changes-unified-diff")
            .when(cfg!(test), |element| {
                element.debug_selector(|| "CHANGES_DIFF_CODE".into())
            })
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .overflow_hidden()
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::select_all))
            .on_key_down(cx.listener(Self::on_key))
            .child(UnifiedDiffElement { view: cx.entity() })
            .when(show_footer, |el| {
                el.child(
                    div()
                        .id("changes-diff-mark-viewed")
                        .when(cfg!(test), |element| {
                            element.debug_selector(|| "CHANGES_DIFF_VIEWED".into())
                        })
                        .absolute()
                        .left_0()
                        .right(app_scrollbar_gutter())
                        .top(footer_top)
                        .h(px(FOOTER_HEIGHT))
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .border_t_1()
                        .border_color(theme::border())
                        .cursor_pointer()
                        .hover(|row| row.bg(theme::with_alpha(theme::sidebar_selected(), 0.35)))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |_, window, cx| {
                            if let Some(cb) = &toggle_cb {
                                cb(window, cx);
                            }
                        })
                        .child(render_viewed_checkbox(viewed, changed))
                        .child(
                            div()
                                .text_size(theme::text_ui())
                                .text_color(theme::muted())
                                .child(label),
                        ),
                )
            })
            .child(self.vertical_bar.clone())
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(text_left)
                    .right(app_scrollbar_gutter())
                    .child(self.horizontal_bar.clone()),
            )
    }
}

struct ShapedDiffRow {
    row_index: usize,
    bg: Option<Hsla>,
    cursor: bool,
    old_num: Option<ShapedLine>,
    new_num: Option<ShapedLine>,
    sign: Option<ShapedLine>,
    segments: Vec<(usize, ShapedLine)>,
    hunk_line: Option<ShapedLine>,
    #[cfg(test)]
    painted: Vec<(String, Hsla)>,
}

struct RowStyle<'a> {
    font: &'a Font,
    font_size: Pixels,
    char_width: Pixels,
    color: Hsla,
    syntax: Option<(&'a Highlights, &'a SyntaxTheme)>,
}

struct ShapedDocLine {
    segments: Vec<(usize, ShapedLine)>,
    #[cfg(test)]
    painted: Vec<(String, Hsla)>,
}

fn shape_doc_line(
    document: &Document,
    line: usize,
    (from, to): (usize, usize),
    style: &RowStyle,
    window: &Window,
) -> ShapedDocLine {
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
    ShapedDocLine {
        segments,
        #[cfg(test)]
        painted,
    }
}

fn shape_plain(
    text: SharedString,
    font: &Font,
    size: Pixels,
    color: Hsla,
    window: &Window,
) -> ShapedLine {
    let len = text.len();
    window.text_system().shape_line(
        text,
        size,
        &[TextRun {
            len,
            font: font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }],
        None,
    )
}

struct UnifiedDiffElement {
    view: Entity<UnifiedDiffView>,
}

struct DiffPrepaint {
    hitbox: Hitbox,
    metrics: Metrics,
    scroll: Point<Pixels>,
    focused: bool,
    rows: Vec<ShapedDiffRow>,
    selections: Vec<Bounds<Pixels>>,
}

impl IntoElement for UnifiedDiffElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for UnifiedDiffElement {
    type RequestLayoutState = ();
    type PrepaintState = DiffPrepaint;

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
    ) -> DiffPrepaint {
        let (font, font_size) = CodeView::font(cx);
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&font);
        let char_width = text_system
            .em_advance(font_id, font_size)
            .unwrap_or(font_size * 0.6);
        let line_height = (font_size * crate::app_settings::TERMINAL_LINE_HEIGHT_DEFAULT).round();
        let num_font_size = (font_size * 0.9).max(px(10.));

        let (old_digits, new_digits, focused) = {
            let v = self.view.read(cx);
            (v.old_digits, v.new_digits, v.focus.is_focused(window))
        };
        let old_col_w = char_width * old_digits as f32;
        let new_col_w = char_width * new_digits as f32;
        let old_num_right = px(GUTTER_PAD_LEFT) + old_col_w;
        let new_num_right = old_num_right + px(COL_GAP) + new_col_w;
        let sign_left = new_num_right + px(COL_GAP);
        let text_left = sign_left + px(SIGN_WIDTH + TEXT_GAP);

        let metrics = Metrics {
            bounds,
            line_height,
            char_width,
            old_num_right,
            new_num_right,
            sign_left,
            text_left,
        };
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);

        self.view.update(cx, |view, _| {
            view.metrics.set(Some(metrics));
            view.apply_reveal(&metrics);
            view.clamp_scroll(&metrics);
            view.measure_scrollbars(&metrics);
        });

        let view = self.view.read(cx);
        let scroll = view.scroll;
        let top = bounds.top() + px(TOP_PAD) - scroll.y;
        let first = (f32::from(scroll.y - px(TOP_PAD)) / f32::from(line_height))
            .floor()
            .max(0.) as usize;
        let shown_lines =
            (f32::from(bounds.size.height) / f32::from(line_height)).ceil() as usize + 1;
        let last = (first + shown_lines).min(view.rows.len());

        let visible = metrics.text_width();
        let from = (f32::from(scroll.x) / f32::from(char_width)).floor() as usize;
        let to = from + (f32::from(visible) / f32::from(char_width)).ceil() as usize + 2;

        let light = theme::active_variant().is_light();
        let add_bg = theme::with_alpha(theme::accent(), if light { 0.12 } else { 0.10 });
        let del_bg = theme::with_alpha(theme::danger(), if light { 0.12 } else { 0.13 });
        let hunk_bg = theme::panel();
        let number_color = theme::faint();
        let text_color = theme::text();

        let syntax = (view.old_highlights.is_some() || view.new_highlights.is_some())
            .then(|| SyntaxTheme::current(cx));
        let old_style = RowStyle {
            font: &font,
            font_size,
            char_width,
            color: text_color,
            syntax: view.old_highlights.as_deref().zip(syntax.as_ref()),
        };
        let new_style = RowStyle {
            font: &font,
            font_size,
            char_width,
            color: text_color,
            syntax: view.new_highlights.as_deref().zip(syntax.as_ref()),
        };

        let mut shaped_rows = Vec::with_capacity(last.saturating_sub(first));
        for row_index in first..last {
            let cursor = row_index == view.cursor_row;
            match &view.rows[row_index] {
                DiffRow::Hunk { header } => {
                    let hunk_line = Some(shape_plain(
                        header.clone(),
                        &font,
                        num_font_size,
                        number_color,
                        window,
                    ));
                    shaped_rows.push(ShapedDiffRow {
                        row_index,
                        bg: Some(hunk_bg),
                        cursor,
                        old_num: None,
                        new_num: None,
                        sign: None,
                        segments: Vec::new(),
                        hunk_line,
                        #[cfg(test)]
                        painted: Vec::new(),
                    });
                }
                DiffRow::Context { old_line, new_line } => {
                    let shaped =
                        shape_doc_line(&view.new_doc, *new_line, (from, to), &new_style, window);
                    let old_num = Some(shape_plain(
                        (*old_line + 1).to_string().into(),
                        &font,
                        num_font_size,
                        number_color,
                        window,
                    ));
                    let new_num = Some(shape_plain(
                        (*new_line + 1).to_string().into(),
                        &font,
                        num_font_size,
                        number_color,
                        window,
                    ));
                    shaped_rows.push(ShapedDiffRow {
                        row_index,
                        bg: None,
                        cursor,
                        old_num,
                        new_num,
                        sign: None,
                        segments: shaped.segments,
                        hunk_line: None,
                        #[cfg(test)]
                        painted: shaped.painted,
                    });
                }
                DiffRow::Removed { old_line } => {
                    let shaped =
                        shape_doc_line(&view.old_doc, *old_line, (from, to), &old_style, window);
                    let old_num = Some(shape_plain(
                        (*old_line + 1).to_string().into(),
                        &font,
                        num_font_size,
                        number_color,
                        window,
                    ));
                    let sign = Some(shape_plain(
                        "−".into(),
                        &font,
                        font_size,
                        theme::danger(),
                        window,
                    ));
                    shaped_rows.push(ShapedDiffRow {
                        row_index,
                        bg: Some(del_bg),
                        cursor,
                        old_num,
                        new_num: None,
                        sign,
                        segments: shaped.segments,
                        hunk_line: None,
                        #[cfg(test)]
                        painted: shaped.painted,
                    });
                }
                DiffRow::Added { new_line } => {
                    let shaped =
                        shape_doc_line(&view.new_doc, *new_line, (from, to), &new_style, window);
                    let new_num = Some(shape_plain(
                        (*new_line + 1).to_string().into(),
                        &font,
                        num_font_size,
                        number_color,
                        window,
                    ));
                    let sign = Some(shape_plain(
                        "+".into(),
                        &font,
                        font_size,
                        theme::accent(),
                        window,
                    ));
                    shaped_rows.push(ShapedDiffRow {
                        row_index,
                        bg: Some(add_bg),
                        cursor,
                        old_num: None,
                        new_num,
                        sign,
                        segments: shaped.segments,
                        hunk_line: None,
                        #[cfg(test)]
                        painted: shaped.painted,
                    });
                }
            }
        }

        #[cfg(test)]
        view.painted.replace(
            shaped_rows
                .iter()
                .map(|r| (r.row_index, r.painted.clone()))
                .collect(),
        );

        let text_x = bounds.left() + text_left - scroll.x;
        let row_top = |r: usize| top + line_height * r as f32;
        let mut selections = Vec::new();
        if let Some((anchor, head)) = view.selection.filter(|(a, b)| a != b || view.selects_all) {
            let (start, end) = (anchor.min(head), anchor.max(head));
            for shaped in shaped_rows
                .iter()
                .filter(|r| r.row_index >= start.row && r.row_index <= end.row)
            {
                let Some((doc, doc_line)) = view.row_doc_and_line(shaped.row_index) else {
                    continue;
                };
                let line_str = doc.line(doc_line);
                let s_off = if shaped.row_index == start.row {
                    start.offset.min(line_str.len())
                } else {
                    0
                };
                let e_off = if shaped.row_index == end.row {
                    end.offset.min(line_str.len())
                } else {
                    line_str.len()
                };
                let left = text_x + char_width * doc.column_at(doc_line, s_off) as f32;
                let right = text_x + char_width * doc.column_at(doc_line, e_off) as f32;
                let mut sel_bounds = Bounds::new(
                    point(left, row_top(shaped.row_index)),
                    size((right - left).max(px(0.)), line_height),
                );
                if shaped.row_index != end.row {
                    sel_bounds.size.width += char_width / 2.;
                }
                selections.push(sel_bounds);
            }
        }

        DiffPrepaint {
            hitbox,
            metrics,
            scroll,
            focused,
            rows: shaped_rows,
            selections,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut (),
        prepaint: &mut DiffPrepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let metrics = prepaint.metrics;
        window.set_cursor_style(CursorStyle::IBeam, &prepaint.hitbox);
        let top = bounds.top() + px(TOP_PAD) - prepaint.scroll.y;
        let cursor_tint =
            theme::with_alpha(theme::info(), if prepaint.focused { 0.14 } else { 0.06 });

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for row in &prepaint.rows {
                let y = top + metrics.line_height * row.row_index as f32;
                let row_bounds = Bounds::new(
                    point(bounds.left(), y),
                    size(bounds.size.width, metrics.line_height),
                );
                if let Some(bg) = row.bg {
                    window.paint_quad(fill(row_bounds, bg));
                }
                if row.cursor {
                    window.paint_quad(fill(row_bounds, cursor_tint));
                    if prepaint.focused {
                        window.paint_quad(fill(
                            Bounds::new(point(bounds.left(), y), size(px(2.), metrics.line_height)),
                            theme::info(),
                        ));
                    }
                }
            }
        });

        let text_bounds = Bounds::new(
            point(bounds.left() + metrics.text_left, bounds.top()),
            size(metrics.text_width(), bounds.size.height),
        );
        window.with_content_mask(
            Some(ContentMask {
                bounds: text_bounds,
            }),
            |window| {
                let selection_color = theme::with_alpha(theme::accent(), 0.28);
                for sel in &prepaint.selections {
                    window.paint_quad(fill(*sel, selection_color));
                }
                let text_x = bounds.left() + metrics.text_left - prepaint.scroll.x;
                for row in &prepaint.rows {
                    let y = top + metrics.line_height * row.row_index as f32;
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
            for row in &prepaint.rows {
                let y = top + metrics.line_height * row.row_index as f32;
                if let Some(hunk) = &row.hunk_line {
                    let _ = hunk.paint(
                        point(bounds.left() + px(10.) - prepaint.scroll.x.min(px(0.)), y),
                        metrics.line_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                    continue;
                }
                if let Some(old_num) = &row.old_num {
                    let x = bounds.left() + metrics.old_num_right - old_num.width;
                    let _ = old_num.paint(
                        point(x, y),
                        metrics.line_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
                if let Some(new_num) = &row.new_num {
                    let x = bounds.left() + metrics.new_num_right - new_num.width;
                    let _ = new_num.paint(
                        point(x, y),
                        metrics.line_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
                if let Some(sign) = &row.sign {
                    let x = bounds.left() + metrics.sign_left;
                    let _ = sign.paint(
                        point(x, y),
                        metrics.line_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
            }
        });

        register_mouse(self.view.clone(), prepaint.hitbox.clone(), metrics, window);
    }
}

fn position_at(view: &UnifiedDiffView, metrics: &Metrics, at: Point<Pixels>) -> DiffPos {
    if view.rows.is_empty() {
        return DiffPos::default();
    }
    let y = at.y - metrics.bounds.top() - px(TOP_PAD) + view.scroll.y;
    let row_f = (f32::from(y) / f32::from(metrics.line_height)).floor();
    if row_f < 0. {
        return DiffPos { row: 0, offset: 0 };
    }
    let row = (row_f as usize).min(view.rows.len() - 1);
    let Some((doc, doc_line)) = view.row_doc_and_line(row) else {
        return DiffPos { row, offset: 0 };
    };
    let x = at.x - metrics.bounds.left() - metrics.text_left + view.scroll.x;
    let column = f32::from(x.max(px(0.))) / f32::from(metrics.char_width);
    DiffPos {
        row,
        offset: doc.offset_near(doc_line, column),
    }
}

fn register_mouse(
    view: Entity<UnifiedDiffView>,
    hitbox: Hitbox,
    metrics: Metrics,
    window: &mut Window,
) {
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
        let mut focus_cb = None;
        down_view.update(cx, |view, cx| {
            view.focus.focus(window, cx);
            focus_cb = view.on_focus_in.clone();
            let at = position_at(view, &metrics, event.position);
            view.cursor_row = at.row;
            view.selects_all = false;
            match event.click_count {
                2 => {
                    if let Some(line) = view.row_line_text(at.row) {
                        let (start, end) = word_at(line, at.offset);
                        view.selection = Some((
                            DiffPos {
                                row: at.row,
                                offset: start,
                            },
                            DiffPos {
                                row: at.row,
                                offset: end,
                            },
                        ));
                    }
                }
                count if count >= 3 => {
                    let end = view.row_line_text(at.row).map_or(0, str::len);
                    view.selection = Some((
                        DiffPos {
                            row: at.row,
                            offset: 0,
                        },
                        DiffPos {
                            row: at.row,
                            offset: end,
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
        if let Some(cb) = focus_cb {
            cb(window, cx);
        }
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
            view.cursor_row = head.row;
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
