use super::*;

pub(crate) struct UnifiedDiffView {
    pub(super) old_doc: Arc<Document>,
    pub(super) new_doc: Arc<Document>,
    pub(super) mode: DiffViewMode,
    show_viewed_footer: bool,
    pub(super) rows: Arc<Vec<DiffRow>>,
    hunk_rows: Vec<usize>,
    pub(super) split_rows: Arc<Vec<SplitRow>>,
    split_hunk_rows: Vec<usize>,
    unified_max_columns: usize,
    pub(super) old_digits: usize,
    pub(super) new_digits: usize,
    grammar: Option<&'static Grammar>,
    pub(super) old_highlights: Option<Arc<Highlights>>,
    pub(super) new_highlights: Option<Arc<Highlights>>,
    cancel: Arc<AtomicUsize>,
    pub(super) focus: FocusHandle,
    pub(super) scroll: Point<Pixels>,
    pub(super) cursor_row: usize,
    reveal_row: Option<usize>,
    pub(super) selection: Option<(DiffPos, DiffPos)>,
    pub(super) selection_side: DiffSide,
    pub(super) selects_all: bool,
    pub(super) dragging: bool,
    file_name: SharedString,
    viewed: bool,
    changed_since_viewed: bool,
    pub(super) metrics: Cell<Option<Metrics>>,
    bar_metrics: Rc<Cell<(ScrollbarMetrics, ScrollbarMetrics)>>,
    vertical_bar: Entity<Scrollbar>,
    horizontal_bar: Entity<Scrollbar>,
    on_escape: Option<DiffCallback>,
    on_toggle_viewed: Option<DiffCallback>,
    pub(super) on_focus_in: Option<DiffCallback>,
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
) -> (Vec<SplitRow>, Vec<usize>) {
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

    (rows, hunk_rows)
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
        let (split_rows, split_hunk_rows) = build_split_rows(&old_doc, &new_doc, hunks);
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
        let (split_rows, split_hunk_rows) = build_split_rows(&old_doc, &new_doc, hunks);
        self.old_doc = old_doc;
        self.new_doc = new_doc;
        self.rows = Arc::new(rows);
        self.hunk_rows = hunk_rows;
        self.split_rows = Arc::new(split_rows);
        self.split_hunk_rows = split_hunk_rows;
        self.unified_max_columns = unified_max_columns;
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

    pub(super) fn active_row_count(&self) -> usize {
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

    pub(super) fn row_doc_and_line_for_side(
        &self,
        row: usize,
        side: DiffSide,
    ) -> Option<(&Document, usize)> {
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

    pub(super) fn row_line_text(&self, row: usize) -> Option<&str> {
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
            let visible =
                ((f32::from(metrics.viewport().size.height) / line_h).ceil() as usize).max(1);
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

    pub(super) fn content_height(&self, metrics: &Metrics) -> Pixels {
        metrics.line_height * self.active_row_count() as f32 + px(TOP_PAD + self.bottom_extra())
    }

    /// The text width and overflow past it for the horizontal scrollbar and
    /// clamp. Split's two sides have their own text and width and scroll
    /// together, so whichever overflows further governs.
    fn horizontal_extent(&self, metrics: &Metrics) -> (Pixels, Pixels) {
        let width = |columns: usize| metrics.char_width * columns as f32 + px(TEXT_PAD_RIGHT);
        match metrics.mode {
            DiffViewMode::Unified => {
                let viewport = metrics.scroll_viewport_width();
                (viewport, width(self.unified_max_columns) - viewport)
            }
            DiffViewMode::Split => {
                let left_vp = metrics.left_text_width();
                let left_overflow = width(self.old_doc.columns()) - left_vp;
                let right_vp = metrics.right_text_width();
                let right_overflow = width(self.new_doc.columns()) - right_vp;
                if left_overflow > right_overflow {
                    (left_vp, left_overflow)
                } else {
                    (right_vp, right_overflow)
                }
            }
        }
    }

    pub(super) fn overflow_x(&self, metrics: &Metrics) -> Pixels {
        self.horizontal_extent(metrics).1
    }

    pub(super) fn measure_scrollbars(&self, metrics: &Metrics) {
        let (viewport_width, overflow_x) = self.horizontal_extent(metrics);
        self.bar_metrics.set((
            ScrollbarMetrics {
                viewport: f32::from(metrics.viewport().size.height),
                content: f32::from(self.content_height(metrics)),
                position: f32::from(self.scroll.y),
            },
            ScrollbarMetrics {
                viewport: f32::from(viewport_width),
                content: f32::from(viewport_width + overflow_x),
                position: f32::from(self.scroll.x),
            },
        ));
    }

    pub(super) fn clamp_scroll(&mut self, metrics: &Metrics) {
        let max_x = self.overflow_x(metrics).max(px(0.));
        let max_y = (self.content_height(metrics) - metrics.viewport().size.height).max(px(0.));
        self.scroll.x = self.scroll.x.clamp(px(0.), max_x);
        self.scroll.y = self.scroll.y.clamp(px(0.), max_y);
    }

    pub(super) fn apply_reveal(&mut self, metrics: &Metrics) {
        let Some(row) = self.reveal_row.take() else {
            return;
        };
        let top = metrics.line_height * row as f32 + px(TOP_PAD);
        let height = metrics.viewport().size.height;
        if top < self.scroll.y {
            self.scroll.y = (top - px(TOP_PAD)).max(px(0.));
        } else if top + metrics.line_height * 2. > self.scroll.y + height {
            self.scroll.y = (top + metrics.line_height * 2. - height).max(px(0.));
        }
    }

    pub(super) fn scroll_by(&mut self, delta: Point<Pixels>, cx: &mut Context<Self>) {
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
                    let page = metrics.viewport().size.height - metrics.line_height;
                    self.scroll_by(point(px(0.), page), cx);
                }
            }
            "pagedown" => {
                if let Some(metrics) = self.metrics.get() {
                    cx.stop_propagation();
                    let page = metrics.viewport().size.height - metrics.line_height;
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
        )
        .fixed_width();
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
        let lanes = self
            .metrics
            .get()
            .map_or_else(Default::default, |metrics| metrics.lanes);
        let footer_top = px(TOP_PAD) + line_height * self.active_row_count() as f32 - self.scroll.y;
        let show_footer = self.show_viewed_footer
            && self.metrics.get().is_none_or(|metrics| {
                footer_top < metrics.viewport().size.height
                    && footer_top + px(FOOTER_HEIGHT) > px(0.)
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
                        .absolute()
                        .top_0()
                        .left_0()
                        .right(lanes.right)
                        .bottom(lanes.bottom)
                        .overflow_hidden()
                        .child(
                            div()
                                .id("changes-diff-mark-viewed")
                                .when(cfg!(test), |element| {
                                    element.debug_selector(|| "CHANGES_DIFF_VIEWED".into())
                                })
                                .absolute()
                                .left_0()
                                .right_0()
                                .top(footer_top)
                                .h(px(FOOTER_HEIGHT))
                                .px(px(12.))
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .border_t_1()
                                .border_color(theme::border())
                                .cursor_pointer()
                                .hover(|row| {
                                    row.bg(theme::with_alpha(theme::sidebar_selected(), 0.35))
                                })
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
                    .right(lanes.right)
                    .child(self.horizontal_bar.clone()),
            )
    }
}
