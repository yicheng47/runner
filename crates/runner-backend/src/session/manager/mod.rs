#[cfg(test)]
use super::status::AgentObservation;
use crate::session::status::{Activity, AgentStatus, ObservationSource, TurnOutcome};
// Per-role session manager.
//
// One `Session` = one child process attached to an in-process PTY via
// `SessionRuntime`. The SessionManager holds the map of live sessions so
// frontend and MCP operations can look them up by id (for stdin injection,
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

use crate::model::Runtime;
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
use crate::model::{Mission, Role};
use crate::router;
use crate::session::runtime::{
    OutputStream, RuntimeOutput, RuntimeSession, SessionRuntime, SpawnSpec,
};

mod lifecycle;
mod output;
mod spawn;

#[cfg(test)]
use super::state::ObservedInput;
pub use super::state::{InputObservation, InputState, StatusSource};
use super::state::{KeyOrigin, PersistKey, SessionEvent, SessionModel};
#[cfg(test)]
use super::status::Lifecycle;
pub(crate) use output::{classify_local_input, LocalInputClass};
#[cfg(test)]
const RECENT_LOCAL_INPUT_WINDOW: Duration = Duration::from_secs(2);

#[cfg(test)]
mod tests;

// Router delivery normally holds the gate for its 80 ms submit delay. Five
// seconds also clears the 500 ms input-flush grace comfortably under load.
const DIRECT_INPUT_GATE_TIMEOUT: Duration = Duration::from_secs(5);
const RUNNER_STATUS_APPEND_MAX_ATTEMPTS: usize = 8;
const RUNNER_STATUS_APPEND_RETRY_DELAY: Duration = Duration::from_millis(5);

pub(crate) const DEFAULT_PTY_SIZE: (u16, u16) = (80, 24);

/// Trailing debounce for width-changing full-repaint TUI resizes.
const RESIZE_SETTLE_MS: u64 = 175;

/// Inputs the forwarder consumer needs to translate a
/// `RuntimeOutput::StatusTransition` into a real `session_status`
/// event on the mission's NDJSON log (issue #124). All fields are
/// correlated — a mission spawn has all of them; a direct chat has
/// none — so they live together in one optional struct. The
/// forwarder consumer carries an `Option<Self>`: `Some` for mission
/// sessions, `None` for direct chats. See
/// `docs/features/archive/13-pty-silence-idle-detection.md` §Scope for why
/// direct chats are skipped.
///
/// The `EventLog` handle is opened once at construction (on the caller's
/// thread, where a brief blocking flock during tail
/// repair is fine) and cached so the forwarder consumer thread's
/// hot path never calls `EventLog::open` — that path takes a
/// blocking flock to repair any dangling tail, and the forwarder
/// thread also drains terminal output and exit events through the
/// same channel; blocking it would freeze them.
#[derive(Clone)]
pub(crate) struct ForwarderEmitCtx {
    /// `mission.crew_id` — needed for the `EventDraft.crew_id`
    /// field so the appended row matches the event-log schema.
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
    fn session_status_draft(
        &self,
        state: SessionActivityState,
        source: StatusSource,
        status: &AgentStatus,
    ) -> EventDraft {
        let state_str = match state {
            SessionActivityState::Busy => "busy",
            SessionActivityState::Idle => "idle",
        };
        EventDraft::signal(
            self.crew_id.clone(),
            self.mission_id.clone(),
            self.handle.clone(),
            SignalType::new("session_status"),
            serde_json::json!({ "state": state_str, "source": source, "status": status }),
        )
    }

