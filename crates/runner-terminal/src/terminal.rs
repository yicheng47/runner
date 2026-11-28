use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use runner_core::companion::MAX_MESSAGE_SIZE as REMOTE_QUEUE_MAX_BYTES;
use runner_core::protocol::terminal::{
    SnapshotFormat, TerminalAttachment, TerminalController, TerminalFrame, TerminalMetadata,
    TerminalPalette, TerminalSnapshot, TerminalSnapshotActivity, TerminalSubscription,
    TERMINAL_QUEUE_CAPACITY,
};
use runner_core::protocol::{ClientError, ClientEvent, DaemonClient};

pub mod find;
mod links;
mod model;
pub use links::{FileLinkContext, LinkTarget, TerminalLink};
pub use model::{ClipboardTarget, ModelOptions, TerminalHost, TerminalModel, MAX_CLIPBOARD_BYTES};

#[cfg(any(test, windows))]
use links::strip_drive_slash;
#[cfg(test)]
use links::{
    candidate_paths, file_target_from_editor_uri_with_platform, file_target_from_uri_with_platform,
    scan_plain_file_candidates, stripped_prefix_candidates, terminal_link_at_with_prober,
    MAX_LINK_PATH_PROBES, MAX_WRAP_JOIN_ROWS,
};
use links::{percent_decode, percent_decode_bytes, strip_file_scheme, terminal_link_at};

use crate::palette;
use crate::screen::{ScreenData, ScreenGuard, ScreenView, ViewportSnapshot};
use crate::types::{
    Column, CursorShape, CursorStyle, GridPoint, Line, SelectionKind, Side, TerminalModeFlags,
};
use crate::vt::{
    self, FormatKind, FormatOptions, GridPointTag, RenderState, ScrollViewport,
    SnapshotDecodeOptions, TerminalConfig, TerminalEffect, DEFAULT_CONTINUATION_MAX_BYTES,
    DEFAULT_SCROLLBACK_LINES,
};
use crate::{
    fixtures::FixtureRecorder,
    input_state::{InputEvent, InputObservation, InputTracker},
};

pub const XTERM_WORD_SEPARATORS: &str = " ()[]{}',\"`";

const XTERM_WORD_BOUNDARIES: [u32; 11] = [
    b' ' as u32,
    b'(' as u32,
    b')' as u32,
    b'[' as u32,
    b']' as u32,
    b'{' as u32,
    b'}' as u32,
    b'\'' as u32,
    b',' as u32,
    b'"' as u32,
    b'`' as u32,
];

const MIRROR_SYNC_HOLD_TIMEOUT: Duration = Duration::from_millis(150);

type PersistSessionTitle = Box<dyn FnOnce(&str, bool) -> Option<Box<dyn FnOnce() + Send>> + Send>;

pub fn query_color(
    screen: &ScreenView<'_>,
    index: usize,
    base: &[palette::Rgb; 256],
) -> palette::Rgb {
    query_color_for(screen, index, base, palette::RUNNER)
}

pub fn query_color_for(
    screen: &ScreenView<'_>,
    index: usize,
    base: &[palette::Rgb; 256],
    theme: palette::TerminalPalette,
) -> palette::Rgb {
    screen.query_color(index, base, theme)
}

