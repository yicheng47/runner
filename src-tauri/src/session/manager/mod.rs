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
//     `session/output` Tauri events. When the channel closes, the
//     thread queries the runtime for final exit code, emits
//     `session/exit`, and updates the DB row.
//
// At app restart, in-process PTYs are gone with the prior app process.
// Startup cleanup demotes stale running DB rows to stopped; user-facing
// resume respawns a fresh PTY with the same session row id.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
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

const MAX_OUTPUT_BUFFER_CHUNKS: usize = 4096;

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

fn scan_mode_transition(bytes: &[u8], patterns: &[(&[u8], bool)]) -> Option<bool> {
    let mut latest: Option<(usize, bool)> = None;
    for (needle, state) in patterns {
        if bytes.len() < needle.len() {
            continue;
        }
        if let Some(pos) = bytes.windows(needle.len()).rposition(|w| w == *needle) {
            latest = match latest {
                Some((p, _)) if p >= pos => latest,
                _ => Some((pos, *state)),
            };
        }
    }
    latest.map(|(_, state)| state)
}

/// Returns the resulting alt-screen state if `bytes` contains one or
/// more enter/exit alt-screen escapes; `None` when no such escape is
/// present. Recognized escapes: `\x1b[?1049h` / `\x1b[?1049l` (the
/// modern combined save-cursor + alt-screen pair claude-code / codex
/// emit) and `\x1b[?47h` / `\x1b[?47l` (the legacy alt-screen pair,
/// kept for older TUIs).
///
/// The *latest* match in the slice wins — chunks that enter then exit
/// within a single buffer (rare but legal) resolve to the trailing
/// state, not whichever bracket happens to come first in the linear
/// scan.
fn scan_alt_screen_transition(bytes: &[u8]) -> Option<bool> {
    const PATTERNS: &[(&[u8], bool)] = &[
        (b"\x1b[?1049h", true),
        (b"\x1b[?1049l", false),
        (b"\x1b[?47h", true),
        (b"\x1b[?47l", false),
    ];
    scan_mode_transition(bytes, PATTERNS)
}

fn scan_bracketed_paste_transition(bytes: &[u8]) -> Option<bool> {
    const PATTERNS: &[(&[u8], bool)] = &[(b"\x1b[?2004h", true), (b"\x1b[?2004l", false)];
    scan_mode_transition(bytes, PATTERNS)
}

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
    /// Non-blocking append of a forwarder-emitted `runner_status`
    /// row. The consumer thread runs this on every status
    /// transition; it must not block (it shares the mpsc receiver
    /// with the terminal output stream and the exit-event reap, so
    /// a stuck flock would freeze them too). Wire shape mirrors
    /// `cli/src/signal.rs::run_status` so router / UI projections
    /// can't tell the two apart except by `payload.source`.
    fn try_append_runner_status(&self, state: RunnerStatus, source: &'static str) -> AppendOutcome {
        let state_str = match state {
            RunnerStatus::Busy => "busy",
            RunnerStatus::Idle => "idle",
        };
        let draft = EventDraft::signal(
            self.crew_id.clone(),
            self.mission_id.clone(),
            self.handle.clone(),
            SignalType::new("runner_status"),
            serde_json::json!({ "state": state_str, "source": source }),
        );
        match self.event_log.try_append(draft) {
            Ok(_) => AppendOutcome::Ok,
            Err(TryAppendError::Contended) => AppendOutcome::Contended,
            Err(TryAppendError::Failed(_)) => AppendOutcome::Failed,
        }
    }
}

/// Streak indices at which the forwarder consumer logs a WARN about
/// dropped `runner_status` events. Picked to cover the common
/// cases (first drop, sustained failure on a stuck mission log)
/// without spamming once it's clear the log is broken.
fn drop_streak_is_loggable(streak: u64) -> bool {
    matches!(streak, 1 | 10 | 100 | 1000) || (streak >= 10_000 && streak.is_multiple_of(10_000))
}

/// Decouples the PTY layer from Tauri so the reader thread can be unit-tested
/// with a fake. Prod wraps an `AppHandle::emit`; tests use a no-op or a
/// channel-capture impl.
pub trait SessionEvents: Send + Sync + 'static {
    fn output(&self, ev: &OutputEvent);
    fn exit(&self, ev: &ExitEvent);
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

/// Emitter for the real Tauri app — emits `session/output`, `session/exit`,
/// `session/updated`, and `runner/activity`.
pub struct TauriSessionEvents<R: tauri::Runtime = tauri::Wry>(pub tauri::AppHandle<R>);

