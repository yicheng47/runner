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
