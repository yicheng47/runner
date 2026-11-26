//! Unified `vte::ansi::Handler` wrapper around `alacritty_terminal::Term`.
//!
//! Every place that feeds a `Term` (authoritative model and mirrors, live
//! output, synchronized-update flushes, session title sync and snapshot
//! restore) parses through `StreamHandler`. It forwards every method to `Term`
//! unchanged, counts rows scrolled into history for terminal find, maintains
//! Kitty graphics placements across scrolls/clears/resets/screen switches,
//! tracks DEC private mode 1016 (SGR-Pixels) with DECRQM, and answers
//! `CSI 14 t` in `u32` pixels.

use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::{BidirectionalIterator as _, Dimensions as _, Grid, GridCell as _};
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::cursor_icon::CursorIcon;
use alacritty_terminal::vte::ansi::{
    Attr, CharsetIndex, ClearMode, CursorShape, CursorStyle, Handler, Hyperlink, KeyboardModes,
    KeyboardModesApplyBehavior, LineClearMode, Mode, ModifyOtherKeys, NamedPrivateMode,
    PrivateMode, Rgb, ScpCharPath, ScpUpdateMode, StandardCharset, TabulationClearMode,
};

use crate::kitty::exec::{self, ExecMode, ExecResult, ExecuteHost};
use crate::kitty::media::PreparedCommand;
use crate::kitty::storage::{CellSize, ImageStorage, TerminalGraphics};

pub struct StreamHandler<'a, T> {
    term: &'a mut Term<T>,
    graphics: &'a mut TerminalGraphics,
    sgr_pixels: &'a AtomicBool,
    cell_size: CellSize,
    reply: Option<&'a mut dyn FnMut(String)>,
    initial_alt_screen: bool,
    lines: usize,
}

impl<'a, T: EventListener> StreamHandler<'a, T> {
    pub fn new(
        term: &'a mut Term<T>,
        graphics: &'a mut TerminalGraphics,
        sgr_pixels: &'a AtomicBool,
        cell_size: CellSize,
        reply: Option<&'a mut dyn FnMut(String)>,
    ) -> Self {
        let initial_alt_screen = term.mode().contains(TermMode::ALT_SCREEN);
        Self {
            term,
            graphics,
            sgr_pixels,
            cell_size,
            reply,
            initial_alt_screen,
            lines: 0,
        }
    }

    /// Lines moved up on the initial screen, or `None` when the parse ended on
    /// the other screen.
    pub fn finish(self) -> Option<usize> {
        (self.term.mode().contains(TermMode::ALT_SCREEN) == self.initial_alt_screen)
            .then_some(self.lines)
    }

    /// Execute one prepared Kitty graphics command against the active screen
    /// and emit its reply (if any) through the handler's reply sink in stream
    /// order.
    pub fn execute_kitty(&mut self, prepared: &PreparedCommand, mode: ExecMode) -> ExecResult {
        let result = exec::execute(self, self.cell_size, prepared, mode);
        if let Some(reply) = &result.reply {
            self.emit_reply(reply.clone());
        }
        result
    }

    fn emit_reply(&mut self, text: String) {
        if let Some(sink) = self.reply.as_mut() {
            sink(text);
        }
    }

    /// Measure how many rows alacritty moved `Line(0)` upward, updating find's
    /// scroll counter when still on the initial screen.
    fn measure_and_advance_find(&mut self, row0_before: usize) -> usize {
        let term = &*self.term;
        if row_address(term, Line(0)) == row0_before {
            return 0;
        }
        let screen_lines = term.screen_lines() as i32;
        let in_history = (1..=screen_lines.min(term.history_size() as i32))
            .find(|line| row_address(term, Line(-line)) == row0_before);
        let reused =
            || (1..screen_lines).find(|line| row_address(term, Line(*line)) == row0_before);
        let moved = match in_history {
            Some(line) => line as usize,
            None => reused().map_or(screen_lines as usize, |line| (screen_lines - line) as usize),
        };
        if term.mode().contains(TermMode::ALT_SCREEN) == self.initial_alt_screen {
            self.lines += moved;
        }
        moved
    }

