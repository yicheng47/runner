//! Read-only borrowed view over a terminal screen snapshot.
//!
//! `ScreenData` is an immutable, flat-buffer representation built from
//! `libghostty-vt` (`GhosttyRenderState` for viewport-only frames or
//! `GhosttyTerminal` grid refs for full-scrollback inspection). Painting borrows
//! a published `Arc<ViewportSnapshot>` directly with zero per-frame copies and
//! zero actor thread blocking.

use std::sync::Arc;

use crate::palette::{self, Color, NamedColor, Rgb, ScreenColors, TerminalPalette};
use crate::types::{
    point_to_viewport, CellFlags, Column, CursorShape, CursorStyle, GridPoint, Hyperlink, Line,
    RenderCursor, SelectionRange, TerminalModeFlags,
};
use crate::vt::{
    self, CellContentTag, CellInfo, GridPointTag, RenderCursorInfo, RenderState, Style, StyleColor,
};

#[derive(Clone, Copy, Debug)]
struct ScreenCell {
    c: char,
    flags: CellFlags,
    fg: Color,
    bg: Color,
    underline_color: Option<Color>,
    /// Start index and length into `ScreenData::zerowidth_pool`.
    zw_start: u32,
    zw_len: u16,
    /// 1-based index into `ScreenData::hyperlinks` (`0` means no hyperlink).
    hyperlink: u32,
    selected: bool,
}

impl Default for ScreenCell {
    fn default() -> Self {
        Self {
            c: ' ',
            flags: CellFlags::empty(),
            fg: Color::Named(NamedColor::Foreground),
            bg: Color::Named(NamedColor::Background),
            underline_color: None,
            zw_start: 0,
            zw_len: 0,
            hyperlink: 0,
            selected: false,
        }
    }
}

/// Immutable owned terminal grid snapshot backing [`ScreenView`] and
/// [`ViewportSnapshot`].
#[derive(Clone)]
pub(crate) struct ScreenData {
    cols: usize,
    rows: usize,
    history_size: usize,
    display_offset: usize,
    /// Grid `Line` represented by row 0 of `cells`.
    top_line: Line,
    stored_rows: usize,
    cells: Vec<ScreenCell>,
    zerowidth_pool: Vec<char>,
    hyperlinks: Vec<Hyperlink>,
    colors: ScreenColors,
    mode: TerminalModeFlags,
    cursor: RenderCursor,
    cursor_style: CursorStyle,
    raw_cursor_point: GridPoint,
    cursor_input_needs_wrap: bool,
    selection: Option<SelectionRange>,
    /// When built from `RenderState`, `ScreenCell::selected` comes straight from
    /// Ghostty's render state; otherwise selection membership falls back to
    /// `self.selection`.
    cell_selection_baked: bool,
}

impl ScreenData {
    pub(crate) fn empty(cols: usize, rows: usize) -> Self {
        let cols = cols.max(1);
        let rows = rows.max(1);
        Self {
            cols,
            rows,
            history_size: 0,
            display_offset: 0,
            top_line: Line(0),
            stored_rows: rows,
            cells: vec![ScreenCell::default(); cols * rows],
            zerowidth_pool: Vec::new(),
            hyperlinks: Vec::new(),
            colors: ScreenColors::default(),
            mode: TerminalModeFlags::SHOW_CURSOR | TerminalModeFlags::LINE_WRAP,
            cursor: RenderCursor {
                point: GridPoint::default(),
                shape: CursorShape::Block,
                blinking: true,
            },
            cursor_style: CursorStyle {
                shape: CursorShape::Block,
                blinking: true,
            },
            raw_cursor_point: GridPoint::default(),
            cursor_input_needs_wrap: false,
            selection: None,
            cell_selection_baked: true,
        }
    }

    /// Fast pure-Rust check whether any cell in this snapshot carries
    /// non-whitespace, non-control visible text.
    pub(crate) fn has_visible_text(&self) -> bool {
        self.cells
            .iter()
            .any(|cell| !cell.c.is_whitespace() && !cell.c.is_control())
    }

    /// Pure-Rust equivalent of `vt::Terminal::active_visible_text_state()` over
    /// the stored viewport rows (`0..self.rows`). Callers only use this when
    /// `self.display_offset == 0` so the viewport coincides with the active screen.
    pub(crate) fn visible_text_state(&self) -> (bool, u64) {
        const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
        #[inline]
        fn mix_u32(hash: &mut u64, value: u32) {
            for b in value.to_le_bytes() {
                *hash ^= u64::from(b);
                *hash = hash.wrapping_mul(FNV_PRIME);
            }
        }

        let mut has_visible = false;
        let mut hash = FNV_OFFSET;
        mix_u32(&mut hash, self.cols as u32);
        mix_u32(&mut hash, self.rows as u32);

        let base_row = self.stored_rows.saturating_sub(self.rows);
        for row_off in 0..self.rows {
            let row_start = (base_row + row_off) * self.cols;
            let row_cells = &self.cells[row_start..row_start + self.cols];
            if let Some(end_x) = row_cells.iter().rposition(|cell| cell.c != ' ') {
                for cell in &row_cells[..=end_x] {
                    mix_u32(&mut hash, cell.c as u32);
                    if !has_visible && !cell.c.is_whitespace() && !cell.c.is_control() {
                        has_visible = true;
                    }
                }
            }
            mix_u32(&mut hash, u32::MAX);
        }
        (has_visible, hash)
    }

