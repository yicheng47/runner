use std::collections::VecDeque;

use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use base64::Engine as _;

pub const PROBE_REPLY: &str = "\x1b]7501;?\x1b\\";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Working,
    Done,
    Blocked,
    Error,
    Clear,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub state: State,
    pub id: String,
    pub kind: Option<String>,
    pub progress: Option<u8>,
    pub app: Option<String>,
    pub title: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Event {
    Query,
    Changed(Option<Report>),
    Shell(ShellMark),
}

/// A shell's FinalTerm command marks: `133;C` as a command starts and
/// `133;D` once the shell is back. `133;A` is not one, because agent TUIs
/// also emit A, B and C for their own transcript zones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellMark {
    CommandStarted,
    CommandFinished,
}

/// Reads [`ShellMark`]s from a shell's PTY output, across reads.
#[derive(Default)]
pub struct ShellMarks(Parser);

impl ShellMarks {
    /// The command marks in `bytes`, in order.
    pub fn scan(&mut self, bytes: &[u8]) -> Vec<ShellMark> {
        self.0
            .scan(bytes)
            .into_iter()
            .filter_map(|(_, event)| match event {
                Event::Shell(mark) => Some(mark),
                _ => None,
            })
            .collect()
    }
}

#[derive(Default)]
pub(crate) struct Parser {
    mode: Mode,
    payload: Vec<u8>,
    length: usize,
    records: VecDeque<Report>,
    discard_report: bool,
    shell_finished: bool,
}

#[derive(Clone, Copy, Default)]
enum Mode {
    #[default]
    Ground,
    Escape,
    Osc,
    OscEscape,
    String,
    StringEscape,
}

impl Parser {
    pub(crate) fn invalidate_records(&mut self) {
        self.records.clear();
        self.discard_report = matches!(self.mode, Mode::Osc | Mode::OscEscape);
        self.shell_finished = false;
    }

    pub(crate) fn scan(&mut self, bytes: &[u8]) -> Vec<(usize, Event)> {
        let mut events = Vec::new();
        for (index, &byte) in bytes.iter().enumerate() {
            let mut changed = false;
            self.mode = match self.mode {
                Mode::Ground if byte == 0x1b => Mode::Escape,
                Mode::Escape => match byte {
                    b']' => {
                        self.payload.clear();
                        self.length = 2;
                        self.discard_report = false;
                        Mode::Osc
                    }
                    b'P' | b'X' | b'^' | b'_' => Mode::String,
                    b'c' => {
                        changed = !self.records.is_empty();
                        self.records.clear();
                        self.shell_finished = false;
                        Mode::Ground
                    }
                    0x1b => Mode::Escape,
                    _ => Mode::Ground,
                },
                Mode::Osc | Mode::OscEscape => {
                    self.length = (self.length + 1).min(4097);
                    match byte {
                        b'\\' if matches!(self.mode, Mode::OscEscape) => {
                            if let Some(event) = self.finish() {
                                events.push((index + 1, event));
                            }
                            Mode::Ground
                        }
                        0x07 if matches!(self.mode, Mode::Osc) => {
                            if let Some(event) = self.finish() {
                                events.push((index + 1, event));
                            }
                            Mode::Ground
                        }
                        0x18 | 0x1a => Mode::Ground,
                        0x1b if matches!(self.mode, Mode::OscEscape) => Mode::Escape,
                        0x1b => Mode::OscEscape,
                        _ if matches!(self.mode, Mode::OscEscape) => {
                            // An ESC that is not ST aborts this OSC.
                            self.payload.clear();
                            match byte {
                                b']' => {
                                    self.length = 2;
                                    self.discard_report = false;
                                    Mode::Osc
                                }
                                b'P' | b'X' | b'^' | b'_' => Mode::String,
                                b'c' => {
                                    changed = !self.records.is_empty();
                                    self.records.clear();
                                    self.shell_finished = false;
                                    Mode::Ground
                                }
                                _ => Mode::Ground,
                            }
                        }
                        _ => {
                            if self.payload.len() < 4096 {
                                self.payload.push(byte);
                            }
                            Mode::Osc
                        }
                    }
                }
                Mode::String | Mode::StringEscape => match byte {
                    b'\\' if matches!(self.mode, Mode::StringEscape) => Mode::Ground,
                    0x18 | 0x1a => Mode::Ground,
                    0x1b => Mode::StringEscape,
                    _ => Mode::String,
                },
                Mode::Ground => Mode::Ground,
            };
            if changed {
                events.push((index + 1, Event::Changed(self.current())));
            }
        }
        events
    }

