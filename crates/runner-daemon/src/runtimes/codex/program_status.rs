// Codex rust-v0.163.0 terminal_program_status.rs. Keep raw bytes: terminal
// parsers may discard embedded controls before dispatching an OSC payload.
#[derive(Default)]
pub(super) struct ProgramStatusParser {
    state: State,
    payload: Vec<u8>,
    owned: bool,
    // Candidate controls are withheld across chunks until recognized or rejected.
    pending_bytes: usize,
    activity_bytes: usize,
}

#[derive(Default, Clone, Copy)]
enum State {
    #[default]
    Ground,
    Escape,
    Osc,
    OscEscape,
    Discard,
    DiscardEscape,
    String,
    StringEscape,
}

impl ProgramStatusParser {
    pub(super) fn activity_bytes(&self) -> usize {
        self.activity_bytes
    }

    pub(super) fn observe(&mut self, bytes: &[u8]) -> Option<bool> {
        self.activity_bytes = 0;
        let mut report = None;
        for &byte in bytes {
            self.state = match self.state {
                State::Ground if byte == 0x1b => {
                    self.pending_bytes = 1;
                    State::Escape
                }
                State::Escape => match byte {
                    b']' => {
                        self.pending_bytes += 1;
                        self.payload.clear();
                        State::Osc
                    }
                    0x1b => {
                        self.activity_bytes += 1;
                        State::Escape
                    }
                    _ => {
                        self.activity_bytes += self.pending_bytes + 1;
                        self.pending_bytes = 0;
                        if matches!(byte, b'P' | b'X' | b'^' | b'_') {
                            State::String
                        } else {
                            State::Ground
                        }
                    }
                },
                State::Osc => {
                    self.pending_bytes += 1;
                    match byte {
                        0x07 => {
                            report = self.finish().or(report);
                            State::Ground
                        }
                        0x1b => State::OscEscape,
                        0x20..=0x7e if self.payload.len() < 64 => {
                            self.payload.push(byte);
                            if self.candidate() {
                                State::Osc
                            } else {
                                self.release_activity();
                                State::Discard
                            }
                        }
                        _ => {
                            self.release_activity();
                            State::Discard
                        }
                    }
                }
                State::OscEscape => {
                    self.pending_bytes += 1;
                    if byte == b'\\' {
                        report = self.finish().or(report);
                        State::Ground
                    } else {
                        self.release_activity();
                        match byte {
                            0x07 => State::Ground,
                            0x1b => State::DiscardEscape,
                            _ => State::Discard,
                        }
                    }
                }
                State::Discard | State::DiscardEscape => {
                    self.activity_bytes += 1;
                    match byte {
                        0x07 => State::Ground,
                        b'\\' if matches!(self.state, State::DiscardEscape) => State::Ground,
                        0x1b => State::DiscardEscape,
                        _ => State::Discard,
                    }
                }
                State::String | State::StringEscape => {
                    self.activity_bytes += 1;
                    match byte {
                        b'\\' if matches!(self.state, State::StringEscape) => State::Ground,
                        0x1b => State::StringEscape,
                        _ => State::String,
                    }
                }
                State::Ground => {
                    self.activity_bytes += 1;
                    State::Ground
                }
            };
        }
        report
    }

    fn candidate(&self) -> bool {
        [
            b"7501;state=blocked:app=codex".as_slice(),
            b"7501;state=working:app=codex",
            b"7501;state=idle:app=codex",
            b"7501;state=clear",
        ]
        .iter()
        .any(|payload| payload.starts_with(&self.payload))
    }

    fn release_activity(&mut self) {
        self.activity_bytes += self.pending_bytes;
        self.pending_bytes = 0;
    }

    fn finish(&mut self) -> Option<bool> {
        let report = self.report();
        if report.is_some() || self.payload == b"7501;state=clear" {
            self.pending_bytes = 0;
        } else {
            self.release_activity();
        }
        report
    }