    /// Build a viewport-only snapshot from an already-updated `RenderState` and
    /// its owning `vt::Terminal`. Only iterates the visible viewport rows
    /// (`O(viewport_cells)`), keeping paint-path publication fast regardless of
    /// scrollback depth.
    pub(crate) fn from_render_state(
        term: &vt::Terminal,
        render_state: &mut RenderState,
        default_cursor_hidden: bool,
    ) -> Self {
        let cols = usize::from(term.cols()).max(1);
        let rows = usize::from(term.rows()).max(1);
        let history_size = term.scrollback_rows();
        let display_offset = term.display_offset_from_bottom();
        let mode = TerminalModeFlags::from_vt(term);
        let (cx, cy) = term.cursor_pos();
        let raw_cursor_point = GridPoint::new(Line(cy as i32), Column(cx as usize));
        let cursor_input_needs_wrap = term.cursor_pending_wrap();
        let rs_cursor = render_state.cursor().ok();
        let (cursor_style, cursor) =
            resolve_cursors(raw_cursor_point, mode, rs_cursor, default_cursor_hidden);
        let selection = active_selection_range(term, history_size);
        let colors = ScreenColors::from_vt(term);
        let top_line = Line(-(display_offset as i32));

        let mut cells = Vec::with_capacity(cols * rows);
        let mut zerowidth_pool = Vec::new();
        let mut hyperlinks = Vec::new();

        if let Ok(render_rows) = render_state.collect_rows() {
            for (row_idx, rrow) in render_rows.into_iter().take(rows).enumerate() {
                let row_links = if rrow.info.hyperlink {
                    extract_row_hyperlinks(term, GridPointTag::Viewport, row_idx as u32, cols)
                } else {
                    Vec::new()
                };
                for col in 0..cols {
                    if let Some(rcell) = rrow.cells.get(col) {
                        let wrapline = rrow.info.wrap && col + 1 == cols;
                        let (c, zw_start, zw_len) =
                            extract_cell_text(&rcell.info, &rcell.graphemes, &mut zerowidth_pool);
                        let hyperlink = if rcell.info.has_hyperlink {
                            let uri = term
                                .grid_ref(GridPointTag::Viewport, col as u16, row_idx as u32)
                                .and_then(|gref| gref.hyperlink_uri().ok().flatten());
                            intern_hyperlink(uri, col, &row_links, &mut hyperlinks)
                        } else {
                            0
                        };
                        cells.push(ScreenCell {
                            c,
                            flags: CellFlags::from_vt_style(
                                rcell.style.flags,
                                rcell.info.wide,
                                wrapline,
                            ),
                            fg: style_color_to_runner(rcell.style.fg, NamedColor::Foreground),
                            bg: cell_bg_to_runner(&rcell.info, &rcell.style),
                            underline_color: style_underline_color_to_runner(rcell.style.underline),
                            zw_start,
                            zw_len,
                            hyperlink,
                            selected: rcell.selected,
                        });
                    } else {
                        cells.push(ScreenCell::default());
                    }
                }
            }
        }
        if cells.len() < cols * rows {
            cells.resize(cols * rows, ScreenCell::default());
        }

        Self {
            cols,
            rows,
            history_size,
            display_offset,
            top_line,
            stored_rows: rows,
            cells,
            zerowidth_pool,
            hyperlinks,
            colors,
            mode,
            cursor,
            cursor_style,
            raw_cursor_point,
            cursor_input_needs_wrap,
            selection,
            cell_selection_baked: true,
        }
    }

    /// Build a snapshot covering only the active screen (`0..rows`) directly
    /// from `vt::Terminal` — used by `InputTracker` and `input_rows` where
    /// scrollback history is never inspected.
    pub(crate) fn from_terminal_active(
        term: &vt::Terminal,
        rs_cursor: Option<RenderCursorInfo>,
        default_cursor_hidden: bool,
    ) -> Self {
        Self::from_terminal_range(term, rs_cursor, default_cursor_hidden, false)
    }