    fn apply_scroll_up_graphics(&mut self, region: Range<Line>, count: usize) {
        if count == 0 {
            return;
        }
        let is_alt = self.term.mode().contains(TermMode::ALT_SCREEN);
        let screen_lines = self.term.screen_lines() as i32;
        let topmost = self.term.topmost_line().0;
        let storage = self.graphics.active_mut(is_alt);
        if region.start == Line(0) && region.end == Line(screen_lines) {
            storage.scroll_full_up(count, topmost, self.cell_size);
        } else {
            if region.start == Line(0) {
                storage.scroll_history_only_up(count, topmost, self.cell_size);
            }
            storage.scroll_margins(
                region.start.0,
                region.end.0 - 1,
                -(count as i32),
                self.cell_size,
            );
        }
    }

    fn apply_scroll_down_graphics(&mut self, region: Range<Line>, count: usize) {
        if count == 0 {
            return;
        }
        let is_alt = self.term.mode().contains(TermMode::ALT_SCREEN);
        let screen_lines = self.term.screen_lines();
        let storage = self.graphics.active_mut(is_alt);
        if region.start == Line(0) && region.end == Line(screen_lines as i32) {
            storage.scroll_full_down(count, screen_lines, self.cell_size);
        } else {
            storage.scroll_margins(
                region.start.0,
                region.end.0 - 1,
                count as i32,
                self.cell_size,
            );
        }
    }

    fn wrap_scrolling_call(&mut self, call: impl FnOnce(&mut Term<T>)) {
        let region = self.term.snapshot_state().scroll_region.clone();
        let row0 = row_address(self.term, Line(0));
        let region_top = row_address(self.term, region.start);
        let cursor_before = self.term.grid().cursor.point;
        let needs_wrap_before = self.term.grid().cursor.input_needs_wrap;
        let line_wrap = self.term.mode().contains(TermMode::LINE_WRAP);

        call(self.term);

        let find_moved = self.measure_and_advance_find(row0);
        let scrolled = if region.start == Line(0) {
            if find_moved > 0 {
                if region.end == Line(self.term.screen_lines() as i32) {
                    find_moved
                } else {
                    measure_region_scroll_up(self.term, &region, row0)
                }
            } else if self.term.screen_lines() == 1
                && line_wrap
                && needs_wrap_before
                && cursor_before.line + 1 == region.end
                && self.term.grid().cursor.point.column == Column(0)
            {
                1
            } else {
                0
            }
        } else if row_address(self.term, region.start) != region_top {
            measure_region_scroll_up(self.term, &region, region_top)
        } else {
            0
        };
        if scrolled > 0 {
            self.apply_scroll_up_graphics(region, scrolled);
        }
    }
}

impl<T: EventListener> ExecuteHost for StreamHandler<'_, T> {
    fn storage(&self) -> &ImageStorage {
        let is_alt = self.term.mode().contains(TermMode::ALT_SCREEN);
        self.graphics.active(is_alt)
    }

    fn storage_mut(&mut self) -> &mut ImageStorage {
        let is_alt = self.term.mode().contains(TermMode::ALT_SCREEN);
        self.graphics.active_mut(is_alt)
    }

    fn term_cols(&self) -> usize {
        self.term.columns()
    }

    fn term_rows(&self) -> usize {
        self.term.screen_lines()
    }

    fn term_cursor(&self) -> (usize, i32) {
        let p = self.term.grid().cursor.point;
        (p.column.0, p.line.0)
    }

    fn term_scroll_region(&self) -> (i32, i32) {
        let r = &self.term.snapshot_state().scroll_region;
        (r.start.0, r.end.0 - 1)
    }

    fn term_linefeed(&mut self) {
        <Self as Handler>::linefeed(self);
    }

    fn term_set_cursor_col(&mut self, col: usize) {
        exec::set_term_cursor_col(self.term, col);
    }
}

fn row_address<T>(term: &Term<T>, line: Line) -> usize {
    std::ptr::from_ref(&term.grid()[line][Column(0)]) as usize
}

fn measure_region_scroll_up<T>(term: &Term<T>, region: &Range<Line>, top_before: usize) -> usize {
    let height = (region.end.0 - region.start.0).max(0);
    if region.start == Line(0) {
        let max_hist = height.min(term.history_size() as i32);
        if let Some(line) = (1..=max_hist).find(|l| row_address(term, Line(-l)) == top_before) {
            return line as usize;
        }
    }
    if let Some(line) =
        (region.start.0..region.end.0).find(|l| row_address(term, Line(*l)) == top_before)
    {
        return (region.end.0 - line) as usize;
    }
    1
}

