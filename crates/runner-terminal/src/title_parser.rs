//! OSC 0/2, title push/pop (`CSI 22/23 t`), and RIS (`ESC c`) title stream
//! parser backed by `libghostty-vt`.
//!
//! Keeps VT sequence parsing inside `runner-terminal` so callers such as
//! runtime title classifiers never depend on any VT engine directly and share
//! Ghostty's exact VT500/OSC semantics without a duplicate byte scanner.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::OnceLock;
use std::thread;

use crate::vt::{Terminal, TerminalConfig, TerminalEffect};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TitleEvent {
    Set(String),
    Reset,
}

enum WorkerCmd {
    Advance {
        id: u64,
        bytes: Vec<u8>,
        reply: Sender<Vec<TitleEvent>>,
    },
    Drop(u64),
}

static NEXT_PARSER_ID: AtomicU64 = AtomicU64::new(1);
static WORKER: OnceLock<Sender<WorkerCmd>> = OnceLock::new();

fn worker_tx() -> &'static Sender<WorkerCmd> {
    WORKER.get_or_init(|| {
        crate::vt::init_sys();
        let (tx, rx) = mpsc::channel::<WorkerCmd>();
        thread::Builder::new()
            .name("native-term-title-parser".into())
            .spawn(move || {
                let mut terminals: HashMap<u64, Terminal> = HashMap::new();
                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        WorkerCmd::Advance { id, bytes, reply } => {
                            let term = terminals.entry(id).or_insert_with(|| {
                                Terminal::new(&TerminalConfig {
                                    cols: 1,
                                    rows: 1,
                                    scrollback_lines: 0,
                                    continuation_max_bytes: 0,
                                    kitty_image_storage_limit: 0,
                                    enable_host_callbacks: false,
                                    enable_title_events: true,
                                    enable_render_hold: false,
                                    ..TerminalConfig::default()
                                })
                                .expect("create title parser libghostty-vt terminal")
                            });
                            term.vt_write(&bytes);
                            let mut events = Vec::new();
                            for effect in term.take_effects() {
                                match effect {
                                    TerminalEffect::TitleChanged(title) => {
                                        if title.is_empty() {
                                            events.push(TitleEvent::Reset);
                                        } else {
                                            events.push(TitleEvent::Set(title));
                                        }
                                    }
                                    TerminalEffect::Reset => events.push(TitleEvent::Reset),
                                    _ => {}
                                }
                            }
                            let _ = reply.send(events);
                        }
                        WorkerCmd::Drop(id) => {
                            terminals.remove(&id);
                        }
                    }
                }
            })
            .expect("spawn title parser worker thread");
        tx
    })
}

#[derive(Default)]
pub struct TitleParser {
    id: Option<u64>,
}

impl TitleParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn advance(&mut self, bytes: &[u8]) -> Vec<TitleEvent> {
        if bytes.is_empty() {
            return Vec::new();
        }
        let id = *self
            .id
            .get_or_insert_with(|| NEXT_PARSER_ID.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = mpsc::channel();
        if worker_tx()
            .send(WorkerCmd::Advance {
                id,
                bytes: bytes.to_vec(),
                reply: tx,
            })
            .is_err()
        {
            return Vec::new();
        }
        rx.recv().unwrap_or_default()
    }
}

impl Drop for TitleParser {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            if let Some(tx) = WORKER.get() {
                let _ = tx.send(WorkerCmd::Drop(id));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TitleEvent, TitleParser};

    #[test]
    fn parses_osc0_osc2_bel_st_and_preserves_semicolons() {
        let mut parser = TitleParser::new();
        assert_eq!(
            parser.advance(b"\x1b]0;project; branch\x07"),
            vec![TitleEvent::Set("project; branch".into())]
        );
        assert_eq!(
            parser.advance(b"\x1b]2;another;title\x1b\\"),
            vec![TitleEvent::Set("another;title".into())]
        );
        assert_eq!(parser.advance(b"\x1b]7;file:///tmp\x07"), Vec::new());
    }

    #[test]
    fn matches_ghostty_vt500_esc_termination_and_title_stack() {
        let mut parser = TitleParser::new();
        assert_eq!(
            parser.advance(b"\x1b]2;one\x1b[31m"),
            vec![TitleEvent::Set("one".into())]
        );
        assert_eq!(
            parser.advance(b"\x1b]2;one\x1b]2;two\x07"),
            vec![TitleEvent::Set("one".into()), TitleEvent::Set("two".into())]
        );
        assert_eq!(
            parser.advance(b"\x1b]2;A\x07\x1b[22;0t\x1b]2;B\x07\x1b[23;0t"),
            vec![
                TitleEvent::Set("A".into()),
                TitleEvent::Set("B".into()),
                TitleEvent::Set("A".into()),
            ]
        );
    }

    #[test]
    fn parses_split_osc_across_every_byte_boundary() {
        let sequence = b"\x1b]0;\xe2\xa0\x99 project; branch\x07";
        for split in 1..sequence.len() {
            let mut parser = TitleParser::new();
            assert_eq!(
                parser.advance(&sequence[..split]),
                Vec::new(),
                "split {split}"
            );
            assert_eq!(
                parser.advance(&sequence[split..]),
                vec![TitleEvent::Set("⠙ project; branch".into())],
                "split {split}"
            );
        }
    }

    #[test]
    fn empty_and_ris_emit_reset_and_invalid_utf8_is_ignored() {
        let mut parser = TitleParser::new();
        assert_eq!(parser.advance(b"\x1b]0;\x07"), vec![TitleEvent::Reset]);
        // Ghostty validates UTF-8 before dispatching `window_title` and ignores
        // invalid UTF-8 titles without disturbing title authority.
        assert_eq!(parser.advance(b"\x1b]2;\xff\xfe\x07"), Vec::new());
        assert_eq!(parser.advance(b"\x1bc"), vec![TitleEvent::Reset]);
    }
}