    /// Build a full-grid snapshot covering scrollback + active screen
    /// (`-history_size..rows`) directly from `vt::Terminal`.
    pub(crate) fn from_terminal_full(
        term: &vt::Terminal,
        rs_cursor: Option<RenderCursorInfo>,
        default_cursor_hidden: bool,
    ) -> Self {
        Self::from_terminal_range(term, rs_cursor, default_cursor_hidden, true)
    }

    fn from_terminal_range(
        term: &vt::Terminal,
        rs_cursor: Option<RenderCursorInfo>,
        default_cursor_hidden: bool,
        include_history: bool,
    ) -> Self {
        let cols = usize::from(term.cols()).max(1);
        let rows = usize::from(term.rows()).max(1);
        let history_size = term.scrollback_rows();
        let display_offset = term.display_offset_from_bottom();
        let mode = TerminalModeFlags::from_vt(term);
        let (cx, cy) = term.cursor_pos();
        let raw_cursor_point = GridPoint::new(Line(cy as i32), Column(cx as usize));
        let cursor_input_needs_wrap = term.cursor_pending_wrap();
        let (cursor_style, cursor) =
            resolve_cursors(raw_cursor_point, mode, rs_cursor, default_cursor_hidden);
        let selection = active_selection_range(term, history_size);
        let colors = ScreenColors::from_vt(term);

        let (tag, start_y, stored_rows, top_line) = if include_history {
            (
                GridPointTag::Screen,
                0usize,
                history_size + rows,
                Line(-(history_size as i32)),
            )
        } else {
            (GridPointTag::Active, 0usize, rows, Line(0))
        };

        let mut cells = Vec::with_capacity(cols * stored_rows);
        let mut zerowidth_pool = Vec::new();
        let mut hyperlinks = Vec::new();

        for row_idx in 0..stored_rows {
            let y = (start_y + row_idx) as u32;
            let row_info = term.grid_ref(tag, 0, y).and_then(|gref| gref.row().ok());
            let row_wrap = row_info.is_some_and(|r| r.wrap);
            let row_has_links = row_info.is_some_and(|r| r.hyperlink);
            let row_links = if row_has_links {
                extract_row_hyperlinks(term, tag, y, cols)
            } else {
                Vec::new()
            };
            for col in 0..cols {
                let Some(gref) = term.grid_ref(tag, col as u16, y) else {
                    cells.push(ScreenCell::default());
                    continue;
                };
                let Ok(info) = gref.cell() else {
                    cells.push(ScreenCell::default());
                    continue;
                };
                let style = if info.has_styling {
                    gref.style().unwrap_or(Style {
                        fg: StyleColor::None,
                        bg: StyleColor::None,
                        underline: StyleColor::None,
                        flags: 0,
                    })
                } else {
                    Style {
                        fg: StyleColor::None,
                        bg: StyleColor::None,
                        underline: StyleColor::None,
                        flags: 0,
                    }
                };
                let graphemes = if info.content_tag == CellContentTag::CodepointGrapheme {
                    gref.graphemes().unwrap_or_default()
                } else {
                    Vec::new()
                };
                let (c, zw_start, zw_len) =
                    extract_cell_text(&info, &graphemes, &mut zerowidth_pool);
                let hyperlink = if info.has_hyperlink {
                    let uri = gref.hyperlink_uri().ok().flatten();
                    intern_hyperlink(uri, col, &row_links, &mut hyperlinks)
                } else {
                    0
                };
                let wrapline = row_wrap && col + 1 == cols;
                cells.push(ScreenCell {
                    c,
                    flags: CellFlags::from_vt_style(style.flags, info.wide, wrapline),
                    fg: style_color_to_runner(style.fg, NamedColor::Foreground),
                    bg: cell_bg_to_runner(&info, &style),
                    underline_color: style_underline_color_to_runner(style.underline),
                    zw_start,
                    zw_len,
                    hyperlink,
                    selected: false,
                });
            }
        }

        Self {
            cols,
            rows,
            history_size,
            display_offset,
            top_line,
            stored_rows,
            cells,
            zerowidth_pool,
            hyperlinks,
            colors,
            mode,
            cursor,
            cursor_style,
            raw_cursor_point,
            cursor_input_needs_wrap,
            selection,
            cell_selection_baked: false,
        }
    }

    #[inline]
    pub(crate) fn view(&self) -> ScreenView<'_> {
        ScreenView { data: self }
    }

    #[inline]
    fn row_index(&self, line: Line) -> Option<usize> {
        let offset = line.0.checked_sub(self.top_line.0)?;
        let idx = usize::try_from(offset).ok()?;
        (idx < self.stored_rows).then_some(idx)
    }

    #[inline]
    fn cell_at(&self, line: Line, col: usize) -> Option<&ScreenCell> {
        if col >= self.cols {
            return None;
        }
        let row = self.row_index(line)?;
        self.cells.get(row * self.cols + col)
    }

    #[inline]
    fn cell_view(&self, cell: &ScreenCell) -> CellView<'_> {
        CellView {
            cell: *cell,
            data: self,
        }
    }
}

