//! Virtualized Unified and Split diff view with two-version background
//! highlighting, gutter-free text selection/copy, hunk navigation, and
//! end-of-diff viewed box.

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

use crate::app_settings::DiffViewMode;
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
const BOTTOM_PAD_NO_FOOTER: f32 = 8.;
const FOOTER_HEIGHT: f32 = 40.;

pub(crate) type DiffCallback = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct DiffPos {
    pub(crate) row: usize,
    pub(crate) offset: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DiffSide {
    Left,
    #[default]
    Right,
}

#[derive(Clone, Debug)]
pub(crate) enum DiffRow {
    Hunk { header: SharedString },
    Context { old_line: usize, new_line: usize },
    Removed { old_line: usize },
    Added { new_line: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SplitCell {
    Empty,
    Context { line: usize },
    Removed { line: usize },
    Added { line: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SplitRow {
    Hunk { header: SharedString },
    Pair { left: SplitCell, right: SplitCell },
}

#[derive(Clone, Copy, Debug)]
struct Metrics {
    mode: DiffViewMode,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    char_width: Pixels,
    old_num_right: Pixels,
    new_num_right: Pixels,
    sign_left: Pixels,
    text_left: Pixels,
    left_col_width: Pixels,
    right_col_left: Pixels,
    right_col_width: Pixels,
    right_num_right: Pixels,
    right_sign_left: Pixels,
    right_text_left: Pixels,
}

impl Metrics {
    fn left_text_width(&self) -> Pixels {
        (self.left_col_width - self.text_left).max(px(0.))
    }

    fn right_text_width(&self) -> Pixels {
        (self.right_col_width - self.right_text_left).max(px(0.))
    }

    fn scroll_viewport_width(&self) -> Pixels {
        match self.mode {
            DiffViewMode::Unified => (self.bounds.size.width - self.text_left).max(px(0.)),
            DiffViewMode::Split => self.left_text_width().min(self.right_text_width()),
        }
    }

    fn shape_viewport_width(&self) -> Pixels {
        match self.mode {
            DiffViewMode::Unified => (self.bounds.size.width - self.text_left).max(px(0.)),
            DiffViewMode::Split => self.left_text_width().max(self.right_text_width()),
        }
    }

    fn side_at_x(&self, x: Pixels) -> DiffSide {
        if x < self.bounds.left() + self.right_col_left {
            DiffSide::Left
        } else {
            DiffSide::Right
        }
    }

    fn side_text_left_abs(&self, side: DiffSide) -> Pixels {
        match (self.mode, side) {
            (DiffViewMode::Unified, _) | (DiffViewMode::Split, DiffSide::Left) => {
                self.bounds.left() + self.text_left
            }
            (DiffViewMode::Split, DiffSide::Right) => {
                self.bounds.left() + self.right_col_left + self.right_text_left
            }
        }
    }

    fn side_right_abs(&self, side: DiffSide) -> Pixels {
        match (self.mode, side) {
            (DiffViewMode::Split, DiffSide::Left) => self.bounds.left() + self.left_col_width,
            _ => self.bounds.right(),
        }
    }
}

pub(crate) struct UnifiedDiffView {
    old_doc: Arc<Document>,
    new_doc: Arc<Document>,
    mode: DiffViewMode,
    show_viewed_footer: bool,
    rows: Arc<Vec<DiffRow>>,
    hunk_rows: Vec<usize>,
    split_rows: Arc<Vec<SplitRow>>,
    split_hunk_rows: Vec<usize>,
    unified_max_columns: usize,
    split_max_columns: usize,
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
    selection_side: DiffSide,
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

fn sanitize_header(header: &str) -> SharedString {
    if header.contains('\r') {
        header.replace('\r', "").into()
    } else {
        header.to_owned().into()
    }
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
        let header = sanitize_header(&hunk.header);
        max_columns = max_columns.max(header.chars().count());
        rows.push(DiffRow::Hunk { header });
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

pub(crate) fn build_split_rows(
    old_doc: &Document,
    new_doc: &Document,
    hunks: &[DiffHunk],
) -> (Vec<SplitRow>, Vec<usize>, usize) {
    let mut rows = Vec::new();
    let mut hunk_rows = Vec::with_capacity(hunks.len());
    let old_count = old_doc.line_count();
    let new_count = new_doc.line_count();

    let flush_run = |rows: &mut Vec<SplitRow>, removed: &mut Vec<usize>, added: &mut Vec<usize>| {
        let pair_count = removed.len().max(added.len());
        for i in 0..pair_count {
            let left = removed
                .get(i)
                .map_or(SplitCell::Empty, |&line| SplitCell::Removed { line });
            let right = added
                .get(i)
                .map_or(SplitCell::Empty, |&line| SplitCell::Added { line });
            rows.push(SplitRow::Pair { left, right });
        }
        removed.clear();
        added.clear();
    };

    let mut removed_run = Vec::new();
    let mut added_run = Vec::new();

    for hunk in hunks {
        hunk_rows.push(rows.len());
        rows.push(SplitRow::Hunk {
            header: sanitize_header(&hunk.header),
        });
        let mut old_idx = hunk.old_start.saturating_sub(1) as usize;
        let mut new_idx = hunk.new_start.saturating_sub(1) as usize;
        for op in &hunk.ops {
            match *op {
                DiffOp::Context { count } => {
                    flush_run(&mut rows, &mut removed_run, &mut added_run);
                    for _ in 0..count {
                        let o = old_idx.min(old_count.saturating_sub(1));
                        let n = new_idx.min(new_count.saturating_sub(1));
                        rows.push(SplitRow::Pair {
                            left: SplitCell::Context { line: o },
                            right: SplitCell::Context { line: n },
                        });
                        old_idx += 1;
                        new_idx += 1;
                    }
                }
                DiffOp::Removed { count } => {
                    if !added_run.is_empty() {
                        flush_run(&mut rows, &mut removed_run, &mut added_run);
                    }
                    for _ in 0..count {
                        let o = old_idx.min(old_count.saturating_sub(1));
                        removed_run.push(o);
                        old_idx += 1;
                    }
                }
                DiffOp::Added { count } => {
                    for _ in 0..count {
                        let n = new_idx.min(new_count.saturating_sub(1));
                        added_run.push(n);
                        new_idx += 1;
                    }
                }
            }
        }
        flush_run(&mut rows, &mut removed_run, &mut added_run);
    }

    let max_columns = old_doc.columns().max(new_doc.columns());
    (rows, hunk_rows, max_columns)
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
        let (rows, hunk_rows, unified_max_columns, old_digits, new_digits) =
            build_rows(&old_doc, &new_doc, hunks);
        let (split_rows, split_hunk_rows, split_max_columns) =
            build_split_rows(&old_doc, &new_doc, hunks);
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
            mode: DiffViewMode::Unified,
            show_viewed_footer: true,
            rows: Arc::new(rows),
            hunk_rows,
            split_rows: Arc::new(split_rows),
            split_hunk_rows,
            unified_max_columns,
            split_max_columns,
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
            selection_side: DiffSide::Right,
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

    pub(crate) fn set_show_viewed_footer(&mut self, show: bool) {
        self.show_viewed_footer = show;
    }

    pub(crate) fn set_mode(&mut self, mode: DiffViewMode, cx: &mut Context<Self>) {
        if self.mode == mode {
            return;
        }
        let mapped_cursor = self.map_cursor_to_mode(mode);
        self.mode = mode;
        self.cursor_row = mapped_cursor;
        self.reveal_row = Some(mapped_cursor);
        self.selection = None;
        self.selects_all = false;
        self.dragging = false;
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn mode(&self) -> DiffViewMode {
        self.mode
    }

    fn map_cursor_to_mode(&self, target_mode: DiffViewMode) -> usize {
        if target_mode == self.mode {
            return self.cursor_row;
        }
        match (self.mode, target_mode) {
            (DiffViewMode::Unified, DiffViewMode::Split) => {
                let Some(row) = self.rows.get(self.cursor_row) else {
                    return 0;
                };
                let found = match *row {
                    DiffRow::Hunk { .. } => self
                        .hunk_rows
                        .iter()
                        .position(|&h| h == self.cursor_row)
                        .and_then(|ord| self.split_hunk_rows.get(ord).copied()),
                    DiffRow::Context { old_line, new_line } => {
                        self.split_rows.iter().position(|sr| {
                            matches!(
                                sr,
                                SplitRow::Pair {
                                    left: SplitCell::Context { line: o },
                                    right: SplitCell::Context { line: n },
                                } if *o == old_line && *n == new_line
                            )
                        })
                    }
                    DiffRow::Removed { old_line } => self.split_rows.iter().position(|sr| {
                        matches!(
                            sr,
                            SplitRow::Pair {
                                left: SplitCell::Removed { line: o },
                                ..
                            } if *o == old_line
                        )
                    }),
                    DiffRow::Added { new_line } => self.split_rows.iter().position(|sr| {
                        matches!(
                            sr,
                            SplitRow::Pair {
                                right: SplitCell::Added { line: n },
                                ..
                            } if *n == new_line
                        )
                    }),
                };
                found
                    .unwrap_or_else(|| self.cursor_row.min(self.split_rows.len().saturating_sub(1)))
            }
            (DiffViewMode::Split, DiffViewMode::Unified) => {
                let Some(sr) = self.split_rows.get(self.cursor_row) else {
                    return 0;
                };
                let found = match *sr {
                    SplitRow::Hunk { .. } => self
                        .split_hunk_rows
                        .iter()
                        .position(|&h| h == self.cursor_row)
                        .and_then(|ord| self.hunk_rows.get(ord).copied()),
                    SplitRow::Pair { left, right } => {
                        let prefer_left = self.selection_side == DiffSide::Left;
                        let match_cell = |cell: SplitCell| -> Option<usize> {
                            match cell {
                                SplitCell::Empty => None,
                                SplitCell::Context { line } => self.rows.iter().position(|ur| {
                                    if prefer_left {
                                        matches!(ur, DiffRow::Context { old_line, .. } if *old_line == line)
                                    } else {
                                        matches!(ur, DiffRow::Context { new_line, .. } if *new_line == line)
                                    }
                                }),
                                SplitCell::Removed { line } => self.rows.iter().position(|ur| {
                                    matches!(ur, DiffRow::Removed { old_line } if *old_line == line)
                                }),
                                SplitCell::Added { line } => self.rows.iter().position(|ur| {
                                    matches!(ur, DiffRow::Added { new_line } if *new_line == line)
                                }),
                            }
                        };
                        if prefer_left {
                            match_cell(left).or_else(|| match_cell(right))
                        } else {
                            match_cell(right).or_else(|| match_cell(left))
                        }
                    }
                };
                found.unwrap_or_else(|| self.cursor_row.min(self.rows.len().saturating_sub(1)))
            }
            _ => self.cursor_row,
        }
    }

    pub(crate) fn set_callbacks(
        &mut self,
        on_escape: DiffCallback,
        on_toggle_viewed: Option<DiffCallback>,
        on_focus_in: DiffCallback,
    ) {
        self.on_escape = Some(on_escape);
        self.on_toggle_viewed = on_toggle_viewed;
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
        let (rows, hunk_rows, unified_max_columns, old_digits, new_digits) =
            build_rows(&old_doc, &new_doc, hunks);
        let (split_rows, split_hunk_rows, split_max_columns) =
            build_split_rows(&old_doc, &new_doc, hunks);
        self.old_doc = old_doc;
        self.new_doc = new_doc;
        self.rows = Arc::new(rows);
        self.hunk_rows = hunk_rows;
        self.split_rows = Arc::new(split_rows);
        self.split_hunk_rows = split_hunk_rows;
        self.unified_max_columns = unified_max_columns;
        self.split_max_columns = split_max_columns;
        self.old_digits = old_digits;
        self.new_digits = new_digits;
        self.grammar = grammar;
        self.file_name = file_name;
        self.viewed = viewed;
        self.changed_since_viewed = changed_since_viewed;
        let len = self.active_row_count();
        self.cursor_row = self.cursor_row.min(len.saturating_sub(1));
        self.selection = self
            .selection
            .map(|(a, b)| (self.clamp_pos(a), self.clamp_pos(b)));
        self.old_highlights = None;
        self.new_highlights = None;
        self.highlight(cx);
        cx.notify();
    }

    fn active_row_count(&self) -> usize {
        match self.mode {
            DiffViewMode::Unified => self.rows.len(),
            DiffViewMode::Split => self.split_rows.len(),
        }
    }

    fn active_hunk_rows(&self) -> &[usize] {
        match self.mode {
            DiffViewMode::Unified => &self.hunk_rows,
            DiffViewMode::Split => &self.split_hunk_rows,
        }
    }

    fn active_max_columns(&self) -> usize {
        match self.mode {
            DiffViewMode::Unified => self.unified_max_columns,
            DiffViewMode::Split => self.split_max_columns,
        }
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

    fn row_doc_and_line_for_side(&self, row: usize, side: DiffSide) -> Option<(&Document, usize)> {
        match self.mode {
            DiffViewMode::Unified => match self.rows.get(row)? {
                DiffRow::Hunk { .. } => None,
                DiffRow::Context { new_line, .. } | DiffRow::Added { new_line } => {
                    Some((&self.new_doc, *new_line))
                }
                DiffRow::Removed { old_line } => Some((&self.old_doc, *old_line)),
            },
            DiffViewMode::Split => match self.split_rows.get(row)? {
                SplitRow::Hunk { .. } => None,
                SplitRow::Pair { left, right } => match side {
                    DiffSide::Left => match *left {
                        SplitCell::Context { line } | SplitCell::Removed { line } => {
                            Some((&self.old_doc, line))
                        }
                        SplitCell::Empty | SplitCell::Added { .. } => None,
                    },
                    DiffSide::Right => match *right {
                        SplitCell::Context { line } | SplitCell::Added { line } => {
                            Some((&self.new_doc, line))
                        }
                        SplitCell::Empty | SplitCell::Removed { .. } => None,
                    },
                },
            },
        }
    }

    fn row_doc_and_line(&self, row: usize) -> Option<(&Document, usize)> {
        self.row_doc_and_line_for_side(row, self.selection_side)
    }

    fn row_line_text(&self, row: usize) -> Option<&str> {
        let (doc, line) = self.row_doc_and_line(row)?;
        Some(doc.line(line))
    }

    fn side_has_any_lines(&self, side: DiffSide) -> bool {
        (0..self.active_row_count()).any(|row| self.row_doc_and_line_for_side(row, side).is_some())
    }

    fn clamp_pos(&self, pos: DiffPos) -> DiffPos {
        let count = self.active_row_count();
        if count == 0 {
            return DiffPos::default();
        }
        let row = pos.row.min(count - 1);
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
        let count = self.active_row_count();
        if count == 0 {
            return None;
        }
        if self.selects_all {
            let lines: Vec<&str> = (0..count).filter_map(|r| self.row_line_text(r)).collect();
            return (!lines.is_empty()).then(|| lines.join("\n"));
        }
        let (anchor, head) = self.selection?;
        if anchor == head {
            return None;
        }
        let (start, end) = (anchor.min(head), anchor.max(head));
        let mut pieces = Vec::new();
        for r in start.row..=end.row.min(count.saturating_sub(1)) {
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
        let count = self.active_row_count();
        let hunks = self.active_hunk_rows();
        if hunks.is_empty() || count == 0 {
            return false;
        }
        let last = count - 1;
        let targets: Vec<usize> = hunks.iter().map(|&h| (h + 1).min(last)).collect();
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
        let count = self.active_row_count();
        if count == 0 {
            return;
        }
        let next =
            (self.cursor_row as isize + delta).clamp(0, count.saturating_sub(1) as isize) as usize;
        if next != self.cursor_row {
            self.cursor_row = next;
            self.reveal_row = Some(next);
            cx.notify();
        }
    }

    /// 1-based new-file line nearest the current diff cursor / viewport, used when
    /// flipping `Diff → Code` in place so reading context is preserved (#1421).
    pub(crate) fn current_new_line(&self) -> Option<u32> {
        let count = self.active_row_count();
        if count == 0 {
            return None;
        }
        let anchor = if let Some(metrics) = self.metrics.get() {
            let line_h = f32::from(metrics.line_height).max(1.);
            let top = ((f32::from(self.scroll.y) - TOP_PAD).max(0.) / line_h) as usize;
            let visible = ((f32::from(metrics.bounds.size.height) / line_h).ceil() as usize).max(1);
            if self.cursor_row >= top && self.cursor_row < top.saturating_add(visible) {
                self.cursor_row
            } else {
                top.min(count - 1)
            }
        } else {
            self.cursor_row.min(count - 1)
        };
        let row_new_line = |idx: usize| -> Option<u32> {
            match self.mode {
                DiffViewMode::Unified => match self.rows.get(idx)? {
                    DiffRow::Context { new_line, .. } | DiffRow::Added { new_line } => {
                        Some(*new_line as u32 + 1)
                    }
                    DiffRow::Hunk { .. } | DiffRow::Removed { .. } => None,
                },
                DiffViewMode::Split => match self.split_rows.get(idx)? {
                    SplitRow::Pair {
                        right: SplitCell::Context { line } | SplitCell::Added { line },
                        ..
                    } => Some(*line as u32 + 1),
                    _ => None,
                },
            }
        };
        (anchor..count)
            .find_map(&row_new_line)
            .or_else(|| (0..anchor).rev().find_map(&row_new_line))
    }

    /// Scrolls the diff near 1-based new-file `line` when flipping `Code → Diff` (#1421).
    pub(crate) fn scroll_to_new_line(&mut self, target_line: u32, cx: &mut Context<Self>) {
        let target_idx = target_line.saturating_sub(1) as usize;
        let found = match self.mode {
            DiffViewMode::Unified => self.rows.iter().position(|row| match *row {
                DiffRow::Context { new_line, .. } | DiffRow::Added { new_line } => {
                    new_line >= target_idx
                }
                _ => false,
            }),
            DiffViewMode::Split => self.split_rows.iter().position(|row| match *row {
                SplitRow::Pair {
                    right: SplitCell::Context { line } | SplitCell::Added { line },
                    ..
                } => line >= target_idx,
                _ => false,
            }),
        };
        if let Some(row) = found {
            self.cursor_row = row;
            self.reveal_row = Some(row);
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
    pub(crate) fn split_rows(&self) -> &[SplitRow] {
        &self.split_rows
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

    #[cfg(test)]
    pub(crate) fn set_split_selection_for_test(
        &mut self,
        side: DiffSide,
        start: DiffPos,
        end: DiffPos,
        cx: &mut Context<Self>,
    ) {
        self.selection_side = side;
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
        let count = self.active_row_count();
        if count == 0 {
            return;
        }
        if self.mode == DiffViewMode::Split && !self.side_has_any_lines(self.selection_side) {
            let other = match self.selection_side {
                DiffSide::Left => DiffSide::Right,
                DiffSide::Right => DiffSide::Left,
            };
            if self.side_has_any_lines(other) {
                self.selection_side = other;
            }
        }
        let last = count - 1;
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

    fn bottom_extra(&self) -> f32 {
        if self.show_viewed_footer {
            FOOTER_HEIGHT
        } else {
            BOTTOM_PAD_NO_FOOTER
        }
    }

    fn content_size(&self, metrics: &Metrics) -> (Pixels, Pixels) {
        let width = metrics.char_width * self.active_max_columns() as f32 + px(TEXT_PAD_RIGHT);
        let height = metrics.line_height * self.active_row_count() as f32
            + px(TOP_PAD + self.bottom_extra());
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
                viewport: f32::from(metrics.scroll_viewport_width()),
                content: f32::from(width),
                position: f32::from(self.scroll.x),
            },
        ));
    }

    fn clamp_scroll(&mut self, metrics: &Metrics) {
        let (width, height) = self.content_size(metrics);
        let max_x = (width - metrics.scroll_viewport_width()).max(px(0.));
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
        let horizontal_left = match self.mode {
            DiffViewMode::Unified => self
                .metrics
                .get()
                .map_or(px(0.), |metrics| metrics.text_left),
            DiffViewMode::Split => px(0.),
        };
        let footer_top = px(TOP_PAD) + line_height * self.active_row_count() as f32 - self.scroll.y;
        let show_footer = self.show_viewed_footer
            && self.metrics.get().is_none_or(|metrics| {
                footer_top < metrics.bounds.size.height && footer_top + px(FOOTER_HEIGHT) > px(0.)
            });
        let toggle_cb = self.on_toggle_viewed.clone();
        let viewed = self.viewed;
        let changed = self.changed_since_viewed;
        let label = crate::i18n::tf(
            "changes.mark_file_viewed",
            &[("name", self.file_name.to_string())],
        );
        let is_split = self.mode == DiffViewMode::Split;

        div()
            .id("changes-unified-diff")
            .when(cfg!(test), move |element| {
                element.debug_selector(move || {
                    if is_split {
                        "CHANGES_DIFF_CODE split".into()
                    } else {
                        "CHANGES_DIFF_CODE".into()
                    }
                })
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
                    .left(horizontal_left)
                    .right(app_scrollbar_gutter())
                    .child(self.horizontal_bar.clone()),
            )
    }
}

struct ShapedUnifiedRow {
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

struct ShapedSplitHalf {
    bg: Option<Hsla>,
    num: Option<ShapedLine>,
    sign: Option<ShapedLine>,
    segments: Vec<(usize, ShapedLine)>,
    #[cfg(test)]
    painted: Vec<(String, Hsla)>,
}

#[allow(clippy::large_enum_variant)]
enum ShapedSplitRowKind {
    Hunk(ShapedLine),
    Pair {
        left: ShapedSplitHalf,
        right: ShapedSplitHalf,
    },
}

struct ShapedSplitRow {
    row_index: usize,
    cursor: bool,
    kind: ShapedSplitRowKind,
}

enum ShapedRows {
    Unified(Vec<ShapedUnifiedRow>),
    Split(Vec<ShapedSplitRow>),
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

#[allow(clippy::too_many_arguments)]
fn shape_split_cell(
    cell: SplitCell,
    doc: &Document,
    style: &RowStyle,
    (from, to): (usize, usize),
    font: &Font,
    font_size: Pixels,
    num_font_size: Pixels,
    number_color: Hsla,
    add_bg: Hsla,
    del_bg: Hsla,
    filler_bg: Hsla,
    window: &Window,
) -> ShapedSplitHalf {
    match cell {
        SplitCell::Empty => ShapedSplitHalf {
            bg: Some(filler_bg),
            num: None,
            sign: None,
            segments: Vec::new(),
            #[cfg(test)]
            painted: Vec::new(),
        },
        SplitCell::Context { line } => {
            let shaped = shape_doc_line(doc, line, (from, to), style, window);
            let num = Some(shape_plain(
                (line + 1).to_string().into(),
                font,
                num_font_size,
                number_color,
                window,
            ));
            ShapedSplitHalf {
                bg: None,
                num,
                sign: None,
                segments: shaped.segments,
                #[cfg(test)]
                painted: shaped.painted,
            }
        }
        SplitCell::Removed { line } => {
            let shaped = shape_doc_line(doc, line, (from, to), style, window);
            let num = Some(shape_plain(
                (line + 1).to_string().into(),
                font,
                num_font_size,
                number_color,
                window,
            ));
            let sign = Some(shape_plain(
                "−".into(),
                font,
                font_size,
                theme::danger(),
                window,
            ));
            ShapedSplitHalf {
                bg: Some(del_bg),
                num,
                sign,
                segments: shaped.segments,
                #[cfg(test)]
                painted: shaped.painted,
            }
        }
        SplitCell::Added { line } => {
            let shaped = shape_doc_line(doc, line, (from, to), style, window);
            let num = Some(shape_plain(
                (line + 1).to_string().into(),
                font,
                num_font_size,
                number_color,
                window,
            ));
            let sign = Some(shape_plain(
                "+".into(),
                font,
                font_size,
                theme::accent(),
                window,
            ));
            ShapedSplitHalf {
                bg: Some(add_bg),
                num,
                sign,
                segments: shaped.segments,
                #[cfg(test)]
                painted: shaped.painted,
            }
        }
    }
}

struct UnifiedDiffElement {
    view: Entity<UnifiedDiffView>,
}

struct DiffPrepaint {
    hitbox: Hitbox,
    metrics: Metrics,
    scroll: Point<Pixels>,
    focused: bool,
    selection_side: DiffSide,
    rows: ShapedRows,
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

        let (mode, old_digits, new_digits, focused, selection_side) = {
            let v = self.view.read(cx);
            (
                v.mode,
                v.old_digits,
                v.new_digits,
                v.focus.is_focused(window),
                v.selection_side,
            )
        };
        let old_col_w = char_width * old_digits as f32;
        let new_col_w = char_width * new_digits as f32;

        let old_num_right = px(GUTTER_PAD_LEFT) + old_col_w;
        let (new_num_right, sign_left, text_left) = match mode {
            DiffViewMode::Unified => {
                let new_num_right = old_num_right + px(COL_GAP) + new_col_w;
                let sign_left = new_num_right + px(COL_GAP);
                let text_left = sign_left + px(SIGN_WIDTH + TEXT_GAP);
                (new_num_right, sign_left, text_left)
            }
            DiffViewMode::Split => {
                let sign_left = old_num_right + px(COL_GAP);
                let text_left = sign_left + px(SIGN_WIDTH + TEXT_GAP);
                (px(0.), sign_left, text_left)
            }
        };

        let left_col_width = ((bounds.size.width - px(1.)) * 0.5).floor().max(px(0.));
        let right_col_left = left_col_width + px(1.);
        let right_col_width = (bounds.size.width - right_col_left).max(px(0.));
        let right_num_right = px(GUTTER_PAD_LEFT) + new_col_w;
        let right_sign_left = right_num_right + px(COL_GAP);
        let right_text_left = right_sign_left + px(SIGN_WIDTH + TEXT_GAP);

        let metrics = Metrics {
            mode,
            bounds,
            line_height,
            char_width,
            old_num_right,
            new_num_right,
            sign_left,
            text_left,
            left_col_width,
            right_col_left,
            right_col_width,
            right_num_right,
            right_sign_left,
            right_text_left,
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
        let last = (first + shown_lines).min(view.active_row_count());

        let visible = metrics.shape_viewport_width();
        let from = (f32::from(scroll.x) / f32::from(char_width)).floor() as usize;
        let to = from + (f32::from(visible) / f32::from(char_width)).ceil() as usize + 2;

        let light = theme::active_variant().is_light();
        let add_bg = theme::with_alpha(theme::accent(), if light { 0.12 } else { 0.10 });
        let del_bg = theme::with_alpha(theme::danger(), if light { 0.12 } else { 0.13 });
        let hunk_bg = theme::panel();
        let filler_bg = theme::panel();
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

        let rows = match mode {
            DiffViewMode::Unified => {
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
                            shaped_rows.push(ShapedUnifiedRow {
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
                            let shaped = shape_doc_line(
                                &view.new_doc,
                                *new_line,
                                (from, to),
                                &new_style,
                                window,
                            );
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
                            shaped_rows.push(ShapedUnifiedRow {
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
                            let shaped = shape_doc_line(
                                &view.old_doc,
                                *old_line,
                                (from, to),
                                &old_style,
                                window,
                            );
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
                            shaped_rows.push(ShapedUnifiedRow {
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
                            let shaped = shape_doc_line(
                                &view.new_doc,
                                *new_line,
                                (from, to),
                                &new_style,
                                window,
                            );
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
                            shaped_rows.push(ShapedUnifiedRow {
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
                ShapedRows::Unified(shaped_rows)
            }
            DiffViewMode::Split => {
                let mut shaped_rows = Vec::with_capacity(last.saturating_sub(first));
                #[cfg(test)]
                let mut test_painted = std::collections::BTreeMap::new();
                for row_index in first..last {
                    let cursor = row_index == view.cursor_row;
                    match &view.split_rows[row_index] {
                        SplitRow::Hunk { header } => {
                            let hunk_line = shape_plain(
                                header.clone(),
                                &font,
                                num_font_size,
                                number_color,
                                window,
                            );
                            #[cfg(test)]
                            test_painted.insert(row_index, Vec::new());
                            shaped_rows.push(ShapedSplitRow {
                                row_index,
                                cursor,
                                kind: ShapedSplitRowKind::Hunk(hunk_line),
                            });
                        }
                        SplitRow::Pair { left, right } => {
                            let left_shaped = shape_split_cell(
                                *left,
                                &view.old_doc,
                                &old_style,
                                (from, to),
                                &font,
                                font_size,
                                num_font_size,
                                number_color,
                                add_bg,
                                del_bg,
                                filler_bg,
                                window,
                            );
                            let right_shaped = shape_split_cell(
                                *right,
                                &view.new_doc,
                                &new_style,
                                (from, to),
                                &font,
                                font_size,
                                num_font_size,
                                number_color,
                                add_bg,
                                del_bg,
                                filler_bg,
                                window,
                            );
                            #[cfg(test)]
                            {
                                let mut row_tokens = left_shaped.painted.clone();
                                row_tokens.extend(right_shaped.painted.iter().cloned());
                                test_painted.insert(row_index, row_tokens);
                            }
                            shaped_rows.push(ShapedSplitRow {
                                row_index,
                                cursor,
                                kind: ShapedSplitRowKind::Pair {
                                    left: left_shaped,
                                    right: right_shaped,
                                },
                            });
                        }
                    }
                }
                #[cfg(test)]
                view.painted.replace(test_painted);
                ShapedRows::Split(shaped_rows)
            }
        };

        let text_x = metrics.side_text_left_abs(selection_side) - scroll.x;
        let row_top = |r: usize| top + line_height * r as f32;
        let mut selections = Vec::new();
        if let Some((anchor, head)) = view.selection.filter(|(a, b)| a != b || view.selects_all) {
            let (start, end) = (anchor.min(head), anchor.max(head));
            let vis_start = start.row.max(first);
            let vis_end = end.row.min(last.saturating_sub(1));
            if vis_start <= vis_end && last > first {
                for row_index in vis_start..=vis_end {
                    let Some((doc, doc_line)) =
                        view.row_doc_and_line_for_side(row_index, selection_side)
                    else {
                        continue;
                    };
                    let line_str = doc.line(doc_line);
                    let s_off = if row_index == start.row {
                        start.offset.min(line_str.len())
                    } else {
                        0
                    };
                    let e_off = if row_index == end.row {
                        end.offset.min(line_str.len())
                    } else {
                        line_str.len()
                    };
                    let left = text_x + char_width * doc.column_at(doc_line, s_off) as f32;
                    let right = text_x + char_width * doc.column_at(doc_line, e_off) as f32;
                    let mut sel_bounds = Bounds::new(
                        point(left, row_top(row_index)),
                        size((right - left).max(px(0.)), line_height),
                    );
                    if row_index != end.row {
                        sel_bounds.size.width += char_width / 2.;
                    }
                    selections.push(sel_bounds);
                }
            }
        }

        DiffPrepaint {
            hitbox,
            metrics,
            scroll,
            focused,
            selection_side,
            rows,
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
        let selection_color = theme::with_alpha(theme::accent(), 0.28);

        match &prepaint.rows {
            ShapedRows::Unified(rows) => {
                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                    for row in rows {
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
                                    Bounds::new(
                                        point(bounds.left(), y),
                                        size(px(2.), metrics.line_height),
                                    ),
                                    theme::info(),
                                ));
                            }
                        }
                    }
                });

                let text_bounds = Bounds::new(
                    point(bounds.left() + metrics.text_left, bounds.top()),
                    size(metrics.scroll_viewport_width(), bounds.size.height),
                );
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: text_bounds,
                    }),
                    |window| {
                        for sel in &prepaint.selections {
                            window.paint_quad(fill(*sel, selection_color));
                        }
                        let text_x = bounds.left() + metrics.text_left - prepaint.scroll.x;
                        for row in rows {
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
                    for row in rows {
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
            }
            ShapedRows::Split(rows) => {
                let hunk_bg = theme::panel();
                let border_color = theme::border();

                // 1. Backgrounds, center divider on code rows, cursor tint, and hunk text.
                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                    for row in rows {
                        let y = top + metrics.line_height * row.row_index as f32;
                        let row_bounds = Bounds::new(
                            point(bounds.left(), y),
                            size(bounds.size.width, metrics.line_height),
                        );
                        match &row.kind {
                            ShapedSplitRowKind::Hunk(hunk) => {
                                window.paint_quad(fill(row_bounds, hunk_bg));
                                if row.cursor {
                                    window.paint_quad(fill(row_bounds, cursor_tint));
                                    if prepaint.focused {
                                        window.paint_quad(fill(
                                            Bounds::new(
                                                point(bounds.left(), y),
                                                size(px(2.), metrics.line_height),
                                            ),
                                            theme::info(),
                                        ));
                                    }
                                }
                                let _ = hunk.paint(
                                    point(
                                        bounds.left() + px(10.) - prepaint.scroll.x.min(px(0.)),
                                        y,
                                    ),
                                    metrics.line_height,
                                    TextAlign::Left,
                                    None,
                                    window,
                                    cx,
                                );
                            }
                            ShapedSplitRowKind::Pair { left, right } => {
                                if let Some(bg) = left.bg {
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            point(bounds.left(), y),
                                            size(metrics.left_col_width, metrics.line_height),
                                        ),
                                        bg,
                                    ));
                                }
                                if let Some(bg) = right.bg {
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            point(bounds.left() + metrics.right_col_left, y),
                                            size(metrics.right_col_width, metrics.line_height),
                                        ),
                                        bg,
                                    ));
                                }
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(bounds.left() + metrics.left_col_width, y),
                                        size(px(1.), metrics.line_height),
                                    ),
                                    border_color,
                                ));
                                if row.cursor {
                                    window.paint_quad(fill(row_bounds, cursor_tint));
                                    if prepaint.focused {
                                        window.paint_quad(fill(
                                            Bounds::new(
                                                point(bounds.left(), y),
                                                size(px(2.), metrics.line_height),
                                            ),
                                            theme::info(),
                                        ));
                                    }
                                }
                            }
                        }
                    }
                });

                // 2. Left column text & selection (clipped to left text area).
                let left_text_bounds = Bounds::new(
                    point(bounds.left() + metrics.text_left, bounds.top()),
                    size(metrics.left_text_width(), bounds.size.height),
                );
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: left_text_bounds,
                    }),
                    |window| {
                        if prepaint.selection_side == DiffSide::Left {
                            for sel in &prepaint.selections {
                                window.paint_quad(fill(*sel, selection_color));
                            }
                        }
                        let left_text_x = bounds.left() + metrics.text_left - prepaint.scroll.x;
                        for row in rows {
                            if let ShapedSplitRowKind::Pair { left, .. } = &row.kind {
                                let y = top + metrics.line_height * row.row_index as f32;
                                for (column, shaped) in &left.segments {
                                    let origin =
                                        point(left_text_x + metrics.char_width * *column as f32, y);
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
                        }
                    },
                );

                // 3. Right column text & selection (clipped to right text area).
                let right_text_bounds = Bounds::new(
                    point(
                        bounds.left() + metrics.right_col_left + metrics.right_text_left,
                        bounds.top(),
                    ),
                    size(metrics.right_text_width(), bounds.size.height),
                );
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: right_text_bounds,
                    }),
                    |window| {
                        if prepaint.selection_side == DiffSide::Right {
                            for sel in &prepaint.selections {
                                window.paint_quad(fill(*sel, selection_color));
                            }
                        }
                        let right_text_x =
                            bounds.left() + metrics.right_col_left + metrics.right_text_left
                                - prepaint.scroll.x;
                        for row in rows {
                            if let ShapedSplitRowKind::Pair { right, .. } = &row.kind {
                                let y = top + metrics.line_height * row.row_index as f32;
                                for (column, shaped) in &right.segments {
                                    let origin = point(
                                        right_text_x + metrics.char_width * *column as f32,
                                        y,
                                    );
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
                        }
                    },
                );

                // 4. Left & right gutters (line numbers and −/+ signs).
                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                    for row in rows {
                        if let ShapedSplitRowKind::Pair { left, right } = &row.kind {
                            let y = top + metrics.line_height * row.row_index as f32;
                            if let Some(num) = &left.num {
                                let x = bounds.left() + metrics.old_num_right - num.width;
                                let _ = num.paint(
                                    point(x, y),
                                    metrics.line_height,
                                    TextAlign::Left,
                                    None,
                                    window,
                                    cx,
                                );
                            }
                            if let Some(sign) = &left.sign {
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
                            if let Some(num) = &right.num {
                                let x = bounds.left()
                                    + metrics.right_col_left
                                    + metrics.right_num_right
                                    - num.width;
                                let _ = num.paint(
                                    point(x, y),
                                    metrics.line_height,
                                    TextAlign::Left,
                                    None,
                                    window,
                                    cx,
                                );
                            }
                            if let Some(sign) = &right.sign {
                                let x = bounds.left()
                                    + metrics.right_col_left
                                    + metrics.right_sign_left;
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
                    }
                });
            }
        }

        register_mouse(self.view.clone(), prepaint.hitbox.clone(), metrics, window);
    }
}

fn position_at(
    view: &UnifiedDiffView,
    metrics: &Metrics,
    at: Point<Pixels>,
    side: DiffSide,
) -> DiffPos {
    let count = view.active_row_count();
    if count == 0 {
        return DiffPos::default();
    }
    let y = at.y - metrics.bounds.top() - px(TOP_PAD) + view.scroll.y;
    let row_f = (f32::from(y) / f32::from(metrics.line_height)).floor();
    if row_f < 0. {
        return DiffPos { row: 0, offset: 0 };
    }
    let row = (row_f as usize).min(count - 1);
    let Some((doc, doc_line)) = view.row_doc_and_line_for_side(row, side) else {
        return DiffPos { row, offset: 0 };
    };
    let x = at.x - metrics.side_text_left_abs(side) + view.scroll.x;
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
            let keep_side = event.modifiers.shift && view.selection.is_some();
            if metrics.mode == DiffViewMode::Split && !keep_side {
                view.selection_side = metrics.side_at_x(event.position.x);
            }
            let side = view.selection_side;
            let at = position_at(view, &metrics, event.position, side);
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
            let side = view.selection_side;
            let mut delta = Point::default();
            if event.position.y < bounds.top() {
                delta.y = bounds.top() - event.position.y;
            } else if event.position.y > bounds.bottom() {
                delta.y = bounds.bottom() - event.position.y;
            }
            let text_left = metrics.side_text_left_abs(side);
            let side_right = metrics.side_right_abs(side);
            if event.position.x < text_left {
                delta.x = text_left - event.position.x;
            } else if event.position.x > side_right {
                delta.x = side_right - event.position.x;
            }
            if delta != Point::default() {
                view.scroll_by(delta, cx);
            }
            let head = position_at(view, &metrics, event.position, side);
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
