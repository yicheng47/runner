//! Replay recorded PTY bytes into a headless Runner-owned terminal and
//! snapshot the visible grid — the regression harness the fixtures exist for.

use std::sync::mpsc::{self, Receiver, Sender};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions as _, Scroll};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;

use crate::fixtures::{decode_chunk, Fixture, FixtureEvent};
use crate::palette::{Rgb, TerminalPalette};
use crate::screen::ScreenView;
use crate::snapshot::{BoundaryScanner, TerminalSnapshotState, TrackedProcessor};
use crate::types::{GridPoint, Line, TerminalModeFlags};

pub(crate) struct ReplayListener(Sender<Event>);

impl EventListener for ReplayListener {
    fn send_event(&self, event: Event) {
        let _ = self.0.send(event);
    }
}

/// Reply emitted by a headless terminal while processing VT queries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalReply {
    ColorQuery { index: usize, reply: String },
    PtyWrite(String),
}

/// Runner-owned headless terminal for fixture replay, snapshot round-trips,
/// and unit/integration tests without exposing underlying VT engine types.
pub struct Terminal {
    term: Term<ReplayListener>,
    parser: TrackedProcessor,
    boundary: BoundaryScanner,
    events: Receiver<Event>,
}

impl Terminal {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self::from_config(cols, rows, Config::default())
    }

    pub fn with_scrollback(cols: u16, rows: u16, scrollback: usize) -> Self {
        Self::from_config(
            cols,
            rows,
            Config {
                scrolling_history: scrollback,
                ..Config::default()
            },
        )
    }

    pub fn with_kitty_keyboard(cols: u16, rows: u16) -> Self {
        Self::from_config(
            cols,
            rows,
            Config {
                kitty_keyboard: true,
                ..Config::default()
            },
        )
    }

    fn from_config(cols: u16, rows: u16, config: Config) -> Self {
        let (tx, rx) = mpsc::channel();
        let size = TermSize::new(cols as usize, rows as usize);
        Self {
            term: Term::new(config, &size, ReplayListener(tx)),
            parser: TrackedProcessor::new(),
            boundary: BoundaryScanner::default(),
            events: rx,
        }
    }

    /// Feed bytes using a stateless fresh parser, matching legacy `replay::feed`.
    pub fn feed(&mut self, bytes: &[u8]) {
        let mut parser: Processor = Processor::new();
        parser.advance(&mut self.term, bytes);
    }

    /// Advance the persistent parser and boundary scanner, matching live session parsing.
    pub fn advance(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
        let held = self
            .parser
            .sync_timeout()
            .sync_timeout()
            .map(|_| self.parser.sync_bytes_count());
        self.boundary.advance(bytes, held);
    }

    /// Flush any held synchronized update bytes immediately.
    pub fn stop_sync(&mut self) {
        self.parser.stop_sync(&mut self.term);
        self.boundary.end_sync();
    }

    pub fn sync_bytes_count(&self) -> usize {
        self.parser.sync_bytes_count()
    }

    pub fn preceding_char(&self) -> Option<char> {
        self.parser.preceding_char()
    }

    pub fn restore_preceding_char(&mut self, preceding: Option<char>) {
        self.parser.restore_preceding_char(preceding);
    }

    pub fn unfinished(&self) -> &[u8] {
        self.boundary.unfinished()
    }

    pub fn end_sync(&mut self) {
        self.boundary.end_sync();
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.term
            .resize(TermSize::new(cols as usize, rows as usize));
    }

    pub fn scroll_top(&mut self) {
        self.term.scroll_display(Scroll::Top);
    }

    pub fn scroll_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    #[inline]
    pub fn screen(&self) -> ScreenView<'_> {
        ScreenView::from_term(&self.term)
    }

    #[inline]
    pub fn mode(&self) -> TerminalModeFlags {
        TerminalModeFlags::from_alacritty(*self.term.mode())
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
    pub fn history_size(&self) -> usize {
        self.term.history_size()
    }

    #[inline]
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    pub fn row_cells(&self, line: i32) -> String {
        self.screen().row_cells(Line(line))
    }

    pub fn row_to_string(&self, line: i32) -> String {
        self.screen().row_to_string(Line(line))
    }

    pub fn visible_lines(&self) -> Vec<String> {
        self.screen().visible_lines()
    }

    pub fn screen_snapshot(&self) -> String {
        self.screen().screen_snapshot()
    }

    pub fn bounds_to_string(&self, start: GridPoint, end: GridPoint) -> String {
        self.term
            .bounds_to_string(start.to_alacritty(), end.to_alacritty())
    }

    pub fn snapshot_state(&self) -> TerminalSnapshotState {
        crate::snapshot::inspect_term(&self.term)
    }

    /// Drain queued query replies, resolving OSC color requests through the
    /// production `query_color_for` path against this terminal's current state.
    pub fn drain_replies(&self, base: &[Rgb; 256], theme: TerminalPalette) -> Vec<TerminalReply> {
        let mut replies = Vec::new();
        for event in self.events.try_iter() {
            match event {
                Event::ColorRequest(index, format) => {
                    let color =
                        crate::terminal::query_color_for(&self.screen(), index, base, theme);
                    replies.push(TerminalReply::ColorQuery {
                        index,
                        reply: format(color.to_alacritty()),
                    });
                }
                Event::PtyWrite(text) => replies.push(TerminalReply::PtyWrite(text)),
                _ => {}
            }
        }
        replies
    }

    #[inline]
    pub(crate) fn term(&self) -> &Term<ReplayListener> {
        &self.term
    }

    #[inline]
    pub(crate) fn term_mut(&mut self) -> &mut Term<ReplayListener> {
        &mut self.term
    }
}

