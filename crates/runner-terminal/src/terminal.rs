use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
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
use alacritty_terminal::term::{Config, Term, TermMode};
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
        self.write_user_bytes(&crate::mappings::encode_paste(text, bracketed))
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

fn terminal_hyperlink_at<T>(term: &Term<T>, point: Point) -> Option<TerminalLink> {
    let hyperlink = term.grid()[point].hyperlink()?;
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
    Some(TerminalLink::from_points(
        file_target_from_uri(hyperlink.uri())
            .unwrap_or_else(|| LinkTarget::Url(hyperlink.uri().to_owned())),
        points,
    ))
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
    if prefix.starts_with("http://") || prefix.starts_with("https://") {
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
                        && (cell.c.is_whitespace() || (column > first && cell.c == '/'))
                })
                .map_or(term.columns(), |column| {
                    column + usize::from(next[Column(column)].c == '/')
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
    if let Some(link) = terminal_hyperlink_at(term, point) {
        return Some(link);
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
        Some(matched) => (
            matched.range(),
            LinkTarget::Url(matched.as_str().to_owned()),
        ),
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
mod tests {
    use super::*;

    fn link_term(columns: usize, bytes: &[u8]) -> Term<alacritty_terminal::event::VoidListener> {
        let mut term = Term::new(
            Config::default(),
            &TermSize::new(columns, 8),
            alacritty_terminal::event::VoidListener,
        );
        crate::snapshot::TrackedProcessor::new().advance(&mut term, bytes);
        term
    }

    #[test]
    fn terminal_links_follow_fragmented_osc_8_in_table_cells() {
        let uri = "https://www.shopneighbour.com/en-us/collections/all-exclusions-cant-discount/products/our-legacy-mens-third-cut-blue-selvage";
        let rows = [
            vec!["Blue Selvedge at Neighbour", " (", "https://"],
            vec!["www.shopneighbour.com/en", "-us/"],
            vec!["collections/all-exclusions-cant-"],
            vec!["discount/products/our-legacy-mens-"],
            vec!["third-cut-blue-selvage", ")"],
        ];
        let mut bytes = String::new();
        let mut linked_points = Vec::new();
        let mut unlinked_points = Vec::new();
        for (line, fragments) in rows.iter().enumerate() {
            if line > 0 {
                bytes.push_str("\r\n");
            }
            bytes.push_str("  ");
            let mut column = 2;
            for fragment in fragments {
                let linked = !matches!(*fragment, " (" | ")");
                if linked {
                    bytes.push_str(&format!("\x1b]8;;{uri}\x1b\\\x1b[4;34m"));
                }
                bytes.push_str(fragment);
                for offset in 0..fragment.len() {
                    let point = Point::new(Line(line as i32), Column(column + offset));
                    if linked {
                        linked_points.push(point);
                    } else {
                        unlinked_points.push(point);
                    }
                }
                if linked {
                    bytes.push_str("\x1b[0m\x1b]8;;\x1b\\");
                }
                column += fragment.len();
            }
            bytes.push_str(&" ".repeat(38 - column));
            bytes.push_str("│ ");
            bytes.push_str(&format!("\x1b]8;;{uri}\x1b\\other cell\x1b]8;;\x1b\\"));
            unlinked_points.push(Point::new(Line(line as i32), Column(38)));
            unlinked_points.push(Point::new(Line(line as i32), Column(40)));
            unlinked_points.push(Point::new(Line(line as i32), Column(0)));
        }
        let term = link_term(72, bytes.as_bytes());
        let expected = terminal_link_at(&term, linked_points[0], &[]).unwrap();
        assert_eq!(expected.target, LinkTarget::Url(uri.into()));
        assert_eq!(expected.start, linked_points[0]);
        assert_eq!(expected.end, *linked_points.last().unwrap());
        for point in linked_points {
            assert!(expected.contains(point), "missing linked cell {point:?}");
            assert_eq!(terminal_link_at(&term, point, &[]), Some(expected.clone()));
        }
        for point in unlinked_points {
            assert!(
                !expected.contains(point),
                "included unlinked cell {point:?}"
            );
        }
        let restored = link_term(72, &crate::snapshot::serialize(&term, &[]));
        assert_eq!(
            terminal_link_at(&restored, Point::new(Line(1), Column(4)), &[]),
            Some(expected)
        );
    }

    #[test]
    fn terminal_links_keep_unrelated_osc_8_rows_separate() {
        for separator in [
            "",
            "────────────────────",
            "\x1b]8;;https://other.example\x1b\\other\x1b]8;;\x1b\\",
        ] {
            let bytes = format!(
                "\x1b]8;;https://example.com\x1b\\first\x1b]8;;\x1b\\\r\n  \x1b]8;;https://example.com\x1b\\continued\x1b]8;;\x1b\\\r\n{separator}\r\n\x1b]8;;https://example.com\x1b\\separate\x1b]8;;\x1b\\"
            );
            let term = link_term(40, bytes.as_bytes());
            let first = terminal_link_at(&term, Point::new(Line(0), Column(1)), &[]).unwrap();
            let separate = terminal_link_at(&term, Point::new(Line(3), Column(1)), &[]).unwrap();
            assert_eq!(first.end, Point::new(Line(1), Column(10)));
            assert_eq!(separate.start, Point::new(Line(3), Column(0)));
            assert!(!first.contains(separate.start));
            assert!(!separate.contains(first.start));
        }
    }

    #[test]
    fn terminal_links_follow_indented_underlined_url_rows() {
        let term = link_term(
            30,
            b"  (\x1b[4;34mhttps://example.com/repo/\x1b[0m\r\n  \x1b[4;34mcommit/abc123\x1b[0m).",
        );
        let first = terminal_link_at(&term, Point::new(Line(0), Column(5)), &[]).unwrap();
        let continuation =
            terminal_link_at(&term, Point::new(Line(1), Column(3)), &[]).expect("URL continuation");
        assert_eq!(first, continuation);
        assert_eq!(
            first.target,
            LinkTarget::Url("https://example.com/repo/commit/abc123".into())
        );
        assert_eq!(first.start, Point::new(Line(0), Column(3)));
        assert_eq!(first.end, Point::new(Line(1), Column(14)));
        assert!(first.contains(Point::new(Line(0), Column(27))));
        assert!(first.contains(Point::new(Line(1), Column(2))));
        assert!(!first.contains(Point::new(Line(0), Column(28))));
        assert!(!first.contains(Point::new(Line(1), Column(1))));
        assert!(!first.contains(Point::new(Line(1), Column(15))));

        let restored = link_term(30, &crate::snapshot::serialize(&term, &[]));
        assert_eq!(
            terminal_link_at(&restored, Point::new(Line(1), Column(3)), &[]),
            Some(first)
        );
    }

    #[test]
    fn terminal_links_follow_osc_8_across_indented_rows() {
        let term = link_term(
            40,
            b"\x1b]8;;https://example.com/target\x1b\\first row\x1b]8;;\x1b\\\r\n  \x1b]8;;https://example.com/target\x1b\\second row\x1b]8;;\x1b\\",
        );
        let first = terminal_link_at(&term, Point::new(Line(0), Column(1)), &[]).unwrap();
        let second = terminal_link_at(&term, Point::new(Line(1), Column(3)), &[]).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.end, Point::new(Line(1), Column(11)));
        assert!(!first.contains(Point::new(Line(1), Column(0))));
        assert!(!first.contains(Point::new(Line(0), Column(9))));
    }

    #[test]
    fn terminal_links_keep_unrelated_hard_lines_separate() {
        for bytes in [
            b"https://example.com/path\r\n  continuation".as_slice(),
            b"\x1b[4;34mhttps://example.com/path\x1b[0m\r\n  continuation",
            b"\x1b[4;34mhttps://example.com/path\r\n  https://other.example/path",
            b"\x1b[4;34mhttps://example.com/path\r\n  \x1b[31mcontinuation",
            b"\x1b[4;34mhttps://example.com/path\r\n\r\n  continuation",
            b"\x1b[4;34mhttps://example.com/path\r\n  www.b.example/two",
            b"\x1b[4;34mhttps://example.com/path\r\n  docs/guide.md",
            b"\x1b[4;34mSee https://example.com/path\r\n  and more text",
        ] {
            let term = link_term(40, bytes);
            let first = terminal_link_at(&term, Point::new(Line(0), Column(4)), &[]).unwrap();
            assert_eq!(
                first.target,
                LinkTarget::Url("https://example.com/path".into())
            );
            assert_eq!(first.end.line, Line(0));
        }
    }

    #[test]
    fn terminal_links_follow_three_rows_and_preserve_soft_wraps() {
        let term = link_term(
            30,
            b"  (\x1b[4;34mhttps://example.com/repo/\x1b[0m\r\n  \x1b[4;34mcommit/abcdef0123456789/\x1b[0m\r\n  \x1b[4;34mabc123\x1b[0m.",
        );
        let first = terminal_link_at(&term, Point::new(Line(0), Column(4)), &[]).unwrap();
        for line in 1..=2 {
            assert_eq!(
                terminal_link_at(&term, Point::new(Line(line), Column(3)), &[]),
                Some(first.clone())
            );
            assert!(!first.contains(Point::new(Line(line), Column(0))));
        }
        assert_eq!(
            first.target,
            LinkTarget::Url("https://example.com/repo/commit/abcdef0123456789/abc123".into())
        );

        let term = link_term(14, b"xxhttps://example.com/path");
        let first = terminal_link_at(&term, Point::new(Line(0), Column(3)), &[]).unwrap();
        assert_eq!(
            terminal_link_at(&term, Point::new(Line(1), Column(3)), &[]),
            Some(first.clone())
        );
        assert_eq!(
            first.target,
            LinkTarget::Url("https://example.com/path".into())
        );
        assert_eq!(first.start, Point::new(Line(0), Column(2)));
        assert_eq!(first.end, Point::new(Line(1), Column(11)));
    }

    #[test]
    fn terminal_links_keep_underlined_file_rows_separate() {
        let cwd = tempfile::tempdir().unwrap();
        std::fs::create_dir(cwd.path().join("src")).unwrap();
        for names in [
            ["a", "b", "c"],
            [
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "ccccccccccccccccccccccccccccc",
            ],
        ] {
            let mut bytes = String::from("\x1b[4;34m");
            for (line, name) in names.iter().enumerate() {
                std::fs::write(cwd.path().join(format!("src/{name}.rs")), "").unwrap();
                if line > 0 {
                    bytes.push_str("\r\n");
                }
                bytes.push_str(&format!("src/{name}.rs"));
            }
            let term = link_term(40, bytes.as_bytes());
            for (line, name) in names.into_iter().enumerate() {
                let link = terminal_link_at(
                    &term,
                    Point::new(Line(line as i32), Column(2)),
                    &[cwd.path()],
                )
                .expect("independent file link");
                assert_eq!(
                    link.target,
                    LinkTarget::File {
                        path: cwd.path().join(format!("src/{name}.rs")),
                        line: None,
                        column: None,
                    }
                );
                assert_eq!(link.start.line, link.end.line);
            }
        }
    }

    #[test]
    fn terminal_links_do_not_join_an_underlined_scrollback_block() {
        let mut bytes = String::from("\x1b[4;34m");
        for line in 0..3000 {
            if line > 0 {
                bytes.push_str("\r\n");
            }
            bytes.push_str(&format!("row https://example.com/{line}"));
        }
        let term = link_term(40, bytes.as_bytes());
        let point = Point::new(term.topmost_line(), Column(5));
        let link = terminal_link_at(&term, point, &[]).unwrap();
        assert_eq!(link.target, LinkTarget::Url("https://example.com/0".into()));
        assert_eq!(link.start.line, point.line);
        assert_eq!(link.end.line, point.line);
    }

    #[test]
    fn terminal_links_keep_distinct_explicit_hyperlink_ids_separate() {
        let term = link_term(
            40,
            b"\x1b]8;id=A;https://example.com\x1b\\first\x1b]8;id=B;https://example.com\x1b\\second\x1b]8;;\x1b\\\r\n\x1b]8;id=C;https://example.com\x1b\\third\x1b]8;;\x1b\\",
        );
        for (point, start, end) in [
            (Point::new(Line(0), Column(1)), 0, 4),
            (Point::new(Line(0), Column(6)), 5, 10),
            (Point::new(Line(1), Column(1)), 0, 4),
        ] {
            let link = terminal_link_at(&term, point, &[]).unwrap();
            assert_eq!(link.start, Point::new(point.line, Column(start)));
            assert_eq!(link.end, Point::new(point.line, Column(end)));
        }
    }

    #[test]
    fn reconnect_cancels_an_idle_subscription_replaced_by_resync() {
        use runner_core::protocol::terminal::{TerminalAttachment, TerminalSubscription};
        use runner_core::protocol::{ClientError, EventSubscription, Request, Response, Transport};
        use std::time::Duration;
        type FrameResult = std::result::Result<TerminalFrame, ClientError>;
        struct Frames {
            rx: mpsc::Receiver<FrameResult>,
            tx: mpsc::Sender<FrameResult>,
            id: u64,
            ready: mpsc::Sender<u64>,
        }
        impl TerminalSubscription for Frames {
            fn recv(&mut self) -> std::result::Result<TerminalFrame, ClientError> {
                self.rx.recv().unwrap()
            }
            fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
                self.ready.send(self.id).unwrap();
                let tx = self.tx.clone();
                Arc::new(move || {
                    let _ = tx.send(Err(ClientError::msg("cancelled")));
                })
            }
        }
        struct Mock {
            senders: Mutex<Vec<mpsc::Sender<FrameResult>>>,
            ready: mpsc::Sender<u64>,
        }
        impl Transport for Mock {
            fn call(&self, _: Request) -> std::result::Result<Response, ClientError> {
                Ok(Response::terminal_metadata(Ok(TerminalMetadata::default())))
            }
            fn subscribe(&self) -> Box<dyn EventSubscription> {
                unreachable!()
            }
            fn attach(&self, _: &str) -> std::result::Result<TerminalAttachment, ClientError> {
                let (tx, rx) = mpsc::channel();
                let mut senders = self.senders.lock().unwrap();
                let id = senders.len() as u64;
                senders.push(tx.clone());
                Ok(TerminalAttachment {
                    subscriber_id: id,
                    snapshot: runner_core::protocol::terminal::TerminalSnapshot {
                        seq: 0,
                        cols: 80,
                        rows: 24,
                        bytes: Vec::new(),
                        unfinished_len: 0,
                        preceding_char: None,
                        controller: Default::default(),
                    },
                    frames: Box::new(Frames {
                        rx,
                        tx,
                        id,
                        ready: self.ready.clone(),
                    }),
                })
            }
        }
        let (ready, installed) = mpsc::channel();
        let host = Arc::new(Mock {
            senders: Mutex::default(),
            ready,
        });
        let mirror = TerminalMirror::attach(
            DaemonClient::new(host.clone()),
            "idle".into(),
            Arc::new(|| {}),
        )
        .unwrap();
        assert_eq!(installed.recv_timeout(Duration::from_secs(2)).unwrap(), 0);
        host.senders.lock().unwrap()[0]
            .send(Ok(TerminalFrame::Resync))
            .unwrap();
        assert_eq!(installed.recv_timeout(Duration::from_secs(2)).unwrap(), 1);
        let next = mirror.clone();
        let (done, result) = mpsc::channel();
        let worker = thread::spawn(move || {
            done.send(next.reattach()).unwrap();
        });
        result
            .recv_timeout(Duration::from_secs(2))
            .expect("reattach must wake an idle Resync subscription")
            .unwrap();
        worker.join().unwrap();
        let frames = mirror.frame_worker.lock().unwrap().take().unwrap();
        {
            let state = frames.state.lock().unwrap();
            state.stopped.store(true, Ordering::Release);
            (state.cancel)();
        }
        frames.thread.join().unwrap();
    }

    #[test]
    fn burst_hold_passes_echo_and_holds_clears_and_volume_until_quiet_or_cap() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut hold = BurstHold::default();
        assert_eq!(hold.feed(b"a", at(0), false), Burst::Pass);
        assert_eq!(hold.deadline(), None);

        assert_eq!(hold.feed(b"\x1b[2J\x1b[3J", at(10), false), Burst::Held);
        assert_eq!(hold.feed(b"history", at(50), false), Burst::Held);
        assert_eq!(hold.deadline(), Some(at(130)));
        assert_eq!(hold.take(), b"\x1b[2J\x1b[3Jhistory");
        assert_eq!(hold.deadline(), None);

        let streaming = vec![b'x'; 16 * 1024];
        assert_eq!(hold.feed(&streaming, at(200), false), Burst::Pass);
        assert_eq!(
            hold.feed(&streaming[..15 * 1024], at(250), false),
            Burst::Pass
        );
        assert_eq!(
            hold.feed(&streaming[..2 * 1024], at(280), false),
            Burst::Held
        );
        assert_eq!(hold.deadline(), Some(at(360)));
        assert_eq!(hold.take().len(), 2 * 1024);
        assert_eq!(hold.feed(&streaming, at(1000), false), Burst::Pass);
        assert_eq!(
            hold.feed(&streaming, at(1150), false),
            Burst::Pass,
            "a new window starts after a quiet gap"
        );

        assert_eq!(hold.feed(b"\x1b[3J", at(2000), false), Burst::Held);
        for ms in (2060..2500).step_by(60) {
            assert_eq!(hold.feed(b"more", at(ms), false), Burst::Held);
        }
        assert_eq!(hold.deadline(), Some(at(2500)));
        let Burst::Release(released) = hold.feed(b"last", at(2500), false) else {
            panic!("a continuous burst is released at the cap");
        };
        assert!(released.starts_with(b"\x1b[3J") && released.ends_with(b"last"));
        assert_eq!(hold.deadline(), None);

        assert!(matches!(
            hold.feed(&vec![b'x'; BURST_MAX_HELD], at(3000), false),
            Burst::Release(bytes) if bytes.len() == BURST_MAX_HELD
        ));
    }

    fn compositions(marker: &[u8]) -> Vec<Vec<&[u8]>> {
        (0..1u32 << (marker.len() - 1))
            .map(|cuts| {
                let mut parts = Vec::new();
                let mut start = 0;
                for end in 1..=marker.len() {
                    if end == marker.len() || cuts & (1 << (end - 1)) != 0 {
                        parts.push(&marker[start..end]);
                        start = end;
                    }
                }
                parts
            })
            .collect()
    }

    #[test]
    fn burst_hold_finds_a_clear_split_anywhere_across_chunks() {
        let now = Instant::now();
        for parts in compositions(CLEAR_SCROLLBACK) {
            let mut hold = BurstHold::default();
            assert_eq!(hold.feed(b"before\x1b[H", now, false), Burst::Pass);
            let (last, leading) = parts.split_last().unwrap();
            for part in leading {
                assert_eq!(hold.feed(part, now, false), Burst::Pass, "{parts:?}");
            }
            let mut completing = last.to_vec();
            completing.extend_from_slice(b"short repaint");
            assert_eq!(hold.feed(&completing, now, false), Burst::Held, "{parts:?}");
            assert_eq!(hold.feed(b"more repaint", now, false), Burst::Held);
            hold.take();
            assert_eq!(
                hold.feed(b"echo", now, false),
                Burst::Pass,
                "a marker already seen is not found again"
            );
        }
    }

    #[test]
    fn burst_hold_yields_to_an_explicit_synchronized_update() {
        let now = Instant::now();
        let volume = vec![b'x'; 2 * BURST_BYTES];
        let mut hold = BurstHold::default();
        assert_eq!(hold.feed(&volume, now, true), Burst::Pass);
        assert_eq!(hold.feed(CLEAR_SCROLLBACK, now, true), Burst::Pass);
        assert_eq!(hold.feed(b"echo", now, false), Burst::Pass);

        for parts in compositions(BEGIN_SYNC) {
            let mut hold = BurstHold::default();
            assert_eq!(hold.feed(CLEAR_SCROLLBACK, now, false), Burst::Held);
            let (last, leading) = parts.split_last().unwrap();
            for part in leading {
                assert_eq!(hold.feed(part, now, false), Burst::Held, "{parts:?}");
            }
            let mut expected = CLEAR_SCROLLBACK.to_vec();
            expected.extend_from_slice(BEGIN_SYNC);
            assert_eq!(
                hold.feed(last, now, false),
                Burst::Release(expected),
                "{parts:?}"
            );
            assert_eq!(hold.deadline(), None);
        }

        let mut hold = BurstHold::default();
        let mut opening = BEGIN_SYNC.to_vec();
        opening.extend_from_slice(CLEAR_SCROLLBACK);
        opening.extend_from_slice(&volume);
        assert_eq!(hold.feed(&opening, now, false), Burst::Pass);
    }

    type ScriptedFrame = std::result::Result<TerminalFrame, ClientError>;

    struct ScriptedFrames(mpsc::Receiver<ScriptedFrame>, mpsc::Sender<ScriptedFrame>);

    impl runner_core::protocol::terminal::TerminalSubscription for ScriptedFrames {
        fn recv(&mut self) -> ScriptedFrame {
            self.0
                .recv()
                .unwrap_or_else(|_| Err(ClientError::msg("closed")))
        }
        fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
            let tx = self.1.clone();
            Arc::new(move || {
                let _ = tx.send(Err(ClientError::msg("cancelled")));
            })
        }
    }

    struct ScriptedHost(Mutex<Option<ScriptedFrames>>);

    impl runner_core::protocol::Transport for ScriptedHost {
        fn call(
            &self,
            _: runner_core::protocol::Request,
        ) -> std::result::Result<runner_core::protocol::Response, ClientError> {
            Ok(runner_core::protocol::Response::terminal_metadata(Ok(
                TerminalMetadata::default(),
            )))
        }
        fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
            unreachable!()
        }
        fn attach(&self, _: &str) -> std::result::Result<TerminalAttachment, ClientError> {
            Ok(TerminalAttachment {
                subscriber_id: 1,
                snapshot: TerminalSnapshot {
                    seq: 0,
                    cols: 80,
                    rows: 24,
                    bytes: Vec::new(),
                    unfinished_len: 0,
                    preceding_char: None,
                    controller: Default::default(),
                },
                frames: Box::new(self.0.lock().unwrap().take().expect("one attachment")),
            })
        }
    }

    fn scripted_mirror(
        remote: bool,
    ) -> (
        Arc<TerminalMirror>,
        mpsc::Sender<ScriptedFrame>,
        Arc<AtomicUsize>,
    ) {
        let (tx, rx) = mpsc::channel();
        let host = Arc::new(ScriptedHost(Mutex::new(Some(ScriptedFrames(
            rx,
            tx.clone(),
        )))));
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&wakes);
        let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        let client = DaemonClient::new(host);
        let mirror = if remote {
            TerminalMirror::attach_remote(client, "burst".into(), waker)
        } else {
            TerminalMirror::attach(client, "burst".into(), waker)
        }
        .unwrap();
        (mirror, tx, wakes)
    }

    fn screen_text(mirror: &TerminalMirror) -> String {
        mirror
            .term
            .lock()
            .grid()
            .display_iter()
            .map(|cell| cell.c)
            .collect()
    }

    fn wait_for(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if done() {
                return true;
            }
            thread::sleep(Duration::from_millis(5));
        }
        done()
    }

    fn send_output(frames: &mpsc::Sender<ScriptedFrame>, seq: u64, bytes: &[u8]) {
        frames
            .send(Ok(TerminalFrame::Output {
                seq,
                bytes: bytes.to_vec(),
            }))
            .unwrap();
    }

    #[test]
    fn remote_mirror_paints_echo_at_once_and_a_full_repaint_in_one_wake() {
        let (mirror, frames, wakes) = scripted_mirror(true);
        let _view = mirror.view();
        send_output(&frames, 1, b"typed");
        assert!(wait_for(Duration::from_secs(2), || mirror
            .output_activity()
            .last_seq
            == 1));
        assert!(screen_text(&mirror).contains("typed"));
        assert!(wait_for(Duration::from_secs(2), || wakes
            .load(Ordering::SeqCst)
            == 1));

        let mut repaint = b"\x1b[H\x1b[2J\x1b[3J".to_vec();
        for line in 0..3000 {
            repaint.extend_from_slice(format!("history line {line}\r\n").as_bytes());
        }
        repaint.extend_from_slice(b"repaint done");
        let chunks = repaint.chunks(8 * 1024).collect::<Vec<_>>();
        assert!(chunks.len() > 4);
        for (index, chunk) in chunks.iter().enumerate() {
            send_output(&frames, index as u64 + 2, chunk);
        }
        assert!(wait_for(Duration::from_secs(2), || screen_text(&mirror)
            .contains("repaint done")));
        assert!(!screen_text(&mirror).contains("typed"));
        thread::sleep(BURST_QUIET * 2);
        assert_eq!(
            wakes.load(Ordering::SeqCst),
            2,
            "every held chunk is painted by a single release"
        );
        assert_eq!(mirror.sync_state(), (None, 0, false));
    }

    fn filler(bytes: usize) -> Vec<u8> {
        let mut filler = Vec::new();
        while filler.len() < bytes {
            filler.extend_from_slice(b"\x1b[1;1H");
            filler.extend_from_slice(&[b'f'; 60]);
        }
        filler
    }

    #[test]
    fn remote_mirror_keeps_explicit_synchronized_updates_with_vte() {
        let (mirror, frames, _) = scripted_mirror(true);
        let parsed = |seq: u64| {
            assert!(wait_for(Duration::from_secs(2), || mirror
                .output_activity()
                .last_seq
                == seq));
            screen_text(&mirror)
        };

        send_output(&frames, 1, b"\x1b[?2026h\x1b[3;1Hprefix-a");
        send_output(&frames, 2, &filler(2 * BURST_BYTES));
        send_output(&frames, 3, b"\x1b[4;1Hafter-a\x1b[?2026l");
        let screen = parsed(3);
        assert!(
            screen.contains("prefix-a") && screen.contains("after-a"),
            "an update that ends after the volume threshold paints at its end marker"
        );

        let mut whole = b"\x1b[?2026h\x1b[3J".to_vec();
        whole.extend_from_slice(&filler(2 * BURST_BYTES));
        whole.extend_from_slice(b"\x1b[5;1Hwhole-b\x1b[?2026l");
        send_output(&frames, 4, &whole);
        assert!(parsed(4).contains("whole-b"));
        assert_eq!(mirror.sync_state().0, None);

        send_output(&frames, 5, b"\x1b[?2026h\x1b[6;1Hpartial-c");
        let mut unfinished = filler(2 * BURST_BYTES);
        unfinished.extend_from_slice(b"\x1b[7;1Htail-c");
        send_output(&frames, 6, &unfinished);
        assert!(wait_for(Duration::from_secs(2), || screen_text(&mirror)
            .contains("partial-c")));
        assert!(
            screen_text(&mirror).contains("tail-c"),
            "vte's own timeout flushes everything that arrived inside the update"
        );
        assert!(wait_for(Duration::from_secs(2), || mirror.sync_state()
            == (None, 0, false)));
    }

    #[test]
    fn remote_mirror_releases_a_continuous_burst_at_the_cap() {
        let (mirror, frames, _) = scripted_mirror(true);
        send_output(&frames, 1, b"\x1b[3Jtick 1");
        let started = Instant::now();
        let mut seq = 1;
        while !screen_text(&mirror).contains("tick") {
            assert!(
                started.elapsed() < BURST_CAP * 3,
                "a burst that never goes quiet is still painted"
            );
            thread::sleep(BURST_QUIET / 2);
            seq += 1;
            send_output(&frames, seq, format!("\r\ntick {seq}").as_bytes());
        }
    }

    #[test]
    fn a_released_burst_advances_the_scroll_mark_and_a_hold_keeps_the_viewport_still() {
        let (mirror, frames, _) = scripted_mirror(true);
        let _view = mirror.view();
        let lines = |range: std::ops::Range<usize>| {
            range
                .map(|line| format!("line {line:05} of the output\r\n"))
                .collect::<String>()
                .into_bytes()
        };
        send_output(&frames, 1, &lines(0..30));
        assert!(wait_for(Duration::from_secs(2), || mirror
            .output_activity()
            .last_seq
            == 1));
        let mark = mirror.scroll_mark();
        let hold = mirror.hold_viewport();
        let burst = lines(30..6030);
        let chunks = burst.chunks(8 * 1024).collect::<Vec<_>>();
        assert!(burst.len() > 2 * BURST_BYTES);
        for (index, chunk) in chunks.iter().enumerate() {
            send_output(&frames, index as u64 + 2, chunk);
        }
        assert!(wait_for(Duration::from_secs(2), || mark
            .moved_since(mirror.scroll_mark())
            == Some(6000)));
        assert_eq!(mirror.scroll_state().display_offset, 6000);
        drop(hold);
    }

    #[test]
    fn local_mirror_parses_a_clear_scrollback_at_once() {
        let (mirror, frames, _) = scripted_mirror(false);
        send_output(&frames, 1, b"\x1b[3Jlocal output");
        assert!(wait_for(Duration::from_secs(2), || mirror
            .output_activity()
            .last_seq
            == 1));
        assert!(screen_text(&mirror).contains("local output"));
        let (mirror, frames, _) = scripted_mirror(false);
        let volume = vec![b'x'; 4 * BURST_BYTES];
        send_output(&frames, 1, &volume);
        assert!(wait_for(Duration::from_secs(2), || mirror
            .output_activity()
            .last_seq
            == 1));
        assert!(screen_text(&mirror).contains('x'));
    }

    #[test]
    fn output_advances_the_scroll_mark_and_a_hold_keeps_the_viewport_still() {
        use runner_core::protocol::terminal::{TerminalAttachment, TerminalSubscription};
        use runner_core::protocol::{EventSubscription, Request, Response, Transport};
        struct Frames(mpsc::Receiver<TerminalFrame>);
        impl TerminalSubscription for Frames {
            fn recv(&mut self) -> std::result::Result<TerminalFrame, ClientError> {
                self.0.recv().map_err(|_| ClientError::msg("closed"))
            }
            fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
                Arc::new(|| {})
            }
        }
        struct Host(Mutex<Option<mpsc::Receiver<TerminalFrame>>>);
        impl Transport for Host {
            fn call(&self, _: Request) -> std::result::Result<Response, ClientError> {
                Ok(Response::terminal_metadata(Ok(TerminalMetadata::default())))
            }
            fn subscribe(&self) -> Box<dyn EventSubscription> {
                unreachable!()
            }
            fn attach(&self, _: &str) -> std::result::Result<TerminalAttachment, ClientError> {
                let rx = self.0.lock().unwrap().take().unwrap();
                Ok(TerminalAttachment {
                    subscriber_id: 0,
                    snapshot: TerminalSnapshot {
                        seq: 0,
                        cols: 20,
                        rows: 5,
                        bytes: Vec::new(),
                        unfinished_len: 0,
                        preceding_char: None,
                        controller: Default::default(),
                    },
                    frames: Box::new(Frames(rx)),
                })
            }
        }
        let (frames, rx) = mpsc::channel();
        let wakes = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&wakes);
        let mirror = TerminalMirror::attach(
            DaemonClient::new(Arc::new(Host(Mutex::new(Some(rx))))),
            "find".into(),
            Arc::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .unwrap();
        let _view = mirror.view();
        let lines = |count: usize| "\r\nline".repeat(count).into_bytes();
        mirror.test_feed(1, &lines(4));
        let start = mirror.scroll_mark();
        mirror.test_feed(2, &lines(6));
        assert_eq!(start.moved_since(mirror.scroll_mark()), Some(6));
        assert_eq!(mirror.scroll_state().display_offset, 0);

        let hold = mirror.hold_viewport();
        mirror.test_feed(3, &lines(3));
        assert_eq!(mirror.scroll_state().display_offset, 3);
        drop(hold);
        mirror.test_feed(4, &lines(2));
        assert_eq!(mirror.scroll_state().display_offset, 5);
        mirror.scroll_to_bottom();
        mirror.test_feed(5, &lines(2));
        assert_eq!(mirror.scroll_state().display_offset, 0);
        assert_eq!(start.moved_since(mirror.scroll_mark()), Some(13));

        mirror.resize_local(20, 5);
        assert_eq!(start.moved_since(mirror.scroll_mark()), Some(13));

        // A tab that wraps a full bottom row scrolls a held viewport too.
        let hold = mirror.hold_viewport();
        let mark = mirror.scroll_mark();
        mirror.test_feed(6, b"\x1b[5;1Habcdefghijklmnopqrst");
        assert_eq!(mark.moved_since(mirror.scroll_mark()), Some(0));
        mirror.test_feed(7, b"\t");
        assert_eq!(mark.moved_since(mirror.scroll_mark()), Some(1));
        assert_eq!(mirror.scroll_state().display_offset, 1);
        drop(hold);
        mirror.scroll_to_bottom();

        // A burst past the scrollback leaves a held viewport at its top, also
        // when it scrolls the ring buffer's whole length (1005 rows here) so
        // the same row allocations come back to the top.
        mirror.configure(20, CursorShape::Block);
        for (seq, burst) in [(8, 40), (9, 1005), (10, 1007)] {
            let mark = mirror.scroll_mark();
            let hold = mirror.hold_viewport();
            mirror.test_feed(seq, &lines(burst));
            let state = mirror.scroll_state();
            assert_eq!(
                (state.history_lines, state.display_offset),
                (20, 20),
                "{burst}"
            );
            assert_eq!(mark.moved_since(mirror.scroll_mark()), Some(burst as i32));
            drop(hold);
            mirror.scroll_to_bottom();
        }

        // A resize from the daemon, with the controller unchanged, still
        // wakes the views so an open find bar recomputes.
        let before = wakes.load(Ordering::SeqCst);
        let mark = mirror.scroll_mark();
        frames
            .send(TerminalFrame::Resized {
                seq: 11,
                cols: 24,
                rows: 5,
                controller: Default::default(),
            })
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while wakes.load(Ordering::SeqCst) == before {
            assert!(
                Instant::now() < deadline,
                "a resize frame did not wake the views"
            );
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(mirror.size(), (24, 5));
        assert_eq!(mark.moved_since(mirror.scroll_mark()), None);
        let resized = mirror.scroll_mark();
        mirror.test_feed(12, b"\x1b[?1049h");
        assert_eq!(resized.moved_since(mirror.scroll_mark()), None);
        drop(frames);
        mirror.test_stop_frames();
    }

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
