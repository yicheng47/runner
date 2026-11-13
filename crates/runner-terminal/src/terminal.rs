use std::collections::HashMap;
#[cfg(not(windows))]
use std::path::Component;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions as _, Scroll};
use alacritty_terminal::index::{Boundary, Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{ClipboardType, Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle, Handler, KeyboardModes};
use anyhow::{Context as _, Result};
use regex::Regex;
use runner_core::protocol::terminal::{
    TerminalAttachment, TerminalController, TerminalFrame, TerminalMetadata, TerminalPalette,
    TerminalSnapshot,
};
use runner_core::protocol::{ClientError, ClientEvent, DaemonClient};

pub mod find;
mod model;
pub use model::{ClipboardTarget, ModelOptions, TerminalHost, TerminalModel, MAX_CLIPBOARD_BYTES};

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
    spans: Vec<(Point, Point)>,
}

impl TerminalLink {
    pub fn contains(&self, point: Point) -> bool {
        self.spans.iter().any(|(start, end)| {
            point.line == start.line && point.column >= start.column && point.column <= end.column
        })
    }

    fn from_points(target: LinkTarget, points: impl Iterator<Item = Point>) -> Self {
        let mut spans: Vec<(Point, Point)> = Vec::new();
        for point in points {
            match spans.last_mut() {
                Some((_, end)) if end.line == point.line && end.column + 1 == point.column => {
                    *end = point;
                }
                _ => spans.push((point, point)),
            }
        }
        Self {
            target,
            start: spans[0].0,
            end: spans[spans.len() - 1].1,
            spans,
        }
    }
}

type PersistSessionTitle = Box<dyn FnOnce(&str, bool) -> Option<Box<dyn FnOnce() + Send>> + Send>;

enum ProxyEvent {
    Terminal(Event),
    SyncSessionTitle {
        initial_title: String,
        apply: Box<dyn FnOnce() + Send>,
    },
    ApplySessionTitle {
        title: String,
        persist: PersistSessionTitle,
    },
}

pub struct EventProxy {
    tx: Sender<ProxyEvent>,
    waker: Arc<dyn Fn() + Send + Sync>,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        let _ = self.tx.send(ProxyEvent::Terminal(event));
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

fn strip_file_scheme(uri: &str) -> Option<&str> {
    let prefix = uri.get(..7)?;
    prefix.eq_ignore_ascii_case("file://").then(|| &uri[7..])
}

/// The directory an OSC 7 payload reports: a `file://` URI whose host is
/// this machine and whose path, cut at `?` or `#` and percent-decoded, is
/// absolute UTF-8. Anything else reports nothing.
fn parse_cwd_report(payload: &[u8], is_local: &impl Fn(&str) -> bool) -> Option<PathBuf> {
    let text = std::str::from_utf8(payload).ok()?;
    let after_scheme = strip_file_scheme(text)?;
    let (host, path) = after_scheme.split_at(after_scheme.find('/')?);
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
    burst: BurstHold,
}

/// A remote mirror receives an agent's full-history reprint, sent when its
/// terminal is resized, as many relay chunks. Painting each chunk scrolls the
/// view through the whole history, so a clear-scrollback or a large volume
/// holds parsing until the stream is briefly quiet, and never past the cap.
/// Typing echo is far below the volume threshold and is parsed at once. An
/// explicit synchronized update stays with vte: the chunk that opens one, and
/// every chunk while one is open, releases anything held and is parsed at once.
const BURST_WINDOW: Duration = Duration::from_millis(100);
const BURST_BYTES: usize = 32 * 1024;
const BURST_QUIET: Duration = Duration::from_millis(80);
const BURST_CAP: Duration = Duration::from_millis(500);
const BURST_MAX_HELD: usize = 2 * 1024 * 1024;
const CLEAR_SCROLLBACK: &[u8] = b"\x1b[3J";
const BEGIN_SYNC: &[u8] = b"\x1b[?2026h";

#[derive(Default)]
struct BurstHold {
    window: Option<(Instant, usize)>,
    held: Vec<u8>,
    holding: Option<(Instant, Instant)>,
    // The stream's last bytes, so a marker split across chunks is still seen.
    tail: Vec<u8>,
}

#[derive(Debug, PartialEq)]
enum Burst {
    Pass,
    Held,
    Release(Vec<u8>),
}

impl BurstHold {
    fn feed(&mut self, bytes: &[u8], now: Instant, synchronized: bool) -> Burst {
        let begins_sync = self.contains(bytes, BEGIN_SYNC);
        let clears = self.contains(bytes, CLEAR_SCROLLBACK);
        self.tail.extend_from_slice(bytes);
        let excess = self.tail.len().saturating_sub(BEGIN_SYNC.len() - 1);
        self.tail.drain(..excess);
        if synchronized || begins_sync {
            if self.holding.is_none() {
                self.window = None;
                return Burst::Pass;
            }
            self.held.extend_from_slice(bytes);
            return Burst::Release(self.take());
        }
        let started = match self.holding {
            Some((started, _)) => started,
            None => {
                let total = match self.window {
                    Some((start, total)) if now.duration_since(start) <= BURST_WINDOW => {
                        self.window = Some((start, total + bytes.len()));
                        total + bytes.len()
                    }
                    _ => {
                        self.window = Some((now, bytes.len()));
                        bytes.len()
                    }
                };
                if total < BURST_BYTES && !clears {
                    return Burst::Pass;
                }
                now
            }
        };
        self.held.extend_from_slice(bytes);
        self.holding = Some((started, now));
        if self.held.len() >= BURST_MAX_HELD || now >= started + BURST_CAP {
            return Burst::Release(self.take());
        }
        Burst::Held
    }