/// Immutable, `Send + Sync` viewport snapshot published by `TerminalMirror`'s
/// actor thread so GPUI can paint without blocking on the VT engine or copying
/// cells per frame.
pub struct ViewportSnapshot(pub(crate) ScreenData);

impl ViewportSnapshot {
    #[inline]
    pub fn view(&self) -> ScreenView<'_> {
        self.0.view()
    }

    #[inline]
    pub fn cols(&self) -> usize {
        self.0.cols
    }

    #[inline]
    pub fn rows(&self) -> usize {
        self.0.rows
    }

    #[inline]
    pub fn history_size(&self) -> usize {
        self.0.history_size
    }

    #[inline]
    pub fn display_offset(&self) -> usize {
        self.0.display_offset
    }

    #[inline]
    pub fn mode(&self) -> TerminalModeFlags {
        self.0.mode
    }

    #[inline]
    pub fn cursor_style(&self) -> CursorStyle {
        self.0.cursor_style
    }

    #[inline]
    pub fn selection(&self) -> Option<SelectionRange> {
        self.0.selection
    }
}

/// Borrowed view of one terminal cell.
#[derive(Clone, Copy)]
pub struct CellView<'a> {
    cell: ScreenCell,
    data: &'a ScreenData,
}

impl<'a> CellView<'a> {
    #[inline]
    pub fn c(self) -> char {
        self.cell.c
    }

    #[inline]
    pub fn zerowidth(self) -> Option<&'a [char]> {
        if self.cell.zw_len == 0 {
            return None;
        }
        let start = self.cell.zw_start as usize;
        let end = start + usize::from(self.cell.zw_len);
        self.data.zerowidth_pool.get(start..end)
    }

    #[inline]
    pub fn flags(self) -> CellFlags {
        self.cell.flags
    }

    #[inline]
    pub fn fg(self) -> Color {
        self.cell.fg
    }

    #[inline]
    pub fn bg(self) -> Color {
        self.cell.bg
    }

    #[inline]
    pub fn underline_color(self) -> Option<Color> {
        self.cell.underline_color
    }

    #[inline]
    pub fn hyperlink(self) -> Option<Hyperlink> {
        let idx = usize::try_from(self.cell.hyperlink).ok()?.checked_sub(1)?;
        self.data.hyperlinks.get(idx).cloned()
    }
}

/// Borrowed visible cell yielded while iterating the viewport.
#[derive(Clone, Copy)]
pub struct ViewportCell<'a> {
    pub point: GridPoint,
    pub viewport_row: usize,
    pub viewport_col: usize,
    pub selected: bool,
    cell: CellView<'a>,
}

impl<'a> ViewportCell<'a> {
    #[inline]
    pub fn cell(self) -> CellView<'a> {
        self.cell
    }

    #[inline]
    pub fn c(self) -> char {
        self.cell.c()
    }

    #[inline]
    pub fn zerowidth(self) -> Option<&'a [char]> {
        self.cell.zerowidth()
    }

    #[inline]
    pub fn flags(self) -> CellFlags {
        self.cell.flags()
    }

    #[inline]
    pub fn fg(self) -> Color {
        self.cell.fg()
    }

    #[inline]
    pub fn bg(self) -> Color {
        self.cell.bg()
    }

    #[inline]
    pub fn underline_color(self) -> Option<Color> {
        self.cell.underline_color()
    }

    #[inline]
    pub fn hyperlink(self) -> Option<Hyperlink> {
        self.cell.hyperlink()
    }
}

/// Zero-allocation iterator yielding visible cells in row-major order.
pub struct VisibleCells<'a> {
    data: &'a ScreenData,
    viewport_row: usize,
    viewport_col: usize,
}

impl<'a> Iterator for VisibleCells<'a> {
    type Item = ViewportCell<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        while self.viewport_row < self.data.rows {
            let row = self.viewport_row;
            let col = self.viewport_col;
            self.viewport_col += 1;
            if self.viewport_col >= self.data.cols {
                self.viewport_col = 0;
                self.viewport_row += 1;
            }
            let line = Line(row as i32 - self.data.display_offset as i32);
            let point = GridPoint::new(line, Column(col));
            let Some(vp) = point_to_viewport(self.data.display_offset, point) else {
                continue;
            };
            let Some(raw) = self.data.cell_at(line, col) else {
                continue;
            };
            let selected = if self.data.cell_selection_baked {
                raw.selected
            } else {
                self.data
                    .selection
                    .is_some_and(|range| range.contains(point))
            };
            return Some(ViewportCell {
                point,
                viewport_row: vp.line,
                viewport_col: vp.column.0,
                selected,
                cell: self.data.cell_view(raw),
            });
        }
        None
    }
}

