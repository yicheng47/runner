use std::path::{Path, PathBuf};

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::{Cursor, Dimensions, Grid};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermMode};
use runner_terminal::fixtures::{decode_chunk, Fixture, FixtureEvent};
use runner_terminal::snapshot::TrackedProcessor as Processor;
use runner_terminal::snapshot::{serialize, BoundaryScanner};

fn new_term(cols: u16, rows: u16) -> Term<VoidListener> {
    Term::new(
        Config {
            kitty_keyboard: true,
            ..Config::default()
        },
        &TermSize::new(cols as usize, rows as usize),
        VoidListener,
    )
}

fn cell_equal(a: &Cell, b: &Cell, context: &str) {
    assert_eq!(
        (
            a.c,
            a.fg,
            a.bg,
            a.flags,
            a.zerowidth().unwrap_or_default(),
            a.underline_color()
        ),
        (
            b.c,
            b.fg,
            b.bg,
            b.flags,
            b.zerowidth().unwrap_or_default(),
            b.underline_color()
        ),
        "{context}"
    );
}

fn cursor_equal(a: &Cursor<Cell>, b: &Cursor<Cell>, context: &str) {
    assert_eq!(a.point, b.point, "{context}: point");
    assert_eq!(a.charsets, b.charsets, "{context}: charsets");
    assert_eq!(a.input_needs_wrap, b.input_needs_wrap, "{context}: wrap");
    cell_equal(&a.template, &b.template, context);
}

fn grid_equal(a: &Grid<Cell>, b: &Grid<Cell>, context: &str) {
    assert_eq!(a.history_size(), b.history_size(), "{context}: history");
    cursor_equal(&a.cursor, &b.cursor, &format!("{context}: cursor"));
    cursor_equal(
        &a.saved_cursor,
        &b.saved_cursor,
        &format!("{context}: saved cursor"),
    );
    for line in -(a.history_size() as i32)..a.screen_lines() as i32 {
        for column in 0..a.columns() {
            let a = &a[Line(line)][Column(column)];
            let b = &b[Line(line)][Column(column)];
            cell_equal(a, b, &format!("{context}: cell {line}:{column}"));
        }
    }
}

fn equal(a: &Term<VoidListener>, b: &Term<VoidListener>, context: &str) {
    assert_eq!(a.mode(), b.mode(), "{context}: modes");
    grid_equal(a.grid(), b.grid(), context);
    let x = a.snapshot_state();
    let y = b.snapshot_state();
    if a.mode().contains(TermMode::ALT_SCREEN) {
        grid_equal(x.inactive_grid, y.inactive_grid, context);
    }
    cursor_equal(
        &x.inactive_grid.saved_cursor,
        &y.inactive_grid.saved_cursor,
        &format!("{context}: inactive saved cursor"),
    );
    assert_eq!(x.scroll_region, y.scroll_region, "{context}: scroll region");
    assert_eq!(x.title, y.title, "{context}: title");
    assert_eq!(x.title_stack, y.title_stack, "{context}: title stack");
    assert_eq!(x.tabs, y.tabs, "{context}: tabs");
    assert_eq!(
        x.keyboard_mode_stack, y.keyboard_mode_stack,
        "{context}: kitty stack"
    );
    assert_eq!(
        x.inactive_keyboard_mode_stack, y.inactive_keyboard_mode_stack,
        "{context}: inactive kitty stack"
    );
    assert_eq!(x.cursor_style, y.cursor_style, "{context}: cursor style");
    assert_eq!(x.active_charset, y.active_charset, "{context}: charset");
}

fn restore(
    original: &Term<VoidListener>,
    parser: &Processor,
    scanner: &BoundaryScanner,
) -> (Term<VoidListener>, Processor, BoundaryScanner) {
    let mut restored = new_term(original.columns() as u16, original.screen_lines() as u16);
    let mut replay = Processor::new();
    let mut boundary = BoundaryScanner::default();
    let bytes = serialize(original, scanner.unfinished());
    let split = bytes.len() - scanner.unfinished().len();
    parse(&mut restored, &mut replay, &mut boundary, &bytes[..split]);
    replay.restore_preceding_char(parser.preceding_char());
    parse(&mut restored, &mut replay, &mut boundary, &bytes[split..]);
    (restored, replay, boundary)
}

