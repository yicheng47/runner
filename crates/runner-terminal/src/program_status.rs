//! OSC 7501 Program Status record store, FinalTerm shell-mark scanner, and
//! non-7501 byte activity filter.
//!
//! Live terminal models receive parsed `OSC 7501` and `OSC 133` events directly
//! from `libghostty-vt`'s native callbacks (`RecordStore`). `ActivityFilter`
//! mirrors Ghostty's VT100 + `OSC 7501` parser byte-for-byte so byte-activity
//! heuristics exclude the exact sequences Ghostty recognizes.

use std::collections::VecDeque;

use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use base64::Engine as _;

use crate::vt::{
    ProgramStatusKind, ProgramStatusState, RawProgramStatus, SemanticPromptEvent,
    SemanticPromptKind,
};

pub const PROBE_REPLY_BODY: &str =
    "?:states=idle,working,done,blocked,error:kinds=permission,question,auth";
pub const PROBE_REPLY: &str =
    "\x1b]7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth\x1b\\";

/// Ghostty's `osc.program_status.max_body_bytes` (`4096 - "\x1b]7501;".len - "\x1b\\".len`).
const MAX_7501_BODY_BYTES: usize = 4087;
const MAX_KEY_BYTES: usize = 16;
const MAX_APP_BYTES: usize = 32;
const MAX_ID_BYTES: usize = 128;
const MAX_ID_SEGMENT_BYTES: usize = 32;
const MAX_ID_DEPTH: usize = 8;
const MAX_TITLE_ENCODED_BYTES: usize = 256;
const MAX_TITLE_BYTES: usize = 192;
const MAX_MSG_ENCODED_BYTES: usize = 2732;
const MAX_MSG_BYTES: usize = 2048;

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

