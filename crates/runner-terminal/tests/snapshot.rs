use std::path::{Path, PathBuf};

use runner_terminal::fixtures::{decode_chunk, Fixture, FixtureEvent};
use runner_terminal::replay::Terminal;
use runner_terminal::snapshot::{serialize, CellSnapshot, CursorSnapshot, GridSnapshot};
use runner_terminal::TerminalModeFlags;

fn new_term(cols: u16, rows: u16) -> Terminal {
    Terminal::with_kitty_keyboard(cols, rows)
}

fn cell_equal(a: &CellSnapshot, b: &CellSnapshot, context: &str) {
    assert_eq!(
        (
            a.c,
            a.fg,
            a.bg,
            a.flags,
            a.zerowidth.as_slice(),
            a.underline_color,
            &a.hyperlink
        ),
        (
            b.c,
            b.fg,
            b.bg,
            b.flags,
            b.zerowidth.as_slice(),
            b.underline_color,
            &b.hyperlink
        ),
        "{context}"
    );
}

fn cursor_equal(a: &CursorSnapshot, b: &CursorSnapshot, context: &str) {
    assert_eq!(a.point, b.point, "{context}: point");
    assert_eq!(a.charsets, b.charsets, "{context}: charsets");
    assert_eq!(a.input_needs_wrap, b.input_needs_wrap, "{context}: wrap");
    cell_equal(&a.template, &b.template, context);
}

fn grid_equal(a: &GridSnapshot, b: &GridSnapshot, context: &str) {
    assert_eq!(a.history_size, b.history_size, "{context}: history");
    assert_eq!(a.rows.len(), b.rows.len(), "{context}: row count");
    cursor_equal(&a.cursor, &b.cursor, &format!("{context}: cursor"));
    cursor_equal(
        &a.saved_cursor,
        &b.saved_cursor,
        &format!("{context}: saved cursor"),
    );
    for (row_index, (row_a, row_b)) in a.rows.iter().zip(b.rows.iter()).enumerate() {
        let line = row_index as i32 - a.history_size as i32;
        assert_eq!(
            row_a.len(),
            row_b.len(),
            "{context}: row {line} column count"
        );
        for (column, (cell_a, cell_b)) in row_a.iter().zip(row_b.iter()).enumerate() {
            cell_equal(cell_a, cell_b, &format!("{context}: cell {line}:{column}"));
        }
    }
    assert_eq!(a, b, "{context}: grid snapshot");
}

