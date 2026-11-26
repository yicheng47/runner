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
mod input_writer;
mod lifecycle;
mod mission_spawn;
mod output;
mod resume;
mod shell;
mod spawn;
pub(crate) mod spawn_spec;
mod startup;
mod status;
pub mod terminal;
#[cfg(windows)]
pub(crate) mod windows_shutdown;

pub use direct_spawn::DirectLaunch;
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
    /// This process's PTY input; dropping the handle drops whatever it has not written.
    input: input_writer::InputQueue,
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
    stamp: u64,
    cols: u16,
    rows: u16,
    deadline: Instant,
    suppressed: u32,
    ioctl_count: u32,
    deferred_during_lifecycle: bool,
    pool: Arc<DbPool>,
}

pub(super) struct PendingResizePersist {
    state: Arc<Mutex<SessionState>>,
    stamp: u64,
    cols: u16,
    rows: u16,
    latest_stamp: Arc<AtomicU64>,
    persist_gate: Arc<Mutex<u64>>,
}

impl PendingResizePersist {
    fn is_current(&self) -> bool {
        self.latest_stamp.load(Ordering::Acquire) == self.stamp
    }

    pub(super) fn run(
        self,
        persist_size: impl FnOnce(u16, u16, &dyn Fn() -> bool) -> bool,
    ) -> bool {
        if !self.is_current() {
            return false;
        }
        let persisted = {
            let mut last_persisted = self.persist_gate.lock().unwrap();
            if *last_persisted >= self.stamp || !self.is_current() {
                return false;
            }
            let is_current = || self.is_current();
            let ok = persist_size(self.cols, self.rows, &is_current);
            if ok {
                *last_persisted = self.stamp;
            }
            ok
        };
        if persisted {
            let mut state = self.state.lock().unwrap();
            if state.current_resize_stamp() == self.stamp {
                state.last_requested_size_dirty = false;
            }
        }
        persisted
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OptimisticCompletion {
    pub(crate) prev_session_unread: Option<i64>,
    pub(crate) written_session_unread: i64,
    pub(crate) tab_id: Option<String>,
    pub(crate) prev_tab_completed_at: Option<String>,
    pub(crate) written_tab_completed_at: Option<String>,
}

#[derive(Default)]
struct SessionState {
    // Retain pre-attachment keys without exposing a runtime state that did not exist before.
    key_only: bool,
    pending_spawn: bool,
    delivery_runtime: String,
    turn_started: u64,
    #[cfg(all(test, unix))]
    program_turn_confirmed: u64,
    #[cfg(all(test, unix))]
    turn_notify: Arc<Condvar>,
    /// The latest line the router typed, and whether the agent has reported a turn from it.
    delivery_line: Option<String>,
    delivery_prompted: bool,
    handle: Option<SessionHandle>,
    last_launch: Option<Arc<AtomicBool>>,
    model: SessionModel,
    optimistic_completion: Option<OptimisticCompletion>,
    delivery_gate: Arc<DeliveryGate>,
    mission_status_sink: Option<ForwarderEmitCtx>,
    output_seq: u64,
    controller: terminal::Controller,
    terminal: Option<Arc<runner_terminal::terminal::TerminalModel>>,
    terminal_started_at: Option<terminal::TerminalStartedAt>,
    osc_title_tracker: Option<(String, Arc<Mutex<terminal::OscTitleTracker>>)>,
    terminal_input: Option<terminal::TerminalInput>,
    terminal_input_worker: Option<thread::JoinHandle<()>>,
    subscribers: Vec<terminal::Subscriber>,
    companion_terminals: Vec<Weak<terminal::CompanionTerminal>>,
    /// Latest grid measurement, including pushes that arrive before a PTY
    /// handle exists. Spawn and resume reconcile this under the state lock.
    last_requested_size: Option<(u16, u16)>,
    last_requested_cell_size: Option<(u16, u16)>,
    last_requested_size_dirty: bool,
    resize_stamp: Arc<AtomicU64>,
    resize_persist_gate: Arc<Mutex<u64>>,
    /// Present while resize persistence awaits settlement.
    pending_resize: Option<PendingResize>,
    resuming: bool,
    killed: bool,
    last_exit_was_killed: bool,
}

impl SessionState {
    pub(super) fn bump_resize_stamp(&mut self) -> u64 {
        self.resize_stamp
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1)
    }

    pub(super) fn current_resize_stamp(&self) -> u64 {
        self.resize_stamp.load(Ordering::Acquire)
    }

    pub(super) fn prepare_resize_persist(
        &self,
        state: &Arc<Mutex<SessionState>>,
        cols: u16,
        rows: u16,
        stamp: u64,
    ) -> PendingResizePersist {
        PendingResizePersist {
            state: Arc::clone(state),
            stamp,
            cols,
            rows,
            latest_stamp: Arc::clone(&self.resize_stamp),
            persist_gate: Arc::clone(&self.resize_persist_gate),
        }
    }
    fn delivery_readiness(
        &self,
        delivery: &DeliveryGateState,
        now: Instant,
    ) -> router::DeliveryReadiness {
        self.readiness_for(delivery, now, false)
    }

    /// A report waits for an idle session on every runtime, so it never lands
    /// in a turn the person started, and shares the screen-observed draft cap (#1409).
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
        // Preserve Claude Code's existing queueing during a synthetic Busy, but every newly
        // qualifying runtime must keep its unconfirmed-submit hold.
        if self.model.unconfirmed_busy()
            && (report || self.delivery_runtime != Runtime::ClaudeCode.key())
        {
            return R::Unconfirmed;
        }
        let idle = self.model.activity() == Some(SessionActivityState::Idle);
        if !idle
            && !report
            && runner_core::protocol::runtime_metadata::for_key(&self.delivery_runtime)
                .accepts_input_mid_turn()
        {
            return R::Working;
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
            && self.last_requested_cell_size.is_none()
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
    runtime_events: RwLock<Option<crate::events::EventChannel>>,
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
    input_deadline_ms: AtomicU64,
    terminal_palette: Mutex<runner_terminal::palette::TerminalPalette>,
    terminal_control: Mutex<terminal::ControlState>,
    control_group_revision: AtomicU64,
    last_desktop_cell_size: Mutex<Option<(u16, u16)>>,
    image_budget: Arc<runner_terminal::kitty::GlobalImageBudget>,
    kitty_staging_budget_bytes: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    max_staged_kitty_bytes: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    wake_baseline_read: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    #[cfg(test)]
    terminal_retention_read: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    live_session_scan_count: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    agy_summary_poll_count: AtomicU64,
    #[cfg(test)]
    agy_capture_exit_count: AtomicU64,
    #[cfg(test)]
    install_checkout_hooks: Mutex<HashMap<String, Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    after_install_snapshot_hooks: Mutex<HashMap<String, Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    forwarder_exit_hooks: Mutex<HashMap<String, Vec<std::sync::mpsc::Sender<()>>>>,
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

    pub fn set_runtime_events(&self, events: crate::events::EventChannel) {
        *self.runtime_events.write().unwrap() = Some(events);
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
        Self::new_with_budget(
            shell_env,
            discovery_state,
            runtime,
            Arc::new(runner_terminal::kitty::GlobalImageBudget::default()),
        )
    }

    #[cfg(test)]
    pub(crate) fn new_with_image_budget(
        shell_env: crate::runtime_status::SharedShellEnv,
        discovery_state: crate::runtime_status::SharedDiscoveryState,
        runtime: Arc<dyn SessionRuntime>,
        image_budget: Arc<runner_terminal::kitty::GlobalImageBudget>,
    ) -> Arc<Self> {
        Self::new_with_budget(shell_env, discovery_state, runtime, image_budget)
    }

    fn new_with_budget(
        shell_env: crate::runtime_status::SharedShellEnv,
        discovery_state: crate::runtime_status::SharedDiscoveryState,
        runtime: Arc<dyn SessionRuntime>,
        image_budget: Arc<runner_terminal::kitty::GlobalImageBudget>,
    ) -> Arc<Self> {
        Arc::new(Self {
            runtime_updates: Mutex::default(),
            sessions: Mutex::new(HashMap::new()),
            delivery_listeners: Mutex::new(HashMap::new()),
            shell_env,
            discovery_state,
            runtime_events: RwLock::new(None),
            launch_gate: Mutex::new(None),
            pending_mission_cancels: Mutex::new(HashMap::new()),
            runtime,
            hook_endpoint: RwLock::new(None),
            shutdown: RwLock::new(false),
            #[cfg(windows)]
            session_end: Arc::default(),
            resize_settle_ms: AtomicU64::new(RESIZE_SETTLE_MS),
            resize_generation: AtomicU64::new(0),
            input_deadline_ms: AtomicU64::new(input_writer::INPUT_WRITE_DEADLINE.as_millis() as u64),
            terminal_palette: Mutex::new(runner_terminal::palette::RUNNER),
            terminal_control: Mutex::default(),
            control_group_revision: AtomicU64::new(0),
            last_desktop_cell_size: Mutex::new(None),
            image_budget,
            kitty_staging_budget_bytes: std::sync::atomic::AtomicUsize::new(
                runner_terminal::kitty::MAX_NORMALIZED_FRAME_BYTES,
            ),
            #[cfg(test)]
            max_staged_kitty_bytes: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            wake_baseline_read: Mutex::default(),
            #[cfg(test)]
            terminal_retention_read: Mutex::default(),
            #[cfg(test)]
            live_session_scan_count: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            agy_summary_poll_count: AtomicU64::new(0),
            #[cfg(test)]
            agy_capture_exit_count: AtomicU64::new(0),
            #[cfg(test)]
            install_checkout_hooks: Mutex::default(),
            #[cfg(test)]
            after_install_snapshot_hooks: Mutex::default(),
            #[cfg(test)]
            forwarder_exit_hooks: Mutex::default(),
        })
    }

    #[cfg(test)]
    pub(crate) fn set_kitty_staging_budget(&self, bytes: usize) {
        self.kitty_staging_budget_bytes
            .store(bytes.max(1), Ordering::Release);
        self.max_staged_kitty_bytes.store(0, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn max_staged_kitty_bytes(&self) -> usize {
        self.max_staged_kitty_bytes.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn watch_forwarder_exit(&self, session_id: &str) -> std::sync::mpsc::Receiver<()> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.forwarder_exit_hooks
            .lock()
            .unwrap()
            .entry(session_id.to_owned())
            .or_default()
            .push(tx);
        rx
    }

    #[cfg(test)]
    pub(super) fn notify_forwarder_exit_done(&self, session_id: &str) {
        if let Some(waiters) = self.forwarder_exit_hooks.lock().unwrap().remove(session_id) {
            for tx in waiters {
                let _ = tx.send(());
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn set_install_checkout_hook(
        &self,
        session_id: &str,
        hook: impl FnOnce() + Send + 'static,
    ) {
        self.install_checkout_hooks
            .lock()
            .unwrap()
            .insert(session_id.to_owned(), Box::new(hook));
    }

    #[cfg(test)]
    pub(super) fn run_install_checkout_hook(&self, session_id: &str) {
        if let Some(hook) = self
            .install_checkout_hooks
            .lock()
            .unwrap()
            .remove(session_id)
        {
            hook();
        }
    }

    #[cfg(test)]
    pub(crate) fn set_after_install_snapshot_hook(
        &self,
        session_id: &str,
        hook: impl FnOnce() + Send + 'static,
    ) {
        self.after_install_snapshot_hooks
            .lock()
            .unwrap()
            .insert(session_id.to_owned(), Box::new(hook));
    }

    #[cfg(test)]
    pub(super) fn run_after_install_snapshot_hook(&self, session_id: &str) {
        if let Some(hook) = self
            .after_install_snapshot_hooks
            .lock()
            .unwrap()
            .remove(session_id)
        {
            hook();
        }
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

    /// Applies to the writers of handles that have not written input yet.
    #[cfg(test)]
    pub(crate) fn set_input_deadline(&self, deadline: Duration) {
        self.input_deadline_ms
            .store(deadline.as_millis() as u64, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn wait_for_queued_input(&self, session_id: &str, queued: usize) {
        self.test_input_writer(session_id).wait_until_queued(queued);
    }

    #[cfg(test)]
    pub(crate) fn wait_for_idle_input(&self, session_id: &str) {
        self.test_input_writer(session_id).wait_until_idle();
    }

    #[cfg(all(test, unix))]
    pub(crate) fn wait_for_turn_started(&self, session_id: &str, before: u64) -> u64 {
        let state = self.session_state(session_id).expect("session state");
        let notify = Arc::clone(&state.lock().unwrap().turn_notify);
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut guard = state.lock().unwrap();
        while guard.turn_started == before {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "turn_started for {session_id} stayed at {before}"
            );
            let (next, _) = notify.wait_timeout(guard, remaining).unwrap();
            guard = next;
        }
        guard.turn_started
    }

    #[cfg(all(test, unix))]
    pub(crate) fn program_turn_confirmed_count(&self, session_id: &str) -> u64 {
        self.session_state(session_id)
            .expect("session state")
            .lock()
            .unwrap()
            .program_turn_confirmed
    }

    #[cfg(all(test, unix))]
    pub(crate) fn wait_for_program_turn_confirmed(&self, session_id: &str, before: u64) -> u64 {
        let state = self.session_state(session_id).expect("session state");
        let notify = Arc::clone(&state.lock().unwrap().turn_notify);
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut guard = state.lock().unwrap();
        while guard.program_turn_confirmed == before {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "program_turn_confirmed for {session_id} stayed at {before}"
            );
            let (next, _) = notify.wait_timeout(guard, remaining).unwrap();
            guard = next;
        }
        guard.program_turn_confirmed
    }

    #[cfg(test)]
    fn test_input_writer(&self, session_id: &str) -> Arc<input_writer::InputWriter> {
        let state = self.session_state(session_id).expect("session state");
        let state = state.lock().unwrap();
        self.input_writer(state.handle.as_ref().expect("live session"))
    }

    fn input_writer(&self, handle: &SessionHandle) -> Arc<input_writer::InputWriter> {
        handle.input.writer(
            &self.runtime,
            &handle.runtime_session,
            Duration::from_millis(self.input_deadline_ms.load(Ordering::Relaxed)),
        )
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