impl<R: tauri::Runtime> SessionEvents for TauriSessionEvents<R> {
    fn output(&self, ev: &OutputEvent) {
        use tauri::Emitter;
        let _ = self.0.emit("session/output", ev);
    }
    fn exit(&self, ev: &ExitEvent) {
        use tauri::Emitter;
        let _ = self.0.emit("session/exit", ev);
    }
    fn updated(&self, ev: &SessionUpdatedEvent) {
        use tauri::Emitter;
        let _ = self.0.emit("session/updated", ev);
    }
    fn status(&self, ev: &SessionActivityEvent) {
        use tauri::Emitter;
        if ev.state == SessionActivityState::Idle {
            if let Err(error) =
                crate::commands::tab::record_session_completion(&self.0, &ev.session_id)
            {
                log::warn!(
                    "record direct-chat completion for {} failed: {error}",
                    ev.session_id
                );
            }
        }
        let _ = self.0.emit("session/status", ev);
    }
    fn runner_activity(&self, ev: &RunnerActivityEvent) {
        use tauri::Emitter;
        let _ = self.0.emit("runner/activity", ev);
    }
    fn warning(&self, ev: &WarningEvent) {
        use tauri::Emitter;
        let _ = self.0.emit("session/warning", ev);
    }
}

/// Contents of `session/output` events emitted to the frontend. The raw PTY
/// bytes are base64-encoded so the event payload is valid JSON regardless of
/// what the child wrote (ANSI escapes, split UTF-8 sequences, non-UTF-8, etc.).
/// The frontend decodes before feeding xterm.js.
///
/// `mission_id` is `None` for direct-chat sessions (C8.5) — they have no
/// parent mission and consumers should filter on `session_id` instead.
#[derive(Debug, Clone, Serialize)]
pub struct OutputEvent {
    pub session_id: String,
    pub mission_id: Option<String>,
    /// Monotonic per-session sequence number. Frontend attach uses this to
    /// merge a replay snapshot with live events without duplicating chunks.
    pub seq: u64,
    /// Base64-encoded raw bytes read from the PTY.
    pub data: String,
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
    /// spawn. Internal signal: `commands::session::session_resume`
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
    /// Codex cannot be given a caller-owned session id at launch.
    /// When this is present, user activity can retry native id
    /// capture after Codex has actually created its rollout file.
    codex_capture: Option<CodexCaptureContext>,
    /// Forwarder thread that drains the runtime's `OutputStream`
    /// into `session/output` events. `kill` joins on this so callers
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
    spawn_cwd: String,
    started_at: DateTime<Utc>,
    row_started_at: String,
    spawn_pid: Option<i32>,
    prompt_marker: Option<String>,
    pool: Arc<DbPool>,
    events: Arc<dyn SessionEvents>,
}

#[derive(Default)]
struct SessionState {
    handle: Option<SessionHandle>,
    activity: Option<SessionActivityState>,
    suppress_local_input_busy: bool,
    completion_armed: bool,
    output_buffer: VecDeque<OutputEvent>,
    output_seq: u64,
    /// `output_seq` at the moment the most recent resume started.
    /// The pill fast-paths only honor TUI-ready escapes in chunks
    /// with `seq > resume_watermark_seq`, so pre-resume bytes kept
    /// in the ring (claude-code) can never clear a resuming overlay
    /// that's waiting on the *new* PTY. 0 outside resume flows.
    resume_watermark_seq: u64,
    alt_screen_on: bool,
    bracketed_paste_on: bool,
    resuming: bool,
    killed: bool,
}

impl SessionState {
    fn is_empty(&self) -> bool {
        self.handle.is_none()
            && self.activity.is_none()
            && !self.suppress_local_input_busy
            && !self.completion_armed
            && self.output_buffer.is_empty()
            && self.output_seq == 0
            && self.resume_watermark_seq == 0
            && !self.alt_screen_on
            && !self.bracketed_paste_on
            && !self.resuming
            && !self.killed
    }
}