/// Borrowed read-only view of a terminal's active screen, modes, cursor, and colors.
#[derive(Clone, Copy)]
pub struct ScreenView<'a> {
    data: &'a ScreenData,
}

impl<'a> ScreenView<'a> {
    #[inline]
    pub fn cols(&self) -> usize {
        self.data.cols
    }

    #[inline]
    pub fn rows(&self) -> usize {
        self.data.rows
    }

    #[inline]
    pub fn history_size(&self) -> usize {
        self.data.history_size
    }

    #[inline]
    pub fn display_offset(&self) -> usize {
        self.data.display_offset
    }

    #[inline]
    pub fn topmost_line(&self) -> Line {
        Line(-(self.data.history_size as i32))
    }

    #[inline]
    pub fn bottommost_line(&self) -> Line {
        Line(self.data.rows as i32 - 1)
    }

    #[inline]
    pub fn mode(&self) -> TerminalModeFlags {
        self.data.mode
    }

    #[inline]
    pub fn cursor(&self) -> RenderCursor {
        self.data.cursor
    }

    #[inline]
    pub fn cursor_style(&self) -> CursorStyle {
        self.data.cursor_style
    }

    #[inline]
    pub fn raw_cursor_point(&self) -> GridPoint {
        self.data.raw_cursor_point
    }

    #[inline]
    pub fn cursor_input_needs_wrap(&self) -> bool {
        self.data.cursor_input_needs_wrap
    }

    #[inline]
    pub fn selection(&self) -> Option<SelectionRange> {
        self.data.selection
    }

    #[inline]
    pub fn color_override(&self, index: usize) -> Option<Rgb> {
        self.data.colors.get(index)
    }

    #[inline]
    pub fn resolve_color(&self, color: Color, base: &[Rgb; 256], theme: TerminalPalette) -> Rgb {
        palette::resolve_with_overrides(color, &self.data.colors, base, theme)
    }

    #[inline]
    pub fn query_color(&self, index: usize, base: &[Rgb; 256], theme: TerminalPalette) -> Rgb {
        self.color_override(index)
            .unwrap_or_else(|| palette::resolve_index_for(index, base, theme))
    }