    fn report(&mut self) -> Option<bool> {
        match self.payload.as_slice() {
            b"7501;state=blocked:app=codex" => {
                self.owned = true;
                Some(true)
            }
            b"7501;state=working:app=codex" | b"7501;state=idle:app=codex" => {
                self.owned = true;
                Some(false)
            }
            b"7501;state=clear" if self.owned => {
                self.owned = false;
                Some(false)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_reports_survive_every_chunk_boundary_and_both_terminators() {
        for terminator in ["\x07", "\x1b\\"] {
            for state in ["blocked", "working", "idle"] {
                let report = format!("\x1b]7501;state={state}:app=codex{terminator}");
                for split in 0..=report.len() {
                    let mut parser = ProgramStatusParser::default();
                    let first = parser.observe(&report.as_bytes()[..split]);
                    let second = parser.observe(&report.as_bytes()[split..]);
                    assert_eq!(second.or(first), Some(state == "blocked"));
                    assert_eq!(parser.observe(b"\x1b]7501;state=clear\x1b\\"), Some(false));
                    assert_eq!(parser.observe(b"\x1b]7501;state=clear\x07"), None);
                }
            }
        }
    }

    #[test]
    fn rejects_unowned_malformed_oversized_and_control_containing_reports() {
        let mut parser = ProgramStatusParser::default();
        for payload in [
            "7501;state=clear",
            "7501;state=blocked:app=other",
            "7501;state=blocked",
            "7501;state=blocked:app=codex:extra=true",
            "7501;state=blocked:app=codex;",
            "7501;state=BLOCKED:app=codex",
            "7501;state=blocked:app=co\ndex",
            "7501;state=blocked:app=co\x00dex",
            "7501;state=blocked:app=codex\x7f",
            "2;state=blocked:app=codex",
        ] {
            assert_eq!(
                parser.observe(format!("\x1b]{payload}\x07").as_bytes()),
                None,
                "{payload:?}"
            );
        }
        let oversized = format!(
            "\x1b]{}7501;state=blocked:app=codex\x1b\\",
            "x".repeat(100_000)
        );
        assert_eq!(parser.observe(oversized.as_bytes()), None);
        assert!(parser.payload.len() <= 64);
        assert_eq!(
            parser.observe(b"\x1bP\x1b]7501;state=blocked:app=codex\x07\x1b\\"),
            None
        );
        assert_eq!(
            parser.observe(b"\x1b]7501;state=blocked:app=codex\x07"),
            Some(true)
        );
    }

    #[test]
    fn native_controls_are_not_activity_but_other_output_is_preserved_at_every_split() {
        for payload in [
            "7501;state=blocked:app=codex",
            "7501;state=working:app=codex",
            "7501;state=idle:app=codex",
            "7501;state=clear",
        ] {
            for terminator in ["\x07", "\x1b\\"] {
                let control = format!("\x1b]{payload}{terminator}");
                for split in 0..=control.len() {
                    let mut parser = ProgramStatusParser::default();
                    parser.observe(&control.as_bytes()[..split]);
                    assert_eq!(parser.activity_bytes(), 0);
                    parser.observe(&control.as_bytes()[split..]);
                    assert_eq!(parser.activity_bytes(), 0);
                    let mixed = format!("before{control}after");
                    let mut count = 0;
                    for byte in mixed.as_bytes() {
                        parser.observe(&[*byte]);
                        count += parser.activity_bytes();
                    }
                    assert_eq!(count, b"beforeafter".len());
                }
            }
        }
        for payload in [
            "2;custom title",
            "7501;state=blocked:app=other",
            "7501;state=blocked:app=codex:extra=true",
            "7501;state=blocked:app=co\ndex",
        ] {
            let control = format!("\x1b]{payload}\x07");
            let mut parser = ProgramStatusParser::default();
            let mut count = 0;
            for byte in control.as_bytes() {
                assert_eq!(parser.observe(&[*byte]), None);
                count += parser.activity_bytes();
            }
            assert_eq!(count, control.len());
        }
    }

    #[test]
    fn last_valid_report_in_one_chunk_wins() {
        let mut parser = ProgramStatusParser::default();
        assert_eq!(
            parser.observe(
                b"\x1b]7501;state=blocked:app=codex\x07\x1b]7501;state=idle:app=codex\x1b\\"
            ),
            Some(false)
        );
    }
}
