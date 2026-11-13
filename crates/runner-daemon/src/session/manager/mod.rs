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

mod attach;
mod bookkeeping;
mod capture;
mod direct_spawn;
mod events;
mod fork;
mod lifecycle;
mod mission_spawn;
mod output;
mod resume;
mod shell;
mod spawn;
mod spawn_spec;
mod startup;
mod status;
pub mod terminal;
#[cfg(windows)]
pub(crate) mod windows_shutdown;

pub use events::*;
use events::{drop_streak_is_loggable, emit_role_activity, open_mission_event_log, AppendOutcome};
use shell::{is_terminal_shell, resolve_shell_resume_cwd};
use spawn::delete_session_row;
use spawn_spec::resolve_spawn_cwd;
pub(crate) use spawn_spec::{resolve_runtime_override, runtime_direct_role};

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

struct SessionHandle {
    startup: Option<startup::StartupDelivery>,
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
    rollout_capture: Option<RolloutCaptureContext>,
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
struct RolloutCaptureContext {
    runtime: String,
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
    pending_spawn: bool,
    delivery_runtime: String,
    turn_started: u64,
    /// The latest line the router typed, and whether the agent has reported a turn from it.
    delivery_line: Option<String>,
    delivery_prompted: bool,
    handle: Option<SessionHandle>,
    model: SessionModel,
    delivery_gate: Arc<DeliveryGate>,
    mission_status_sink: Option<ForwarderEmitCtx>,
    output_seq: u64,
    controller: terminal::Controller,
    terminal: Option<Arc<runner_terminal::terminal::TerminalModel>>,
    terminal_started_at: Option<terminal::TerminalStartedAt>,
    terminal_input: Option<std::sync::mpsc::Sender<Vec<u8>>>,
    terminal_input_worker: Option<thread::JoinHandle<()>>,
    subscribers: Vec<(u64, Arc<terminal::FrameQueue>)>,
    companion_terminals: Vec<Weak<terminal::CompanionTerminal>>,
    /// Latest grid measurement, including pushes that arrive before a PTY
    /// handle exists. Spawn and resume reconcile this under the state lock.
    last_requested_size: Option<(u16, u16)>,
    last_requested_size_dirty: bool,
    /// Present while resize persistence awaits settlement.
    pending_resize: Option<PendingResize>,
    resuming: bool,
    killed: bool,
    last_exit_was_killed: bool,
}

impl SessionState {
    fn delivery_readiness(
        &self,
        delivery: &DeliveryGateState,
        now: Instant,
    ) -> router::DeliveryReadiness {
        self.readiness_for(delivery, now, false)
    }

    /// A report waits for an idle session on every runtime, so it never lands
    /// in a turn the person started, and a draft holds it without a cap (#748).
    fn report_readiness(
        &self,
        delivery: &DeliveryGateState,
        now: Instant,
    ) -> router::DeliveryReadiness {
        self.readiness_for(delivery, now, true)
    }

    fn readiness_for(
        &self,
        delivery: &DeliveryGateState,
        now: Instant,
        report: bool,
    ) -> router::DeliveryReadiness {
        use router::{DeliveryReadiness as R, DeliveryReservation as D};
        if self.pending_spawn {
            return R::Pending;
        }
        let Some(handle) = &self.handle else {
            return R::Unavailable;
        };
        if handle
            .startup
            .as_ref()
            .is_some_and(|startup| report || !startup.reports_only)
        {
            return R::Starting;
        }
        if self.model.status().observation.needs_you() {
            return R::NeedsYou;
        }
        if delivery.in_flight || delivery.next_ticket != delivery.next_served {
            return R::InFlight;
        }
        let hold = if report {
            self.model.report_draft_hold(now)
        } else {
            self.model.draft_hold(now)
        };
        if let Some(hold) = hold {
            return match hold {
                D::RecentlyTyping(delay) => R::RecentlyTyping(delay),
                D::Unsent => R::Unsent,
                _ => R::Drafting,
            };
        }
        let idle = self.model.activity() == Some(SessionActivityState::Idle);
        if !idle
            && !report
            && runner_core::protocol::runtime_metadata::for_key(&self.delivery_runtime)
                .queues_input_mid_turn()
        {
            return R::Working;
        }
        if self.model.unconfirmed_busy() {
            return R::Unconfirmed;
        }
        R::Ready { idle }
    }