fn equal(a: &Terminal, b: &Terminal, context: &str) {
    let x = a.snapshot_state();
    let y = b.snapshot_state();
    assert_eq!(x.mode, y.mode, "{context}: modes");
    grid_equal(&x.active_grid, &y.active_grid, context);
    if x.mode.contains(TerminalModeFlags::ALT_SCREEN) {
        grid_equal(
            x.inactive_grid.as_ref().expect("inactive grid"),
            y.inactive_grid.as_ref().expect("inactive grid"),
            context,
        );
    }
    cursor_equal(
        &x.inactive_saved_cursor,
        &y.inactive_saved_cursor,
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

fn restore(original: &Terminal) -> Terminal {
    let bytes = original.encode_binary_snapshot();
    Terminal::from_binary_snapshot(&bytes, 10_000).unwrap()
}

#[test]
fn companion_vt_formatter_round_trips_visible_screen_and_modes() {
    let mut original = new_term(30, 6);
    original.advance(b"\x1b[?2004h\x1b[31mhello\x1b[0m world\r\nsecond");
    let bytes = serialize(&original, original.unfinished());
    let mut restored = new_term(30, 6);
    restored.advance(&bytes);
    assert_eq!(
        original.screen().visible_lines(),
        restored.screen().visible_lines()
    );
    assert_eq!(original.screen().mode(), restored.screen().mode());
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

#[test]
fn recording_round_trips() {
    for path in recordings() {
        let fixture = Fixture::load(&path).unwrap();
        let mut original = new_term(fixture.header.cols, fixture.header.rows);
        for event in &fixture.events {
            apply_event(&mut original, event);
        }
        let restored = restore(&original);
        equal(&original, &restored, path.to_str().unwrap());
    }
}

#[test]
fn hyperlinks_preserve_cells_cursors_screens_and_continuation() {
    let bytes = b"\x1b]8;;https://example.com/anonymous\x1b\\anonymous\x1b]8;;\x1b\\ plain\r\n\x1b]8;id=saved;file:///tmp/example.rs#L12\x1b\\saved\x1b7\x1b]8;;\x1b\\\r\n\x1b]8;id=edge;https://example.com/edge\x1b\\\x1b[3;20HX\x1b[?1049h\x1b]8;id=alt;https://example.com/alt\x1b\\alternate\x1b7\x1b]8;id=active;https://example.com/active\x1b\\\x1b[2;20HZ";
    let mut original = new_term(20, 4);
    original.advance(bytes);
    let mut restored = restore(&original);
    equal(&original, &restored, "hyperlink snapshot");
    for bytes in [
        b"wrap\x1b]8;;\x1b\\ plain".as_slice(),
        b"\x1b8continued\x1b]8;;\x1b\\",
        b"\x1b[?1049l\x1b8continued\x1b]8;;\x1b\\ plain",
    ] {
        original.advance(bytes);
        restored.advance(bytes);
        equal(&original, &restored, "hyperlink continuation");
    }
}

macro_rules! split_recordings {
    ($($name:ident => $file:literal),+ $(,)?) => {
        mod recording_split_points {
            use super::*;
            $(#[test]
            fn $name() {
                super::recording_split_points($file);
            })+

            #[test]
            fn all_recordings_are_covered() {
                let mut expected = vec![$($file),+];
                expected.sort_unstable();
                assert_eq!(
                    recordings().iter().map(|path| path.file_name().unwrap().to_str().unwrap()).collect::<Vec<_>>(),
                    expected
                );
            }
        }
    };
}

split_recordings! {
    fixture_645_m2_claude_clear_resize => "645-m2-claude-clear-resize.ndjson",
    fixture_645_m2_claude_long_transcript => "645-m2-claude-long-transcript.ndjson",
    fixture_645_m2_codex_alt_resize => "645-m2-codex-alt-resize.ndjson",
    fixture_645_m2_pi_shift_enter => "645-m2-pi-shift-enter.ndjson",
    fixture_645_m2_zsh_less_vim => "645-m2-zsh-less-vim.ndjson",
    grok_first_turn => "grok-first-turn.ndjson",
    agy_first_turn => "agy-first-turn.ndjson",
    claude_session => "claude-session.ndjson",
    codex_title_working => "codex-title-working.ndjson",
    copilot_first_turn => "copilot-first-turn.ndjson",
    input_claude_control_submit => "input-claude-control-submit.ndjson",
    input_claude_delete => "input-claude-delete.ndjson",
    input_codex_active_tool_paste => "input-codex-active-tool-paste.ndjson",
    input_codex_coalesced_typing => "input-codex-coalesced-typing.ndjson",
    input_codex_filled_submit => "input-codex-filled-submit.ndjson",
    input_codex_fresh_submit => "input-codex-fresh-submit.ndjson",
    input_codex_menu => "input-codex-menu.ndjson",
    input_codex_multiline_paste => "input-codex-multiline-paste.ndjson",
    input_codex_queued_submit => "input-codex-queued-submit.ndjson",
    input_codex_submit_history => "input-codex-submit-history.ndjson",
    input_zsh_submit => "input-zsh-submit.ndjson",
    procedural_glyphs => "procedural-glyphs.ndjson",
    top_busy => "top-busy.ndjson",
    width_torture => "width-torture.ndjson",
}

fn recording_split_points(name: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name);
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
                    original.advance(&chunk[..remaining]);
                    resume = (index, remaining);
                    break;
                }
                remaining -= chunk.len();
            }
            apply_event(&mut original, event);
        }
        let mut restored = restore(&original);
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
                original.advance(&chunk[resume.1..]);
                restored.advance(&chunk[resume.1..]);
            } else {
                apply_event(&mut original, event);
                apply_event(&mut restored, event);
            }
        }
        original.stop_sync();
        restored.stop_sync();
        equal(
            &original,
            &restored,
            &format!("{} offset {offset}", path.display()),
        );
    }
}

