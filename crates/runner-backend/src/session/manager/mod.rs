// Per-runner session manager.
//
// One `Session` = one child process attached to an in-process PTY via
// `SessionRuntime`. The SessionManager holds the map of live sessions
// so Tauri commands can look them up by id (for stdin injection,
// resume, kill). Each session owns:
//
//   - A `RuntimeSession` that the manager hands back to the runtime
//     for every operation.
//   - A forwarder thread that drains the runtime's `OutputStream` into
//     the process-local terminal sink. When the channel closes, the
//     thread queries the runtime for final exit code, emits
//     `session/exit`, and updates the DB row.
//
// At app restart, in-process PTYs are gone with the prior app process.
// Startup cleanup demotes stale running DB rows to stopped; user-facing
// resume respawns a fresh PTY with the same session row id.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Condvar, Mutex, RwLock, Weak};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use rusqlite::params;
use serde::Serialize;

use runner_core::event_log::{EventLog, TryAppendError};
use runner_core::model::{EventDraft, SignalType};

use crate::db::DbPool;
use crate::error::{Error, Result};
use crate::model::{Mission, Runner};
use crate::router;
use crate::session::runtime::{
    OutputStream, RunnerStatus, RuntimeOutput, RuntimeSession, SessionRuntime, SpawnSpec,
};

mod lifecycle;
mod output;
mod spawn;

#[cfg(test)]
mod tests;

const RECENT_LOCAL_INPUT_WINDOW: Duration = Duration::from_secs(2);
// Router delivery normally holds the gate for its 80 ms submit delay. Five
// seconds also clears the 500 ms input-flush grace comfortably under load.
const DIRECT_INPUT_GATE_TIMEOUT: Duration = Duration::from_secs(5);
const RUNNER_STATUS_APPEND_MAX_ATTEMPTS: usize = 8;
const RUNNER_STATUS_APPEND_RETRY_DELAY: Duration = Duration::from_millis(5);

pub(crate) const DEFAULT_PTY_SIZE: (u16, u16) = (80, 24);

/// Trailing debounce for width-changing full-repaint TUI resizes.
const RESIZE_SETTLE_MS: u64 = 175;

/// Minimum spacing between consecutive `claude-code` PTY launches.
/// Long enough for one claude's OAuth refresh round-trip (network
/// POST to api.anthropic.com plus keychain write) to land before a
/// sibling spawn reads the same refresh token. Refresh tokens are
/// conventionally single-use, so concurrent refresh from N parallel
/// claudes causes `invalid_grant` on the losers and forces relogin
/// in those panes. See issue #171.
///
/// Conservative default at 1500ms — covers typical 100-500ms
/// round-trips with margin for slow networks. A user spawning a
/// 3-slot mission pays ~3s of wall clock for the gate (1.5s × 2
/// post-first-spawn waits); a 7-slot werewolf pays ~9s.
///
/// **First spawn through pays zero**: the gate is deadline-based,
/// not RAII-on-drop. It only sleeps when a prior claude spawned
/// within the last GRACE — single direct chats and cold-start
/// mission starts see ~0ms overhead. Scoped to claude-code only;
/// codex / other runtimes bypass.
///
/// Zeroed under `#[cfg(test)]` so existing claude-code path tests
/// don't pay the wall-clock tax. Pure-function `compute_gate_wait`
/// covers the wait-math in tests with explicit grace values.
#[cfg(not(test))]
const CLAUDE_LAUNCH_GATE_GRACE: Duration = Duration::from_millis(1500);
#[cfg(test)]
const CLAUDE_LAUNCH_GATE_GRACE: Duration = Duration::from_millis(0);

/// Inputs the forwarder consumer needs to translate a
/// `RuntimeOutput::StatusTransition` into a real `runner_status`
/// event on the mission's NDJSON log (issue #124). All fields are
/// correlated — a mission spawn has all of them; a direct chat has
/// none — so they live together in one optional struct. The
/// forwarder consumer carries an `Option<Self>`: `Some` for mission
/// sessions, `None` for direct chats. See
/// `docs/features/archive/13-pty-silence-idle-detection.md` §Scope for why
/// direct chats are skipped.
///
/// The `EventLog` handle is opened once at construction (on the
/// Tauri command thread, where a brief blocking flock during tail
/// repair is fine) and cached so the forwarder consumer thread's
/// hot path never calls `EventLog::open` — that path takes a
/// blocking flock to repair any dangling tail, and the forwarder
/// thread also drains terminal output and exit events through the
/// same channel; blocking it would freeze them.
#[derive(Clone)]
pub(crate) struct ForwarderEmitCtx {
    /// `mission.crew_id` — needed for the `EventDraft.crew_id`
    /// field so the appended row matches what the CLI's
    /// `runner status` would have written.
    pub crew_id: String,
    /// Mission id, redundant with the forwarder's outer
    /// `mission_id` argument but copied here so this struct is
    /// self-contained.
    pub mission_id: String,
    /// `slots.slot_handle` (mission spawns) — the `from` field on
    /// the appended event. The router projects state by `from`,
    /// not by session id.
    pub handle: String,
    /// Cached event-log handle. Constructed via `EventLog::open` on
    /// the spawn/resume path; the forwarder consumer
    /// reuses it for every `try_append` so it never blocks on the
    /// open-time tail-repair flock.
    pub event_log: Arc<EventLog>,
}

/// Open the mission's event log on the calling (non-forwarder)
/// thread. Used by spawn / resume to construct a
/// `ForwarderEmitCtx`. Logs at WARN and returns `None` if the open
/// fails — the forwarder still runs the detector for free; we just
/// can't surface its events.
fn open_mission_event_log(
    app_data_dir: &Path,
    crew_id: &str,
    mission_id: &str,
) -> Option<Arc<EventLog>> {
    let mission_dir = runner_core::event_log::path::mission_dir(app_data_dir, crew_id, mission_id);
    match EventLog::open(&mission_dir) {
        Ok(log) => Some(Arc::new(log)),
        Err(e) => {
            log::error!(
                "open event log for mission {mission_id} ({}): {e}",
                mission_dir.display(),
            );
            None
        }
    }
}