    fn is_empty(&self) -> bool {
        self.handle.is_none()
            && !self.pending_spawn
            && self.model.is_empty()
            && self.terminal.is_none()
            && self.terminal_input.is_none()
            && self.terminal_input_worker.is_none()
            && self.subscribers.is_empty()
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
    pub(crate) runtime_updates: Mutex<crate::ops::runtime_tasks::UpdateTasks>,
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
    launch_gate: Mutex<Option<Instant>>,
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
    hook_endpoint: RwLock<Option<runner_core::app_paths::IpcEndpoint>>,
    shutdown: RwLock<bool>,
    #[cfg(windows)]
    pub(crate) session_end: Arc<windows_shutdown::SessionEnd>,
    resize_settle_ms: AtomicU64,
    resize_generation: AtomicU64,
    terminal_palette: Mutex<runner_terminal::palette::TerminalPalette>,
    terminal_control: Mutex<terminal::ControlState>,
    #[cfg(test)]
    wake_baseline_read: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    #[cfg(test)]
    terminal_retention_read: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

/// RAII guard that releases a session state's `resuming` flag on drop. The
/// claim excludes Resume, Restart and removal until the guard drops.
pub(crate) struct ResumeClaim {
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
            let pending = manager
                .raw_session_state(&self.session_id)
                .is_some_and(|state| {
                    let mut state = state.lock().unwrap();
                    std::mem::take(&mut state.pending_spawn)
                });
            if pending {
                manager
                    .notify_delivery_event(&self.session_id, router::SessionDeliveryEvent::Exited);
            }
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
    pub(crate) launch_definition: crate::model::LaunchDefinition,
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
    first_turn: Option<String>,
    resolved_cwd: Option<String>,
    row_started_at: String,
    capture_prompt_marker: Option<String>,
    app_data_dir: PathBuf,
    pool: Arc<DbPool>,
}

impl SessionManager {
    pub(crate) fn set_hook_endpoint(&self, endpoint: runner_core::app_paths::IpcEndpoint) {
        *self.hook_endpoint.write().unwrap() = Some(endpoint);
    }

    pub(crate) fn admit_hook(
        &self,
        report: runner_core::protocol::HookReport,
    ) -> Result<super::hook_queue::Admission> {
        self.runtime.admit_hook(report)
    }

    pub fn new(
        shell_env: crate::runtime_status::SharedShellEnv,
        discovery_state: crate::runtime_status::SharedDiscoveryState,
        runtime: Arc<dyn SessionRuntime>,
    ) -> Arc<Self> {
        Arc::new(Self {
            runtime_updates: Mutex::default(),
            sessions: Mutex::new(HashMap::new()),
            delivery_listeners: Mutex::new(HashMap::new()),
            shell_env,
            discovery_state,
            launch_gate: Mutex::new(None),
            pending_mission_cancels: Mutex::new(HashMap::new()),
            runtime,
            hook_endpoint: RwLock::new(None),
            shutdown: RwLock::new(false),
            #[cfg(windows)]
            session_end: Arc::default(),
            resize_settle_ms: AtomicU64::new(RESIZE_SETTLE_MS),
            resize_generation: AtomicU64::new(0),
            terminal_palette: Mutex::new(runner_terminal::palette::RUNNER),
            terminal_control: Mutex::default(),
            #[cfg(test)]
            wake_baseline_read: Mutex::default(),
            #[cfg(test)]
            terminal_retention_read: Mutex::default(),
        })
    }

    pub fn start_runtime_watchers(
        self: &Arc<Self>,
        app_data_dir: &Path,
        pool: Arc<DbPool>,
        _events: Arc<dyn SessionEvents>,
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
                hooks.install_with_env(
                    app_data_dir,
                    &self
                        .shell_env
                        .read()
                        .expect("runtime shell environment lock poisoned")
                        .vars,
                );
            }
            adapter.cleanup_capture(app_data_dir, &pool);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn set_resize_settle_ms(&self, ms: u64) {
        self.resize_settle_ms.store(ms, Ordering::Relaxed);
    }
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

// `TEAM_COORDINATION_PREAMBLE` and the per-runtime first-turn
// composition helpers (`compose_member_first_turn`,
// `compose_direct_first_turn`) live in `router::prompt`; the spawn
// paths here only decide how to hand that composed text to the CLI.