    #[inline]
    pub fn visible_cells(&self) -> VisibleCells<'a> {
        VisibleCells {
            data: self.data,
            viewport_row: 0,
            viewport_col: 0,
        }
    }

    #[inline]
    pub fn cell(&self, point: GridPoint) -> Option<CellView<'a>> {
        self.data
            .cell_at(point.line, point.column.0)
            .map(|raw| self.data.cell_view(raw))
    }

    /// Preserve trailing space cells for probes that join a wrapped line.
    pub fn row_cells(&self, line: Line) -> String {
        let mut s = String::with_capacity(self.data.cols);
        for col in 0..self.data.cols {
            let Some(cell) = self.data.cell_at(line, col) else {
                break;
            };
            if cell
                .flags
                .intersects(CellFlags::WIDE_CHAR_SPACER | CellFlags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            s.push(cell.c);
            if let Some(zw) = self.data.cell_view(cell).zerowidth() {
                s.extend(zw.iter());
            }
        }
        s
    }

    /// One visible row as a string: wide chars keep their glyph, spacer
    /// cells are skipped, zero-width combining marks are re-attached.
    pub fn row_to_string(&self, line: Line) -> String {
        let mut s = self.row_cells(line);
        s.truncate(s.trim_end().len());
        s
    }

    pub fn visible_lines(&self) -> Vec<String> {
        (0..self.rows() as i32)
            .map(|l| self.row_to_string(Line(l)))
            .collect()
    }

    /// Deterministic textual snapshot of the visible screen, compared
    /// against `.snapshot.txt` files in the fixture corpus.
    pub fn screen_snapshot(&self) -> String {
        let cursor = self.data.raw_cursor_point;
        let mut out = format!(
            "cols={} rows={} cursor={},{}\n",
            self.cols(),
            self.rows(),
            cursor.line.0,
            cursor.column.0
        );
        for line in self.visible_lines() {
            out.push('|');
            out.push_str(&line);
            out.push('\n');
        }
        out
    }

    /// Convenience row cell iterator for internal callers (`InputTracker`, etc.).
    #[inline]
    pub fn row_cell(&self, line: Line, column: Column) -> Option<CellView<'a>> {
        self.cell(GridPoint::new(line, column))
    }

    #[inline]
    pub(crate) fn clamp_grid_point(&self, point: GridPoint) -> GridPoint {
        let top = self.topmost_line();
        let bottom = self.bottommost_line();
        let last_col = self.cols().saturating_sub(1);
        if point.line < top {
            GridPoint::new(top, Column(0))
        } else if point.line > bottom {
            GridPoint::new(bottom, Column(last_col))
        } else {
            GridPoint::new(point.line, Column(point.column.0.min(last_col)))
        }
    }

    #[inline]
    pub(crate) fn soft_wrapped_line_bounds(&self, line: Line) -> (Line, Line) {
        let last_col = Column(self.cols().saturating_sub(1));
        let top = self.data.top_line;
        let bottom = Line(self.data.top_line.0 + self.data.stored_rows as i32 - 1);
        let mut start = line.clamp(top, bottom);
        while start > top
            && self
                .row_cell(start - 1, last_col)
                .is_some_and(|cell| cell.flags().contains(CellFlags::WRAPLINE))
        {
            start -= 1;
        }
        let mut end = line.clamp(top, bottom);
        while end < bottom
            && self
                .row_cell(end, last_col)
                .is_some_and(|cell| cell.flags().contains(CellFlags::WRAPLINE))
        {
            end += 1;
        }
        (start, end)
    }

    /// Extract the text spanning inclusive grid coordinates `start..=end`,
    /// joining soft-wrapped rows without newlines and trimming trailing spaces
    /// on non-wrapped rows.
    pub fn bounds_to_string(&self, start: GridPoint, end: GridPoint) -> String {
        if start > end || self.cols() == 0 {
            return String::new();
        }
        let last_col = self.cols() - 1;
        let mut res = String::new();
        for line_i in start.line.0..=end.line.0 {
            let line = Line(line_i);
            let mut start_col = if line == start.line {
                start.column.0.min(last_col)
            } else {
                0
            };
            let end_col = if line == end.line {
                end.column.0.min(last_col)
            } else {
                last_col
            };
            if start_col > 0
                && self
                    .data
                    .cell_at(line, start_col)
                    .is_some_and(|c| c.flags.contains(CellFlags::WIDE_CHAR_SPACER))
            {
                start_col -= 1;
            }
            let row_wrapped = self
                .data
                .cell_at(line, last_col)
                .is_some_and(|c| c.flags.contains(CellFlags::WRAPLINE));
            let line_len = self.row_effective_length(line, row_wrapped);
            let limit = line_len.min(end_col + 1);
            for col in start_col..limit {
                let Some(cell) = self.data.cell_at(line, col) else {
                    break;
                };
                if cell
                    .flags
                    .intersects(CellFlags::WIDE_CHAR_SPACER | CellFlags::LEADING_WIDE_CHAR_SPACER)
                {
                    continue;
                }
                res.push(cell.c);
                if let Some(zw) = self.data.cell_view(cell).zerowidth() {
                    res.extend(zw.iter());
                }
            }
            // Include the trailing glyph if a wide char's left half was at `end_col`
            // and `end_col == last_col` didn't clip it; already pushed above.
            if !row_wrapped && (line < end.line || end_col >= line_len) {
                res.push('\n');
            }
        }
        res.strip_suffix('\n').map(str::to_owned).unwrap_or(res)
    }

    fn row_effective_length(&self, line: Line, wrapped: bool) -> usize {
        if wrapped {
            return self.data.cols;
        }
        let mut len = self.data.cols;
        while len > 0 {
            let Some(cell) = self.data.cell_at(line, len - 1) else {
                break;
            };
            if cell.c == ' '
                && cell.zw_len == 0
                && !cell
                    .flags
                    .intersects(CellFlags::WIDE_CHAR_SPACER | CellFlags::LEADING_WIDE_CHAR_SPACER)
            {
                len -= 1;
            } else {
                break;
            }
        }
        len
    }

    #[inline]
    pub(crate) fn data(&self) -> &'a ScreenData {
        self.data
    }
}

/// Borrowed handle to a terminal screen captured inside `with_screen`.
pub struct ScreenGuard<'a> {
    data: ScreenData,
    #[cfg(any(test, feature = "test-support"))]
    snapshot_state_fn: Option<Box<dyn Fn() -> crate::snapshot::TerminalSnapshotState + 'a>>,
    #[cfg(not(any(test, feature = "test-support")))]
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a> ScreenGuard<'a> {
    pub(crate) fn from_data(
        data: ScreenData,
        #[cfg(any(test, feature = "test-support"))] snapshot_state_fn: Option<
            Box<dyn Fn() -> crate::snapshot::TerminalSnapshotState + 'a>,
        >,
    ) -> Self {
        Self {
            data,
            #[cfg(any(test, feature = "test-support"))]
            snapshot_state_fn,
            #[cfg(not(any(test, feature = "test-support")))]
            _marker: std::marker::PhantomData,
        }
    }

    #[inline]
    pub fn cols(&self) -> usize {
        self.data.cols
    }

    #[inline]
    pub fn rows(&self) -> usize {
        self.data.rows
    }

    #[inline]
    pub fn display_offset(&self) -> usize {
        self.data.display_offset
    }

    #[inline]
    pub fn mode(&self) -> TerminalModeFlags {
        self.data.mode
    }

    #[inline]
    pub fn view(&self) -> ScreenView<'_> {
        self.data.view()
    }

    pub fn bounds_to_string(&self, start: GridPoint, end: GridPoint) -> String {
        self.view().bounds_to_string(start, end)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn snapshot_state(&self) -> crate::snapshot::TerminalSnapshotState {
        self.snapshot_state_fn
            .as_ref()
            .expect("ScreenGuard constructed without snapshot_state provider")()
    }
}