/// The directory an OSC 7 payload reports: a `file://` URI whose host is
/// this machine and whose path, cut at `?` or `#` and percent-decoded, is
/// absolute UTF-8. Anything else reports nothing.
pub(crate) fn parse_cwd_report(
    payload: &[u8],
    is_local: &impl Fn(&str) -> bool,
) -> Option<PathBuf> {
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

/// A remote mirror receives an agent's full-history reprint, sent when its
/// terminal is resized, as many relay chunks. Painting each chunk scrolls the
/// view through the whole history, so a clear-scrollback or a large volume
/// holds parsing until the stream is briefly quiet, and never past the cap.
/// Typing echo is far below the volume threshold and is parsed at once. An
/// explicit synchronized update stays with Ghostty's DEC 2026 render hold:
/// the chunk that opens one, and every chunk while one is open, releases
/// anything held and is parsed at once.
const BURST_WINDOW: Duration = Duration::from_millis(100);
const BURST_BYTES: usize = 32 * 1024;
const BURST_QUIET: Duration = Duration::from_millis(80);
const BURST_CAP: Duration = Duration::from_millis(500);
const BURST_MAX_HELD: usize = 2 * 1024 * 1024;
const CLEAR_SCROLLBACK: &[u8] = b"\x1b[3J";
pub(crate) const BEGIN_SYNC: &[u8] = b"\x1b[?2026h";

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

/// Minimal stateful VT ground-text scanner used solely to distinguish visible
/// printable output from pure escape/mode/OSC chunks for `last_paint_seq`.
#[derive(Clone, Copy, Default)]
pub(crate) struct VisibleTextScanner {
    state: VisibleScanState,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum VisibleScanState {
    #[default]
    Ground,
    Esc,
    EscIntermediate,
    Csi,
    Osc,
    OscEsc,
    String,
    StringEsc,
}

impl VisibleTextScanner {
    pub(crate) fn reset(&mut self) {
        self.state = VisibleScanState::Ground;
    }

    pub(crate) fn feed(&mut self, bytes: &[u8]) -> bool {
        let mut wrote_visible = false;
        for &b in bytes {
            if matches!(b, 0x18 | 0x1a) {
                self.state = VisibleScanState::Ground;
                continue;
            }
            match self.state {
                VisibleScanState::Ground => match b {
                    0x1b => self.state = VisibleScanState::Esc,
                    0x00..=0x20 | 0x7f => {}
                    _ => wrote_visible = true,
                },
                VisibleScanState::Esc => match b {
                    b'[' => self.state = VisibleScanState::Csi,
                    b']' => self.state = VisibleScanState::Osc,
                    b'P' | b'X' | b'^' | b'_' => self.state = VisibleScanState::String,
                    0x20..=0x2f => self.state = VisibleScanState::EscIntermediate,
                    0x1b => self.state = VisibleScanState::Esc,
                    0x00..=0x1f | 0x7f => {}
                    _ => self.state = VisibleScanState::Ground,
                },
                VisibleScanState::EscIntermediate => match b {
                    0x20..=0x2f => {}
                    0x1b => self.state = VisibleScanState::Esc,
                    0x00..=0x1f | 0x7f => {}
                    _ => self.state = VisibleScanState::Ground,
                },
                VisibleScanState::Csi => match b {
                    0x40..=0x7e => self.state = VisibleScanState::Ground,
                    0x1b => self.state = VisibleScanState::Esc,
                    _ => {}
                },
                VisibleScanState::Osc => match b {
                    0x07 => self.state = VisibleScanState::Ground,
                    0x1b => self.state = VisibleScanState::OscEsc,
                    _ => {}
                },
                VisibleScanState::OscEsc => match b {
                    b'\\' => self.state = VisibleScanState::Ground,
                    b'[' => self.state = VisibleScanState::Csi,
                    b']' => self.state = VisibleScanState::Osc,
                    b'P' | b'X' | b'^' | b'_' => self.state = VisibleScanState::String,
                    0x1b => self.state = VisibleScanState::Esc,
                    _ => self.state = VisibleScanState::Ground,
                },
                VisibleScanState::String => {
                    if b == 0x1b {
                        self.state = VisibleScanState::StringEsc;
                    }
                }
                VisibleScanState::StringEsc => match b {
                    b'\\' => self.state = VisibleScanState::Ground,
                    0x1b => self.state = VisibleScanState::StringEsc,
                    _ => self.state = VisibleScanState::String,
                },
            }
        }
        wrote_visible
    }
}

#[derive(Default)]
pub(crate) struct SequenceState {
    pub(crate) last: u64,
    pub(crate) last_output_seq: u64,
    pub(crate) tui_ready_seq: u64,
    pub(crate) first_paint_seq: u64,
    pub(crate) last_paint_seq: u64,
    pub(crate) last_output_at: Option<Instant>,
}

pub(crate) fn sanitize_title(raw: &str) -> String {
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
pub(crate) struct PaletteState {
    pub(crate) theme: palette::TerminalPalette,
}

impl PaletteState {
    pub(crate) fn new(theme: palette::TerminalPalette) -> Self {
        Self { theme }
    }
}

#[derive(Clone, Copy, Debug)]
struct SelectionState {
    kind: SelectionKind,
    anchor: GridPoint,
    anchor_side: Side,
}

const LOCAL_PUSH_MAILBOX_MAX_BYTES: usize = 4 * 1024 * 1024;
const PUSH_DRAIN_SLICE: usize = 8;
const DECODE_RETRY_MAX_ATTEMPTS: u8 = 5;

#[derive(Clone, Copy)]
struct DecodeRetry {
    deadline: Instant,
    attempts: u8,
}

impl DecodeRetry {
    fn next(prev_attempts: u8) -> Option<Self> {
        if prev_attempts >= DECODE_RETRY_MAX_ATTEMPTS {
            return None;
        }
        let delay_ms = 20u64 << u32::from(prev_attempts.min(3));
        Some(Self {
            deadline: Instant::now() + Duration::from_millis(delay_ms),
            attempts: prev_attempts + 1,
        })
    }
}

fn frame_byte_cost(frame: &TerminalFrame) -> usize {
    match frame {
        TerminalFrame::Output { bytes, .. } => bytes.len(),
        TerminalFrame::Resized { .. } | TerminalFrame::Resync => 64,
    }
}

#[derive(Default)]
struct PushMailboxState {
    generation: u64,
    frames: VecDeque<TerminalFrame>,
    bytes: usize,
    eof: bool,
    closed: bool,
    wake_pending: bool,
}

/// Bounded non-blocking mailbox for push-delivered stream frames. Enforces
/// per-mirror frame and byte caps without blocking the transport producer,
/// collapsing overflow to a single `TerminalFrame::Resync` and coalescing
/// actor wakeups so UI commands and `Shutdown` never queue behind a PTY flood.
struct PushMailbox {
    max_frames: usize,
    max_bytes: usize,
    state: Mutex<PushMailboxState>,
}

impl PushMailbox {
    fn new(max_frames: usize, max_bytes: usize) -> Self {
        Self {
            max_frames,
            max_bytes,
            state: Mutex::default(),
        }
    }

    fn reset(&self, generation: u64) {
        let mut state = self.state.lock().unwrap();
        state.generation = generation;
        state.frames.clear();
        state.bytes = 0;
        state.eof = false;
        state.closed = false;
    }

    fn push(
        &self,
        generation: u64,
        event: Option<TerminalFrame>,
        wake_tx: &Sender<MirrorCommand>,
    ) -> bool {
        let (keep, should_wake) = {
            let mut state = self.state.lock().unwrap();
            if generation != state.generation || state.closed {
                return false;
            }
            let keep = match event {
                None => {
                    state.eof = true;
                    state.closed = true;
                    false
                }
                Some(frame) => {
                    let size = frame_byte_cost(&frame);
                    let overflow = matches!(frame, TerminalFrame::Resync)
                        || state.frames.len() >= self.max_frames
                        || state.bytes.saturating_add(size) > self.max_bytes;
                    if overflow {
                        state.frames.clear();
                        state.bytes = 0;
                        state.frames.push_back(TerminalFrame::Resync);
                        state.closed = true;
                        false
                    } else {
                        state.bytes += size;
                        state.frames.push_back(frame);
                        true
                    }
                }
            };
            let should_wake = !std::mem::replace(&mut state.wake_pending, true);
            (keep, should_wake)
        };
        if should_wake && wake_tx.send(MirrorCommand::WakeStream).is_err() {
            return false;
        }
        keep
    }

    fn pop(&self, current_generation: u64) -> Option<Option<TerminalFrame>> {
        let mut state = self.state.lock().unwrap();
        if state.generation != current_generation {
            state.wake_pending = false;
            return None;
        }
        if let Some(frame) = state.frames.pop_front() {
            state.bytes = state.bytes.saturating_sub(frame_byte_cost(&frame));
            return Some(Some(frame));
        }
        if state.eof {
            state.eof = false;
            state.wake_pending = false;
            return Some(None);
        }
        state.wake_pending = false;
        None
    }
}

enum MirrorCommand {
    Restore {
        snapshot: TerminalSnapshot,
        reply: Sender<Result<()>>,
    },
    AttachSubscription {
        frames: Box<dyn TerminalSubscription>,
        reply: Sender<Option<Box<dyn TerminalSubscription>>>,
    },
    WakeStream,
    StopFrames {
        reply: Sender<()>,
    },
    SyncViewport {
        reply: Sender<()>,
    },
    FeedOutput {
        seq: u64,
        bytes: Vec<u8>,
        reply: Sender<bool>,
    },
    ResizedFrame {
        seq: u64,
        cols: u16,
        rows: u16,
        controller: TerminalController,
        reply: Sender<bool>,
    },
    #[cfg(test)]
    ResizeLocal {
        cols: u16,
        rows: u16,
        reply: Sender<()>,
    },
    SetPalette {
        palette: palette::TerminalPalette,
        reply: Sender<()>,
    },
    Configure {
        scrollback: usize,
        cursor_shape: CursorShape,
        blinking: bool,
        reply: Sender<()>,
    },
    WithScreen {
        reply: Sender<ScreenData>,
    },
    #[cfg(any(test, feature = "test-support"))]
    SnapshotState {
        reply: Sender<crate::snapshot::TerminalSnapshotState>,
    },
    ScrollLocal {
        delta_lines: i32,
        reply: Sender<()>,
    },
    ScrollToBottom {
        reply: Sender<()>,
    },
    ScrollToDisplayOffset {
        display_offset: usize,
        reply: Sender<()>,
    },
    StartSelection {
        kind: SelectionKind,
        point: GridPoint,
        side: Side,
        reply: Sender<()>,
    },
    UpdateSelection {
        point: GridPoint,
        side: Side,
        reply: Sender<bool>,
    },
    ClearSelection {
        reply: Sender<bool>,
    },
    SelectionText {
        reply: Sender<Option<String>>,
    },
    SelectionContains {
        point: GridPoint,
        reply: Sender<bool>,
    },
    CellIsWhitespace {
        point: GridPoint,
        reply: Sender<bool>,
    },
    #[cfg(any(test, feature = "test-support"))]
    TestSetWrapline {
        point: GridPoint,
        wrap: bool,
        reply: Sender<()>,
    },
    #[cfg(any(test, feature = "test-support"))]
    TestSetRowText {
        line: Line,
        text: String,
        reply: Sender<()>,
    },
    Shutdown,
}

struct MirrorShared {
    published: Mutex<Arc<ViewportSnapshot>>,
    viewport_dirty: AtomicBool,
    size: Mutex<(u16, u16)>,
    scroll_mark: Mutex<find::ScrollMark>,
    activity: Mutex<TerminalOutputActivity>,
    sync_snapshot: Mutex<(Option<Instant>, usize, bool)>,
    selection_kind: Mutex<Option<SelectionKind>>,
    viewport_holds: Arc<AtomicUsize>,
    viewers: Arc<AtomicUsize>,
    waker: Arc<dyn Fn() + Send + Sync>,
    client: DaemonClient,
    session_id: String,
    subscriber_id: AtomicU64,
    metadata: Mutex<TerminalMetadata>,
    control: Mutex<TerminalControl>,
    composition_clear_pending: Mutex<bool>,
    composition_clear_retry: AtomicBool,
    #[cfg(any(test, feature = "test-support"))]
    actor_thread: Mutex<Option<thread::ThreadId>>,
    #[cfg(test)]
    render_state_updates: AtomicUsize,
}

impl MirrorShared {
    fn wake_viewers(&self) {
        if self.viewers.load(Ordering::Acquire) > 0 {
            (self.waker)();
        }
    }

    fn refresh_metadata(&self) {
        if let Ok(metadata) = self.client.terminal_metadata(&self.session_id) {
            *self.metadata.lock().unwrap() = metadata;
        }
    }

    fn control(&self) -> TerminalControl {
        self.control.lock().unwrap().clone()
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
            let Some(shared) = weak.upgrade() else {
                return;
            };
            let mut pending = shared.composition_clear_pending.lock().unwrap();
            if *pending && shared.control().desktop() {
                *pending = shared
                    .client
                    .terminal_observe_input(
                        &shared.session_id,
                        InputEvent::Composing { composing: false },
                    )
                    .is_err();
            }
            if !*pending || !shared.control().desktop() {
                shared
                    .composition_clear_retry
                    .store(false, Ordering::Release);
                return;
            }
        });
    }
}

struct MirrorActor {
    term: vt::Terminal,
    render_state: RenderState,
    shared: Arc<MirrorShared>,
    self_tx: Sender<MirrorCommand>,
    mailbox: Arc<PushMailbox>,
    active_subscription: Option<Box<dyn TerminalSubscription>>,
    stream_generation: u64,
    has_synced_view: bool,
    awaiting_resync: bool,
    decode_retry: Option<DecodeRetry>,
    sequence: SequenceState,
    burst: BurstHold,
    coalesce_bursts: bool,
    palette: PaletteState,
    scrollback: usize,
    cursor_shape: CursorShape,
    cursor_blinking: bool,
    default_cursor_hidden: bool,
    render_hold_deadline: Option<Instant>,
    held_sync_bytes: usize,
    sync_wrote_visible: bool,
    frozen_snapshot: Option<Arc<ViewportSnapshot>>,
    selection: Option<SelectionState>,
    visible_scanner: VisibleTextScanner,
    last_visible_text_hash: Option<u64>,
    scroll_tracker: find::ScreenScrollTracker,
}

fn mirror_vt_config(cols: u16, rows: u16, scrollback: usize) -> TerminalConfig {
    TerminalConfig {
        cols: cols.max(1),
        rows: rows.max(1),
        scrollback_lines: scrollback,
        continuation_max_bytes: DEFAULT_CONTINUATION_MAX_BYTES,
        kitty_image_storage_limit: 0,
        enable_host_callbacks: false,
        enable_render_hold: true,
        ..TerminalConfig::default()
    }
}

fn new_mirror_terminal(
    cols: u16,
    rows: u16,
    scrollback: usize,
    palette: palette::TerminalPalette,
    cursor_shape: CursorShape,
    cursor_blinking: bool,
) -> Result<vt::Terminal> {
    let config = mirror_vt_config(cols, rows, scrollback);
    let mut term = vt::Terminal::new(&config)?;
    let _ = term.set_theme_defaults(palette);
    let _ = term.set_default_cursor_style(cursor_shape, cursor_blinking);
    Ok(term)
}

impl MirrorActor {
    fn update_render_state(&mut self) {
        let _ = self.render_state.update(&self.term);
        #[cfg(test)]
        self.shared
            .render_state_updates
            .fetch_add(1, Ordering::Relaxed);
    }

    fn next_deadline(&self) -> Option<Instant> {
        [
            self.burst.deadline(),
            self.render_hold_deadline,
            self.decode_retry.map(|retry| retry.deadline),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    fn service_deadlines(&mut self) {
        let now = Instant::now();
        let held_due = self.burst.deadline().is_some_and(|d| d <= now)
            || self.render_hold_deadline.is_some_and(|d| d <= now);
        if held_due {
            self.flush_held_output();
        }
        if self.decode_retry.is_some_and(|retry| retry.deadline <= now) {
            self.attempt_resync();
        }
    }

    fn run(&mut self, rx: mpsc::Receiver<MirrorCommand>) {
        #[cfg(any(test, feature = "test-support"))]
        {
            *self.shared.actor_thread.lock().unwrap() = Some(thread::current().id());
        }
        loop {
            if let Some(deadline) = self.next_deadline() {
                if deadline <= Instant::now() {
                    self.service_deadlines();
                    continue;
                }
            }
            let mut handled_cmd = false;
            loop {
                match rx.try_recv() {
                    Ok(MirrorCommand::WakeStream) => continue,
                    Ok(cmd) => {
                        if !self.handle(cmd) {
                            self.active_subscription = None;
                            return;
                        }
                        handled_cmd = true;
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.active_subscription = None;
                        return;
                    }
                }
            }
            if handled_cmd {
                continue;
            }
            if self.drain_mailbox_slice() {
                continue;
            }
            let cmd = match self.next_deadline() {
                Some(deadline) => {
                    let now = Instant::now();
                    if deadline <= now {
                        self.service_deadlines();
                        continue;
                    }
                    match rx.recv_timeout(deadline - now) {
                        Ok(cmd) => cmd,
                        Err(RecvTimeoutError::Timeout) => {
                            self.service_deadlines();
                            continue;
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                None => match rx.recv() {
                    Ok(cmd) => cmd,
                    Err(_) => break,
                },
            };
            if !matches!(cmd, MirrorCommand::WakeStream) && !self.handle(cmd) {
                break;
            }
        }
        self.active_subscription = None;
    }

    fn sync_shared_metadata(&self) {
        *self.shared.activity.lock().unwrap() = TerminalOutputActivity {
            last_seq: self.sequence.last,
            last_output_seq: self.sequence.last_output_seq,
            tui_ready_seq: self.sequence.tui_ready_seq,
            first_paint_seq: self.sequence.first_paint_seq,
            last_paint_seq: self.sequence.last_paint_seq,
            last_output_at: self.sequence.last_output_at,
        };
        let scheduled = self.render_hold_deadline.is_some() || self.burst.deadline().is_some();
        *self.shared.sync_snapshot.lock().unwrap() =
            (self.render_hold_deadline, self.held_sync_bytes, scheduled);
        *self.shared.scroll_mark.lock().unwrap() = self.scroll_tracker.mark();
        *self.shared.selection_kind.lock().unwrap() = self.selection.map(|s| s.kind);
        *self.shared.size.lock().unwrap() = (self.term.cols(), self.term.rows());
    }

    fn publish_live_viewport(&mut self, force: bool) {
        if self.frozen_snapshot.is_some() {
            self.sync_shared_metadata();
            return;
        }
        if !force && self.shared.viewers.load(Ordering::Acquire) == 0 {
            self.shared.viewport_dirty.store(true, Ordering::Release);
            self.sync_shared_metadata();
            return;
        }
        self.update_render_state();
        let data = ScreenData::from_render_state(
            &self.term,
            &mut self.render_state,
            self.default_cursor_hidden,
        );
        *self.shared.published.lock().unwrap() = Arc::new(ViewportSnapshot(data));
        self.shared.viewport_dirty.store(false, Ordering::Release);
        self.sync_shared_metadata();
    }

    fn full_screen_data(&mut self) -> ScreenData {
        if let Some(frozen) = &self.frozen_snapshot {
            return frozen.view().data().clone();
        }
        self.update_render_state();
        let cursor = self.render_state.cursor().ok();
        ScreenData::from_terminal_full(&self.term, cursor, self.default_cursor_hidden)
    }

    fn apply_output_chunk(&mut self, seq: u64, bytes: &[u8], now: Instant) {
        let previous_mode = TerminalModeFlags::from_vt(&self.term);
        let was_tui_ready =
            term_indicates_tui_ready_vt(&self.term, self.render_hold_deadline.is_some());
        let was_held = self.render_hold_deadline.is_some();
        let wrote_printable = self.visible_scanner.feed(bytes);
        let has_viewers = self.shared.viewers.load(Ordering::Acquire) > 0;
        let before_hash = if wrote_printable {
            None
        } else {
            Some(
                self.last_visible_text_hash
                    .unwrap_or_else(|| self.term.active_visible_text_state().1),
            )
        };

        let mut effects = Vec::new();
        let mut pre_hold_screen: Option<ScreenData> = None;
        let render_state = &mut self.render_state;
        let default_cursor_hidden = self.default_cursor_hidden;
        let moved = self
            .scroll_tracker
            .write_and_track(&mut self.term, bytes, |term, slice| {
                let captured =
                    term.vt_write_capturing_hold(slice, render_state, default_cursor_hidden);
                let slice_effects = term.take_effects();
                if slice_effects
                    .iter()
                    .any(|e| matches!(e, TerminalEffect::RenderHold(false)))
                {
                    pre_hold_screen = None;
                }
                if let Some(screen) = captured {
                    pre_hold_screen = Some(screen);
                }
                effects.extend(slice_effects);
            });
        self.term.clamp_kitty_keyboard_flags();

        if let Some(lines) = moved.filter(|lines| *lines > 0) {
            if self.term.display_offset_from_bottom() == 0
                && self.shared.viewport_holds.load(Ordering::Acquire) > 0
            {
                let delta = -(isize::try_from(lines).unwrap_or(isize::MAX));
                let _ = self.term.scroll_viewport(ScrollViewport::Delta(delta));
            }
        }

        let mut held_now = was_held;
        for effect in effects {
            if let TerminalEffect::RenderHold(held) = effect {
                held_now = held;
                if held {
                    self.render_hold_deadline = Some(now + MIRROR_SYNC_HOLD_TIMEOUT);
                } else {
                    self.render_hold_deadline = None;
                    self.held_sync_bytes = 0;
                    self.frozen_snapshot = None;
                }
            }
        }
        if held_now {
            if was_held && bytes.windows(BEGIN_SYNC.len()).any(|w| w == BEGIN_SYNC) {
                self.render_hold_deadline = Some(now + MIRROR_SYNC_HOLD_TIMEOUT);
            }
            if let Some(captured) = pre_hold_screen {
                #[cfg(test)]
                self.shared
                    .render_state_updates
                    .fetch_add(1, Ordering::Relaxed);
                let snap = Arc::new(ViewportSnapshot(captured));
                *self.shared.published.lock().unwrap() = Arc::clone(&snap);
                self.shared.viewport_dirty.store(false, Ordering::Release);
                self.frozen_snapshot = Some(snap);
            } else if self.frozen_snapshot.is_none() {
                if self.shared.viewport_dirty.load(Ordering::Acquire) {
                    self.update_render_state();
                    let data = ScreenData::from_render_state(
                        &self.term,
                        &mut self.render_state,
                        self.default_cursor_hidden,
                    );
                    *self.shared.published.lock().unwrap() = Arc::new(ViewportSnapshot(data));
                    self.shared.viewport_dirty.store(false, Ordering::Release);
                }
                let current = Arc::clone(&*self.shared.published.lock().unwrap());
                self.frozen_snapshot = Some(current);
            }
            self.held_sync_bytes = self.held_sync_bytes.saturating_add(bytes.len());
        } else {
            self.render_hold_deadline = None;
            self.held_sync_bytes = 0;
            self.frozen_snapshot = None;
        }

        let mode = TerminalModeFlags::from_vt(&self.term);
        if !previous_mode.intersects(TerminalModeFlags::MOUSE_MODE)
            && mode.intersects(TerminalModeFlags::MOUSE_MODE)
        {
            let _ = self.term.clear_selection();
            self.selection = None;
        }

        if held_now {
            let wrote_visible = if wrote_printable {
                self.last_visible_text_hash = None;
                true
            } else {
                let (_, after_hash) = self.term.active_visible_text_state();
                self.last_visible_text_hash = Some(after_hash);
                Some(after_hash) != before_hash
            };
            self.sync_wrote_visible |= wrote_visible;
            let frozen_has_paint = {
                let published = self.shared.published.lock().unwrap();
                view_has_visible_paint(&published.view())
            };
            finish_parsed_chunk_paint(
                &mut self.sequence,
                &self.term,
                frozen_has_paint,
                true,
                was_tui_ready,
                false,
                seq,
            );
            self.sync_shared_metadata();
        } else if !has_viewers {
            let (has_paint, wrote_visible) = if wrote_printable {
                self.last_visible_text_hash = None;
                (self.term.active_has_visible_text(), true)
            } else {
                let (has_paint, after_hash) = self.term.active_visible_text_state();
                self.last_visible_text_hash = Some(after_hash);
                (has_paint, Some(after_hash) != before_hash)
            };
            let synced = std::mem::take(&mut self.sync_wrote_visible);
            finish_parsed_chunk_paint(
                &mut self.sequence,
                &self.term,
                has_paint,
                false,
                was_tui_ready,
                wrote_visible || synced,
                seq,
            );
            self.shared.viewport_dirty.store(true, Ordering::Release);
            self.sync_shared_metadata();
        } else {
            self.update_render_state();
            let data = ScreenData::from_render_state(
                &self.term,
                &mut self.render_state,
                self.default_cursor_hidden,
            );
            let at_bottom = data.view().display_offset() == 0;
            let (has_paint, wrote_visible) = if wrote_printable {
                self.last_visible_text_hash = None;
                let has_paint = if at_bottom {
                    data.has_visible_text()
                } else {
                    self.term.active_has_visible_text()
                };
                (has_paint, true)
            } else {
                let (has_paint, after_hash) = if at_bottom {
                    data.visible_text_state()
                } else {
                    self.term.active_visible_text_state()
                };
                self.last_visible_text_hash = Some(after_hash);
                (has_paint, Some(after_hash) != before_hash)
            };
            let synced = std::mem::take(&mut self.sync_wrote_visible);
            finish_parsed_chunk_paint(
                &mut self.sequence,
                &self.term,
                has_paint,
                false,
                was_tui_ready,
                wrote_visible || synced,
                seq,
            );
            *self.shared.published.lock().unwrap() = Arc::new(ViewportSnapshot(data));
            self.shared.viewport_dirty.store(false, Ordering::Release);
            self.sync_shared_metadata();
        }
    }

    fn feed_output(&mut self, seq: u64, bytes: &[u8]) -> bool {
        if self.awaiting_resync || seq <= self.sequence.last {
            return false;
        }
        let now = Instant::now();
        self.sequence.last = seq;
        self.sequence.last_output_seq = seq;
        self.sequence.last_output_at = Some(now);
        if chunk_indicates_tui_ready(bytes) {
            self.sequence.tui_ready_seq = self.sequence.tui_ready_seq.max(seq);
        }
        let released;
        let to_parse = if self.coalesce_bursts {
            let synchronized = self.render_hold_deadline.is_some();
            match self.burst.feed(bytes, now, synchronized) {
                Burst::Pass => bytes,
                Burst::Held => {
                    self.sync_shared_metadata();
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
        self.apply_output_chunk(seq, to_parse, now);
        true
    }

    fn release_burst(&mut self, now: Instant) -> bool {
        if self.burst.deadline().is_none() {
            return false;
        }
        let held = self.burst.take();
        let seq = active_output_seq(&self.sequence);
        self.apply_output_chunk(seq, &held, now);
        true
    }

    fn flush_held_output(&mut self) {
        let now = Instant::now();
        let mut flushed = self
            .burst
            .deadline()
            .is_some_and(|deadline| deadline <= now)
            && self.release_burst(now);
        if self
            .render_hold_deadline
            .is_some_and(|deadline| deadline <= now)
        {
            let previous_mode = TerminalModeFlags::from_vt(&self.term);
            let was_tui_ready = term_indicates_tui_ready_vt(&self.term, true);
            let _ = self.term.set_mode(vt::dec_mode(2026), false);
            let _ = self.term.take_effects();
            self.render_hold_deadline = None;
            self.held_sync_bytes = 0;
            self.frozen_snapshot = None;
            let wrote_visible = std::mem::take(&mut self.sync_wrote_visible);
            let seq = active_output_seq(&self.sequence);
            self.last_visible_text_hash = None;
            let has_paint = self.term.active_has_visible_text();
            let mode = TerminalModeFlags::from_vt(&self.term);
            if !previous_mode.intersects(TerminalModeFlags::MOUSE_MODE)
                && mode.intersects(TerminalModeFlags::MOUSE_MODE)
            {
                let _ = self.term.clear_selection();
                self.selection = None;
            }
            finish_parsed_chunk_paint(
                &mut self.sequence,
                &self.term,
                has_paint,
                false,
                was_tui_ready,
                wrote_visible,
                seq,
            );
            self.publish_live_viewport(false);
            flushed = true;
        } else {
            self.sync_shared_metadata();
        }
        if flushed {
            self.shared.wake_viewers();
        }
    }

    fn restore(&mut self, snapshot: &TerminalSnapshot) -> Result<()> {
        if snapshot.unavailable {
            // Host could not serialize mid-sequence; retain any prior good view
            // without advancing `sequence.last` and wait for `Resync` on ground.
            self.awaiting_resync = true;
            self.sync_shared_metadata();
            return Ok(());
        }

        let cols = snapshot.cols.max(1);
        let rows = snapshot.rows.max(1);
        let split = snapshot.bytes.len().saturating_sub(snapshot.unfinished_len);
        let unfinished = &snapshot.bytes[split..];

        let config = mirror_vt_config(cols, rows, self.scrollback);
        let decode_opts = SnapshotDecodeOptions {
            max_continuation_bytes: DEFAULT_CONTINUATION_MAX_BYTES,
            retain_continuation: true,
            compress_history: true,
            kitty_image_storage_limit: 0,
        };
        let mut restored_term = match snapshot.format {
            SnapshotFormat::GhosttyBinary => {
                if snapshot.bytes.is_empty() {
                    if self.has_synced_view {
                        self.awaiting_resync = true;
                    }
                    anyhow::bail!("empty ghostty binary snapshot");
                }
                match vt::Terminal::decode_snapshot(&snapshot.bytes, decode_opts, &config) {
                    Ok(term) => term,
                    Err(err) => {
                        if self.has_synced_view {
                            self.awaiting_resync = true;
                        }
                        anyhow::bail!("decode ghostty binary snapshot: {err}");
                    }
                }
            }
            SnapshotFormat::Vt => {
                let mut term = new_mirror_terminal(
                    cols,
                    rows,
                    self.scrollback,
                    self.palette.theme,
                    self.cursor_shape,
                    self.cursor_blinking,
                )?;
                term.vt_write(&snapshot.bytes[..split]);
                let _ = term.take_effects();
                if let Some(ch) = snapshot.preceding_char {
                    if term.previous_char() != Some(ch) {
                        if let Ok(mut blob) = term.encode_snapshot() {
                            if crate::snapshot::patch_snapshot_previous_char(&mut blob, Some(ch)) {
                                if let Ok(reloaded) =
                                    vt::Terminal::decode_snapshot(&blob, decode_opts, &config)
                                {
                                    term = reloaded;
                                }
                            }
                        }
                    }
                }
                if !unfinished.is_empty() {
                    term.vt_write(unfinished);
                    let _ = term.take_effects();
                }
                term
            }
        };
        let _ = restored_term.set_theme_defaults(self.palette.theme);
        let _ = restored_term.set_default_cursor_style(self.cursor_shape, self.cursor_blinking);
        let _ = restored_term.set_kitty_image_storage_limit(0);
        restored_term.clamp_kitty_keyboard_flags();
        let _ = restored_term.take_effects();

        let held = restored_term.mode(vt::dec_mode(2026));
        let had_synced_view = self.has_synced_view;
        if held && had_synced_view && self.shared.viewport_dirty.load(Ordering::Acquire) {
            // Preserve the last committed pre-hold frame from the previous
            // synced terminal before swapping to the mid-hold snapshot.
            self.update_render_state();
            let data = ScreenData::from_render_state(
                &self.term,
                &mut self.render_state,
                self.default_cursor_hidden,
            );
            *self.shared.published.lock().unwrap() = Arc::new(ViewportSnapshot(data));
            self.shared.viewport_dirty.store(false, Ordering::Release);
        }

        let was_tui_ready = self.sequence.last > 0
            && snapshot.seq >= self.sequence.last
            && term_indicates_tui_ready_vt(&self.term, self.render_hold_deadline.is_some());
        let previous_output_seq = if snapshot.seq < self.sequence.last {
            self.sequence = SequenceState::default();
            0
        } else {
            self.sequence.last_output_seq
        };

        self.burst = BurstHold::default();
        self.render_hold_deadline = None;
        self.held_sync_bytes = 0;
        self.sync_wrote_visible = false;
        self.frozen_snapshot = None;
        self.selection = None;
        self.visible_scanner.reset();

        self.term = restored_term;
        self.render_state = RenderState::new().expect("mirror render state");
        self.has_synced_view = true;
        self.awaiting_resync = false;
        self.decode_retry = None;
        self.last_visible_text_hash = None;
        let has_visible_text = snapshot.activity.is_none() && self.term.active_has_visible_text();

        if held {
            self.render_hold_deadline = Some(Instant::now() + MIRROR_SYNC_HOLD_TIMEOUT);
            let frozen = Arc::clone(&*self.shared.published.lock().unwrap());
            self.frozen_snapshot = Some(frozen);
        }

        self.sequence.last = snapshot.seq;
        match snapshot.activity {
            Some(provenance) => {
                self.sequence.last_output_seq =
                    self.sequence.last_output_seq.max(provenance.output_seq);
                if provenance.output_seq > 0
                    && (provenance.output_seq > previous_output_seq
                        || self.sequence.last_output_at.is_none())
                {
                    self.sequence.last_output_at = Some(Instant::now());
                }
                self.sequence.tui_ready_seq =
                    self.sequence.tui_ready_seq.max(provenance.tui_ready_seq);
                self.sequence.first_paint_seq =
                    match (self.sequence.first_paint_seq, provenance.first_paint_seq) {
                        (0, next) | (next, 0) => next,
                        (existing, next) => existing.min(next),
                    };
                self.sequence.last_paint_seq =
                    self.sequence.last_paint_seq.max(provenance.paint_seq);
            }
            None => {
                let snapshot_painted = !held && has_visible_text;
                let snapshot_tui_ready = term_indicates_tui_ready_vt(&self.term, held)
                    || chunk_indicates_tui_ready(unfinished);
                let cursor_moved = self.term.cursor_pos() != (0, 0);
                let snapshot_active = !snapshot.bytes.is_empty()
                    && (snapshot_tui_ready
                        || snapshot_painted
                        || cursor_moved
                        || !unfinished.is_empty());
                if snapshot.seq > 0 && snapshot_active {
                    self.sequence.last_output_seq = self.sequence.last_output_seq.max(snapshot.seq);
                    if snapshot.seq > previous_output_seq || self.sequence.last_output_at.is_none()
                    {
                        self.sequence.last_output_at = Some(Instant::now());
                    }
                    update_tui_ready_vt(
                        &mut self.sequence,
                        &self.term,
                        held,
                        was_tui_ready,
                        snapshot.seq,
                    );
                    if (self.sequence.tui_ready_seq == 0 || !was_tui_ready)
                        && chunk_indicates_tui_ready(unfinished)
                    {
                        self.sequence.tui_ready_seq = self.sequence.tui_ready_seq.max(snapshot.seq);
                    }
                }
                if snapshot_painted && self.sequence.first_paint_seq == 0 {
                    self.sequence.first_paint_seq = snapshot.seq;
                }
                if snapshot.seq > 0 && snapshot_painted {
                    self.sequence.last_paint_seq = self.sequence.last_paint_seq.max(snapshot.seq);
                }
            }
        }
        self.scroll_tracker.reset(&self.term);
        self.publish_live_viewport(!had_synced_view);
        Ok(())
    }

    fn install_subscription(
        &mut self,
        mut frames: Box<dyn TerminalSubscription>,
    ) -> Option<Box<dyn TerminalSubscription>> {
        self.stream_generation = self.stream_generation.wrapping_add(1);
        let generation = self.stream_generation;
        self.mailbox.reset(generation);
        let mailbox = Arc::clone(&self.mailbox);
        let tx = self.self_tx.clone();
        if frames.install_sink(Box::new(move |event| mailbox.push(generation, event, &tx))) {
            self.active_subscription = Some(frames);
            None
        } else {
            self.active_subscription = None;
            Some(frames)
        }
    }

    fn attempt_resync(&mut self) {
        let prev_attempts = self.decode_retry.map_or(0, |retry| retry.attempts);
        match self.shared.client.attach(&self.shared.session_id) {
            Ok(attachment) => {
                match self.restore(&attachment.snapshot) {
                    Ok(()) => {
                        self.decode_retry = None;
                    }
                    Err(err) => {
                        log::warn!(
                            "terminal {}: resync restore failed, retaining prior view: {err}",
                            self.shared.session_id
                        );
                        self.decode_retry = DecodeRetry::next(prev_attempts);
                    }
                }
                self.shared
                    .apply_controller(attachment.snapshot.controller.clone());
                self.shared
                    .subscriber_id
                    .store(attachment.subscriber_id, Ordering::Relaxed);
                let _ = self.install_subscription(attachment.frames);
                self.shared.refresh_metadata();
                self.shared.wake_viewers();
            }
            Err(_) => {
                self.decode_retry = None;
                self.active_subscription = None;
            }
        }
    }

    fn drain_mailbox_slice(&mut self) -> bool {
        let mut processed = 0usize;
        let mut changed = false;
        while processed < PUSH_DRAIN_SLICE {
            let Some(event) = self.mailbox.pop(self.stream_generation) else {
                break;
            };
            processed += 1;
            let Some(frame) = event else {
                self.active_subscription = None;
                break;
            };
            match frame {
                TerminalFrame::Output { seq, bytes } => {
                    if self.feed_output(seq, &bytes) {
                        changed = true;
                    }
                }
                TerminalFrame::Resized {
                    seq,
                    cols,
                    rows,
                    controller,
                } => {
                    if self.apply_resized_frame(seq, cols, rows, controller) {
                        changed = true;
                    }
                }
                TerminalFrame::Resync => {
                    self.attempt_resync();
                    changed = false;
                    break;
                }
            }
            if self.next_deadline().is_some_and(|d| d <= Instant::now()) {
                break;
            }
        }
        if changed {
            self.shared.refresh_metadata();
            self.shared.wake_viewers();
        }
        processed > 0
    }

    fn apply_resized_frame(
        &mut self,
        seq: u64,
        cols: u16,
        rows: u16,
        controller: TerminalController,
    ) -> bool {
        if seq <= self.sequence.last {
            return false;
        }
        if self.awaiting_resync {
            if self.shared.apply_controller(controller) {
                (self.shared.waker)();
            }
            return false;
        }
        self.sequence.last = seq;
        let now = Instant::now();
        self.release_burst(now);
        self.resize_local(cols, rows);
        if self.shared.apply_controller(controller) {
            (self.shared.waker)();
        }
        true
    }

    fn resize_local(&mut self, cols: u16, rows: u16) {
        let old = (self.term.cols(), self.term.rows());
        let rows_changed = old.1 != rows;
        let was_held = self.render_hold_deadline.is_some();
        let _ = self.term.resize(cols, rows, 0, 0);
        // `ghostty_terminal_resize` emits a synthetic RenderHold(false); keep any
        // in-progress DEC 2026 hold alive until its end marker or deadline.
        let _ = self.term.take_effects();
        if was_held {
            let _ = self.term.set_mode(vt::dec_mode(2026), true);
            let _ = self.term.take_effects();
        }
        if old != (cols, rows) {
            self.scroll_tracker.reset(&self.term);
        }
        if rows_changed {
            let _ = self.term.clear_selection();
            self.selection = None;
        }
        self.last_visible_text_hash = None;
        self.publish_live_viewport(false);
    }

    fn clamp_grid_point(&self, point: GridPoint) -> GridPoint {
        let history = self.term.scrollback_rows() as i32;
        let rows = i32::from(self.term.rows()).max(1);
        let cols = usize::from(self.term.cols()).max(1);
        let top = Line(-history);
        let bottom = Line(rows - 1);
        if point.line < top {
            GridPoint::new(top, Column(0))
        } else if point.line > bottom {
            GridPoint::new(bottom, Column(cols - 1))
        } else {
            GridPoint::new(point.line, Column(point.column.0.min(cols - 1)))
        }
    }

    fn grid_to_screen(&self, point: GridPoint) -> (u16, u32) {
        let clamped = self.clamp_grid_point(point);
        let history = self.term.scrollback_rows() as i64;
        let total = self.term.total_rows().max(1) as i64;
        let y = (i64::from(clamped.line.0) + history).clamp(0, total - 1) as u32;
        (clamped.column.0 as u16, y)
    }

    fn apply_selection_range(
        &mut self,
        state: SelectionState,
        target: GridPoint,
        target_side: Side,
    ) {
        let cols = usize::from(self.term.cols()).max(1);
        let top = Line(-(self.term.scrollback_rows() as i32));
        let bottom = Line(i32::from(self.term.rows()).max(1) - 1);
        let anchor = self.clamp_grid_point(state.anchor);
        let target = self.clamp_grid_point(target);

        match state.kind {
            SelectionKind::Simple => {
                let side_ord = |s: Side| u8::from(s == Side::Right);
                let ((a, a_side), (b, b_side)) =
                    if (anchor, side_ord(state.anchor_side)) <= (target, side_ord(target_side)) {
                        ((anchor, state.anchor_side), (target, target_side))
                    } else {
                        ((target, target_side), (anchor, state.anchor_side))
                    };
                if a == b && a_side == b_side {
                    let _ = self.term.clear_selection();
                    return;
                }
                let start = if a_side == Side::Right {
                    if a.column.0 + 1 < cols {
                        GridPoint::new(a.line, Column(a.column.0 + 1))
                    } else if a.line < bottom {
                        GridPoint::new(a.line + 1, Column(0))
                    } else {
                        let _ = self.term.clear_selection();
                        return;
                    }
                } else {
                    a
                };
                let end = if b_side == Side::Left {
                    if b.column.0 > 0 {
                        GridPoint::new(b.line, Column(b.column.0 - 1))
                    } else if b.line > top {
                        GridPoint::new(b.line - 1, Column(cols - 1))
                    } else {
                        let _ = self.term.clear_selection();
                        return;
                    }
                } else {
                    b
                };
                if start > end {
                    let _ = self.term.clear_selection();
                    return;
                }
                let s = self.grid_to_screen(start);
                let e = self.grid_to_screen(end);
                let _ = self
                    .term
                    .set_selection_points(GridPointTag::Screen, s, e, false);
            }
            SelectionKind::Block => {
                let (top_line, bottom_line) = if anchor.line <= target.line {
                    (anchor.line, target.line)
                } else {
                    (target.line, anchor.line)
                };
                let side_ord = |s: Side| u8::from(s == Side::Right);
                let ((c1, s1), (c2, s2)) = if (anchor.column, side_ord(state.anchor_side))
                    <= (target.column, side_ord(target_side))
                {
                    (
                        (anchor.column, state.anchor_side),
                        (target.column, target_side),
                    )
                } else {
                    (
                        (target.column, target_side),
                        (anchor.column, state.anchor_side),
                    )
                };
                if c1 == c2 && s1 == s2 {
                    let _ = self.term.clear_selection();
                    return;
                }
                let start_col = if s1 == Side::Right { c1.0 + 1 } else { c1.0 };
                let end_col = if s2 == Side::Left {
                    match c2.0.checked_sub(1) {
                        Some(col) => col,
                        None => {
                            let _ = self.term.clear_selection();
                            return;
                        }
                    }
                } else {
                    c2.0
                };
                if start_col > end_col || start_col >= cols {
                    let _ = self.term.clear_selection();
                    return;
                }
                let s = self.grid_to_screen(GridPoint::new(top_line, Column(start_col)));
                let e =
                    self.grid_to_screen(GridPoint::new(bottom_line, Column(end_col.min(cols - 1))));
                let _ = self
                    .term
                    .set_selection_points(GridPointTag::Screen, s, e, true);
            }
            SelectionKind::Semantic => {
                let (ax, ay) = self.grid_to_screen(anchor);
                let (tx, ty) = self.grid_to_screen(target);
                let span_a = self
                    .term
                    .grid_ref(GridPointTag::Screen, ax, ay)
                    .and_then(|gref| {
                        self.term
                            .select_word(gref, Some(&XTERM_WORD_BOUNDARIES))
                            .ok()
                            .flatten()
                    })
                    .and_then(|sel| {
                        self.term
                            .selection_span(&sel, GridPointTag::Screen)
                            .ok()
                            .flatten()
                    });
                let span_b = self
                    .term
                    .grid_ref(GridPointTag::Screen, tx, ty)
                    .and_then(|gref| {
                        self.term
                            .select_word(gref, Some(&XTERM_WORD_BOUNDARIES))
                            .ok()
                            .flatten()
                    })
                    .and_then(|sel| {
                        self.term
                            .selection_span(&sel, GridPointTag::Screen)
                            .ok()
                            .flatten()
                    });
                match (span_a, span_b) {
                    (Some(a), Some(b)) => {
                        let ord = |(x, y): (u16, u32)| (y, x);
                        let start = if ord(a.start) <= ord(b.start) {
                            a.start
                        } else {
                            b.start
                        };
                        let end = if ord(a.end) >= ord(b.end) {
                            a.end
                        } else {
                            b.end
                        };
                        let _ =
                            self.term
                                .set_selection_points(GridPointTag::Screen, start, end, false);
                    }
                    (Some(s), None) | (None, Some(s)) => {
                        let _ = self.term.set_selection_points(
                            GridPointTag::Screen,
                            s.start,
                            s.end,
                            false,
                        );
                    }
                    (None, None) => {
                        let _ = self.term.clear_selection();
                    }
                }
            }
            SelectionKind::Lines => {
                let (ax, ay) = self.grid_to_screen(anchor);
                let (tx, ty) = self.grid_to_screen(target);
                let span_a = self
                    .term
                    .grid_ref(GridPointTag::Screen, ax, ay)
                    .and_then(|gref| self.term.select_line(gref, None, false).ok().flatten())
                    .and_then(|sel| {
                        self.term
                            .selection_span(&sel, GridPointTag::Screen)
                            .ok()
                            .flatten()
                    });
                let span_b = self
                    .term
                    .grid_ref(GridPointTag::Screen, tx, ty)
                    .and_then(|gref| self.term.select_line(gref, None, false).ok().flatten())
                    .and_then(|sel| {
                        self.term
                            .selection_span(&sel, GridPointTag::Screen)
                            .ok()
                            .flatten()
                    });
                match (span_a, span_b) {
                    (Some(a), Some(b)) => {
                        let ord = |(x, y): (u16, u32)| (y, x);
                        let start = if ord(a.start) <= ord(b.start) {
                            a.start
                        } else {
                            b.start
                        };
                        let end = if ord(a.end) >= ord(b.end) {
                            a.end
                        } else {
                            b.end
                        };
                        let _ =
                            self.term
                                .set_selection_points(GridPointTag::Screen, start, end, false);
                    }
                    (Some(s), None) | (None, Some(s)) => {
                        let _ = self.term.set_selection_points(
                            GridPointTag::Screen,
                            s.start,
                            s.end,
                            false,
                        );
                    }
                    (None, None) => {
                        let _ = self.term.clear_selection();
                    }
                }
            }
        }
    }

    fn handle(&mut self, cmd: MirrorCommand) -> bool {
        match cmd {
            MirrorCommand::Shutdown => return false,
            MirrorCommand::Restore { snapshot, reply } => {
                let res = self.restore(&snapshot);
                let _ = reply.send(res);
            }
            MirrorCommand::AttachSubscription { frames, reply } => {
                let fallback = self.install_subscription(frames);
                let _ = reply.send(fallback);
            }
            MirrorCommand::WakeStream => {
                self.drain_mailbox_slice();
            }
            MirrorCommand::StopFrames { reply } => {
                self.stream_generation = self.stream_generation.wrapping_add(1);
                self.mailbox.reset(self.stream_generation);
                self.decode_retry = None;
                self.active_subscription = None;
                let _ = reply.send(());
            }
            MirrorCommand::SyncViewport { reply } => {
                self.publish_live_viewport(true);
                let _ = reply.send(());
            }
            MirrorCommand::FeedOutput { seq, bytes, reply } => {
                let parsed = self.feed_output(seq, &bytes);
                let _ = reply.send(parsed);
            }
            MirrorCommand::ResizedFrame {
                seq,
                cols,
                rows,
                controller,
                reply,
            } => {
                let applied = self.apply_resized_frame(seq, cols, rows, controller);
                let _ = reply.send(applied);
            }
            #[cfg(test)]
            MirrorCommand::ResizeLocal { cols, rows, reply } => {
                self.resize_local(cols, rows);
                let _ = reply.send(());
            }
            MirrorCommand::SetPalette { palette, reply } => {
                self.palette = PaletteState::new(palette);
                let _ = self.term.set_theme_defaults(palette);
                self.publish_live_viewport(false);
                let _ = reply.send(());
            }
            MirrorCommand::Configure {
                scrollback,
                cursor_shape,
                blinking,
                reply,
            } => {
                self.scrollback = scrollback;
                self.cursor_shape = cursor_shape;
                self.cursor_blinking = blinking;
                self.default_cursor_hidden = cursor_shape == CursorShape::Hidden;
                let _ = self.term.set_scrollback_lines(scrollback);
                let _ = self.term.set_default_cursor_style(cursor_shape, blinking);
                self.publish_live_viewport(false);
                let _ = reply.send(());
            }
            MirrorCommand::WithScreen { reply } => {
                let data = self.full_screen_data();
                let _ = reply.send(data);
            }
            #[cfg(any(test, feature = "test-support"))]
            MirrorCommand::SnapshotState { reply } => {
                let state = crate::snapshot::inspect_vt(&self.term);
                let _ = reply.send(state);
            }
            MirrorCommand::ScrollLocal { delta_lines, reply } => {
                // Runner's delta_lines is positive-up (history); Ghostty's is negative-up.
                let _ = self
                    .term
                    .scroll_viewport(ScrollViewport::Delta(-(delta_lines as isize)));
                self.publish_live_viewport(false);
                let _ = reply.send(());
            }
            MirrorCommand::ScrollToBottom { reply } => {
                let _ = self.term.scroll_viewport(ScrollViewport::Bottom);
                self.publish_live_viewport(false);
                let _ = reply.send(());
            }
            MirrorCommand::ScrollToDisplayOffset {
                display_offset,
                reply,
            } => {
                let history = self.term.scrollback_rows();
                let target = display_offset.min(history);
                let row = history.saturating_sub(target);
                let _ = self.term.scroll_viewport(ScrollViewport::Row(row));
                self.publish_live_viewport(false);
                let _ = reply.send(());
            }
            MirrorCommand::StartSelection {
                kind,
                point,
                side,
                reply,
            } => {
                let clamped = self.clamp_grid_point(point);
                self.selection = Some(SelectionState {
                    kind,
                    anchor: clamped,
                    anchor_side: side,
                });
                let (x, y) = self.grid_to_screen(clamped);
                match kind {
                    SelectionKind::Semantic => {
                        let _ = self.term.select_and_set_word(
                            GridPointTag::Screen,
                            x,
                            y,
                            Some(&XTERM_WORD_BOUNDARIES),
                        );
                    }
                    SelectionKind::Lines => {
                        let selected = self
                            .term
                            .select_and_set_line(GridPointTag::Screen, x, y, None, false)
                            .unwrap_or(false);
                        if !selected {
                            let last_col = self.term.cols().saturating_sub(1);
                            let _ = self.term.set_selection_points(
                                GridPointTag::Screen,
                                (0, y),
                                (last_col, y),
                                false,
                            );
                        }
                    }
                    SelectionKind::Simple | SelectionKind::Block => {
                        let _ = self.term.clear_selection();
                    }
                }
                self.publish_live_viewport(false);
                let _ = reply.send(());
            }
            MirrorCommand::UpdateSelection { point, side, reply } => {
                let Some(state) = self.selection else {
                    let _ = reply.send(false);
                    return true;
                };
                self.apply_selection_range(state, point, side);
                self.publish_live_viewport(false);
                let _ = reply.send(true);
            }
            MirrorCommand::ClearSelection { reply } => {
                let had = self.selection.take().is_some() || self.term.selection().is_some();
                let _ = self.term.clear_selection();
                if had {
                    self.publish_live_viewport(false);
                }
                let _ = reply.send(had);
            }
            MirrorCommand::SelectionText { reply } => {
                let text = self
                    .term
                    .format_active_selection(FormatOptions {
                        kind: FormatKind::Plain,
                        unwrap: true,
                        trim: true,
                        extras: 0,
                    })
                    .ok()
                    .flatten()
                    .map(|bytes| {
                        let mut text = String::from_utf8_lossy(&bytes).into_owned();
                        if self
                            .selection
                            .is_some_and(|s| s.kind == SelectionKind::Lines)
                            && !text.ends_with('\n')
                        {
                            text.push('\n');
                        }
                        text
                    })
                    .filter(|text| !text.is_empty());
                let _ = reply.send(text);
            }
            MirrorCommand::SelectionContains { point, reply } => {
                let contains = if let Some(sel) = self.term.selection() {
                    let history = self.term.scrollback_rows() as i64;
                    let total = self.term.total_rows() as i64;
                    let cols = usize::from(self.term.cols());
                    let y = i64::from(point.line.0) + history;
                    if y >= 0 && y < total && point.column.0 < cols {
                        self.term
                            .selection_contains(
                                &sel,
                                GridPointTag::Screen,
                                point.column.0 as u16,
                                y as u32,
                            )
                            .unwrap_or(false)
                    } else {
                        false
                    }
                } else {
                    false
                };
                let _ = reply.send(contains);
            }
            MirrorCommand::CellIsWhitespace { point, reply } => {
                let clamped = self.clamp_grid_point(point);
                let (x, y) = self.grid_to_screen(clamped);
                let is_ws = self
                    .term
                    .grid_ref(GridPointTag::Screen, x, y)
                    .and_then(|gref| gref.cell().ok())
                    .is_none_or(|info| {
                        let spacer = matches!(
                            info.wide,
                            vt::CellWide::SpacerTail | vt::CellWide::SpacerHead
                        );
                        let ch = if info.codepoint == 0 {
                            ' '
                        } else {
                            char::from_u32(info.codepoint).unwrap_or(' ')
                        };
                        !spacer && ch.is_whitespace()
                    });
                let _ = reply.send(is_ws);
            }
            #[cfg(any(test, feature = "test-support"))]
            MirrorCommand::TestSetWrapline { point, wrap, reply } => {
                self.test_set_wrapline_impl(point, wrap);
                self.publish_live_viewport(false);
                let _ = reply.send(());
            }
            #[cfg(any(test, feature = "test-support"))]
            MirrorCommand::TestSetRowText { line, text, reply } => {
                self.test_set_row_text_impl(line, &text);
                self.publish_live_viewport(false);
                let _ = reply.send(());
            }
        }
        true
    }

    #[cfg(any(test, feature = "test-support"))]
    fn row_chars(&self, row: u32) -> Vec<char> {
        let cols = usize::from(self.term.cols());
        (0..cols)
            .map(|col| {
                self.term
                    .grid_ref(GridPointTag::Active, col as u16, row)
                    .and_then(|gref| gref.cell().ok())
                    .and_then(|info| {
                        (info.codepoint != 0)
                            .then(|| char::from_u32(info.codepoint))
                            .flatten()
                    })
                    .unwrap_or(' ')
            })
            .collect()
    }

    #[cfg(any(test, feature = "test-support"))]
    fn test_set_wrapline_impl(&mut self, point: GridPoint, wrap: bool) {
        let rows = usize::from(self.term.rows());
        let cols = usize::from(self.term.cols());
        if point.line.0 < 0 || (point.line.0 as usize) >= rows || cols == 0 {
            return;
        }
        let row = point.line.0 as u32;
        if wrap {
            if (row as usize) + 1 >= rows {
                return;
            }
            let last_ch = self.row_chars(row)[cols - 1];
            let next_first = self.row_chars(row + 1)[0];
            let seq = format!(
                "\x1b7\x1b[?7h\x1b[{};{}H{}{}\x1b8",
                row + 1,
                cols,
                last_ch,
                next_first
            );
            self.term.vt_write(seq.as_bytes());
        } else {
            let chars = self.row_chars(row);
            let mut text: String = chars.into_iter().collect();
            let trimmed_len = text.trim_end_matches(' ').len();
            text.truncate(trimmed_len);
            let seq = format!("\x1b7\x1b[{};1H\x1b[2K{}\x1b8", row + 1, text);
            self.term.vt_write(seq.as_bytes());
        }
        let _ = self.term.take_effects();
    }

    #[cfg(any(test, feature = "test-support"))]
    fn test_set_row_text_impl(&mut self, line: Line, text: &str) {
        let rows = usize::from(self.term.rows());
        let cols = usize::from(self.term.cols());
        if line.0 < 0 || (line.0 as usize) >= rows || cols == 0 {
            return;
        }
        let row = line.0 as u32;
        let was_wrapped = self
            .term
            .grid_ref(GridPointTag::Active, 0, row)
            .and_then(|gref| gref.row().ok())
            .is_some_and(|info| info.wrap);
        let written: String = text.chars().take(cols).collect();
        let seq = format!("\x1b7\x1b[{};1H{}\x1b8", row + 1, written);
        self.term.vt_write(seq.as_bytes());
        let _ = self.term.take_effects();
        if was_wrapped && (row as usize) + 1 < rows {
            self.test_set_wrapline_impl(GridPoint::new(line, Column(cols - 1)), true);
        }
    }
}

pub struct TerminalMirror {
    cmd_tx: Sender<MirrorCommand>,
    shared: Arc<MirrorShared>,
    palette: Mutex<PaletteState>,
    actor_thread: Mutex<Option<thread::JoinHandle<()>>>,
    frame_worker: Mutex<Option<FrameWorker>>,
    link_cwd: std::sync::OnceLock<Option<PathBuf>>,
    /// Keys, typed text and pastes sent, so a blinking cursor shows again
    /// before the echo arrives.
    typed: AtomicU64,
}

impl Drop for TerminalMirror {
    fn drop(&mut self) {
        let worker = self.frame_worker.lock().unwrap().take();
        if let Some(worker) = worker.as_ref() {
            let cancel = {
                let state = worker.state.lock().unwrap();
                state.stopped.store(true, Ordering::Release);
                Arc::clone(&state.cancel)
            };
            cancel();
        }
        let _ = self.cmd_tx.send(MirrorCommand::Shutdown);
        if let Some(actor) = self.actor_thread.lock().unwrap().take() {
            if actor.thread().id() != thread::current().id() {
                let _ = actor.join();
            }
        }
        if let Some(worker) = worker {
            if worker.thread.thread().id() != thread::current().id() {
                let _ = worker.thread.join();
            }
        }
    }
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
    pub last_output_seq: u64,
    pub tui_ready_seq: u64,
    pub first_paint_seq: u64,
    pub last_paint_seq: u64,
    pub last_output_at: Option<Instant>,
}

impl TerminalMirror {
    fn start_frames(
        self: &Arc<Self>,
        attachment: runner_core::protocol::terminal::TerminalAttachment,
    ) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        let fallback = if self
            .cmd_tx
            .send(MirrorCommand::AttachSubscription {
                frames: attachment.frames,
                reply: tx,
            })
            .is_ok()
        {
            rx.recv().ok().flatten()
        } else {
            None
        };
        let Some(mut frames) = fallback else {
            return Ok(());
        };
        let weak = Arc::downgrade(self);
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(Mutex::new(FrameWorkerState {
            stopped: stopping.clone(),
            cancel: frames.cancellation(),
        }));
        let stopped = stop.clone();
        let worker = thread::Builder::new()
            .name(format!("native-term-frames-{}", self.shared.session_id))
            .spawn(move || {
                loop {
                    let frame = match frames.recv() {
                        Ok(frame) => frame,
                        Err(_) => return,
                    };
                    if stopping.load(Ordering::Acquire) {
                        return;
                    }
                    let keep = {
                        let Some(mirror) = weak.upgrade() else {
                            return;
                        };
                        match frame {
                            TerminalFrame::Output { seq, bytes } => {
                                if !mirror.feed_output(seq, &bytes) {
                                    continue;
                                }
                                mirror.refresh_metadata();
                                mirror.wake_viewers();
                                true
                            }
                            TerminalFrame::Resized {
                                seq,
                                cols,
                                rows,
                                controller,
                            } => {
                                let (tx, rx) = mpsc::channel();
                                let _ = mirror
                                    .cmd_tx
                                    .send(MirrorCommand::ResizedFrame {
                                        seq,
                                        cols,
                                        rows,
                                        controller,
                                        reply: tx,
                                    })
                                    .is_ok()
                                    && rx.recv().unwrap_or(false);
                                mirror.refresh_metadata();
                                mirror.wake_viewers();
                                true
                            }
                            TerminalFrame::Resync => {
                                let mut attempts = 0u8;
                                let mut recovered = true;
                                loop {
                                    match mirror.shared.client.attach(&mirror.shared.session_id) {
                                        Ok(attachment) => {
                                            let restore_err = {
                                                let mut state = stopped.lock().unwrap();
                                                if state.stopped.load(Ordering::Acquire) {
                                                    return;
                                                }
                                                let err = mirror.restore(&attachment).err();
                                                state.cancel = attachment.frames.cancellation();
                                                frames = attachment.frames;
                                                err
                                            };
                                            let Some(err) = restore_err else {
                                                mirror.refresh_metadata();
                                                mirror.wake_viewers();
                                                break;
                                            };
                                            log::warn!(
                                                "terminal {}: resync restore failed, retaining prior view: {err}",
                                                mirror.shared.session_id
                                            );
                                            let Some(retry) = DecodeRetry::next(attempts) else {
                                                break;
                                            };
                                            attempts = retry.attempts;
                                            while Instant::now() < retry.deadline {
                                                if stopping.load(Ordering::Acquire) {
                                                    return;
                                                }
                                                thread::sleep(Duration::from_millis(5));
                                            }
                                        }
                                        Err(_) => {
                                            recovered = false;
                                            break;
                                        }
                                    }
                                }
                                recovered
                            }
                        }
                    };
                    if !keep || stopping.load(Ordering::Acquire) {
                        return;
                    }
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
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(MirrorCommand::StopFrames { reply: tx })
            .is_ok()
        {
            let _ = rx.recv();
        }
        if let Some(worker) = self.frame_worker.lock().unwrap().take() {
            {
                let state = worker.state.lock().unwrap();
                state.stopped.store(true, Ordering::Release);
                (state.cancel)();
            }
            let _ = worker.thread.join();
        }
        let attachment = self.shared.client.attach(&self.shared.session_id)?;
        self.restore(&attachment)?;
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
        crate::vt::init_sys();
        let attachment = client.attach(&session_id)?;
        let cols = attachment.snapshot.cols.max(1);
        let rows = attachment.snapshot.rows.max(1);
        let viewers = Arc::new(AtomicUsize::new(0));
        let shared = Arc::new(MirrorShared {
            published: Mutex::new(Arc::new(ViewportSnapshot(ScreenData::empty(
                usize::from(cols),
                usize::from(rows),
            )))),
            viewport_dirty: AtomicBool::new(false),
            size: Mutex::new((cols, rows)),
            scroll_mark: Mutex::default(),
            activity: Mutex::default(),
            sync_snapshot: Mutex::new((None, 0, false)),
            selection_kind: Mutex::new(None),
            viewport_holds: Arc::default(),
            viewers,
            waker,
            client,
            session_id: session_id.clone(),
            subscriber_id: AtomicU64::new(attachment.subscriber_id),
            metadata: Mutex::default(),
            control: Mutex::default(),
            composition_clear_pending: Mutex::new(false),
            composition_clear_retry: AtomicBool::new(false),
            #[cfg(any(test, feature = "test-support"))]
            actor_thread: Mutex::new(None),
            #[cfg(test)]
            render_state_updates: AtomicUsize::new(0),
        });

        let mailbox = Arc::new(PushMailbox::new(
            TERMINAL_QUEUE_CAPACITY,
            if coalesce_bursts {
                REMOTE_QUEUE_MAX_BYTES
            } else {
                LOCAL_PUSH_MAILBOX_MAX_BYTES
            },
        ));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let actor_tx = cmd_tx.clone();
        let (init_tx, init_rx) = mpsc::channel::<Result<()>>();
        let actor_shared = Arc::clone(&shared);
        let actor_handle = thread::Builder::new()
            .name(format!("native-term-actor-{session_id}"))
            .spawn(move || {
                let init = (|| -> Result<MirrorActor> {
                    let term = new_mirror_terminal(
                        cols,
                        rows,
                        DEFAULT_SCROLLBACK_LINES,
                        palette::RUNNER,
                        CursorShape::Block,
                        true,
                    )?;
                    let render_state = RenderState::new()?;
                    let scroll_tracker = find::ScreenScrollTracker::new(&term);
                    Ok(MirrorActor {
                        term,
                        render_state,
                        shared: actor_shared,
                        self_tx: actor_tx,
                        mailbox,
                        active_subscription: None,
                        stream_generation: 0,
                        has_synced_view: false,
                        awaiting_resync: false,
                        decode_retry: None,
                        sequence: SequenceState::default(),
                        burst: BurstHold::default(),
                        coalesce_bursts,
                        palette: PaletteState::new(palette::RUNNER),
                        scrollback: DEFAULT_SCROLLBACK_LINES,
                        cursor_shape: CursorShape::Block,
                        cursor_blinking: true,
                        default_cursor_hidden: false,
                        render_hold_deadline: None,
                        held_sync_bytes: 0,
                        sync_wrote_visible: false,
                        frozen_snapshot: None,
                        selection: None,
                        visible_scanner: VisibleTextScanner::default(),
                        last_visible_text_hash: None,
                        scroll_tracker,
                    })
                })();
                let mut actor = match init {
                    Ok(actor) => {
                        let _ = init_tx.send(Ok(()));
                        actor
                    }
                    Err(err) => {
                        let _ = init_tx.send(Err(err));
                        return;
                    }
                };
                actor.run(cmd_rx);
            })
            .context("spawn terminal mirror actor")?;

        init_rx
            .recv()
            .map_err(|_| anyhow::anyhow!("terminal mirror actor exited during init"))??;

        let mirror = Arc::new(Self {
            cmd_tx,
            shared,
            palette: Mutex::new(PaletteState::new(palette::RUNNER)),
            actor_thread: Mutex::new(Some(actor_handle)),
            frame_worker: Mutex::default(),
            link_cwd: std::sync::OnceLock::new(),
            typed: AtomicU64::new(0),
        });
        mirror.restore(&attachment)?;
        mirror.refresh_metadata();
        mirror.start_frames(attachment)?;
        Ok(mirror)
    }

    fn restore(self: &Arc<Self>, attachment: &TerminalAttachment) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        self.cmd_tx
            .send(MirrorCommand::Restore {
                snapshot: attachment.snapshot.clone(),
                reply: tx,
            })
            .map_err(|_| anyhow::anyhow!("terminal mirror actor stopped"))?;
        rx.recv()
            .map_err(|_| anyhow::anyhow!("terminal mirror actor exited during restore"))??;
        self.shared
            .apply_controller(attachment.snapshot.controller.clone());
        self.shared
            .subscriber_id
            .store(attachment.subscriber_id, Ordering::Relaxed);
        Ok(())
    }

    fn sync_viewport(&self) {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(MirrorCommand::SyncViewport { reply: tx })
            .is_ok()
        {
            let _ = rx.recv();
        }
    }

    pub fn refresh_metadata(&self) {
        self.shared.refresh_metadata();
    }

    pub fn session_id(&self) -> &str {
        &self.shared.session_id
    }

    pub fn view(&self) -> TerminalView {
        let prev = self.shared.viewers.fetch_add(1, Ordering::AcqRel);
        if prev == 0 && self.shared.viewport_dirty.load(Ordering::Acquire) {
            self.sync_viewport();
        }
        TerminalView {
            viewers: Arc::clone(&self.shared.viewers),
        }
    }

    pub fn title(&self) -> String {
        self.shared.metadata.lock().unwrap().title.clone()
    }

    pub fn set_palette(&self, palette: palette::TerminalPalette) {
        *self.palette.lock().unwrap() = PaletteState::new(palette);
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(MirrorCommand::SetPalette { palette, reply: tx })
            .is_ok()
        {
            let _ = rx.recv();
        }
        (self.shared.waker)();
    }

    pub fn palette(&self) -> palette::TerminalPalette {
        self.palette.lock().unwrap().theme
    }

    /// `blinking` is the default cursor's; a program's DECSCUSR still sets
    /// its own. It stays app-side: runnerd's model never draws a cursor.
    pub fn configure(&self, scrollback: usize, cursor_shape: CursorShape, blinking: bool) {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(MirrorCommand::Configure {
                scrollback,
                cursor_shape,
                blinking,
                reply: tx,
            })
            .is_ok()
        {
            let _ = rx.recv();
        }
        let shape = match cursor_shape {
            CursorShape::Block => "block",
            CursorShape::Underline => "underline",
            CursorShape::Beam => "beam",
            CursorShape::HollowBlock => "hollow",
            CursorShape::Hidden => "hidden",
        };
        let _ = self
            .shared
            .client
            .terminal_configure(&self.shared.session_id, scrollback, shape);
        (self.shared.waker)();
    }

    /// Latest published immutable viewport snapshot for GPUI painting without
    /// blocking on `vt_write` (takes the short published-slot mutex to clone the `Arc`).
    pub fn viewport_snapshot(&self) -> Arc<ViewportSnapshot> {
        if self.shared.viewport_dirty.load(Ordering::Acquire) {
            self.sync_viewport();
        }
        Arc::clone(&*self.shared.published.lock().unwrap())
    }

    pub fn with_screen<R>(&self, f: impl FnOnce(&ScreenGuard<'_>) -> R) -> R {
        let (tx, rx) = mpsc::channel();
        let data = if self
            .cmd_tx
            .send(MirrorCommand::WithScreen { reply: tx })
            .is_ok()
        {
            rx.recv()
                .unwrap_or_else(|_| self.viewport_snapshot().view().data().clone())
        } else {
            self.viewport_snapshot().view().data().clone()
        };
        #[cfg(any(test, feature = "test-support"))]
        let snap_tx = self.cmd_tx.clone();
        let guard = ScreenGuard::from_data(
            data,
            #[cfg(any(test, feature = "test-support"))]
            Some(Box::new(move || {
                let (tx, rx) = mpsc::channel();
                let _ = snap_tx.send(MirrorCommand::SnapshotState { reply: tx });
                rx.recv().expect("mirror actor returned snapshot_state")
            })),
        );
        f(&guard)
    }

    pub fn display_offset(&self) -> usize {
        self.viewport_snapshot().view().display_offset()
    }

    pub fn cursor_style(&self) -> CursorStyle {
        self.viewport_snapshot().view().cursor_style()
    }

    pub fn visible_lines(&self) -> Vec<String> {
        self.viewport_snapshot().view().visible_lines()
    }

    pub fn output_activity(&self) -> TerminalOutputActivity {
        *self.shared.activity.lock().unwrap()
    }

    /// Returns whether the bytes reached the terminal; a held burst has not.
    fn feed_output(&self, seq: u64, bytes: &[u8]) -> bool {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(MirrorCommand::FeedOutput {
                seq,
                bytes: bytes.to_vec(),
                reply: tx,
            })
            .is_err()
        {
            return false;
        }
        rx.recv().unwrap_or(false)
    }

    fn wake_viewers(&self) {
        self.shared.wake_viewers();
    }

    pub fn scroll_mark(&self) -> find::ScrollMark {
        *self.shared.scroll_mark.lock().unwrap()
    }

    pub fn hold_viewport(&self) -> ViewportHold {
        self.shared.viewport_holds.fetch_add(1, Ordering::AcqRel);
        ViewportHold {
            holds: Arc::clone(&self.shared.viewport_holds),
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
        self.shared.client.input(&self.shared.session_id, bytes)
    }

    pub fn send_key_bytes(
        &self,
        bytes: &[u8],
        kind: crate::mappings::InputKind,
    ) -> std::result::Result<(), ClientError> {
        self.observe_input(&InputEvent::Key { kind });
        self.write_typed_bytes(bytes)
    }

    pub fn send_key(
        &self,
        key: &str,
        ctrl: bool,
        alt: bool,
        shift: bool,
        key_char: Option<&str>,
    ) -> std::result::Result<bool, ClientError> {
        let mode = self.mode();
        let kind = crate::mappings::classify_key(mode, key, ctrl, alt, shift, key_char);
        match crate::mappings::encode_key(key, ctrl, alt, shift, key_char, mode) {
            Some(bytes) => {
                self.send_key_bytes(&bytes, kind)?;
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
        let mut pending = self.shared.composition_clear_pending.lock().unwrap();
        if self.control().desktop() {
            let sent = self
                .shared
                .client
                .terminal_observe_input(
                    &self.shared.session_id,
                    InputEvent::Composing { composing },
                )
                .is_ok();
            *pending = !composing && !sent;
        } else if !composing {
            // Clear the daemon's preedit after handover without sending input while covered.
            *pending = true;
        }
        drop(pending);
        self.shared.retry_composition_clear();
    }

    pub fn paste(&self, text: &str) -> std::result::Result<(), ClientError> {
        let bracketed = self.mode().contains(TerminalModeFlags::BRACKETED_PASTE);
        self.observe_input(&InputEvent::Paste {
            text: text.to_owned(),
        });
        self.write_typed_bytes(&crate::mappings::encode_paste(text, bracketed))
    }

    fn write_typed_bytes(&self, bytes: &[u8]) -> std::result::Result<(), ClientError> {
        self.write_user_bytes(bytes)?;
        if self.control().desktop() {
            self.typed.fetch_add(1, Ordering::Relaxed);
            (self.shared.waker)();
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
            .shared
            .client
            .terminal_observe_input(&self.shared.session_id, input.clone());
    }

    pub fn input_reset_guard(&self) -> u64 {
        self.shared
            .client
            .terminal_input_reset_guard(&self.shared.session_id)
            .unwrap_or_default()
    }

    pub fn reset_input_state(&self, guard: u64) {
        if !self.control().desktop() {
            return;
        }
        let _ = self
            .shared
            .client
            .terminal_reset_input_state(&self.shared.session_id, guard);
    }

    pub fn control(&self) -> TerminalControl {
        self.shared.control()
    }

    pub fn take_back(&self, cols: u16, rows: u16) -> std::result::Result<(), ClientError> {
        self.shared
            .client
            .terminal_take_back(&self.shared.session_id, cols, rows)
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        if !self.control().desktop() {
            return;
        }
        let (cols, rows) = (cols.max(2), rows.max(2));
        if self.size() == (cols, rows) {
            return;
        }
        self.shared.client.resize(
            &self.shared.session_id,
            self.shared.subscriber_id.load(Ordering::Relaxed),
            cols,
            rows,
        );
    }

    #[cfg(test)]
    fn resize_local(&self, cols: u16, rows: u16) {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(MirrorCommand::ResizeLocal {
                cols,
                rows,
                reply: tx,
            })
            .is_ok()
        {
            let _ = rx.recv();
        }
    }

    pub fn size(&self) -> (u16, u16) {
        *self.shared.size.lock().unwrap()
    }

    pub fn scroll(&self, delta_lines: i32, bypass_reporting: bool, column: usize, row: usize) {
        let mode = self.mode();
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
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(MirrorCommand::ScrollLocal {
                delta_lines,
                reply: tx,
            })
            .is_ok()
        {
            let _ = rx.recv();
        }
        (self.shared.waker)();
    }

    pub fn scroll_to_bottom(&self) {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(MirrorCommand::ScrollToBottom { reply: tx })
            .is_ok()
        {
            let _ = rx.recv();
        }
    }

    pub fn mode(&self) -> TerminalModeFlags {
        self.viewport_snapshot().view().mode()
    }

    pub fn start_selection(&self, kind: SelectionKind, point: GridPoint, side: Side) {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(MirrorCommand::StartSelection {
                kind,
                point,
                side,
                reply: tx,
            })
            .is_ok()
        {
            let _ = rx.recv();
        }
        (self.shared.waker)();
    }

    pub fn update_selection(&self, point: GridPoint, side: Side) -> bool {
        let (tx, rx) = mpsc::channel();
        let updated = self
            .cmd_tx
            .send(MirrorCommand::UpdateSelection {
                point,
                side,
                reply: tx,
            })
            .is_ok()
            && rx.recv().unwrap_or(false);
        if updated {
            (self.shared.waker)();
        }
        updated
    }

    pub fn clear_selection(&self) -> bool {
        let (tx, rx) = mpsc::channel();
        let cleared = self
            .cmd_tx
            .send(MirrorCommand::ClearSelection { reply: tx })
            .is_ok()
            && rx.recv().unwrap_or(false);
        if cleared {
            (self.shared.waker)();
        }
        cleared
    }

    pub fn selection_text(&self) -> Option<String> {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(MirrorCommand::SelectionText { reply: tx })
            .is_err()
        {
            return None;
        }
        rx.recv().ok().flatten()
    }

    pub fn selection_contains(&self, point: GridPoint) -> bool {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(MirrorCommand::SelectionContains { point, reply: tx })
            .is_err()
        {
            return false;
        }
        rx.recv().unwrap_or(false)
    }

    pub fn selection_type(&self) -> Option<SelectionKind> {
        *self.shared.selection_kind.lock().unwrap()
    }

    pub fn cell_is_whitespace(&self, point: GridPoint) -> bool {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(MirrorCommand::CellIsWhitespace { point, reply: tx })
            .is_err()
        {
            return true;
        }
        rx.recv().unwrap_or(true)
    }

    /// The directory the shell last reported through OSC 7, while it still
    /// is one. `None` for any other runtime, for a shell that has not
    /// reported, and for a directory removed since its report.
    pub fn live_cwd(&self) -> Option<PathBuf> {
        self.shared
            .metadata
            .lock()
            .unwrap()
            .live_cwd
            .clone()
            .filter(|cwd| cwd.is_dir())
    }

    /// Relative paths try the shell's live cwd first, then the spawn cwd, so
    /// output printed before a `cd` still resolves.
    pub fn link_at(&self, point: GridPoint) -> Option<TerminalLink> {
        self.link_at_with(point, &FileLinkContext::default())
    }

    /// Relative paths try the shell's live cwd, then the spawn cwd, then any
    /// extra root from `context`, without duplicates.
    pub fn link_at_with(
        &self,
        point: GridPoint,
        context: &FileLinkContext,
    ) -> Option<TerminalLink> {
        let live = self.shared.metadata.lock().unwrap().live_cwd.clone();
        let spawn = self.link_cwd.get_or_init(|| {
            self.shared
                .client
                .terminal_link_cwd(&self.shared.session_id)
                .ok()
                .flatten()
        });
        let mut cwds: Vec<&Path> = Vec::with_capacity(3);
        for root in [
            live.as_deref(),
            spawn.as_deref(),
            context.extra_root.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if !cwds.contains(&root) {
                cwds.push(root);
            }
        }
        self.with_screen(|screen| terminal_link_at(&screen.view(), point, &cwds))
    }

    pub fn move_cursor_to_viewport(&self, column: usize, row: usize) {
        let snap = self.viewport_snapshot();
        let view = snap.view();
        let cursor = view.raw_cursor_point();
        let columns = view.cols();
        let mode = view.mode();
        let display_offset = view.display_offset();
        if display_offset != 0 || columns == 0 {
            return;
        }

        let mut bytes = Vec::new();
        if mode.contains(TerminalModeFlags::ALT_SCREEN) {
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
        let snap = self.viewport_snapshot();
        let view = snap.view();
        TerminalScrollState {
            history_lines: view.history_size(),
            display_offset: view.display_offset(),
            screen_lines: view.rows(),
        }
    }

    pub fn scroll_to_display_offset(&self, display_offset: usize) {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(MirrorCommand::ScrollToDisplayOffset {
                display_offset,
                reply: tx,
            })
            .is_ok()
        {
            let _ = rx.recv();
        }
        (self.shared.waker)();
    }
}

pub(crate) fn finish_parsed_chunk_vt(
    sequence: &mut SequenceState,
    term: &vt::Terminal,
    view: &ScreenView<'_>,
    render_held: bool,
    was_tui_ready: bool,
    wrote_visible: bool,
    seq: u64,
) {
    finish_parsed_chunk_paint(
        sequence,
        term,
        !render_held && view_has_visible_paint(view),
        render_held,
        was_tui_ready,
        wrote_visible,
        seq,
    );
}

fn finish_parsed_chunk_paint(
    sequence: &mut SequenceState,
    term: &vt::Terminal,
    has_paint: bool,
    render_held: bool,
    was_tui_ready: bool,
    wrote_visible: bool,
    seq: u64,
) {
    update_tui_ready_vt(sequence, term, render_held, was_tui_ready, seq);
    if render_held {
        return;
    }
    if sequence.first_paint_seq == 0 && has_paint {
        sequence.first_paint_seq = seq;
    }
    if seq > 0 && wrote_visible && has_paint {
        sequence.last_paint_seq = sequence.last_paint_seq.max(seq);
    } else if sequence.first_paint_seq > 0 && sequence.last_paint_seq == 0 {
        sequence.last_paint_seq = sequence.first_paint_seq;
    }
}

pub(crate) fn active_output_seq(sequence: &SequenceState) -> u64 {
    if sequence.last_output_seq > 0 {
        sequence.last_output_seq
    } else {
        sequence.last
    }
}

fn view_has_visible_paint(view: &ScreenView<'_>) -> bool {
    view.visible_cells()
        .any(|cell| !cell.c().is_whitespace() && !cell.c().is_control())
}

const TUI_READY_MODES: TerminalModeFlags = TerminalModeFlags::ALT_SCREEN
    .union(TerminalModeFlags::BRACKETED_PASTE)
    .union(TerminalModeFlags::MOUSE_MODE)
    .union(TerminalModeFlags::FOCUS_IN_OUT)
    .union(TerminalModeFlags::KITTY_KEYBOARD_PROTOCOL);

pub(crate) fn term_indicates_tui_ready_vt(term: &vt::Terminal, render_held: bool) -> bool {
    let mode = TerminalModeFlags::from_vt(term);
    mode.intersects(TUI_READY_MODES)
        || !mode.contains(TerminalModeFlags::SHOW_CURSOR)
        || render_held
        || term.mode(vt::dec_mode(2026))
}

fn update_tui_ready_vt(
    sequence: &mut SequenceState,
    term: &vt::Terminal,
    render_held: bool,
    was_tui_ready: bool,
    seq: u64,
) {
    if seq > 0
        && (sequence.tui_ready_seq == 0 || !was_tui_ready)
        && term_indicates_tui_ready_vt(term, render_held)
    {
        sequence.tui_ready_seq = sequence.tui_ready_seq.max(seq);
    }
}

pub(crate) fn chunk_indicates_tui_ready(bytes: &[u8]) -> bool {
    [
        b"\x1b[?2004h".as_slice(),
        b"\x1b[?1049h",
        b"\x1b[?1047h",
        b"\x1b[?47h",
        b"\x1b[?2026h",
        b"\x1b[?2031h",
        b"\x1b[?1004h",
        b"\x1b[?25l",
    ]
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
                    log::debug!("reattach {}: {error}", mirror.session_id());
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
    let rgb = |color: palette::Rgb| [color.r, color.g, color.b];
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
    let rgb = |[r, g, b]: [u8; 3]| palette::Rgb { r, g, b };
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
pub struct TestGridObserver {
    shared: Arc<MirrorShared>,
}

#[cfg(any(test, feature = "test-support"))]
impl TestGridObserver {
    pub fn visible_lines(&self) -> Vec<String> {
        let snap = Arc::clone(&*self.shared.published.lock().unwrap());
        snap.view().visible_lines()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl TerminalMirror {
    pub fn test_stop_frames(&self) {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(MirrorCommand::StopFrames { reply: tx })
            .is_ok()
        {
            let _ = rx.recv();
        }
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
        *self.shared.sync_snapshot.lock().unwrap()
    }

    pub fn viewer_count(&self) -> usize {
        self.shared.viewers.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn test_render_state_updates(&self) -> usize {
        self.shared.render_state_updates.load(Ordering::Relaxed)
    }

    pub fn test_is_screen_locked(&self) -> bool {
        *self.shared.actor_thread.lock().unwrap() == Some(thread::current().id())
    }

    pub fn test_semantic_escape_chars(&self) -> String {
        XTERM_WORD_SEPARATORS.to_owned()
    }

    pub fn test_grid_observer(&self) -> TestGridObserver {
        TestGridObserver {
            shared: Arc::clone(&self.shared),
        }
    }

    pub fn test_set_wrapline(&self, point: GridPoint, wrap: bool) {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(MirrorCommand::TestSetWrapline {
                point,
                wrap,
                reply: tx,
            })
            .is_ok()
        {
            let _ = rx.recv();
        }
    }

    pub fn test_set_row_text(&self, line: Line, text: &str) {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(MirrorCommand::TestSetRowText {
                line,
                text: text.to_owned(),
                reply: tx,
            })
            .is_ok()
        {
            let _ = rx.recv();
        }
    }
}

impl runner_core::protocol::terminal::TerminalLifecycle for TerminalBridge {
    fn event(&self, event: ClientEvent) {
        self.handle_event(&event);
    }
}

#[cfg(test)]
mod tests;