/// Outcome of a single forwarder-side `try_append` attempt. Drives
/// the streak counter in the consumer thread (P2 in the @reviewer
/// punch list — see issue #124 comments).
#[derive(Debug)]
enum AppendOutcome {
    Ok,
    Contended,
    Failed,
}

impl ForwarderEmitCtx {
    fn runner_status_draft(&self, state: RunnerStatus, source: &'static str) -> EventDraft {
        let state_str = match state {
            RunnerStatus::Busy => "busy",
            RunnerStatus::Idle => "idle",
        };
        EventDraft::signal(
            self.crew_id.clone(),
            self.mission_id.clone(),
            self.handle.clone(),
            SignalType::new("runner_status"),
            serde_json::json!({ "state": state_str, "source": source }),
        )
    }

    /// Non-blocking append of a forwarder-emitted `runner_status`
    /// row. The consumer thread runs this on every status
    /// transition; it must not block (it shares the mpsc receiver
    /// with the terminal output stream and the exit-event reap, so
    /// a stuck flock would freeze them too). Wire shape mirrors
    /// `cli/src/signal.rs::run_status` so router / UI projections
    /// can't tell the two apart except by `payload.source`.
    fn try_append_runner_status(&self, state: RunnerStatus, source: &'static str) -> AppendOutcome {
        match self.try_append_with_retry(self.runner_status_draft(state, source)) {
            Ok(()) => AppendOutcome::Ok,
            Err(TryAppendError::Contended) => AppendOutcome::Contended,
            Err(TryAppendError::Failed(_)) => AppendOutcome::Failed,
        }
    }