fn resolve_cursors(
    raw_cursor_point: GridPoint,
    mode: TerminalModeFlags,
    rs_cursor: Option<RenderCursorInfo>,
    default_cursor_hidden: bool,
) -> (CursorStyle, RenderCursor) {
    let base_shape = rs_cursor.map_or(CursorShape::Block, |c| c.shape);
    let shape = if default_cursor_hidden && base_shape == CursorShape::Block {
        CursorShape::Hidden
    } else {
        base_shape
    };
    let blinking = rs_cursor.is_none_or(|c| c.blinking);
    let cursor_style = CursorStyle { shape, blinking };
    let visible = rs_cursor.map_or(mode.contains(TerminalModeFlags::SHOW_CURSOR), |c| {
        c.visible && mode.contains(TerminalModeFlags::SHOW_CURSOR)
    });
    let render_cursor = RenderCursor {
        point: raw_cursor_point,
        shape: if visible { shape } else { CursorShape::Hidden },
        blinking,
    };
    (cursor_style, render_cursor)
}

fn active_selection_range(term: &vt::Terminal, history_size: usize) -> Option<SelectionRange> {
    let span = term.active_selection_span(GridPointTag::Screen)?;
    let to_grid = |(x, y): (u16, u32)| {
        GridPoint::new(Line(y as i32 - history_size as i32), Column(usize::from(x)))
    };
    let a = to_grid(span.start);
    let b = to_grid(span.end);
    let (start, end) = if a <= b { (a, b) } else { (b, a) };
    Some(SelectionRange {
        start,
        end,
        is_block: span.rectangle,
    })
}

fn extract_cell_text(
    info: &CellInfo,
    graphemes: &[char],
    pool: &mut Vec<char>,
) -> (char, u32, u16) {
    if matches!(
        info.content_tag,
        CellContentTag::BgColorPalette | CellContentTag::BgColorRgb
    ) {
        return (' ', 0, 0);
    }
    if !graphemes.is_empty() {
        let base = graphemes[0];
        let extra = &graphemes[1..];
        if extra.is_empty() {
            return (base, 0, 0);
        }
        let start = u32::try_from(pool.len()).unwrap_or(u32::MAX);
        let len = u16::try_from(extra.len()).unwrap_or(u16::MAX);
        pool.extend_from_slice(&extra[..usize::from(len)]);
        return (base, start, len);
    }
    let ch = if info.codepoint == 0 {
        ' '
    } else {
        char::from_u32(info.codepoint).unwrap_or(' ')
    };
    (ch, 0, 0)
}

#[inline]
pub(crate) fn style_color_to_runner(color: StyleColor, default: NamedColor) -> Color {
    match color {
        StyleColor::None => Color::Named(default),
        StyleColor::Palette(idx) => Color::Indexed(idx),
        StyleColor::Rgb(rgb) => Color::Rgb(rgb),
    }
}

#[inline]
pub(crate) fn style_underline_color_to_runner(color: StyleColor) -> Option<Color> {
    match color {
        StyleColor::None => None,
        StyleColor::Palette(idx) => Some(Color::Indexed(idx)),
        StyleColor::Rgb(rgb) => Some(Color::Rgb(rgb)),
    }
}

#[inline]
pub(crate) fn cell_bg_to_runner(info: &CellInfo, style: &Style) -> Color {
    match info.content_tag {
        CellContentTag::BgColorPalette => info
            .bg_palette
            .map(Color::Indexed)
            .unwrap_or(Color::Named(NamedColor::Background)),
        CellContentTag::BgColorRgb => info
            .bg_rgb
            .map(Color::Rgb)
            .unwrap_or(Color::Named(NamedColor::Background)),
        _ => style_color_to_runner(style.bg, NamedColor::Background),
    }
}

struct RowHyperlinkSpan {
    col_start: usize,
    col_end: usize,
    id: String,
    uri: String,
}

/// Lazily recover OSC 8 `id=` parameters for a single row that Ghostty marks
/// as containing at least one hyperlink.
fn extract_row_hyperlinks(
    term: &vt::Terminal,
    tag: GridPointTag,
    y: u32,
    cols: usize,
) -> Vec<RowHyperlinkSpan> {
    let Some(bytes) = term.format_row_vt(tag, y) else {
        return Vec::new();
    };
    parse_row_osc8_spans(&bytes, cols)
}

