//! Read-only borrowed view over a locked terminal screen.
//! Shaped so the app's paint path borrows Runner-typed cells under the lock it
//! already holds with zero allocation or copy per cell or per frame.

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::{Dimensions as _, Grid, GridIterator};
use alacritty_terminal::index as alacritty_index;
use alacritty_terminal::selection as alacritty_selection;
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::CursorShape as AlacrittyCursorShape;

use crate::palette::{self, Color, Rgb, TerminalPalette};
use crate::terminal::EventProxy;
use crate::types::{
    point_to_viewport, CellFlags, Column, CursorShape, CursorStyle, GridPoint, Hyperlink, Line,
    RenderCursor, SelectionRange, TerminalModeFlags,
};

/// Borrowed view of one terminal cell.
#[derive(Clone, Copy)]
pub struct CellView<'a> {
    cell: &'a Cell,
}

impl<'a> CellView<'a> {
    #[inline]
    pub(crate) fn from_alacritty(cell: &'a Cell) -> Self {
        Self { cell }
    }

    #[inline]
    pub fn c(self) -> char {
        self.cell.c
    }

    #[inline]
    pub fn zerowidth(self) -> Option<&'a [char]> {
        self.cell.zerowidth()
    }

    #[inline]
    pub fn flags(self) -> CellFlags {
        CellFlags::from_alacritty(self.cell.flags)
    }

    #[inline]
    pub fn fg(self) -> Color {
        Color::from_alacritty(self.cell.fg)
    }

    #[inline]
    pub fn bg(self) -> Color {
        Color::from_alacritty(self.cell.bg)
    }

    #[inline]
    pub fn underline_color(self) -> Option<Color> {
        self.cell.underline_color().map(Color::from_alacritty)
    }

    #[inline]
    pub fn hyperlink(self) -> Option<Hyperlink> {
        self.cell.hyperlink().map(Hyperlink::from_alacritty)
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
    iter: GridIterator<'a, Cell>,
    display_offset: usize,
    selection: Option<alacritty_selection::SelectionRange>,
    render_cursor_point: alacritty_index::Point,
}

impl<'a> Iterator for VisibleCells<'a> {
    type Item = ViewportCell<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        for indexed in self.iter.by_ref() {
            let point = GridPoint::from_alacritty(indexed.point);
            let Some(vp) = point_to_viewport(self.display_offset, point) else {
                continue;
            };
            let selected = self.selection.is_some_and(|range| {
                range.contains_cell(
                    &indexed,
                    self.render_cursor_point,
                    AlacrittyCursorShape::Hidden,
                )
            });
            return Some(ViewportCell {
                point,
                viewport_row: vp.line,
                viewport_col: vp.column.0,
                selected,
                cell: CellView::from_alacritty(indexed.cell),
            });
        }
        None
    }
}

/// Borrowed read-only view of a terminal's active screen, modes, cursor, and colors.
#[derive(Clone, Copy)]
pub struct ScreenView<'a> {
    grid: &'a Grid<Cell>,
    colors: &'a Colors,
    mode: TerminalModeFlags,
    cursor: RenderCursor,
    cursor_style: CursorStyle,
    raw_cursor_point: GridPoint,
    cursor_input_needs_wrap: bool,
    selection: Option<alacritty_selection::SelectionRange>,
    render_cursor_point: alacritty_index::Point,
    display_offset: usize,
}

impl<'a> ScreenView<'a> {
    pub(crate) fn from_term_grid<T>(term: &'a Term<T>) -> Self {
        let cursor_style = term.cursor_style();
        let raw_cursor = &term.grid().cursor;
        let mode = TerminalModeFlags::from_alacritty(*term.mode());
        let shape = if mode.contains(TerminalModeFlags::SHOW_CURSOR) {
            CursorShape::from_alacritty(cursor_style.shape)
        } else {
            CursorShape::Hidden
        };
        Self {
            grid: term.grid(),
            colors: term.colors(),
            mode,
            cursor: RenderCursor {
                point: GridPoint::from_alacritty(raw_cursor.point),
                shape,
                blinking: cursor_style.blinking,
            },
            cursor_style: CursorStyle::from_alacritty(cursor_style),
            raw_cursor_point: GridPoint::from_alacritty(raw_cursor.point),
            cursor_input_needs_wrap: raw_cursor.input_needs_wrap,
            selection: None,
            render_cursor_point: raw_cursor.point,
            display_offset: term.grid().display_offset(),
        }
    }

    pub(crate) fn from_term<T: EventListener>(term: &'a Term<T>) -> Self {
        let content = term.renderable_content();
        let cursor_style = term.cursor_style();
        let raw_cursor = &term.grid().cursor;
        Self {
            grid: term.grid(),
            colors: content.colors,
            mode: TerminalModeFlags::from_alacritty(content.mode),
            cursor: RenderCursor {
                point: GridPoint::from_alacritty(content.cursor.point),
                shape: CursorShape::from_alacritty(content.cursor.shape),
                blinking: cursor_style.blinking,
            },
            cursor_style: CursorStyle::from_alacritty(cursor_style),
            raw_cursor_point: GridPoint::from_alacritty(raw_cursor.point),
            cursor_input_needs_wrap: raw_cursor.input_needs_wrap,
            selection: content.selection,
            render_cursor_point: content.cursor.point,
            display_offset: content.display_offset,
        }
    }

    #[inline]
    pub fn cols(&self) -> usize {
        self.grid.columns()
    }