/// Mirror of `alacritty_terminal::grid::Grid::clear_viewport`'s row count so
/// ED 2 moves placements by the exact number of rows alacritty shifts into
/// history even when scrollback is smaller than the shift.
fn clear_viewport_positions<T>(term: &Term<T>) -> usize {
    let grid = term.grid();
    let end = Point::new(Line(grid.screen_lines() as i32 - 1), Column(grid.columns()));
    let mut iter = grid.iter_from(end);
    while let Some(cell) = iter.prev() {
        if !cell.is_empty() || cell.point.line < 0 {
            break;
        }
    }
    (iter.point().line.0 + 1).max(0) as usize
}

/// Compute the line shift `Grid::resize` applies to one grid (`primary` or
/// `alt`) before `Term::resize` runs.
pub fn grid_resize_line_shift(grid: &Grid<Cell>, new_rows: usize) -> i32 {
    let old_rows = grid.screen_lines();
    if new_rows > old_rows {
        let added = new_rows - old_rows;
        grid.history_size().min(added) as i32
    } else if new_rows < old_rows {
        let required = (grid.cursor.point.line.0 as usize + 1).saturating_sub(new_rows);
        -(required as i32)
    } else {
        0
    }
}

/// Resize `term` and shift/prune both screens' Kitty graphics placements to
/// match alacritty's `Grid::resize` on primary and alternate grids.
pub fn resize_term_and_graphics<T: EventListener>(
    term: &mut Term<T>,
    graphics: &mut TerminalGraphics,
    new_cols: usize,
    new_rows: usize,
    cell_size: CellSize,
) {
    if term.columns() == new_cols && term.screen_lines() == new_rows {
        return;
    }
    let is_alt = term.mode().contains(TermMode::ALT_SCREEN);
    let (primary_shift, alt_shift) = {
        let state = term.snapshot_state();
        let (primary, alt) = if is_alt {
            (state.inactive_grid, term.grid())
        } else {
            (term.grid(), state.inactive_grid)
        };
        (
            grid_resize_line_shift(primary, new_rows),
            grid_resize_line_shift(alt, new_rows),
        )
    };

    term.resize(alacritty_terminal::term::test::TermSize::new(
        new_cols, new_rows,
    ));

    let (primary_topmost, alt_topmost) = if is_alt {
        (
            term.snapshot_state().inactive_grid.topmost_line().0,
            term.topmost_line().0,
        )
    } else {
        (
            term.topmost_line().0,
            term.snapshot_state().inactive_grid.topmost_line().0,
        )
    };
    graphics.primary.apply_resize(
        primary_shift,
        primary_topmost,
        new_rows,
        new_cols,
        cell_size,
    );
    graphics
        .alt
        .apply_resize(alt_shift, alt_topmost, new_rows, new_cols, cell_size);
}

/// Apply `Term::set_options` and prune any primary-screen placements scrolled
/// past the new scrollback limit.
pub fn configure_term_and_graphics<T: EventListener>(
    term: &mut Term<T>,
    graphics: &mut TerminalGraphics,
    config: Config,
    cell_size: CellSize,
) {
    term.set_options(config);
    let is_alt = term.mode().contains(TermMode::ALT_SCREEN);
    let primary_topmost = if is_alt {
        term.snapshot_state().inactive_grid.topmost_line().0
    } else {
        term.topmost_line().0
    };
    graphics.primary.prune_above(primary_topmost, cell_size);
}

macro_rules! forward {
    ($($name:ident($($arg:ident: $ty:ty),*);)*) => {
        $(
            #[inline]
            fn $name(&mut self, $($arg: $ty),*) {
                <Term<T> as Handler>::$name(self.term, $($arg),*)
            }
        )*
    };
}