fn recordings() -> Vec<PathBuf> {
    let mut paths = std::fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures"))
        .unwrap()
        .map(|p| p.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "ndjson"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn parse(
    term: &mut Term<VoidListener>,
    parser: &mut Processor,
    scanner: &mut BoundaryScanner,
    bytes: &[u8],
) {
    parser.advance(term, bytes);
    let held = parser
        .sync_timeout()
        .sync_timeout()
        .map(|_| parser.sync_bytes_count());
    scanner.advance(bytes, held);
}

#[test]
fn recording_round_trips() {
    for path in recordings() {
        let fixture = Fixture::load(&path).unwrap();
        let mut original = new_term(fixture.header.cols, fixture.header.rows);
        let mut parser = Processor::new();
        let mut scanner = BoundaryScanner::default();
        for event in &fixture.events {
            apply_event(&mut original, &mut parser, &mut scanner, event);
        }
        let (restored, _, _) = restore(&original, &parser, &scanner);
        equal(&original, &restored, path.to_str().unwrap());
    }
}

#[test]
fn recording_split_points() {
    for path in recordings() {
        let fixture = Fixture::load(&path).unwrap();
        let bytes = fixture.output_bytes().unwrap();
        let mut offsets = vec![0, bytes.len()];
        let mut offset = 0;
        for event in &fixture.events {
            if let FixtureEvent::Data { data, .. } = event {
                offset += decode_chunk(data).unwrap().len();
                offsets.push(offset);
            }
        }
        let mut seed = 0x0006_451b_u64;
        for _ in 0..200 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            offsets.push(seed as usize % (bytes.len() + 1));
        }
        offsets.sort_unstable();
        offsets.dedup();
        for offset in offsets {
            let mut original = new_term(fixture.header.cols, fixture.header.rows);
            let mut parser = Processor::new();
            let mut scanner = BoundaryScanner::default();
            let mut remaining = offset;
            let mut resume = (fixture.events.len(), 0);
            for (index, event) in fixture.events.iter().enumerate() {
                if remaining == 0 {
                    resume = (index, 0);
                    break;
                }
                if let FixtureEvent::Data { data, .. } = event {
                    let chunk = decode_chunk(data).unwrap();
                    if remaining < chunk.len() {
                        parse(
                            &mut original,
                            &mut parser,
                            &mut scanner,
                            &chunk[..remaining],
                        );
                        resume = (index, remaining);
                        break;
                    }
                    remaining -= chunk.len();
                }
                apply_event(&mut original, &mut parser, &mut scanner, event);
            }
            let (mut restored, mut restored_parser, mut restored_scanner) =
                restore(&original, &parser, &scanner);
            equal(
                &original,
                &restored,
                &format!("{} offset {offset} immediate", path.display()),
            );
            for (index, event) in fixture.events.iter().enumerate().skip(resume.0) {
                if index == resume.0 && resume.1 != 0 {
                    let FixtureEvent::Data { data, .. } = event else {
                        unreachable!();
                    };
                    let chunk = decode_chunk(data).unwrap();
                    parse(&mut original, &mut parser, &mut scanner, &chunk[resume.1..]);
                    parse(
                        &mut restored,
                        &mut restored_parser,
                        &mut restored_scanner,
                        &chunk[resume.1..],
                    );
                } else {
                    apply_event(&mut original, &mut parser, &mut scanner, event);
                    apply_event(
                        &mut restored,
                        &mut restored_parser,
                        &mut restored_scanner,
                        event,
                    );
                }
            }
            parser.stop_sync(&mut original);
            restored_parser.stop_sync(&mut restored);
            equal(
                &original,
                &restored,
                &format!("{} offset {offset}", path.display()),
            );
        }
    }
}

#[test]
fn partial_sequences_and_modes_round_trip_at_every_byte() {
    let bytes=b"\x1b[?1h\x1b[?2004h\x1b[?1002h\x1b[?1006h\x1b[?1004h\x1b=\x1b[3;8r\x1b[?6h\x1b[>7u\x1b[>1u\x1b)0\x0eq\x1b[?2026h\x1b[31mheld\x1b[?2026l\x1b]2;title\x1b\\\x1b[<1u\x0fq";
    for offset in 0..=bytes.len() {
        let mut original = new_term(20, 10);
        let mut parser = Processor::new();
        let mut scanner = BoundaryScanner::default();
        parse(&mut original, &mut parser, &mut scanner, &bytes[..offset]);
        let (mut restored, mut restored_parser, _) = restore(&original, &parser, &scanner);
        equal(&original, &restored, &format!("byte {offset} immediate"));
        parser.advance(&mut original, &bytes[offset..]);
        restored_parser.advance(&mut restored, &bytes[offset..]);
        parser.stop_sync(&mut original);
        restored_parser.stop_sync(&mut restored);
        equal(&original, &restored, &format!("byte {offset}"));
    }
}

fn apply_event(
    term: &mut Term<VoidListener>,
    parser: &mut Processor,
    scanner: &mut BoundaryScanner,
    event: &FixtureEvent,
) {
    match event {
        FixtureEvent::Data { data, .. } => {
            parse(term, parser, scanner, &decode_chunk(data).unwrap())
        }
        FixtureEvent::Resize { cols, rows, .. } => {
            term.resize(TermSize::new(*cols as usize, *rows as usize))
        }
        _ => {}
    }
}

#[test]
fn saved_alternate_cursor_survives_attach_on_the_primary_screen() {
    let bytes = b"primary\x1b[?1049h\x1b[4;6H\x1b[31m\x1b7\x1b[?1049lmore\x1b[?1049h\x1b8X";
    for offset in 0..=bytes.len() {
        let mut original = new_term(20, 10);
        let mut parser = Processor::new();
        let mut scanner = BoundaryScanner::default();
        parse(&mut original, &mut parser, &mut scanner, &bytes[..offset]);
        let (mut restored, mut restored_parser, _) = restore(&original, &parser, &scanner);
        equal(&original, &restored, &format!("byte {offset} immediate"));
        parser.advance(&mut original, &bytes[offset..]);
        restored_parser.advance(&mut restored, &bytes[offset..]);
        equal(
            &original,
            &restored,
            &format!("alternate saved cursor {offset}"),
        );
    }
}

#[test]
fn resize_during_a_partial_sequence_or_held_update_round_trips() {
    let mut original = new_term(20, 10);
    let mut parser = Processor::new();
    let mut scanner = BoundaryScanner::default();
    for bytes in [
        b"wrapped text at initial width".as_slice(),
        b"\x1b[?2026hheld",
        b"\x1b[38;2;",
        b"120;80;40mcolour",
        b"\x1b[?2026l",
    ] {
        parse(&mut original, &mut parser, &mut scanner, bytes);
        for (cols, rows) in [(12, 8), (30, 14)] {
            original.resize(TermSize::new(cols, rows));
            let (mut restored, mut restored_parser, _) = restore(&original, &parser, &scanner);
            equal(&original, &restored, "resize unfinished immediate");
            let suffix = b"\x1b[?2026l";
            parser.advance(&mut original, suffix);
            scanner.advance(suffix, None);
            scanner.end_sync();
            restored_parser.advance(&mut restored, suffix);
            equal(&original, &restored, "resize unfinished");
        }
    }
}

fn every_split(bytes: &[u8], cols: u16, rows: u16, context: &str) {
    for offset in 0..=bytes.len() {
        let mut original = new_term(cols, rows);
        let mut parser = Processor::new();
        let mut scanner = BoundaryScanner::default();
        parse(&mut original, &mut parser, &mut scanner, &bytes[..offset]);
        let (mut restored, mut replay, mut boundary) = restore(&original, &parser, &scanner);
        equal(
            &original,
            &restored,
            &format!("{context} byte {offset} immediate"),
        );
        assert_eq!(parser.preceding_char(), replay.preceding_char());
        parse(&mut original, &mut parser, &mut scanner, &bytes[offset..]);
        parse(&mut restored, &mut replay, &mut boundary, &bytes[offset..]);
        parser.stop_sync(&mut original);
        replay.stop_sync(&mut restored);
        equal(
            &original,
            &restored,
            &format!("{context} byte {offset} continued"),
        );
        assert_eq!(parser.preceding_char(), replay.preceding_char());
    }
}

#[test]
fn tabs_preserve_cells_styles_stops_history_and_continuation() {
    for bytes in [
        b"a\tb".as_slice(),
        b"\x1b[3g\x1b[5G\x1bH\x1b[11G\x1bH\rA\tB\tC",
        b"\x1b[20G\t\rnext",
        b"\x1b[31;44m \r\t\x1b[0mend",
        b"a\tb\r\na\tb\r\na\tb\r\na\tb\r\na\tb\r\na\tb\tend",
    ] {
        every_split(bytes, 20, 5, "tabs");
    }
}

#[test]
fn rep_continues_the_parser_character_after_moves_charset_changes_and_held_updates() {
    for bytes in [
        b"abc\x1b[3b".as_slice(),
        b"abc\x1b[H\x1b[3b",
        b"q\x1b(0\x1b[3b",
        b"\x1b(0q\x1b(B\x1b[3b",
        b"z\x1b[2J\x1b[H\x1b[3b",
        b"\x1b[3b",
        "e\u{301}\x1b[3b".as_bytes(),
        b"old\x1b[?2026hnew\x1b[3b\x1b[?2026l\x1b[3b",
    ] {
        every_split(bytes, 20, 5, "REP");
    }
}

#[test]
fn controls_in_unfinished_sequences_are_not_executed_twice() {
    for control in [b'\n', b'\r', 8, b'\t'] {
        for (prefix, suffix) in [
            (b"hello\x1b".as_slice(), b"[31mX".as_slice()),
            (b"hello\x1b(", b"0q"),
            (b"hello\x1b[", b"31mX"),
            (b"hello\x1b]2;", b"title\x07X"),
            (b"hello\x1bP1;", b"2qignored\x1b\\X"),
            (b"hello\x1bPq", b"ignored\x1b\\X"),
            (b"hello\x1b]2;title\x1b", b"\\X"),
            (b"hello\x1bPqignored\x1b", b"\\X"),
            (b"hello\x1bX", b"ignored\x1b\\X"),
            (b"hello\x1b^", b"ignored\x1b\\X"),
            (b"hello\x1b_", b"ignored\x1b\\X"),
            (b"hello\x1b[?2026h\x1b[", b"31mX\x1b[?2026l"),
        ] {
            let mut bytes = prefix.to_vec();
            bytes.push(control);
            bytes.extend_from_slice(suffix);
            every_split(&bytes, 20, 5, &format!("C0 {control} in {prefix:?}"));
        }
    }
}

#[test]
fn synchronized_capacity_flushes_and_restarts_preserve_immediate_and_continued_state() {
    let mut first = b"\x1b[?2026h".to_vec();
    first.resize(1024 * 1024, b'x');
    let mut second = vec![b'y'; 1024 * 1024 - 12];
    second.extend_from_slice(b"\x1b[?2026hheld");
    for last in [
        b"z\x1b[?2026hnew".as_slice(),
        b"z\x1b[?2026h\x1b[31",
        b"zzzzzzz",
    ] {
        let chunks = [first.as_slice(), second.as_slice(), last];
        for checkpoint in 0..=chunks.len() {
            let mut original = new_term(20, 5);
            let mut parser = Processor::new();
            let mut scanner = BoundaryScanner::default();
            for chunk in &chunks[..checkpoint] {
                parse(&mut original, &mut parser, &mut scanner, chunk);
            }
            let (mut restored, mut replay, mut boundary) = restore(&original, &parser, &scanner);
            let context = format!("capacity {last:?}, checkpoint {checkpoint}");
            equal(&original, &restored, &format!("{context} immediate"));
            assert_eq!(
                parser.sync_bytes_count(),
                replay.sync_bytes_count(),
                "{context}"
            );
            assert_eq!(scanner.unfinished(), boundary.unfinished(), "{context}");
            if checkpoint == chunks.len() && last.starts_with(b"z\x1b") {
                assert_eq!(scanner.unfinished().len(), 8 + parser.sync_bytes_count());
                assert!(parser.sync_bytes_count() <= 4);
            }
            for chunk in &chunks[checkpoint..] {
                parse(&mut original, &mut parser, &mut scanner, chunk);
                parse(&mut restored, &mut replay, &mut boundary, chunk);
            }
            let suffix = b"m\nX\x1b[2b\x1b[?2026l\r\ncontinued";
            parse(&mut original, &mut parser, &mut scanner, suffix);
            parse(&mut restored, &mut replay, &mut boundary, suffix);
            equal(&original, &restored, &format!("{context} continued"));
            assert_eq!(parser.sync_bytes_count(), 0);
            assert_eq!(replay.sync_bytes_count(), 0);
        }
    }
    for bytes in [
        b"old\x1b[?2026hheld\x1b[?2026l\x1b[?2026hnew\x1b[?2026l".as_slice(),
        b"old\x1b[?2026;2004hheld\x1b[?2026l",
    ] {
        every_split(bytes, 20, 5, "synchronized restart");
    }
}