fn intern_hyperlink(
    uri: Option<String>,
    col: usize,
    row_spans: &[RowHyperlinkSpan],
    table: &mut Vec<Hyperlink>,
) -> u32 {
    let Some(uri) = uri else {
        return 0;
    };
    let id = row_spans
        .iter()
        .find(|s| s.uri == uri && col >= s.col_start && col < s.col_end)
        .or_else(|| row_spans.iter().find(|s| s.uri == uri))
        .map_or("", |s| s.id.as_str());
    if let Some(pos) = table
        .iter()
        .position(|existing| existing.id() == id && existing.uri() == uri)
    {
        return (pos + 1) as u32;
    }
    let link = Hyperlink::new(Arc::<str>::from(id), Arc::<str>::from(uri));
    table.push(link);
    table.len() as u32
}

/// Parse OSC 8 hyperlink spans out of a single-row VT sequence produced by
/// `GhosttyFormatter`.
fn parse_row_osc8_spans(bytes: &[u8], cols: usize) -> Vec<RowHyperlinkSpan> {
    let mut spans = Vec::new();
    let mut active: Option<(usize, String, String)> = None;
    let mut col = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            if i + 1 >= bytes.len() {
                break;
            }
            match bytes[i + 1] {
                b'[' => {
                    // CSI: read until final byte 0x40..=0x7E.
                    let start = i + 2;
                    let mut j = start;
                    while j < bytes.len() && !(0x40..=0x7e).contains(&bytes[j]) {
                        j += 1;
                    }
                    if j < bytes.len() {
                        let params = std::str::from_utf8(&bytes[start..j]).unwrap_or("");
                        let first_param = params
                            .split(';')
                            .next()
                            .and_then(|p| p.parse::<usize>().ok());
                        match bytes[j] {
                            b'C' => col = (col + first_param.unwrap_or(1).max(1)).min(cols),
                            b'G' | b'`' => {
                                col = first_param.unwrap_or(1).saturating_sub(1).min(cols)
                            }
                            _ => {}
                        }
                        i = j + 1;
                    } else {
                        break;
                    }
                }
                b']' => {
                    // OSC: scan until BEL (0x07) or ST (ESC \).
                    let body_start = i + 2;
                    let mut j = body_start;
                    let mut next_i = bytes.len();
                    while j < bytes.len() {
                        if bytes[j] == 0x07 {
                            next_i = j + 1;
                            break;
                        }
                        if bytes[j] == 0x1b && bytes.get(j + 1) == Some(&b'\\') {
                            next_i = j + 2;
                            break;
                        }
                        j += 1;
                    }
                    let body = &bytes[body_start..j];
                    if let Some(rest) = body.strip_prefix(b"8;") {
                        if let Some((start_col, id, uri)) = active.take() {
                            if col > start_col {
                                spans.push(RowHyperlinkSpan {
                                    col_start: start_col,
                                    col_end: col,
                                    id,
                                    uri,
                                });
                            }
                        }
                        if let Some(semi) = rest.iter().position(|&b| b == b';') {
                            let params = std::str::from_utf8(&rest[..semi]).unwrap_or("");
                            let uri = std::str::from_utf8(&rest[semi + 1..]).unwrap_or("");
                            if !uri.is_empty() {
                                let id = params
                                    .split(':')
                                    .find_map(|kv| kv.strip_prefix("id="))
                                    .unwrap_or("")
                                    .to_owned();
                                active = Some((col, id, uri.to_owned()));
                            }
                        }
                    }
                    i = next_i;
                }
                b'(' | b')' | b'*' | b'+' => {
                    i = (i + 3).min(bytes.len());
                }
                _ => {
                    i += 2;
                }
            }
            continue;
        }
        if bytes[i] < 0x20 {
            if bytes[i] == b'\r' {
                col = 0;
            }
            i += 1;
            continue;
        }
        let s = match std::str::from_utf8(&bytes[i..]) {
            Ok(s) => s,
            Err(err) => {
                let valid = err.valid_up_to();
                if valid == 0 {
                    i += 1;
                    continue;
                }
                std::str::from_utf8(&bytes[i..i + valid]).unwrap_or("")
            }
        };
        if let Some(ch) = s.chars().next() {
            let w = usize::from(vt::codepoint_width(ch));
            col = (col + w).min(cols);
            i += ch.len_utf8();
        } else {
            i += 1;
        }
    }
    if let Some((start_col, id, uri)) = active.take() {
        spans.push(RowHyperlinkSpan {
            col_start: start_col,
            col_end: col.max(start_col + 1),
            id,
            uri,
        });
    }
    spans
}
