//! The regression harness (impl 0031 Phase 1): recorded PTY byte
//! logs replayed into `alacritty_terminal`, visible grid compared
//! against blessed snapshots. Re-bless after intentional changes:
//! `UPDATE_SNAPSHOTS=1 cargo test -p runner-terminal`.

use std::path::{Path, PathBuf};

use alacritty_terminal::grid::Dimensions as _;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;

use runner_terminal::fixtures::Fixture;
use runner_terminal::replay::{
    feed, new_term, replay_bytes, replay_fixture, screen_snapshot, visible_lines,
};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

#[test]
fn fixture_corpus_snapshots() {
    let mut checked = 0;
    let mut entries: Vec<_> = std::fs::read_dir(fixtures_dir())
        .expect("fixtures dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "ndjson"))
        .collect();
    entries.sort();
    for path in entries {
        let fixture = Fixture::load(&path).unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
        let term = replay_fixture(&fixture).expect("replay");
        let snapshot = screen_snapshot(&term);
        let expected_path = path.with_extension("snapshot.txt");
        if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
            std::fs::write(&expected_path, &snapshot).expect("bless snapshot");
        } else {
            let expected = std::fs::read_to_string(&expected_path).unwrap_or_else(|_| {
                panic!(
                    "missing {} — run UPDATE_SNAPSHOTS=1 cargo test -p runner-terminal",
                    expected_path.display()
                )
            });
            assert_eq!(
                snapshot,
                expected,
                "grid snapshot drifted for {}",
                path.display()
            );
        }
        checked += 1;
    }
    assert!(
        checked > 0,
        "fixture corpus is empty — record fixtures first"
    );
}

#[test]
fn cjk_cells_are_wide_with_spacers() {
    let term = replay_bytes(20, 4, "a中b".as_bytes());
    let grid = term.grid();
    let row = &grid[Line(0)];
    assert_eq!(row[Column(0)].c, 'a');
    assert_eq!(row[Column(1)].c, '中');
    assert!(row[Column(1)].flags.contains(Flags::WIDE_CHAR));
    assert!(row[Column(2)].flags.contains(Flags::WIDE_CHAR_SPACER));
    assert_eq!(row[Column(3)].c, 'b');
    assert_eq!(visible_lines(&term)[0], "a中b");
}

#[test]
fn emoji_is_wide() {
    let term = replay_bytes(20, 4, "x😀y".as_bytes());
    let grid = term.grid();
    let row = &grid[Line(0)];
    assert_eq!(row[Column(1)].c, '😀');
    assert!(row[Column(1)].flags.contains(Flags::WIDE_CHAR));
    assert!(row[Column(2)].flags.contains(Flags::WIDE_CHAR_SPACER));
    assert_eq!(row[Column(3)].c, 'y');
}

#[test]
fn combining_marks_attach_to_base_cell() {
    // e + U+0301 occupies one cell with a zerowidth attachment.
    let term = replay_bytes(20, 4, "e\u{0301}x".as_bytes());
    let grid = term.grid();
    let row = &grid[Line(0)];
    assert_eq!(row[Column(0)].c, 'e');
    assert_eq!(row[Column(0)].zerowidth(), Some(&['\u{0301}'][..]));
    assert_eq!(row[Column(1)].c, 'x');
    assert_eq!(visible_lines(&term)[0], "e\u{0301}x");
}

#[test]
fn box_drawing_is_narrow() {
    let term = replay_bytes(20, 4, "┌─┐".as_bytes());
    let grid = term.grid();
    let row = &grid[Line(0)];
    assert_eq!(row[Column(0)].c, '┌');
    assert!(!row[Column(0)].flags.contains(Flags::WIDE_CHAR));
    assert_eq!(row[Column(2)].c, '┐');
}

#[test]
fn wide_char_at_line_end_wraps_with_leading_spacer() {
    // Column 9 of 10 can't fit a wide char: a spacer holds the slot
    // and the glyph moves to the next row.
    let term = replay_bytes(10, 4, "123456789中".as_bytes());
    let grid = term.grid();
    assert!(grid[Line(0)][Column(9)]
        .flags
        .contains(Flags::LEADING_WIDE_CHAR_SPACER));
    assert_eq!(grid[Line(1)][Column(0)].c, '中');
    assert_eq!(visible_lines(&term)[0], "123456789");
    assert_eq!(visible_lines(&term)[1], "中");
}

#[test]
fn resize_reflows_wrapped_lines() {
    // 30 chars in a 10-col grid wrap to 3 rows; widening to 40 pulls
    // them back into one row. Narrowing again re-wraps, but alacritty
    // pins the cursor's viewport line: the cursor sat on row 0 after
    // widening, so the re-wrapped rows ABOVE it go to scrollback and
    // row 0 keeps the cursor's segment (#308 class).
    let text = "abcdefghijklmnopqrstuvwxyz0123";
    let mut term = replay_bytes(10, 6, text.as_bytes());
    assert_eq!(visible_lines(&term)[0], "abcdefghij");
    assert_eq!(visible_lines(&term)[2], "uvwxyz0123");
    assert_eq!(term.history_size(), 0);

    term.resize(TermSize::new(40, 6));
    assert_eq!(visible_lines(&term)[0], text);
    assert_eq!(term.grid().cursor.point.line.0, 0);
    assert_eq!(term.history_size(), 0);

    term.resize(TermSize::new(10, 6));
    assert_eq!(term.history_size(), 2);
    assert_eq!(
        runner_terminal::replay::row_to_string(&term, -2),
        "abcdefghij"
    );
    assert_eq!(
        runner_terminal::replay::row_to_string(&term, -1),
        "klmnopqrst"
    );
    assert_eq!(visible_lines(&term)[0], "uvwxyz0123");
    assert_eq!(
        term.grid().cursor.point,
        alacritty_terminal::index::Point::new(Line(0), Column(9))
    );
}

#[test]
fn resize_reflow_preserves_wide_chars() {
    let mut term = replay_bytes(10, 6, "汉字宽度测试abcd".as_bytes());
    term.resize(TermSize::new(6, 6));
    term.resize(TermSize::new(20, 6));
    let joined = visible_lines(&term).join("");
    assert!(
        joined.contains("汉字宽度测试abcd"),
        "wide chars corrupted by reflow round-trip: {joined:?}"
    );
}

#[test]
fn alt_screen_enter_and_leave_restores_primary() {
    let mut term = new_term(20, 5);
    feed(&mut term, b"primary line\r\n");
    // Enter alt screen (1049), draw, leave.
    feed(&mut term, b"\x1b[?1049halt content\x1b[?1049l");
    assert_eq!(visible_lines(&term)[0], "primary line");
}

#[test]
fn scrollback_survives_and_display_offset_clamps() {
    let mut term = new_term(10, 4);
    for i in 0..20 {
        feed(&mut term, format!("line{i}\r\n").as_bytes());
    }
    assert!(term.history_size() > 0);
    term.scroll_display(alacritty_terminal::grid::Scroll::Top);
    assert_eq!(term.grid().display_offset(), term.history_size());
    term.scroll_display(alacritty_terminal::grid::Scroll::Bottom);
    assert_eq!(term.grid().display_offset(), 0);
}