    fn contains(&self, bytes: &[u8], marker: &[u8]) -> bool {
        let tail = &self.tail[self.tail.len().saturating_sub(marker.len() - 1)..];
        let mut edge = tail.to_vec();
        edge.extend_from_slice(&bytes[..bytes.len().min(marker.len() - 1)]);
        edge.windows(marker.len()).any(|window| window == marker)
            || bytes.windows(marker.len()).any(|window| window == marker)
    }

    fn deadline(&self) -> Option<Instant> {
        self.holding
            .map(|(started, last)| (last + BURST_QUIET).min(started + BURST_CAP))
    }

    fn take(&mut self) -> Vec<u8> {
        self.window = None;
        self.holding = None;
        std::mem::take(&mut self.held)
    }
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
    parse_with(parser, term, bytes);
}

fn parse_with<H: Handler>(parser: &mut ParserState, handler: &mut H, bytes: &[u8]) {
    parser.processor.advance(handler, bytes);
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
    control: Mutex<TerminalControl>,
    composition_clear_pending: Mutex<bool>,
    composition_clear_retry: AtomicBool,
    palette: Mutex<PaletteState>,
    waker: Arc<dyn Fn() + Send + Sync>,
    viewers: Arc<AtomicUsize>,
    frame_worker: Mutex<Option<FrameWorker>>,
    link_cwd: std::sync::OnceLock<Option<PathBuf>>,
    config: Mutex<Config>,
    coalesce_bursts: bool,
    /// Written only while the term is locked, so a reader holding the term
    /// lock sees the mark that matches the grid.
    scroll_mark: Mutex<find::ScrollMark>,
    viewport_holds: Arc<AtomicUsize>,
    /// Keys, typed text and pastes sent, so a blinking cursor shows again
    /// before the echo arrives.
    typed: AtomicU64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalControl {
    pub controller: TerminalController,
    pub last_remote_name: Option<String>,
}

impl TerminalControl {
    fn apply(&mut self, controller: TerminalController) {
        if let TerminalController::Browser { name, .. } = &controller {
            self.last_remote_name = Some(name.clone());
        }
        self.controller = controller;
    }

    pub fn desktop(&self) -> bool {
        self.controller == TerminalController::Desktop
    }
}

struct FrameWorker {
    state: Arc<Mutex<FrameWorkerState>>,
    thread: thread::JoinHandle<()>,
}

struct FrameWorkerState {
    stopped: Arc<AtomicBool>,
    cancel: Arc<dyn Fn() + Send + Sync>,
}

pub struct TerminalView {
    viewers: Arc<AtomicUsize>,
}

impl Drop for TerminalView {
    fn drop(&mut self) {
        self.viewers.fetch_sub(1, Ordering::Release);
    }
}

/// While any hold is alive, output arriving at the bottom of the scrollback
/// scrolls the viewport back so it stays on the same content.
pub struct ViewportHold {
    holds: Arc<AtomicUsize>,
}

impl Drop for ViewportHold {
    fn drop(&mut self) {
        self.holds.fetch_sub(1, Ordering::Release);
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
    fn start_frames(
        self: &Arc<Self>,
        attachment: runner_core::protocol::terminal::TerminalAttachment,
    ) -> Result<()> {
        let weak = Arc::downgrade(self);
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(Mutex::new(FrameWorkerState {
            stopped: stopping.clone(),
            cancel: attachment.frames.cancellation(),
        }));
        let stopped = stop.clone();
        let worker = thread::Builder::new()
            .name(format!("native-term-frames-{}", self.session_id))
            .spawn(move || {
                let mut frames = attachment.frames;
                loop {
                    let frame = match frames.recv() {
                        Ok(frame) => frame,
                        Err(_) => return,
                    };
                    if stopping.load(Ordering::Acquire) {
                        return;
                    }
                    let Some(mirror) = weak.upgrade() else {
                        return;
                    };
                    match frame {
                        TerminalFrame::Output { seq, bytes } => {
                            if !mirror.feed_output(seq, &bytes) {
                                continue;
                            }
                        }
                        TerminalFrame::Resized {
                            seq,
                            cols,
                            rows,
                            controller,
                        } => {
                            let mut sequence = mirror.sequence.lock().unwrap();
                            if seq > sequence.last {
                                sequence.last = seq;
                                mirror.release_burst(
                                    &mut sequence,
                                    &mut mirror.parser.lock().unwrap(),
                                );
                                mirror.resize_local(cols, rows);
                                let changed = mirror.apply_controller(controller);
                                if changed {
                                    (mirror.waker)();
                                }
                            }
                        }
                        TerminalFrame::Resync => match mirror.client.attach(&mirror.session_id) {
                            Ok(attachment) => {
                                let mut state = stopped.lock().unwrap();
                                if state.stopped.load(Ordering::Acquire) {
                                    return;
                                }
                                mirror.restore(&attachment);
                                state.cancel = attachment.frames.cancellation();
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
        *self.frame_worker.lock().unwrap() = Some(FrameWorker {
            state: stop,
            thread: worker,
        });
        Ok(())
    }
    pub fn reattach(self: &Arc<Self>) -> Result<()> {
        if let Some(worker) = self.frame_worker.lock().unwrap().take() {
            {
                let state = worker.state.lock().unwrap();
                state.stopped.store(true, Ordering::Release);
                (state.cancel)();
            }
            let _ = worker.thread.join();
        }
        let attachment = self.client.attach(&self.session_id)?;
        self.restore(&attachment);
        self.start_frames(attachment)?;
        self.refresh_metadata();
        self.wake_viewers();
        Ok(())
    }
    pub fn attach(
        client: DaemonClient,
        session_id: String,
        waker: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Arc<Self>> {
        Self::connect(client, session_id, waker, false)
    }

    /// A mirror of another computer's terminal, which paints output bursts once.
    pub fn attach_remote(
        client: DaemonClient,
        session_id: String,
        waker: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Arc<Self>> {
        Self::connect(client, session_id, waker, true)
    }

    fn connect(
        client: DaemonClient,
        session_id: String,
        waker: Arc<dyn Fn() + Send + Sync>,
        coalesce_bursts: bool,
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
            control: Mutex::default(),
            composition_clear_pending: Mutex::new(false),
            composition_clear_retry: AtomicBool::new(false),
            palette: Mutex::new(PaletteState::new(palette::RUNNER)),
            waker,
            viewers,
            frame_worker: Mutex::default(),
            link_cwd: std::sync::OnceLock::new(),
            config: Mutex::new(term_config()),
            coalesce_bursts,
            scroll_mark: Mutex::default(),
            viewport_holds: Arc::default(),
            typed: AtomicU64::new(0),
        });
        mirror.restore(&attachment);
        mirror.refresh_metadata();
        mirror.start_frames(attachment)?;
        let weak = Arc::downgrade(&mirror);
        thread::Builder::new()
            .name(format!("native-term-sync-{}", mirror.session_id))
            .spawn(move || {
                while requests.recv().is_ok() {
                    loop {
                        let Some(next) = weak.upgrade().map(|mirror| mirror.flush_held_output())
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

    fn restore(self: &Arc<Self>, attachment: &TerminalAttachment) {
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
        self.scroll_mark.lock().unwrap().rebuild();
        *self.size.lock().unwrap() = (attachment.snapshot.cols, attachment.snapshot.rows);
        self.apply_controller(attachment.snapshot.controller.clone());
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
    /// `blinking` is the default cursor's; a program's DECSCUSR still sets
    /// its own. It stays app-side: runnerd's model never draws a cursor.
    pub fn configure(&self, scrollback: usize, cursor_shape: CursorShape, blinking: bool) {
        let config = Config {
            scrolling_history: scrollback,
            semantic_escape_chars: XTERM_WORD_SEPARATORS.to_owned(),
            default_cursor_style: CursorStyle {
                shape: cursor_shape,
                blinking,
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
    /// Returns whether the bytes reached the terminal; a held burst has not.
    fn feed_output(&self, seq: u64, bytes: &[u8]) -> bool {
        let mut sequence = self.sequence.lock().unwrap();
        if seq <= sequence.last {
            return false;
        }
        let now = Instant::now();
        sequence.last = seq;
        sequence.last_output_at = Some(now);
        if chunk_indicates_tui_ready(bytes) {
            sequence.tui_ready_seq = sequence.tui_ready_seq.max(seq);
        }
        let mut parser = self.parser.lock().unwrap();
        let released;
        let bytes = if self.coalesce_bursts {
            let synchronized = parser.processor.sync_timeout().sync_timeout().is_some();
            match parser.burst.feed(bytes, now, synchronized) {
                Burst::Pass => bytes,
                Burst::Held => {
                    if !std::mem::replace(&mut parser.flush_scheduled, true) {
                        let _ = self.sync_flush.send(());
                    }
                    return false;
                }
                Burst::Release(held) => {
                    released = held;
                    &released
                }
            }
        } else {
            bytes
        };
        let mut term = self.term.lock();
        let previous = *term.mode();
        self.track_scroll(&mut term, |tracker| parse_with(&mut parser, tracker, bytes));
        mark_paint(&mut sequence, &mut term, previous, seq);
        if parser.processor.sync_timeout().sync_timeout().is_some()
            && !std::mem::replace(&mut parser.flush_scheduled, true)
        {
            let _ = self.sync_flush.send(());
        }
        true
    }
    fn release_burst(&self, sequence: &mut SequenceState, parser: &mut ParserState) -> bool {
        if parser.burst.deadline().is_none() {
            return false;
        }
        let held = parser.burst.take();
        let mut term = self.term.lock();
        let previous = *term.mode();
        self.track_scroll(&mut term, |tracker| parse_with(parser, tracker, &held));
        let seq = sequence.last;
        mark_paint(sequence, &mut term, previous, seq);
        true
    }
    fn flush_held_output(&self) -> Option<Instant> {
        let mut sequence = self.sequence.lock().unwrap();
        let mut parser = self.parser.lock().unwrap();
        let now = Instant::now();
        let mut flushed = parser
            .burst
            .deadline()
            .is_some_and(|deadline| deadline <= now)
            && self.release_burst(&mut sequence, &mut parser);
        if parser
            .processor
            .sync_timeout()
            .sync_timeout()
            .is_some_and(|deadline| deadline <= now)
        {
            let mut term = self.term.lock();
            let previous = *term.mode();
            self.track_scroll(&mut term, |tracker| parser.processor.stop_sync(tracker));
            parser.boundary.end_sync();
            let seq = sequence.last;
            mark_paint(&mut sequence, &mut term, previous, seq);
            flushed = true;
        }
        let next = [
            parser.burst.deadline(),
            parser.processor.sync_timeout().sync_timeout(),
        ]
        .into_iter()
        .flatten()
        .min();
        if next.is_none() {
            parser.flush_scheduled = false;
        }
        drop(parser);
        drop(sequence);
        if flushed {
            self.wake_viewers();
        }
        next
    }
    fn wake_viewers(&self) {
        if self.viewers.load(Ordering::Acquire) > 0 {
            (self.waker)();
        }
    }

    /// Feeds the term through a scroll tracker and records how far the
    /// content moved. A held viewport at the bottom scrolls back by the same
    /// amount, stopping at the oldest line; one already scrolled back is kept
    /// in place by alacritty.
    fn track_scroll(
        &self,
        term: &mut Term<EventProxy>,
        apply: impl FnOnce(&mut find::ScrollTracker<'_, EventProxy>),
    ) {
        let display_offset = term.grid().display_offset();
        let mut tracker = find::ScrollTracker::new(term);
        apply(&mut tracker);
        let moved = tracker.finish();
        if let Some(lines) = moved.filter(|lines| *lines > 0) {
            if display_offset == 0 && self.viewport_holds.load(Ordering::Acquire) > 0 {
                term.scroll_display(Scroll::Delta(i32::try_from(lines).unwrap_or(i32::MAX)));
            }
        }
        self.scroll_mark.lock().unwrap().advance(moved);
    }

    pub fn scroll_mark(&self) -> find::ScrollMark {
        *self.scroll_mark.lock().unwrap()
    }

    pub fn hold_viewport(&self) -> ViewportHold {
        self.viewport_holds.fetch_add(1, Ordering::AcqRel);
        ViewportHold {
            holds: Arc::clone(&self.viewport_holds),
        }
    }
    pub fn submit_text(&self, text: &str) -> std::result::Result<(), ClientError> {
        self.send_text(text)?;
        self.send_key("enter", false, false, false, None)?;
        Ok(())
    }
    pub fn write_user_bytes(&self, bytes: &[u8]) -> std::result::Result<(), ClientError> {
        if !self.control().desktop() {
            return Ok(());
        }
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
        let mode = *self.term.lock_unfair().mode();
        let input = InputEvent::Key {
            kind: crate::mappings::classify_key(mode, key, ctrl, alt, shift, key_char),
        };
        match crate::mappings::encode_key(key, ctrl, alt, shift, key_char, mode) {
            Some(bytes) => {
                self.observe_input(&input);
                self.write_typed_bytes(&bytes)?;
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
        self.write_typed_bytes(text.as_bytes())
    }
    pub fn set_composing(self: &Arc<Self>, composing: bool) {
        let mut pending = self.composition_clear_pending.lock().unwrap();
        if self.control().desktop() {
            let sent = self
                .client
                .terminal_observe_input(&self.session_id, InputEvent::Composing { composing })
                .is_ok();
            *pending = !composing && !sent;
        } else if !composing {
            // Clear the daemon's preedit after handover without sending input while covered.
            *pending = true;
        }
        drop(pending);
        self.retry_composition_clear();
    }

    fn retry_composition_clear(self: &Arc<Self>) {
        let pending = self.composition_clear_pending.lock().unwrap();
        if !*pending
            || !self.control().desktop()
            || self.composition_clear_retry.swap(true, Ordering::AcqRel)
        {
            return;
        }
        drop(pending);
        let weak = Arc::downgrade(self);
        thread::spawn(move || loop {
            thread::sleep(Duration::from_millis(100));
            let Some(mirror) = weak.upgrade() else {
                return;
            };
            let mut pending = mirror.composition_clear_pending.lock().unwrap();
            if *pending && mirror.control().desktop() {
                *pending = mirror
                    .client
                    .terminal_observe_input(
                        &mirror.session_id,
                        InputEvent::Composing { composing: false },
                    )
                    .is_err();
            }
            if !*pending || !mirror.control().desktop() {
                mirror
                    .composition_clear_retry
                    .store(false, Ordering::Release);
                return;
            }
        });
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
        self.write_typed_bytes(&crate::mappings::encode_paste(text, bracketed))
    }
    fn write_typed_bytes(&self, bytes: &[u8]) -> std::result::Result<(), ClientError> {
        self.write_user_bytes(bytes)?;
        if self.control().desktop() {
            self.typed.fetch_add(1, Ordering::Relaxed);
            (self.waker)();
        }
        Ok(())
    }
    /// How many keys, typed texts and pastes this mirror has sent.
    pub fn typed_count(&self) -> u64 {
        self.typed.load(Ordering::Relaxed)
    }
    fn observe_input(&self, input: &InputEvent) {
        if !self.control().desktop() {
            return;
        }
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
        if !self.control().desktop() {
            return;
        }
        let _ = self
            .client
            .terminal_reset_input_state(&self.session_id, guard);
    }
    fn apply_controller(self: &Arc<Self>, controller: TerminalController) -> bool {
        let changed = {
            let mut control = self.control.lock().unwrap();
            let changed = control.controller != controller;
            control.apply(controller);
            changed
        };
        let mut pending = self.composition_clear_pending.lock().unwrap();
        if *pending
            && self.control().desktop()
            && self
                .client
                .terminal_observe_input(
                    &self.session_id,
                    InputEvent::Composing { composing: false },
                )
                .is_ok()
        {
            *pending = false;
        }
        drop(pending);
        self.retry_composition_clear();
        changed
    }

    pub fn control(&self) -> TerminalControl {
        self.control.lock().unwrap().clone()
    }

    pub fn take_back(&self, cols: u16, rows: u16) -> std::result::Result<(), ClientError> {
        self.client.terminal_take_back(&self.session_id, cols, rows)
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        if !self.control().desktop() {
            return;
        }
        let (cols, rows) = (cols.max(2), rows.max(2));
        if self.size() == (cols, rows) {
            return;
        }
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
        if *size != (cols, rows) {
            self.scroll_mark.lock().unwrap().rebuild();
        }
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
}

fn link_row_content<T>(term: &Term<T>, line: Line) -> Option<(usize, usize)> {
    let row = &term.grid()[line];
    let first = (0..term.columns()).find(|&column| !row[Column(column)].c.is_whitespace())?;
    let last = (first..term.columns()).rfind(|&column| !row[Column(column)].c.is_whitespace())?;
    Some((first, last))
}

fn wrapped_hyperlink_ids_match(left: &str, right: &str) -> bool {
    left == right || (left.ends_with("_alacritty") && right.ends_with("_alacritty"))
}

enum Osc8Lookup {
    None,
    Rejected,
    Link(TerminalLink),
}

fn terminal_hyperlink_at<T>(term: &Term<T>, point: Point) -> Osc8Lookup {
    let Some(hyperlink) = term.grid()[point].hyperlink() else {
        return Osc8Lookup::None;
    };
    let is_separator = |column| {
        let cell = &term.grid()[point.line][Column(column)];
        matches!(cell.c, '│' | '|') && cell.hyperlink().is_none()
    };
    let first = (0..point.column.0)
        .rfind(|&column| is_separator(column))
        .map_or(0, |column| column + 1);
    let last = (point.column.0 + 1..term.columns())
        .find(|&column| is_separator(column))
        .unwrap_or(term.columns());
    let columns = first..last;
    let matches_link = |point: Point| {
        term.grid()[point].hyperlink().is_some_and(|other| {
            other.uri() == hyperlink.uri()
                && wrapped_hyperlink_ids_match(other.id(), hyperlink.id())
        })
    };
    let row_has_link = |line| {
        columns
            .clone()
            .any(|column| matches_link(Point::new(line, Column(column))))
    };
    // Redraws can assign new anonymous IDs inside a row; table padding is not link text.
    let mut start = point.line;
    while start > term.topmost_line() && row_has_link(start - 1) {
        start -= 1;
    }
    let mut end = point.line;
    while end < term.bottommost_line() && row_has_link(end + 1) {
        end += 1;
    }
    let points = (start.0..=end.0)
        .flat_map(|line| {
            columns
                .clone()
                .map(move |column| Point::new(Line(line), Column(column)))
        })
        .filter(|&point| matches_link(point));
    let target = if strip_file_scheme(hyperlink.uri()).is_some() {
        let Some(target) = file_target_from_uri(hyperlink.uri()) else {
            return Osc8Lookup::Rejected;
        };
        target
    } else {
        LinkTarget::Url(hyperlink.uri().to_owned())
    };
    Osc8Lookup::Link(TerminalLink::from_points(target, points))
}

fn link_rows_joined<T>(term: &Term<T>, line: Line) -> bool {
    if term.grid()[line][Column(term.columns() - 1)]
        .flags
        .contains(Flags::WRAPLINE)
    {
        return true;
    }
    let (Some((_, last)), Some((first, _))) = (
        link_row_content(term, line),
        link_row_content(term, Line(line.0 + 1)),
    ) else {
        return false;
    };
    let next = &term.grid()[Line(line.0 + 1)];
    let prefix: String = (first..(first + 8).min(term.columns()))
        .map(|column| next[Column(column)].c)
        .collect();
    let prefix = prefix.to_ascii_lowercase();
    if prefix.starts_with("http://")
        || prefix.starts_with("https://")
        || prefix.starts_with("file://")
    {
        return false;
    }
    let left = &term.grid()[line][Column(last)];
    let right = &next[Column(first)];
    // TUIs also wrap links with explicit newlines, leaving WRAPLINE unset.
    match (left.hyperlink(), right.hyperlink()) {
        (Some(left), Some(right)) => {
            left.uri() == right.uri() && wrapped_hyperlink_ids_match(left.id(), right.id())
        }
        (None, None) => {
            if !left.flags.intersects(Flags::ALL_UNDERLINES)
                || left.flags & Flags::ALL_UNDERLINES != right.flags & Flags::ALL_UNDERLINES
                || left.fg != right.fg
                || left.bg != right.bg
            {
                return false;
            }
            let fragment_end = (first..term.columns())
                .find(|&column| {
                    let cell = &next[Column(column)];
                    !cell.flags.contains(Flags::WIDE_CHAR_SPACER)
                        && (cell.c.is_whitespace()
                            || (column > first && matches!(cell.c, '/' | '\\')))
                })
                .map_or(term.columns(), |column| {
                    column + usize::from(matches!(next[Column(column)].c, '/' | '\\'))
                });
            // Allow a two-column TUI margin, but do not join text that would have fit.
            fragment_end - first > (term.columns() - last - 1).saturating_sub(2)
        }
        _ => false,
    }
}

fn terminal_link_at<T>(term: &Term<T>, point: Point, cwds: &[&Path]) -> Option<TerminalLink> {
    terminal_link_at_with_hard_breaks(term, point, cwds, true)
}

fn terminal_link_at_with_hard_breaks<T>(
    term: &Term<T>,
    point: Point,
    cwds: &[&Path],
    hard_breaks: bool,
) -> Option<TerminalLink> {
    let mut point = point.grid_clamp(term, Boundary::Grid);
    if term.grid()[point]
        .flags
        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
    {
        point.column = Column(point.column.0.saturating_sub(1));
    }
    match terminal_hyperlink_at(term, point) {
        Osc8Lookup::Link(link) => return Some(link),
        Osc8Lookup::Rejected => return None,
        Osc8Lookup::None => {}
    }

    let soft_start = term.line_search_left(point).line;
    let soft_end = term.line_search_right(point).line;
    let mut start = soft_start;
    let mut end = soft_end;
    if hard_breaks {
        while start > term.topmost_line() && link_rows_joined(term, start - 1) {
            start -= 1;
        }
        while end < term.bottommost_line() && link_rows_joined(term, end) {
            end += 1;
        }
    }
    let mut text = String::new();
    let mut cells = Vec::new();
    for line in start.0..=end.0 {
        let line = Line(line);
        let (first, last) = link_row_content(term, line).unwrap_or((0, term.columns() - 1));
        let hard_break_before = line > start
            && !term.grid()[Line(line.0 - 1)][Column(term.columns() - 1)]
                .flags
                .contains(Flags::WRAPLINE);
        let hard_break_after = line < end
            && !term.grid()[line][Column(term.columns() - 1)]
                .flags
                .contains(Flags::WRAPLINE);
        let first = if hard_break_before { first } else { 0 };
        let last = if hard_break_after {
            last
        } else {
            term.columns() - 1
        };
        for column in first..=last {
            let cell_point = Point::new(line, Column(column));
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
            });
        }
    }

    let hit = cells.iter().position(|cell| cell.point == point)?;
    let hit_cell = &cells[hit];
    let overlaps_hit =
        |range: &std::ops::Range<usize>| range.start < hit_cell.end && range.end > hit_cell.start;
    let (range, target) = match url_regex()
        .find_iter(&text)
        .find(|matched| overlaps_hit(&matched.range()))
    {
        Some(matched) => {
            let uri = matched.as_str();
            let target = if strip_file_scheme(uri).is_some() {
                file_target_from_uri(uri)?
            } else {
                LinkTarget::Url(uri.to_owned())
            };
            (matched.range(), target)
        }
        None if hard_breaks && (start != soft_start || end != soft_end) => {
            return terminal_link_at_with_hard_breaks(term, point, cwds, false);
        }
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
    let first = cells.iter().position(|cell| cell.end > range.start)?;
    let last = cells.iter().rposition(|cell| cell.start < range.end)?;
    Some(TerminalLink::from_points(
        target,
        cells[first..=last].iter().map(|cell| cell.point),
    ))
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
                (?: ~[/\\] | (?:\.\.?[/\\])+ | [A-Za-z]:[/\\] | \\\\ | / )?
                (?: [^\s/:()\[\]{}<>'"`|\\,;=*?!\x23]+ [/\\] )*
                [^\s/:()\[\]{}<>'"`|\\,;=*?!\x23]* [^\s/:()\[\]{}<>'"`|\\,;=*?!\x23.]
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

fn is_windows_drive_path(path: &str) -> bool {
    matches!(
        path.as_bytes(),
        [drive, b':', b'/' | b'\\', ..] if drive.is_ascii_alphabetic()
    )
}

fn has_windows_drive_prefix(path: &str) -> bool {
    matches!(
        path.as_bytes(),
        [drive, b':', ..] if drive.is_ascii_alphabetic()
    )
}

fn is_windows_drive_root(path: &str) -> bool {
    matches!(
        path.as_bytes(),
        [drive, b':'] | [drive, b':', b'/' | b'\\'] if drive.is_ascii_alphabetic()
    )
}

fn is_windows_unc_path(path: &str) -> bool {
    path.starts_with("\\\\")
}

fn strip_tilde_prefix(candidate: &str, windows: bool) -> Option<&str> {
    candidate
        .strip_prefix("~/")
        .or_else(|| windows.then(|| candidate.strip_prefix("~\\")).flatten())
}

fn join_platform_path(base: &Path, rest: &str, windows: bool) -> PathBuf {
    if !windows {
        return base.join(rest);
    }
    let base = base.to_string_lossy();
    let base = base.as_ref();
    if base.is_empty() {
        return PathBuf::from(rest);
    }
    let rest = rest.trim_start_matches(['/', '\\']);
    if base.ends_with(['/', '\\']) {
        PathBuf::from(format!("{base}{rest}"))
    } else {
        PathBuf::from(format!("{base}\\{rest}"))
    }
}

fn candidate_paths(
    candidate: &str,
    cwds: &[&Path],
    home: Option<&Path>,
    windows: bool,
) -> Vec<PathBuf> {
    let raw = if let Some(rest) = strip_tilde_prefix(candidate, windows) {
        let Some(home) = home else {
            return Vec::new();
        };
        vec![join_platform_path(home, rest, windows)]
    } else if (windows && (is_windows_drive_path(candidate) || is_windows_unc_path(candidate)))
        || candidate.starts_with('/')
    {
        vec![PathBuf::from(candidate)]
    } else {
        cwds.iter()
            .map(|cwd| join_platform_path(cwd, candidate, windows))
            .collect()
    };
    raw.into_iter()
        .map(|path| normalize_path_for(&path, windows))
        .collect()
}

fn resolve_file_candidate(candidate: &str, cwds: &[&Path]) -> Option<PathBuf> {
    let home = strip_tilde_prefix(candidate, cfg!(windows))
        .and_then(|_| runner_core::app_paths::home_dir());
    candidate_paths(candidate, cwds, home.as_deref(), cfg!(windows))
        .into_iter()
        .find(|path| std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file()))
}

fn normalize_path_for(path: &Path, windows: bool) -> PathBuf {
    if windows {
        PathBuf::from(normalize_windows_path(&path.to_string_lossy()))
    } else {
        normalize_posix_path(path)
    }
}

fn normalize_posix_path(path: &Path) -> PathBuf {
    #[cfg(not(windows))]
    {
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    if normalized.as_os_str().is_empty()
                        || normalized
                            .components()
                            .next_back()
                            .is_some_and(|last| last == Component::ParentDir)
                    {
                        normalized.push(Component::ParentDir);
                    } else {
                        normalized.pop();
                    }
                }
                other => normalized.push(other),
            }
        }
        normalized
    }
    #[cfg(windows)]
    {
        let raw = path.to_string_lossy();
        let (has_root, rest) = match raw.strip_prefix('/') {
            Some(rest) => (true, rest),
            None => (false, raw.as_ref()),
        };
        PathBuf::from(normalize_lexical_path(
            String::new(),
            has_root,
            rest,
            &['/'],
            '/',
        ))
    }
}

fn normalize_lexical_path(
    mut normalized: String,
    has_root: bool,
    rest: &str,
    separators: &[char],
    separator: char,
) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for part in rest.split(separators) {
        match part {
            "" | "." => {}
            ".." => {
                if segments.last().is_some_and(|last| *last != "..") {
                    segments.pop();
                } else if !has_root {
                    segments.push("..");
                }
            }
            other => segments.push(other),
        }
    }
    if has_root {
        normalized.push(separator);
    }
    let mut buf = [0u8; 4];
    let sep = separator.encode_utf8(&mut buf);
    normalized.push_str(&segments.join(sep));
    normalized
}

fn split_unc_share<'a>(after_unc: &'a str, leader: &str) -> Option<(String, &'a str)> {
    let mut parts = after_unc.splitn(3, ['/', '\\']);
    let server = parts.next().filter(|s| !s.is_empty())?;
    let share = parts.next().filter(|s| !s.is_empty())?;
    let rest = parts.next().unwrap_or("");
    Some((format!("{leader}{server}\\{share}"), rest))
}

fn split_windows_prefix(path: &str) -> (String, bool, &str) {
    if let Some(after_verbatim) = path.strip_prefix("\\\\?\\") {
        if let Some(after_unc) = after_verbatim
            .get(..4)
            .filter(|prefix| prefix.eq_ignore_ascii_case("UNC\\"))
            .map(|_| &after_verbatim[4..])
        {
            if let Some((prefix, rest)) = split_unc_share(after_unc, "\\\\?\\UNC\\") {
                return (prefix, true, rest);
            }
        }
        if is_windows_drive_path(after_verbatim) {
            let prefix = format!("\\\\?\\{}", &after_verbatim[..2]);
            return (prefix, true, &after_verbatim[3..]);
        }
    }
    if let Some(after_unc) = path.strip_prefix("\\\\") {
        if let Some((prefix, rest)) = split_unc_share(after_unc, "\\\\") {
            return (prefix, true, rest);
        }
    }
    if has_windows_drive_prefix(path) {
        let has_root = matches!(path.as_bytes().get(2), Some(b'/' | b'\\'));
        let rest = if has_root { &path[3..] } else { &path[2..] };
        return (path[..2].to_owned(), has_root, rest);
    }
    if let Some(rest) = path.strip_prefix(['/', '\\']) {
        return (String::new(), true, rest);
    }
    (String::new(), false, path)
}

fn normalize_windows_path(path: &str) -> String {
    let (normalized, has_root, rest) = split_windows_prefix(path);
    normalize_lexical_path(normalized, has_root, rest, &['/', '\\'], '\\')
}

fn file_target_from_uri(uri: &str) -> Option<LinkTarget> {
    let local_name = runner_core::local_host::local_hostname();
    file_target_from_uri_with_platform(uri, cfg!(windows), local_name.as_deref())
}

fn file_target_from_uri_with_platform(
    uri: &str,
    windows: bool,
    local_name: Option<&str>,
) -> Option<LinkTarget> {
    let rest = strip_file_scheme(uri)?;
    let (rest, fragment) = rest
        .split_once('#')
        .map_or((rest, None), |(rest, fragment)| (rest, Some(fragment)));
    let (before_query, _) = rest.split_once('?').unwrap_or((rest, ""));
    let slash = before_query.find('/')?;
    let host = String::from_utf8(percent_decode_bytes(&before_query[..slash])).ok()?;
    if !runner_core::local_host::is_local_host_for(&host, local_name) {
        return None;
    }
    let decoded = String::from_utf8(percent_decode_bytes(&before_query[slash..])).ok()?;
    if decoded.contains('\0')
        || decoded.starts_with("//")
        || decoded.starts_with(r"/\\")
        || decoded.starts_with("\\\\")
        || (windows && decoded.starts_with(r"/\"))
    {
        return None;
    }
    let (path, line, column) = split_line_suffix(&decoded);
    let line = line.or_else(|| {
        fragment?
            .strip_prefix('L')?
            .split(|c: char| !c.is_ascii_digit())
            .next()?
            .parse()
            .ok()
    });
    let path = if windows {
        let stripped = strip_drive_slash(path);
        if is_windows_drive_path(stripped) {
            let normalized = normalize_windows_path(stripped);
            if is_windows_drive_root(&normalized) {
                return None;
            }
            normalized
        } else if has_windows_drive_prefix(stripped) || path == "/" || path.is_empty() {
            return None;
        } else {
            path.to_owned()
        }
    } else {
        if path == "/" || path.is_empty() {
            return None;
        }
        path.to_owned()
    };
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
        Regex::new(r#"(?i:https?|file)://[^\s\"'!*(){}|\\\^<>`]*[^\s\"':,.!?{}|\\\^~\[\]`()<>]"#)
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
    palette: Mutex<Option<palette::TerminalPalette>>,
    waker: Arc<dyn Fn(&str) + Send + Sync>,
}

impl TerminalBridge {
    pub fn new(client: DaemonClient, waker: Arc<dyn Fn() + Send + Sync>) -> Result<Arc<Self>> {
        Self::with_session_waker(client, Arc::new(move |_| waker()))
    }

    pub fn with_session_waker(
        client: DaemonClient,
        waker: Arc<dyn Fn(&str) + Send + Sync>,
    ) -> Result<Arc<Self>> {
        let bridge = Arc::new(Self {
            client,
            sessions: Mutex::default(),
            palette: Mutex::default(),
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
        let wake = Arc::clone(&self.waker);
        let id = session_id.to_owned();
        let mirror = TerminalMirror::attach(
            self.client.clone(),
            session_id.to_owned(),
            Arc::new(move || wake(&id)),
        )?;
        mirror.set_palette(self.palette.lock().unwrap().unwrap_or(palette::RUNNER));
        sessions.insert(session_id.to_owned(), Arc::clone(&mirror));
        Ok(mirror)
    }
    pub fn attach_live_sessions(&self) -> Result<()> {
        for id in self.client.session_live_ids()? {
            self.attach(&id)?;
        }
        Ok(())
    }
    pub fn handle_event(&self, event: &ClientEvent) {
        if event.name == "daemon/reconnected" {
            if let Some(palette) = *self.palette.lock().unwrap() {
                self.send_palette(palette);
            }
            let mirrors: Vec<_> = self.sessions.lock().unwrap().values().cloned().collect();
            for mirror in mirrors {
                if let Err(error) = mirror.reattach() {
                    log::debug!("reattach {}: {error}", mirror.session_id);
                }
            }
            if let Err(error) = self.attach_live_sessions() {
                log::error!("reattach live terminals: {error}");
            }
            return;
        }

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
        (self.waker)(id);
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
        if *current == Some(palette) {
            return;
        }
        *current = Some(palette);
        self.send_palette(palette);
        for session in sessions.values() {
            session.set_palette(palette);
        }
    }
    fn send_palette(&self, palette: palette::TerminalPalette) {
        let _ = self.client.terminal_palette(palette_to_wire(palette));
    }
    pub fn live_session_count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }
}

pub fn palette_to_wire(palette: palette::TerminalPalette) -> TerminalPalette {
    let rgb = |color: alacritty_terminal::vte::ansi::Rgb| [color.r, color.g, color.b];
    TerminalPalette {
        background: rgb(palette.background),
        foreground: rgb(palette.foreground),
        cursor: rgb(palette.cursor),
        cursor_accent: rgb(palette.cursor_accent),
        selection: rgb(palette.selection),
        ansi: palette.ansi.map(rgb),
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
    pub fn test_stop_frames(&self) {
        if let Some(worker) = self.frame_worker.lock().unwrap().take() {
            {
                let state = worker.state.lock().unwrap();
                state.stopped.store(true, Ordering::Release);
                (state.cancel)();
            }
            worker.thread.join().unwrap();
        }
    }
    pub fn test_feed(&self, seq: u64, bytes: &[u8]) {
        if self.feed_output(seq, bytes) {
            self.wake_viewers();
        }
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
mod tests;