impl Report {
    pub(crate) fn from_vt(raw: RawProgramStatus) -> Self {
        let state = match raw.state {
            ProgramStatusState::Idle => State::Idle,
            ProgramStatusState::Working => State::Working,
            ProgramStatusState::Done => State::Done,
            ProgramStatusState::Blocked => State::Blocked,
            ProgramStatusState::Error => State::Error,
            ProgramStatusState::Clear => State::Clear,
        };
        let kind = raw.kind.map(|k| match k {
            ProgramStatusKind::Permission => "permission".to_owned(),
            ProgramStatusKind::Question => "question".to_owned(),
            ProgramStatusKind::Auth => "auth".to_owned(),
        });
        Self {
            state,
            id: raw.id,
            kind,
            progress: raw.progress,
            app: raw.app,
            title: raw.title,
            message: raw.message,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChildCount {
    pub app: Option<String>,
    pub active: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub current: Option<Report>,
    pub trigger: Option<Report>,
    pub root: Option<Report>,
    pub active_children: Vec<ChildCount>,
}

/// Stateful OSC 7501 record table driven by `libghostty-vt`'s native
/// `PROGRAM_STATUS`, `SEMANTIC_PROMPT`, and `RESET` callbacks.
#[derive(Default)]
pub(crate) struct RecordStore {
    records: VecDeque<Report>,
    shell_finished: bool,
    discard_inflight_report: bool,
}

impl RecordStore {
    pub(crate) fn clear(&mut self) {
        self.records.clear();
        self.shell_finished = false;
        self.discard_inflight_report = false;
    }

    pub(crate) fn invalidate_records(&mut self, inflight_7501_report: bool) {
        self.records.clear();
        self.shell_finished = false;
        self.discard_inflight_report = inflight_7501_report;
    }

    pub(crate) fn note_vt_ground(&mut self) {
        self.discard_inflight_report = false;
    }

    pub(crate) fn is_discarding_inflight(&self) -> bool {
        self.discard_inflight_report
    }

    pub(crate) fn apply_raw_report(&mut self, raw: RawProgramStatus) -> Option<Update> {
        if self.discard_inflight_report {
            self.discard_inflight_report = false;
            return None;
        }
        let report = Report::from_vt(raw);
        let mut trigger = report.clone();
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
            self.resolve_app(&mut trigger);
        }
        Some(self.update(Some(trigger)))
    }

    pub(crate) fn apply_semantic_prompt(&mut self, event: &SemanticPromptEvent) -> Option<Update> {
        match event.kind {
            SemanticPromptKind::CommandEnd => {
                self.shell_finished = true;
                None
            }
            SemanticPromptKind::OutputStart => {
                self.shell_finished = false;
                None
            }
            SemanticPromptKind::PromptStart => {
                // TUIs also emit 133;A/B/C for transcript zones; 133;D marks a shell return.
                if !std::mem::take(&mut self.shell_finished) {
                    return None;
                }
                let before = self.records.len();
                self.records
                    .retain(|report| matches!(report.state, State::Done | State::Error));
                (before != self.records.len()).then(|| self.update(None))
            }
            SemanticPromptKind::InputStart => None,
        }
    }

    pub(crate) fn apply_reset(&mut self) -> Option<Update> {
        self.shell_finished = false;
        self.discard_inflight_report = false;
        let changed = !self.records.is_empty();
        self.records.clear();
        changed.then(|| self.update(None))
    }

    fn update(&self, trigger: Option<Report>) -> Update {
        Update {
            current: self.current(),
            trigger,
            root: self.root(),
            active_children: self.active_children(),
        }
    }

    fn resolve_app_ref<'a>(&'a self, report: &'a Report) -> Option<&'a str> {
        if let Some(app) = report.app.as_deref() {
            return Some(app);
        }
        let mut ancestor = report.id.as_str();
        while !ancestor.is_empty() {
            ancestor = ancestor.rsplit_once('/').map_or("", |(parent, _)| parent);
            if let Some(app) = self
                .records
                .iter()
                .find(|entry| entry.id == ancestor)
                .and_then(|entry| entry.app.as_deref())
            {
                return Some(app);
            }
        }
        None
    }

    fn resolve_app(&self, report: &mut Report) {
        if report.app.is_none() {
            report.app = self.resolve_app_ref(report).map(str::to_owned);
        }
    }

    pub(crate) fn root(&self) -> Option<Report> {
        self.records
            .iter()
            .find(|entry| entry.id.is_empty())
            .cloned()
    }

    pub(crate) fn active_children(&self) -> Vec<ChildCount> {
        let mut counts: Vec<ChildCount> = Vec::new();
        let app_index: Option<std::collections::HashMap<&str, &str>> = if self.records.len() > 8 {
            Some(
                self.records
                    .iter()
                    .filter_map(|entry| Some((entry.id.as_str(), entry.app.as_deref()?)))
                    .collect(),
            )
        } else {
            None
        };
        for entry in &self.records {
            if entry.id.is_empty() || !matches!(entry.state, State::Working | State::Blocked) {
                continue;
            }
            let app = if let Some(index) = app_index.as_ref() {
                if let Some(explicit) = entry.app.as_deref() {
                    Some(explicit)
                } else {
                    let mut ancestor = entry.id.as_str();
                    let mut found = None;
                    while !ancestor.is_empty() {
                        ancestor = ancestor.rsplit_once('/').map_or("", |(parent, _)| parent);
                        if let Some(&inherited) = index.get(ancestor) {
                            found = Some(inherited);
                            break;
                        }
                    }
                    found
                }
            } else {
                self.resolve_app_ref(entry)
            };
            if let Some(existing) = counts.iter_mut().find(|item| item.app.as_deref() == app) {
                existing.active += 1;
            } else {
                counts.push(ChildCount {
                    app: app.map(str::to_owned),
                    active: 1,
                });
            }
        }
        counts
    }

    pub(crate) fn current(&self) -> Option<Report> {
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
        self.resolve_app(&mut report);
        Some(report)
    }
}

/// A shell's FinalTerm command marks: `133;C` as a command starts and
/// `133;D` once the shell is back. `133;A` is not one, because agent TUIs
/// also emit A, B and C for their own transcript zones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellMark {
    CommandStarted,
    CommandFinished,
}

#[derive(Clone, Copy, Default)]
enum ShellScanMode {
    #[default]
    Ground,
    Escape,
    Osc,
    OscEscape,
}

/// Reads [`ShellMark`]s from a shell's PTY output across reads.
#[derive(Default)]
pub struct ShellMarks {
    mode: ShellScanMode,
    payload: Vec<u8>,
    overflowed: bool,
}

impl ShellMarks {
    /// The command marks in `bytes`, in order.
    pub fn scan(&mut self, bytes: &[u8]) -> Vec<ShellMark> {
        const MAX_OSC133_BYTES: usize = 512;
        let mut out = Vec::new();
        for &byte in bytes {
            self.mode = match self.mode {
                ShellScanMode::Ground if byte == 0x1b => ShellScanMode::Escape,
                ShellScanMode::Ground => ShellScanMode::Ground,
                ShellScanMode::Escape => match byte {
                    b']' => {
                        self.payload.clear();
                        self.overflowed = false;
                        ShellScanMode::Osc
                    }
                    0x1b => ShellScanMode::Escape,
                    _ => ShellScanMode::Ground,
                },
                ShellScanMode::Osc => match byte {
                    0x07 => {
                        if let Some(mark) = self.finish() {
                            out.push(mark);
                        }
                        ShellScanMode::Ground
                    }
                    0x18 | 0x1a => ShellScanMode::Ground,
                    0x1b => ShellScanMode::OscEscape,
                    0x00..=0x06 | 0x08..=0x17 | 0x19 | 0x1c..=0x1f => ShellScanMode::Osc,
                    _ => {
                        if self.payload.len() < MAX_OSC133_BYTES {
                            self.payload.push(byte);
                        } else {
                            self.overflowed = true;
                        }
                        ShellScanMode::Osc
                    }
                },
                ShellScanMode::OscEscape => match byte {
                    b'\\' => {
                        if let Some(mark) = self.finish() {
                            out.push(mark);
                        }
                        ShellScanMode::Ground
                    }
                    b']' => {
                        if let Some(mark) = self.finish() {
                            out.push(mark);
                        }
                        self.payload.clear();
                        self.overflowed = false;
                        ShellScanMode::Osc
                    }
                    0x1b => {
                        if let Some(mark) = self.finish() {
                            out.push(mark);
                        }
                        ShellScanMode::Escape
                    }
                    _ => {
                        if let Some(mark) = self.finish() {
                            out.push(mark);
                        }
                        ShellScanMode::Ground
                    }
                },
            };
        }
        out
    }

    fn finish(&self) -> Option<ShellMark> {
        if self.overflowed {
            return None;
        }
        if self.payload == b"133;D" || self.payload.starts_with(b"133;D;") {
            Some(ShellMark::CommandFinished)
        } else if self.payload == b"133;C" || self.payload.starts_with(b"133;C;") {
            Some(ShellMark::CommandStarted)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Default)]
enum FilterMode {
    #[default]
    Ground,
    Escape,
    /// `0x1b` immediately terminated a valid OSC 7501; if `\` follows, the
    /// pair was an ST terminator and contributes 0 activity bytes.
    AfterValid7501Esc,
    Osc,
}

/// Counts non-OSC-7501 bytes in PTY output across reads, withholding candidate
/// OSC 7501 controls across chunk boundaries until recognized or rejected.
///
/// Follows Ghostty's VT100 + `osc.program_status` parser rules: ignored C0
/// controls inside an OSC string do not alter the payload, `0x1b` terminates
/// the OSC immediately (with an optional following `\` consumed as ST), and
/// `CAN`/`SUB` cancel the sequence.
#[derive(Default)]
pub struct ActivityFilter {
    mode: FilterMode,
    payload: Vec<u8>,
    payload_overflow: bool,
    pending_bytes: usize,
    activity_bytes: usize,
}

impl ActivityFilter {
    pub fn observe(&mut self, bytes: &[u8]) -> usize {
        self.activity_bytes = 0;
        for &byte in bytes {
            self.step(byte);
        }
        self.activity_bytes
    }

