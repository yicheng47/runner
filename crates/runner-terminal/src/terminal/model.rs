use super::*;

use crate::screen::ScreenData;
use crate::vt::{
    self, ClipboardLocation, ColorScheme, RenderState, TerminalConfig, TerminalEffect,
    DEFAULT_CONTINUATION_MAX_BYTES, DEFAULT_SCROLLBACK_LINES,
};
use runner_core::protocol::terminal::SnapshotFormat;

const SYNC_HOLD_TIMEOUT: Duration = Duration::from_millis(150);

pub trait TerminalHost: Send + Sync + 'static {
    fn write_input(&self, session_id: &str, bytes: &[u8]) -> Result<()>;
    fn write_reply(&self, session_id: &str, bytes: &[u8]) -> Result<()>;
    fn report_input_state(&self, session_id: &str, observation: InputObservation);
    fn set_live_title(&self, session_id: &str, title: &str) -> Result<()>;
    /// Called on every accepted OSC title before cache deduplication so a runtime can track
    /// title freshness even when the title text matches the cached value or persistence fails.
    fn observe_live_title(&self, _session_id: &str, _title: &str) {}
    fn resize(&self, session_id: &str, cols: u16, rows: u16) -> Result<()>;
    fn is_local_host(&self, host: &str) -> bool;
    /// A clipboard write (OSC 52, OSC 1337 Copy, or OSC 5522) from the program
    /// in the pane; `text` is never empty and at most [`MAX_CLIPBOARD_BYTES`].
    fn clipboard_store(&self, session_id: &str, target: ClipboardTarget, text: &str);
}

pub const MAX_CLIPBOARD_BYTES: usize = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardTarget {
    Clipboard,
    /// The primary selection on Linux; the clipboard elsewhere.
    Selection,
}
impl ClipboardTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clipboard => "clipboard",
            Self::Selection => "selection",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "clipboard" => Some(Self::Clipboard),
            "selection" => Some(Self::Selection),
            _ => None,
        }
    }
}

#[derive(Default)]
pub struct ModelOptions {
    pub runtime: Option<runner_core::Runtime>,
    pub initial_title: String,
    pub cwd: Option<String>,
}

enum HostEffect {
    WriteReply(Vec<u8>),
    TitleChanged(String),
    ClipboardStore {
        target: ClipboardTarget,
        text: String,
    },
    SyncSessionTitle {
        initial_title: String,
        apply: Box<dyn FnOnce() + Send>,
    },
    ApplySessionTitle {
        title: String,
        persist: PersistSessionTitle,
    },
    ReportInputState,
    #[cfg(any(test, feature = "test-support"))]
    Flush(Sender<()>),
    Stop,
}

enum ScreenSessionCmd {
    #[cfg(any(test, feature = "test-support"))]
    SnapshotState(Sender<crate::snapshot::TerminalSnapshotState>),
    Done,
}

enum ModelCommand {
    FeedOutput {
        seq: u64,
        bytes: Vec<u8>,
        reply: Sender<Option<InputObservation>>,
    },
    Resize {
        cols: u16,
        rows: u16,
        reply: Sender<()>,
    },
    Configure {
        scrollback: usize,
        cursor_shape: CursorShape,
        reply: Sender<()>,
    },
    SetPalette {
        palette: palette::TerminalPalette,
        reply: Sender<()>,
    },
    SnapshotBinary {
        seq: u64,
        reply: Sender<TerminalSnapshot>,
    },
    SnapshotVt {
        seq: u64,
        history: usize,
        reply: Sender<TerminalSnapshot>,
    },
    SyncSessionTitle {
        initial_title: String,
        apply: Box<dyn FnOnce() + Send>,
        reply: Sender<Option<InputObservation>>,
    },
    ObserveInput {
        input: InputEvent,
        reply: Sender<Option<InputObservation>>,
    },
    TrackDeliveryInput {
        text: String,
        reply: Sender<()>,
    },
    TrackDeliverySubmit {
        reply: Sender<()>,
    },
    ConfirmDelivery {
        revision: u64,
        reply: Sender<Option<InputObservation>>,
    },
    RecheckDelivery {
        reply: Sender<Option<InputObservation>>,
    },
    ResetInputState {
        guard: u64,
        reply: Sender<Option<InputObservation>>,
    },
    WithUnchangedInput {
        revision: u64,
        probe: crate::delivery_probe::InputProbe,
        ready: Sender<bool>,
        commit: mpsc::Receiver<bool>,
        done: Sender<()>,
    },
    WithScreen {
        ready: Sender<ScreenData>,
        session: mpsc::Receiver<ScreenSessionCmd>,
    },
    InputRows {
        reply: Sender<Vec<String>>,
    },
    CursorPoint {
        reply: Sender<GridPoint>,
    },
    Mode {
        reply: Sender<TerminalModeFlags>,
    },
    ClearProgramStatus {
        reply: Sender<()>,
    },
    InvalidateProgramStatus {
        reply: Sender<()>,
    },
    #[cfg(test)]
    UnsentNotice {
        reply: Sender<bool>,
    },
    Stop,
}

struct ModelShared {
    stopped: AtomicBool,
    resync_needed: AtomicBool,
    size: Mutex<(u16, u16)>,
    title: Mutex<String>,
    live_cwd: Mutex<Option<PathBuf>>,
    pending_program_status: Mutex<Vec<(u64, crate::program_status::Update)>>,
    latest_observation: Mutex<InputObservation>,
    reset_guard: AtomicU64,
    delivery_held: AtomicBool,
    publish_order: Mutex<()>,
    #[cfg(test)]
    delivery_in_flight: AtomicBool,
}

pub struct TerminalModel {
    workers: Mutex<Vec<thread::JoinHandle<()>>>,
    shared: Arc<ModelShared>,
    cmd_tx: Sender<ModelCommand>,
    effects_tx: Sender<HostEffect>,
    host: Arc<dyn TerminalHost>,
    session_id: String,
    initial_observation: InputObservation,
    #[cfg(any(test, feature = "test-support"))]
    is_shell: bool,
}

