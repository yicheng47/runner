//! Regression for the review finding: OSC 10/11/12 color queries hit
//! named color-table slots (256 Foreground, 257 Background, 258 Cursor),
//! which must not be clamped into the 0-255 palette — and must prefer
//! runtime OSC overrides stored in the terminal over the static defaults.
//! Replies are produced through the production `query_color_for` path
//! against real terminal state. Also pins the DA query path (`PtyWrite`).

use runner_terminal::palette;
use runner_terminal::replay::{new_term, Terminal, TerminalReply};

fn feed(bytes: &[u8]) -> Terminal {
    let mut term = new_term(80, 24);
    term.feed(bytes);
    term
}

/// Resolve the first ColorRequest the way the event pump does: the
/// production `query_color_for` against the live terminal.
fn color_reply(term: &Terminal, expected_index: usize) -> String {
    let base = palette::base_palette();
    for event in term.drain_replies(&base, palette::RUNNER) {
        if let TerminalReply::ColorQuery { index, reply } = event {
            assert_eq!(index, expected_index, "unexpected color-table index");
            return reply;
        }
    }
    panic!("no ColorQuery reply for index {expected_index}");
}

#[test]
fn osc10_reports_runner_foreground() {
    let term = feed(b"\x1b]10;?\x07");
    let reply = color_reply(&term, 256);
    assert!(
        reply.contains("dcdc/dcdc/e0e0"),
        "OSC 10 should report the Runner foreground, got {reply:?}"
    );
}

#[test]
fn osc11_reports_runner_background() {
    let term = feed(b"\x1b]11;?\x07");
    let reply = color_reply(&term, 257);
    assert!(
        reply.contains("1515/1616/1b1b"),
        "OSC 11 should report the Runner background, got {reply:?}"
    );
}

#[test]
fn osc12_reports_cursor_color() {
    let term = feed(b"\x1b]12;?\x07");
    let reply = color_reply(&term, 258);
    assert!(
        reply.contains("0000/ffff/9c9c"),
        "OSC 12 should report the cursor color, got {reply:?}"
    );
}

#[test]
fn osc4_reports_palette_slot() {
    // Slot 1 = Runner ANSI red (#ff4d6d).
    let term = feed(b"\x1b]4;1;?\x07");
    let reply = color_reply(&term, 1);
    assert!(
        reply.contains("ffff/4d4d/6d6d"),
        "OSC 4;1 should report ANSI red, got {reply:?}"
    );
}

#[test]
fn osc10_set_then_query_reports_override() {
    // A program sets the foreground via OSC 10, then queries it: the
    // reply must be the runtime override, not the static default.
    let term = feed(b"\x1b]10;#ff8800\x07\x1b]10;?\x07");
    let reply = color_reply(&term, 256);
    assert!(
        reply.contains("ffff/8888/0000"),
        "OSC 10 query after set should report the override, got {reply:?}"
    );
}

#[test]
fn osc4_set_then_query_reports_override() {
    let term = feed(b"\x1b]4;1;#102030\x07\x1b]4;1;?\x07");
    let reply = color_reply(&term, 1);
    assert!(
        reply.contains("1010/2020/3030"),
        "OSC 4;1 query after set should report the override, got {reply:?}"
    );
}

#[test]
fn osc104_reset_restores_default() {
    // Set slot 1, reset it (OSC 104), query again: back to defaults.
    let term = feed(b"\x1b]4;1;#102030\x07\x1b]104;1\x07\x1b]4;1;?\x07");
    let reply = color_reply(&term, 1);
    assert!(
        reply.contains("ffff/4d4d/6d6d"),
        "OSC 4;1 after reset should report the default, got {reply:?}"
    );
}

#[test]
fn resolve_index_named_slots() {
    let base = palette::base_palette();
    assert_eq!(palette::resolve_index(256, &base), palette::FOREGROUND);
    assert_eq!(palette::resolve_index(257, &base), palette::BACKGROUND);
    assert_eq!(palette::resolve_index(258, &base), palette::CURSOR);
    assert_eq!(palette::resolve_index(1, &base), base[1]);
}

#[test]
fn primary_da_query_emits_pty_write() {
    let term = feed(b"\x1b[c");
    let base = palette::base_palette();
    let reply = term
        .drain_replies(&base, palette::RUNNER)
        .into_iter()
        .find_map(|event| match event {
            TerminalReply::PtyWrite(s) => Some(s),
            _ => None,
        })
        .expect("DA query should produce a PtyWrite reply");
    assert!(
        reply.starts_with("\x1b[?"),
        "DA reply should be a CSI ? response, got {reply:?}"
    );
}