    pub fn activity_bytes(&self) -> usize {
        self.activity_bytes
    }

    fn release_pending(&mut self) {
        self.activity_bytes += self.pending_bytes;
        self.pending_bytes = 0;
    }

    fn is_7501_candidate(&self) -> bool {
        if self.payload_overflow {
            return false;
        }
        const PREFIX: &[u8] = b"7501;";
        if self.payload.len() <= PREFIX.len() {
            PREFIX.starts_with(&self.payload)
        } else {
            self.payload.starts_with(PREFIX)
                && self.payload.len() - PREFIX.len() <= MAX_7501_BODY_BYTES
        }
    }

    fn is_valid_7501(&self) -> bool {
        if self.payload_overflow {
            return false;
        }
        let Some(body) = self.payload.strip_prefix(b"7501;") else {
            return false;
        };
        body == b"?" || is_valid_ghostty_7501_body(body)
    }

    fn step(&mut self, byte: u8) {
        self.mode = match self.mode {
            FilterMode::Ground => {
                if byte == 0x1b {
                    self.pending_bytes = 1;
                    FilterMode::Escape
                } else {
                    self.activity_bytes += 1;
                    FilterMode::Ground
                }
            }
            FilterMode::Escape => match byte {
                b']' => {
                    self.pending_bytes += 1;
                    self.payload.clear();
                    self.payload_overflow = false;
                    FilterMode::Osc
                }
                0x1b => {
                    self.release_pending();
                    self.pending_bytes = 1;
                    FilterMode::Escape
                }
                _ => {
                    self.release_pending();
                    self.activity_bytes += 1;
                    FilterMode::Ground
                }
            },
            FilterMode::AfterValid7501Esc => match byte {
                b'\\' => {
                    // `ESC \` completed ST for the recognized OSC 7501.
                    self.pending_bytes = 0;
                    FilterMode::Ground
                }
                b']' => {
                    // Stray ESC ended the first 7501 and opened a new OSC.
                    self.pending_bytes = 2;
                    self.payload.clear();
                    self.payload_overflow = false;
                    FilterMode::Osc
                }
                0x1b => {
                    // The first ESC was a stray ESC; hold the new ESC.
                    self.release_pending();
                    self.pending_bytes = 1;
                    FilterMode::Escape
                }
                _ => {
                    self.release_pending();
                    self.activity_bytes += 1;
                    FilterMode::Ground
                }
            },
            FilterMode::Osc => match byte {
                0x07 => {
                    if self.pending_bytes > 0 {
                        self.pending_bytes += 1;
                    } else {
                        self.activity_bytes += 1;
                    }
                    if self.is_valid_7501() {
                        self.pending_bytes = 0;
                    } else {
                        self.release_pending();
                    }
                    FilterMode::Ground
                }
                0x18 | 0x1a => {
                    self.release_pending();
                    self.activity_bytes += 1;
                    FilterMode::Ground
                }
                0x1b => {
                    if self.is_valid_7501() {
                        self.pending_bytes = 1;
                        FilterMode::AfterValid7501Esc
                    } else {
                        self.release_pending();
                        self.pending_bytes = 1;
                        FilterMode::Escape
                    }
                }
                0x00..=0x06 | 0x08..=0x17 | 0x19 | 0x1c..=0x1f => {
                    // VT100 parser ignores these C0 controls inside `osc_string`.
                    if self.pending_bytes > 0 {
                        self.pending_bytes += 1;
                    } else {
                        self.activity_bytes += 1;
                    }
                    FilterMode::Osc
                }
                _ => {
                    if self.payload.len() < 5 + MAX_7501_BODY_BYTES {
                        self.payload.push(byte);
                    } else {
                        self.payload_overflow = true;
                    }
                    if self.pending_bytes > 0 {
                        self.pending_bytes += 1;
                        if !self.is_7501_candidate() {
                            self.release_pending();
                        }
                    } else {
                        self.activity_bytes += 1;
                    }
                    FilterMode::Osc
                }
            },
        };
    }
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'+' | b'-')
}

fn is_value_byte(b: u8) -> bool {
    is_name_byte(b) || matches!(b, b',' | b'/' | b'=')
}

fn is_name(s: &[u8]) -> bool {
    !s.is_empty() && s.iter().copied().all(is_name_byte)
}

fn trim_zig_ascii_whitespace(mut s: &[u8]) -> &[u8] {
    let is_ws = |b: u8| matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r');
    while let Some((&first, rest)) = s.split_first() {
        if is_ws(first) {
            s = rest;
        } else {
            break;
        }
    }
    while let Some((&last, rest)) = s.split_last() {
        if is_ws(last) {
            s = rest;
        } else {
            break;
        }
    }
    s
}

fn validate_ghostty_id(value: &[u8]) -> bool {
    if value.len() > MAX_ID_BYTES {
        return false;
    }
    let mut depth = 0usize;
    for seg in value.split(|&b| b == b'/') {
        depth += 1;
        if depth > MAX_ID_DEPTH || seg.len() > MAX_ID_SEGMENT_BYTES || !is_name(seg) {
            return false;
        }
    }
    true
}

fn last_value<'a>(key: &[u8], data: &'a [u8]) -> Option<&'a [u8]> {
    let mut last = None;
    for pair in data.split(|&b| b == b':') {
        let Some(eq) = pair.iter().position(|&b| b == b'=') else {
            continue;
        };
        let field_key = trim_zig_ascii_whitespace(&pair[..eq]);
        if field_key != key {
            continue;
        }
        let value = trim_zig_ascii_whitespace(&pair[eq + 1..]);
        if !value.iter().copied().all(is_value_byte) {
            continue;
        }
        last = Some(value);
    }
    last
}

fn decode_safe_text(encoded: &[u8], max_decoded: usize) -> bool {
    let decoded = if encoded.ends_with(b"=") {
        STANDARD.decode(encoded)
    } else {
        STANDARD_NO_PAD.decode(encoded)
    };
    let Ok(bytes) = decoded else {
        return false;
    };
    if bytes.len() > max_decoded {
        return false;
    }
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return false;
    };
    !text.chars().any(|ch| {
        let cp = ch as u32;
        cp <= 0x1f || cp == 0x7f || (0x80..=0x9f).contains(&cp)
    })
}