/// Every `Handler` method, so the terminal behaves identically to a plain
/// `Term` parse while tracking scrollback, graphics, mode 1016 and `CSI 14 t`.
impl<T: EventListener> Handler for StreamHandler<'_, T> {
    #[inline]
    fn input(&mut self, c: char) {
        self.wrap_scrolling_call(|term| <Term<T> as Handler>::input(term, c));
    }

    #[inline]
    fn put_tab(&mut self, count: u16) {
        self.wrap_scrolling_call(|term| <Term<T> as Handler>::put_tab(term, count));
    }

    #[inline]
    fn linefeed(&mut self) {
        let region = self.term.snapshot_state().scroll_region.clone();
        let scrolled = self.term.grid().cursor.point.line + 1 == region.end;
        let row0 = row_address(self.term, Line(0));
        <Term<T> as Handler>::linefeed(self.term);
        self.measure_and_advance_find(row0);
        if scrolled {
            self.apply_scroll_up_graphics(region, 1);
        }
    }

    #[inline]
    fn newline(&mut self) {
        let region = self.term.snapshot_state().scroll_region.clone();
        let scrolled = self.term.grid().cursor.point.line + 1 == region.end;
        let row0 = row_address(self.term, Line(0));
        <Term<T> as Handler>::newline(self.term);
        self.measure_and_advance_find(row0);
        if scrolled {
            self.apply_scroll_up_graphics(region, 1);
        }
    }

    #[inline]
    fn scroll_up(&mut self, lines: usize) {
        let region = self.term.snapshot_state().scroll_region.clone();
        let clamped = lines.min((region.end - region.start).0.max(0) as usize);
        let row0 = row_address(self.term, Line(0));
        <Term<T> as Handler>::scroll_up(self.term, lines);
        self.measure_and_advance_find(row0);
        if clamped > 0 {
            self.apply_scroll_up_graphics(region, clamped);
        }
    }

    #[inline]
    fn scroll_down(&mut self, lines: usize) {
        let region = self.term.snapshot_state().scroll_region.clone();
        let clamped = lines.min((region.end - region.start).0.max(0) as usize);
        <Term<T> as Handler>::scroll_down(self.term, lines);
        if clamped > 0 {
            self.apply_scroll_down_graphics(region, clamped);
        }
    }

    #[inline]
    fn reverse_index(&mut self) {
        let region = self.term.snapshot_state().scroll_region.clone();
        let scrolled = self.term.grid().cursor.point.line == region.start;
        <Term<T> as Handler>::reverse_index(self.term);
        if scrolled {
            self.apply_scroll_down_graphics(region, 1);
        }
    }

    #[inline]
    fn delete_lines(&mut self, lines: usize) {
        let region = self.term.snapshot_state().scroll_region.clone();
        let origin = self.term.grid().cursor.point.line;
        let screen_lines = self.term.screen_lines();
        let effective = if origin == Line(0) && region.contains(&origin) {
            lines
                .min(screen_lines)
                .min((region.end - region.start).0.max(0) as usize)
        } else {
            0
        };
        let row0 = row_address(self.term, Line(0));
        <Term<T> as Handler>::delete_lines(self.term, lines);
        self.measure_and_advance_find(row0);
        if effective > 0 {
            let is_alt = self.term.mode().contains(TermMode::ALT_SCREEN);
            let topmost = self.term.topmost_line().0;
            self.graphics.active_mut(is_alt).scroll_history_only_up(
                effective,
                topmost,
                self.cell_size,
            );
        }
    }

    #[inline]
    fn clear_screen(&mut self, mode: ClearMode) {
        let is_alt = self.term.mode().contains(TermMode::ALT_SCREEN);
        let row0 = row_address(self.term, Line(0));
        let clear_all = matches!(mode, ClearMode::All);
        let clear_saved = matches!(mode, ClearMode::Saved);
        let pushed_on_all = if clear_all && !is_alt {
            clear_viewport_positions(self.term)
        } else {
            0
        };
        <Term<T> as Handler>::clear_screen(self.term, mode);
        self.measure_and_advance_find(row0);

        if clear_all {
            let screen_lines = self.term.screen_lines();
            let screen_cols = self.term.columns();
            let topmost = self.term.topmost_line().0;
            let storage = self.graphics.active_mut(is_alt);
            if pushed_on_all > 0 {
                storage.scroll_full_up(pushed_on_all, topmost, self.cell_size);
            }
            storage.clear_screen(screen_lines, screen_cols, self.cell_size);
        } else if clear_saved && !is_alt {
            self.graphics.primary.erase_history();
        }
    }

    #[inline]
    fn reset_state(&mut self) {
        <Term<T> as Handler>::reset_state(self.term);
        self.graphics.reset();
        self.sgr_pixels.store(false, Ordering::Release);
    }

    #[inline]
    fn set_private_mode(&mut self, mode: PrivateMode) {
        let entering_alt = matches!(
            mode,
            PrivateMode::Named(NamedPrivateMode::SwapScreenAndSetRestoreCursor)
        ) && !self.term.mode().contains(TermMode::ALT_SCREEN);

        match mode {
            PrivateMode::Unknown(1016) => {
                self.sgr_pixels.store(true, Ordering::Release);
            }
            PrivateMode::Named(NamedPrivateMode::SgrMouse | NamedPrivateMode::Utf8Mouse)
            | PrivateMode::Unknown(1015) => {
                self.sgr_pixels.store(false, Ordering::Release);
            }
            _ => {}
        }

        <Term<T> as Handler>::set_private_mode(self.term, mode);

        if entering_alt {
            let screen_lines = self.term.screen_lines();
            let screen_cols = self.term.columns();
            self.graphics
                .alt
                .clear_screen(screen_lines, screen_cols, self.cell_size);
        }
    }

    #[inline]
    fn unset_private_mode(&mut self, mode: PrivateMode) {
        if matches!(mode, PrivateMode::Unknown(1016)) {
            self.sgr_pixels.store(false, Ordering::Release);
        }
        <Term<T> as Handler>::unset_private_mode(self.term, mode);
    }

    #[inline]
    fn report_private_mode(&mut self, mode: PrivateMode) {
        if matches!(mode, PrivateMode::Unknown(1016)) {
            let state = if self.sgr_pixels.load(Ordering::Acquire) {
                1
            } else {
                2
            };
            self.emit_reply(format!("\x1b[?1016;{state}$y"));
            return;
        }
        <Term<T> as Handler>::report_private_mode(self.term, mode);
    }

    #[inline]
    fn text_area_size_pixels(&mut self) {
        let width = u32::from(self.cell_size.width) * (self.term.columns() as u32);
        let height = u32::from(self.cell_size.height) * (self.term.screen_lines() as u32);
        self.emit_reply(format!("\x1b[4;{height};{width}t"));
    }

    forward! {
        set_title(title: Option<String>);
        set_cursor_style(style: Option<CursorStyle>);
        set_cursor_shape(shape: CursorShape);
        goto(line: i32, col: usize);
        goto_line(line: i32);
        goto_col(col: usize);
        insert_blank(count: usize);
        move_up(lines: usize);
        move_down(lines: usize);
        identify_terminal(intermediate: Option<char>);
        device_status(arg: usize);
        move_forward(col: usize);
        move_backward(col: usize);
        move_down_and_cr(row: usize);
        move_up_and_cr(row: usize);
        backspace();
        carriage_return();
        bell();
        substitute();
        set_horizontal_tabstop();
        insert_blank_lines(lines: usize);
        erase_chars(count: usize);
        delete_chars(count: usize);
        move_backward_tabs(count: u16);
        move_forward_tabs(count: u16);
        save_cursor_position();
        restore_cursor_position();
        clear_line(mode: LineClearMode);
        clear_tabs(mode: TabulationClearMode);
        set_tabs(interval: u16);
        terminal_attribute(attr: Attr);
        set_mode(mode: Mode);
        unset_mode(mode: Mode);
        report_mode(mode: Mode);
        set_scrolling_region(top: usize, bottom: Option<usize>);
        set_keypad_application_mode();
        unset_keypad_application_mode();
        set_active_charset(index: CharsetIndex);
        configure_charset(index: CharsetIndex, charset: StandardCharset);
        set_color(index: usize, color: Rgb);
        dynamic_color_sequence(prefix: String, index: usize, terminator: &str);
        reset_color(index: usize);
        clipboard_store(clipboard: u8, data: &[u8]);
        clipboard_load(clipboard: u8, terminator: &str);
        decaln();
        push_title();
        pop_title();
        text_area_size_chars();
        set_hyperlink(hyperlink: Option<Hyperlink>);
        set_mouse_cursor_icon(icon: CursorIcon);
        report_keyboard_mode();
        push_keyboard_mode(mode: KeyboardModes);
        pop_keyboard_modes(to_pop: u16);
        set_keyboard_mode(mode: KeyboardModes, behavior: KeyboardModesApplyBehavior);
        set_modify_other_keys(mode: ModifyOtherKeys);
        report_modify_other_keys();
        set_scp(char_path: ScpCharPath, update_mode: ScpUpdateMode);
    }
}