    fn try_append_with_retry(&self, draft: EventDraft) -> std::result::Result<(), TryAppendError> {
        for attempt in 1..=RUNNER_STATUS_APPEND_MAX_ATTEMPTS {
            match self.event_log.try_append(draft.clone()) {
                Ok(_) => return Ok(()),
                Err(TryAppendError::Contended) if attempt < RUNNER_STATUS_APPEND_MAX_ATTEMPTS => {
                    thread::sleep(RUNNER_STATUS_APPEND_RETRY_DELAY);
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!("bounded append loop always returns")
    }

    fn append_runner_status(
        &self,
        state: RunnerStatus,
        source: &'static str,
    ) -> runner_core::Result<()> {
        self.event_log
            .append(self.runner_status_draft(state, source))
            .map(|_| ())
    }
}

/// Streak indices at which the forwarder consumer logs a WARN about
/// dropped `runner_status` events. Picked to cover the common
/// cases (first drop, sustained failure on a stuck mission log)
/// without spamming once it's clear the log is broken.
fn drop_streak_is_loggable(streak: u64) -> bool {
    matches!(streak, 1 | 10 | 100 | 1000) || (streak >= 10_000 && streak.is_multiple_of(10_000))
}

/// Decouples the PTY layer from the app event channel so the reader thread
/// can be unit-tested with a fake. Prod uses `CoreSessionEvents`; tests use
/// a no-op or a channel-capture impl.
pub trait SessionEvents: Send + Sync + 'static {
    fn output(&self, ev: &OutputEvent);
    fn spawned(&self, _ev: &SessionSpawnedEvent) {}
    fn exit(&self, ev: &ExitEvent);
    fn archived(&self, _ev: &SessionUpdatedEvent) {}
    /// Persisted session metadata changed without a lifecycle event
    /// (e.g. async agent_session_key capture). Default no-op so test
    /// fakes don't have to opt in.
    fn updated(&self, _ev: &SessionUpdatedEvent) {}
    /// Live direct-chat activity projection. Mission sessions keep using
    /// `runner_status` rows in the mission log instead.
    fn status(&self, _ev: &SessionActivityEvent) {}
    /// Live activity counter for a runner — emitted on every spawn/reap so
    /// the Runners list can update its "N sessions / M missions" badges
    /// without polling. Default no-op so test fakes don't have to opt in.
    fn runner_activity(&self, _ev: &RunnerActivityEvent) {}
    /// Non-fatal, user-facing advisory (resume fallback, etc.). Default
    /// no-op so test fakes don't have to opt in.
    fn warning(&self, _ev: &WarningEvent) {}
}

#[derive(Clone, Default)]
pub struct SessionEventObserverRegistry {
    observer: Arc<RwLock<Option<Weak<dyn SessionEvents>>>>,
}

impl SessionEventObserverRegistry {
    pub fn install(&self, observer: Weak<dyn SessionEvents>) {
        *self.observer.write().unwrap() = Some(observer);
    }

    fn observer(&self) -> Option<Arc<dyn SessionEvents>> {
        self.observer
            .read()
            .unwrap()
            .as_ref()
            .and_then(Weak::upgrade)
    }
}

/// Payload for `runner/activity`. Derived from the same query
/// `RunnerActivity` (`runner_activity` Tauri command) returns, so a fresh
/// page load and a live update agree.
#[derive(Debug, Clone, Serialize)]
pub struct RunnerActivityEvent {
    pub runner_id: String,
    pub handle: String,
    pub active_sessions: i64,
    pub active_missions: i64,
    pub crew_count: i64,
    /// Most recent running direct-chat session id, if any. Mirrors
    /// `RunnerActivity::direct_session_id` so the sidebar can re-attach
    /// to a live PTY without an extra round-trip.
    pub direct_session_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionActivityState {
    Busy,
    Idle,
}

impl From<RunnerStatus> for SessionActivityState {
    fn from(state: RunnerStatus) -> Self {
        match state {
            RunnerStatus::Busy => Self::Busy,
            RunnerStatus::Idle => Self::Idle,
        }
    }
}

/// Payload for `session/status`. Emitted only for direct chats, where
/// busy/idle is a live UI projection rather than persisted DB state.
#[derive(Debug, Clone, Serialize)]
pub struct SessionActivityEvent {
    pub session_id: String,
    pub state: SessionActivityState,
    pub source: String,
}

/// Production emitter. Raw output goes synchronously to the process-local
/// observer; lifecycle and metadata changes stay on the app event channel.
///
/// Holds the manager as `Weak`: instances get stored inside the manager's
/// own session state (codex capture context, forwarder threads), so a
/// strong ref would create an Arc cycle that keeps both alive past app
/// teardown. `AppCore` owns the strong ref for the process lifetime, so
/// the upgrade only fails during shutdown — where skipping the
/// tab-completion hook is correct anyway.
pub struct CoreSessionEvents {
    db: Arc<DbPool>,
    sessions: std::sync::Weak<SessionManager>,
    windows: Arc<crate::windows::WindowRegistry>,
    events: crate::events::EventChannel,
    observer: SessionEventObserverRegistry,
}

impl CoreSessionEvents {
    pub fn new(
        db: Arc<DbPool>,
        sessions: std::sync::Weak<SessionManager>,
        windows: Arc<crate::windows::WindowRegistry>,
        events: crate::events::EventChannel,
        observer: SessionEventObserverRegistry,
    ) -> Self {
        Self {
            db,
            sessions,
            windows,
            events,
            observer,
        }
    }
}

impl SessionEvents for CoreSessionEvents {
    fn output(&self, ev: &OutputEvent) {
        if let Some(observer) = self.observer.observer() {
            observer.output(ev);
        }
    }
    fn spawned(&self, ev: &SessionSpawnedEvent) {
        if let Some(observer) = self.observer.observer() {
            observer.spawned(ev);
        }
        self.events.emit("session/spawned", ev);
    }
    fn exit(&self, ev: &ExitEvent) {
        if let Some(observer) = self.observer.observer() {
            observer.exit(ev);
        }
        self.events.emit("session/exit", ev);
    }
    fn archived(&self, ev: &SessionUpdatedEvent) {
        if let Some(observer) = self.observer.observer() {
            observer.archived(ev);
        }
        self.events.emit("session/archived", ev);
    }
    fn updated(&self, ev: &SessionUpdatedEvent) {
        self.events.emit("session/updated", ev);
    }
    fn status(&self, ev: &SessionActivityEvent) {
        if ev.state == SessionActivityState::Idle {
            if let Some(sessions) = self.sessions.upgrade() {
                if let Err(error) = crate::ops::node::record_session_completion(
                    &self.db,
                    &sessions,
                    &self.windows,
                    &self.events,
                    &ev.session_id,
                ) {
                    log::warn!(
                        "record direct-chat completion for {} failed: {error}",
                        ev.session_id
                    );
                }
            }
        }
        self.events.emit("session/status", ev);
    }
    fn runner_activity(&self, ev: &RunnerActivityEvent) {
        self.events.emit("runner/activity", ev);
    }
    fn warning(&self, ev: &WarningEvent) {
        self.events.emit("session/warning", ev);
    }
}

/// Raw PTY output delivered synchronously to the process-local terminal sink.
#[derive(Debug, Clone, Serialize)]
pub struct OutputEvent {
    pub session_id: String,
    pub mission_id: Option<String>,
    /// Monotonic per-session sequence number used by terminal readiness gates.
    pub seq: u64,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSpawnedEvent {
    pub session_id: String,
    pub mission_id: Option<String>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExitEvent {
    pub session_id: String,
    pub mission_id: Option<String>,
    pub exit_code: Option<i32>,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionUpdatedEvent {
    pub session_id: String,
    pub mission_id: Option<String>,
}

/// Non-fatal advisory the UI can render as a banner. Emitted on
/// `session/warning`. Today the only producer is the resume-fallback path:
/// when the runtime adapter asked the agent CLI to resume a prior
/// conversation but the child exited fast and unsuccessfully, we treat that
/// as a resume failure, wipe the bad key, and tell the user the next spawn
/// will start fresh.
#[derive(Debug, Clone, Serialize)]
pub struct WarningEvent {
    pub session_id: String,
    pub mission_id: Option<String>,
    /// Stable string the UI can switch on. Free-form strings are
    /// intentional — adding cases shouldn't require a frontend rebuild.
    pub kind: String,
    /// Human-readable detail. Safe to render verbatim.
    pub message: String,
}

/// Row returned to the frontend after a spawn. Subset of the DB `sessions`
/// row with the runner handle denormalized so the debug page can render
/// `@coder`-style labels without a separate lookup.
#[derive(Debug, Clone, Serialize)]
pub struct SpawnedSession {
    pub id: String,
    pub mission_id: Option<String>,
    pub runner_id: Option<String>,
    pub handle: String,
    pub pid: Option<u32>,
    /// True iff this resume detected a missing claude-code
    /// conversation file for a lead slot and degraded to a fresh
    /// spawn. Internal signal: `ops::session::session_resume`
    /// uses it to ask the router to fire the rich launch prompt
    /// (the bus's `mission_goal` handler can't, since
    /// `mission_attach`'s watermark suppresses replay on resume).
    /// Always false on initial spawn / direct chat / non-lead resume
    /// — kept off the frontend type since it's not actionable from
    /// the UI.
    #[serde(skip)]
    pub fresh_fallback_lead: bool,
}

struct SessionHandle {
    // Kept for debugging and future kill-by-pid / identity checks.
    #[allow(dead_code)]
    id: String,
    /// `None` for direct-chat sessions (C8.5). `kill_all_for_mission`
    /// filters on this so direct chats don't get torn down when a mission
    /// stops, and vice versa.
    mission_id: Option<String>,
    /// The runner this session is an instance of. `kill_all_for_runner`
    /// filters on this so deleting a runner can reap its live PTY
    /// children before the cascade nukes the DB rows underneath.
    runner_id: Option<String>,
    /// Runtime-side identity returned from `SessionRuntime::spawn`.
    /// The manager passes this back to `runtime.send_bytes` /
    /// `runtime.resize` / `runtime.stop` for every operation on the
    /// live session.
    runtime_session: RuntimeSession,
    /// Codex-lineage runtimes cannot be given a caller-owned session id at launch.
    /// When this is present, user activity can retry native id
    /// capture after the runtime has actually created its rollout file.
    codex_capture: Option<CodexCaptureContext>,
    /// Forwarder thread that drains the runtime's `OutputStream`
    /// into the process-local terminal sink. `kill` joins on this so callers
    /// (mission_stop) get the same "no live sessions after we
    /// return" contract the portable-pty path provided.
    forwarder: Option<thread::JoinHandle<()>>,
    /// Cancellation flag the forwarder thread polls between
    /// `recv_timeout` calls. `kill` flips it so the consumer
    /// breaks out within ~500ms regardless of whether the PTY reader
    /// has observed EOF and dropped the channel sender. Without this,
    /// kill could hang waiting on the channel-disconnect path if that
    /// cleanup stalled — observed live as a stuck "Archiving…" pill
    /// on the chat page.
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Clone)]
struct CodexCaptureContext {
    mission_id: Option<String>,
    sessions_root: PathBuf,
    spawn_cwd: String,
    started_at: DateTime<Utc>,
    row_started_at: String,
    spawn_pid: Option<i32>,
    prompt_marker: Option<String>,
    pool: Arc<DbPool>,
    events: Arc<dyn SessionEvents>,
}

#[derive(Default)]
struct DeliveryGateState {
    /// Generation invalidates a delayed Enter when the session exits or
    /// respawns with the same database id.
    generation: u64,
    /// Held from router body injection through its delayed Enter so local
    /// keystrokes and another router delivery cannot join the same draft.
    in_flight: bool,
    next_ticket: u64,
    next_served: u64,
    cancelled_tickets: BTreeSet<u64>,
}

impl DeliveryGateState {
    fn skip_cancelled_tickets(&mut self) {
        while self.cancelled_tickets.remove(&self.next_served) {
            self.next_served = self.next_served.wrapping_add(1);
        }
    }
}

#[derive(Default)]
struct DeliveryGate {
    state: Mutex<DeliveryGateState>,
    ready: Condvar,
}

/// Latest PTY size waiting for a resize storm to quiesce.
struct PendingResize {
    generation: u64,
    cols: u16,
    rows: u16,
    deadline: Instant,
    suppressed: u32,
    ioctl_count: u32,
    pool: Arc<DbPool>,
}

#[derive(Default)]
struct SessionState {
    handle: Option<SessionHandle>,
    activity: Option<SessionActivityState>,
    activity_revision: u64,
    suppress_local_input_busy: bool,
    local_input_pending: bool,
    last_local_input_at: Option<Instant>,
    delivery_gate: Arc<DeliveryGate>,
    mission_status_sink: Option<ForwarderEmitCtx>,
    completion_armed: bool,
    output_seq: u64,
    /// Latest grid measurement, including pushes that arrive before a PTY
    /// handle exists. Spawn and resume reconcile this under the state lock.
    last_requested_size: Option<(u16, u16)>,
    last_requested_size_dirty: bool,
    /// Present while resize persistence awaits settlement.
    pending_resize: Option<PendingResize>,
    resuming: bool,
    killed: bool,
}

impl SessionState {
    fn is_empty(&self) -> bool {
        self.handle.is_none()
            && self.activity.is_none()
            && !self.suppress_local_input_busy
            && !self.local_input_pending
            && self.last_local_input_at.is_none()
            && self.mission_status_sink.is_none()
            && !self.completion_armed
            && self.output_seq == 0
            && self.last_requested_size.is_none()
            && !self.last_requested_size_dirty
            && self.pending_resize.is_none()
            && !self.resuming
            && !self.killed
    }
}

pub struct SessionManager {
    /// Per-session state. The outer map lock protects membership only;
    /// each session's hot mutable state lives behind its own mutex so
    /// PTY output for one busy session does not block lifecycle work on
    /// other sessions. Never lock a SessionState while holding this map;
    /// prune is the sole nested path and locks the state before the map.
    sessions: Mutex<HashMap<String, Arc<Mutex<SessionState>>>>,
    delivery_listeners: Mutex<HashMap<String, Vec<Weak<dyn router::SessionDeliveryListener>>>>,
    /// User's current login-shell env snapshot. Discovery swaps this
    /// handle after a successful background probe; spawns clone one
    /// coherent value under a short read lock.
    ///
    /// `path` is composed into every child PTY's PATH (so GUI-launched
    /// apps can find tools like claude / codex / mise that aren't on
    /// launchd's stripped default PATH — issue #65); `vars` (the
    /// proxy quartet in both cases) is layered into every spawn's env
    /// under `runner.env` so the child can reach the network the same
    /// way Terminal.app's children would (issues #109 / #152).
    shell_env: Arc<RwLock<crate::shell_path::LoginShellEnv>>,
    discovery_state: crate::runtime_status::SharedDiscoveryState,
    /// Timestamp of the most recent claude-code spawn through the
    /// launch gate. `None` until the first claude-code spawn lands.
    /// Each new claude-code spawn reads this, sleeps the remainder
    /// of `CLAUDE_LAUNCH_GATE_GRACE`, then updates it. Non-claude
    /// runtimes never touch this field. See `enter_claude_launch_gate`
    /// + issue #171.
    claude_launch_gate: Mutex<Option<Instant>>,
    /// Cancellation flags for in-flight background mission spawns,
    /// keyed by `mission_id`. `mission_start` registers a fresh flag
    /// before dispatching the `complete_mission_session_spawn`
    /// background task; `kill_all_for_mission` flips it. The task
    /// checks the flag around the gate sleep and at the top of each
    /// iteration so queued slots do not keep firing into a stopped or
    /// archived mission. See `cancel_pending_mission_spawns`.
    pending_mission_cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Underlying terminal runtime. Every spawn / resume / kill /
    /// inject_stdin / resize routes through this trait — the manager
    /// owns DB + event-buffer state but never reads/writes a PTY
    /// directly.
    runtime: Arc<dyn SessionRuntime>,
    resize_settle_ms: AtomicU64,
    resize_generation: AtomicU64,
}

/// RAII guard that releases a session state's `resuming` flag on drop. The
/// entry is inserted at the start of `resume()`; the guard's Drop
/// removes it on every exit path (Ok, Err, panic), so a failed
/// resume doesn't leave the session permanently locked out from
/// future retries.
struct ResumeClaim {
    mgr: Arc<SessionManager>,
    session_id: String,
}

impl Drop for ResumeClaim {
    fn drop(&mut self) {
        self.mgr.release_resume_claim(&self.session_id);
    }
}

/// Result of a `complete_mission_session_spawn` call. The
/// background mission-spawn task uses the variant to decide whether
/// to mark the session row stopped (cancelled mid-queue) or leave
/// the just-installed forwarder thread to keep the row in `running`
/// (the normal success path). `Err(_)` is reserved for genuine
/// spawn failures (e.g., `runtime.spawn` couldn't fork the PTY) —
/// the caller marks those rows crashed and emits `session/exit`.
#[derive(Debug, PartialEq, Eq)]
pub enum CompleteSpawnOutcome {
    /// PTY came up, forwarder thread installed, session row reflects
    /// the live runtime metadata. The session is in
    /// `SessionManager.sessions` and behaving like any other live
    /// session.
    Spawned,
    /// `kill_all_for_mission` flipped the cancel flag (Stop / Archive).
    /// The PTY was never forked. Caller should mark the
    /// session row stopped so the workspace UI reflects reality.
    Cancelled,
}

/// Inputs `complete_mission_session_spawn` needs that
/// `register_mission_session` already computed. The two-phase split
/// lets `ops::mission::mission_start` finish row inserts +
/// router/bus mount synchronously and return its Tauri command in
/// ~milliseconds, then drive the slow PTY-spawn phase in a
/// background task. Without the split, the modal Start button
/// blocks ~1500ms per claude-code worker (gate cost) before the
/// workspace loads. See issue #171.
///
/// All fields are owned (clones / Arcs) so the value can travel
/// across thread boundaries into a `spawn_blocking` task.
pub struct PendingMissionSpawn {
    pub session_id: String,
    spec: SpawnSpec,
    mission: Mission,
    runner: Runner,
    slot_handle: String,
    /// Where `spec.initial_size` came from (caller-supplied / mission-hint /
    /// DEFAULT_PTY_SIZE), for the post-spawn fork log line (#366).
    size_source: &'static str,
    plan: router::runtime::ResumePlan,
    first_turn_delivered_via_argv: bool,
    resolved_cwd: Option<String>,
    row_started_at: String,
    codex_prompt_marker: Option<String>,
    app_data_dir: PathBuf,
    pool: Arc<DbPool>,
}

/// Pure helper for `enter_claude_launch_gate`: how long to sleep
/// before letting a new claude-code spawn proceed, given the
/// timestamp of the most recent prior spawn.
///
/// - `None` last → zero (no prior claude to race against).
/// - prior was ≥ `grace` ago → zero (refresh window already elapsed).
/// - prior was < `grace` ago → the remainder.
///
/// Factored out so the wait-math has direct test coverage with
/// explicit grace values, independent of the cfg(test)-zeroed
/// production constant.
fn compute_gate_wait(last: Option<Instant>, now: Instant, grace: Duration) -> Duration {
    match last {
        None => Duration::ZERO,
        Some(t) => {
            let elapsed = now.saturating_duration_since(t);
            grace.saturating_sub(elapsed)
        }
    }
}

impl SessionManager {
    pub fn new(
        shell_env: crate::runtime_status::SharedShellEnv,
        discovery_state: crate::runtime_status::SharedDiscoveryState,
        runtime: Arc<dyn SessionRuntime>,
    ) -> Arc<Self> {
        Arc::new(Self {
            sessions: Mutex::new(HashMap::new()),
            delivery_listeners: Mutex::new(HashMap::new()),
            shell_env,
            discovery_state,
            claude_launch_gate: Mutex::new(None),
            pending_mission_cancels: Mutex::new(HashMap::new()),
            runtime,
            resize_settle_ms: AtomicU64::new(RESIZE_SETTLE_MS),
            resize_generation: AtomicU64::new(0),
        })
    }

    #[cfg(test)]
    pub(crate) fn set_resize_settle_ms(&self, ms: u64) {
        self.resize_settle_ms.store(ms, Ordering::Relaxed);
    }

    fn session_state(&self, session_id: &str) -> Option<Arc<Mutex<SessionState>>> {
        self.sessions.lock().unwrap().get(session_id).cloned()
    }

    fn session_state_or_insert(&self, session_id: &str) -> Arc<Mutex<SessionState>> {
        self.sessions
            .lock()
            .unwrap()
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(SessionState::default())))
            .clone()
    }

    fn latest_requested_size(&self, session_id: &str) -> Option<(u16, u16)> {
        self.session_state(session_id)
            .and_then(|state| state.lock().unwrap().last_requested_size)
    }

    pub fn register_delivery_listener(
        &self,
        session_id: &str,
        listener: Weak<dyn router::SessionDeliveryListener>,
    ) {
        let mut listeners = self.delivery_listeners.lock().unwrap();
        let listeners = listeners.entry(session_id.to_string()).or_default();
        if !listeners.iter().any(|existing| existing.ptr_eq(&listener)) {
            listeners.push(listener);
        }
    }

    fn notify_delivery_event(&self, session_id: &str, event: router::SessionDeliveryEvent) {
        let listeners = {
            let mut listeners_by_session = self.delivery_listeners.lock().unwrap();
            let (live, remove_entry) = {
                let Some(listeners) = listeners_by_session.get_mut(session_id) else {
                    return;
                };
                let mut live = Vec::with_capacity(listeners.len());
                listeners.retain(|listener| {
                    if let Some(listener) = listener.upgrade() {
                        live.push(listener);
                        true
                    } else {
                        false
                    }
                });
                (live, listeners.is_empty())
            };
            if remove_entry {
                listeners_by_session.remove(session_id);
            }
            live
        };
        for listener in listeners {
            listener.session_delivery_event(session_id, event);
        }
    }

    pub fn input_quiescent(&self, session_id: &str) -> bool {
        let Some(session) = self.session_state(session_id) else {
            return false;
        };
        let gate = session.lock().unwrap().delivery_gate.clone();
        let delivery = gate.state.lock().unwrap();
        let session = session.lock().unwrap();
        session.handle.is_some()
            && !delivery.in_flight
            && delivery.next_ticket == delivery.next_served
            && !session.local_input_pending
            && session
                .last_local_input_at
                .is_none_or(|last| last.elapsed() >= RECENT_LOCAL_INPUT_WINDOW)
    }

    pub fn session_live(&self, session_id: &str) -> bool {
        self.session_state(session_id)
            .is_some_and(|session| session.lock().unwrap().handle.is_some())
    }

    pub fn reserve_delivery(&self, session_id: &str) -> Result<router::DeliveryReservation> {
        let Some(session) = self.session_state(session_id) else {
            return Ok(router::DeliveryReservation::Unavailable);
        };
        let gate = session.lock().unwrap().delivery_gate.clone();
        let mut delivery = gate.state.lock().unwrap();
        let session = session.lock().unwrap();
        if session.handle.is_none() {
            return Ok(router::DeliveryReservation::Unavailable);
        }
        if delivery.in_flight {
            return Ok(router::DeliveryReservation::InFlight);
        }
        if delivery.next_ticket != delivery.next_served {
            return Ok(router::DeliveryReservation::InFlight);
        }
        if session.local_input_pending {
            return Ok(router::DeliveryReservation::PendingInput);
        }
        if let Some(last) = session.last_local_input_at {
            let elapsed = last.elapsed();
            if elapsed < RECENT_LOCAL_INPUT_WINDOW {
                return Ok(router::DeliveryReservation::RecentlyTyping(
                    RECENT_LOCAL_INPUT_WINDOW - elapsed,
                ));
            }
        }
        delivery.in_flight = true;
        Ok(router::DeliveryReservation::Ready(delivery.generation))
    }

    pub fn finish_delivery(&self, session_id: &str, token: u64) {
        let Some(session) = self.session_state(session_id) else {
            return;
        };
        let gate = session.lock().unwrap().delivery_gate.clone();
        let finished = {
            let mut delivery = gate.state.lock().unwrap();
            if !delivery.in_flight || delivery.generation != token {
                false
            } else {
                delivery.in_flight = false;
                gate.ready.notify_all();
                true
            }
        };
        if finished {
            self.notify_delivery_event(session_id, router::SessionDeliveryEvent::DeliveryFinished);
        }
    }

    fn prune_empty_session_state(&self, session_id: &str) {
        let Some(state) = self.session_state(session_id) else {
            return;
        };
        let state_guard = state.lock().unwrap();
        if !state_guard.is_empty() {
            return;
        }
        let mut sessions = self.sessions.lock().unwrap();
        if sessions
            .get(session_id)
            .is_some_and(|current| Arc::ptr_eq(current, &state))
        {
            sessions.remove(session_id);
        }
    }

    fn install_handle(
        &self,
        session_id: &str,
        handle: SessionHandle,
        mission_status_sink: Option<ForwarderEmitCtx>,
        initial_size: Option<(u16, u16)>,
        pool: &DbPool,
        events: &dyn SessionEvents,
    ) {
        let initial_size = initial_size.expect("spawn size must be resolved before handle install");
        let mission_id = handle.mission_id.clone();
        let mut terminal_size = initial_size;
        let state = self.session_state_or_insert(session_id);
        let gate = state.lock().unwrap().delivery_gate.clone();
        {
            let mut delivery = gate.state.lock().unwrap();
            let mut state = state.lock().unwrap();
            delivery.generation = delivery.generation.wrapping_add(1);
            delivery.in_flight = false;
            delivery.next_ticket = 0;
            delivery.next_served = 0;
            delivery.cancelled_tickets.clear();
            gate.ready.notify_all();
            state.local_input_pending = false;
            state.last_local_input_at = None;
            state.handle = Some(handle);
            state.mission_status_sink = mission_status_sink;
            state.killed = false;
            state.activity_revision = state.activity_revision.wrapping_add(1);

            let requested_size = state.last_requested_size;
            if let Some((cols, rows)) = requested_size.filter(|size| *size != initial_size) {
                let rt_session = state
                    .handle
                    .as_ref()
                    .expect("handle was just installed")
                    .runtime_session
                    .clone();
                match self.runtime.resize(&rt_session, cols, rows) {
                    Ok(()) => {
                        terminal_size = (cols, rows);
                        log::info!(
                            "pty size reconciled after fork: session={session_id} {cols}x{rows} \
                             (pushed mid-fork)"
                        );
                    }
                    Err(error) => log::warn!(
                        "pty size reconcile after fork failed: session={session_id} \
                         {cols}x{rows}: {error}"
                    ),
                }
            }
            if state.last_requested_size_dirty {
                if let Some((cols, rows)) = state.last_requested_size {
                    // This spawn/resume thread owns the state lock so a
                    // newer resize cannot be overwritten by this dirty size.
                    match pool.get() {
                        Ok(conn) => match crate::repo::session::update_last_size(
                            &conn, session_id, cols, rows,
                        ) {
                            Ok(_) => state.last_requested_size_dirty = false,
                            Err(error) => log::warn!(
                                "resize persistence after handle install failed: \
                                 session={session_id} {cols}x{rows}: {error}"
                            ),
                        },
                        Err(error) => log::warn!(
                            "resize persistence after handle install pool checkout failed: \
                             session={session_id} {cols}x{rows}: {error}"
                        ),
                    }
                }
            }
            state.pending_resize = None;
        }
        events.spawned(&SessionSpawnedEvent {
            session_id: session_id.to_owned(),
            mission_id,
            cols: terminal_size.0,
            rows: terminal_size.1,
        });
        self.notify_delivery_event(session_id, router::SessionDeliveryEvent::Respawned);
    }

    fn install_forwarder(&self, session_id: &str, forwarder: thread::JoinHandle<()>) {
        if let Some(state) = self.session_state(session_id) {
            if let Some(handle) = state.lock().unwrap().handle.as_mut() {
                handle.forwarder = Some(forwarder);
            }
        }
    }

    pub(crate) fn note_forwarder_transition(
        &self,
        session_id: &str,
        state: SessionActivityState,
        source: &str,
    ) -> bool {
        let session = self.session_state_or_insert(session_id);
        let mut session = session.lock().unwrap();
        if source == "forwarder"
            && state == SessionActivityState::Busy
            && session.suppress_local_input_busy
        {
            return false;
        }
        if state == SessionActivityState::Idle {
            session.suppress_local_input_busy = false;
        }
        if session.activity == Some(state) {
            return false;
        }
        session.activity = Some(state);
        session.activity_revision = session.activity_revision.wrapping_add(1);
        true
    }

    pub(crate) fn synthesize_wake_busy(&self, session_id: &str, draft: EventDraft) -> Result<()> {
        let session = self
            .session_state(session_id)
            .ok_or_else(|| Error::msg(format!("session not found: {session_id}")))?;
        let (sink, activity_revision) = {
            let session = session.lock().unwrap();
            let sink = session.mission_status_sink.clone().ok_or_else(|| {
                Error::msg(format!("session has no mission status sink: {session_id}"))
            })?;
            (sink, session.activity_revision)
        };
        match sink.try_append_with_retry(draft) {
            Ok(()) => {}
            Err(TryAppendError::Contended) => return Err(Error::msg("event log busy")),
            Err(TryAppendError::Failed(error)) => return Err(error.into()),
        }
        let mut session = session.lock().unwrap();
        // Teardown clears the sink and advances the revision before this
        // state can be pruned, so an orphaned Arc cannot publish stale Busy.
        if session.activity_revision == activity_revision {
            session.activity = Some(SessionActivityState::Busy);
            session.activity_revision = session.activity_revision.wrapping_add(1);
        }
        Ok(())
    }

    pub(crate) fn publish_direct_activity(
        &self,
        session_id: &str,
        state: SessionActivityState,
        source: &str,
        events: &dyn SessionEvents,
    ) {
        if !self.note_forwarder_transition(session_id, state, source) {
            return;
        }
        events.status(&SessionActivityEvent {
            session_id: session_id.to_string(),
            state,
            source: source.to_string(),
        });
    }

    pub(crate) fn arm_completion(&self, session_id: &str) {
        self.session_state_or_insert(session_id)
            .lock()
            .unwrap()
            .completion_armed = true;
    }

    pub(crate) fn take_completion_armed(&self, session_ids: &[String]) -> bool {
        let sessions: Vec<_> = {
            let sessions = self.sessions.lock().unwrap();
            session_ids
                .iter()
                .filter_map(|session_id| sessions.get(session_id).cloned())
                .collect()
        };
        let mut armed = false;
        for session in sessions {
            let mut session = session.lock().unwrap();
            armed |= session.completion_armed;
            session.completion_armed = false;
        }
        armed
    }

    pub fn activity_snapshot(&self) -> BTreeMap<String, SessionActivityState> {
        let sessions: Vec<_> = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .map(|(id, session)| (id.clone(), Arc::clone(session)))
            .collect();
        sessions
            .into_iter()
            .filter_map(|(id, session)| {
                session
                    .lock()
                    .unwrap()
                    .activity
                    .map(|activity| (id, activity))
            })
            .collect()
    }

    fn codex_capture_context(&self, session_id: &str) -> Option<CodexCaptureContext> {
        let state = self.session_state(session_id)?;
        let state = state.lock().unwrap();
        state
            .handle
            .as_ref()
            .and_then(|handle| handle.codex_capture.clone())
    }

    fn spawn_codex_capture_if_unkeyed(&self, session_id: &str, ctx: &CodexCaptureContext) {
        let Ok(conn) = ctx.pool.get() else { return };
        let should_capture = conn
            .query_row(
                "SELECT agent_session_key IS NULL
                   FROM sessions
                  WHERE id = ?1
                    AND started_at = ?2",
                params![session_id, ctx.row_started_at],
                |r| r.get::<_, bool>(0),
            )
            .unwrap_or(false);
        drop(conn);
        if !should_capture {
            return;
        }
        crate::session::codex_capture::spawn_capture(
            crate::session::codex_capture::CaptureRequest {
                session_id: session_id.to_string(),
                mission_id: ctx.mission_id.clone(),
                sessions_root: ctx.sessions_root.clone(),
                spawn_cwd: ctx.spawn_cwd.clone(),
                started_at: ctx.started_at,
                expected_row_started_at: ctx.row_started_at.clone(),
                spawn_pid: ctx.spawn_pid,
                prompt_marker: ctx.prompt_marker.clone(),
                pool: Arc::clone(&ctx.pool),
                events: Arc::clone(&ctx.events),
            },
        );
    }

    fn live_runtime_session(&self, session_id: &str) -> Result<RuntimeSession> {
        let Some(state) = self.session_state(session_id) else {
            return Err(Error::msg(format!("session not found: {session_id}")));
        };
        let rt_session = state
            .lock()
            .unwrap()
            .handle
            .as_ref()
            .map(|h| h.runtime_session.clone())
            .ok_or_else(|| Error::msg(format!("session not found: {session_id}")))?;
        Ok(rt_session)
    }

    fn release_resume_claim(&self, session_id: &str) {
        if let Some(state) = self.session_state(session_id) {
            state.lock().unwrap().resuming = false;
        }
        self.prune_empty_session_state(session_id);
    }

    fn take_killed(&self, session_id: &str) -> bool {
        let Some(state) = self.session_state(session_id) else {
            return false;
        };
        let was_killed = {
            let mut state = state.lock().unwrap();
            let was_killed = state.killed;
            state.killed = false;
            was_killed
        };
        self.prune_empty_session_state(session_id);
        was_killed
    }

    fn clear_killed(&self, session_id: &str) {
        if let Some(state) = self.session_state(session_id) {
            state.lock().unwrap().killed = false;
        }
        self.prune_empty_session_state(session_id);
    }

    /// Borrow the underlying session runtime. Held on the manager
    /// itself rather than passed through every method so the
    /// Step 9 cutovers can land one entry point at a time without
    /// rewiring every Tauri command's signature in the same change.
    #[allow(dead_code)] // Wired into spawn paths in subsequent commits.
    pub(crate) fn runtime(&self) -> &Arc<dyn SessionRuntime> {
        &self.runtime
    }
}

/// Compute current activity counters for `runner` and emit a
/// `runner/activity` event. Best-effort: if the DB roundtrip fails we drop
/// the emission rather than failing the spawn/reap path. Runners list will
/// reconcile via the next emission or a manual refresh.
/// Resolve the cwd the codex_capture watcher should match against.
/// portable-pty substitutes $HOME when the spawn has no cwd (or a
/// nonexistent one), so the fallback must be the home dir — that is
/// what codex stamps into the rollout's `payload.cwd`.
fn capture_cwd(explicit: Option<String>) -> Option<String> {
    if let Some(cwd) = explicit {
        if !cwd.is_empty() {
            return Some(cwd);
        }
    }
    std::env::var_os("HOME").and_then(|h| h.into_string().ok())
}

/// Outcome of resolving a runtime override against a runner row
/// (feature 41).
#[derive(Debug)]
pub(crate) struct RuntimeOverrideResolution {
    /// Rebuilt runner config after applying any runtime, model, or
    /// effort override. `None` means the runner row is byte-identical.
    pub effective: Option<Runner>,
    /// True when a non-blank runtime override was explicitly requested —
    /// including one matching the runner's current runtime. Spawn
    /// paths record the effective runtime on the session row for
    /// pinned spawns so a later edit to the runner template's
    /// runtime can't silently re-engine this session's resume (and
    /// hand its native session key to a different CLI).
    pub pinned: bool,
}

/// Resolve the runner config a spawn should actually use. Layering is
/// runner template, then runtime override, then model/effort overrides.
/// A matching runtime override keeps an otherwise unchanged spawn
/// byte-identical but still pins. Model/effort-only overrides never pin.
pub(crate) fn resolve_runtime_override(
    runner: &Runner,
    runtime_override: Option<&str>,
    model_override: Option<&str>,
    effort_override: Option<&str>,
) -> Result<RuntimeOverrideResolution> {
    let runtime_override = runtime_override.map(str::trim).filter(|s| !s.is_empty());
    let model_override = model_override
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let effort_override = effort_override
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let pinned = runtime_override.is_some();
    if runtime_override.is_none() && model_override.is_none() && effort_override.is_none() {
        return Ok(RuntimeOverrideResolution {
            effective: None,
            pinned: false,
        });
    }
    if runtime_override == Some(runner.runtime.as_str())
        && model_override.is_none()
        && effort_override.is_none()
    {
        return Ok(RuntimeOverrideResolution {
            effective: None,
            pinned: true,
        });
    }
    let mut effective = runner.clone();
    if let Some(name) = runtime_override.filter(|name| *name != runner.runtime.as_str()) {
        let def = router::runtime::runtime_definition(name)
            .ok_or_else(|| Error::msg(format!("unknown runtime: {name}")))?;
        effective.runtime = def.name.to_string();
        effective.command = def.command.to_string();
        effective.args = router::runtime::apply_permission_mode(
            def.name,
            &[],
            crate::ops::runner::default_permission_mode(),
        );
        // A differing engine starts from its own defaults; the
        // runner's model/effort belong to the original runtime.
        effective.model = None;
        effective.effort = None;
    }
    if model_override.is_some() {
        effective.model = model_override.map(ToOwned::to_owned);
    }
    if effort_override.is_some() {
        effective.effort = effort_override.map(ToOwned::to_owned);
    }
    Ok(RuntimeOverrideResolution {
        effective: Some(effective),
        pinned,
    })
}

pub(crate) fn runtime_direct_runner(
    runtime: &str,
    command: Option<&str>,
    model: Option<&str>,
    effort: Option<&str>,
) -> Result<Runner> {
    let runtime = runtime.trim();
    if runtime.is_empty() {
        return Err(Error::msg("runtime is required"));
    }
    let registry = router::runtime::runtime_definition(runtime);
    let command = command
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| registry.map(|r| r.command))
        .ok_or_else(|| Error::msg(format!("unknown runtime: {runtime}")))?;
    let now = Utc::now();
    Ok(Runner {
        id: format!("runtime:{runtime}"),
        handle: runtime.to_string(),
        display_name: registry
            .map(|r| r.display_name.to_string())
            .unwrap_or_else(|| runtime.to_string()),
        runtime: runtime.to_string(),
        command: command.to_string(),
        args: router::runtime::apply_permission_mode(
            runtime,
            &[],
            crate::ops::runner::default_permission_mode(),
        ),
        working_dir: None,
        system_prompt: None,
        env: HashMap::new(),
        model: model
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        effort: effort
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        created_at: now,
        updated_at: now,
    })
}

// The first-prompt readback machinery (FirstPromptConfig,
// FIRST_PROMPT_CONFIG, PLACEHOLDER_MIN_BODY_LEN) lived here before
// docs/impls/archive/0011 retired the verify-and-retry loop it tuned;
// `inject_paste` is now a single write-then-Enter and the previous
// "schedule continue on resume" auto-nudge has been removed — Resume
// now just respawns the PTY and lets the user drive the agent.

// Pre-#88 `inject_first_turn` (the paste-fallback orchestrator) was
// removed when first-turn delivery moved to spawn-time argv. The
// post-spawn auto-paste of "continue" on resume has also been removed
// — Resume now just respawns the PTY without injecting any stdin.

// `WORKER_COORDINATION_PREAMBLE` and the per-runtime first-turn
// composition helpers (`compose_worker_first_turn`,
// `compose_direct_first_turn`) live in `router::prompt`; the spawn
// paths here only decide how to hand that composed text to the CLI.

fn emit_runner_activity(pool: &DbPool, runner: &Runner, events: &dyn SessionEvents) {
    let Ok(conn) = pool.get() else { return };
    let active_sessions: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE runner_id = ?1 AND status = 'running'",
            params![runner.id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let active_missions: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT mission_id) FROM sessions
              WHERE runner_id = ?1 AND status = 'running' AND mission_id IS NOT NULL",
            params![runner.id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    // Count distinct crews this runner is wired into via the slots
    // table. Mirrors the cold-path query in
    // `ops::runner::runner_activity` so live `runner/activity`
    // events stay consistent with what the Runners list shows on a
    // refresh.
    let crew_count: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT crew_id) FROM slots WHERE runner_id = ?1",
            params![runner.id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let direct_session_id: Option<String> = conn
        .query_row(
            "SELECT id FROM sessions
              WHERE runner_id = ?1
                AND status = 'running'
                AND mission_id IS NULL
                AND slot_id IS NULL
                AND archived_at IS NULL
              ORDER BY started_at DESC
              LIMIT 1",
            params![runner.id],
            |r| r.get(0),
        )
        .ok();
    events.runner_activity(&RunnerActivityEvent {
        runner_id: runner.id.clone(),
        handle: runner.handle.clone(),
        active_sessions,
        active_missions,
        crew_count,
        direct_session_id,
    });
}