/// Exact mirror of `vendor/libghostty-vt/src/terminal/osc/parsers/program_status.zig:validate`.
fn is_valid_ghostty_7501_body(data: &[u8]) -> bool {
    if data.len() > MAX_7501_BODY_BYTES {
        return false;
    }
    for pair in data.split(|&b| b == b':') {
        let Some(eq) = pair.iter().position(|&b| b == b'=') else {
            continue;
        };
        let key = trim_zig_ascii_whitespace(&pair[..eq]);
        if key.len() > MAX_KEY_BYTES {
            return false;
        }
        let raw_value = trim_zig_ascii_whitespace(&pair[eq + 1..]);
        match key {
            b"id" => {
                if !validate_ghostty_id(raw_value) {
                    return false;
                }
            }
            b"app" => {
                if raw_value.len() > MAX_APP_BYTES {
                    return false;
                }
            }
            b"title" if raw_value.len() > MAX_TITLE_ENCODED_BYTES => {
                return false;
            }
            b"msg" if raw_value.len() > MAX_MSG_ENCODED_BYTES => {
                return false;
            }
            _ => {}
        }
    }
    if let Some(title) = last_value(b"title", data) {
        if !decode_safe_text(title, MAX_TITLE_BYTES) {
            return false;
        }
    }
    if let Some(msg) = last_value(b"msg", data) {
        if !decode_safe_text(msg, MAX_MSG_BYTES) {
            return false;
        }
    }
    matches!(
        last_value(b"state", data),
        Some(b"idle" | b"working" | b"done" | b"blocked" | b"error" | b"clear")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vt::{Terminal, TerminalConfig, TerminalEffect};

    #[derive(Debug, PartialEq, Eq)]
    enum TestEvent {
        Query(Vec<u8>),
        Changed(Box<Update>),
    }

    struct Harness {
        term: Terminal,
        store: RecordStore,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                term: Terminal::new(&TerminalConfig::default()).expect("vt terminal"),
                store: RecordStore::default(),
            }
        }

        fn scan(&mut self, bytes: &[u8]) -> Vec<TestEvent> {
            let mut out = Vec::new();
            let mut drain = |term: &mut Terminal, store: &mut RecordStore| {
                for effect in term.take_effects() {
                    match effect {
                        TerminalEffect::WritePty(reply) if reply.starts_with(b"\x1b]7501;") => {
                            out.push(TestEvent::Query(reply));
                        }
                        TerminalEffect::ProgramStatus(raw) => {
                            if let Some(update) = store.apply_raw_report(raw) {
                                out.push(TestEvent::Changed(Box::new(update)));
                            }
                        }
                        TerminalEffect::SemanticPrompt(ev) => {
                            if let Some(update) = store.apply_semantic_prompt(&ev) {
                                out.push(TestEvent::Changed(Box::new(update)));
                            }
                        }
                        TerminalEffect::Reset => {
                            if let Some(update) = store.apply_reset() {
                                out.push(TestEvent::Changed(Box::new(update)));
                            }
                        }
                        _ => {}
                    }
                }
            };
            if self.store.is_discarding_inflight() {
                let (consumed, reached) = self
                    .term
                    .vt_write_until_osc_or_ground(bytes)
                    .unwrap_or((bytes.len(), false));
                drain(&mut self.term, &mut self.store);
                if reached || self.term.vt_ground() {
                    self.store.note_vt_ground();
                }
                if consumed < bytes.len() {
                    self.term.vt_write(&bytes[consumed..]);
                    drain(&mut self.term, &mut self.store);
                    if self.term.vt_ground() {
                        self.store.note_vt_ground();
                    }
                }
            } else {
                self.term.vt_write(bytes);
                drain(&mut self.term, &mut self.store);
                if self.term.vt_ground() {
                    self.store.note_vt_ground();
                }
            }
            out
        }

        fn invalidate_records(&mut self) {
            let cont = self.term.continuation().unwrap_or_default();
            let inflight = cont.starts_with(b"\x1b]7501;")
                && cont != b"\x1b]7501;?"
                && cont != b"\x1b]7501;?\x1b";
            self.store.invalidate_records(inflight);
        }

        fn current(&self) -> Option<Report> {
            self.store.current()
        }
    }

    fn report(harness: &mut Harness, payload: &str) -> Option<Report> {
        harness
            .scan(format!("\x1b]7501;{payload}\x07").as_bytes())
            .into_iter()
            .find_map(|event| match event {
                TestEvent::Changed(update) => update.current,
                _ => None,
            })
    }

    fn parse_via_vt(payload: &[u8]) -> Option<Report> {
        let mut h = Harness::new();
        let mut bytes = b"\x1b]7501;".to_vec();
        bytes.extend_from_slice(payload);
        bytes.push(0x07);
        h.scan(&bytes).into_iter().find_map(|ev| match ev {
            TestEvent::Changed(update) => update.trigger,
            _ => None,
        })
    }

    #[test]
    fn invalidating_records_discards_partial_reports_but_keeps_split_probes() {
        let mut h = Harness::new();
        report(&mut h, "state=blocked:id=old").unwrap();
        h.scan(b"\x1b]7501;state=blocked:id=partial");
        h.invalidate_records();
        assert!(h.scan(b"\x07").is_empty());
        assert!(h.current().is_none());
        assert_eq!(report(&mut h, "state=idle").unwrap().state, State::Idle);
        for prefix in [b"\x1b]7501;?".as_slice(), b"\x1b]7501;?\x1b"] {
            let mut h = Harness::new();
            let mut events = h.scan(prefix);
            h.invalidate_records();
            let (suffix, term) = if prefix.ends_with(b"\x1b") {
                (b"\\".as_slice(), "\x1b\\")
            } else {
                (b"\x07".as_slice(), "\x07")
            };
            events.extend(h.scan(suffix));
            let expected = format!("\x1b]7501;{PROBE_REPLY_BODY}{term}").into_bytes();
            assert_eq!(events, vec![TestEvent::Query(expected)]);
        }
    }

    #[test]
    fn probes_and_reports_survive_every_boundary_and_terminator() {
        for terminator in ["\x07", "\x1b\\"] {
            for payload in ["?", "state=blocked:kind=question:app=pi:msg=SGVsbG8"] {
                let bytes = format!("\x1b]7501;{payload}{terminator}");
                for split in 0..=bytes.len() {
                    let mut h = Harness::new();
                    let mut events = h.scan(&bytes.as_bytes()[..split]);
                    events.extend(h.scan(&bytes.as_bytes()[split..]));
                    assert_eq!(events.len(), 1, "{split}");
                    match &events[0] {
                        TestEvent::Query(reply) => {
                            assert_eq!(payload, "?");
                            let expected =
                                format!("\x1b]7501;{PROBE_REPLY_BODY}{terminator}").into_bytes();
                            assert_eq!(reply, &expected);
                        }
                        TestEvent::Changed(update) => {
                            let report = update.current.as_ref().expect("current report");
                            let trigger = update.trigger.as_ref().expect("trigger report");
                            assert_eq!(report.state, State::Blocked);
                            assert_eq!(report.message.as_deref(), Some("Hello"));
                            assert_eq!(report.kind.as_deref(), Some("question"));
                            assert_eq!(trigger, report);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn grammar_skips_bad_pairs_and_unknown_keys_with_last_value_winning() {
        let report = parse_via_vt(b"bad:=x:bad-key=x:nope=$:future=yes: state = working :state=blocked:kind=future:progress=101:app=bad/name:msg=SGk=").unwrap();
        assert_eq!(report.state, State::Blocked);
        assert_eq!(report.kind, None);
        assert_eq!(report.progress, None);
        assert_eq!(report.app, None);
        assert_eq!(report.message.as_deref(), Some("Hi"));
        assert!(parse_via_vt(b"\t state \t= working \r\n:unknown=\xff").is_some());
        assert!(parse_via_vt(b"state=working:id=\xff").is_none());
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
            assert!(parse_via_vt(payload.as_bytes()).is_none(), "{payload}");
        }
    }

    #[test]
    fn decoded_text_and_every_limit_are_validated_before_record_changes() {
        for (key, limit) in [("msg", 2048), ("title", 192)] {
            let valid = format!("state=working:{key}={}", STANDARD.encode("a".repeat(limit)));
            assert!(parse_via_vt(valid.as_bytes()).is_some());
            let invalid = format!(
                "state=idle:{key}={}",
                STANDARD.encode("a".repeat(limit + 1))
            );
            assert!(parse_via_vt(invalid.as_bytes()).is_none());
            for text in ["\n", "\t", "\x1b", "\u{7f}", "\u{80}", "\u{9f}"] {
                let invalid = format!("state=idle:{key}={}", STANDARD.encode(text));
                assert!(parse_via_vt(invalid.as_bytes()).is_none());
            }
            for text in ["a", "_", "/w=="] {
                let invalid = format!("state=idle:{key}={text}");
                assert!(parse_via_vt(invalid.as_bytes()).is_none());
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
            assert!(parse_via_vt(payload.as_bytes()).is_none(), "{payload}");
        }
        // Ghostty caps the OSC 7501 body at 4087 bytes (4096 total with ST, 4095 with BEL).
        let mut h = Harness::new();
        report(&mut h, "state=blocked").unwrap();
        for terminator in ["\x07", "\x1b\\"] {
            let prefix = "\x1b]7501;state=idle:unknown=";
            let fill_len = MAX_7501_BODY_BYTES - "state=idle:unknown=".len();
            let bytes = format!("{prefix}{}{terminator}", "a".repeat(fill_len));
            assert_eq!(h.scan(bytes.as_bytes()).len(), 1);
            let oversized = bytes.replacen("unknown=", "unknown=a", 1);
            assert!(h.scan(oversized.as_bytes()).is_empty());
        }
    }

    #[test]
    fn controls_follow_ghostty_vt100_osc_rules() {
        // CAN and SUB cancel the OSC; VT100 ignores embedded C0 controls (\t),
        // ends an OSC on a stray ESC, and exits DCS on ESC to start a new OSC.
        for (bytes, expected_events) in [
            (b"\x1b]7501;state=blo\tcked\x07".as_slice(), 1usize),
            (b"\x1b]7501;state=blocked\x18\x07", 0),
            (b"\x1b]7501;state=blocked\x1a\x1b\\", 0),
            (b"\x1b]7501;state=blocked\x1b\x1b\\", 1),
            (b"\x1bP\x1b]7501;state=blocked\x07\x1b\\", 1),
            (b"\x1b]0;\x1b]7501;state=blocked\x07", 1),
        ] {
            let mut h = Harness::new();
            let events = h.scan(bytes);
            assert_eq!(events.len(), expected_events, "{bytes:?}");
        }
    }

    #[test]
    fn transcript_zones_preserve_status_and_shell_return_clears_at_every_split() {
        for terminator in ["\x07", "\x1b\\"] {
            for state in ["working", "blocked"] {
                let stream = format!("\x1b]7501;state={state}:app=pi{terminator}\x1b]133;A{terminator}user\x1b]133;B{terminator}\x1b]133;C{terminator}\x1b]133;A{terminator}assistant\x1b]133;B{terminator}\x1b]133;C{terminator}");
                for split in 0..=stream.len() {
                    let mut h = Harness::new();
                    h.scan(&stream.as_bytes()[..split]);
                    h.scan(&stream.as_bytes()[split..]);
                    assert_eq!(
                        h.current().unwrap().state,
                        if state == "working" {
                            State::Working
                        } else {
                            State::Blocked
                        }
                    );
                }
                let boundary = format!("\x1b]133;D;0{terminator}\x1b]133;A{terminator}");
                for boundary_split in 0..=boundary.len() {
                    let mut h = Harness::new();
                    h.scan(stream.as_bytes());
                    h.scan(&boundary.as_bytes()[..boundary_split]);
                    h.scan(&boundary.as_bytes()[boundary_split..]);
                    assert!(h.current().is_none());
                }
            }
        }
        let mut h = Harness::new();
        report(&mut h, "state=working");
        h.scan(b"\x1b]133;D\x07\x1b]133;C\x07\x1b]133;A\x07");
        assert_eq!(h.current().unwrap().state, State::Working);
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
        let mut h = Harness::new();
        report(&mut h, "state=idle:app=build").unwrap();
        let child = report(&mut h, "state=blocked:id=a/b:msg=SGk=").unwrap();
        assert_eq!(child.app.as_deref(), Some("build"));
        report(&mut h, "state=working:id=a/b").unwrap();
        assert_eq!(h.current().unwrap().message, None);
        h.scan(b"\x1b[?1049h\x1b[!p");
        assert_eq!(h.current().unwrap().state, State::Working);
        report(&mut h, "state=done:id=finished").unwrap();
        h.scan(b"\x1b]133;D;0\x07\x1b]133;A\x07");
        assert_eq!(h.current().unwrap().state, State::Done);
        report(&mut h, "state=blocked:id=a/b").unwrap();
        report(&mut h, "state=clear:id=a").unwrap();
        assert_eq!(h.current().unwrap().state, State::Done);
        h.scan(b"\x1bc");
        assert!(h.current().is_none());
        for index in 0..257 {
            report(&mut h, &format!("state=idle:id=r{index}"));
        }
        assert_eq!(h.store.records.len(), 256);
        assert_eq!(h.store.records.front().unwrap().id, "r1");
        assert!(h
            .scan(b"\x1b]7501;state=clear\x07")
            .iter()
            .any(|event| matches!(
                event,
                TestEvent::Changed(update)
                    if update.current.is_none()
                        && update.trigger.as_ref().is_some_and(|trigger| {
                            trigger.state == State::Clear && trigger.id.is_empty()
                        })
            )));
    }

    #[test]
    fn changed_update_preserves_triggering_record_even_when_foreign_child_wins_aggregate() {
        let mut h = Harness::new();
        h.scan(b"\x1b]7501;state=blocked:app=codex:kind=permission\x1b\\");
        h.scan(b"\x1b]7501;state=blocked:id=sub:app=other:kind=question\x1b\\");
        let events = h.scan(b"\x1b]7501;state=done:app=codex\x1b\\");
        assert_eq!(events.len(), 1);
        match &events[0] {
            TestEvent::Changed(update) => {
                let current = update.current.as_ref().expect("aggregate current");
                let trigger = update.trigger.as_ref().expect("triggering root done");
                assert_eq!(current.id, "sub");
                assert_eq!(current.state, State::Blocked);
                assert_eq!(current.app.as_deref(), Some("other"));
                assert!(trigger.id.is_empty());
                assert_eq!(trigger.state, State::Done);
                assert_eq!(trigger.app.as_deref(), Some("codex"));
            }
            other => panic!("expected aggregate current + triggering root done, got {other:?}"),
        }
    }

    #[test]
    fn codex_0163_and_0164_plus_reports_survive_every_split_and_terminator() {
        for terminator in ["\x07", "\x1b\\"] {
            for (payload, expected_state, expected_kind, expected_app) in [
                (
                    "state=blocked:app=codex",
                    State::Blocked,
                    None,
                    Some("codex"),
                ),
                ("state=blocked", State::Blocked, None, None),
                (
                    "state=blocked:app=codex:kind=permission",
                    State::Blocked,
                    Some("permission"),
                    Some("codex"),
                ),
                (
                    "state=blocked:app=codex:kind=question",
                    State::Blocked,
                    Some("question"),
                    Some("codex"),
                ),
                (
                    "state=blocked:app=codex:kind=auth",
                    State::Blocked,
                    Some("auth"),
                    Some("codex"),
                ),
                (
                    "state=working:app=codex",
                    State::Working,
                    None,
                    Some("codex"),
                ),
                ("state=idle:app=codex", State::Idle, None, Some("codex")),
                ("state=done:app=codex", State::Done, None, Some("codex")),
                ("state=error:app=codex", State::Error, None, Some("codex")),
            ] {
                let bytes = format!("\x1b]7501;{payload}{terminator}");
                for split in 0..=bytes.len() {
                    let mut h = Harness::new();
                    let mut events = h.scan(&bytes.as_bytes()[..split]);
                    events.extend(h.scan(&bytes.as_bytes()[split..]));
                    assert_eq!(events.len(), 1, "{payload} split={split}");
                    match &events[0] {
                        TestEvent::Changed(update) => {
                            let report = update.current.as_ref().expect("current report");
                            let trigger = update.trigger.as_ref().expect("trigger report");
                            assert_eq!(report.state, expected_state, "{payload}");
                            assert_eq!(report.kind.as_deref(), expected_kind, "{payload}");
                            assert_eq!(report.app.as_deref(), expected_app, "{payload}");
                            assert!(report.id.is_empty(), "{payload}");
                            assert_eq!(trigger, report, "{payload}");
                        }
                        other => panic!("expected report for {payload}, got {other:?}"),
                    }
                    let clear = format!("\x1b]7501;state=clear{terminator}");
                    let cleared = h.scan(clear.as_bytes());
                    assert_eq!(cleared.len(), 1);
                    assert!(matches!(
                        &cleared[0],
                        TestEvent::Changed(update)
                            if update.current.is_none()
                                && update.trigger.as_ref().is_some_and(|trigger| {
                                    trigger.state == State::Clear
                                        && trigger.id.is_empty()
                                        && trigger.app.is_none()
                                })
                    ));
                    assert!(h.current().is_none());
                }
            }
        }
    }

    #[test]
    fn activity_filter_excludes_recognized_7501_controls_and_preserves_other_bytes_at_every_split()
    {
        for payload in [
            "7501;?",
            "7501;state=blocked:app=codex",
            "7501;state=blocked:app=codex:kind=permission",
            "7501;state=blocked:app=codex:kind=question",
            "7501;state=blocked:app=codex:kind=auth",
            "7501;state=working:app=codex",
            "7501;state=idle:app=codex",
            "7501;state=done:app=codex",
            "7501;state=error:app=codex",
            "7501;state=clear",
        ] {
            for terminator in ["\x07", "\x1b\\"] {
                let control = format!("\x1b]{payload}{terminator}");
                for split in 0..=control.len() {
                    let mut filter = ActivityFilter::default();
                    assert_eq!(
                        filter.observe(&control.as_bytes()[..split]),
                        0,
                        "{payload} prefix split={split}"
                    );
                    assert_eq!(
                        filter.observe(&control.as_bytes()[split..]),
                        0,
                        "{payload} suffix split={split}"
                    );
                    let mixed = format!("before{control}after");
                    let mut count = 0;
                    for byte in mixed.as_bytes() {
                        count += filter.observe(&[*byte]);
                    }
                    assert_eq!(count, b"beforeafter".len(), "{payload}");
                }
            }
        }

        // Exact Ghostty 4087-byte body is excluded; 4088-byte oversized body is released.
        for terminator in ["\x07", "\x1b\\"] {
            let prefix = "\x1b]7501;state=idle:unknown=";
            let fill = MAX_7501_BODY_BYTES - "state=idle:unknown=".len();
            let valid = format!("{prefix}{}{terminator}", "a".repeat(fill));
            let mut filter = ActivityFilter::default();
            let mut count = 0;
            for byte in valid.as_bytes() {
                count += filter.observe(&[*byte]);
            }
            assert_eq!(count, 0);

            let oversized = valid.replacen("unknown=", "unknown=a", 1);
            let mut filter = ActivityFilter::default();
            let mut count = 0;
            for byte in oversized.as_bytes() {
                count += filter.observe(&[*byte]);
            }
            assert_eq!(count, oversized.len());
        }

        // Non-7501 OSCs and rejected/cancelled 7501 reports keep full byte counts.
        for stream in [
            b"\x1b]2;custom title\x07".as_slice(),
            b"\x1b]0;project\x1b\\",
            b"\x1b]133;D;0\x1b\\\x1b]133;A\x1b\\",
            b"\x1b[?2004h",
            b"\x1b]7501;state=BLOCKED:app=codex\x07",
            b"\x1b]7501;state=working:id=bad!\x07",
            b"\x1b]7501;state=blocked:msg=a\x07",
            b"\x1b]7501;state=blocked:msg=/w==\x07",
            b"\x1b]7501;state=blocked:msg=Cg==\x07",
            b"\x1b]7501;state=blocked\x18\x07",
            b"\x1b]7501;state=blocked\x1a\x1b\\",
        ] {
            let mut filter = ActivityFilter::default();
            let mut count = 0;
            for byte in stream {
                count += filter.observe(&[*byte]);
            }
            assert_eq!(count, stream.len(), "{stream:?}");
        }

        // Embedded tab, stray ESC, and DCS-wrapped OSC 7501 match Ghostty's VT100 parser:
        // the recognized 7501 bytes are excluded while surrounding bytes stay counted.
        for (stream, expected_activity) in [
            (b"\x1b]7501;state=blo\tcked\x07".as_slice(), 0usize),
            (b"\x1b]7501;state=blocked\x1b\x1b\\", 3),
            (b"\x1bP\x1b]7501;state=blocked:app=codex\x07\x1b\\", 4),
        ] {
            let mut filter = ActivityFilter::default();
            let mut count = 0;
            for byte in stream {
                count += filter.observe(&[*byte]);
            }
            assert_eq!(count, expected_activity, "{stream:?}");
        }

        // An aborted non-7501 OSC followed by a valid OSC 7501 counts only the aborted bytes.
        let combined = [
            b"\x1b]0;title".as_slice(),
            b"\x1b]7501;state=idle:app=codex\x07".as_slice(),
        ]
        .concat();
        let mut filter = ActivityFilter::default();
        let mut count = 0;
        for byte in &combined {
            count += filter.observe(&[*byte]);
        }
        assert_eq!(count, b"\x1b]0;title".len());

        // In Ghostty's VT100 parser, an OSC 7501 ended directly by `ESC ]` of a second
        // OSC 7501 dispatches BOTH valid reports, so both are excluded.
        let back_to_back = [
            b"\x1b]7501;state=blocked".as_slice(),
            b"\x1b]7501;state=clear\x1b\\".as_slice(),
        ]
        .concat();
        let mut filter = ActivityFilter::default();
        let mut count = 0;
        for byte in &back_to_back {
            count += filter.observe(&[*byte]);
        }
        assert_eq!(count, 0);
    }

    #[test]
    fn activity_filter_matches_ghostty_vt_parser_on_identical_streams() {
        let max_fill = "a".repeat(MAX_7501_BODY_BYTES - "state=idle:unknown=".len());
        let valid_max_bel = format!("\x1b]7501;state=idle:unknown={max_fill}\x07");
        let valid_max_st = format!("\x1b]7501;state=idle:unknown={max_fill}\x1b\\");
        let over_bel = format!("\x1b]7501;state=idle:unknown=a{max_fill}\x07");
        let over_st = format!("\x1b]7501;state=idle:unknown=a{max_fill}\x1b\\");

        let cases: &[(&str, &[u8])] = &[
            ("bel-report", b"\x1b]7501;state=working:app=codex\x07"),
            ("st-report", b"\x1b]7501;state=blocked:kind=auth\x1b\\"),
            ("bel-query", b"\x1b]7501;?\x07"),
            ("st-query", b"\x1b]7501;?\x1b\\"),
            ("embedded-tab", b"\x1b]7501;state=blo\tcked\x07"),
            ("embedded-cr", b"\x1b]7501;state=wor\rking\x07"),
            ("can-cancel", b"\x1b]7501;state=blocked\x18\x07"),
            ("sub-cancel", b"\x1b]7501;state=blocked\x1a\x1b\\"),
            ("stray-esc-st", b"\x1b]7501;state=blocked\x1b\x1b\\"),
            ("dcs-wrapped", b"\x1bP\x1b]7501;state=blocked\x07\x1b\\"),
            (
                "esc-back-to-back",
                b"\x1b]7501;state=blocked\x1b]7501;state=clear\x1b\\",
            ),
            ("empty-msg", b"\x1b]7501;state=done:msg=:title=\x07"),
            (
                "dup-msg-last-valid",
                b"\x1b]7501;state=working:msg=a:msg=YQ==\x07",
            ),
            (
                "dup-msg-last-invalid",
                b"\x1b]7501;state=working:msg=YQ==:msg=a\x07",
            ),
            ("bad-state", b"\x1b]7501;state=BLOCKED\x07"),
            ("bad-id", b"\x1b]7501;state=working:id=bad!\x07"),
            ("non-7501-osc", b"\x1b]2;hello\x07"),
            ("max-bel", valid_max_bel.as_bytes()),
            ("max-st", valid_max_st.as_bytes()),
            ("over-bel", over_bel.as_bytes()),
            ("over-st", over_st.as_bytes()),
        ];

        for &(name, stream) in cases {
            let mut h = Harness::new();
            let vt_events = h.scan(stream);
            let vt_recognized = !vt_events.is_empty();

            // Feed byte-by-byte to exercise every split boundary.
            let mut filter = ActivityFilter::default();
            let mut counted = 0usize;
            for &b in stream {
                counted += filter.observe(&[b]);
            }
            if vt_recognized {
                assert!(
                    counted < stream.len(),
                    "{name}: Ghostty parsed OSC 7501 ({vt_events:?}) but ActivityFilter counted all {counted}/{} bytes",
                    stream.len()
                );
            } else {
                assert_eq!(
                    counted,
                    stream.len(),
                    "{name}: Ghostty rejected stream but ActivityFilter excluded {} bytes",
                    stream.len() - counted
                );
            }
        }
    }

    #[test]
    fn root_and_active_children_stay_separate_across_updates_clears_shell_marks_and_ris() {
        let mut h = Harness::new();
        let mut scan_update = |bytes: &[u8]| -> Update {
            h.scan(bytes)
                .into_iter()
                .find_map(|event| match event {
                    TestEvent::Changed(update) => Some(*update),
                    _ => None,
                })
                .expect("expected Changed update")
        };
        let active_for = |update: &Update, app: &str| -> usize {
            update
                .active_children
                .iter()
                .filter(|c| c.app.as_deref() == Some(app))
                .map(|c| c.active)
                .sum()
        };

        let root_done = scan_update(b"\x1b]7501;state=done:app=claude-code\x1b\\");
        assert_eq!(root_done.root.as_ref().unwrap().state, State::Done);
        assert!(root_done.active_children.is_empty());

        let one = scan_update(b"\x1b]7501;state=working:id=bg1:app=claude-code\x1b\\");
        assert_eq!(one.root.as_ref().unwrap().state, State::Done);
        assert_eq!(
            one.trigger.as_ref().unwrap().app.as_deref(),
            Some("claude-code")
        );
        assert_eq!(active_for(&one, "claude-code"), 1);

        let two = scan_update(b"\x1b]7501;state=blocked:id=bg2:kind=question\x1b\\");
        assert_eq!(two.root.as_ref().unwrap().state, State::Done);
        assert_eq!(
            two.trigger.as_ref().unwrap().app.as_deref(),
            Some("claude-code"),
            "child trigger inherits ancestor app"
        );
        assert_eq!(active_for(&two, "claude-code"), 2);

        let done_child = scan_update(b"\x1b]7501;state=done:id=bg3\x1b\\");
        assert_eq!(active_for(&done_child, "claude-code"), 2);

        let foreign = scan_update(b"\x1b]7501;state=working:id=sub:app=other\x1b\\");
        assert_eq!(active_for(&foreign, "claude-code"), 2);
        assert_eq!(active_for(&foreign, "other"), 1);

        let cleared_one = scan_update(b"\x1b]7501;state=clear:id=bg1\x1b\\");
        assert_eq!(cleared_one.root.as_ref().unwrap().state, State::Done);
        assert_eq!(active_for(&cleared_one, "claude-code"), 1);
        assert_eq!(active_for(&cleared_one, "other"), 1);

        let boundary = scan_update(b"\x1b]133;D;0\x07\x1b]133;A\x07");
        assert!(boundary.trigger.is_none());
        assert_eq!(boundary.root.as_ref().unwrap().state, State::Done);
        assert!(boundary.active_children.is_empty());

        scan_update(b"\x1b]7501;state=blocked:app=claude-code:kind=permission\x1b\\");
        let blocked_cleared = scan_update(b"\x1b]133;D;0\x07\x1b]133;A\x07");
        assert!(blocked_cleared.trigger.is_none());
        assert!(blocked_cleared.root.is_none());
        assert!(blocked_cleared.current.is_some());

        scan_update(b"\x1b]7501;state=done:app=claude-code\x1b\\");
        scan_update(b"\x1b]7501;state=working:id=bg4:app=claude-code\x1b\\");
        let cleared_all = scan_update(b"\x1b]7501;state=clear\x1b\\");
        assert!(cleared_all.root.is_none());
        assert!(cleared_all.active_children.is_empty());
        assert!(cleared_all.current.is_none());

        scan_update(b"\x1b]7501;state=idle:app=claude-code\x1b\\");
        scan_update(b"\x1b]7501;state=working:id=bg5:app=claude-code\x1b\\");
        let ris = scan_update(b"\x1bc");
        assert!(ris.root.is_none());
        assert!(ris.active_children.is_empty());

        let mut filter = ActivityFilter::default();
        assert_eq!(
            filter.observe(b"\x1b]7501;state=working:id=bg1:app=claude-code\x1b\\ok"),
            2
        );
    }
}
