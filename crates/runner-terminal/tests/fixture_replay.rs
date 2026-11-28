//! The regression harness (impl 0031 Phase 1): recorded PTY byte
//! logs replayed into `runner-terminal`, visible grid compared
//! against blessed snapshots. Re-bless after intentional changes:
//! `UPDATE_SNAPSHOTS=1 cargo test -p runner-terminal`.

use std::path::{Path, PathBuf};

use runner_terminal::fixtures::Fixture;
use runner_terminal::mappings::encode_scroll;
use runner_terminal::replay::{
    feed, new_term, replay_bytes, replay_fixture, screen_snapshot, visible_lines,
};
use runner_terminal::{CellFlags, Column, GridPoint, Line, TerminalModeFlags};

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
fn agy_first_turn_returns_to_primary_screen_for_idle_wheel() {
    let fixture = Fixture::load(&fixtures_dir().join("agy-first-turn.ndjson")).unwrap();
    let bytes = fixture.output_bytes().unwrap();
    assert!(bytes.windows(8).any(|window| window == b"\x1b[?1049h"));
    assert!(bytes.windows(8).any(|window| window == b"\x1b[?1049l"));
    let terminal = replay_fixture(&fixture).unwrap();
    let mode = terminal.mode();
    assert!(!mode.contains(TerminalModeFlags::ALT_SCREEN));
    assert!(!mode.intersects(TerminalModeFlags::MOUSE_MODE));
    assert_eq!(encode_scroll(mode, 1, false, 7, 4), None);
    assert_eq!(encode_scroll(mode, -1, false, 7, 4), None);
    let screen = screen_snapshot(&terminal);
    assert!(screen.contains("OK"));
    assert!(screen.contains("Gemini 3.8 Flash"));
}

#[test]
fn cjk_cells_are_wide_with_spacers() {
    let term = replay_bytes(20, 4, "a中b".as_bytes());
    let screen = term.screen();
    let at = |col| screen.cell(GridPoint::new(Line(0), Column(col))).unwrap();
    assert_eq!(at(0).c(), 'a');
    assert_eq!(at(1).c(), '中');
    assert!(at(1).flags().contains(CellFlags::WIDE_CHAR));
    assert!(at(2).flags().contains(CellFlags::WIDE_CHAR_SPACER));
    assert_eq!(at(3).c(), 'b');
    assert_eq!(visible_lines(&term)[0], "a中b");
}

#[test]
fn emoji_is_wide() {
    let term = replay_bytes(20, 4, "x😀y".as_bytes());
    let screen = term.screen();
    let at = |col| screen.cell(GridPoint::new(Line(0), Column(col))).unwrap();
    assert_eq!(at(1).c(), '😀');
    assert!(at(1).flags().contains(CellFlags::WIDE_CHAR));
    assert!(at(2).flags().contains(CellFlags::WIDE_CHAR_SPACER));
    assert_eq!(at(3).c(), 'y');
}

#[test]
fn combining_marks_attach_to_base_cell() {
    // e + U+0301 occupies one cell with a zerowidth attachment.
    let term = replay_bytes(20, 4, "e\u{0301}x".as_bytes());
    let screen = term.screen();
    let at = |col| screen.cell(GridPoint::new(Line(0), Column(col))).unwrap();
    assert_eq!(at(0).c(), 'e');
    assert_eq!(at(0).zerowidth(), Some(&['\u{0301}'][..]));
    assert_eq!(at(1).c(), 'x');
    assert_eq!(visible_lines(&term)[0], "e\u{0301}x");
}

#[test]
fn box_drawing_is_narrow() {
    let term = replay_bytes(20, 4, "┌─┐".as_bytes());
    let screen = term.screen();
    let at = |col| screen.cell(GridPoint::new(Line(0), Column(col))).unwrap();
    assert_eq!(at(0).c(), '┌');
    assert!(!at(0).flags().contains(CellFlags::WIDE_CHAR));
    assert_eq!(at(2).c(), '┐');
}

#[test]
fn wide_char_at_line_end_wraps_with_leading_spacer() {
    // Column 9 of 10 can't fit a wide char: a spacer holds the slot
    // and the glyph moves to the next row.
    let term = replay_bytes(10, 4, "123456789中".as_bytes());
    let screen = term.screen();
    assert!(screen
        .cell(GridPoint::new(Line(0), Column(9)))
        .unwrap()
        .flags()
        .contains(CellFlags::LEADING_WIDE_CHAR_SPACER));
    assert_eq!(
        screen.cell(GridPoint::new(Line(1), Column(0))).unwrap().c(),
        '中'
    );
    assert_eq!(visible_lines(&term)[0], "123456789");
    assert_eq!(visible_lines(&term)[1], "中");
}

#[test]
fn resize_reflows_wrapped_lines() {
    // 30 chars in a 10-col grid wrap to 3 rows; widening to 40 pulls
    // them back into one row. Narrowing again re-wraps in place (Decision 11:
    // Ghostty preserves top-anchored reflow when the re-wrapped rows still fit
    // in the viewport, rather than pushing rows above the cursor to scrollback).
    let text = "abcdefghijklmnopqrstuvwxyz0123";
    let mut term = replay_bytes(10, 6, text.as_bytes());
    assert_eq!(visible_lines(&term)[0], "abcdefghij");
    assert_eq!(visible_lines(&term)[2], "uvwxyz0123");
    assert_eq!(term.history_size(), 0);

    term.resize(40, 6);
    assert_eq!(visible_lines(&term)[0], text);
    assert_eq!(term.screen().raw_cursor_point().line.0, 0);
    assert_eq!(term.history_size(), 0);

    term.resize(10, 6);
    assert_eq!(term.history_size(), 0);
    assert_eq!(visible_lines(&term)[0], "abcdefghij");
    assert_eq!(visible_lines(&term)[1], "klmnopqrst");
    assert_eq!(visible_lines(&term)[2], "uvwxyz0123");
    assert_eq!(
        term.screen().raw_cursor_point(),
        GridPoint::new(Line(3), Column(0))
    );
}

#[test]
fn resize_reflow_preserves_wide_chars() {
    let mut term = replay_bytes(10, 6, "汉字宽度测试abcd".as_bytes());
    term.resize(6, 6);
    term.resize(20, 6);
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
    term.scroll_top();
    assert_eq!(term.display_offset(), term.history_size());
    term.scroll_bottom();
    assert_eq!(term.display_offset(), 0);
}

#[test]
fn grok_first_turn_uses_alternate_screen_and_reports_mouse_at_idle() {
    let fixture = Fixture::load(&fixtures_dir().join("grok-first-turn.ndjson")).unwrap();
    let terminal = replay_fixture(&fixture).unwrap();
    let mode = terminal.mode();
    assert!(mode.contains(TerminalModeFlags::ALT_SCREEN));
    assert!(mode.intersects(TerminalModeFlags::MOUSE_MODE));
    assert!(mode.contains(TerminalModeFlags::BRACKETED_PASTE));
    let screen = screen_snapshot(&terminal);
    assert!(screen.contains("GROK_FIXTURE_OK"));
    assert!(screen.contains("Grok 4.5"));
    assert!(encode_scroll(mode, 1, false, 7, 4).is_some());
}