pub fn new_term(cols: u16, rows: u16) -> Terminal {
    Terminal::new(cols, rows)
}

pub fn replay_bytes(cols: u16, rows: u16, bytes: &[u8]) -> Terminal {
    let mut term = new_term(cols, rows);
    feed(&mut term, bytes);
    term
}

pub fn replay_fixture(fixture: &Fixture) -> anyhow::Result<Terminal> {
    let mut term = new_term(fixture.header.cols, fixture.header.rows);
    let mut parser: Processor = Processor::new();
    for event in &fixture.events {
        match event {
            FixtureEvent::Data { data, .. } => {
                parser.advance(term.term_mut(), &decode_chunk(data)?)
            }
            FixtureEvent::Resize { cols, rows, .. } => term.resize(*cols, *rows),
            _ => {}
        }
    }
    Ok(term)
}

pub fn feed(term: &mut Terminal, bytes: &[u8]) {
    term.feed(bytes);
}

/// One visible row as a string: wide chars keep their glyph, spacer
/// cells are skipped, zero-width combining marks are re-attached.
pub fn row_to_string(term: &Terminal, line: i32) -> String {
    term.row_to_string(line)
}

/// Preserve trailing space cells for probes that join a wrapped line.
pub fn row_cells(term: &Terminal, line: i32) -> String {
    term.row_cells(line)
}

pub fn visible_lines(term: &Terminal) -> Vec<String> {
    term.visible_lines()
}

/// Deterministic textual snapshot of the visible screen, compared
/// against `.snapshot.txt` files in the fixture corpus.
pub fn screen_snapshot(term: &Terminal) -> String {
    term.screen_snapshot()
}

#[cfg(test)]
mod tests {
    //! Why terminals attach at the PTY's recorded size rather than a
    //! layout estimate: alacritty never reflows the alternate screen, so
    //! output replayed into a grid of the wrong width keeps its wrap
    //! damage through every later resize.
    use super::*;
    use crate::palette::{Color, Rgb};
    use crate::types::{Column, GridPoint, Line};

    const GRAY: Color = Color::Rgb(Rgb {
        r: 50,
        g: 50,
        b: 56,
    });

    /// One full-width row as a TUI paints it for a `width`-column PTY:
    /// background-filled cells, then a newline.
    fn painted_row(width: usize) -> Vec<u8> {
        let mut bytes = b"\x1b[48;2;50;50;56m".to_vec();
        bytes.extend(std::iter::repeat_n(b' ', width));
        bytes.extend_from_slice(b"\x1b[0m\r\n");
        bytes
    }

    fn column_zero_painted(term: &Terminal) -> Vec<bool> {
        let screen = term.screen();
        (0..6)
            .map(|line| {
                screen
                    .cell(GridPoint::new(Line(line), Column(0)))
                    .is_some_and(|cell| cell.bg() == GRAY)
            })
            .collect()
    }

    fn alt_screen_replay(grid_cols: u16, pty_cols: usize) -> Terminal {
        let mut term = new_term(grid_cols, 10);
        feed(&mut term, b"\x1b[?1049h");
        for _ in 0..3 {
            feed(&mut term, &painted_row(pty_cols));
        }
        term
    }

    #[test]
    fn alt_screen_replay_narrower_than_the_pty_keeps_its_wrap_tails_after_growing() {
        let mut term = alt_screen_replay(111, 112);
        assert_eq!(
            column_zero_painted(&term),
            [true; 6],
            "every row wrapped its last cell onto the next line"
        );
        term.resize(112, 10);
        assert_eq!(
            column_zero_painted(&term),
            [true; 6],
            "the alt screen does not reflow, so growing cannot rejoin them"
        );
    }

    #[test]
    fn alt_screen_replay_at_the_pty_width_is_clean() {
        let term = alt_screen_replay(112, 112);
        assert_eq!(
            column_zero_painted(&term),
            [true, true, true, false, false, false]
        );
    }
}
