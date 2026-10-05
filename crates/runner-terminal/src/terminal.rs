use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions as _, Scroll};
use alacritty_terminal::index::{Boundary, Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle, KeyboardModes};
use anyhow::{Context as _, Result};
use regex::Regex;
use runner_core::protocol::terminal::{
    TerminalAttachment, TerminalFrame, TerminalMetadata, TerminalPalette, TerminalSnapshot,
};
use runner_core::protocol::{ClientError, ClientEvent, DaemonClient};

mod model;
pub use model::{ModelOptions, TerminalHost, TerminalModel};

use crate::palette;
use crate::{
    fixtures::FixtureRecorder,
    input_state::{InputEvent, InputObservation, InputTracker},
};

pub const XTERM_WORD_SEPARATORS: &str = " ()[]{}',\"`";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LinkTarget {
    Url(String),
    File {
        path: PathBuf,
        line: Option<u32>,
        column: Option<u32>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalLink {
    pub target: LinkTarget,
    pub start: Point,
    pub end: Point,
}

impl TerminalLink {
    pub fn contains(&self, point: Point) -> bool {
        if point.line < self.start.line || point.line > self.end.line {
            return false;
        }
        if self.start.line == self.end.line {
            return point.column >= self.start.column && point.column <= self.end.column;
        }
        (point.line != self.start.line || point.column >= self.start.column)
            && (point.line != self.end.line || point.column <= self.end.column)
    }
}

pub struct EventProxy {
    tx: Sender<Event>,
    waker: Arc<dyn Fn() + Send + Sync>,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        let _ = self.tx.send(event);
        (self.waker)();
    }
}

pub fn query_color<T>(
    term: &Term<T>,
    index: usize,
    base: &[alacritty_terminal::vte::ansi::Rgb; 256],
) -> alacritty_terminal::vte::ansi::Rgb {
    query_color_for(term, index, base, palette::RUNNER)
}

pub fn query_color_for<T>(
    term: &Term<T>,
    index: usize,
    base: &[alacritty_terminal::vte::ansi::Rgb; 256],
    theme: palette::TerminalPalette,
) -> alacritty_terminal::vte::ansi::Rgb {
    let stored = if index < alacritty_terminal::term::color::COUNT {
        term.colors()[index]
    } else {
        None
    };
    stored.unwrap_or_else(|| palette::resolve_index_for(index, base, theme))
}

/// DEC private mode 2031: colour-scheme change notifications (Contour's
/// spec, spoken by Ghostty, Kitty and WezTerm, listened for by Claude Code on
/// `/theme` auto). `alacritty_terminal` files 2031 under unknown modes, so it
/// ignores the set/reset, drops the `CSI ? 996 n` query and answers the
/// DECRQM probe with "not recognised". The session tracks the subscription by
/// scanning its output stream, rewrites that one probe answer, and reports
/// `CSI ? 997 ; 1|2 n` when a palette swap flips the ground's lightness.
const SCHEME_SUBSCRIBE: &[u8] = b"\x1b[?2031h";
const SCHEME_UNSUBSCRIBE: &[u8] = b"\x1b[?2031l";
const SCHEME_QUERY: &[u8] = b"\x1b[?996n";
const SCHEME_PROBE_UNSUPPORTED: &str = "\x1b[?2031;0$y";

/// `alacritty_terminal` answers `CSI ? u` with every flag the app pushed.
/// Runner implements only the disambiguate level, so the session masks the
/// answer down to that bit: an app detects a partial implementation by
/// pushing flags and reading back which stuck, and would otherwise wait for
/// key-up, repeat or all-keys events that never come. The answer is built
/// at the query's position in the output, so masking that text keeps a
/// batched push, query and pop honest, where reading the live mode from the
/// reply worker would answer every query in the chunk from its last byte.
fn kitty_flags_reply(text: &str) -> Option<KeyboardModes> {
    let flags = text.strip_prefix("\x1b[?")?.strip_suffix('u')?;
    if !flags.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(KeyboardModes::from_bits_truncate(flags.parse().ok()?))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SchemeSequence {
    Subscribe,
    Unsubscribe,
    Query,
}

#[derive(Default)]
struct SchemeState {
    subscribed: AtomicBool,
    tail: Mutex<Vec<u8>>,
}

/// The scheme sequences that end inside `chunk`, as (end offset in `chunk`,
/// sequence) in stream order. `tail` carries the last bytes of the previous
/// chunk so a sequence split across two PTY reads is seen, and seen once: a
/// match ending inside the carried bytes was already counted.
fn scan_scheme_sequences(tail: &mut Vec<u8>, chunk: &[u8]) -> Vec<(usize, SchemeSequence)> {
    const TAIL: usize = 8;
    let mut buf = std::mem::take(tail);
    let boundary = buf.len();
    buf.extend_from_slice(chunk);
    let mut found = Vec::new();
    for (needle, sequence) in [
        (SCHEME_SUBSCRIBE, SchemeSequence::Subscribe),
        (SCHEME_UNSUBSCRIBE, SchemeSequence::Unsubscribe),
        (SCHEME_QUERY, SchemeSequence::Query),
    ] {
        for (start, window) in buf.windows(needle.len()).enumerate() {
            let end = start + needle.len();
            if window == needle && end > boundary {
                found.push((end, sequence));
            }
        }
    }
    found.sort_by_key(|(end, _)| *end);
    *tail = buf[buf.len().saturating_sub(TAIL)..].to_vec();
    found
        .into_iter()
        .map(|(end, sequence)| (end - boundary, sequence))
        .collect()
}

fn scheme_report(palette: palette::TerminalPalette) -> String {
    format!("\x1b[?997;{}n", if palette.is_light() { 2 } else { 1 })
}

/// OSC 7, `ESC ] 7 ; file://host/path` ended by BEL or ST: the shell's
/// working directory (#575). `vte` drops it as an unhandled OSC, so a shell
/// session scans its raw output for it before parsing.
const OSC7_INTRODUCER: &[u8] = b"\x1b]7;";
const OSC7_MAX_PAYLOAD: usize = 16 * 1024;

/// The OSC 7 reports in a shell's output. `carry` holds what the previous
/// chunk cut off: a prefix of the introducer, or an unterminated report.
#[derive(Default)]
struct CwdReports {
    carry: Vec<u8>,
}

impl CwdReports {
    /// The directory reported by the last valid report that completes in
    /// `chunk`, if any.
    fn scan_with_host(&mut self, chunk: &[u8], is_local: impl Fn(&str) -> bool) -> Option<PathBuf> {
        self.payloads(chunk)
            .iter()
            .rev()
            .find_map(|payload| parse_cwd_report(payload, &is_local))
    }

    /// The payloads of the reports that complete in `chunk`, in order.
    fn payloads(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        let joined;
        let buf = if self.carry.is_empty() {
            chunk
        } else {
            let mut carried = std::mem::take(&mut self.carry);
            carried.extend_from_slice(chunk);
            joined = carried;
            &joined
        };
        let (reports, carry) = scan_osc7(buf);
        self.carry = carry.to_vec();
        reports.into_iter().map(<[u8]>::to_vec).collect()
    }
}

/// The complete OSC 7 payloads in `buf`, in order, and the tail to carry into
/// the next chunk. BEL and ST terminate a report; ESC followed by anything
/// else, CAN and SUB abort it, as they do in `vte`. `0x9C` is not taken as
/// ST: in UTF-8 output it is a continuation byte.
fn scan_osc7(buf: &[u8]) -> (Vec<&[u8]>, &[u8]) {
    let mut reports = Vec::new();
    let mut pos = 0;
    while let Some(offset) = buf[pos..].iter().position(|&byte| byte == 0x1b) {
        let start = pos + offset;
        let rest = &buf[start..];
        if !rest.starts_with(OSC7_INTRODUCER) {
            if rest.len() < OSC7_INTRODUCER.len() && OSC7_INTRODUCER.starts_with(rest) {
                return (reports, rest);
            }
            pos = start + 1;
            continue;
        }
        let body = start + OSC7_INTRODUCER.len();
        let Some(end) = buf[body..]
            .iter()
            .position(|byte| matches!(byte, 0x07 | 0x18 | 0x1a | 0x1b))
            .map(|offset| body + offset)
        else {
            let overlong = buf.len() - body > OSC7_MAX_PAYLOAD;
            return (reports, if overlong { &[] } else { rest });
        };
        let payload = &buf[body..end];
        let mut complete = |next: usize| {
            if payload.len() <= OSC7_MAX_PAYLOAD {
                reports.push(payload);
            }
            next
        };
        pos = match (buf[end], buf.get(end + 1)) {
            (0x07, _) => complete(end + 1),
            (0x1b, Some(b'\\')) => complete(end + 2),
            (0x1b, None) if payload.len() <= OSC7_MAX_PAYLOAD => return (reports, rest),
            (0x1b, _) => end,
            _ => end + 1,
        };
    }
    (reports, &[])
}

/// The directory an OSC 7 payload reports: a `file://` URI whose host is
/// this machine and whose path, cut at `?` or `#` and percent-decoded, is
/// absolute UTF-8. Anything else reports nothing.
fn parse_cwd_report(payload: &[u8], is_local: &impl Fn(&str) -> bool) -> Option<PathBuf> {
    let text = std::str::from_utf8(payload).ok()?;
    if !text.get(..7)?.eq_ignore_ascii_case("file://") {
        return None;
    }
    let (host, path) = text[7..].split_at(text[7..].find('/')?);
    if !is_local(&percent_decode(host)) {
        return None;
    }
    let path = String::from_utf8(percent_decode_bytes(path.split(['?', '#']).next()?)).ok()?;
    #[cfg(windows)]
    let path = strip_drive_slash(&path).to_owned();
    let path = PathBuf::from(path);
    path.is_absolute().then_some(path)
}

/// `/C:/Users/me` as a Windows path: a file URI's path keeps a slash before
/// the drive letter.
#[cfg(any(windows, test))]
fn strip_drive_slash(path: &str) -> &str {
    match path.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => &path[1..],
        _ => path,
    }
}

/// vte holds every byte of a synchronized update (`ESC[?2026h` … `ESC[?2026l`)
/// and records a deadline for it, but only its caller can act on that deadline:
/// alacritty's event loop does, and Runner, which feeds the parser itself, runs
/// a flusher per session instead. `flush_scheduled` lives under the parser's
/// lock so a feed that opens an update and a flusher that finds none pending
/// cannot interleave and leave a deadline unwatched.
#[derive(Default)]
struct ParserState {
    processor: crate::snapshot::TrackedProcessor,
    flush_scheduled: bool,
    boundary: crate::snapshot::BoundaryScanner,
}

#[derive(Default)]
struct SequenceState {
    last: u64,
    // No production reader after M6.11; kept as a cheaper mode-level signal should one appear.
    tui_ready_seq: u64,
    first_paint_seq: u64,
    last_output_at: Option<Instant>,
}

fn sanitize_title(raw: &str) -> String {
    raw.chars()
        .take(512)
        .map(|c| {
            if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .collect()
}

#[derive(Clone, Copy)]
struct PaletteState {
    theme: palette::TerminalPalette,
    base: [alacritty_terminal::vte::ansi::Rgb; 256],
}

impl PaletteState {
    fn new(theme: palette::TerminalPalette) -> Self {
        Self {
            theme,
            base: palette::base_palette_for(theme),
        }
    }
}

fn parse(parser: &mut ParserState, term: &mut Term<EventProxy>, bytes: &[u8]) {
    parser.processor.advance(term, bytes);
    let held = parser
        .processor
        .sync_timeout()
        .sync_timeout()
        .map(|_| parser.processor.sync_bytes_count());
    parser.boundary.advance(bytes, held);
}

pub struct TerminalMirror {
    pub term: Arc<FairMutex<Term<EventProxy>>>,
    client: DaemonClient,
    session_id: String,
    subscriber_id: AtomicU64,
    parser: Mutex<ParserState>,
    sync_flush: Sender<()>,
    sequence: Mutex<SequenceState>,
    size: Mutex<(u16, u16)>,
    metadata: Mutex<TerminalMetadata>,
    palette: Mutex<PaletteState>,
    waker: Arc<dyn Fn() + Send + Sync>,
    viewers: Arc<AtomicUsize>,
    link_cwd: std::sync::OnceLock<Option<PathBuf>>,
    config: Mutex<Config>,
}

pub struct TerminalView {
    viewers: Arc<AtomicUsize>,
}

impl Drop for TerminalView {
    fn drop(&mut self) {
        self.viewers.fetch_sub(1, Ordering::Release);
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TerminalScrollState {
    pub history_lines: usize,
    pub display_offset: usize,
    pub screen_lines: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TerminalOutputActivity {
    pub last_seq: u64,
    pub tui_ready_seq: u64,
    pub first_paint_seq: u64,
    pub last_output_at: Option<Instant>,
}

/// `alacritty_terminal` leaves the kitty keyboard protocol off by default,
/// which drops an app's `CSI ? u` query, so pi, Claude Code and Codex fall
/// back to legacy key decoding. `configure` rebuilds the config from here
/// too, or it would switch the protocol back off.
fn term_config() -> Config {
    Config {
        kitty_keyboard: true,
        ..Config::default()
    }
}

impl TerminalMirror {
    pub fn attach(
        client: DaemonClient,
        session_id: String,
        waker: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Arc<Self>> {
        let attachment = client.attach(&session_id)?;
        let (events, discarded) = mpsc::channel();
        drop(discarded);
        let (sync_flush, requests) = mpsc::channel();
        let viewers = Arc::new(AtomicUsize::new(0));
        let listener_viewers = Arc::clone(&viewers);
        let listener_wake = Arc::clone(&waker);
        let listener_wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            if listener_viewers.load(Ordering::Acquire) > 0 {
                listener_wake();
            }
        });
        let term = Arc::new(FairMutex::new(Term::new(
            term_config(),
            &TermSize::new(
                attachment.snapshot.cols as usize,
                attachment.snapshot.rows as usize,
            ),
            EventProxy {
                tx: events,
                waker: listener_wake,
            },
        )));
        let mirror = Arc::new(Self {
            term,
            client,
            session_id,
            subscriber_id: AtomicU64::new(attachment.subscriber_id),
            parser: Mutex::default(),
            sync_flush,
            sequence: Mutex::default(),
            size: Mutex::new((attachment.snapshot.cols, attachment.snapshot.rows)),
            metadata: Mutex::default(),
            palette: Mutex::new(PaletteState::new(palette::RUNNER)),
            waker,
            viewers,
            link_cwd: std::sync::OnceLock::new(),
            config: Mutex::new(term_config()),
        });
        mirror.restore(&attachment);
        mirror.refresh_metadata();
        let weak = Arc::downgrade(&mirror);
        thread::Builder::new()
            .name(format!("native-term-frames-{}", mirror.session_id))
            .spawn(move || {
                let mut frames = attachment.frames;
                loop {
                    let frame = match frames.recv() {
                        Ok(frame) => frame,
                        Err(_) => return,
                    };
                    let Some(mirror) = weak.upgrade() else {
                        return;
                    };
                    match frame {
                        TerminalFrame::Output { seq, bytes } => mirror.feed_output(seq, &bytes),
                        TerminalFrame::Resized { seq, cols, rows } => {
                            let mut sequence = mirror.sequence.lock().unwrap();
                            if seq > sequence.last {
                                sequence.last = seq;
                                mirror.resize_local(cols, rows);
                            }
                        }
                        TerminalFrame::Resync => match mirror.client.attach(&mirror.session_id) {
                            Ok(attachment) => {
                                mirror.restore(&attachment);
                                frames = attachment.frames;
                            }
                            Err(_) => return,
                        },
                    }
                    mirror.refresh_metadata();
                    mirror.wake_viewers();
                }
            })
            .context("spawn terminal mirror frames")?;
        let weak = Arc::downgrade(&mirror);
        thread::Builder::new()
            .name(format!("native-term-sync-{}", mirror.session_id))
            .spawn(move || {
                while requests.recv().is_ok() {
                    loop {
                        let Some(next) = weak.upgrade().map(|mirror| mirror.flush_sync_update())
                        else {
                            return;
                        };
                        let Some(deadline) = next else {
                            break;
                        };
                        if let Err(RecvTimeoutError::Disconnected) = requests
                            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                        {
                            return;
                        }
                    }
                }
            })
            .context("spawn terminal mirror sync flush")?;
        Ok(mirror)
    }

    fn restore(&self, attachment: &TerminalAttachment) {
        let mut sequence = self.sequence.lock().unwrap();
        let mut parser = self.parser.lock().unwrap();
        let mut term = self.term.lock();
        let (events, discarded) = mpsc::channel();
        drop(discarded);
        *term = Term::new(
            self.config.lock().unwrap().clone(),
            &TermSize::new(
                attachment.snapshot.cols as usize,
                attachment.snapshot.rows as usize,
            ),
            EventProxy {
                tx: events,
                waker: {
                    let viewers = Arc::clone(&self.viewers);
                    let wake = Arc::clone(&self.waker);
                    Arc::new(move || {
                        if viewers.load(Ordering::Acquire) > 0 {
                            wake();
                        }
                    })
                },
            },
        );
        *parser = ParserState::default();
        let split = attachment.snapshot.bytes.len() - attachment.snapshot.unfinished_len;
        parse(&mut parser, &mut term, &attachment.snapshot.bytes[..split]);
        parser
            .processor
            .restore_preceding_char(attachment.snapshot.preceding_char);
        parse(&mut parser, &mut term, &attachment.snapshot.bytes[split..]);
        sequence.last = attachment.snapshot.seq;
        mark_paint(
            &mut sequence,
            &mut term,
            TermMode::empty(),
            attachment.snapshot.seq,
        );
        *self.size.lock().unwrap() = (attachment.snapshot.cols, attachment.snapshot.rows);
        self.subscriber_id
            .store(attachment.subscriber_id, Ordering::Relaxed);
        if parser.processor.sync_timeout().sync_timeout().is_some() {
            parser.flush_scheduled = true;
            let _ = self.sync_flush.send(());
        }
    }

    pub fn refresh_metadata(&self) {
        if let Ok(metadata) = self.client.terminal_metadata(&self.session_id) {
            *self.metadata.lock().unwrap() = metadata;
        }
    }
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn view(&self) -> TerminalView {
        self.viewers.fetch_add(1, Ordering::AcqRel);
        TerminalView {
            viewers: Arc::clone(&self.viewers),
        }
    }
    pub fn title(&self) -> String {
        self.metadata.lock().unwrap().title.clone()
    }
    pub fn set_palette(&self, palette: palette::TerminalPalette) {
        *self.palette.lock().unwrap() = PaletteState::new(palette);
        (self.waker)();
    }
    pub fn palette(&self) -> palette::TerminalPalette {
        self.palette.lock().unwrap().theme
    }
    pub fn configure(&self, scrollback: usize, cursor_shape: CursorShape) {
        let config = Config {
            scrolling_history: scrollback,
            semantic_escape_chars: XTERM_WORD_SEPARATORS.to_owned(),
            default_cursor_style: CursorStyle {
                shape: cursor_shape,
                blinking: true,
            },
            ..term_config()
        };
        *self.config.lock().unwrap() = config.clone();
        self.term.lock().set_options(config);
        let shape = match cursor_shape {
            CursorShape::Block => "block",
            CursorShape::Underline => "underline",
            CursorShape::Beam => "beam",
            CursorShape::HollowBlock => "hollow",
            CursorShape::Hidden => "hidden",
        };
        let _ = self
            .client
            .terminal_configure(&self.session_id, scrollback, shape);
        (self.waker)();
    }
    pub fn output_activity(&self) -> TerminalOutputActivity {
        let sequence = self.sequence.lock().unwrap();
        TerminalOutputActivity {
            last_seq: sequence.last,
            tui_ready_seq: sequence.tui_ready_seq,
            first_paint_seq: sequence.first_paint_seq,
            last_output_at: sequence.last_output_at,
        }
    }
    fn feed_output(&self, seq: u64, bytes: &[u8]) {
        let mut sequence = self.sequence.lock().unwrap();
        if seq <= sequence.last {
            return;
        }
        sequence.last = seq;
        sequence.last_output_at = Some(Instant::now());
        if chunk_indicates_tui_ready(bytes) {
            sequence.tui_ready_seq = sequence.tui_ready_seq.max(seq);
        }
        let mut parser = self.parser.lock().unwrap();
        let mut term = self.term.lock();
        let previous = *term.mode();
        parse(&mut parser, &mut term, bytes);
        mark_paint(&mut sequence, &mut term, previous, seq);
        if parser.processor.sync_timeout().sync_timeout().is_some()
            && !std::mem::replace(&mut parser.flush_scheduled, true)
        {
            let _ = self.sync_flush.send(());
        }
    }
    fn flush_sync_update(&self) -> Option<Instant> {
        let mut sequence = self.sequence.lock().unwrap();
        let mut parser = self.parser.lock().unwrap();
        match parser.processor.sync_timeout().sync_timeout() {
            Some(deadline) if deadline > Instant::now() => return Some(deadline),
            Some(_) => {}
            None => {
                parser.flush_scheduled = false;
                return None;
            }
        }
        let mut term = self.term.lock();
        let previous = *term.mode();
        parser.processor.stop_sync(&mut *term);
        parser.boundary.end_sync();
        parser.flush_scheduled = false;
        let seq = sequence.last;
        mark_paint(&mut sequence, &mut term, previous, seq);
        drop(term);
        drop(parser);
        drop(sequence);
        self.wake_viewers();
        None
    }
    fn wake_viewers(&self) {
        if self.viewers.load(Ordering::Acquire) > 0 {
            (self.waker)();
        }
    }
    pub fn submit_text(&self, text: &str) -> std::result::Result<(), ClientError> {
        self.send_text(text)?;
        self.send_key("enter", false, false, false, None)?;
        Ok(())
    }
    pub fn write_user_bytes(&self, bytes: &[u8]) -> std::result::Result<(), ClientError> {
        self.clear_selection();
        self.client.input(&self.session_id, bytes)
    }
    pub fn send_key(
        &self,
        key: &str,
        ctrl: bool,
        alt: bool,
        shift: bool,
        key_char: Option<&str>,
    ) -> std::result::Result<bool, ClientError> {
        let input = InputEvent::Key {
            kind: crate::mappings::classify_key(key, ctrl, alt, shift, key_char),
        };
        let mode = *self.term.lock_unfair().mode();
        match crate::mappings::encode_key(key, ctrl, alt, shift, key_char, mode) {
            Some(bytes) => {
                self.observe_input(&input);
                self.write_user_bytes(&bytes)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }
    pub fn send_text(&self, text: &str) -> std::result::Result<(), ClientError> {
        if text.is_empty() {
            return Ok(());
        }
        self.observe_input(&InputEvent::Key {
            kind: crate::mappings::InputKind::Content {
                text: text.to_owned(),
            },
        });
        self.write_user_bytes(text.as_bytes())
    }
    pub fn set_composing(&self, composing: bool) {
        self.observe_input(&InputEvent::Composing { composing });
    }
    pub fn paste(&self, text: &str) -> std::result::Result<(), ClientError> {
        let bracketed = self
            .term
            .lock_unfair()
            .mode()
            .contains(TermMode::BRACKETED_PASTE);
        self.observe_input(&InputEvent::Paste {
            text: text.to_owned(),
        });
        self.write_user_bytes(&crate::mappings::encode_paste(text, bracketed))
    }
    fn observe_input(&self, input: &InputEvent) {
        let _ = self
            .client
            .terminal_observe_input(&self.session_id, input.clone());
    }
    pub fn input_reset_guard(&self) -> u64 {
        self.client
            .terminal_input_reset_guard(&self.session_id)
            .unwrap_or_default()
    }
    pub fn reset_input_state(&self, guard: u64) {
        let _ = self
            .client
            .terminal_reset_input_state(&self.session_id, guard);
    }
    pub fn resize(&self, cols: u16, rows: u16) {
        let (cols, rows) = (cols.max(2), rows.max(2));
        if self.size() == (cols, rows) {
            return;
        }
        self.resize_local(cols, rows);
        self.client.resize(
            &self.session_id,
            self.subscriber_id.load(Ordering::Relaxed),
            cols,
            rows,
        );
    }
    fn resize_local(&self, cols: u16, rows: u16) {
        let _parser = self.parser.lock().unwrap();
        let mut term = self.term.lock();
        let mut size = self.size.lock().unwrap();
        let rows_changed = size.1 != rows;
        *size = (cols, rows);
        term.resize(TermSize::new(cols as usize, rows as usize));
        if rows_changed {
            term.selection = None;
        }
    }
    pub fn size(&self) -> (u16, u16) {
        *self.size.lock().unwrap()
    }
    pub fn scroll(&self, delta_lines: i32, bypass_reporting: bool, column: usize, row: usize) {
        let mode = *self.term.lock_unfair().mode();
        match crate::mappings::encode_scroll(mode, delta_lines, bypass_reporting, column, row) {
            Some(bytes) => {
                if !bytes.is_empty() {
                    let _ = self.write_user_bytes(&bytes);
                }
            }
            None => {
                self.scroll_local(delta_lines);
            }
        }
    }

    pub fn scroll_local(&self, delta_lines: i32) {
        self.term.lock().scroll_display(Scroll::Delta(delta_lines));
        (self.waker)();
    }

    pub fn scroll_to_bottom(&self) {
        self.term.lock().scroll_display(Scroll::Bottom);
    }

    pub fn mode(&self) -> TermMode {
        *self.term.lock_unfair().mode()
    }

    pub fn start_selection(&self, ty: SelectionType, point: Point, side: Side) {
        self.term.lock().selection = Some(Selection::new(ty, point, side));
        (self.waker)();
    }

    pub fn update_selection(&self, point: Point, side: Side) -> bool {
        let mut term = self.term.lock();
        let Some(selection) = term.selection.as_mut() else {
            return false;
        };
        selection.update(point, side);
        drop(term);
        (self.waker)();
        true
    }

    pub fn clear_selection(&self) -> bool {
        let cleared = self.term.lock().selection.take().is_some();
        if cleared {
            (self.waker)();
        }
        cleared
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term
            .lock_unfair()
            .selection_to_string()
            .filter(|text| !text.is_empty())
    }

    pub fn selection_contains(&self, point: Point) -> bool {
        let term = self.term.lock_unfair();
        term.selection
            .as_ref()
            .and_then(|selection| selection.to_range(&*term))
            .is_some_and(|range| range.contains(point))
    }

    pub fn selection_type(&self) -> Option<SelectionType> {
        self.term
            .lock_unfair()
            .selection
            .as_ref()
            .map(|selection| selection.ty)
    }

    pub fn cell_is_whitespace(&self, point: Point) -> bool {
        let term = self.term.lock_unfair();
        let point = point.grid_clamp(&*term, Boundary::Grid);
        let cell = &term.grid()[point];
        !cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            && cell.c.is_whitespace()
    }

    /// The directory the shell last reported through OSC 7, while it still
    /// is one. `None` for any other runtime, for a shell that has not
    /// reported, and for a directory removed since its report.
    pub fn live_cwd(&self) -> Option<PathBuf> {
        self.metadata
            .lock()
            .unwrap()
            .live_cwd
            .clone()
            .filter(|cwd| cwd.is_dir())
    }

    /// Relative paths try the shell's live cwd first, then the spawn cwd, so
    /// output printed before a `cd` still resolves.
    pub fn link_at(&self, point: Point) -> Option<TerminalLink> {
        let live = self.metadata.lock().unwrap().live_cwd.clone();
        let spawn = self.link_cwd.get_or_init(|| {
            self.client
                .terminal_link_cwd(&self.session_id)
                .ok()
                .flatten()
        });
        let cwds: Vec<&Path> = live
            .as_deref()
            .into_iter()
            .chain(spawn.as_deref())
            .collect();
        terminal_link_at(&*self.term.lock_unfair(), point, &cwds)
    }

    pub fn move_cursor_to_viewport(&self, column: usize, row: usize) {
        let (cursor, columns, mode, display_offset) = {
            let term = self.term.lock_unfair();
            (
                term.grid().cursor.point,
                term.columns(),
                *term.mode(),
                term.grid().display_offset(),
            )
        };
        if display_offset != 0 || columns == 0 {
            return;
        }

        let mut bytes = Vec::new();
        if mode.contains(TermMode::ALT_SCREEN) {
            let vertical = row as i32 - cursor.line.0;
            let key = if vertical < 0 { "up" } else { "down" };
            if let Some(sequence) =
                crate::mappings::encode_key(key, false, false, false, None, mode)
            {
                bytes.extend(sequence.repeat(vertical.unsigned_abs() as usize));
            }
            let horizontal = column as i64 - cursor.column.0 as i64;
            let key = if horizontal < 0 { "left" } else { "right" };
            if let Some(sequence) =
                crate::mappings::encode_key(key, false, false, false, None, mode)
            {
                bytes.extend(sequence.repeat(horizontal.unsigned_abs() as usize));
            }
        } else {
            let cursor_index = cursor.line.0 as i64 * columns as i64 + cursor.column.0 as i64;
            let target_index = row as i64 * columns as i64 + column as i64;
            let delta = target_index - cursor_index;
            let key = if delta < 0 { "left" } else { "right" };
            if let Some(sequence) =
                crate::mappings::encode_key(key, false, false, false, None, mode)
            {
                bytes.extend(sequence.repeat(delta.unsigned_abs() as usize));
            }
        }
        if !bytes.is_empty() {
            let _ = self.write_user_bytes(&bytes);
        }
    }

    pub fn scroll_state(&self) -> TerminalScrollState {
        let term = self.term.lock_unfair();
        TerminalScrollState {
            history_lines: term.history_size(),
            display_offset: term.grid().display_offset(),
            screen_lines: term.screen_lines(),
        }
    }

    pub fn scroll_to_display_offset(&self, display_offset: usize) {
        let mut term = self.term.lock();
        let current = term.grid().display_offset();
        let target = display_offset.min(term.history_size());
        let delta = target as i64 - current as i64;
        term.scroll_display(Scroll::Delta(
            delta.clamp(i32::MIN as i64, i32::MAX as i64) as i32
        ));
        drop(term);
        (self.waker)();
    }
}

/// What a PTY chunk and a synchronized-update flush both do once the parser
/// has applied bytes, while the feed locks are still held.
fn observe_parsed(
    sequence: &mut SequenceState,
    input_tracker: &mut InputTracker,
    term: &mut Term<EventProxy>,
    previous_mode: TermMode,
    seq: u64,
    now: Instant,
) -> Option<InputObservation> {
    mark_paint(sequence, term, previous_mode, seq);
    input_tracker.observe_output(now, term)
}

fn mark_paint(
    sequence: &mut SequenceState,
    term: &mut Term<EventProxy>,
    previous_mode: TermMode,
    seq: u64,
) {
    if sequence.first_paint_seq == 0
        && term
            .grid()
            .display_iter()
            .any(|cell| !cell.c.is_whitespace())
    {
        sequence.first_paint_seq = seq;
    }
    if !previous_mode.intersects(TermMode::MOUSE_MODE)
        && term.mode().intersects(TermMode::MOUSE_MODE)
    {
        term.selection = None;
    }
}

struct LinkCell {
    point: Point,
    start: usize,
    end: usize,
    hyperlink: Option<(String, String)>,
}

fn terminal_link_at<T>(term: &Term<T>, point: Point, cwds: &[&Path]) -> Option<TerminalLink> {
    let mut point = point.grid_clamp(term, Boundary::Grid);
    if term.grid()[point]
        .flags
        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
    {
        point.column = Column(point.column.0.saturating_sub(1));
    }

    let start = term.line_search_left(point);
    let end = term.line_search_right(point);
    let mut text = String::new();
    let mut cells = Vec::new();
    for line in start.line.0..=end.line.0 {
        for column in 0..term.columns() {
            let cell_point = Point::new(Line(line), Column(column));
            let cell = &term.grid()[cell_point];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            let rendered_start = text.len();
            text.push(cell.c);
            if let Some(zerowidth) = cell.zerowidth() {
                text.extend(zerowidth.iter());
            }
            cells.push(LinkCell {
                point: cell_point,
                start: rendered_start,
                end: text.len(),
                hyperlink: cell
                    .hyperlink()
                    .map(|link| (link.id().to_owned(), link.uri().to_owned())),
            });
        }
    }

    let hit = cells.iter().position(|cell| cell.point == point)?;
    if let Some((id, uri)) = cells[hit].hyperlink.as_ref() {
        let mut first = hit;
        while first > 0
            && cells[first - 1]
                .hyperlink
                .as_ref()
                .is_some_and(|link| link.0 == *id)
        {
            first -= 1;
        }
        let mut last = hit;
        while last + 1 < cells.len()
            && cells[last + 1]
                .hyperlink
                .as_ref()
                .is_some_and(|link| link.0 == *id)
        {
            last += 1;
        }
        return Some(TerminalLink {
            target: file_target_from_uri(uri).unwrap_or_else(|| LinkTarget::Url(uri.clone())),
            start: cells[first].point,
            end: cells[last].point,
        });
    }

    let hit_cell = &cells[hit];
    let overlaps_hit =
        |range: &std::ops::Range<usize>| range.start < hit_cell.end && range.end > hit_cell.start;
    let (range, target) = match url_regex()
        .find_iter(&text)
        .find(|matched| overlaps_hit(&matched.range()))
    {
        Some(matched) => (
            matched.range(),
            LinkTarget::Url(matched.as_str().to_owned()),
        ),
        None => {
            let captures = file_link_regex()
                .captures_iter(&text)
                .find(|captures| overlaps_hit(&captures.get(0).map_or(0..0, |m| m.range())))?;
            (
                captures.get(0)?.range(),
                file_target_from_captures(&captures, cwds)?,
            )
        }
    };
    let first = cells.iter().find(|cell| cell.end > range.start)?;
    let last = cells.iter().rev().find(|cell| cell.start < range.end)?;
    Some(TerminalLink {
        target,
        start: first.point,
        end: last.point,
    })
}

/// A path-shaped token with an optional `:LINE[:COL]`, `(LINE,COL)`, or
/// `#LLINE` suffix. Deliberately loose: the filesystem check in
/// `resolve_file_candidate` is what separates `src/lib.rs` from `and/or`.
fn file_link_regex() -> &'static Regex {
    static FILE_REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    FILE_REGEX.get_or_init(|| {
        Regex::new(
            r##"(?x)
            (?P<path>
                (?: ~/ | \.\.?/ | / )?
                [^\s/:()\[\]{}<>'"`|\\,;=*?!\x23]* [^\s/:()\[\]{}<>'"`|\\,;=*?!\x23.]
                (?: / [^\s/:()\[\]{}<>'"`|\\,;=*?!\x23]* [^\s/:()\[\]{}<>'"`|\\,;=*?!\x23.] )*
            )
            (?:
                : (?P<line>\d+) (?: : (?P<column>\d+) )?
              | \( (?P<pline>\d+) , \s? (?P<pcolumn>\d+) \)
              | \x23 L (?P<hline>\d+)
            )?
            "##,
        )
        .expect("valid terminal file link regex")
    })
}

fn file_target_from_captures(captures: &regex::Captures<'_>, cwds: &[&Path]) -> Option<LinkTarget> {
    let path = resolve_file_candidate(captures.name("path")?.as_str(), cwds)?;
    let number = |name: &str| {
        captures
            .name(name)
            .and_then(|matched| matched.as_str().parse::<u32>().ok())
    };
    Some(LinkTarget::File {
        path,
        line: number("line")
            .or_else(|| number("pline"))
            .or_else(|| number("hline")),
        column: number("column").or_else(|| number("pcolumn")),
    })
}

fn resolve_file_candidate(candidate: &str, cwds: &[&Path]) -> Option<PathBuf> {
    let paths = if let Some(rest) = candidate.strip_prefix("~/") {
        vec![runner_core::app_paths::home_dir()?.join(rest)]
    } else if candidate.starts_with('/') {
        vec![PathBuf::from(candidate)]
    } else {
        cwds.iter().map(|cwd| cwd.join(candidate)).collect()
    };
    paths
        .iter()
        .map(|path| normalize_path(path))
        .find(|path| std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file()))
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other),
        }
    }
    normalized
}

fn file_target_from_uri(uri: &str) -> Option<LinkTarget> {
    let rest = uri.strip_prefix("file://")?;
    let (rest, fragment) = rest
        .split_once('#')
        .map_or((rest, None), |(rest, fragment)| (rest, Some(fragment)));
    let path = percent_decode(&rest[rest.find('/')?..]);
    let (path, line, column) = split_line_suffix(&path);
    let line = line.or_else(|| {
        fragment?
            .strip_prefix('L')?
            .split(|c: char| !c.is_ascii_digit())
            .next()?
            .parse()
            .ok()
    });
    Some(LinkTarget::File {
        path: PathBuf::from(path),
        line,
        column,
    })
}

fn split_line_suffix(path: &str) -> (&str, Option<u32>, Option<u32>) {
    let mut path = path;
    let mut numbers = Vec::new();
    while numbers.len() < 2 {
        let Some((head, tail)) = path.rsplit_once(':') else {
            break;
        };
        let Ok(number) = tail.parse::<u32>() else {
            break;
        };
        numbers.push(number);
        path = head;
    }
    match numbers.as_slice() {
        [line] => (path, Some(*line), None),
        [column, line] => (path, Some(*line), Some(*column)),
        _ => (path, None, None),
    }
}

fn percent_decode(text: &str) -> String {
    String::from_utf8_lossy(&percent_decode_bytes(text)).into_owned()
}

fn percent_decode_bytes(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = (bytes[index] == b'%' && index + 2 < bytes.len())
            .then(|| std::str::from_utf8(&bytes[index + 1..index + 3]).ok())
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                decoded.push(byte);
                index += 3;
            }
            None => {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
    }
    decoded
}

fn url_regex() -> &'static Regex {
    static URL_REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    URL_REGEX.get_or_init(|| {
        Regex::new(r#"(https?|HTTPS?)://[^\s\"'!*(){}|\\\^<>`]*[^\s\"':,.!?{}|\\\^~\[\]`()<>]"#)
            .expect("valid terminal URL regex")
    })
}

fn chunk_indicates_tui_ready(bytes: &[u8]) -> bool {
    [b"\x1b[?2004h".as_slice(), b"\x1b[?1049h", b"\x1b[?47h"]
        .into_iter()
        .any(|signal| bytes.windows(signal.len()).any(|window| window == signal))
}

pub struct TerminalBridge {
    client: DaemonClient,
    sessions: Mutex<HashMap<String, Arc<TerminalMirror>>>,
    palette: Mutex<palette::TerminalPalette>,
    waker: Arc<dyn Fn() + Send + Sync>,
}

impl TerminalBridge {
    pub fn new(client: DaemonClient, waker: Arc<dyn Fn() + Send + Sync>) -> Result<Arc<Self>> {
        let bridge = Arc::new(Self {
            client,
            sessions: Mutex::default(),
            palette: Mutex::new(palette::RUNNER),
            waker,
        });
        let observer: Arc<dyn runner_core::protocol::terminal::TerminalLifecycle> = bridge.clone();
        bridge.client.observe_terminals(Arc::downgrade(&observer));
        Ok(bridge)
    }

    pub fn attach(&self, session_id: &str) -> Result<Arc<TerminalMirror>> {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(mirror) = sessions.get(session_id) {
            return Ok(Arc::clone(mirror));
        }
        let mirror = TerminalMirror::attach(
            self.client.clone(),
            session_id.to_owned(),
            Arc::clone(&self.waker),
        )?;
        mirror.set_palette(*self.palette.lock().unwrap());
        sessions.insert(session_id.to_owned(), Arc::clone(&mirror));
        Ok(mirror)
    }
    pub fn handle_event(&self, event: &ClientEvent) {
        let Some(id) = event.payload.get("session_id").and_then(|id| id.as_str()) else {
            return;
        };
        match event.name.as_str() {
            "session/spawned" => {
                self.sessions.lock().unwrap().remove(id);
                if let Err(error) = self.attach(id) {
                    log::error!("attach terminal {id}: {error}");
                }
            }
            "session/exit" | "session/archived" => {
                self.sessions.lock().unwrap().remove(id);
            }
            "session/updated" => {
                if let Some(mirror) = self.session(id) {
                    mirror.refresh_metadata();
                }
            }
            _ => return,
        }
        (self.waker)();
    }
    pub fn session(&self, session_id: &str) -> Option<Arc<TerminalMirror>> {
        self.sessions.lock().unwrap().get(session_id).cloned()
    }
    pub fn titles(&self) -> HashMap<String, String> {
        self.sessions
            .lock()
            .unwrap()
            .iter()
            .map(|(id, terminal)| (id.clone(), terminal.title()))
            .collect()
    }
    pub fn set_palette(&self, palette: palette::TerminalPalette) {
        let sessions = self.sessions.lock().unwrap();
        let mut current = self.palette.lock().unwrap();
        if *current == palette {
            return;
        }
        *current = palette;
        let rgb = |color: alacritty_terminal::vte::ansi::Rgb| [color.r, color.g, color.b];
        let _ = self.client.terminal_palette(TerminalPalette {
            background: rgb(palette.background),
            foreground: rgb(palette.foreground),
            cursor: rgb(palette.cursor),
            cursor_accent: rgb(palette.cursor_accent),
            selection: rgb(palette.selection),
            ansi: palette.ansi.map(rgb),
        });
        for session in sessions.values() {
            session.set_palette(palette);
        }
    }
    pub fn live_session_count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }
}

pub fn palette_from_wire(palette: TerminalPalette) -> palette::TerminalPalette {
    let rgb = |[r, g, b]: [u8; 3]| alacritty_terminal::vte::ansi::Rgb { r, g, b };
    palette::TerminalPalette {
        background: rgb(palette.background),
        foreground: rgb(palette.foreground),
        cursor: rgb(palette.cursor),
        cursor_accent: rgb(palette.cursor_accent),
        selection: rgb(palette.selection),
        ansi: palette.ansi.map(rgb),
    }
}

pub type HostResult<T> = anyhow::Result<T>;

#[cfg(any(test, feature = "test-support"))]
impl TerminalMirror {
    pub fn test_feed(&self, seq: u64, bytes: &[u8]) {
        self.feed_output(seq, bytes);
        self.wake_viewers();
    }
    pub fn sync_state(&self) -> (Option<Instant>, usize, bool) {
        let parser = self.parser.lock().unwrap();
        (
            parser.processor.sync_timeout().sync_timeout(),
            parser.processor.sync_bytes_count(),
            parser.flush_scheduled,
        )
    }
    pub fn viewer_count(&self) -> usize {
        self.viewers.load(Ordering::Acquire)
    }
}

impl runner_core::protocol::terminal::TerminalLifecycle for TerminalBridge {
    fn event(&self, event: ClientEvent) {
        self.handle_event(&event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_parse_keeps_only_the_bytes_held_after_a_capacity_restart() {
        let (tx, _) = mpsc::channel();
        let mut term = Term::new(
            Config::default(),
            &TermSize::new(20, 5),
            EventProxy {
                tx,
                waker: Arc::new(|| {}),
            },
        );
        let mut parser = ParserState::default();
        let mut first = b"\x1b[?2026h".to_vec();
        first.resize(1024 * 1024, b'x');
        let mut second = vec![b'y'; 1024 * 1024 - 12];
        second.extend_from_slice(b"\x1b[?2026hheld");
        for chunk in [first.as_slice(), second.as_slice(), b"z\x1b[?2026hnew"] {
            parse(&mut parser, &mut term, chunk);
        }
        assert_eq!(parser.processor.sync_bytes_count(), 3);
        assert_eq!(parser.boundary.unfinished().len(), 11);
        assert_eq!(parser.boundary.unfinished(), b"\x1b[?2026hnew");
    }

    #[test]
    fn shell_title_sanitization_preserves_content_and_bounds_the_label() {
        assert_eq!(sanitize_title("  ⠋ shell ⠹  "), "  ⠋ shell ⠹  ");
        assert_eq!(sanitize_title("A\nB\tC\u{2028}D"), "A B C D");
        assert_eq!(sanitize_title(&"界".repeat(600)).chars().count(), 512);
    }

    fn osc7_payloads(chunks: &[&[u8]]) -> Vec<String> {
        let mut reports = super::CwdReports::default();
        chunks
            .iter()
            .flat_map(|chunk| reports.payloads(chunk))
            .map(|payload| String::from_utf8(payload).unwrap())
            .collect()
    }

    #[test]
    fn osc7_reports_end_at_bel_or_st_across_any_chunk_split() {
        let stream: &[u8] = "\x1b[31mls\x1b[0m\r\n\x1b]7;file://h/a\x07\x1b]0;title\x07\x1b]7;file://h/b%20c\x1b\\\x1b]8;;x\x1b\\$ "
            .as_bytes();
        let expected = vec!["file://h/a".to_string(), "file://h/b%20c".to_string()];
        assert_eq!(osc7_payloads(&[stream]), expected);
        for first in 0..=stream.len() {
            for second in first..=stream.len() {
                assert_eq!(
                    osc7_payloads(&[&stream[..first], &stream[first..second], &stream[second..]]),
                    expected,
                    "split at {first} and {second}"
                );
            }
        }
    }

    #[test]
    fn osc7_reports_abort_on_esc_can_and_sub_and_skip_overlong_payloads() {
        assert_eq!(
            osc7_payloads(&[b"\x1b]7;file:///a\x1b[0m\x1b]7;file:///b\x07"]),
            vec!["file:///b"]
        );
        assert_eq!(
            osc7_payloads(&[b"\x1b]7;file:///a\x1b", b"\x1b]7;file:///b\x07"]),
            vec!["file:///b"]
        );
        assert_eq!(
            osc7_payloads(&[b"\x1b]7;file:///a\x18 \x1b]7;file:///b\x1a \x1b]7;file:///c\x07"]),
            vec!["file:///c"]
        );
        let overlong = format!("\x1b]7;file:///{}", "a".repeat(super::OSC7_MAX_PAYLOAD));
        assert!(osc7_payloads(&[format!("{overlong}\x07").as_bytes()]).is_empty());
        assert_eq!(
            osc7_payloads(&[
                overlong.as_bytes(),
                b"aaaa\x07 \x1b]7;file:///ok\x1b",
                b"\\"
            ]),
            vec!["file:///ok"]
        );
        // 0x9C ends a report only as C1 ST, never inside UTF-8 text: 朝 is E6 9C 9D.
        assert_eq!(
            osc7_payloads(&["\x1b]7;file:///tmp/朝\x07".as_bytes()]),
            vec!["file:///tmp/朝"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn osc7_reports_name_an_absolute_directory_on_this_machine() {
        let parse = |uri: &str| {
            super::parse_cwd_report(uri.as_bytes(), &|host| {
                matches!(host, "" | "localhost" | "test-host")
            })
        };
        let host = "test-host";
        let label = host.split('.').next().unwrap().to_owned();
        let tmp = Some(std::path::PathBuf::from("/tmp/a b/中文"));
        assert_eq!(parse("file:///tmp/a%20b/%E4%B8%AD%E6%96%87"), tmp);
        assert_eq!(parse("file://localhost/tmp/a b/中文"), tmp);
        assert_eq!(parse("FILE://local%68ost/tmp/a%20b/中文"), tmp);
        assert_eq!(parse(&format!("file://{host}/tmp/a%20b/中文")), tmp);
        assert_eq!(parse(&format!("file://{label}/tmp/a%20b/中文")), tmp);
        assert_eq!(parse("file:///tmp/x?query#fragment"), Some("/tmp/x".into()));
        for ignored in [
            "file://runner-575-elsewhere.invalid/tmp",
            "kitty-shell-cwd://localhost/tmp",
            "http://localhost/tmp",
            "file://localhost",
            "file:",
            "file:///tmp/%FF",
            "",
            "garbage",
        ] {
            assert_eq!(parse(ignored), None, "{ignored:?}");
        }
        assert_eq!(
            super::parse_cwd_report(b"file:///tmp/\xff", &|_| true),
            None
        );
    }

    #[test]
    fn osc7_file_uri_paths_drop_the_slash_before_a_windows_drive() {
        assert_eq!(super::strip_drive_slash("/C:/Users/me"), "C:/Users/me");
        assert_eq!(super::strip_drive_slash("/d:"), "d:");
        assert_eq!(super::strip_drive_slash("/Users/me"), "/Users/me");
        assert_eq!(super::strip_drive_slash("/1:/x"), "/1:/x");
    }

    #[test]
    fn only_kitty_flag_queries_are_masked() {
        use super::kitty_flags_reply;
        let bits = |text| kitty_flags_reply(text).map(|flags| flags.bits());
        assert_eq!(bits("\x1b[?0u"), Some(0));
        assert_eq!(bits("\x1b[?31u"), Some(31));
        for other in [
            "\x1b[?u",
            "\x1b[?6c",
            "\x1b[?2031;0$y",
            "\x1b[0n",
            "\x1b[?1;2u",
            "\x1b[?999u",
        ] {
            assert_eq!(bits(other), None, "{other:?}");
        }
    }

    #[test]
    fn scheme_sequences_are_seen_once_across_chunk_splits() {
        use super::{scan_scheme_sequences, SchemeSequence};
        let mut tail = Vec::new();
        assert_eq!(
            scan_scheme_sequences(&mut tail, b"\x1b[?2031h\x1b[?996n"),
            vec![(8, SchemeSequence::Subscribe), (15, SchemeSequence::Query)]
        );
        assert_eq!(scan_scheme_sequences(&mut tail, b"plain"), vec![]);
        assert_eq!(scan_scheme_sequences(&mut tail, b"\x1b[?20"), vec![]);
        assert_eq!(
            scan_scheme_sequences(&mut tail, b"31l"),
            vec![(3, SchemeSequence::Unsubscribe)]
        );
        assert_eq!(scan_scheme_sequences(&mut tail, b""), vec![]);
        assert_eq!(scan_scheme_sequences(&mut tail, b"\x1b[?25h"), vec![]);
    }
}
