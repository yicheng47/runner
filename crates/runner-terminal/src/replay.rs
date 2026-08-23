//! Replay recorded PTY bytes into a headless `Term` and snapshot the
//! visible grid — the regression harness the fixtures exist for.

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions as _;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;

use crate::fixtures::Fixture;

pub fn new_term(cols: u16, rows: u16) -> Term<VoidListener> {
    let size = TermSize::new(cols as usize, rows as usize);
    Term::new(Config::default(), &size, VoidListener)
}

pub fn replay_bytes(cols: u16, rows: u16, bytes: &[u8]) -> Term<VoidListener> {
    let mut term = new_term(cols, rows);
    feed(&mut term, bytes);
    term
}

pub fn replay_fixture(fixture: &Fixture) -> anyhow::Result<Term<VoidListener>> {
    Ok(replay_bytes(
        fixture.header.cols,
        fixture.header.rows,
        &fixture.output_bytes()?,
    ))
}

pub fn feed(term: &mut Term<VoidListener>, bytes: &[u8]) {
    let mut parser: Processor = Processor::new();
    parser.advance(term, bytes);
}

/// One visible row as a string: wide chars keep their glyph, spacer
/// cells are skipped, zero-width combining marks are re-attached.
pub fn row_to_string<T>(term: &Term<T>, line: i32) -> String {
    let grid = term.grid();
    let row = &grid[Line(line)];
    let mut s = String::new();
    for col in 0..grid.columns() {
        let cell = &row[Column(col)];
        if cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }
        s.push(cell.c);
        if let Some(zw) = cell.zerowidth() {
            s.extend(zw.iter());
        }
    }
    s.truncate(s.trim_end().len());
    s
}

pub fn visible_lines<T>(term: &Term<T>) -> Vec<String> {
    (0..term.screen_lines() as i32)
        .map(|l| row_to_string(term, l))
        .collect()
}

/// Deterministic textual snapshot of the visible screen, compared
/// against `.snapshot.txt` files in the fixture corpus.
pub fn screen_snapshot<T>(term: &Term<T>) -> String {
    let content_cursor = {
        let c = term.grid().cursor.point;
        (c.line.0, c.column.0)
    };
    let mut out = format!(
        "cols={} rows={} cursor={},{}\n",
        term.columns(),
        term.screen_lines(),
        content_cursor.0,
        content_cursor.1
    );
    for line in visible_lines(term) {
        out.push('|');
        out.push_str(&line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    //! Why terminals attach at the PTY's recorded size rather than a
    //! layout estimate: alacritty never reflows the alternate screen, so
    //! output replayed into a grid of the wrong width keeps its wrap
    //! damage through every later resize.
    use super::*;
    use alacritty_terminal::index::Point;
    use alacritty_terminal::vte::ansi::{Color, Rgb};

    const GRAY: Color = Color::Spec(Rgb {
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

    fn column_zero_painted(term: &Term<VoidListener>) -> Vec<bool> {
        (0..6)
            .map(|line| term.grid()[Point::new(Line(line), Column(0))].bg == GRAY)
            .collect()
    }

    fn alt_screen_replay(grid_cols: u16, pty_cols: usize) -> Term<VoidListener> {
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
        term.resize(TermSize::new(112, 10));
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