#[test]
fn partial_sequences_and_modes_round_trip_at_every_byte() {
    let bytes = b"\x1b[?1h\x1b[?2004h\x1b[?1002h\x1b[?1006h\x1b[?1004h\x1b=\x1b[3;8r\x1b[?6h\x1b[>7u\x1b[>1u\x1b)0\x0eq\x1b[?2026h\x1b[31mheld\x1b[?2026l\x1b]2;title\x1b\\\x1b[<1u\x0fq";
    for offset in 0..=bytes.len() {
        let mut original = new_term(20, 10);
        original.advance(&bytes[..offset]);
        let mut restored = restore(&original);
        equal(&original, &restored, &format!("byte {offset} immediate"));
        original.advance(&bytes[offset..]);
        restored.advance(&bytes[offset..]);
        original.stop_sync();
        restored.stop_sync();
        equal(&original, &restored, &format!("byte {offset}"));
    }
}

fn apply_event(term: &mut Terminal, event: &FixtureEvent) {
    match event {
        FixtureEvent::Data { data, .. } => term.advance(&decode_chunk(data).unwrap()),
        FixtureEvent::Resize { cols, rows, .. } => term.resize(*cols, *rows),
        _ => {}
    }
}

#[test]
fn saved_alternate_cursor_survives_attach_on_the_primary_screen() {
    let bytes = b"primary\x1b[?1049h\x1b[4;6H\x1b[31m\x1b7\x1b[?1049lmore\x1b[?1049h\x1b8X";
    for offset in 0..=bytes.len() {
        let mut original = new_term(20, 10);
        original.advance(&bytes[..offset]);
        let mut restored = restore(&original);
        equal(&original, &restored, &format!("byte {offset} immediate"));
        original.advance(&bytes[offset..]);
        restored.advance(&bytes[offset..]);
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
    for bytes in [
        b"wrapped text at initial width".as_slice(),
        b"\x1b[?2026hheld",
        b"\x1b[38;2;",
        b"120;80;40mcolour",
        b"\x1b[?2026l",
    ] {
        original.advance(bytes);
        for (cols, rows) in [(12, 8), (30, 14)] {
            original.resize(cols, rows);
            let mut restored = restore(&original);
            equal(&original, &restored, "resize unfinished immediate");
            let suffix = b"\x1b[?2026l";
            original.advance(suffix);
            original.end_sync();
            restored.advance(suffix);
            equal(&original, &restored, "resize unfinished");
        }
    }
}

fn every_split(bytes: &[u8], cols: u16, rows: u16, context: &str) {
    for offset in 0..=bytes.len() {
        let mut original = new_term(cols, rows);
        original.advance(&bytes[..offset]);
        let mut restored = restore(&original);
        equal(
            &original,
            &restored,
            &format!("{context} byte {offset} immediate"),
        );
        assert_eq!(original.preceding_char(), restored.preceding_char());
        original.advance(&bytes[offset..]);
        restored.advance(&bytes[offset..]);
        original.stop_sync();
        restored.stop_sync();
        equal(
            &original,
            &restored,
            &format!("{context} byte {offset} continued"),
        );
        assert_eq!(original.preceding_char(), restored.preceding_char());
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
            for chunk in &chunks[..checkpoint] {
                original.advance(chunk);
            }
            let mut restored = restore(&original);
            let context = format!("capacity {last:?}, checkpoint {checkpoint}");
            equal(&original, &restored, &format!("{context} immediate"));
            assert_eq!(
                original.is_sync_held(),
                restored.is_sync_held(),
                "{context}"
            );
            assert_eq!(original.unfinished(), restored.unfinished(), "{context}");
            if checkpoint > 0 {
                assert!(original.is_sync_held(), "{context}: sync stays held");
            }
            for chunk in &chunks[checkpoint..] {
                original.advance(chunk);
                restored.advance(chunk);
            }
            let suffix = b"m\nX\x1b[2b\x1b[?2026l\r\ncontinued";
            original.advance(suffix);
            restored.advance(suffix);
            equal(&original, &restored, &format!("{context} continued"));
            assert!(!original.is_sync_held());
            assert!(!restored.is_sync_held());
        }
    }
    for bytes in [
        b"old\x1b[?2026hheld\x1b[?2026l\x1b[?2026hnew\x1b[?2026l".as_slice(),
        b"old\x1b[?2026;2004hheld\x1b[?2026l",
    ] {
        every_split(bytes, 20, 5, "synchronized restart");
    }
}