    #[inline]
    pub fn rows(&self) -> usize {
        self.grid.screen_lines()
    }

    #[inline]
    pub fn history_size(&self) -> usize {
        self.grid.history_size()
    }

    #[inline]
    pub fn display_offset(&self) -> usize {
        self.display_offset
    }

    #[inline]
    pub fn topmost_line(&self) -> Line {
        Line::from_alacritty(self.grid.topmost_line())
    }

    #[inline]
    pub fn bottommost_line(&self) -> Line {
        Line::from_alacritty(self.grid.bottommost_line())
    }

    #[inline]
    pub fn mode(&self) -> TerminalModeFlags {
        self.mode
    }

    #[inline]
    pub fn cursor(&self) -> RenderCursor {
        self.cursor
    }

    #[inline]
    pub fn cursor_style(&self) -> CursorStyle {
        self.cursor_style
    }

    #[inline]
    pub fn raw_cursor_point(&self) -> GridPoint {
        self.raw_cursor_point
    }

    #[inline]
    pub fn cursor_input_needs_wrap(&self) -> bool {
        self.cursor_input_needs_wrap
    }

    #[inline]
    pub fn selection(&self) -> Option<SelectionRange> {
        self.selection.map(SelectionRange::from_alacritty)
    }

    #[inline]
    pub fn color_override(&self, index: usize) -> Option<Rgb> {
        (index < alacritty_terminal::term::color::COUNT)
            .then(|| self.colors[index].map(Rgb::from_alacritty))
            .flatten()
    }

    #[inline]
    pub fn resolve_color(&self, color: Color, base: &[Rgb; 256], theme: TerminalPalette) -> Rgb {
        palette::resolve_with_alacritty_colors(color, self.colors, base, theme)
    }

    #[inline]
    pub fn query_color(&self, index: usize, base: &[Rgb; 256], theme: TerminalPalette) -> Rgb {
        self.color_override(index)
            .unwrap_or_else(|| palette::resolve_index_for(index, base, theme))
    }

    #[inline]
    pub fn visible_cells(&self) -> VisibleCells<'a> {
        VisibleCells {
            iter: self.grid.display_iter(),
            display_offset: self.display_offset,
            selection: self.selection,
            render_cursor_point: self.render_cursor_point,
        }
    }

    #[inline]
    pub fn cell(&self, point: GridPoint) -> Option<CellView<'a>> {
        if point.line < self.topmost_line()
            || point.line > self.bottommost_line()
            || point.column.0 >= self.cols()
        {
            return None;
        }
        Some(CellView::from_alacritty(&self.grid[point.to_alacritty()]))
    }

    /// Preserve trailing space cells for probes that join a wrapped line.
    pub fn row_cells(&self, line: Line) -> String {
        let row = &self.grid[line.to_alacritty()];
        let mut s = String::new();
        for col in 0..self.grid.columns() {
            let cell = &row[alacritty_index::Column(col)];
            if cell.flags.intersects(
                alacritty_terminal::term::cell::Flags::WIDE_CHAR_SPACER
                    | alacritty_terminal::term::cell::Flags::LEADING_WIDE_CHAR_SPACER,
            ) {
                continue;
            }
            s.push(cell.c);
            if let Some(zw) = cell.zerowidth() {
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
        let cursor = self.raw_cursor_point;
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
        GridPoint::from_alacritty(
            point
                .to_alacritty()
                .grid_clamp(self.grid, alacritty_index::Boundary::Grid),
        )
    }

    #[inline]
    pub(crate) fn soft_wrapped_line_bounds(&self, line: Line) -> (Line, Line) {
        let last_column = self.grid.last_column();
        let mut start = line.to_alacritty();
        while start > self.grid.topmost_line()
            && self.grid[start - 1i32][last_column]
                .flags
                .contains(alacritty_terminal::term::cell::Flags::WRAPLINE)
        {
            start -= 1;
        }
        let mut end = line.to_alacritty();
        while end + 1 < self.grid.screen_lines()
            && self.grid[end][last_column]
                .flags
                .contains(alacritty_terminal::term::cell::Flags::WRAPLINE)
        {
            end += 1;
        }
        (Line::from_alacritty(start), Line::from_alacritty(end))
    }
}

/// Borrowed handle to a locked live terminal mirror or model screen.
pub struct ScreenGuard<'a> {
    term: &'a Term<EventProxy>,
}

impl<'a> ScreenGuard<'a> {
    #[inline]
    pub(crate) fn new(term: &'a Term<EventProxy>) -> Self {
        Self { term }
    }

    #[inline]
    pub fn cols(&self) -> usize {
        self.term.columns()
    }

    #[inline]
    pub fn rows(&self) -> usize {
        self.term.screen_lines()
    }

    #[inline]
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    #[inline]
    pub fn mode(&self) -> TerminalModeFlags {
        TerminalModeFlags::from_alacritty(*self.term.mode())
    }

    #[inline]
    pub fn view(&self) -> ScreenView<'_> {
        ScreenView::from_term(self.term)
    }

    pub fn bounds_to_string(&self, start: GridPoint, end: GridPoint) -> String {
        self.term
            .bounds_to_string(start.to_alacritty(), end.to_alacritty())
    }

    #[inline]
    pub(crate) fn term(&self) -> &Term<EventProxy> {
        self.term
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn snapshot_state(&self) -> crate::snapshot::TerminalSnapshotState {
        crate::snapshot::inspect_term(self.term)
    }
}