impl TerminalModel {
    pub fn with_screen<R>(&self, f: impl FnOnce(&ScreenGuard<'_>) -> R) -> R {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (session_tx, session_rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(ModelCommand::WithScreen {
                ready: ready_tx,
                session: session_rx,
            })
            .is_err()
        {
            let (cols, rows) = self.size();
            let data = ScreenData::empty(usize::from(cols), usize::from(rows));
            let guard = ScreenGuard::from_data(
                data,
                #[cfg(any(test, feature = "test-support"))]
                None,
            );
            return f(&guard);
        }
        let data = match ready_rx.recv() {
            Ok(data) => data,
            Err(_) => {
                let (cols, rows) = self.size();
                ScreenData::empty(usize::from(cols), usize::from(rows))
            }
        };
        #[cfg(any(test, feature = "test-support"))]
        let snap_tx = session_tx.clone();
        let guard = ScreenGuard::from_data(
            data,
            #[cfg(any(test, feature = "test-support"))]
            Some(Box::new(move || {
                let (tx, rx) = mpsc::channel();
                let _ = snap_tx.send(ScreenSessionCmd::SnapshotState(tx));
                rx.recv().expect("actor returned snapshot_state")
            })),
        );
        let result = f(&guard);
        let _ = session_tx.send(ScreenSessionCmd::Done);
        result
    }

    pub fn mode(&self) -> TerminalModeFlags {
        let (tx, rx) = mpsc::channel();
        if self.cmd_tx.send(ModelCommand::Mode { reply: tx }).is_err() {
            return TerminalModeFlags::empty();
        }
        rx.recv().unwrap_or_default()
    }

    pub fn cursor_point(&self) -> GridPoint {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(ModelCommand::CursorPoint { reply: tx })
            .is_err()
        {
            return GridPoint::default();
        }
        rx.recv().unwrap_or_default()
    }

    pub fn visible_lines(&self) -> Vec<String> {
        self.with_screen(|screen| screen.view().visible_lines())
    }

    pub fn input_rows(&self) -> Vec<String> {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(ModelCommand::InputRows { reply: tx })
            .is_err()
        {
            return Vec::new();
        }
        rx.recv().unwrap_or_default()
    }

    /// Serialize the revision and row checks with native input observations until `write` has
    /// committed the Enter. `write` returns `None` to decline it, which leaves the tracker as it was.
    pub fn with_unchanged_input<T>(
        &self,
        revision: u64,
        probe: &crate::delivery_probe::InputProbe,
        write: impl FnOnce() -> Option<T>,
    ) -> Option<T> {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (commit_tx, commit_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        self.cmd_tx
            .send(ModelCommand::WithUnchangedInput {
                revision,
                probe: probe.clone(),
                ready: ready_tx,
                commit: commit_rx,
                done: done_tx,
            })
            .ok()?;
        if !ready_rx.recv().unwrap_or(false) {
            return None;
        }
        #[cfg(test)]
        self.shared.delivery_in_flight.store(true, Ordering::SeqCst);
        let result = write();
        #[cfg(test)]
        self.shared
            .delivery_in_flight
            .store(false, Ordering::SeqCst);
        let _ = commit_tx.send(result.is_some());
        let _ = done_rx.recv();
        result
    }

    pub fn take_workers(&self) -> Vec<thread::JoinHandle<()>> {
        std::mem::take(&mut *self.workers.lock().unwrap())
    }

    pub fn stop_workers(&self) -> Vec<thread::JoinHandle<()>> {
        self.shared.stopped.store(true, Ordering::Release);
        let _ = self.cmd_tx.send(ModelCommand::Stop);
        let _ = self.effects_tx.send(HostEffect::Stop);
        self.take_workers()
    }

    pub fn new(
        host: Arc<dyn TerminalHost>,
        session_id: String,
        cols: u16,
        rows: u16,
        options: ModelOptions,
    ) -> Result<Arc<Self>> {
        crate::vt::init_sys();
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (effects_tx, effects_rx) = mpsc::channel();

        let now = Instant::now();
        let mut input_tracker = InputTracker::for_session(session_id.clone(), now);
        let initial_observation = input_tracker.initial_observation(now);
        let initial_guard = input_tracker.reset_guard();
        let initial_held = input_tracker.delivery_held();
        let fixture_recorder = match FixtureRecorder::from_env(&session_id, cols, rows) {
            Ok(recorder) => recorder,
            Err(error) => {
                log::error!("start input fixture recorder for {session_id} failed: {error:#}");
                None
            }
        };
        let is_shell = options.runtime.is_some_and(runner_core::Runtime::is_shell);

        let shared = Arc::new(ModelShared {
            stopped: AtomicBool::new(false),
            resync_needed: AtomicBool::new(false),
            size: Mutex::new((cols, rows)),
            title: Mutex::new(options.initial_title),
            live_cwd: Mutex::new(None),
            pending_program_status: Mutex::default(),
            latest_observation: Mutex::new(initial_observation),
            reset_guard: AtomicU64::new(initial_guard),
            delivery_held: AtomicBool::new(initial_held),
            publish_order: Mutex::default(),
            #[cfg(test)]
            delivery_in_flight: AtomicBool::new(false),
        });

        // Dedicated FIFO host-effects worker: dispatches PTY replies, title
        // persistence, clipboard writes, and title-generation fences outside
        // the VT actor so slow host callbacks never stall PTY ingestion.
        let fx_shared = Arc::clone(&shared);
        let fx_host = Arc::clone(&host);
        let fx_session = session_id.clone();
        let fx_runtime = options.runtime;
        let fx_cwd = options.cwd.clone();
        let effects_worker = thread::Builder::new()
            .name(format!("native-term-events-{session_id}"))
            .spawn(move || {
                while let Ok(effect) = effects_rx.recv() {
                    if fx_shared.stopped.load(Ordering::Acquire) {
                        break;
                    }
                    match effect {
                        HostEffect::Stop => break,
                        HostEffect::WriteReply(bytes) => {
                            let _ = fx_host.write_reply(&fx_session, &bytes);
                        }
                        HostEffect::TitleChanged(raw) => {
                            let cleaned = match fx_runtime {
                                Some(runtime) if !runtime.is_shell() => {
                                    let Some(title) =
                                        runner_core::protocol::session_title::osc_provider_title(
                                            &raw,
                                            runtime,
                                            fx_cwd.as_deref(),
                                        )
                                    else {
                                        continue;
                                    };
                                    title
                                }
                                _ => sanitize_title(raw.trim()),
                            };
                            fx_host.observe_live_title(&fx_session, &cleaned);
                            let changed = {
                                let mut held = fx_shared.title.lock().unwrap();
                                let changed = *held != cleaned;
                                if changed {
                                    *held = cleaned.clone();
                                }
                                changed
                            };
                            if changed {
                                if let Err(error) = fx_host.set_live_title(&fx_session, &cleaned) {
                                    log::warn!("persist terminal title for {fx_session}: {error}");
                                }
                            }
                        }
                        HostEffect::ClipboardStore { target, text } => {
                            fx_host.clipboard_store(&fx_session, target, &text);
                        }
                        HostEffect::SyncSessionTitle {
                            initial_title,
                            apply,
                        } => {
                            *fx_shared.title.lock().unwrap() = initial_title;
                            apply();
                        }
                        HostEffect::ApplySessionTitle {
                            title: next_title,
                            persist,
                        } => {
                            let cache_changed = *fx_shared.title.lock().unwrap() != next_title;
                            if let Some(after_cache_update) = persist(&next_title, cache_changed) {
                                if cache_changed {
                                    *fx_shared.title.lock().unwrap() = next_title;
                                }
                                after_cache_update();
                            }
                        }
                        HostEffect::ReportInputState => {
                            let _order = fx_shared.publish_order.lock().unwrap();
                            let latest = *fx_shared.latest_observation.lock().unwrap();
                            fx_host.report_input_state(&fx_session, latest);
                        }
                        #[cfg(any(test, feature = "test-support"))]
                        HostEffect::Flush(tx) => {
                            let _ = tx.send(());
                        }
                    }
                }
            })
            .context("spawn terminal event thread")?;

        // Single-owner VT actor thread: owns GhosttyTerminal, RenderState,
        // InputTracker, RecordStore, FixtureRecorder, and 150 ms render_hold timer.
        let actor_shared = Arc::clone(&shared);
        let actor_host = Arc::clone(&host);
        let actor_effects = effects_tx.clone();
        let actor_session = session_id.clone();
        let (init_tx, init_rx) = mpsc::channel();
        let actor_worker = thread::Builder::new()
            .name(format!("native-term-actor-{session_id}"))
            .spawn(move || {
                let config = TerminalConfig {
                    cols: cols.max(1),
                    rows: rows.max(1),
                    scrollback_lines: DEFAULT_SCROLLBACK_LINES,
                    continuation_max_bytes: DEFAULT_CONTINUATION_MAX_BYTES,
                    kitty_image_storage_limit: 0,
                    clipboard_write_max_bytes: MAX_CLIPBOARD_BYTES,
                    color_scheme: if palette::RUNNER.is_light() {
                        ColorScheme::Light
                    } else {
                        ColorScheme::Dark
                    },
                    cell_size_px: (0, 0),
                    enable_host_callbacks: true,
                    enable_title_events: false,
                    enable_render_hold: true,
                };
                let init_result = (|| -> Result<ModelActor> {
                    let mut term = vt::Terminal::new(&config)
                        .map_err(|err| anyhow::anyhow!("create libghostty-vt terminal: {err}"))?;
                    let _ = term.set_theme_defaults(palette::RUNNER);
                    let render_state = RenderState::new().map_err(|err| {
                        anyhow::anyhow!("create libghostty-vt render state: {err}")
                    })?;
                    Ok(ModelActor {
                        term,
                        render_state,
                        session_id: actor_session,
                        shared: actor_shared,
                        host: actor_host,
                        effects_tx: actor_effects,
                        sequence: SequenceState::default(),
                        input_tracker,
                        program_status: crate::program_status::RecordStore::default(),
                        fixture_recorder,
                        palette: PaletteState::new(palette::RUNNER),
                        is_shell,
                        render_hold_deadline: None,
                        held_sync_bytes: 0,
                        sync_wrote_visible: false,
                        visible_scanner: VisibleTextScanner::default(),
                        last_visible_lines: vec![String::new(); usize::from(rows.max(1))],
                        default_cursor_hidden: false,
                        resync_pending_available: false,
                    })
                })();
                let mut actor = match init_result {
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
            .context("spawn terminal actor thread")?;

        init_rx
            .recv()
            .map_err(|_| anyhow::anyhow!("terminal actor exited during init"))??;

        let model = Arc::new(Self {
            workers: Mutex::new(vec![effects_worker, actor_worker]),
            shared,
            cmd_tx,
            effects_tx,
            host,
            session_id,
            initial_observation,
            #[cfg(any(test, feature = "test-support"))]
            is_shell,
        });
        Ok(model)
    }

    pub fn feed_output(&self, seq: u64, bytes: &[u8]) -> Option<InputObservation> {
        if self.shared.stopped.load(Ordering::Acquire) {
            return None;
        }
        let (tx, rx) = mpsc::channel();
        self.cmd_tx
            .send(ModelCommand::FeedOutput {
                seq,
                bytes: bytes.to_vec(),
                reply: tx,
            })
            .ok()?;
        rx.recv().ok().flatten()
    }

    pub fn publish_initial(&self) {
        self.publish_input(Some(self.initial_observation));
    }

    pub fn take_program_statuses(&self) -> Vec<(u64, crate::program_status::Update)> {
        std::mem::take(&mut *self.shared.pending_program_status.lock().unwrap())
    }

    pub fn take_program_status(&self) -> Option<(u64, crate::program_status::Update)> {
        self.take_program_statuses().pop()
    }

    /// True once when the VT parser returns to ground after an earlier
    /// snapshot could not be serialized mid-sequence, signaling callers to emit
    /// `TerminalFrame::Resync` so waiting mirrors recover automatically.
    pub fn take_resync_needed(&self) -> bool {
        self.shared.resync_needed.swap(false, Ordering::AcqRel)
    }

    pub fn clear_program_status(&self) {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(ModelCommand::ClearProgramStatus { reply: tx })
            .is_ok()
        {
            let _ = rx.recv();
        }
        self.shared.pending_program_status.lock().unwrap().clear();
    }

    pub fn invalidate_program_status(&self) {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(ModelCommand::InvalidateProgramStatus { reply: tx })
            .is_ok()
        {
            let _ = rx.recv();
        }
        self.shared.pending_program_status.lock().unwrap().clear();
    }

    /// Callers publish after releasing the tracker, so each publication reports the tracker's
    /// latest observation in order: a delayed caller cannot replace a newer state with its own.
    pub fn publish_input(&self, observation: Option<InputObservation>) {
        if self.shared.stopped.load(Ordering::Acquire) {
            return;
        }
        if observation.is_none() {
            return;
        }
        let _order = self.shared.publish_order.lock().unwrap();
        let latest = *self.shared.latest_observation.lock().unwrap();
        self.host.report_input_state(&self.session_id, latest);
    }

    /// Lossless `libghostty-vt` binary snapshot for local desktop attaches.
    pub fn snapshot(&self, seq: u64) -> TerminalSnapshot {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(ModelCommand::SnapshotBinary { seq, reply: tx })
            .is_ok()
        {
            if let Ok(snapshot) = rx.recv() {
                return snapshot;
            }
        }
        let (cols, rows) = self.size();
        TerminalSnapshot {
            seq,
            format: SnapshotFormat::GhosttyBinary,
            unavailable: true,
            cols,
            rows,
            bytes: Vec::new(),
            unfinished_len: 0,
            preceding_char: None,
            activity: None,
            controller: Default::default(),
        }
    }

    /// VT-formatted snapshot via Ghostty's `Formatter` for companion / browser
    /// consumers (`companion::VERSION = 1`), capped at `history` scrollback lines.
    pub fn snapshot_with_history(&self, seq: u64, history: usize) -> TerminalSnapshot {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(ModelCommand::SnapshotVt {
                seq,
                history,
                reply: tx,
            })
            .is_ok()
        {
            if let Ok(snapshot) = rx.recv() {
                return snapshot;
            }
        }
        let (cols, rows) = self.size();
        TerminalSnapshot {
            seq,
            format: SnapshotFormat::Vt,
            unavailable: true,
            cols,
            rows,
            bytes: Vec::new(),
            unfinished_len: 0,
            preceding_char: None,
            activity: None,
            controller: Default::default(),
        }
    }

    pub fn write_input(&self, bytes: &[u8]) -> Result<()> {
        if self.shared.stopped.load(Ordering::Acquire) {
            anyhow::bail!("terminal stopped");
        }
        self.host.write_input(&self.session_id, bytes)
    }

    pub fn size(&self) -> (u16, u16) {
        *self.shared.size.lock().unwrap()
    }

    pub fn configure_named(&self, scrollback: usize, shape: &str) {
        let shape = match shape {
            "underline" => CursorShape::Underline,
            "beam" => CursorShape::Beam,
            "hollow" => CursorShape::HollowBlock,
            "hidden" => CursorShape::Hidden,
            _ => CursorShape::Block,
        };
        self.configure(scrollback, shape);
    }

    pub fn title(&self) -> String {
        self.shared.title.lock().unwrap().clone()
    }

    pub fn sync_session_title(&self, initial_title: String, apply: impl FnOnce() + Send + 'static) {
        if self.shared.stopped.load(Ordering::Acquire) {
            return;
        }
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(ModelCommand::SyncSessionTitle {
                initial_title,
                apply: Box::new(apply),
                reply: tx,
            })
            .is_ok()
        {
            if let Ok(observation) = rx.recv() {
                self.publish_input(observation);
            }
        }
    }

    pub fn apply_session_title(
        &self,
        title: String,
        persist: impl FnOnce(&str, bool) -> Option<Box<dyn FnOnce() + Send>> + Send + 'static,
    ) -> bool {
        if self.shared.stopped.load(Ordering::Acquire) {
            return false;
        }
        self.effects_tx
            .send(HostEffect::ApplySessionTitle {
                title,
                persist: Box::new(persist),
            })
            .is_ok()
    }

    pub fn live_cwd(&self) -> Option<PathBuf> {
        self.shared.live_cwd.lock().unwrap().clone()
    }

    pub fn set_palette(&self, palette: palette::TerminalPalette) {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(ModelCommand::SetPalette { palette, reply: tx })
            .is_ok()
        {
            let _ = rx.recv();
        }
    }

    pub fn configure(&self, scrollback: usize, cursor_shape: CursorShape) {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(ModelCommand::Configure {
                scrollback,
                cursor_shape,
                reply: tx,
            })
            .is_ok()
        {
            let _ = rx.recv();
        }
    }

    pub fn observe_input(&self, input: &InputEvent) {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(ModelCommand::ObserveInput {
                input: input.clone(),
                reply: tx,
            })
            .is_ok()
        {
            if let Ok(observation) = rx.recv() {
                self.publish_input(observation);
            }
        }
    }

    /// Observe the injected composer's echo without advancing the person's input revision.
    pub fn track_delivery_input(&self, text: &str) {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(ModelCommand::TrackDeliveryInput {
                text: text.to_owned(),
                reply: tx,
            })
            .is_ok()
        {
            let _ = rx.recv();
        }
    }

    pub fn track_delivery_submit(&self) {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(ModelCommand::TrackDeliverySubmit { reply: tx })
            .is_ok()
        {
            let _ = rx.recv();
        }
    }

    pub fn confirm_delivery(&self, revision: u64) -> Option<InputObservation> {
        let (tx, rx) = mpsc::channel();
        self.cmd_tx
            .send(ModelCommand::ConfirmDelivery {
                revision,
                reply: tx,
            })
            .ok()?;
        rx.recv().ok().flatten()
    }

    pub fn recheck_delivery(&self) {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(ModelCommand::RecheckDelivery { reply: tx })
            .is_ok()
        {
            if let Ok(observation) = rx.recv() {
                self.publish_input(observation);
            }
        }
    }

    pub fn input_reset_guard(&self) -> u64 {
        self.shared.reset_guard.load(Ordering::Acquire)
    }

    pub fn delivery_held(&self) -> bool {
        self.shared.delivery_held.load(Ordering::Acquire)
    }

    pub fn reset_input_state(&self, guard: u64) {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(ModelCommand::ResetInputState { guard, reply: tx })
            .is_ok()
        {
            if let Ok(observation) = rx.recv() {
                self.publish_input(observation);
            }
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        let (tx, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(ModelCommand::Resize {
                cols,
                rows,
                reply: tx,
            })
            .is_ok()
        {
            let _ = rx.recv();
        }
    }
}

struct ModelActor {
    term: vt::Terminal,
    render_state: RenderState,
    session_id: String,
    shared: Arc<ModelShared>,
    host: Arc<dyn TerminalHost>,
    effects_tx: Sender<HostEffect>,
    sequence: SequenceState,
    input_tracker: InputTracker,
    program_status: crate::program_status::RecordStore,
    fixture_recorder: Option<FixtureRecorder>,
    palette: PaletteState,
    is_shell: bool,
    render_hold_deadline: Option<Instant>,
    held_sync_bytes: usize,
    sync_wrote_visible: bool,
    visible_scanner: VisibleTextScanner,
    last_visible_lines: Vec<String>,
    default_cursor_hidden: bool,
    resync_pending_available: bool,
}

impl ModelActor {
    fn run(&mut self, rx: mpsc::Receiver<ModelCommand>) {
        loop {
            if self.shared.stopped.load(Ordering::Acquire) {
                break;
            }
            let cmd = match self.render_hold_deadline {
                Some(deadline) => {
                    let now = Instant::now();
                    if deadline <= now {
                        self.flush_sync_timeout();
                        continue;
                    }
                    match rx.recv_timeout(deadline - now) {
                        Ok(cmd) => cmd,
                        Err(RecvTimeoutError::Timeout) => {
                            self.flush_sync_timeout();
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
            if self.shared.stopped.load(Ordering::Acquire) {
                break;
            }
            if !self.handle(cmd) {
                break;
            }
        }
    }

    fn sync_tracker_cache(&self) {
        *self.shared.latest_observation.lock().unwrap() = self.input_tracker.latest_observation();
        self.shared
            .reset_guard
            .store(self.input_tracker.reset_guard(), Ordering::Release);
        self.shared
            .delivery_held
            .store(self.input_tracker.delivery_held(), Ordering::Release);
    }

    fn active_screen_data(&mut self) -> ScreenData {
        let _ = self.render_state.update(&self.term);
        let cursor = self.render_state.cursor().ok();
        ScreenData::from_terminal_active(&self.term, cursor, self.default_cursor_hidden)
    }

    fn full_screen_data(&mut self) -> ScreenData {
        let _ = self.render_state.update(&self.term);
        let cursor = self.render_state.cursor().ok();
        ScreenData::from_terminal_full(&self.term, cursor, self.default_cursor_hidden)
    }

    fn active_input_rows(&mut self) -> Vec<String> {
        let data = self.active_screen_data();
        let view = data.view();
        (0..view.rows())
            .map(|row| view.row_cells(crate::types::Line(row as i32)))
            .collect()
    }

    fn drain_vt_effects(&mut self, seq: u64, now: Instant, held_now: &mut bool) {
        for effect in self.term.take_effects() {
            match effect {
                TerminalEffect::WritePty(reply) => {
                    let masked = mask_kitty_keyboard_query_reply(reply);
                    let _ = self.effects_tx.send(HostEffect::WriteReply(masked));
                }
                TerminalEffect::TitleChanged(raw) => {
                    let _ = self.effects_tx.send(HostEffect::TitleChanged(raw));
                }
                TerminalEffect::PwdChanged(raw_pwd) => {
                    if self.is_shell {
                        if let Some(cwd) =
                            parse_cwd_report(&raw_pwd, &|host| self.host.is_local_host(host))
                        {
                            *self.shared.live_cwd.lock().unwrap() = Some(cwd);
                        }
                    }
                }
                TerminalEffect::ClipboardWrite { location, text } => {
                    let target = match location {
                        ClipboardLocation::Standard => ClipboardTarget::Clipboard,
                        ClipboardLocation::Selection | ClipboardLocation::Primary => {
                            ClipboardTarget::Selection
                        }
                    };
                    let _ = self
                        .effects_tx
                        .send(HostEffect::ClipboardStore { target, text });
                }
                TerminalEffect::ProgramStatus(raw) => {
                    if let Some(update) = self.program_status.apply_raw_report(raw) {
                        self.shared
                            .pending_program_status
                            .lock()
                            .unwrap()
                            .push((seq, update));
                    }
                }
                TerminalEffect::SemanticPrompt(ev) => {
                    if let Some(update) = self.program_status.apply_semantic_prompt(&ev) {
                        self.shared
                            .pending_program_status
                            .lock()
                            .unwrap()
                            .push((seq, update));
                    }
                }
                TerminalEffect::Reset => {
                    if let Some(update) = self.program_status.apply_reset() {
                        self.shared
                            .pending_program_status
                            .lock()
                            .unwrap()
                            .push((seq, update));
                    }
                }
                TerminalEffect::RenderHold(held) => {
                    *held_now = held;
                    if held {
                        self.render_hold_deadline = Some(now + SYNC_HOLD_TIMEOUT);
                    } else {
                        self.render_hold_deadline = None;
                        self.held_sync_bytes = 0;
                    }
                }
                TerminalEffect::Bell => {}
            }
        }
    }

    fn feed_vt_and_dispatch(&mut self, seq: u64, now: Instant, bytes: &[u8]) {
        let was_held = self.render_hold_deadline.is_some();
        let mut held_now = was_held;
        if self.program_status.is_discarding_inflight() {
            let (consumed, reached) = self
                .term
                .vt_write_until_osc_or_ground(bytes)
                .unwrap_or((bytes.len(), false));
            self.drain_vt_effects(seq, now, &mut held_now);
            if reached || self.term.vt_ground() {
                self.program_status.note_vt_ground();
            }
            if consumed < bytes.len() {
                self.term.vt_write(&bytes[consumed..]);
                self.drain_vt_effects(seq, now, &mut held_now);
            }
        } else {
            self.term.vt_write(bytes);
            self.drain_vt_effects(seq, now, &mut held_now);
        }
        self.term.clamp_kitty_keyboard_flags();
        if held_now {
            if was_held && bytes.windows(BEGIN_SYNC.len()).any(|w| w == BEGIN_SYNC) {
                self.render_hold_deadline = Some(now + SYNC_HOLD_TIMEOUT);
            }
            self.held_sync_bytes = self.held_sync_bytes.saturating_add(bytes.len());
        } else {
            self.render_hold_deadline = None;
            self.held_sync_bytes = 0;
        }
        if self.term.vt_ground() {
            self.program_status.note_vt_ground();
        }
        if self.resync_pending_available
            && (self.term.vt_ground() || self.term.continuation().is_ok())
        {
            self.resync_pending_available = false;
            self.shared.resync_needed.store(true, Ordering::Release);
        }
    }

    fn flush_sync_timeout(&mut self) {
        let now = Instant::now();
        let Some(deadline) = self.render_hold_deadline else {
            return;
        };
        if deadline > now {
            return;
        }
        let held = self.held_sync_bytes;
        let was_tui_ready = term_indicates_tui_ready_vt(&self.term, true);
        let _ = self.term.set_mode(vt::dec_mode(2026), false);
        let _ = self.term.take_effects();
        self.render_hold_deadline = None;
        self.held_sync_bytes = 0;
        let wrote_visible = std::mem::take(&mut self.sync_wrote_visible);
        let seq = active_output_seq(&self.sequence);
        let data = self.active_screen_data();
        let view = data.view();
        self.last_visible_lines = view.visible_lines();
        finish_parsed_chunk_vt(
            &mut self.sequence,
            &self.term,
            &view,
            false,
            was_tui_ready,
            wrote_visible,
            seq,
        );
        let observation = self.input_tracker.observe_output(now, &view);
        self.sync_tracker_cache();
        log::info!(
            "terminal {}: synchronized update timed out without its end marker; flushed {held} held bytes",
            self.session_id
        );
        if observation.is_some() {
            let _ = self.effects_tx.send(HostEffect::ReportInputState);
        }
    }

    fn handle(&mut self, cmd: ModelCommand) -> bool {
        match cmd {
            ModelCommand::Stop => return false,
            ModelCommand::FeedOutput { seq, bytes, reply } => {
                let now = Instant::now();
                if seq <= self.sequence.last {
                    let _ = reply.send(None);
                    return true;
                }
                if let Some(recorder) = &self.fixture_recorder {
                    recorder.record_output(&bytes);
                }
                self.sequence.last = seq;
                self.sequence.last_output_seq = seq;
                self.sequence.last_output_at = Some(now);
                if chunk_indicates_tui_ready(&bytes) {
                    self.sequence.tui_ready_seq = self.sequence.tui_ready_seq.max(seq);
                }
                let was_tui_ready =
                    term_indicates_tui_ready_vt(&self.term, self.render_hold_deadline.is_some());
                let wrote_printable = self.visible_scanner.feed(&bytes);
                self.feed_vt_and_dispatch(seq, now, &bytes);
                let data = self.active_screen_data();
                let view = data.view();
                let after_lines = view.visible_lines();
                let lines_changed = self.last_visible_lines != after_lines;
                self.last_visible_lines = after_lines;
                let wrote_visible = wrote_printable || lines_changed;
                let render_held = self.render_hold_deadline.is_some();
                if render_held {
                    self.sync_wrote_visible |= wrote_visible;
                }
                let effective_wrote_visible = if render_held {
                    false
                } else {
                    let synced = std::mem::take(&mut self.sync_wrote_visible);
                    wrote_visible || synced
                };
                finish_parsed_chunk_vt(
                    &mut self.sequence,
                    &self.term,
                    &view,
                    render_held,
                    was_tui_ready,
                    effective_wrote_visible,
                    seq,
                );
                let observation = self.input_tracker.observe_output(now, &view);
                self.sync_tracker_cache();
                let _ = reply.send(observation);
            }
            ModelCommand::Resize { cols, rows, reply } => {
                let mut size = self.shared.size.lock().unwrap();
                if *size != (cols, rows) {
                    if let Some(recorder) = &self.fixture_recorder {
                        recorder.record_resize(cols, rows);
                    }
                    *size = (cols, rows);
                }
                drop(size);
                let was_held = self.render_hold_deadline.is_some();
                let _ = self.term.resize(cols, rows, 0, 0);
                // Ignore synthetic RenderHold(false) emitted by ghostty_terminal_resize.
                let _ = self.term.take_effects();
                if was_held {
                    let _ = self.term.set_mode(vt::dec_mode(2026), true);
                    let _ = self.term.take_effects();
                }
                self.last_visible_lines = self.active_screen_data().view().visible_lines();
                let _ = reply.send(());
            }
            ModelCommand::Configure {
                scrollback,
                cursor_shape,
                reply,
            } => {
                self.default_cursor_hidden = cursor_shape == CursorShape::Hidden;
                let _ = self.term.set_scrollback_lines(scrollback);
                let _ = self.term.set_default_cursor_style(cursor_shape, true);
                let _ = reply.send(());
            }
            ModelCommand::SetPalette { palette, reply } => {
                if self.palette.theme != palette {
                    let flipped = self.palette.theme.is_light() != palette.is_light();
                    self.palette = PaletteState::new(palette);
                    let _ = self.term.set_theme_defaults(palette);
                    if flipped && self.term.mode(vt::dec_mode(2031)) {
                        let scheme = if palette.is_light() {
                            ColorScheme::Light
                        } else {
                            ColorScheme::Dark
                        };
                        if let Ok(bytes) = vt::encode_color_scheme_report(scheme) {
                            let _ = self.effects_tx.send(HostEffect::WriteReply(bytes));
                        }
                    }
                }
                let _ = reply.send(());
            }
            ModelCommand::SnapshotBinary { seq, reply } => {
                let (bytes, unavailable) = match crate::snapshot::encode_binary(&self.term) {
                    Ok(bytes) => (bytes, false),
                    Err(err) => {
                        log::warn!(
                            "terminal {}: binary snapshot unavailable at seq {seq}: {err}",
                            self.session_id
                        );
                        self.resync_pending_available = true;
                        (Vec::new(), true)
                    }
                };
                // Ghostty's binary snapshot already encodes `previous_codepoint`
                // directly in its TERMINAL record; skip a redundant encode.
                let snapshot = TerminalSnapshot {
                    seq,
                    format: SnapshotFormat::GhosttyBinary,
                    unavailable,
                    cols: self.term.cols(),
                    rows: self.term.rows(),
                    bytes,
                    unfinished_len: 0,
                    preceding_char: None,
                    activity: Some(TerminalSnapshotActivity {
                        output_seq: self.sequence.last_output_seq,
                        tui_ready_seq: self.sequence.tui_ready_seq,
                        first_paint_seq: self.sequence.first_paint_seq,
                        paint_seq: self.sequence.last_paint_seq,
                    }),
                    controller: Default::default(),
                };
                let _ = reply.send(snapshot);
            }
            ModelCommand::SnapshotVt {
                seq,
                history,
                reply,
            } => {
                let continuation = match self.term.continuation() {
                    Ok(bytes) => Ok(bytes),
                    Err(_) if self.term.vt_ground() => Ok(Vec::new()),
                    Err(err) => Err(err),
                };
                let (bytes, unfinished_len, preceding_char, unavailable) = match continuation {
                    Ok(continuation) => match crate::snapshot::try_serialize_vt_with_history(
                        &self.term,
                        &continuation,
                        history,
                    ) {
                        Ok(bytes) => {
                            let preceding_char = self.term.previous_char();
                            (bytes, continuation.len(), preceding_char, false)
                        }
                        Err(err) => {
                            log::warn!(
                                "terminal {}: VT snapshot format unavailable at seq {seq}: {err}",
                                self.session_id
                            );
                            self.resync_pending_available = true;
                            (Vec::new(), 0, None, true)
                        }
                    },
                    Err(err) => {
                        log::warn!(
                            "terminal {}: VT snapshot continuation unavailable at seq {seq}: {err}",
                            self.session_id
                        );
                        self.resync_pending_available = true;
                        (Vec::new(), 0, None, true)
                    }
                };
                let snapshot = TerminalSnapshot {
                    seq,
                    format: SnapshotFormat::Vt,
                    unavailable,
                    cols: self.term.cols(),
                    rows: self.term.rows(),
                    bytes,
                    unfinished_len,
                    preceding_char,
                    activity: Some(TerminalSnapshotActivity {
                        output_seq: self.sequence.last_output_seq,
                        tui_ready_seq: self.sequence.tui_ready_seq,
                        first_paint_seq: self.sequence.first_paint_seq,
                        paint_seq: self.sequence.last_paint_seq,
                    }),
                    controller: Default::default(),
                };
                let _ = reply.send(snapshot);
            }
            ModelCommand::SyncSessionTitle {
                initial_title,
                apply,
                reply,
            } => {
                let now = Instant::now();
                let observation = if self.render_hold_deadline.is_some() {
                    let was_tui_ready = term_indicates_tui_ready_vt(&self.term, true);
                    let _ = self.term.set_mode(vt::dec_mode(2026), false);
                    self.render_hold_deadline = None;
                    self.held_sync_bytes = 0;
                    let seq = active_output_seq(&self.sequence);
                    let data = self.active_screen_data();
                    let view = data.view();
                    finish_parsed_chunk_vt(
                        &mut self.sequence,
                        &self.term,
                        &view,
                        false,
                        was_tui_ready,
                        true,
                        seq,
                    );
                    let obs = self.input_tracker.observe_output(now, &view);
                    self.sync_tracker_cache();
                    obs
                } else {
                    None
                };
                // Abort any dangling partial OSC/CSI/DCS sequence and drop any
                // previous generation's stacked titles without disturbing cells
                // or `previous_codepoint`.
                self.term.clear_title_stack();
                if !self.term.vt_ground() {
                    self.term.vt_write(b"\x18");
                    let _ = self.term.take_effects();
                }
                let _ = self.effects_tx.send(HostEffect::SyncSessionTitle {
                    initial_title,
                    apply,
                });
                let _ = reply.send(observation);
            }
            ModelCommand::ObserveInput { input, reply } => {
                if let Some(recorder) = &self.fixture_recorder {
                    recorder.record_input(&input);
                }
                let data = self.active_screen_data();
                let obs = self
                    .input_tracker
                    .observe_input(&input, Instant::now(), &data.view());
                self.sync_tracker_cache();
                let _ = reply.send(obs);
            }
            ModelCommand::TrackDeliveryInput { text, reply } => {
                let data = self.active_screen_data();
                self.input_tracker
                    .track_delivery_input(&text, Instant::now(), &data.view());
                self.sync_tracker_cache();
                let _ = reply.send(());
            }
            ModelCommand::TrackDeliverySubmit { reply } => {
                let data = self.active_screen_data();
                self.input_tracker
                    .track_delivery_submit(Instant::now(), &data.view());
                self.sync_tracker_cache();
                let _ = reply.send(());
            }
            ModelCommand::ConfirmDelivery { revision, reply } => {
                let obs = self
                    .input_tracker
                    .confirm_delivery(revision, Instant::now());
                self.sync_tracker_cache();
                let _ = reply.send(obs);
            }
            ModelCommand::RecheckDelivery { reply } => {
                let data = self.active_screen_data();
                let obs = self
                    .input_tracker
                    .recheck_delivery(Instant::now(), &data.view());
                self.sync_tracker_cache();
                let _ = reply.send(obs);
            }
            ModelCommand::ResetInputState { guard, reply } => {
                let obs = self.input_tracker.reset_if_unchanged(guard, Instant::now());
                self.sync_tracker_cache();
                let _ = reply.send(obs);
            }
            ModelCommand::WithUnchangedInput {
                revision,
                probe,
                ready,
                commit,
                done,
            } => {
                let rows = self.active_input_rows();
                let data = self.active_screen_data();
                let view = data.view();
                let ok = self.input_tracker.reset_guard() == revision
                    && (probe.state(&rows) == crate::delivery_probe::ProbeState::Present
                        || self.input_tracker.unsent_notice(&view));
                if !ok {
                    let _ = ready.send(false);
                    let _ = done.send(());
                    return true;
                }
                let _ = ready.send(true);
                if commit.recv().unwrap_or(false) {
                    let data = self.active_screen_data();
                    self.input_tracker
                        .track_delivery_submit(Instant::now(), &data.view());
                    self.sync_tracker_cache();
                }
                let _ = done.send(());
            }
            ModelCommand::WithScreen { ready, session } => {
                let data = self.full_screen_data();
                if ready.send(data).is_err() {
                    return true;
                }
                #[cfg_attr(not(any(test, feature = "test-support")), allow(clippy::never_loop))]
                while let Ok(msg) = session.recv() {
                    match msg {
                        #[cfg(any(test, feature = "test-support"))]
                        ScreenSessionCmd::SnapshotState(tx) => {
                            let _ = tx.send(crate::snapshot::inspect_vt(&self.term));
                        }
                        ScreenSessionCmd::Done => break,
                    }
                }
            }
            ModelCommand::InputRows { reply } => {
                let _ = reply.send(self.active_input_rows());
            }
            ModelCommand::CursorPoint { reply } => {
                let (cx, cy) = self.term.cursor_pos();
                let _ = reply.send(GridPoint::new(
                    crate::types::Line(i32::from(cy)),
                    crate::types::Column(usize::from(cx)),
                ));
            }
            ModelCommand::Mode { reply } => {
                let _ = reply.send(TerminalModeFlags::from_vt(&self.term));
            }
            ModelCommand::ClearProgramStatus { reply } => {
                self.program_status.clear();
                let _ = reply.send(());
            }
            ModelCommand::InvalidateProgramStatus { reply } => {
                let cont = self.term.continuation().unwrap_or_default();
                let inflight = cont.starts_with(b"\x1b]7501;")
                    && cont != b"\x1b]7501;?"
                    && cont != b"\x1b]7501;?\x1b";
                self.program_status.invalidate_records(inflight);
                let _ = reply.send(());
            }
            #[cfg(test)]
            ModelCommand::UnsentNotice { reply } => {
                let data = self.active_screen_data();
                let _ = reply.send(self.input_tracker.unsent_notice(&data.view()));
            }
        }
        true
    }
}

/// Mask Ghostty's `CSI ? <flags> u` reply down to bit 0 (`DISAMBIGUATE_ESC_CODES`)
/// for Phase 1 so apps do not wait for key-release / repeat events before the
/// desktop UI wires key-up dispatch.
pub(super) fn mask_kitty_keyboard_query_reply(bytes: Vec<u8>) -> Vec<u8> {
    if let Ok(text) = std::str::from_utf8(&bytes) {
        if let Some(flags_str) = text
            .strip_prefix("\x1b[?")
            .and_then(|s| s.strip_suffix('u'))
        {
            if !flags_str.is_empty() && flags_str.bytes().all(|b| b.is_ascii_digit()) {
                if let Ok(flags) = flags_str.parse::<u8>() {
                    return format!("\x1b[?{}u", flags & 1).into_bytes();
                }
            }
        }
    }
    bytes
}

#[cfg(any(test, feature = "test-support"))]
impl TerminalModel {
    pub fn flush_events(&self) {
        // Round-trip through the actor first so all prior commands have enqueued
        // their host effects, then wait on the host-effects worker FIFO.
        let _ = self.mode();
        let (tx, rx) = mpsc::channel();
        self.effects_tx.send(HostEffect::Flush(tx)).unwrap();
        rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    }

    pub fn is_shell(&self) -> bool {
        self.is_shell
    }
}

#[cfg(test)]
impl TerminalModel {
    fn unsent_notice(&self) -> bool {
        let (tx, rx) = mpsc::channel();
        self.cmd_tx
            .send(ModelCommand::UnsentNotice { reply: tx })
            .unwrap();
        rx.recv().unwrap()
    }
}

#[cfg(test)]
mod delivery_tests {
    use super::*;
    use crate::delivery_probe::InputProbe;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    #[derive(Default)]
    struct Host {
        entered: AtomicBool,
    }
    impl TerminalHost for Host {
        fn write_input(&self, _: &str, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn write_reply(&self, _: &str, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn report_input_state(&self, _: &str, _: InputObservation) {}
        fn set_live_title(&self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn resize(&self, _: &str, _: u16, _: u16) -> Result<()> {
            Ok(())
        }
        fn is_local_host(&self, _: &str) -> bool {
            true
        }
        fn clipboard_store(&self, _: &str, _: ClipboardTarget, _: &str) {}
    }

    #[test]
    fn stopped_workers_join_while_the_model_is_still_retained() {
        let model = TerminalModel::new(
            Arc::new(Host::default()),
            "stopped".into(),
            80,
            4,
            ModelOptions::default(),
        )
        .unwrap();
        model.feed_output(1, b"\x1b[?2026hbuffered update");
        let workers = model.stop_workers();
        assert_eq!(workers.len(), 2);
        for worker in workers {
            worker.join().unwrap();
        }
        assert!(model.stop_workers().is_empty());
        assert!(model.feed_output(2, b"\x1b]10;?\x1b\\").is_none());
        assert!(model.write_input(b"late input").is_err());
    }

    #[test]
    fn sync_session_title_orders_queued_titles_across_generations_and_resets_cache() {
        struct GenerationHost {
            generation: Arc<Mutex<String>>,
            persisted: Mutex<Vec<(String, String)>>,
            first_entered: Sender<()>,
            release_first: Mutex<Option<mpsc::Receiver<()>>>,
        }
        impl TerminalHost for GenerationHost {
            fn write_input(&self, _: &str, _: &[u8]) -> Result<()> {
                Ok(())
            }
            fn write_reply(&self, _: &str, _: &[u8]) -> Result<()> {
                Ok(())
            }
            fn report_input_state(&self, _: &str, _: InputObservation) {}
            fn set_live_title(&self, _: &str, title: &str) -> Result<()> {
                let first = self.release_first.lock().unwrap().take();
                if let Some(release) = first {
                    let _ = self.first_entered.send(());
                    release.recv_timeout(Duration::from_secs(5)).unwrap();
                }
                let generation = self.generation.lock().unwrap().clone();
                self.persisted
                    .lock()
                    .unwrap()
                    .push((generation, title.to_owned()));
                Ok(())
            }
            fn resize(&self, _: &str, _: u16, _: u16) -> Result<()> {
                Ok(())
            }
            fn is_local_host(&self, _: &str) -> bool {
                true
            }
            fn clipboard_store(&self, _: &str, _: ClipboardTarget, _: &str) {}
        }

        let generation = Arc::new(Mutex::new("gen-1".to_owned()));
        let (first_entered, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let host = Arc::new(GenerationHost {
            generation: Arc::clone(&generation),
            persisted: Mutex::default(),
            first_entered,
            release_first: Mutex::new(Some(release_rx)),
        });
        let model = TerminalModel::new(
            host.clone(),
            "rebind".into(),
            80,
            4,
            ModelOptions {
                runtime: Some(runner_core::Runtime::Codex),
                ..ModelOptions::default()
            },
        )
        .unwrap();

        // Hold the worker inside the first title callback, then queue a late pre-resume
        // title, the rebind fence (clearing cached title for fresh fallback), and an
        // identical post-resume title before releasing the worker.
        model.feed_output(1, b"\x1b]2;First Run\x07");
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        model.feed_output(2, b"\x1b]2;Same Topic\x07");
        let rebind_gen = Arc::clone(&generation);
        model.sync_session_title(String::new(), move || {
            *rebind_gen.lock().unwrap() = "gen-2".to_owned();
        });
        model.feed_output(3, b"\x1b]2;Same Topic\x07");
        release_tx.send(()).unwrap();
        model.flush_events();

        assert_eq!(
            *host.persisted.lock().unwrap(),
            vec![
                ("gen-1".to_owned(), "First Run".to_owned()),
                ("gen-1".to_owned(), "Same Topic".to_owned()),
                ("gen-2".to_owned(), "Same Topic".to_owned()),
            ]
        );
        let workers = model.stop_workers();
        for worker in workers {
            worker.join().unwrap();
        }
    }

    #[test]
    fn sync_session_title_drains_unfinished_synchronized_updates_and_partial_osc_before_rebind() {
        struct RecordingHost {
            generation: Arc<Mutex<&'static str>>,
            emitted: Mutex<Vec<(&'static str, String)>>,
        }
        impl TerminalHost for RecordingHost {
            fn write_input(&self, _: &str, _: &[u8]) -> Result<()> {
                Ok(())
            }
            fn write_reply(&self, _: &str, _: &[u8]) -> Result<()> {
                Ok(())
            }
            fn report_input_state(&self, _: &str, _: InputObservation) {}
            fn set_live_title(&self, _: &str, title: &str) -> Result<()> {
                self.emitted
                    .lock()
                    .unwrap()
                    .push((*self.generation.lock().unwrap(), title.to_owned()));
                Ok(())
            }
            fn resize(&self, _: &str, _: u16, _: u16) -> Result<()> {
                Ok(())
            }
            fn is_local_host(&self, _: &str) -> bool {
                true
            }
            fn clipboard_store(&self, _: &str, _: ClipboardTarget, _: &str) {}
        }

        let generation = Arc::new(Mutex::new("old"));
        let host = Arc::new(RecordingHost {
            generation: Arc::clone(&generation),
            emitted: Mutex::default(),
        });
        let model = TerminalModel::new(
            host.clone(),
            "sync-rebind".into(),
            80,
            4,
            ModelOptions {
                runtime: Some(runner_core::Runtime::Codex),
                ..ModelOptions::default()
            },
        )
        .unwrap();

        // 1. Unfinished ESC[?2026h block followed by rebind and later ESC[?2026l.
        model.feed_output(1, b"\x1b[?2026h\x1b]2;Old buffered title\x07");
        let (bound_tx, bound_rx) = mpsc::channel();
        let gen_for_rebind = Arc::clone(&generation);
        model.sync_session_title(String::new(), move || {
            *gen_for_rebind.lock().unwrap() = "new";
            bound_tx.send(()).unwrap();
        });
        bound_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        model.feed_output(2, b"\x1b[?2026l");
        model.flush_events();
        assert_eq!(model.title(), "");
        assert_eq!(
            *host.emitted.lock().unwrap(),
            vec![("old", "Old buffered title".to_owned())]
        );

        // 2. Unfinished ESC[?2026h block racing with the sync-flush timeout worker, plus
        // stacked title (`CSI 22;0t`) and unterminated OSC 2 bytes before rebind.
        model.feed_output(
            3,
            b"\x1b]2;Stacked Old\x07\x1b[22;0t\x1b[?2026h\x1b]2;Buffered Before Timeout\x07",
        );
        model.feed_output(4, b"\x1b[?2026l\x1b]2;Unterminated Old ");
        model.feed_output(5, b"\x07\x1b[?2026h\x1b]2;Held Sync Two\x07");
        let (bound2_tx, bound2_rx) = mpsc::channel();
        let gen_for_rebind2 = Arc::clone(&generation);
        model.sync_session_title(String::new(), move || {
            *gen_for_rebind2.lock().unwrap() = "gen-3";
            bound2_tx.send(()).unwrap();
        });
        bound2_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        // Also test an unterminated OSC sequence across rebind into gen-4.
        model.feed_output(6, b"\x1b]2;Dangling Prefix ");
        let (bound3_tx, bound3_rx) = mpsc::channel();
        let gen_for_rebind3 = Arc::clone(&generation);
        model.sync_session_title(String::new(), move || {
            *gen_for_rebind3.lock().unwrap() = "gen-4";
            bound3_tx.send(()).unwrap();
        });
        bound3_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        model.feed_output(7, b"Plain Text\x07\x1b[23;0t\x1b]2;Clean Gen Four\x07");
        model.flush_events();

        let emitted = host.emitted.lock().unwrap().clone();
        assert!(
            !emitted.iter().any(|(generation, _)| *generation == "gen-3"),
            "unexpected gen-3 emissions: {emitted:?}"
        );
        let gen4_titles: Vec<_> = emitted
            .iter()
            .filter(|(generation, _)| *generation == "gen-4")
            .map(|(_, title)| title.as_str())
            .collect();
        assert_eq!(gen4_titles, vec!["Clean Gen Four"]);
        assert_eq!(model.title(), "Clean Gen Four");

        let workers = model.stop_workers();
        for worker in workers {
            worker.join().unwrap();
        }
    }

    #[test]
    fn pi_runtime_ignores_unbranded_subprocess_osc_titles() {
        struct TitleHost {
            persisted: Mutex<Vec<String>>,
        }
        impl TerminalHost for TitleHost {
            fn write_input(&self, _: &str, _: &[u8]) -> Result<()> {
                Ok(())
            }
            fn write_reply(&self, _: &str, _: &[u8]) -> Result<()> {
                Ok(())
            }
            fn report_input_state(&self, _: &str, _: InputObservation) {}
            fn set_live_title(&self, _: &str, title: &str) -> Result<()> {
                self.persisted.lock().unwrap().push(title.to_owned());
                Ok(())
            }
            fn resize(&self, _: &str, _: u16, _: u16) -> Result<()> {
                Ok(())
            }
            fn is_local_host(&self, _: &str) -> bool {
                true
            }
            fn clipboard_store(&self, _: &str, _: ClipboardTarget, _: &str) {}
        }

        let host = Arc::new(TitleHost {
            persisted: Mutex::default(),
        });
        let model = TerminalModel::new(
            host.clone(),
            "pi-titles".into(),
            80,
            4,
            ModelOptions {
                runtime: Some(runner_core::Runtime::Pi),
                cwd: Some(r"C:\Users\jason\runner".into()),
                ..ModelOptions::default()
            },
        )
        .unwrap();

        model.feed_output(1, b"\x1b]0;npm view pi-hashline-edit-pro version\x07");
        model.feed_output(
            2,
            b"\x1b]2;C:\\Windows\\system32\\cmd.exe - npm view pi-hashline-edit-pro version\x07",
        );
        model.feed_output(3, "\x1b]0;π - runner\x07".as_bytes());
        model.flush_events();
        assert_eq!(model.title(), "");
        assert!(host.persisted.lock().unwrap().is_empty());

        model.feed_output(4, "\x1b]0;π - Fix login bug - runner\x07".as_bytes());
        model.feed_output(5, b"\x1b]0;npm view pi-hashline-edit-pro version\x07");
        model.feed_output(6, "\x1b]0;π - runner\x07".as_bytes());
        model.flush_events();
        assert_eq!(model.title(), "Fix login bug");
        assert_eq!(*host.persisted.lock().unwrap(), vec!["Fix login bug"]);

        model.feed_output(7, "\x1b]2;π - Follow-up topic\x1b\\".as_bytes());
        model.flush_events();
        assert_eq!(model.title(), "Follow-up topic");
        assert_eq!(
            *host.persisted.lock().unwrap(),
            vec!["Fix login bug", "Follow-up topic"]
        );

        for worker in model.stop_workers() {
            worker.join().unwrap();
        }
    }

    #[test]
    fn observe_live_title_fires_on_repeated_accepted_osc_titles_without_duplicate_persists() {
        struct ObservingHost {
            observed: Mutex<Vec<String>>,
            persisted: Mutex<Vec<String>>,
        }
        impl TerminalHost for ObservingHost {
            fn write_input(&self, _: &str, _: &[u8]) -> Result<()> {
                Ok(())
            }
            fn write_reply(&self, _: &str, _: &[u8]) -> Result<()> {
                Ok(())
            }
            fn report_input_state(&self, _: &str, _: InputObservation) {}
            fn observe_live_title(&self, _: &str, title: &str) {
                self.observed.lock().unwrap().push(title.to_owned());
            }
            fn set_live_title(&self, _: &str, title: &str) -> Result<()> {
                self.persisted.lock().unwrap().push(title.to_owned());
                Ok(())
            }
            fn resize(&self, _: &str, _: u16, _: u16) -> Result<()> {
                Ok(())
            }
            fn is_local_host(&self, _: &str) -> bool {
                true
            }
            fn clipboard_store(&self, _: &str, _: ClipboardTarget, _: &str) {}
        }

        let host = Arc::new(ObservingHost {
            observed: Mutex::default(),
            persisted: Mutex::default(),
        });
        let model = TerminalModel::new(
            host.clone(),
            "agy-observe".into(),
            80,
            4,
            ModelOptions {
                runtime: Some(runner_core::Runtime::Antigravity),
                initial_title: "Title B".into(),
                ..ModelOptions::default()
            },
        )
        .unwrap();

        model.feed_output(1, b"\x1b]2;Antigravity CLI\x07");
        model.feed_output(2, b"\x1b]2;Title B\x07");
        model.feed_output(3, b"\x1b]2;Title B\x07");
        model.feed_output(4, b"\x1b]2;Title C\x07");
        model.flush_events();

        assert_eq!(
            *host.observed.lock().unwrap(),
            vec!["Title B", "Title B", "Title C"]
        );
        assert_eq!(*host.persisted.lock().unwrap(), vec!["Title C"]);

        for worker in model.stop_workers() {
            worker.join().unwrap();
        }
    }

    #[test]
    fn program_probe_reply_precedes_da_in_the_same_or_next_chunk() {
        #[derive(Default)]
        struct Replies(Mutex<Vec<Vec<u8>>>);
        impl TerminalHost for Replies {
            fn write_input(&self, _: &str, _: &[u8]) -> Result<()> {
                Ok(())
            }
            fn write_reply(&self, _: &str, bytes: &[u8]) -> Result<()> {
                if !bytes.is_empty() {
                    self.0.lock().unwrap().push(bytes.to_vec());
                }
                Ok(())
            }
            fn report_input_state(&self, _: &str, _: InputObservation) {}
            fn set_live_title(&self, _: &str, _: &str) -> Result<()> {
                Ok(())
            }
            fn resize(&self, _: &str, _: u16, _: u16) -> Result<()> {
                Ok(())
            }
            fn is_local_host(&self, _: &str) -> bool {
                true
            }
            fn clipboard_store(&self, _: &str, _: ClipboardTarget, _: &str) {}
        }
        for terminator in ["\x07", "\x1b\\"] {
            let expected_probe = format!(
                "\x1b]7501;{}{terminator}",
                crate::program_status::PROBE_REPLY_BODY
            );
            let bytes = format!("\x1b]7501;?{terminator}\x1b[c");
            for split in 0..=bytes.len() {
                let host = Arc::new(Replies::default());
                let model = TerminalModel::new(
                    host.clone(),
                    "probe".into(),
                    80,
                    4,
                    ModelOptions::default(),
                )
                .unwrap();
                model.feed_output(1, &bytes.as_bytes()[..split]);
                model.feed_output(2, &bytes.as_bytes()[split..]);
                model.flush_events();
                let replies = host.0.lock().unwrap();
                assert_eq!(replies.len(), 2, "split {split}");
                assert_eq!(replies[0], expected_probe.as_bytes());
                assert!(replies[1].starts_with(b"\x1b[?"));
                assert!(replies[1].ends_with(b"c"));
                for worker in model.stop_workers() {
                    worker.join().unwrap();
                }
            }
        }
    }

    #[test]
    fn resubmit_accepts_the_trackers_unsent_notice_when_the_probe_cannot_see_it() {
        let model = TerminalModel::new(
            Arc::new(Host::default()),
            "delivery".into(),
            80,
            4,
            ModelOptions::default(),
        )
        .unwrap();
        model.feed_output(1, b"> ");
        let text = "[inbox] 1 new messages — run `runner msg read` to view.";
        let blind = InputProbe::new(Vec::new(), text);
        model.track_delivery_input(text);
        model.feed_output(2, text.as_bytes());
        let revision = model.input_reset_guard();
        assert_eq!(
            model.with_unchanged_input(revision, &blind, || Some(())),
            None
        );
        model.track_delivery_submit();
        model.feed_output(3, format!("\r> {text}").as_bytes());
        assert_eq!(
            model.with_unchanged_input(revision, &blind, || Some(())),
            Some(())
        );
        model.observe_input(&InputEvent::Key {
            kind: crate::mappings::InputKind::Navigate,
        });
        assert!(model.unsent_notice());
        assert_eq!(
            model.with_unchanged_input(revision, &blind, || Some(())),
            None
        );
        let workers = model.take_workers();
        drop(model);
        for worker in workers {
            worker.join().unwrap();
        }
    }

    #[test]
    fn resubmit_serializes_composition_at_the_revision_check_and_write_boundary() {
        let host = Arc::new(Host::default());
        let model = TerminalModel::new(
            host.clone(),
            "delivery".into(),
            80,
            4,
            ModelOptions::default(),
        )
        .unwrap();
        model.feed_output(1, b"> ");
        let text = "[inbox] 1 new messages — run `runner msg read` to view.";
        let mut probe = InputProbe::new(model.input_rows(), text);
        model.feed_output(2, text.as_bytes());
        probe.locate(&model.input_rows());
        let revision = model.input_reset_guard();
        let (start, go) = mpsc::channel();
        let (ready, started) = mpsc::channel();
        let (done, finished) = mpsc::channel();
        let observing = model.clone();
        let observing_host = host.clone();
        let observer = thread::spawn(move || {
            go.recv_timeout(Duration::from_secs(2)).unwrap();
            ready.send(()).unwrap();
            observing.observe_input(&InputEvent::Composing { composing: true });
            assert!(observing_host.entered.load(Ordering::SeqCst));
            done.send(()).unwrap();
        });
        assert_eq!(
            model.with_unchanged_input(revision, &probe, || {
                assert!(model.shared.delivery_in_flight.load(Ordering::SeqCst));
                start.send(()).unwrap();
                started.recv_timeout(Duration::from_secs(2)).unwrap();
                assert!(finished.try_recv().is_err());
                host.entered.store(true, Ordering::SeqCst);
                Some(())
            }),
            Some(())
        );
        observer.join().unwrap();
        finished.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            model.with_unchanged_input(revision, &probe, || panic!(
                "composition must suppress Enter"
            )),
            None::<()>
        );
        let workers = model.take_workers();
        drop(model);
        for worker in workers {
            worker.join().unwrap();
        }
    }
}

#[cfg(test)]
mod clipboard_tests {
    use super::*;
    use base64::Engine as _;

    #[derive(Default)]
    struct Host {
        stores: Mutex<Vec<(ClipboardTarget, String)>>,
        replies: Mutex<Vec<Vec<u8>>>,
    }
    impl TerminalHost for Host {
        fn write_input(&self, _: &str, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn write_reply(&self, _: &str, bytes: &[u8]) -> Result<()> {
            if !bytes.is_empty() {
                self.replies.lock().unwrap().push(bytes.to_vec());
            }
            Ok(())
        }
        fn report_input_state(&self, _: &str, _: InputObservation) {}
        fn set_live_title(&self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn resize(&self, _: &str, _: u16, _: u16) -> Result<()> {
            Ok(())
        }
        fn is_local_host(&self, host: &str) -> bool {
            host.is_empty() || host.eq_ignore_ascii_case("localhost")
        }
        fn clipboard_store(&self, session_id: &str, target: ClipboardTarget, text: &str) {
            assert_eq!(session_id, "osc52");
            self.stores.lock().unwrap().push((target, text.to_owned()));
        }
    }

    fn feed(output: &[u8]) -> Arc<Host> {
        let host = Arc::new(Host::default());
        let model =
            TerminalModel::new(host.clone(), "osc52".into(), 80, 4, ModelOptions::default())
                .unwrap();
        model.feed_output(1, output);
        model.flush_events();
        let workers = model.take_workers();
        drop(model);
        for worker in workers {
            worker.join().unwrap();
        }
        host
    }

    fn osc52(selector: &str, payload: &str, terminator: &str) -> Vec<u8> {
        format!("\x1b]52;{selector};{payload}{terminator}").into_bytes()
    }

    fn encode(text: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(text)
    }

    #[test]
    fn osc52_writes_reach_the_host_decoded() {
        for terminator in ["\x07", "\x1b\\"] {
            let host = feed(&osc52("c", &encode("runner-osc52 ✓"), terminator));
            assert_eq!(
                *host.stores.lock().unwrap(),
                [(ClipboardTarget::Clipboard, "runner-osc52 ✓".to_owned())]
            );
        }
        for selector in ["p", "s"] {
            let host = feed(&osc52(selector, &encode("primary"), "\x07"));
            assert_eq!(
                *host.stores.lock().unwrap(),
                [(ClipboardTarget::Selection, "primary".to_owned())]
            );
        }
    }

    #[test]
    fn osc1337_copy_and_osc5522_writes_follow_runner_policy() {
        // iTerm2 OSC 1337 Copy reaches the host clipboard decoded.
        let iterm = format!("\x1b]1337;Copy=:{}\x07", encode("from-iterm2 ✓"));
        let host = feed(iterm.as_bytes());
        assert_eq!(
            *host.stores.lock().unwrap(),
            [(ClipboardTarget::Clipboard, "from-iterm2 ✓".to_owned())]
        );

        // Kitty OSC 5522 text/plain write reaches the host clipboard and replies OK.
        let mime_b64 = encode("text/plain");
        let kitty_ok = format!(
            "\x1b]5522;type=write:mime={mime_b64};{}\x1b\\\x1b]5522;type=wdata;{}\x1b\\\x1b]5522;type=wdata;\x1b\\",
            encode("from-kitty"),
            ""
        );
        // Also test single-chunk Kitty OSC 5522 form (type=write with mime + data, then empty wdata).
        let kitty_single = format!(
            "\x1b]5522;type=write;{}\x1b\\\x1b]5522;type=wdata:mime={mime_b64};{}\x1b\\\x1b]5522;type=wdata;\x1b\\",
            "",
            encode("from-kitty ✓")
        );
        let _ = kitty_ok;
        let host = feed(kitty_single.as_bytes());
        assert_eq!(
            *host.stores.lock().unwrap(),
            [(ClipboardTarget::Clipboard, "from-kitty ✓".to_owned())]
        );

        // Non-text MIME via OSC 5522 is rejected without storing.
        let png_mime = encode("image/png");
        let kitty_non_text = format!(
            "\x1b]5522;type=write;\x1b\\\x1b]5522;type=wdata:mime={png_mime};{}\x1b\\\x1b]5522;type=wdata;\x1b\\",
            encode("not-plain-text")
        );
        let host = feed(kitty_non_text.as_bytes());
        assert!(host.stores.lock().unwrap().is_empty());

        // Over-cap OSC 1337 Copy and OSC 5522 writes are ignored.
        let oversize = "a".repeat(MAX_CLIPBOARD_BYTES + 1);
        let iterm_over = format!("\x1b]1337;Copy=:{}\x07", encode(&oversize));
        let host = feed(iterm_over.as_bytes());
        assert!(host.stores.lock().unwrap().is_empty());

        // Kitty OSC 5522 read is denied with EPERM (no clipboard data exposed),
        // and DECRQM for mode 5522 reports unrecognized (`;0$y`) with CLIPBOARD_READ unset.
        let host = feed(b"\x1b]5522;type=read;dGV4dC9wbGFpbg==\x1b\\\x1b[?5522$p");
        assert!(host.stores.lock().unwrap().is_empty());
        assert_eq!(
            *host.replies.lock().unwrap(),
            vec![
                b"\x1b]5522;type=read:status=EPERM\x1b\\".to_vec(),
                b"\x1b[?5522;0$y".to_vec(),
            ]
        );
    }

    #[test]
    fn cwd_callbacks_require_file_uri_and_local_host_across_osc_aliases() {
        let host = Arc::new(Host::default());
        let model = TerminalModel::new(
            host,
            "osc7".into(),
            80,
            4,
            ModelOptions {
                runtime: Some(runner_core::Runtime::Shell),
                ..ModelOptions::default()
            },
        )
        .unwrap();

        // OSC 7 local host accepted.
        model.feed_output(1, b"\x1b]7;file://localhost/tmp/local-dir\x07");
        assert_eq!(model.live_cwd(), Some(PathBuf::from("/tmp/local-dir")));

        // OSC 7 remote host ignored.
        model.feed_output(2, b"\x1b]7;file://remote.example/tmp/remote-dir\x07");
        assert_eq!(model.live_cwd(), Some(PathBuf::from("/tmp/local-dir")));

        // OSC 9;9 plain path ignored.
        model.feed_output(3, b"\x1b]9;9;/tmp/plain-osc9\x07");
        assert_eq!(model.live_cwd(), Some(PathBuf::from("/tmp/local-dir")));

        // OSC 1337 CurrentDir plain path ignored.
        model.feed_output(4, b"\x1b]1337;CurrentDir=/tmp/plain-1337\x07");
        assert_eq!(model.live_cwd(), Some(PathBuf::from("/tmp/local-dir")));

        // OSC 1337 CurrentDir file:// URI with remote host ignored.
        model.feed_output(
            5,
            b"\x1b]1337;CurrentDir=file://remote.example/tmp/remote-1337\x07",
        );
        assert_eq!(model.live_cwd(), Some(PathBuf::from("/tmp/local-dir")));

        // OSC 1337 CurrentDir file:// URI with local host accepted.
        model.feed_output(
            6,
            b"\x1b]1337;CurrentDir=file://localhost/tmp/local-1337\x07",
        );
        assert_eq!(model.live_cwd(), Some(PathBuf::from("/tmp/local-1337")));

        for worker in model.stop_workers() {
            worker.join().unwrap();
        }
    }

    #[test]
    fn empty_oversize_and_invalid_osc52_writes_are_ignored() {
        let largest = "a".repeat(MAX_CLIPBOARD_BYTES);
        let host = feed(&osc52("c", &encode(&largest), "\x07"));
        assert_eq!(host.stores.lock().unwrap().len(), 1);
        for payload in [
            String::new(),
            encode(&format!("{largest}a")),
            "not base64!".into(),
        ] {
            let host = feed(&osc52("c", &payload, "\x07"));
            assert!(host.stores.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn osc52_reads_are_refused_without_a_reply() {
        for terminator in ["\x07", "\x1b\\"] {
            let host = feed(&osc52("c", "?", terminator));
            assert!(host.stores.lock().unwrap().is_empty());
            assert!(host.replies.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn program_status_preserves_ordered_transitions_within_a_single_chunk() {
        use crate::program_status::State;
        let model = TerminalModel::new(
            Arc::new(Host::default()),
            "coalesced-status".into(),
            80,
            4,
            ModelOptions::default(),
        )
        .unwrap();
        model.feed_output(
            1,
            b"\x1b]7501;state=working:app=agy\x07\x1b]7501;state=done:app=agy\x07\x1b]7501;state=idle:app=agy\x07",
        );
        let states = model
            .take_program_statuses()
            .into_iter()
            .map(|(seq, update)| (seq, update.current.map(|report| report.state)))
            .collect::<Vec<_>>();
        assert_eq!(
            states,
            vec![
                (1, Some(State::Working)),
                (1, Some(State::Done)),
                (1, Some(State::Idle)),
            ]
        );
        assert!(model.take_program_statuses().is_empty());
        for worker in model.stop_workers() {
            worker.join().unwrap();
        }
    }
}