    fn finish(&mut self) -> Option<Event> {
        if self.length > 4096 {
            return None;
        }
        if self.payload == b"133;D" || self.payload.starts_with(b"133;D;") {
            self.shell_finished = true;
            return Some(Event::Shell(ShellMark::CommandFinished));
        }
        if self.payload == b"133;C" || self.payload.starts_with(b"133;C;") {
            self.shell_finished = false;
            return Some(Event::Shell(ShellMark::CommandStarted));
        }
        if self.payload == b"133;A" || self.payload.starts_with(b"133;A;") {
            // TUIs also use A/B/C for transcript zones; D distinguishes a shell return.
            if !std::mem::take(&mut self.shell_finished) {
                return None;
            }
            let before = self.records.len();
            self.records
                .retain(|report| matches!(report.state, State::Done | State::Error));
            return (before != self.records.len()).then(|| Event::Changed(self.current()));
        }
        let payload = self.payload.strip_prefix(b"7501;")?;
        if payload == b"?" {
            return Some(Event::Query);
        }
        if self.discard_report {
            return None;
        }
        let report = parse_report(payload)?;
        if report.state == State::Clear {
            self.records.retain(|entry| {
                !report.id.is_empty()
                    && entry.id != report.id
                    && !entry.id.starts_with(&format!("{}/", report.id))
            });
        } else {
            self.records.retain(|entry| entry.id != report.id);
            if self.records.len() == 256 {
                self.records.pop_front();
            }
            self.records.push_back(report);
        }
        Some(Event::Changed(self.current()))
    }

    fn current(&self) -> Option<Report> {
        let mut report = self
            .records
            .iter()
            .enumerate()
            .max_by_key(|(index, report)| {
                let priority = match report.state {
                    State::Blocked => 5,
                    State::Working => 4,
                    State::Error => 3,
                    State::Done => 2,
                    State::Idle => 1,
                    State::Clear => 0,
                };
                (priority, *index)
            })?
            .1
            .clone();
        if report.app.is_none() {
            let mut ancestor = report.id.as_str();
            while !ancestor.is_empty() {
                ancestor = ancestor.rsplit_once('/').map_or("", |(parent, _)| parent);
                if let Some(app) = self
                    .records
                    .iter()
                    .find(|entry| entry.id == ancestor)
                    .and_then(|entry| entry.app.clone())
                {
                    report.app = Some(app);
                    break;
                }
            }
        }
        Some(report)
    }
}

fn segment(value: &str) -> bool {
    (1..=32).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.+-".contains(&byte))
}

fn text(value: &str, encoded_limit: usize, decoded_limit: usize) -> Option<String> {
    if value.len() > encoded_limit {
        return None;
    }
    let decoded = STANDARD
        .decode(value)
        .or_else(|_| STANDARD_NO_PAD.decode(value))
        .ok()?;
    if decoded.len() > decoded_limit {
        return None;
    }
    let decoded = String::from_utf8(decoded).ok()?;
    (!decoded.chars().any(char::is_control)).then_some(decoded)
}