    /// Non-blocking append of a forwarder-emitted `session_status`
    /// row. The consumer thread runs this on every status
    /// transition; it must not block (it shares the mpsc receiver
    /// with the terminal output stream and the exit-event reap, so
    /// a stuck flock would freeze them too). Wire shape mirrors
    /// `crates/runner-cli/src/signal.rs::run_status` so router / UI projections
    /// can't tell the two apart except by `payload.source`.
    fn try_append_session_status(
        &self,
        state: SessionActivityState,
        source: StatusSource,
        status: &AgentStatus,
    ) -> AppendOutcome {
        match self.try_append_with_retry(self.session_status_draft(state, source, status)) {
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

    fn append_session_status(
        &self,
        state: SessionActivityState,
        source: StatusSource,
        status: &AgentStatus,
    ) -> runner_core::Result<()> {
        self.event_log
            .append(self.session_status_draft(state, source, status))
            .map(|_| ())
    }
}

/// Streak indices at which the forwarder consumer logs a WARN about
/// dropped `session_status` events. Picked to cover the common
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
    fn fork_started(&self, _ev: &SessionForkStartedEvent) {}
    fn exit(&self, ev: &ExitEvent);
    fn archived(&self, _ev: &SessionUpdatedEvent) {}
    /// Persisted session metadata changed without a lifecycle event
    /// (e.g. async agent_session_key capture). Default no-op so test
    /// fakes don't have to opt in.
    fn updated(&self, _ev: &SessionUpdatedEvent) {}
    /// Live direct-chat activity projection. Mission sessions keep using
    /// `session_status` rows in the mission log instead.
    fn status(&self, _ev: &SessionActivityEvent) {}
    /// Live activity counter for a role — emitted on every spawn/reap so
    /// the Roles list can update its "N sessions / M missions" badges
    /// without polling. Default no-op so test fakes don't have to opt in.
    fn role_activity(&self, _ev: &RoleActivityEvent) {}
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

/// Payload for `role/activity`. Derived from the same query as the role
/// activity operation, so a fresh page load and a live update agree.
#[derive(Debug, Clone, Serialize)]
pub struct RoleActivityEvent {
    pub role_id: String,
    pub handle: String,
    pub active_sessions: i64,
    pub active_missions: i64,
    pub crew_count: i64,
    /// Most recent running direct-chat session id, if any. Mirrors
    /// `RoleActivity::direct_session_id` so the sidebar can re-attach
    /// to a live PTY without an extra round-trip.
    pub direct_session_id: Option<String>,
}

pub use crate::session::runtime::SessionActivityState;

/// Payload for `session/status`. Shared by direct chats and mission sessions, where
/// busy/idle is a live UI projection rather than persisted DB state.
#[derive(Debug, Clone, Serialize)]
pub struct SessionActivityEvent {
    pub session_id: String,
    pub state: SessionActivityState,
    pub source: StatusSource,
    pub status: AgentStatus,
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
    fn fork_started(&self, ev: &SessionForkStartedEvent) {
        self.events.emit("session/fork-started", ev);
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
        if ev.state == SessionActivityState::Idle
            && ev.status.observation.outcome != Some(TurnOutcome::Interrupted)
            && ev.status.observation.outcome != Some(TurnOutcome::Failed)
        {
            if let Some(sessions) = self.sessions.upgrade() {
                let completion_eligible = matches!(
                    (ev.status.observation.activity, ev.status.observation.source),
                    (Activity::Idle, ObservationSource::Baseline) | (Activity::Ready, _)
                );
                if completion_eligible {
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
        }
        let mut event = ev.clone();
        if let Some(sessions) = self.sessions.upgrade() {
            event.status = sessions.agent_status(&ev.session_id);
        }
        self.events.emit("session/status", &event);
    }
    fn role_activity(&self, ev: &RoleActivityEvent) {
        self.events.emit("role/activity", ev);
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

#[derive(Debug, Clone, Serialize, Eq, PartialEq)]
pub struct SessionForkStartedEvent {
    pub source_session_id: String,
    pub session_id: String,
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
/// row with the role handle denormalized so the debug page can render
/// `@coder`-style labels without a separate lookup.
#[derive(Debug, Clone, Serialize)]
pub struct SpawnedSession {
    pub id: String,
    pub mission_id: Option<String>,
    pub role_id: Option<String>,
    pub handle: String,
    pub pid: Option<u32>,
}

struct SessionHandle {
    #[cfg(windows)]
    pending_first_turn: Option<PendingFirstTurn>,
    id: String,
    /// `None` for direct-chat sessions (C8.5). `kill_all_for_mission`
    /// filters on this so direct chats don't get torn down when a mission
    /// stops, and vice versa.
    mission_id: Option<String>,
    /// The role this session is an instance of. `kill_all_for_role`
    /// filters on this so deleting a role can reap its live PTY
    /// children before the role delete removes the DB rows underneath.
    role_id: Option<String>,
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

#[cfg(windows)]
struct PendingFirstTurn {
    body: String,
    deadline: Instant,
    readiness: super::runtime::TuiReadiness,
}

#[cfg(all(windows, not(test)))]
const WINDOWS_FIRST_TURN_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(all(windows, test))]
const WINDOWS_FIRST_TURN_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone)]
struct CodexCaptureContext {
    manager: Weak<SessionManager>,
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
    // Retain pre-attachment keys without exposing a runtime state that did not exist before.
    key_only: bool,
    handle: Option<SessionHandle>,
    model: SessionModel,
    delivery_gate: Arc<DeliveryGate>,
    mission_status_sink: Option<ForwarderEmitCtx>,
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
            && self.model.is_empty()
            && self.mission_status_sink.is_none()
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
    /// pruning and revealing pre-attachment state lock the state before the map.
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
    /// under `role.env` so the child can reach the network the same
    /// way Terminal.app's children would (issues #109 / #152).
    shell_env: Arc<RwLock<crate::shell_path::LoginShellEnv>>,
    discovery_state: crate::runtime_status::SharedDiscoveryState,
    /// Timestamp of the most recent claude-code spawn through the
    /// launch gate. `None` until the first claude-code spawn lands.
    /// Each new claude-code spawn reads this, sleeps the remainder
    /// of the adapter launch grace, then updates it. Non-claude
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
    claude_session_key_watcher: Mutex<Option<super::claude_rekey::ClaudeSessionKeyWatcher>>,
    /// Underlying terminal runtime. Every spawn / resume / kill /
    /// inject_stdin / resize routes through this trait — the manager
    /// owns DB + event-buffer state but never reads/writes a PTY
    /// directly.
    runtime: Arc<dyn SessionRuntime>,
    resize_settle_ms: AtomicU64,
    resize_generation: AtomicU64,
    /// App-wide permission mode for mission slots (feature 527). The
    /// GPUI settings store pushes the current value here; every
    /// mission spawn and resume reads it, so the MCP `mission_start`
    /// and the Start Mission modal converge on the same flags.
    mission_permission_mode: RwLock<router::runtime::MissionPermissionMode>,
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

struct PreAttachmentState {
    manager: Weak<SessionManager>,
    session_id: String,
}

impl Drop for PreAttachmentState {
    fn drop(&mut self) {
        if let Some(manager) = self.manager.upgrade() {
            manager.prune_key_only_session_state(&self.session_id);
        }
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
/// router/bus mount synchronously and return to the GPUI task in
/// ~milliseconds, then drive the slow PTY-spawn phase in a
/// background task. Without the split, the modal Start button
/// blocks ~1500ms per claude-code worker (gate cost) before the
/// workspace loads. See issue #171.
///
/// All fields are owned (clones / Arcs) so the value can travel
/// across thread boundaries into a `spawn_blocking` task.
pub struct PendingMissionSpawn {
    pub session_id: String,
    pre_attachment: PreAttachmentState,
    spec: SpawnSpec,
    mission: Mission,
    role: Role,
    slot_handle: String,
    /// Where `spec.initial_size` came from (caller-supplied / mission-hint /
    /// DEFAULT_PTY_SIZE), for the post-spawn fork log line (#366).
    size_source: &'static str,
    plan: router::runtime::ResumePlan,
    first_turn_delivered_via_argv: bool,
    #[cfg(windows)]
    first_turn: Option<String>,
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
            claude_session_key_watcher: Mutex::new(None),
            runtime,
            resize_settle_ms: AtomicU64::new(RESIZE_SETTLE_MS),
            resize_generation: AtomicU64::new(0),
            mission_permission_mode: RwLock::new(router::runtime::MissionPermissionMode::default()),
        })
    }

    pub fn set_mission_permission_mode(&self, mode: router::runtime::MissionPermissionMode) {
        *self.mission_permission_mode.write().unwrap() = mode;
    }

    pub fn mission_permission_mode(&self) -> router::runtime::MissionPermissionMode {
        *self.mission_permission_mode.read().unwrap()
    }

    pub fn start_runtime_watchers(
        self: &Arc<Self>,
        app_data_dir: &Path,
        pool: Arc<DbPool>,
        events: Arc<dyn SessionEvents>,
    ) -> Result<()> {
        for runtime in Runtime::ALL {
            if let Some(hooks) = crate::runtimes::adapter(runtime).status_hooks() {
                hooks.cleanup(app_data_dir);
            }
        }
        if let Err(error) = super::system_prompt::clear_leftovers(app_data_dir) {
            log::warn!("clear stale session prompt files: {error}");
        }
        for runtime in Runtime::ALL {
            let adapter = crate::runtimes::adapter(runtime);
            if let Some(hooks) = adapter.status_hooks() {
                hooks.install(app_data_dir);
            }
            if matches!(adapter.key_capture(), crate::runtimes::KeyCapture::LogTail) {
                crate::runtimes::antigravity::agy_capture::clear_orphans(app_data_dir, &pool);
            }
        }
        let watcher = super::claude_rekey::ClaudeSessionKeyWatcher::start(
            app_data_dir,
            pool,
            events,
            Arc::downgrade(self),
        )?;
        *self.claude_session_key_watcher.lock().unwrap() = Some(watcher);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn set_resize_settle_ms(&self, ms: u64) {
        self.resize_settle_ms.store(ms, Ordering::Relaxed);
    }

    fn session_state(&self, session_id: &str) -> Option<Arc<Mutex<SessionState>>> {
        let session = self.raw_session_state(session_id)?;
        let key_only = session.lock().unwrap().key_only;
        (!key_only).then_some(session)
    }

    fn raw_session_state(&self, session_id: &str) -> Option<Arc<Mutex<SessionState>>> {
        self.sessions.lock().unwrap().get(session_id).cloned()
    }

    fn session_state_or_insert(&self, session_id: &str) -> Arc<Mutex<SessionState>> {
        loop {
            let session = self
                .sessions
                .lock()
                .unwrap()
                .entry(session_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(SessionState::default())))
                .clone();
            let mut state = session.lock().unwrap();
            if state.key_only {
                let sessions = self.sessions.lock().unwrap();
                if !sessions
                    .get(session_id)
                    .is_some_and(|current| Arc::ptr_eq(current, &session))
                {
                    continue;
                }
                state.key_only = false;
            }
            drop(state);
            return session;
        }
    }

    pub(crate) fn report_key(
        &self,
        session_id: &str,
        report: PersistKey,
        persist: impl FnOnce(&PersistKey) -> Result<bool>,
    ) -> Result<bool> {
        let session = self
            .sessions
            .lock()
            .unwrap()
            .entry(session_id.to_owned())
            .or_insert_with(|| {
                Arc::new(Mutex::new(SessionState {
                    key_only: true,
                    ..Default::default()
                }))
            })
            .clone();
        let effect = session
            .lock()
            .unwrap()
            .model
            .apply(
                SessionEvent::ConversationChanged(report),
                super::clock::state_now(),
            )
            .persist_key
            .unwrap();
        let written = persist(&effect);
        if matches!(written, Ok(true)) {
            session.lock().unwrap().model.apply(
                SessionEvent::KeyPersisted(effect),
                super::clock::state_now(),
            );
        } else {
            self.prune_key_only_session_state(session_id);
        }
        written
    }

    pub(crate) fn write_key_effect(
        conn: &rusqlite::Connection,
        session_id: &str,
        effect: &PersistKey,
    ) -> Result<bool> {
        match effect.origin {
            KeyOrigin::Assigned => Ok(conn.execute(
                "UPDATE sessions SET agent_session_key = ?1 WHERE id = ?2",
                params![effect.key, session_id],
            )? > 0),
            KeyOrigin::Captured => Ok(crate::repo::session::capture_agent_session_key(
                conn,
                session_id,
                effect.key.as_deref().expect("capture reports a key"),
                &effect.generation,
            )?),
            KeyOrigin::Rekeyed => Ok(crate::repo::session::rekey_agent_session_key(
                conn,
                session_id,
                effect.key.as_deref().expect("rekey reports a key"),
                &effect.generation,
            )?),
        }
    }

    pub(crate) fn persist_reported_key(
        &self,
        session_id: &str,
        report: PersistKey,
        pool: &DbPool,
    ) -> Result<bool> {
        self.report_key(session_id, report, |effect| {
            let conn = pool.get()?;
            Self::write_key_effect(&conn, session_id, effect)
        })
    }

    #[cfg(test)]
    pub(crate) fn key_test_manager() -> Arc<Self> {
        Self::new(
            Arc::new(RwLock::new(crate::shell_path::LoginShellEnv::default())),
            Arc::new(RwLock::new(crate::shell_path::DiscoveryState::pending())),
            Arc::new(super::pty_runtime::PtyRuntime::new()),
        )
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
            && !session.model.status().observation.needs_you()
            && !delivery.in_flight
            && delivery.next_ticket == delivery.next_served
            && session.model.draft_quiescent(crate::session::clock::now())
    }

    /// Ids of every session whose process is attached right now.
    pub fn live_session_ids(&self) -> Vec<String> {
        let sessions: Vec<_> = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .map(|(id, session)| (id.clone(), Arc::clone(session)))
            .collect();
        sessions
            .into_iter()
            .filter(|(_, session)| session.lock().unwrap().handle.is_some())
            .map(|(id, _)| id)
            .collect()
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
        if session.model.status().observation.needs_you() {
            return Ok(router::DeliveryReservation::HumanInteraction);
        }
        if delivery.in_flight {
            return Ok(router::DeliveryReservation::InFlight);
        }
        if delivery.next_ticket != delivery.next_served {
            return Ok(router::DeliveryReservation::InFlight);
        }
        if let Some(hold) = session.model.draft_hold(crate::session::clock::now()) {
            return Ok(hold);
        }
        delivery.in_flight = true;
        Ok(router::DeliveryReservation::Ready(delivery.generation))
    }

    pub fn report_input_state(&self, session_id: &str, observation: InputObservation) {
        let Some(session) = self.session_state(session_id) else {
            return;
        };
        let input_cleared = {
            let mut session = session.lock().unwrap();
            if session.handle.is_none() {
                return;
            }
            session
                .model
                .apply(
                    SessionEvent::Composer {
                        observation,
                        live: true,
                    },
                    super::clock::state_now(),
                )
                .input_cleared
        };
        if input_cleared {
            self.notify_delivery_event(session_id, router::SessionDeliveryEvent::InputCleared);
        }
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
        self.prune_session_state(session_id, false);
    }

    fn prune_key_only_session_state(&self, session_id: &str) {
        self.prune_session_state(session_id, true);
    }

    fn prune_session_state(&self, session_id: &str, key_only: bool) {
        let Some(state) = self.raw_session_state(session_id) else {
            return;
        };
        let state_guard = state.lock().unwrap();
        if !state_guard.is_empty() || (key_only && !state_guard.key_only) {
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
        self.install_handle_with_size_persistence(
            session_id,
            handle,
            mission_status_sink,
            initial_size,
            |cols, rows| match pool.get() {
                Ok(conn) => match crate::repo::session::update_last_size(&conn, session_id, cols, rows) {
                    Ok(_) => true,
                    Err(error) => {
                        log::warn!("resize persistence after handle install failed: session={session_id} {cols}x{rows}: {error}");
                        false
                    }
                },
                Err(error) => {
                    log::warn!("resize persistence after handle install pool checkout failed: session={session_id} {cols}x{rows}: {error}");
                    false
                }
            },
            events,
        );
    }

    fn install_handle_with_size_persistence(
        &self,
        session_id: &str,
        handle: SessionHandle,
        mission_status_sink: Option<ForwarderEmitCtx>,
        initial_size: Option<(u16, u16)>,
        persist_size: impl FnOnce(u16, u16) -> bool,
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
            state.handle = Some(handle);
            state
                .model
                .apply(SessionEvent::Attached, super::clock::state_now());
            state.mission_status_sink = mission_status_sink;
            state.killed = false;

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
                    if persist_size(cols, rows) {
                        state.last_requested_size_dirty = false;
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
        source: StatusSource,
    ) -> bool {
        let session = self.session_state_or_insert(session_id);
        let mut session = session.lock().unwrap();
        let live = session.handle.is_some() && !session.killed;
        session
            .model
            .apply(
                SessionEvent::Transition {
                    state,
                    source,
                    live,
                },
                super::clock::state_now(),
            )
            .publication
            .is_some()
    }

    fn note_terminal_event(&self, session_id: &str, event: crate::runtimes::TerminalEvent) -> bool {
        use crate::runtimes::TerminalEvent;
        let session = self.session_state_or_insert(session_id);
        let mut session = session.lock().unwrap();
        let live = session.handle.is_some() && !session.killed;
        let event = match event {
            TerminalEvent::Activity(state) => SessionEvent::Transition {
                state,
                source: StatusSource::Forwarder,
                live,
            },
            TerminalEvent::Title(state) => SessionEvent::Title { state, live },
            TerminalEvent::Ready(state) => SessionEvent::Readiness { state, live },
        };
        session
            .model
            .apply(event, super::clock::state_now())
            .publication
            .is_some()
    }

    fn publish_direct_terminal_event(
        &self,
        session_id: &str,
        event: crate::runtimes::TerminalEvent,
        events: &dyn SessionEvents,
    ) {
        if !self.note_terminal_event(session_id, event) {
            return;
        }
        events.status(&SessionActivityEvent {
            session_id: session_id.to_owned(),
            state: event.state(),
            source: StatusSource::Forwarder,
            status: self.agent_status(session_id),
        });
    }

    #[cfg(test)]
    pub(crate) fn publish_mission_terminal_event(
        &self,
        session_id: &str,
        event: crate::runtimes::TerminalEvent,
        events: &dyn SessionEvents,
    ) {
        if !self.note_terminal_event(session_id, event) {
            return;
        }
        let (status, sink) = {
            let session = self.session_state(session_id).unwrap();
            let session = session.lock().unwrap();
            (
                session.model.status().clone(),
                session.mission_status_sink.clone(),
            )
        };
        events.status(&SessionActivityEvent {
            session_id: session_id.to_owned(),
            state: event.state(),
            source: StatusSource::Forwarder,
            status: status.clone(),
        });
        if let Some(sink) = sink {
            if let Err(error) =
                sink.append_session_status(event.state(), StatusSource::Forwarder, &status)
            {
                log::warn!("append spawn session_status failed for {session_id}: {error}");
            }
        }
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
            (sink, session.model.revision())
        };
        match sink.try_append_with_retry(draft) {
            Ok(()) => {}
            Err(TryAppendError::Contended) => return Err(Error::msg("event log busy")),
            Err(TryAppendError::Failed(error)) => return Err(error.into()),
        }
        let mut session = session.lock().unwrap();
        session.model.apply(
            SessionEvent::Delivered {
                revision: activity_revision,
            },
            super::clock::state_now(),
        );
        Ok(())
    }

    pub(crate) fn publish_direct_activity(
        &self,
        session_id: &str,
        state: SessionActivityState,
        source: StatusSource,
        events: &dyn SessionEvents,
    ) {
        if !self.note_forwarder_transition(session_id, state, source) {
            return;
        }
        events.status(&SessionActivityEvent {
            session_id: session_id.to_string(),
            state,
            source,
            status: self.agent_status(session_id),
        });
    }

    pub(crate) fn publish_mission_activity(
        &self,
        session_id: &str,
        state: SessionActivityState,
        source: StatusSource,
        events: &dyn SessionEvents,
    ) {
        if !self.note_forwarder_transition(session_id, state, source) {
            return;
        }
        let (status, sink) = {
            let session = self.session_state(session_id).unwrap();
            let session = session.lock().unwrap();
            (
                session.model.status().clone(),
                session.mission_status_sink.clone(),
            )
        };
        events.status(&SessionActivityEvent {
            session_id: session_id.to_string(),
            state,
            source,
            status: status.clone(),
        });
        if let Some(sink) = sink {
            if let Err(error) = sink.append_session_status(state, source, &status) {
                log::warn!("append spawn session_status failed for {session_id}: {error}");
            }
        }
    }

    pub(crate) fn arm_completion(&self, session_id: &str) {
        self.session_state_or_insert(session_id)
            .lock()
            .unwrap()
            .model
            .apply(SessionEvent::ArmCompletion, super::clock::state_now());
    }

    pub(crate) fn take_completion_armed(&self, session_ids: &[String]) -> bool {
        let sessions: Vec<_> = {
            let sessions = self.sessions.lock().unwrap();
            session_ids
                .iter()
                .filter_map(|id| sessions.get(id).cloned())
                .collect()
        };
        let mut armed = false;
        for session in sessions {
            armed |= session
                .lock()
                .unwrap()
                .model
                .apply(SessionEvent::TakeCompletion, super::clock::state_now())
                .completion_consumed;
        }
        armed
    }

    pub fn mark_status_viewed(&self, session_ids: &[String]) {
        for id in session_ids {
            if let Some(session) = self.session_state(id) {
                session
                    .lock()
                    .unwrap()
                    .model
                    .apply(SessionEvent::Viewed, super::clock::state_now());
            }
        }
    }

    pub(crate) fn record_unread(&self, session_id: &str, viewed: bool) {
        if let Some(session) = self.session_state(session_id) {
            session
                .lock()
                .unwrap()
                .model
                .apply(SessionEvent::Unread { viewed }, super::clock::state_now());
        }
    }

    fn record_exit_status(&self, session_id: &str, exit_code: Option<i32>, crashed: bool) {
        if let Some(session) = self.session_state(session_id) {
            session.lock().unwrap().model.apply(
                SessionEvent::Exited {
                    code: exit_code,
                    crashed,
                },
                super::clock::state_now(),
            );
        }
    }

    pub fn agent_status(&self, session_id: &str) -> AgentStatus {
        self.session_state(session_id)
            .map(|session| session.lock().unwrap().model.status().clone())
            .unwrap_or_default()
    }

    pub fn status_snapshot(&self) -> BTreeMap<String, AgentStatus> {
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
                let session = session.lock().unwrap();
                (!session.key_only).then(|| (id, session.model.status().clone()))
            })
            .collect()
    }

    fn publish_agent_event(
        &self,
        session_id: &str,
        event: super::state::agent::AgentEvent,
        events: &dyn SessionEvents,
    ) -> super::state::agent::AdapterFeedback {
        self.publish_model_event(
            session_id,
            SessionEvent::Agent { event, live: true },
            events,
        )
    }

    fn publish_model_event(
        &self,
        session_id: &str,
        mut event: SessionEvent,
        events: &dyn SessionEvents,
    ) -> super::state::agent::AdapterFeedback {
        let Some(session) = self.session_state(session_id) else {
            return Default::default();
        };
        let (status, sink, effects) = {
            let mut session = session.lock().unwrap();
            let is_live = session.handle.is_some() && !session.killed;
            match &mut event {
                SessionEvent::Agent { live, .. } | SessionEvent::BridgeFailed { live } => {
                    *live = is_live
                }
                _ => {}
            }
            let effects = session.model.apply(event, super::clock::state_now());
            (
                session.model.status().clone(),
                session.mission_status_sink.clone(),
                effects,
            )
        };
        if let Some((state, source)) = effects.publication {
            events.status(&SessionActivityEvent {
                session_id: session_id.to_owned(),
                state,
                source,
                status,
            });
            let status = self.agent_status(session_id);
            if let Some(sink) = sink {
                let draft = sink.session_status_draft(state, source, &status);
                if let Err(error) = sink.try_append_with_retry(draft) {
                    log::warn!("publish hook observation: {error:?}");
                }
            }
            if effects.input_cleared {
                self.notify_delivery_event(session_id, router::SessionDeliveryEvent::InputCleared);
            }
        }
        effects.agent_feedback
    }

    #[cfg(test)]
    fn publish_observation(
        &self,
        session_id: &str,
        observation: AgentObservation,
        events: &dyn SessionEvents,
    ) {
        self.publish_agent_event(
            session_id,
            super::state::agent::AgentEvent::Published(observation),
            events,
        );
    }

    fn status_bridge_failed(&self, session_id: &str, events: &dyn SessionEvents) {
        self.publish_model_event(
            session_id,
            SessionEvent::BridgeFailed { live: true },
            events,
        );
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
                    .model
                    .activity()
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
                manager: ctx.manager.clone(),
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
}

fn resolve_spawn_cwd(explicit: Option<&str>, role_default: Option<&str>) -> Option<String> {
    [explicit, role_default]
        .into_iter()
        .flatten()
        .find(|cwd| !cwd.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| {
            runner_core::app_paths::home_dir()
                .and_then(|home| home.into_os_string().into_string().ok())
        })
}

/// Outcome of resolving a runtime override against a role row
/// (feature 41).
#[derive(Debug)]
pub(crate) struct RuntimeOverrideResolution {
    /// Rebuilt role config after applying any runtime, model, or
    /// effort override. `None` means the role row is byte-identical.
    pub effective: Option<Role>,
    /// True when a non-blank runtime override was explicitly requested —
    /// including one matching the role's current runtime. Spawn
    /// paths record the effective runtime on the session row for
    /// pinned spawns so a later edit to the role template's
    /// runtime can't silently re-engine this session's resume (and
    /// hand its native session key to a different CLI).
    pub pinned: bool,
}

/// Resolve the role config a spawn should actually use. Layering is
/// role template, then runtime override, then model/effort overrides.
/// A matching runtime override keeps an otherwise unchanged spawn
/// byte-identical but still pins. Model/effort-only overrides never pin.
pub(crate) fn resolve_runtime_override(
    role: &Role,
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
    if runtime_override == Some(role.runtime.as_str())
        && model_override.is_none()
        && effort_override.is_none()
    {
        return Ok(RuntimeOverrideResolution {
            effective: None,
            pinned: true,
        });
    }
    let mut effective = role.clone();
    if let Some(name) = runtime_override.filter(|name| *name != role.runtime.as_str()) {
        let def = crate::runtimes::for_key(name)
            .catalog()
            .ok_or_else(|| Error::msg(format!("unknown runtime: {name}")))?;
        effective.runtime = def.name.to_string();
        effective.command = def.command.to_string();
        effective.args = crate::runtimes::adapter(def.name)
            .permissions()
            .apply(&[], crate::ops::role::default_permission_mode());
        // A differing engine starts from its own defaults; the
        // role's model/effort belong to the original runtime.
        effective.model = None;
        effective.effort = None;
        effective.codex_speed = None;
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

pub(crate) fn runtime_direct_role(
    runtime: &str,
    command: Option<&str>,
    model: Option<&str>,
    effort: Option<&str>,
) -> Result<Role> {
    let runtime = runtime.trim();
    if runtime.is_empty() {
        return Err(Error::msg("runtime is required"));
    }
    let registry = crate::runtimes::for_key(runtime).catalog();
    let command = command
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| registry.as_ref().map(|r| r.command))
        .ok_or_else(|| Error::msg(format!("unknown runtime: {runtime}")))?;
    let now = Utc::now();
    let args = if Runtime::parse(runtime).is_some_and(Runtime::is_shell) {
        crate::shell_path::shell_login_args(command)
    } else {
        crate::runtimes::for_key(runtime)
            .permissions()
            .apply(&[], crate::ops::role::default_permission_mode())
    };
    Ok(Role {
        id: format!("runtime:{runtime}"),
        handle: runtime.to_string(),
        display_name: registry
            .map(|r| r.display_name.to_string())
            .unwrap_or_else(|| runtime.to_string()),
        runtime: runtime.to_string(),
        command: command.to_string(),
        args,
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
        codex_speed: None,
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

fn emit_role_activity(pool: &DbPool, role: &Role, events: &dyn SessionEvents) {
    let Ok(conn) = pool.get() else { return };
    let activity = crate::repo::role::activity(&conn, &role.id).unwrap_or_default();
    // Count distinct crews this role is wired into via the slots
    // table. Mirrors the cold-path query in
    // `ops::role::role_activity` so live `role/activity`
    // events stay consistent with what the Roles list shows on a
    // refresh.
    events.role_activity(&RoleActivityEvent {
        role_id: role.id.clone(),
        handle: role.handle.clone(),
        active_sessions: activity.active_sessions,
        active_missions: activity.active_missions,
        crew_count: activity.crew_count,
        direct_session_id: activity.direct_session_id,
    });
}