pub struct SessionManager {
    /// Per-session state. The outer map lock protects membership only;
    /// each session's hot mutable state lives behind its own mutex so
    /// PTY output for one busy session does not block lifecycle work on
    /// other sessions.
    sessions: Mutex<HashMap<String, Arc<Mutex<SessionState>>>>,
    /// User's login-shell env snapshot, captured once at app start by
    /// `shell_path::resolve_login_shell_env`. Empty when the resolve
    /// failed/timed out, when running on Windows, or in tests.
    ///
    /// `path` is composed into every child PTY's PATH (so GUI-launched
    /// apps can find tools like claude / codex / mise that aren't on
    /// launchd's stripped default PATH — issue #65); `vars` (the
    /// proxy quartet in both cases) is layered into every spawn's env
    /// under `runner.env` so the child can reach the network the same
    /// way Terminal.app's children would (issues #109 / #152).
    shell_env: crate::shell_path::LoginShellEnv,
    /// Timestamp of the most recent claude-code spawn through the
    /// launch gate. `None` until the first claude-code spawn lands.
    /// Each new claude-code spawn reads this, sleeps the remainder
    /// of `CLAUDE_LAUNCH_GATE_GRACE`, then updates it. Non-claude
    /// runtimes never touch this field. See `enter_claude_launch_gate`
    /// + issue #171.
    claude_launch_gate: Mutex<Option<Instant>>,
    /// Cancellation flags for in-flight background mission spawns,
    /// keyed by `mission_id`. `mission_start` / `mission_reset`
    /// register a fresh flag before dispatching the
    /// `complete_mission_session_spawn` background task;
    /// `kill_all_for_mission` flips it; the background task checks
    /// it around the gate sleep and at the top of each iteration so
    /// the queued slots don't keep firing into a stopped /
    /// archived / reset mission. See `cancel_pending_mission_spawns`.
    pending_mission_cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Underlying terminal runtime. Every spawn / resume / kill /
    /// inject_stdin / resize routes through this trait — the manager
    /// owns DB + event-buffer state but never reads/writes a PTY
    /// directly.
    runtime: Arc<dyn SessionRuntime>,
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
    /// `kill_all_for_mission` flipped the cancel flag (Stop / Archive
    /// / Reset). The PTY was never forked. Caller should mark the
    /// session row stopped so the workspace UI reflects reality.
    Cancelled,
}

/// Inputs `complete_mission_session_spawn` needs that
/// `register_mission_session` already computed. The two-phase split
/// lets `commands::mission::mission_start` finish row inserts +
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
        shell_env: crate::shell_path::LoginShellEnv,
        runtime: Arc<dyn SessionRuntime>,
    ) -> Arc<Self> {
        Arc::new(Self {
            sessions: Mutex::new(HashMap::new()),
            shell_env,
            claude_launch_gate: Mutex::new(None),
            pending_mission_cancels: Mutex::new(HashMap::new()),
            runtime,
        })
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

    fn prune_empty_session_state(&self, session_id: &str) {
        let mut sessions = self.sessions.lock().unwrap();
        let should_remove = sessions
            .get(session_id)
            .map(|state| state.lock().unwrap().is_empty())
            .unwrap_or(false);
        if should_remove {
            sessions.remove(session_id);
        }
    }

    fn install_handle(&self, session_id: &str, handle: SessionHandle) {
        let state = self.session_state_or_insert(session_id);
        let mut state = state.lock().unwrap();
        state.handle = Some(handle);
        state.killed = false;
    }

    fn install_forwarder(&self, session_id: &str, forwarder: thread::JoinHandle<()>) {
        if let Some(state) = self.session_state(session_id) {
            if let Some(handle) = state.lock().unwrap().handle.as_mut() {
                handle.forwarder = Some(forwarder);
            }
        }
    }

    pub(crate) fn publish_direct_activity(
        &self,
        session_id: &str,
        state: SessionActivityState,
        source: &str,
        events: &dyn SessionEvents,
    ) {
        let session = self.session_state_or_insert(session_id);
        let should_emit = {
            let mut session = session.lock().unwrap();
            if source == "forwarder"
                && state == SessionActivityState::Busy
                && session.suppress_local_input_busy
            {
                false
            } else {
                if state == SessionActivityState::Idle {
                    session.suppress_local_input_busy = false;
                }
                if session.activity == Some(state) {
                    false
                } else {
                    session.activity = Some(state);
                    true
                }
            }
        };
        if !should_emit {
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
        let sessions = self.sessions.lock().unwrap();
        let mut armed = false;
        for session_id in session_ids {
            if let Some(session) = sessions.get(session_id) {
                let mut session = session.lock().unwrap();
                armed |= session.completion_armed;
                session.completion_armed = false;
            }
        }
        armed
    }

    pub fn activity_snapshot(&self) -> BTreeMap<String, SessionActivityState> {
        self.sessions
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(id, session)| {
                session
                    .lock()
                    .unwrap()
                    .activity
                    .map(|activity| (id.clone(), activity))
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
/// Resolve the cwd the codex_capture watcher should match against,
/// falling back to the parent process's cwd when the spawn didn't
/// set one (the child inherits parent's cwd, which is what codex
/// stamps into the rollout's `payload.cwd`).
fn capture_cwd(explicit: Option<String>) -> Option<String> {
    if let Some(cwd) = explicit {
        if !cwd.is_empty() {
            return Some(cwd);
        }
    }
    std::env::current_dir()
        .ok()
        .and_then(|p| p.into_os_string().into_string().ok())
}

pub(crate) fn runtime_direct_runner(runtime: &str, command: Option<&str>) -> Result<Runner> {
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
            crate::commands::runner::default_permission_mode(),
        ),
        working_dir: None,
        system_prompt: None,
        env: HashMap::new(),
        model: None,
        effort: None,
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
    // `commands::runner::runner_activity` so live `runner/activity`
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