fn parse_report(payload: &[u8]) -> Option<Report> {
    let mut report = Report {
        state: State::Clear,
        id: String::new(),
        kind: None,
        progress: None,
        app: None,
        title: None,
        message: None,
    };
    let mut state = None;
    for pair in payload.split(|byte| *byte == b':') {
        let Some(equals) = pair.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        let key = pair[..equals].trim_ascii();
        let value = pair[equals + 1..].trim_ascii();
        if key.len() > 16
            || (key == b"msg" && value.len() > 2732)
            || (key == b"title" && value.len() > 256)
            || (key == b"app" && value.len() > 32)
            || (key == b"id" && value.len() > 128)
        {
            return None;
        }
        if key == b"id" && std::str::from_utf8(value).is_err() {
            return None;
        }
        let (Ok(key), Ok(value)) = (std::str::from_utf8(key), std::str::from_utf8(value)) else {
            continue;
        };
        if key == "id" && (value.split('/').count() > 8 || !value.split('/').all(segment)) {
            return None;
        }
        if key.is_empty()
            || !key.bytes().all(|byte| byte.is_ascii_lowercase())
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.,+/=-".contains(&byte))
        {
            continue;
        }
        match key {
            "state" => {
                state = match value {
                    "idle" => Some(State::Idle),
                    "working" => Some(State::Working),
                    "blocked" => Some(State::Blocked),
                    "done" => Some(State::Done),
                    "error" => Some(State::Error),
                    "clear" => Some(State::Clear),
                    _ => None,
                }
            }
            "id" => {
                report.id = value.into();
            }
            "kind" => {
                report.kind =
                    matches!(value, "permission" | "question" | "auth").then(|| value.to_owned())
            }
            "progress" => {
                report.progress = value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit())
                    .then(|| value.parse::<u8>().ok())
                    .flatten()
                    .filter(|value| *value <= 100)
            }
            "app" => report.app = segment(value).then(|| value.to_owned()),
            "msg" => report.message = Some(text(value, 2732, 2048)?),
            "title" => report.title = Some(text(value, 256, 192)?),
            _ => {}
        }
    }
    report.state = state?;
    if report.state != State::Blocked {
        report.kind = None;
    }
    if !matches!(report.state, State::Working | State::Blocked) {
        report.progress = None;
    }
    Some(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(parser: &mut Parser, payload: &str) -> Option<Report> {
        parser
            .scan(format!("\x1b]7501;{payload}\x07").as_bytes())
            .into_iter()
            .find_map(|(_, event)| match event {
                Event::Changed(report) => report,
                _ => None,
            })
    }

    #[test]
    fn invalidating_records_discards_partial_reports_but_keeps_split_probes() {
        let mut parser = Parser::default();
        report(&mut parser, "state=blocked:id=old").unwrap();
        parser.scan(b"\x1b]7501;state=blocked:id=partial");
        parser.invalidate_records();
        assert!(parser.scan(b"\x07").is_empty());
        assert!(parser.current().is_none());
        assert_eq!(
            report(&mut parser, "state=idle").unwrap().state,
            State::Idle
        );
        for prefix in [b"\x1b]7501;?".as_slice(), b"\x1b]7501;?\x1b"] {
            let mut parser = Parser::default();
            parser.scan(prefix);
            parser.invalidate_records();
            let suffix = if prefix.ends_with(b"\x1b") {
                b"\\".as_slice()
            } else {
                b"\x07"
            };
            assert_eq!(parser.scan(suffix), vec![(1, Event::Query)]);
        }
    }

    #[test]
    fn probes_and_reports_survive_every_boundary_and_terminator() {
        for terminator in ["\x07", "\x1b\\"] {
            for payload in ["?", "state=blocked:kind=question:app=pi:msg=SGVsbG8"] {
                let bytes = format!("\x1b]7501;{payload}{terminator}");
                for split in 0..=bytes.len() {
                    let mut parser = Parser::default();
                    let mut events = parser.scan(&bytes.as_bytes()[..split]);
                    events.extend(parser.scan(&bytes.as_bytes()[split..]));
                    assert_eq!(events.len(), 1, "{split}");
                    match &events[0].1 {
                        Event::Query => assert_eq!(payload, "?"),
                        Event::Changed(Some(report)) => {
                            assert_eq!(report.state, State::Blocked);
                            assert_eq!(report.message.as_deref(), Some("Hello"));
                            assert_eq!(report.kind.as_deref(), Some("question"));
                        }
                        _ => panic!("missing report"),
                    }
                }
            }
        }
    }

    #[test]
    fn grammar_skips_bad_pairs_and_unknown_keys_with_last_value_winning() {
        let report = parse_report(b"bad:=x:bad-key=x:nope=$:future=yes: state = working :state=blocked:kind=future:progress=101:app=bad/name:msg=SGk=").unwrap();
        assert_eq!(report.state, State::Blocked);
        assert_eq!(report.kind, None);
        assert_eq!(report.progress, None);
        assert_eq!(report.app, None);
        assert_eq!(report.message.as_deref(), Some("Hi"));
        assert!(parse_report(b"\t state \t= working \r\n:unknown=\xff").is_some());
        assert!(parse_report(b"state=working:id=\xff").is_none());
        for payload in [
            "",
            "app=test",
            "state=future",
            "state=working:state=future",
            "state=working:id=",
            "state=working:id=a!",
            "state=working:id=/a",
            "state=working:id=a//b",
        ] {
            assert!(parse_report(payload.as_bytes()).is_none(), "{payload}");
        }
    }

    #[test]
    fn decoded_text_and_every_limit_are_validated_before_record_changes() {
        for (key, limit) in [("msg", 2048), ("title", 192)] {
            let valid = format!("state=working:{key}={}", STANDARD.encode("a".repeat(limit)));
            assert!(parse_report(valid.as_bytes()).is_some());
            let invalid = format!(
                "state=idle:{key}={}",
                STANDARD.encode("a".repeat(limit + 1))
            );
            assert!(parse_report(invalid.as_bytes()).is_none());
            for text in ["\n", "\t", "\x1b", "\u{7f}", "\u{80}", "\u{9f}"] {
                let invalid = format!("state=idle:{key}={}", STANDARD.encode(text));
                assert!(parse_report(invalid.as_bytes()).is_none());
            }
            for text in ["a", "_", "/w=="] {
                let invalid = format!("state=idle:{key}={text}");
                assert!(parse_report(invalid.as_bytes()).is_none());
            }
        }
        for payload in [
            format!("state=working:{}=x", "a".repeat(17)),
            format!("state=working:app={}", "a".repeat(33)),
            format!("state=working:id={}", "a".repeat(33)),
            format!("state=working:id={}", ["a"; 9].join("/")),
            format!(
                "state=working:id={}",
                ["abcdefghijklmnopqrstuvwxyz"; 5].join("/")
            ),
            format!("state=working:id={}", vec!["a".repeat(32); 4].join("/")) + "aa",
            "state=working:msg=YQ==:msg=a".into(),
        ] {
            assert!(parse_report(payload.as_bytes()).is_none(), "{payload}");
        }
        let mut parser = Parser::default();
        report(&mut parser, "state=blocked").unwrap();
        for terminator in ["\x07", "\x1b\\"] {
            let prefix = "\x1b]7501;state=idle:unknown=";
            let bytes = format!(
                "{prefix}{}{terminator}",
                "a".repeat(4096 - prefix.len() - terminator.len())
            );
            assert_eq!(bytes.len(), 4096);
            assert_eq!(parser.scan(bytes.as_bytes()).len(), 1);
            let oversized = bytes.replacen("unknown=", "unknown=a", 1);
            assert!(parser.scan(oversized.as_bytes()).is_empty());
            assert!(parser.payload.len() <= 4096);
        }
    }

    #[test]
    fn controls_abort_and_nested_terminal_strings_cannot_forge_reports() {
        for bytes in [
            b"\x1b]7501;state=blo\tcked\x07".as_slice(),
            b"\x1b]7501;state=blocked\x18\x07",
            b"\x1b]7501;state=blocked\x1a\x1b\\",
            b"\x1b]7501;state=blocked\x1b\x1b\\",
            b"\x1bP\x1b]7501;state=blocked\x07\x1b\\",
            b"\x1b]0;\x1b]7501;state=blocked\x07",
        ] {
            let mut parser = Parser::default();
            let events = parser.scan(bytes);
            // An OSC aborted by a new OSC introducer starts a new real OSC.
            if bytes.starts_with(b"\x1b]0;") {
                assert_eq!(events.len(), 1);
            } else {
                assert!(events.is_empty(), "{bytes:?}");
            }
        }
    }

    #[test]
    fn transcript_zones_preserve_status_and_shell_return_clears_at_every_split() {
        for terminator in ["\x07", "\x1b\\"] {
            for state in ["working", "blocked"] {
                let stream = format!("\x1b]7501;state={state}:app=pi{terminator}\x1b]133;A{terminator}user\x1b]133;B{terminator}\x1b]133;C{terminator}\x1b]133;A{terminator}assistant\x1b]133;B{terminator}\x1b]133;C{terminator}");
                for split in 0..=stream.len() {
                    let mut parser = Parser::default();
                    parser.scan(&stream.as_bytes()[..split]);
                    parser.scan(&stream.as_bytes()[split..]);
                    assert_eq!(
                        parser.current().unwrap().state,
                        if state == "working" {
                            State::Working
                        } else {
                            State::Blocked
                        }
                    );
                }
                let boundary = format!("\x1b]133;D;0{terminator}\x1b]133;A{terminator}");
                for boundary_split in 0..=boundary.len() {
                    let mut parser = Parser::default();
                    parser.scan(stream.as_bytes());
                    parser.scan(&boundary.as_bytes()[..boundary_split]);
                    parser.scan(&boundary.as_bytes()[boundary_split..]);
                    assert!(parser.current().is_none());
                }
            }
        }
        let mut parser = Parser::default();
        report(&mut parser, "state=working");
        parser.scan(b"\x1b]133;D\x07\x1b]133;C\x07\x1b]133;A\x07");
        assert_eq!(parser.current().unwrap().state, State::Working);
    }

    #[test]
    fn shell_marks_survive_every_split_and_terminator_and_ignore_prompt_zones() {
        for terminator in ["\x07", "\x1b\\"] {
            use ShellMark::{CommandFinished, CommandStarted};
            for (stream, expected) in [
                (format!("out\x1b]133;C{terminator}"), vec![CommandStarted]),
                (
                    format!("\x1b]133;D;130{terminator}\x1b]133;A{terminator}% "),
                    vec![CommandFinished],
                ),
                (
                    format!("\x1b]133;D{terminator}\x1b]133;A{terminator}ls\x1b]133;C{terminator}"),
                    vec![CommandFinished, CommandStarted],
                ),
            ] {
                for split in 0..=stream.len() {
                    let mut marks = ShellMarks::default();
                    let mut found = marks.scan(&stream.as_bytes()[..split]);
                    found.extend(marks.scan(&stream.as_bytes()[split..]));
                    assert_eq!(found, expected, "{stream:?} at {split}");
                }
            }
            let mut marks = ShellMarks::default();
            let zones = format!(
                "\x1b]133;A{terminator}user\x1b]133;B{terminator}\x1b]133;A;k=s{terminator}\x1b]133;Cx{terminator}\x1b]1337;D{terminator}\x1b]7501;state=working{terminator}"
            );
            assert!(marks.scan(zones.as_bytes()).is_empty());
        }
    }

    #[test]
    fn records_replace_inherit_clear_and_follow_lifecycle_without_heartbeats() {
        let mut parser = Parser::default();
        report(&mut parser, "state=idle:app=build").unwrap();
        let child = report(&mut parser, "state=blocked:id=a/b:msg=SGk=").unwrap();
        assert_eq!(child.app.as_deref(), Some("build"));
        report(&mut parser, "state=working:id=a/b").unwrap();
        assert_eq!(parser.current().unwrap().message, None);
        parser.scan(b"\x1b[?1049h\x1b[!p");
        assert_eq!(parser.current().unwrap().state, State::Working);
        report(&mut parser, "state=done:id=finished").unwrap();
        parser.scan(b"\x1b]133;D;0\x07\x1b]133;A\x07");
        assert_eq!(parser.current().unwrap().state, State::Done);
        report(&mut parser, "state=blocked:id=a/b").unwrap();
        report(&mut parser, "state=clear:id=a").unwrap();
        assert_eq!(parser.current().unwrap().state, State::Done);
        parser.scan(b"\x1bc");
        assert!(parser.current().is_none());
        for index in 0..257 {
            report(&mut parser, &format!("state=idle:id=r{index}"));
        }
        assert_eq!(parser.records.len(), 256);
        assert_eq!(parser.records.front().unwrap().id, "r1");
        assert!(parser
            .scan(b"\x1b]7501;state=clear\x07")
            .iter()
            .any(|(_, event)| *event == Event::Changed(None)));
    }
}
