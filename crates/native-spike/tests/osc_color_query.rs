//! Regression for the review finding: OSC 10/11/12 color queries hit
//! alacritty's named color-table slots (256 Foreground, 257
//! Background, 258 Cursor), which must not be clamped into the 0-255
//! palette — and must prefer runtime OSC overrides stored in
//! `Term::colors()` over the static defaults. Replies are produced
//! through the production `query_color` path against real terminal
//! state. Also pins the DA query path (`Event::PtyWrite`).

use std::sync::mpsc::{channel, Receiver, Sender};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;

use native_spike::palette;
use native_spike::terminal::query_color;

struct Proxy(Sender<Event>);

impl EventListener for Proxy {
    fn send_event(&self, event: Event) {
        let _ = self.0.send(event);
    }
}

fn feed(bytes: &[u8]) -> (Term<Proxy>, Receiver<Event>) {
    let (tx, rx) = channel();
    let mut term = Term::new(Config::default(), &TermSize::new(80, 24), Proxy(tx));
    let mut parser: Processor = Processor::new();
    parser.advance(&mut term, bytes);
    (term, rx)
}

/// Resolve the first ColorRequest the way the event pump does: the
/// production `query_color` against the live Term.
fn color_reply(term: &Term<Proxy>, rx: &Receiver<Event>, expected_index: usize) -> String {
    for event in rx.try_iter() {
        if let Event::ColorRequest(index, format) = event {
            assert_eq!(index, expected_index, "unexpected color-table index");
            let base = palette::base_palette();
            return format(query_color(term, index, &base));
        }
    }
    panic!("no ColorRequest event for index {expected_index}");
}

#[test]
fn osc10_reports_runner_foreground() {
    let (term, rx) = feed(b"\x1b]10;?\x07");
    let reply = color_reply(&term, &rx, 256);
    assert!(
        reply.contains("c0c0/caca/f5f5"),
        "OSC 10 should report the Tokyo Night foreground, got {reply:?}"
    );
}

#[test]
fn osc11_reports_runner_background() {
    let (term, rx) = feed(b"\x1b]11;?\x07");
    let reply = color_reply(&term, &rx, 257);
    assert!(
        reply.contains("1a1a/1b1b/2626"),
        "OSC 11 should report the Tokyo Night background, got {reply:?}"
    );
}

#[test]
fn osc12_reports_cursor_color() {
    let (term, rx) = feed(b"\x1b]12;?\x07");
    let reply = color_reply(&term, &rx, 258);
    assert!(
        reply.contains("c0c0/caca/f5f5"),
        "OSC 12 should report the cursor color, got {reply:?}"
    );
}

#[test]
fn osc4_reports_palette_slot() {
    // Slot 1 = ANSI red (Tokyo Night f7768e).
    let (term, rx) = feed(b"\x1b]4;1;?\x07");
    let reply = color_reply(&term, &rx, 1);
    assert!(
        reply.contains("f7f7/7676/8e8e"),
        "OSC 4;1 should report ANSI red, got {reply:?}"
    );
}

#[test]
fn osc10_set_then_query_reports_override() {
    // A program sets the foreground via OSC 10, then queries it: the
    // reply must be the runtime override, not the static default.
    let (term, rx) = feed(b"\x1b]10;#ff8800\x07\x1b]10;?\x07");
    let reply = color_reply(&term, &rx, 256);
    assert!(
        reply.contains("ffff/8888/0000"),
        "OSC 10 query after set should report the override, got {reply:?}"
    );
}

#[test]
fn osc4_set_then_query_reports_override() {
    let (term, rx) = feed(b"\x1b]4;1;#102030\x07\x1b]4;1;?\x07");
    let reply = color_reply(&term, &rx, 1);
    assert!(
        reply.contains("1010/2020/3030"),
        "OSC 4;1 query after set should report the override, got {reply:?}"
    );
}

#[test]
fn osc104_reset_restores_default() {
    // Set slot 1, reset it (OSC 104), query again: back to defaults.
    let (term, rx) = feed(b"\x1b]4;1;#102030\x07\x1b]104;1\x07\x1b]4;1;?\x07");
    let reply = color_reply(&term, &rx, 1);
    assert!(
        reply.contains("f7f7/7676/8e8e"),
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
    let (_term, rx) = feed(b"\x1b[c");
    let reply = rx
        .try_iter()
        .find_map(|event| match event {
            Event::PtyWrite(s) => Some(s),
            _ => None,
        })
        .expect("DA query should produce a PtyWrite reply");
    assert!(
        reply.starts_with("\x1b[?"),
        "DA reply should be a CSI ? response, got {reply:?}"
    );
}
