// Per-runner session manager.
//
// One `Session` = one tmux pane running the runner's CLI agent (via the
// `SessionRuntime` trait → `TmuxRuntime`). The SessionManager holds the
// map of live sessions so Tauri commands can look them up by id (for
// stdin injection, pause/resume, kill). Each session owns:
//
//   - A `RuntimeSession` (tmux session/window/pane ids) that the manager
//     hands back to the runtime for every operation.
//   - A forwarder thread that drains the runtime's `OutputStream` into
//     `session/output` Tauri events. When the channel closes (pane died
//     or we killed it), the thread queries the runtime for final exit
//     code, emits `session/exit`, and updates the DB row.
//
// Drop behavior: tmux server stays alive across app restart by design
// (`exit-empty off` in the generated config). Reattach uses the
// runtime_* columns persisted on each session row to find the pane and
// re-establish the output stream. Step 9 of
// docs/impls/0004-tmux-session-runtime.md.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::Utc;
use rusqlite::params;
use serde::Serialize;

use crate::db::DbPool;
use crate::error::{Error, Result};
use crate::model::{Mission, Runner};
use crate::router;
use crate::session::runtime::{
    OutputStream, RuntimeOutput, RuntimeSession, SessionRuntime, SpawnSpec,
};

const MAX_OUTPUT_BUFFER_CHUNKS: usize = 4096;

/// Decouples the PTY layer from Tauri so the reader thread can be unit-tested
/// with a fake. Prod wraps an `AppHandle::emit`; tests use a no-op or a
/// channel-capture impl.
pub trait SessionEvents: Send + Sync + 'static {
    fn output(&self, ev: &OutputEvent);
    fn exit(&self, ev: &ExitEvent);
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

/// Emitter for the real Tauri app — emits `session/output`, `session/exit`,
/// and `runner/activity`.
pub struct TauriSessionEvents(pub tauri::AppHandle);

impl SessionEvents for TauriSessionEvents {
    fn output(&self, ev: &OutputEvent) {
        use tauri::Emitter;
        let _ = self.0.emit("session/output", ev);
    }
    fn exit(&self, ev: &ExitEvent) {
        use tauri::Emitter;
        let _ = self.0.emit("session/exit", ev);
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
    pub runner_id: String,
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
    runner_id: String,
    /// Runtime-side identifiers (tmux session/window/pane) returned
    /// from `SessionRuntime::spawn`. The manager passes this back
    /// to `runtime.send_bytes` / `runtime.paste` / `runtime.resize`
    /// / `runtime.stop` for every operation on the live session.
    runtime_session: RuntimeSession,
    /// Forwarder thread that drains the runtime's `OutputStream`
    /// into `session/output` events. `kill` joins on this so callers
    /// (mission_stop) get the same "no live sessions after we
    /// return" contract the portable-pty path provided.
    forwarder: Option<thread::JoinHandle<()>>,
}

pub struct SessionManager {
    sessions: Mutex<HashMap<String, SessionHandle>>,
    output_buffers: Mutex<HashMap<String, VecDeque<OutputEvent>>>,
    output_seq: Mutex<HashMap<String, u64>>,
    /// Session ids currently inside `resume()`, between the
    /// validation read and the live-map insert. A second concurrent
    /// `resume` call for the same id refuses on insert collision so
    /// two PTYs can't end up racing against the same row (e.g. fast
    /// double-click on the Resume button, or two windows both
    /// driving resume).
    resuming_claims: Mutex<HashSet<String>>,
    /// Session ids the user explicitly killed via `kill()` /
    /// `kill_all_for_mission()`. The reader thread checks this set
    /// when the child exits: a session in it ends as `stopped`
    /// (intentional), not `crashed` (which is reserved for an
    /// unexpected non-zero exit). Without this distinction, clicking
    /// Stop in the workspace would mark every slot crashed because
    /// SIGTERM produces a non-zero exit code. Entries are cleared by
    /// the reader after the DB row is updated.
    killed: Mutex<HashSet<String>>,
    /// User's login-shell PATH, captured once at app start by
    /// `shell_path::resolve_login_shell_path`. None when the resolve
    /// failed/timed out, when running on Windows, or in tests.
    /// Merged into every child PTY's PATH so GUI-launched apps can
    /// find tools (claude, codex, mise, etc.) that aren't on
    /// launchd's stripped default PATH.
    shell_path: Option<String>,
    /// Underlying terminal runtime (Step 9 of
    /// docs/impls/0004-tmux-session-runtime.md). v1 is `TmuxRuntime`
    /// on macOS + Linux; Windows fails at runtime construction in
    /// `lib.rs::run`. Every spawn / resume / kill / inject_stdin /
    /// resize routes through this trait — the manager owns DB +
    /// event-buffer state but never reads/writes a PTY directly.
    runtime: Arc<dyn SessionRuntime>,
}

/// RAII guard that releases a `resuming_claims` entry on drop. The
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
        self.mgr
            .resuming_claims
            .lock()
            .unwrap()
            .remove(&self.session_id);
    }
}

impl SessionManager {
    pub fn new(shell_path: Option<String>, runtime: Arc<dyn SessionRuntime>) -> Arc<Self> {
        Arc::new(Self {
            sessions: Mutex::new(HashMap::new()),
            killed: Mutex::new(HashSet::new()),
            output_buffers: Mutex::new(HashMap::new()),
            output_seq: Mutex::new(HashMap::new()),
            resuming_claims: Mutex::new(HashSet::new()),
            shell_path,
            runtime,
        })
    }

    /// Borrow the underlying session runtime. Held on the manager
    /// itself rather than passed through every method so the
    /// Step 9 cutovers can land one entry point at a time without
    /// rewiring every Tauri command's signature in the same change.
    #[allow(dead_code)] // Wired into spawn paths in subsequent commits.
    pub(crate) fn runtime(&self) -> &Arc<dyn SessionRuntime> {
        &self.runtime
    }

    /// Build a `SpawnSpec` skeleton with the manager's stable inputs
    /// (shell PATH, runner env after merging system vars). The
    /// runtime adapter argv (resume_plan + trailing_runtime_args)
    /// lives at the call site since it depends on a pre-resolved
    /// `agent_session_key`.
    #[allow(clippy::too_many_arguments)]
    fn base_spawn_spec(
        &self,
        session_id: String,
        runner: &Runner,
        cwd: Option<String>,
        mission: bool,
        shim_dir: Option<PathBuf>,
        bundled_bin_dir: Option<PathBuf>,
        initial_size: Option<(u16, u16)>,
        extra_env: BTreeMap<String, String>,
    ) -> SpawnSpec {
        let mut env: BTreeMap<String, String> = runner
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        // System vars layer on top so the user can't accidentally
        // shadow them. PATH is set by the launch script from the
        // composed path; a runner.env PATH would be filtered by
        // `launch::is_reserved_env_name` but we layer system vars
        // anyway for parity with the prior portable-pty path.
        env.insert("TERM".into(), "xterm-256color".into());
        env.insert("COLORTERM".into(), "truecolor".into());
        for (k, v) in extra_env {
            env.insert(k, v);
        }
        SpawnSpec {
            session_id,
            cwd: cwd.map(PathBuf::from),
            command: runner.command.clone(),
            args: runner.args.clone(),
            env,
            mission,
            shim_dir,
            bundled_bin_dir,
            shell_path: self.shell_path.clone(),
            initial_size,
        }
    }

    /// Apply the runtime adapter's resume + trailing args to a
    /// `SpawnSpec`. Mirrors what the portable-pty `spawn` paths
    /// did inline; factored out so spawn / spawn_direct / resume
    /// can share the argv composition.
    fn apply_runtime_args(
        spec: &mut SpawnSpec,
        runner: &Runner,
        plan: &router::runtime::ResumePlan,
    ) {
        let mut composed: Vec<String> = Vec::new();
        if plan.prepend {
            composed.extend(plan.args.iter().cloned());
            composed.append(&mut spec.args);
        } else {
            composed.append(&mut spec.args);
            composed.extend(plan.args.iter().cloned());
        }
        for extra in router::runtime::trailing_runtime_args(
            &runner.runtime,
            plan.resuming,
            runner.model.as_deref(),
            runner.effort.as_deref(),
            runner.system_prompt.as_deref(),
        ) {
            composed.push(extra);
        }
        spec.args = composed;
    }

    /// Spawn one PTY child for `runner` as part of `mission`. Persists a
    /// `sessions` row, starts the reader thread, and returns a summary for
    /// the frontend.
    ///
    /// `app_data_dir` is the root of `$APPDATA/runner/` so we can prepend
    /// `<app_data_dir>/bin` onto the child's PATH — arch §5.3 Layer 2 and
    /// 0001-v0-mvp.md C9 both require the bundled `runner` CLI to win over any
    /// system binary with the same name.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        self: &Arc<Self>,
        mission: &Mission,
        runner: &Runner,
        slot: &crate::model::Slot,
        app_data_dir: &Path,
        events_log_path: PathBuf,
        pool: Arc<DbPool>,
        events: Arc<dyn SessionEvents>,
    ) -> Result<SpawnedSession> {
        // Agent-native session resume: this is a *fresh* session row, so
        // there's no prior key to inherit. The runtime adapter still
        // self-assigns a UUID for claude-code (`--session-id <uuid>`) so
        // a future `SessionManager::resume` can hand it back.
        let plan = router::runtime::resume_plan(&runner.runtime, None);

        // Working directory: runner override if set, else mission cwd, else
        // inherit parent's. Capture the resolved cwd so we can persist it
        // on the session row — `resume` reads it back to spawn the same
        // dir on respawn, which matters for claude-code (its conversation
        // files are keyed under `~/.claude/projects/<encoded-cwd>/<uuid>.jsonl`;
        // resuming with a different cwd makes `--resume` fail).
        let resolved_cwd: Option<String> =
            runner.working_dir.clone().or_else(|| mission.cwd.clone());

        // Per-slot runner shim: hardcodes the RUNNER_* env vars + exec's
        // the real bundled CLI. claude-code's Bash tool spawns
        // non-login shells that don't inherit the PTY's env, so a CLI
        // call like `runner msg post …` would otherwise see the vars
        // as unset. The shim sits in front of the bundled `runner` on
        // PATH so `runner` resolves to it regardless of shell context.
        let shim_dir = crate::cli_install::install_session_runner_shim(
            app_data_dir,
            &mission.crew_id,
            &mission.id,
            &slot.slot_handle,
            &events_log_path,
            mission.cwd.as_deref(),
        )
        .ok();
        let bundled_bin_dir = Some(app_data_dir.join("bin"));

        let mut mission_env: BTreeMap<String, String> = BTreeMap::new();
        mission_env.insert("RUNNER_CREW_ID".into(), mission.crew_id.clone());
        mission_env.insert("RUNNER_MISSION_ID".into(), mission.id.clone());
        // RUNNER_HANDLE is the slot's in-mission identity, not the
        // runner template's handle.
        mission_env.insert("RUNNER_HANDLE".into(), slot.slot_handle.clone());
        mission_env.insert(
            "RUNNER_EVENT_LOG".into(),
            events_log_path.to_string_lossy().to_string(),
        );
        if let Some(wd) = mission.cwd.as_deref() {
            mission_env.insert("MISSION_CWD".into(), wd.to_string());
        }

        let session_id = ulid::Ulid::new().to_string();
        let mut spec = self.base_spawn_spec(
            session_id.clone(),
            runner,
            resolved_cwd.clone(),
            true,
            shim_dir,
            bundled_bin_dir,
            None, // mission spawn doesn't yet receive cols/rows from the caller
            mission_env,
        );
        Self::apply_runtime_args(&mut spec, runner, &plan);

        // Insert the row first (status=running with no runtime_*
        // metadata yet) so a fast-failing runtime spawn doesn't leave
        // a half-row. We update with runtime metadata once the
        // runtime hands them back.
        let started_at = Utc::now().to_rfc3339();
        {
            let conn = pool.get()?;
            conn.execute(
                "INSERT INTO sessions
                    (id, mission_id, runner_id, slot_id, cwd, status, pid, started_at,
                     agent_session_key)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'running', NULL, ?6, ?7)",
                params![
                    session_id,
                    mission.id,
                    runner.id,
                    slot.id,
                    resolved_cwd,
                    started_at,
                    plan.assigned_key
                ],
            )?;
        }

        let (rt_session, output) = match self.runtime.spawn(spec) {
            Ok(p) => p,
            Err(e) => {
                // Roll back the inserted row so a retry can proceed.
                if let Ok(conn) = pool.get() {
                    let _ = conn.execute("DELETE FROM sessions WHERE id = ?1", params![session_id]);
                }
                return Err(Error::msg(format!("spawn {}: {e}", runner.command)));
            }
        };

        // Persist the runtime-side ids so `resume` after app restart
        // can find this pane.
        if let Ok(conn) = pool.get() {
            let _ = conn.execute(
                "UPDATE sessions
                    SET runtime = ?2,
                        runtime_socket = ?3,
                        runtime_session = ?4,
                        runtime_window = ?5,
                        runtime_pane = ?6
                  WHERE id = ?1",
                params![
                    session_id,
                    rt_session.runtime,
                    rt_session.socket,
                    rt_session.session_name,
                    rt_session.window,
                    rt_session.pane,
                ],
            );
        }

        self.sessions.lock().unwrap().insert(
            session_id.clone(),
            SessionHandle {
                id: session_id.clone(),
                mission_id: Some(mission.id.clone()),
                runner_id: runner.id.clone(),
                runtime_session: rt_session.clone(),
                forwarder: None,
            },
        );

        let forwarder = self.start_forwarder_thread(
            session_id.clone(),
            Some(mission.id.clone()),
            rt_session,
            output,
            Arc::clone(&pool),
            Arc::clone(&events),
            runner.clone(),
            plan.resuming,
        );
        if let Some(h) = self.sessions.lock().unwrap().get_mut(&session_id) {
            h.forwarder = Some(forwarder);
        }

        emit_runner_activity(&pool, runner, events.as_ref());
        schedule_mission_first_prompt(self, session_id.clone(), runner, &plan, slot.lead);

        Ok(SpawnedSession {
            id: session_id,
            mission_id: Some(mission.id.clone()),
            runner_id: runner.id.clone(),
            handle: runner.handle.clone(),
            // pane_pid is populated lazily via runtime.status()
            // when the manager needs it; the SpawnedSession field
            // is informational and the frontend doesn't rely on
            // it.
            pid: None,
            fresh_fallback_lead: false,
        })
    }

    /// Spawn a "direct chat" PTY: a runner process with **no parent
    /// mission**. Schema-supported since C5.5a (`sessions.mission_id` is
    /// nullable); C8.5 surfaces it as the "Chat now" affordance on the
    /// Runner Detail page.
    ///
    /// Differences vs. the mission-flavored `spawn`:
    ///   - No `RUNNER_MISSION_ID`, `RUNNER_EVENT_LOG`, or
    ///     `RUNNER_CREW_ID` env vars. The bundled `runner` CLI is also
    ///     deliberately NOT on PATH for direct chats: `runner msg post`,
    ///     `runner status idle`, etc. would have no event log to write
    ///     to and no crew/mission to attribute against, so removing the
    ///     shim avoids tempting the agent to call verbs that fail
    ///     silently. Direct chats are off-bus.
    ///   - `cwd` lives on the session row directly, since there's no
    ///     mission to inherit it from.
    ///   - The session does not show up in `kill_all_for_mission` for any
    ///     mission_id, so a `mission_stop` on some unrelated crew never
    ///     yanks the user's open chat.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_direct(
        self: &Arc<Self>,
        runner: &Runner,
        cwd: Option<&str>,
        cols: Option<u16>,
        rows: Option<u16>,
        app_data_dir: &Path,
        pool: Arc<DbPool>,
        events: Arc<dyn SessionEvents>,
    ) -> Result<SpawnedSession> {
        let _ = app_data_dir; // direct chats don't get the bundled CLI on PATH

        // Agent-native session resume: `spawn_direct` always opens a *new*
        // chat. The runtime adapter self-assigns a fresh
        // `agent_session_key` (claude-code) or leaves it NULL (codex).
        let plan = router::runtime::resume_plan(&runner.runtime, None);

        // Working directory precedence: explicit `cwd` arg (Chat now
        // dialog folder) ► runner's `working_dir` ► inherit parent's.
        let resolved_cwd: Option<String> = cwd
            .map(|s| s.to_string())
            .or_else(|| runner.working_dir.clone());

        // Direct chats are off-bus: RUNNER_HANDLE is the runner template's
        // own handle, no slot/mission env vars.
        let mut direct_env: BTreeMap<String, String> = BTreeMap::new();
        direct_env.insert("RUNNER_HANDLE".into(), runner.handle.clone());

        let initial_size = cols.zip(rows);

        let session_id = ulid::Ulid::new().to_string();
        let started_at_dt = Utc::now();
        let started_at = started_at_dt.to_rfc3339();

        let mut spec = self.base_spawn_spec(
            session_id.clone(),
            runner,
            resolved_cwd.clone(),
            false,
            None, // shim_dir — off-bus
            None, // bundled_bin_dir — off-bus
            initial_size,
            direct_env,
        );
        Self::apply_runtime_args(&mut spec, runner, &plan);

        // Insert the row first so a fast-failing spawn doesn't leave
        // a half-row.
        {
            let conn = pool.get()?;
            conn.execute(
                "INSERT INTO sessions
                    (id, mission_id, runner_id, cwd, status, pid, started_at,
                     agent_session_key)
                 VALUES (?1, NULL, ?2, ?3, 'running', NULL, ?4, ?5)",
                params![
                    session_id,
                    runner.id,
                    resolved_cwd,
                    started_at,
                    plan.assigned_key
                ],
            )?;
        }

        let (rt_session, output) = match self.runtime.spawn(spec) {
            Ok(p) => p,
            Err(e) => {
                if let Ok(conn) = pool.get() {
                    let _ = conn.execute("DELETE FROM sessions WHERE id = ?1", params![session_id]);
                }
                return Err(Error::msg(format!("spawn {}: {e}", runner.command)));
            }
        };

        if let Ok(conn) = pool.get() {
            let _ = conn.execute(
                "UPDATE sessions
                    SET runtime = ?2,
                        runtime_socket = ?3,
                        runtime_session = ?4,
                        runtime_window = ?5,
                        runtime_pane = ?6
                  WHERE id = ?1",
                params![
                    session_id,
                    rt_session.runtime,
                    rt_session.socket,
                    rt_session.session_name,
                    rt_session.window,
                    rt_session.pane,
                ],
            );
        }

        self.sessions.lock().unwrap().insert(
            session_id.clone(),
            SessionHandle {
                id: session_id.clone(),
                mission_id: None,
                runner_id: runner.id.clone(),
                runtime_session: rt_session.clone(),
                forwarder: None,
            },
        );

        let forwarder = self.start_forwarder_thread(
            session_id.clone(),
            None,
            rt_session,
            output,
            Arc::clone(&pool),
            Arc::clone(&events),
            runner.clone(),
            plan.resuming,
        );
        if let Some(h) = self.sessions.lock().unwrap().get_mut(&session_id) {
            h.forwarder = Some(forwarder);
        }

        // Codex doesn't accept a caller-assigned session id at spawn,
        // so the runtime adapter leaves `assigned_key = None` for
        // fresh codex spawns. Kick off a short-lived watcher that
        // captures codex's auto-generated id from the rollout file
        // and writes it to `agent_session_key` so the *next* resume
        // can drive `codex resume <uuid>`.
        if runner.runtime == "codex" && plan.assigned_key.is_none() {
            if let Some(cwd) = capture_cwd(resolved_cwd.clone()) {
                crate::session::codex_capture::spawn_capture(
                    session_id.clone(),
                    cwd,
                    started_at_dt,
                    Arc::clone(&pool),
                );
            }
        }

        emit_runner_activity(&pool, runner, events.as_ref());
        schedule_direct_first_prompt(self, session_id.clone(), runner, &plan);

        Ok(SpawnedSession {
            id: session_id,
            mission_id: None,
            runner_id: runner.id.clone(),
            handle: runner.handle.clone(),
            pid: None,
            fresh_fallback_lead: false,
        })
    }

    /// Respawn a PTY for an existing direct-chat session row, reusing
    /// its id and (when present) its `agent_session_key`. The row is
    /// updated in place: status flips back to running, pid/started_at
    /// are refreshed, stopped_at clears, and the agent key is rewritten
    /// (claude-code preserves the prior UUID; codex would persist a
    /// captured key once the capture path lands).
    ///
    /// Works for both direct-chat rows (mission_id IS NULL) and
    /// mission-scoped rows. For mission rows the env block additionally
    /// stamps `RUNNER_HANDLE = slot.slot_handle`, `RUNNER_CREW_ID`,
    /// and `RUNNER_MISSION_ID` so a resumed worker keeps its in-mission
    /// identity. The mission's Router must already be mounted (via
    /// `mission_start` originally, or `mission_attach` after restart)
    /// for stdin pushes to land — resume itself doesn't touch the
    /// router; the slot_handle → session_id mapping is unchanged.
    ///
    /// Refused for:
    ///   - rows that don't exist
    ///   - rows already running (caller should attach, not resume)
    ///   - archived rows (un-archive first)
    #[allow(clippy::too_many_arguments)]
    pub fn resume(
        self: &Arc<Self>,
        session_id: &str,
        cols: Option<u16>,
        rows: Option<u16>,
        app_data_dir: &Path,
        pool: Arc<DbPool>,
        events: Arc<dyn SessionEvents>,
    ) -> Result<SpawnedSession> {
        // Atomically claim this session id for the resume. If another
        // resume is already in flight (e.g. two fast clicks, two
        // windows), refuse rather than racing two PTY spawns against
        // the same row. The claim guard releases on every exit path
        // via Drop.
        let _claim = {
            let mut set = self.resuming_claims.lock().unwrap();
            if !set.insert(session_id.to_string()) {
                return Err(Error::msg(format!(
                    "session {session_id} is already being resumed"
                )));
            }
            ResumeClaim {
                mgr: Arc::clone(self),
                session_id: session_id.to_string(),
            }
        };

        // Validate the row + collect everything we need under a single
        // short-lived connection. We deliberately don't hold the conn
        // across the spawn (which itself grabs a pool slot for the
        // status update).
        struct Snapshot {
            runner_id: String,
            mission_id: Option<String>,
            slot_id: Option<String>,
            cwd: Option<String>,
            agent_session_key: Option<String>,
        }
        let snap = {
            let conn = pool.get()?;
            let mut stmt = conn.prepare(
                "SELECT runner_id, mission_id, slot_id, cwd, status, archived_at,
                        agent_session_key
                   FROM sessions WHERE id = ?1",
            )?;
            let row = stmt
                .query_row(params![session_id], |r| {
                    Ok((
                        r.get::<_, String>("runner_id")?,
                        r.get::<_, Option<String>>("mission_id")?,
                        r.get::<_, Option<String>>("slot_id")?,
                        r.get::<_, Option<String>>("cwd")?,
                        r.get::<_, String>("status")?,
                        r.get::<_, Option<String>>("archived_at")?,
                        r.get::<_, Option<String>>("agent_session_key")?,
                    ))
                })
                .map_err(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => {
                        Error::msg(format!("session not found: {session_id}"))
                    }
                    other => other.into(),
                })?;
            let (runner_id, mission_id, slot_id, cwd, status, archived_at, agent_session_key) = row;
            if status == "running" {
                return Err(Error::msg(format!(
                    "session {session_id} is already running — attach instead"
                )));
            }
            if archived_at.is_some() {
                return Err(Error::msg(format!(
                    "session {session_id} is archived — un-archive before resuming"
                )));
            }
            Snapshot {
                runner_id,
                mission_id,
                slot_id,
                cwd,
                agent_session_key,
            }
        };

        // Mission resume: pull the slot + mission so we can stamp the
        // in-mission env (RUNNER_HANDLE = slot_handle, RUNNER_CREW_ID,
        // RUNNER_MISSION_ID). Direct-chat rows skip this lookup —
        // their RUNNER_HANDLE is the runner template's globally-unique
        // handle, no slot involved.
        struct MissionCtx {
            crew_id: String,
            mission_id: String,
            mission_cwd: Option<String>,
            slot_handle: String,
            lead: bool,
        }
        let mission_ctx: Option<MissionCtx> =
            match (snap.mission_id.as_deref(), snap.slot_id.as_deref()) {
                (Some(mid), Some(sid)) => {
                    let conn = pool.get()?;
                    let mission = crate::commands::mission::get(&conn, mid)?;
                    let (slot_handle, lead): (String, i64) = conn
                        .query_row(
                            "SELECT slot_handle, lead FROM slots WHERE id = ?1",
                            params![sid],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .map_err(|e| match e {
                            rusqlite::Error::QueryReturnedNoRows => Error::msg(format!(
                                "slot {sid} referenced by session {session_id} no longer exists"
                            )),
                            other => other.into(),
                        })?;
                    Some(MissionCtx {
                        crew_id: mission.crew_id,
                        mission_id: mission.id,
                        mission_cwd: mission.cwd,
                        slot_handle,
                        lead: lead != 0,
                    })
                }
                _ => None,
            };

        // Pull the runner config fresh — the user may have edited it
        // since the session last ran, and we want the current command /
        // args / env on respawn.
        let runner = {
            let conn = pool.get()?;
            crate::commands::runner::get(&conn, &snap.runner_id)?
        };

        // Resume plan: hand the prior agent_session_key back to the
        // runtime adapter so claude-code uses `--resume <uuid>` and
        // codex (once capture lands) uses `codex resume <uuid>`.
        //
        // claude-code only: if the conversation file for this
        // (cwd, uuid) was never persisted, `--resume <uuid>` would
        // print "No conversation found" and leave the TUI half-broken.
        // Detect the missing file up front and degrade to a fresh
        // spawn that *keeps* the same uuid via `--session-id`.
        let resolved_cwd_for_check: Option<String> =
            snap.cwd.clone().or_else(|| runner.working_dir.clone());
        let is_lead_slot = mission_ctx.as_ref().is_some_and(|c| c.lead);
        let conversation_missing = matches!(
            (runner.runtime.as_str(), snap.agent_session_key.as_deref()),
            ("claude-code", Some(key))
                if !router::runtime::claude_code_conversation_exists(
                    resolved_cwd_for_check.as_deref(),
                    key,
                )
        );
        let fresh_fallback_lead = conversation_missing && is_lead_slot;
        let effective_prior_key = match (runner.runtime.as_str(), snap.agent_session_key.as_deref())
        {
            ("claude-code", Some(_)) if conversation_missing => None,
            (_, k) => k,
        };
        let plan = router::runtime::resume_plan(&runner.runtime, effective_prior_key);

        // Working directory: same precedence as `spawn_direct` — the
        // row's stored cwd wins; otherwise fall back to the runner's
        // current `working_dir`.
        let resolved_cwd: Option<String> = snap.cwd.clone().or_else(|| runner.working_dir.clone());

        // Refresh the per-slot runner shim before composing PATH —
        // mission cwd may have been edited since the last spawn.
        let shim_dir = mission_ctx.as_ref().and_then(|ctx| {
            let event_log_path = runner_core::event_log::path::events_path(
                app_data_dir,
                &ctx.crew_id,
                &ctx.mission_id,
            );
            crate::cli_install::install_session_runner_shim(
                app_data_dir,
                &ctx.crew_id,
                &ctx.mission_id,
                &ctx.slot_handle,
                &event_log_path,
                ctx.mission_cwd.as_deref(),
            )
            .ok()
        });
        // Direct-chat resume stays off-bus.
        let bundled_bin_dir = mission_ctx.as_ref().map(|_| app_data_dir.join("bin"));

        // Mission resume stamps the slot's in-mission identity; direct
        // chat resume falls through to the template handle.
        let mut env_extra: BTreeMap<String, String> = BTreeMap::new();
        if let Some(ctx) = mission_ctx.as_ref() {
            env_extra.insert("RUNNER_CREW_ID".into(), ctx.crew_id.clone());
            env_extra.insert("RUNNER_MISSION_ID".into(), ctx.mission_id.clone());
            env_extra.insert("RUNNER_HANDLE".into(), ctx.slot_handle.clone());
            let event_log_path = runner_core::event_log::path::events_path(
                app_data_dir,
                &ctx.crew_id,
                &ctx.mission_id,
            );
            env_extra.insert(
                "RUNNER_EVENT_LOG".into(),
                event_log_path.to_string_lossy().to_string(),
            );
            if let Some(wd) = ctx.mission_cwd.as_deref() {
                env_extra.insert("MISSION_CWD".into(), wd.to_string());
            }
        } else {
            env_extra.insert("RUNNER_HANDLE".into(), runner.handle.clone());
        }

        let initial_size = cols.zip(rows);
        let mut spec = self.base_spawn_spec(
            session_id.to_string(),
            &runner,
            resolved_cwd.clone(),
            mission_ctx.is_some(),
            shim_dir,
            bundled_bin_dir,
            initial_size,
            env_extra,
        );
        Self::apply_runtime_args(&mut spec, &runner, &plan);

        let started_at_dt = Utc::now();
        let started_at = started_at_dt.to_rfc3339();

        // UPDATE in place: same id, same conversation thread.
        {
            let conn = pool.get()?;
            conn.execute(
                "UPDATE sessions
                    SET status = 'running',
                        pid = NULL,
                        started_at = ?2,
                        stopped_at = NULL,
                        agent_session_key = COALESCE(?3, agent_session_key)
                  WHERE id = ?1",
                params![session_id, started_at, plan.assigned_key],
            )?;
        }

        let (rt_session, output) = match self.runtime.spawn(spec) {
            Ok(p) => p,
            Err(e) => {
                // Roll the row back to stopped so the user can retry.
                if let Ok(conn) = pool.get() {
                    let _ = conn.execute(
                        "UPDATE sessions
                            SET status = 'stopped',
                                stopped_at = ?2
                          WHERE id = ?1",
                        params![session_id, Utc::now().to_rfc3339()],
                    );
                }
                return Err(Error::msg(format!("spawn {}: {e}", runner.command)));
            }
        };

        if let Ok(conn) = pool.get() {
            let _ = conn.execute(
                "UPDATE sessions
                    SET runtime = ?2,
                        runtime_socket = ?3,
                        runtime_session = ?4,
                        runtime_window = ?5,
                        runtime_pane = ?6
                  WHERE id = ?1",
                params![
                    session_id,
                    rt_session.runtime,
                    rt_session.socket,
                    rt_session.session_name,
                    rt_session.window,
                    rt_session.pane,
                ],
            );
        }

        self.sessions.lock().unwrap().insert(
            session_id.to_string(),
            SessionHandle {
                id: session_id.to_string(),
                mission_id: snap.mission_id.clone(),
                runner_id: runner.id.clone(),
                runtime_session: rt_session.clone(),
                forwarder: None,
            },
        );

        // Purge the prior session's output buffer just before the
        // forwarder thread starts pumping chunks. Keeping the seq
        // counter intact means the new chunk seq continues at
        // `last + 1` so the frontend's seq-merge filter doesn't drop
        // the head of post-resume output.
        self.purge_output_buffer(session_id);

        let forwarder = self.start_forwarder_thread(
            session_id.to_string(),
            snap.mission_id.clone(),
            rt_session,
            output,
            Arc::clone(&pool),
            Arc::clone(&events),
            runner.clone(),
            plan.resuming,
        );
        if let Some(h) = self.sessions.lock().unwrap().get_mut(session_id) {
            h.forwarder = Some(forwarder);
        }

        // Same codex post-spawn capture as `spawn_direct`: when we
        // respawn a codex chat that has no agent_session_key on the
        // row yet (every prior codex chat, until this lands), the
        // adapter starts fresh and the watcher writes the new id so
        // the *next* resume drives `codex resume <uuid>`.
        if runner.runtime == "codex" && plan.assigned_key.is_none() {
            if let Some(cwd) = capture_cwd(resolved_cwd.clone()) {
                crate::session::codex_capture::spawn_capture(
                    session_id.to_string(),
                    cwd,
                    started_at_dt,
                    Arc::clone(&pool),
                );
            }
        }

        emit_runner_activity(&pool, &runner, events.as_ref());

        // First-turn injection for fresh claude-code / codex spawns.
        // `plan.resuming` is true on any resume against a real
        // prior_key — those skip naturally (the agent already has its
        // system context). For mission resume, the lead always
        // suppresses the worker preamble: when the lead's
        // conversation file is missing and the resume degrades to a
        // fresh spawn, the *launch prompt* (composed by the router
        // with crew / roster / goal context) is the right thing to
        // inject — the commands::session::session_resume caller fires
        // that path when it sees `fresh_fallback_lead = true` on the
        // returned SpawnedSession. For direct-chat resume there's no
        // slot/lead concept, and the off-bus persona-only injection
        // (`schedule_direct_first_prompt`) is the right shape if the
        // resume happens to degrade to fresh.
        if mission_ctx.is_some() {
            let is_lead_resume = is_lead_slot;
            schedule_mission_first_prompt(
                self,
                session_id.to_string(),
                &runner,
                &plan,
                is_lead_resume,
            );
        } else {
            schedule_direct_first_prompt(self, session_id.to_string(), &runner, &plan);
        }

        // On a real resume (not a fresh-with-known-uuid spawn), nudge
        // the agent with "continue" so it picks up where it left off
        // without the user having to type. Skipped for fresh spawns
        // — the first-prompt path covers those — and for non-claude-
        // code runtimes that don't have a real resume semantic.
        schedule_continue_on_resume(self, session_id.to_string(), &runner, &plan);

        // Return the slot's in-mission identity for mission rows so the
        // frontend (and the router, which keys on slot_handle) sees the
        // identity the resumed PTY actually stamps onto its events.
        let resumed_handle = mission_ctx
            .as_ref()
            .map(|c| c.slot_handle.clone())
            .unwrap_or_else(|| runner.handle.clone());
        Ok(SpawnedSession {
            id: session_id.to_string(),
            mission_id: snap.mission_id.clone(),
            runner_id: runner.id.clone(),
            handle: resumed_handle,
            pid: None,
            fresh_fallback_lead,
        })
    }

    /// Forwarder thread shared by `spawn`, `spawn_direct`, and `resume`.
    /// Drains the runtime's `OutputStream` into `session/output`
    /// events, then on channel close queries the runtime for the
    /// final exit code, flips the DB row, removes the live-map
    /// entry, and emits `session/exit`. `kill` joins this handle so
    /// `mission_stop` gets the no-lying-about-termination contract.
    // The thread genuinely needs every one of these — session_id /
    // mission_id for event payloads, runtime_session for status
    // queries, output for the input stream, pool for the DB row
    // update, events for emitter dispatch, runner for the
    // post-reap activity recompute. Bundling into a Context struct just
    // moves the same arity to the call site without buying clarity.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    fn start_forwarder_thread(
        self: &Arc<Self>,
        session_id: String,
        mission_id: Option<String>,
        rt_session: RuntimeSession,
        output: OutputStream,
        pool: Arc<DbPool>,
        events: Arc<dyn SessionEvents>,
        runner: Runner,
        resuming: bool,
    ) -> thread::JoinHandle<()> {
        let manager_t: Arc<SessionManager> = Arc::clone(self);
        let started_at = std::time::Instant::now();
        thread::spawn(move || {
            // Drain pane output until the runtime closes the
            // channel (pane died, or the OutputStream was dropped).
            // Replay and Stream both flow as `session/output` events
            // — xterm.js appends sequentially regardless. The
            // distinction matters for snapshot-vs-append semantics
            // when the manager exposes a separate `session/replay`
            // event; today we keep the one-channel shape for
            // backward compat with the existing frontend.
            loop {
                match output.recv_timeout(Duration::from_millis(500)) {
                    Ok(RuntimeOutput::Replay(bytes)) | Ok(RuntimeOutput::Stream(bytes)) => {
                        let ev = manager_t.record_output(
                            &session_id,
                            mission_id.as_deref(),
                            BASE64.encode(&bytes),
                        );
                        events.output(&ev);
                    }
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }

            // Channel closed — query the runtime for the final pane
            // status to recover an exit code. `Ok(None)` means the
            // pane is gone (terminal-unavailable); we still need to
            // flip the DB row, just without an exit code.
            let status = manager_t.runtime.status(&rt_session).ok().flatten();
            let exit_code = status.as_ref().and_then(|s| s.exit_code);
            let success = exit_code == Some(0);

            // Best-effort: tear down the tmux session now that the
            // pane is dead. Skipped if `kill` already did it.
            let _ = manager_t.runtime.stop(&rt_session);

            let _ = manager_t.forget(&session_id);
            let was_killed = manager_t.killed.lock().unwrap().remove(&session_id);
            // Resume failure heuristic: prior conversation rejected
            // and the agent died fast.
            let resume_failed = resuming
                && !success
                && !was_killed
                && started_at.elapsed() < std::time::Duration::from_secs(3);
            let final_status = if success || was_killed {
                "stopped"
            } else {
                "crashed"
            };
            if let Ok(conn) = pool.get() {
                if resume_failed {
                    let _ = conn.execute(
                        "UPDATE sessions
                            SET status = ?1, stopped_at = ?2,
                                agent_session_key = NULL
                          WHERE id = ?3",
                        params!["crashed", Utc::now().to_rfc3339(), session_id],
                    );
                } else {
                    let _ = conn.execute(
                        "UPDATE sessions
                            SET status = ?1, stopped_at = ?2
                          WHERE id = ?3",
                        params![final_status, Utc::now().to_rfc3339(), session_id],
                    );
                }
            }
            if resume_failed {
                events.warning(&WarningEvent {
                    session_id: session_id.clone(),
                    mission_id: mission_id.clone(),
                    kind: "resume_failed".into(),
                    message: format!(
                        "Could not resume the previous {} conversation; the next launch will start fresh.",
                        runner.runtime
                    ),
                });
            }
            emit_runner_activity(&pool, &runner, events.as_ref());
            events.exit(&ExitEvent {
                session_id,
                mission_id,
                exit_code,
                success,
            });
        })
    }

    /// Write raw bytes to the session's stdin. Used for keystroke
    /// passthrough from xterm.js — small chunks, no embedded
    /// newlines. Routed through `runtime.send_bytes` which uses
    /// `tmux send-keys -l --` so each character lands as a
    /// keystroke without bracketed-paste markers.
    ///
    /// Multi-line prompt blocks (the system_prompt injection on
    /// fresh spawn) should go through `inject_paste` instead so the
    /// agent's TUI sees them as one paste rather than 50
    /// keystrokes that might trigger an early submit on the first
    /// `\n`.
    pub fn inject_stdin(&self, session_id: &str, bytes: &[u8]) -> Result<()> {
        let rt_session = self
            .sessions
            .lock()
            .unwrap()
            .get(session_id)
            .map(|h| h.runtime_session.clone())
            .ok_or_else(|| Error::msg(format!("session not found: {session_id}")))?;
        // ASCII CR (0x0D) is what claude-code's TUI editor reads as
        // "Enter" — bare-byte writes that just contain `\r` map to
        // `send_key("Enter")` so tmux's key-name lookup runs.
        // Everything else routes as a literal byte stream.
        if bytes == b"\r" {
            self.runtime
                .send_key(&rt_session, "Enter")
                .map_err(Into::into)
        } else {
            self.runtime
                .send_bytes(&rt_session, bytes)
                .map_err(Into::into)
        }
    }

    /// Paste a multi-line prompt block into the session, then submit
    /// with Enter. Uses tmux `paste-buffer -p -r -d` semantics so
    /// the agent's TUI sees the whole block as one bracketed-paste
    /// event (LF stays literal — the runtime would otherwise
    /// translate LF → CR and submit per line).
    ///
    /// Sleeps 120ms between paste and Enter. Without this gap,
    /// Claude Code v2.1.x's input editor sometimes leaves pasted
    /// content sitting in the input box unsubmitted — the
    /// bracketed-paste end marker (`\e[201~`) and the trailing
    /// Enter arrive too close together for the TUI to transition
    /// out of paste mode before interpreting the keystroke.
    /// (Observed live, fix-port of the prior portable-pty path's
    /// 80ms gap between body-bytes and `\r` writes; tmux adds a
    /// little server-side queueing latency on top, hence the
    /// slightly larger 120ms.) `cfg(test)` keeps the same
    /// constant — fake runtimes complete instantly so the wait
    /// is harmless.
    pub fn inject_paste(&self, session_id: &str, payload: &[u8]) -> Result<()> {
        let rt_session = self
            .sessions
            .lock()
            .unwrap()
            .get(session_id)
            .map(|h| h.runtime_session.clone())
            .ok_or_else(|| Error::msg(format!("session not found: {session_id}")))?;
        self.runtime.paste(&rt_session, payload)?;
        std::thread::sleep(std::time::Duration::from_millis(120));
        self.runtime
            .send_key(&rt_session, "Enter")
            .map_err(Into::into)
    }

    /// Resize the session's pane. The frontend calls this after
    /// xterm fits its container — without it, claude-code stays at
    /// the spawn-time grid regardless of how big the visible grid
    /// is.
    pub fn resize(&self, session_id: &str, cols: u16, rows: u16) -> Result<()> {
        let rt_session = self
            .sessions
            .lock()
            .unwrap()
            .get(session_id)
            .map(|h| h.runtime_session.clone())
            .ok_or_else(|| Error::msg(format!("session not found: {session_id}")))?;
        self.runtime
            .resize(&rt_session, cols, rows)
            .map_err(Into::into)
    }

    /// Return the bounded in-memory PTY output snapshot for a session.
    ///
    /// Tauri events are live-only; without this, a terminal pane mounted after
    /// a session already produced output starts blank until the child redraws.
    /// The snapshot is intentionally process-local and bounded: it covers
    /// webview reloads / chat switching for live sessions without turning the
    /// sessions table into a PTY transcript store.
    pub fn output_snapshot(&self, session_id: &str) -> Vec<OutputEvent> {
        self.output_buffers
            .lock()
            .unwrap()
            .get(session_id)
            .map(|chunks| chunks.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Kill the child and wait for the reader thread to reap it.
    ///
    /// Sequence:
    ///   1. Remove the handle from the live map (no further `inject_stdin` /
    ///      `kill` can target it).
    ///   2. Drop the master PTY — the child receives SIGHUP and well-behaved
    ///      programs exit; the reader thread's `read()` returns 0.
    ///   3. On Unix, belt-and-suspenders: signal SIGTERM (then SIGKILL after
    ///      200 ms) so a child that ignores SIGHUP can't stall the reader.
    ///   4. Join the reader thread. It waits the child, updates the DB row
    ///      to stopped/crashed, emits `session/exit`. Only after this
    ///      returns is the caller allowed to consider the session dead —
    ///      which is what `mission_stop` needs in order to flip the mission
    ///      row without lying about termination.
    pub fn kill(&self, session_id: &str) -> Result<()> {
        // Mark the kill as intentional so the forwarder thread
        // classifies the upcoming non-zero exit as `stopped`, not
        // `crashed`. The runtime's `stop` typically lands the
        // pane in a non-zero exit (SIGTERM, etc.); without this
        // flag every user-initiated stop would surface as a crash.
        self.killed.lock().unwrap().insert(session_id.to_string());

        // Take the handle out of the live map so subsequent
        // `inject_stdin` / `kill` / `resize` against this id fail
        // fast; the forwarder thread's runtime_session was cloned
        // at insert time so it stays usable.
        let (rt_session, forwarder) = {
            let mut sessions = self.sessions.lock().unwrap();
            match sessions.remove(session_id) {
                Some(mut h) => (h.runtime_session.clone(), h.forwarder.take()),
                None => return Ok(()),
            }
        };

        // Stop the pane. The forwarder thread sees the channel
        // close, queries final status, and updates the DB +
        // emits exit on its way out.
        let _ = self.runtime.stop(&rt_session);

        // Wait for the forwarder to drain + reconcile so the
        // caller (mission_stop) gets the no-live-sessions-after-
        // we-return contract.
        if let Some(h) = forwarder {
            let _ = h.join();
        }
        Ok(())
    }

    /// Kill every live session; used on mission_stop and at app shutdown.
    /// Returns only after all reader threads have joined — callers rely on
    /// that for the "no live sessions after we return" contract.
    pub fn kill_all_for_mission(&self, mission_id: &str) -> Result<()> {
        let ids: Vec<String> = {
            let sessions = self.sessions.lock().unwrap();
            sessions
                .values()
                .filter(|s| s.mission_id.as_deref() == Some(mission_id))
                .map(|s| s.id.clone())
                .collect()
        };
        for id in ids {
            self.kill(&id)?;
        }
        Ok(())
    }

    /// Kill every live session for `runner_id` — both mission-scoped and
    /// direct-chat. Used by `runner_delete` so the cascade dropping the
    /// `sessions` rows doesn't strand the PTY children running underneath.
    /// Returns only after every reader thread has joined.
    pub fn kill_all_for_runner(&self, runner_id: &str) -> Result<()> {
        let ids: Vec<String> = {
            let sessions = self.sessions.lock().unwrap();
            sessions
                .values()
                .filter(|s| s.runner_id == runner_id)
                .map(|s| s.id.clone())
                .collect()
        };
        for id in ids {
            self.kill(&id)?;
        }
        Ok(())
    }

    /// App-startup reconciliation: for every `sessions` row still
    /// marked `running`, ask the runtime whether the pane is alive.
    /// If yes, reattach (rebuild the SessionHandle + forwarder
    /// thread) so Tauri commands can target the surviving pane. If
    /// the pane is gone or has exited, flip the row to
    /// stopped/crashed using the captured exit code.
    ///
    /// This replaces the prior portable-pty-era logic that
    /// indiscriminately marked every running row stopped on
    /// startup — that was correct when the manager owned the PTY
    /// lifecycle (process death = PTY death), but with tmux the
    /// pane survives Runner's process and we'd lose live agent
    /// sessions on every restart. Step 9 cutover follow-up.
    ///
    /// Best-effort. Errors per-row are logged to stderr; the
    /// overall reattach loop never fails the caller (app startup
    /// must not block on a transient runtime hiccup).
    pub fn reattach_running_sessions(
        self: &Arc<Self>,
        pool: Arc<DbPool>,
        events: Arc<dyn SessionEvents>,
    ) {
        let now = Utc::now().to_rfc3339();
        let rows: Vec<RowSnap> = match collect_running_rows(&pool) {
            Ok(rows) => rows,
            Err(e) => {
                eprintln!("runner: reattach query failed: {e}");
                return;
            }
        };
        for row in rows {
            self.reattach_one(row, &now, &pool, &events);
        }
    }

    fn reattach_one(
        self: &Arc<Self>,
        row: RowSnap,
        now: &str,
        pool: &Arc<DbPool>,
        events: &Arc<dyn SessionEvents>,
    ) {
        // No runtime metadata persisted (legacy row, or a row
        // that crashed before we got a chance to write the
        // runtime_* columns) → mark stopped and move on.
        let Some(rt_session) = row.runtime_session() else {
            mark_session_stopped(pool, &row.id, now);
            return;
        };

        // Mission sessions: refuse to reattach at startup. An
        // agent can keep appending bus events (`ask_lead`,
        // `human_said`, `runner_status`) while Runner is
        // closed, but the mission's bus + router don't mount
        // until `mission_attach` fires from the workspace UI,
        // and `router::mod` explicitly doesn't replay stdin
        // side effects on reconstruction. Reattaching the PTY
        // without the bus would silently miss those events.
        // Kill the pane and mark the row stopped; the user can
        // resume from the workspace, which mounts the bus +
        // router properly.
        //
        // Direct chats are off-bus by design (PR #51) so they
        // reattach unchanged — that's the survivability win the
        // migration was meant to deliver.
        if row.mission_id.is_some() {
            let _ = self.runtime.stop(&rt_session);
            mark_session_stopped(pool, &row.id, now);
            return;
        }

        match self.runtime.status(&rt_session) {
            Ok(Some(status)) if status.alive => {
                // Pane is still alive. Re-attach. On failure,
                // kill the orphan pane before marking the row
                // stopped — otherwise the agent keeps running
                // with no UI / DB record to reach it.
                let id = row.id.clone();
                let rt_for_cleanup = rt_session.clone();
                if let Err(e) = self.attach_existing(row, rt_session, pool, events) {
                    eprintln!("runner: reattach session {} failed: {e}", row_dbg(&id));
                    let _ = self.runtime.stop(&rt_for_cleanup);
                    mark_session_stopped(pool, &id, now);
                }
            }
            Ok(Some(status)) => {
                // Pane is dead but tmux is still holding it
                // (remain-on-exit). Capture the exit code, mark the
                // row, then tear down the dead pane.
                let final_status = if status.exit_code == Some(0) {
                    "stopped"
                } else {
                    "crashed"
                };
                let _ = self.runtime.stop(&rt_session);
                if let Ok(conn) = pool.get() {
                    let _ = conn.execute(
                        "UPDATE sessions
                            SET status = ?2,
                                stopped_at = COALESCE(stopped_at, ?3)
                          WHERE id = ?1",
                        params![row.id, final_status, now],
                    );
                }
            }
            Ok(None) | Err(_) => {
                // tmux can't find the pane — terminal-unavailable.
                mark_session_stopped(pool, &row.id, now);
            }
        }
    }

    fn attach_existing(
        self: &Arc<Self>,
        row: RowSnap,
        rt_session: RuntimeSession,
        pool: &Arc<DbPool>,
        events: &Arc<dyn SessionEvents>,
    ) -> Result<()> {
        // Pull the runner row so the forwarder thread can fire
        // `runner/activity` events with the right handle.
        let runner = {
            let conn = pool.get()?;
            crate::commands::runner::get(&conn, &row.runner_id)?
        };
        let output = self.runtime.resume(&rt_session)?;
        self.sessions.lock().unwrap().insert(
            row.id.clone(),
            SessionHandle {
                id: row.id.clone(),
                mission_id: row.mission_id.clone(),
                runner_id: row.runner_id.clone(),
                runtime_session: rt_session.clone(),
                forwarder: None,
            },
        );
        let forwarder = self.start_forwarder_thread(
            row.id.clone(),
            row.mission_id,
            rt_session,
            output,
            Arc::clone(pool),
            Arc::clone(events),
            runner,
            false, // resuming flag — re-attach to a live pane is not a resume_plan resume
        );
        if let Some(h) = self.sessions.lock().unwrap().get_mut(&row.id) {
            h.forwarder = Some(forwarder);
        }
        Ok(())
    }

    fn forget(&self, session_id: &str) -> Result<()> {
        // Only the live PTY handle is dropped here. We deliberately keep
        // `output_buffers` and `output_seq` alive so that:
        //   - `session_output_snapshot` still returns the dead session's
        //     scrollback after kill, so navigating off the chat and
        //     coming back doesn't blank the terminal.
        //   - When the row is later resumed via `SessionManager::resume`,
        //     the new PTY's first chunk continues at `seq = last + 1`
        //     instead of restarting at 1, which the frontend's
        //     seq-merge filter (`seq <= lastWrittenSeq`) would silently
        //     drop, losing the entire post-resume head of output.
        // Use `purge_session_buffers` for explicit cleanup paths
        // (archive, runner delete).
        self.sessions.lock().unwrap().remove(session_id);
        Ok(())
    }

    /// Drop the in-memory output buffer + seq counter for a session.
    /// Called when the session is genuinely going away (archive, runner
    /// delete) so the bounded ring buffer doesn't accumulate forever.
    /// Safe to call on a session that's never written output.
    pub fn purge_session_buffers(&self, session_id: &str) {
        self.output_buffers.lock().unwrap().remove(session_id);
        self.output_seq.lock().unwrap().remove(session_id);
    }

    /// Drop only the output buffer for a session, keeping the seq
    /// counter. Used by `resume`: clearing the buffer means the
    /// post-resume snapshot is fresh (no double banner / stacked
    /// agent output on remount), while preserving the monotonic seq
    /// means the new PTY's first chunk is `last + 1` rather than
    /// `1` — which the frontend's `seq <= lastWrittenSeq` filter
    /// would otherwise drop.
    pub fn purge_output_buffer(&self, session_id: &str) {
        self.output_buffers.lock().unwrap().remove(session_id);
    }

    fn record_output(
        &self,
        session_id: &str,
        mission_id: Option<&str>,
        data: String,
    ) -> OutputEvent {
        let seq = {
            let mut seqs = self.output_seq.lock().unwrap();
            let next = seqs.entry(session_id.to_string()).or_insert(0);
            *next += 1;
            *next
        };

        let ev = OutputEvent {
            session_id: session_id.into(),
            mission_id: mission_id.map(str::to_string),
            seq,
            data,
        };

        let mut buffers = self.output_buffers.lock().unwrap();
        let chunks = buffers.entry(session_id.to_string()).or_default();
        chunks.push_back(ev.clone());
        while chunks.len() > MAX_OUTPUT_BUFFER_CHUNKS {
            chunks.pop_front();
        }
        ev
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
/// Subset of the `sessions` row needed to reattach to a live pane
/// at app startup. Pulled all-at-once so the conn drops before we
/// start hitting the runtime layer per row.
#[derive(Debug, Clone)]
struct RowSnap {
    id: String,
    runner_id: String,
    mission_id: Option<String>,
    runtime: Option<String>,
    runtime_socket: Option<String>,
    runtime_session: Option<String>,
    runtime_window: Option<String>,
    runtime_pane: Option<String>,
}

impl RowSnap {
    /// Reconstruct the `RuntimeSession` that the original spawn
    /// persisted into the runtime_* columns. Returns `None` for
    /// any row missing pieces — the caller treats that as a
    /// legacy row and marks it stopped.
    fn runtime_session(&self) -> Option<RuntimeSession> {
        Some(RuntimeSession {
            runtime: self.runtime.clone()?,
            socket: self.runtime_socket.clone()?,
            session_name: self.runtime_session.clone()?,
            window: self.runtime_window.clone()?,
            pane: self.runtime_pane.clone()?,
        })
    }
}

fn collect_running_rows(pool: &DbPool) -> Result<Vec<RowSnap>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, runner_id, mission_id,
                runtime, runtime_socket, runtime_session,
                runtime_window, runtime_pane
           FROM sessions
          WHERE status = 'running'",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(RowSnap {
                id: r.get("id")?,
                runner_id: r.get("runner_id")?,
                mission_id: r.get("mission_id")?,
                runtime: r.get("runtime")?,
                runtime_socket: r.get("runtime_socket")?,
                runtime_session: r.get("runtime_session")?,
                runtime_window: r.get("runtime_window")?,
                runtime_pane: r.get("runtime_pane")?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn mark_session_stopped(pool: &DbPool, id: &str, now: &str) {
    if let Ok(conn) = pool.get() {
        let _ = conn.execute(
            "UPDATE sessions
                SET status = 'stopped',
                    stopped_at = COALESCE(stopped_at, ?2)
              WHERE id = ?1",
            params![id, now],
        );
    }
}

/// Trim a session id for stderr logging — show just enough to
/// identify the row without dumping a full ULID into the log line.
fn row_dbg(id: &str) -> &str {
    if id.len() <= 8 {
        id
    } else {
        &id[id.len() - 8..]
    }
}

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

/// How long to wait after spawn before typing the worker's first
/// user turn into the PTY. claude-code and codex both need their TUIs
/// to render the welcome banner, dismiss any "trust this folder" /
/// approval dialog, and bind their raw-mode keypress reader before
/// typed bytes land — anything shorter and the early bytes get
/// swallowed by a dialog still on screen. 500ms wasn't enough in
/// practice (the worker came up without its preamble on warm
/// boots), so we hold the 2.5s budget pending a real
/// readback-verification fix (#50). `cfg(test)` zeros it so unit
/// tests don't have to sleep multiple seconds (we run inline at the
/// call site when zero). Mirrors `LEAD_LAUNCH_PROMPT_DELAY` in
/// `router/handlers.rs` since they solve the same race.
#[cfg(not(test))]
const FIRST_PROMPT_DELAY: std::time::Duration = std::time::Duration::from_millis(2500);
#[cfg(test)]
const FIRST_PROMPT_DELAY: std::time::Duration = std::time::Duration::ZERO;

/// Mission-flavored first-turn injection. Composes the platform
/// coordination preamble (bus mechanics, --to human convention,
/// signal verbs) followed by the user-authored brief on the runner
/// template. Keeping bus protocol out of the user's system_prompt
/// means template authors can focus on persona/role; the runtime
/// adds the "how to talk to the rest of the crew" layer
/// automatically.
///
/// `suppress_lead_preamble` is set by the initial mission_start
/// spawn path: there, the bus's `mission_goal` handler injects a
/// richer launch prompt with `system_prompt` embedded in its "Your
/// brief" section, so a separate first-turn injection would race the
/// launch prompt and waste a turn. On a resume that degrades to a
/// fresh spawn (claude-code conversation file went missing — see
/// `claude_code_conversation_exists`) the bus does NOT replay
/// `mission_goal`, so the lead would otherwise come up with no
/// system context; the resume path passes `false` here so the
/// preamble + system_prompt land via this stdin-injection route
/// instead.
///
/// Skipped on resume against a real prior conversation (the agent
/// already has its system context) and on runtimes that have no
/// concept of a first-turn prompt (shell).
fn schedule_mission_first_prompt(
    mgr: &Arc<SessionManager>,
    session_id: String,
    runner: &Runner,
    plan: &router::runtime::ResumePlan,
    suppress_lead_preamble: bool,
) {
    if runner.runtime != "claude-code" && runner.runtime != "codex" {
        return;
    }
    if plan.resuming {
        return;
    }
    if suppress_lead_preamble {
        return;
    }
    let user_brief = runner
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let mut prompt = String::new();
    prompt.push_str(WORKER_COORDINATION_PREAMBLE);
    if let Some(brief) = user_brief {
        prompt.push_str("\n\n== Your brief ==\n");
        prompt.push_str(&brief);
    }
    inject_first_turn(mgr, session_id, prompt);
}

/// Direct-chat-flavored first-turn injection: types just
/// `runner.system_prompt` (the persona) into stdin, with NO
/// `WORKER_COORDINATION_PREAMBLE` wrapper. Direct chats are off-bus —
/// `runner msg post`, `runner status idle`, etc. wouldn't resolve to
/// anything useful here (no `RUNNER_CREW_ID` / `RUNNER_MISSION_ID`
/// set, the bundled CLI is not even on PATH). Adding the preamble
/// would tell the agent to use verbs that don't exist in this
/// context, which is worse than no instructions at all.
///
/// If `runner.system_prompt` is empty / None, no injection happens —
/// claude-code direct chat then boots vanilla, which is the
/// honest fallback for that edge case.
///
/// Skipped on resume (the agent already has its prior conversation)
/// and on runtimes without a first-turn-prompt concept (shell).
fn schedule_direct_first_prompt(
    mgr: &Arc<SessionManager>,
    session_id: String,
    runner: &Runner,
    plan: &router::runtime::ResumePlan,
) {
    if runner.runtime != "claude-code" && runner.runtime != "codex" {
        return;
    }
    if plan.resuming {
        return;
    }
    let Some(persona) = runner
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
    else {
        return;
    };
    inject_first_turn(mgr, session_id, persona);
}

/// Shared mechanics for typing a first user turn into the PTY.
/// Strips any embedded `\r` so the prompt body is one piece;
/// embedded `\n`s render as line breaks inside the input box. The
/// submit byte goes in a separate write so the TUI sees it as Enter
/// rather than appending it to the input buffer (which is what
/// happens when text + `\r` arrive in the same chunk). Production
/// runs on a 2.5s settle thread; `cfg(test)` zeros the delay and
/// runs inline so unit tests can assert synchronously.
fn inject_first_turn(mgr: &Arc<SessionManager>, session_id: String, prompt: String) {
    // Strip embedded \r so the prompt body is one piece of text;
    // embedded \n inside the body renders as a line break inside
    // the agent's input box. `inject_paste` handles the LF→CR
    // semantics correctly (paste-buffer -p -r -d preserves LF and
    // emits a separate Enter for submit), so we don't need the
    // old two-write split-pattern here.
    let body: String = prompt.chars().filter(|c| *c != '\r').collect();
    let delay = FIRST_PROMPT_DELAY;
    if delay.is_zero() {
        // Inline path used by unit tests (`FIRST_PROMPT_DELAY = ZERO`
        // under `cfg(test)`) so synchronous output assertions can
        // observe the injection without waiting on a background
        // thread. Production never hits this branch.
        let _ = mgr.inject_paste(&session_id, body.as_bytes());
        return;
    }
    let mgr = Arc::clone(mgr);
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        let _ = mgr.inject_paste(&session_id, body.as_bytes());
    });
}

/// Platform-injected preamble for non-lead worker spawns. Covers the
/// bus conventions a worker needs to interact with the crew + the
/// human, leaving the user-authored `system_prompt` free to focus on
/// persona / role. Sent as the first user turn (before any task
/// dispatch from the lead) by `schedule_first_prompt`.
const WORKER_COORDINATION_PREAMBLE: &str = r#"You are a worker in a crew coordinated by the bundled `runner` CLI. The CLI is on your PATH and talks to the rest of the crew + the human operator via a shared event bus. Use these verbs to participate; do not invent your own conventions.

== Coordination ==
- `runner msg read` — read your inbox (pull-based: new messages do NOT auto-print). Run this when you see an `[inbox]` notification or any time you suspect new traffic.
- `runner msg post --to <handle> "<text>"` — direct message to a specific handle. Valid handles: any slot in this crew, plus the reserved virtual handle `human` (the workspace operator).
- `runner msg post "<text>"` — broadcast to the crew (no `--to`).
- `runner signal ask_lead --payload '{"question":"…","context":"…"}'` — escalate to the lead when a load-bearing decision is genuinely ambiguous.
- `runner status idle` — report you've finished the current task. The lead view uses this to dispatch the next slot.

== Replying to the human ==
The human is watching the workspace feed, NOT your TUI. When the human speaks to you directly (raw input lands in your TUI, often prefixed with `[human_said]`), reply via:
    runner msg post --to human "<your reply>"
Plain TUI output (typing into your editor, printing to stdout) stays in your local scrollback only — it never reaches the human. The `--to human` route is the only way your reply lands in the workspace feed."#;

/// Auto-send "continue" as a first user turn after a successful
/// resume so the agent picks up where it left off without the user
/// having to manually nudge it. Only fires when the resume actually
/// reloaded a prior conversation (`plan.resuming == true` AND we
/// have an `agent_session_key` to point claude-code at). For
/// runtimes that don't have a real "resume" semantic (shell, or
/// codex pre-capture), no-op — there's no conversation thread to
/// continue.
///
/// Same split-injection pattern as `schedule_first_prompt`: body
/// first, then a separate `\r` after a small delay so claude-code's
/// editor sees the carriage return as Enter rather than appending
/// it to the input buffer.
fn schedule_continue_on_resume(
    mgr: &Arc<SessionManager>,
    session_id: String,
    runner: &Runner,
    plan: &router::runtime::ResumePlan,
) {
    if runner.runtime != "claude-code" {
        return;
    }
    if !plan.resuming {
        return;
    }
    let mgr = Arc::clone(mgr);
    std::thread::spawn(move || {
        // Same 2.5s budget as `FIRST_PROMPT_DELAY` — claude-code
        // shows the prior conversation history first, and we want
        // the editor bound before typing.
        std::thread::sleep(std::time::Duration::from_millis(2500));
        let _ = mgr.inject_paste(&session_id, b"continue");
    });
}

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
              WHERE runner_id = ?1 AND status = 'running' AND mission_id IS NULL
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

/// Pumps PTY output → `session/output` events, then waits for the child to
/// exit. Returns the exit summary that the caller emits as `session/exit`.
/// `mission_id` is `None` for direct-chat sessions.
#[cfg(test)]
mod tests {
    use super::*;

    // These tests don't touch Tauri — they hit the PTY layer directly. We
    // build a minimal `Runner` row, skip the DB (the SessionManager writes
    // to DB on spawn), and cover: spawn-echo-readback, inject-stdin-roundtrip,
    // and exit-emits-correct-status. For DB coverage we use the app's
    // file-backed pool helper.

    use crate::db;
    use crate::model::{MissionStatus, Runner};
    use crate::session::runtime::{
        OutputStream, RuntimeError, RuntimeResult, RuntimeSession, SessionRuntime, SessionStatus,
        SpawnSpec,
    };
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    /// Test stand-in for `SessionRuntime`. Step 9 wires
    /// `SessionManager` to hold an `Arc<dyn SessionRuntime>` so the
    /// runtime layer is always present, but most legacy tests
    /// exercise the portable-pty path through the manager and never
    /// touch the runtime field. This stub errors on every method —
    /// any test that *does* land in the runtime layer would surface
    /// it, and intentional runtime tests live in
    /// `session::tmux_runtime::tests` instead.
    struct InertRuntime;
    impl SessionRuntime for InertRuntime {
        fn spawn(&self, _: SpawnSpec) -> RuntimeResult<(RuntimeSession, OutputStream)> {
            Err(RuntimeError::Msg(
                "InertRuntime: spawn unsupported in unit tests".into(),
            ))
        }
        fn resume(&self, _: &RuntimeSession) -> RuntimeResult<OutputStream> {
            Err(RuntimeError::Msg("InertRuntime: resume unsupported".into()))
        }
        fn stop(&self, _: &RuntimeSession) -> RuntimeResult<()> {
            Err(RuntimeError::Msg("InertRuntime: stop unsupported".into()))
        }
        fn paste(&self, _: &RuntimeSession, _: &[u8]) -> RuntimeResult<()> {
            Err(RuntimeError::Msg("InertRuntime: paste unsupported".into()))
        }
        fn send_bytes(&self, _: &RuntimeSession, _: &[u8]) -> RuntimeResult<()> {
            Err(RuntimeError::Msg(
                "InertRuntime: send_bytes unsupported".into(),
            ))
        }
        fn send_key(&self, _: &RuntimeSession, _: &str) -> RuntimeResult<()> {
            Err(RuntimeError::Msg(
                "InertRuntime: send_key unsupported".into(),
            ))
        }
        fn resize(&self, _: &RuntimeSession, _: u16, _: u16) -> RuntimeResult<()> {
            Err(RuntimeError::Msg("InertRuntime: resize unsupported".into()))
        }
        fn status(&self, _: &RuntimeSession) -> RuntimeResult<Option<SessionStatus>> {
            Err(RuntimeError::Msg("InertRuntime: status unsupported".into()))
        }
    }

    fn inert_runtime() -> Arc<dyn SessionRuntime> {
        Arc::new(InertRuntime)
    }

    /// Test stand-in that captures every call so assertions can read
    /// back what the manager handed to the runtime layer (env vars,
    /// argv, paste payloads, key names, resize dimensions). Lets
    /// tests that depend on runtime-side behavior — DB writes after
    /// spawn, output buffer machinery, kill semantics, first-prompt
    /// scheduling, agent_session_key resume preservation — run
    /// without a real tmux server. Real tmux interaction lives in
    /// `session::tmux_runtime::tests::integration_*`.
    #[derive(Default)]
    struct FakeRuntime {
        spawns: std::sync::Mutex<Vec<FakeSpawn>>,
        inputs: std::sync::Mutex<Vec<FakeInput>>,
        stops: std::sync::Mutex<Vec<String>>,
        resizes: std::sync::Mutex<Vec<(String, u16, u16)>>,
        /// What `status()` returns for any pane lookup. Most tests
        /// want exit_code=0 (clean stop); the kill-semantics test
        /// wants exit_code=143 (SIGTERM) to verify the
        /// stop-vs-crash discrimination still flips correctly.
        status_response: std::sync::Mutex<SessionStatus>,
    }

    /// One spawn/resume capture. `tx` is the live channel the
    /// forwarder thread is reading; tests can `push_output` to
    /// emit fake bytes or `close` to simulate exit.
    struct FakeSpawn {
        spec: SpawnSpec,
        rt_session: RuntimeSession,
        tx: Option<std::sync::mpsc::Sender<RuntimeOutput>>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum FakeInput {
        Paste { pane: String, payload: Vec<u8> },
        Bytes { pane: String, bytes: Vec<u8> },
        Key { pane: String, key: String },
    }

    impl FakeRuntime {
        fn new() -> Self {
            Self {
                status_response: std::sync::Mutex::new(SessionStatus {
                    alive: false,
                    exit_code: Some(0),
                    pid: Some(99999),
                    command: Some("/bin/sh".into()),
                }),
                ..Default::default()
            }
        }

        /// Push a `Stream` event through the forwarder channel for
        /// the spawn at index `i`. Returns Err if the channel was
        /// already closed (test-side error).
        fn push_output(&self, i: usize, bytes: &[u8]) {
            let spawns = self.spawns.lock().unwrap();
            if let Some(tx) = spawns.get(i).and_then(|s| s.tx.as_ref()) {
                let _ = tx.send(RuntimeOutput::Stream(bytes.to_vec()));
            }
        }

        /// Drop the `Sender` for spawn `i` so the forwarder thread
        /// sees `Disconnected` and exits — the manager-side path
        /// that simulates a pane dying cleanly.
        fn close_spawn(&self, i: usize) {
            let mut spawns = self.spawns.lock().unwrap();
            if let Some(s) = spawns.get_mut(i) {
                s.tx = None;
            }
        }

        /// Update the canned `status()` reply. Use to make the
        /// next `kill`/exit reconciliation observe a non-zero exit
        /// code. (Reserved for future tests; currently every
        /// converted test runs against the default exit_code=0.)
        #[allow(dead_code)]
        fn set_status_exit_code(&self, code: Option<i32>) {
            let mut s = self.status_response.lock().unwrap();
            s.exit_code = code;
        }

        fn spawn_count(&self) -> usize {
            self.spawns.lock().unwrap().len()
        }

        fn last_spawn_spec(&self) -> Option<SpawnSpec> {
            self.spawns.lock().unwrap().last().map(|s| s.spec.clone())
        }

        fn pastes(&self) -> Vec<(String, Vec<u8>)> {
            self.inputs
                .lock()
                .unwrap()
                .iter()
                .filter_map(|i| match i {
                    FakeInput::Paste { pane, payload } => Some((pane.clone(), payload.clone())),
                    _ => None,
                })
                .collect()
        }

        fn keys(&self) -> Vec<(String, String)> {
            self.inputs
                .lock()
                .unwrap()
                .iter()
                .filter_map(|i| match i {
                    FakeInput::Key { pane, key } => Some((pane.clone(), key.clone())),
                    _ => None,
                })
                .collect()
        }

        fn bytes_writes(&self) -> Vec<(String, Vec<u8>)> {
            self.inputs
                .lock()
                .unwrap()
                .iter()
                .filter_map(|i| match i {
                    FakeInput::Bytes { pane, bytes } => Some((pane.clone(), bytes.clone())),
                    _ => None,
                })
                .collect()
        }
    }

    impl SessionRuntime for FakeRuntime {
        fn spawn(&self, spec: SpawnSpec) -> RuntimeResult<(RuntimeSession, OutputStream)> {
            let (tx, rx) = std::sync::mpsc::channel::<RuntimeOutput>();
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let rt_session = RuntimeSession {
                runtime: "fake".into(),
                socket: "fake".into(),
                session_name: format!("runner-{}", spec.session_id),
                window: "main".into(),
                pane: format!("%{}", spec.session_id),
            };
            self.spawns.lock().unwrap().push(FakeSpawn {
                spec: spec.clone(),
                rt_session: rt_session.clone(),
                tx: Some(tx),
            });
            Ok((rt_session, OutputStream::new(rx, stop)))
        }

        fn resume(&self, session: &RuntimeSession) -> RuntimeResult<OutputStream> {
            let (tx, rx) = std::sync::mpsc::channel::<RuntimeOutput>();
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            self.spawns.lock().unwrap().push(FakeSpawn {
                spec: SpawnSpec {
                    session_id: session
                        .session_name
                        .strip_prefix("runner-")
                        .unwrap_or("")
                        .to_string(),
                    ..Default::default()
                },
                rt_session: session.clone(),
                tx: Some(tx),
            });
            Ok(OutputStream::new(rx, stop))
        }

        fn stop(&self, session: &RuntimeSession) -> RuntimeResult<()> {
            self.stops
                .lock()
                .unwrap()
                .push(session.session_name.clone());
            // Drop the matching tx so the forwarder sees Disconnected.
            let target_pane = session.pane.clone();
            let mut spawns = self.spawns.lock().unwrap();
            for s in spawns.iter_mut() {
                if s.rt_session.pane == target_pane {
                    s.tx = None;
                }
            }
            Ok(())
        }

        fn paste(&self, session: &RuntimeSession, payload: &[u8]) -> RuntimeResult<()> {
            self.inputs.lock().unwrap().push(FakeInput::Paste {
                pane: session.pane.clone(),
                payload: payload.to_vec(),
            });
            Ok(())
        }

        fn send_bytes(&self, session: &RuntimeSession, bytes: &[u8]) -> RuntimeResult<()> {
            self.inputs.lock().unwrap().push(FakeInput::Bytes {
                pane: session.pane.clone(),
                bytes: bytes.to_vec(),
            });
            Ok(())
        }

        fn send_key(&self, session: &RuntimeSession, key: &str) -> RuntimeResult<()> {
            self.inputs.lock().unwrap().push(FakeInput::Key {
                pane: session.pane.clone(),
                key: key.to_string(),
            });
            Ok(())
        }

        fn resize(&self, session: &RuntimeSession, cols: u16, rows: u16) -> RuntimeResult<()> {
            self.resizes
                .lock()
                .unwrap()
                .push((session.session_name.clone(), cols, rows));
            Ok(())
        }

        fn status(&self, _: &RuntimeSession) -> RuntimeResult<Option<SessionStatus>> {
            Ok(Some(self.status_response.lock().unwrap().clone()))
        }
    }

    fn fake_runtime() -> Arc<FakeRuntime> {
        Arc::new(FakeRuntime::new())
    }

    /// Build a manager backed by the supplied FakeRuntime. Returns
    /// the Arc so tests can introspect the captured calls.
    fn mgr_with_fake(shell: Option<String>, fake: Arc<FakeRuntime>) -> Arc<SessionManager> {
        SessionManager::new(shell, fake)
    }

    /// Test emitter that just records every event. Replaces the Tauri
    /// `AppHandle` in unit tests — no runtime dependency.
    #[derive(Default)]
    struct Capture {
        output: Mutex<Vec<OutputEvent>>,
        exit: Mutex<Vec<ExitEvent>>,
        activity: Mutex<Vec<RunnerActivityEvent>>,
    }
    impl SessionEvents for Capture {
        fn output(&self, ev: &OutputEvent) {
            self.output.lock().unwrap().push(ev.clone());
        }
        fn exit(&self, ev: &ExitEvent) {
            self.exit.lock().unwrap().push(ev.clone());
        }
        fn runner_activity(&self, ev: &RunnerActivityEvent) {
            self.activity.lock().unwrap().push(ev.clone());
        }
    }

    fn runner(command: &str, args: &[&str]) -> Runner {
        Runner {
            id: ulid::Ulid::new().to_string(),
            handle: "tester".into(),
            display_name: "Tester".into(),
            runtime: "shell".into(),
            command: command.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            working_dir: None,
            system_prompt: None,
            env: HashMap::new(),
            model: None,
            effort: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn slot_for(runner: &Runner) -> crate::model::Slot {
        crate::model::Slot {
            id: ulid::Ulid::new().to_string(),
            crew_id: "c".into(),
            runner_id: runner.id.clone(),
            slot_handle: runner.handle.clone(),
            position: 0,
            lead: true,
            added_at: Utc::now(),
        }
    }

    fn mission() -> Mission {
        Mission {
            id: ulid::Ulid::new().to_string(),
            crew_id: "crew-ignored-in-tests".into(),
            title: "t".into(),
            status: MissionStatus::Running,
            goal_override: None,
            cwd: None,
            started_at: Utc::now(),
            stopped_at: None,
            pinned_at: None,
        }
    }

    fn capture() -> Arc<Capture> {
        Arc::new(Capture::default())
    }

    fn pool_with_schema() -> Arc<DbPool> {
        let tmp = tempfile::tempdir().unwrap();
        // Leak the tempdir so the DB file outlives this fn; fine in tests.
        let path = tmp.path().join("c6.db");
        std::mem::forget(tmp);
        Arc::new(db::open_pool(&path).unwrap())
    }

    fn insert_crew_runner(pool: &DbPool, mission_id: &str, runner_id: &str) -> String {
        // Satisfy the FKs the `sessions` INSERT needs (crew, global runner,
        // slot, mission) and return the slot id so the caller can build a
        // matching `Slot` to hand to `spawn`. Post-crew-slots, membership
        // lives on `slots` and runners no longer carry `role`.
        let conn = pool.get().unwrap();
        let now = Utc::now().to_rfc3339();
        let slot_id = ulid::Ulid::new().to_string();
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES ('c', 'c', ?1, ?1)",
            params![now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO runners
                (id, handle, display_name, runtime, command,
                 args_json, working_dir, system_prompt, env_json,
                 created_at, updated_at)
             VALUES (?1, 't', 'T', 'shell', '/bin/sh',
                     NULL, NULL, NULL, NULL, ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO slots
                (id, crew_id, runner_id, slot_handle, position, lead, added_at)
             VALUES (?1, 'c', ?2, 't', 0, 1, ?3)",
            params![slot_id, runner_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO missions (id, crew_id, title, status, started_at)
             VALUES (?1, 'c', 't', 'running', ?2)",
            params![mission_id, now],
        )
        .unwrap();
        slot_id
    }

    // `compose_path` moved to `session::launch::compose_path` as
    // part of the Step 9 cutover; equivalent coverage lives in
    // `session::launch::tests::compose_path_*`.

    #[test]
    fn concurrent_missions_on_same_crew_keep_session_state_isolated() {
        // Per #55 the per-crew "at most one live mission" guard was
        // lifted. The contract that makes that safe is mission-id
        // namespacing: `sessions.mission_id` is a foreign key,
        // `kill_all_for_mission` filters on `mission_id`, the runner
        // CLI shim path is keyed by mission_id, etc. This test pins
        // the session-isolation half of that contract: spawn one
        // session per mission against the same crew + same runner
        // template, assert both alive concurrently, then assert
        // `kill_all_for_mission(A)` reaps A's session and leaves B's
        // alone.
        let pool = pool_with_schema();
        let runner_id = ulid::Ulid::new().to_string();
        let crew_id = "c-concurrent".to_string();
        let slot_id = ulid::Ulid::new().to_string();
        let mission_a = ulid::Ulid::new().to_string();
        let mission_b = ulid::Ulid::new().to_string();
        let now = Utc::now().to_rfc3339();
        {
            let conn = pool.get().unwrap();
            conn.execute(
                "INSERT INTO crews (id, name, created_at, updated_at)
                 VALUES (?1, 'c', ?2, ?2)",
                params![crew_id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, working_dir, system_prompt, env_json,
                     created_at, updated_at)
                 VALUES (?1, 'concurrent', 'C', 'shell', '/bin/cat',
                         NULL, NULL, NULL, NULL, ?2, ?2)",
                params![runner_id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO slots
                    (id, crew_id, runner_id, slot_handle, position, lead, added_at)
                 VALUES (?1, ?2, ?3, 'concurrent', 0, 1, ?4)",
                params![slot_id, crew_id, runner_id, now],
            )
            .unwrap();
            for mid in [&mission_a, &mission_b] {
                conn.execute(
                    "INSERT INTO missions (id, crew_id, title, status, started_at)
                     VALUES (?1, ?2, 't', 'running', ?3)",
                    params![mid, crew_id, now],
                )
                .unwrap();
            }
        }

        let mut runner = runner("/bin/cat", &[]);
        runner.id = runner_id.clone();
        runner.handle = "concurrent".into();
        let mut slot = slot_for(&runner);
        slot.id = slot_id.clone();
        slot.crew_id = crew_id.clone();

        let mission_row_a = Mission {
            id: mission_a.clone(),
            crew_id: crew_id.clone(),
            ..mission()
        };
        let mission_row_b = Mission {
            id: mission_b.clone(),
            crew_id: crew_id.clone(),
            ..mission()
        };

        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        let spawned_a = mgr
            .spawn(
                &mission_row_a,
                &runner,
                &slot,
                std::path::Path::new("/tmp"),
                PathBuf::from("/dev/null"),
                Arc::clone(&pool),
                capture(),
            )
            .unwrap();
        let spawned_b = mgr
            .spawn(
                &mission_row_b,
                &runner,
                &slot,
                std::path::Path::new("/tmp"),
                PathBuf::from("/dev/null"),
                Arc::clone(&pool),
                capture(),
            )
            .unwrap();
        assert_ne!(
            spawned_a.id, spawned_b.id,
            "two missions on the same crew must produce distinct session ids",
        );

        // Both sessions live in the SessionManager's map at this point
        // — /bin/cat reads stdin until EOF, so neither has exited yet.
        {
            let sessions = mgr.sessions.lock().unwrap();
            assert!(
                sessions.contains_key(&spawned_a.id),
                "session A must be live"
            );
            assert!(
                sessions.contains_key(&spawned_b.id),
                "session B must be live"
            );
        }

        // Reap mission A's sessions only. The filter on mission_id must
        // leave B untouched.
        mgr.kill_all_for_mission(&mission_a).unwrap();

        // After kill_all_for_mission, A's reader thread joins via
        // SessionManager::kill (which awaits the join), so A's row is
        // already terminal in the DB. B is still running.
        let status_a: String = pool
            .get()
            .unwrap()
            .query_row(
                "SELECT status FROM sessions WHERE id = ?1",
                params![spawned_a.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_ne!(status_a, "running", "mission A's session must be reaped");

        {
            let sessions = mgr.sessions.lock().unwrap();
            assert!(
                !sessions.contains_key(&spawned_a.id),
                "mission A's session must be removed from the live map",
            );
            assert!(
                sessions.contains_key(&spawned_b.id),
                "mission B's session must survive kill_all_for_mission(A)",
            );
        }
        let status_b: String = pool
            .get()
            .unwrap()
            .query_row(
                "SELECT status FROM sessions WHERE id = ?1",
                params![spawned_b.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            status_b, "running",
            "mission B's session row must still be running",
        );

        // Cleanup so the test's PTY child doesn't outlive the test.
        mgr.kill(&spawned_b.id).unwrap();
    }

    #[test]
    fn spawn_marks_session_stopped_after_runtime_channel_closes() {
        // Spawn a mission session through FakeRuntime, then close
        // the runtime's output channel to simulate a clean pane exit.
        // The forwarder thread should query status (FakeRuntime
        // returns exit_code=0 by default), flip the DB row to
        // 'stopped', and emit ExitEvent with success=true.
        let pool = pool_with_schema();
        let mission = mission();
        let mut runner = runner("/bin/sh", &["-c", "echo hi"]);
        insert_crew_runner(&pool, &mission.id, &runner.id);
        runner.id = {
            let conn = pool.get().unwrap();
            let id: String = conn
                .query_row("SELECT id FROM runners LIMIT 1", [], |r| r.get(0))
                .unwrap();
            id
        };
        let fresh_mission_id = {
            let conn = pool.get().unwrap();
            let id: String = conn
                .query_row("SELECT id FROM missions LIMIT 1", [], |r| r.get(0))
                .unwrap();
            id
        };
        let mission = Mission {
            id: fresh_mission_id,
            ..mission
        };

        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        let cap = capture();
        let slot = slot_for(&runner);
        let spawned = mgr
            .spawn(
                &mission,
                &runner,
                &slot,
                std::path::Path::new("/tmp"),
                PathBuf::from("/dev/null"),
                Arc::clone(&pool),
                Arc::clone(&cap) as Arc<dyn SessionEvents>,
            )
            .unwrap();
        // pid is no longer pre-known on spawn return — the runtime
        // surfaces it lazily via status() once the manager needs it.
        assert!(spawned.pid.is_none());
        assert_eq!(fake.spawn_count(), 1);

        // Simulate a clean pane exit.
        fake.close_spawn(0);

        // Poll the DB until the forwarder thread has marked the session stopped.
        let deadline = Instant::now() + Duration::from_secs(2);
        let final_status = loop {
            let conn = pool.get().unwrap();
            let status: String = conn
                .query_row(
                    "SELECT status FROM sessions WHERE id = ?1",
                    params![spawned.id],
                    |r| r.get(0),
                )
                .unwrap();
            if status != "running" {
                break status;
            }
            if Instant::now() > deadline {
                panic!("session never exited");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(final_status, "stopped");

        // Exit event should have fired with success=true.
        let exits = cap.exit.lock().unwrap();
        assert_eq!(exits.len(), 1, "expected 1 exit event, got {}", exits.len());
        assert!(exits[0].success);
    }

    #[test]
    fn inject_stdin_roundtrip_routes_through_runtime() {
        // After the Step 9 cutover, inject_stdin no longer writes to
        // a master PTY — it routes through `runtime.send_bytes`
        // (literal byte stream) or `runtime.send_key("Enter")` (the
        // bare `\r` carve-out). FakeRuntime captures both; assert
        // the byte payload landed in `bytes_writes`, then bare `\r`
        // routed as a key press, then kill flips the row.
        let pool = pool_with_schema();
        let mission = mission();
        let mut runner = runner("/bin/cat", &[]);
        insert_crew_runner(&pool, &mission.id, &runner.id);
        runner.id = {
            let conn = pool.get().unwrap();
            conn.query_row("SELECT id FROM runners LIMIT 1", [], |r| r.get(0))
                .unwrap()
        };
        let fresh_mission_id = {
            let conn = pool.get().unwrap();
            conn.query_row("SELECT id FROM missions LIMIT 1", [], |r| r.get(0))
                .unwrap()
        };
        let mission = Mission {
            id: fresh_mission_id,
            ..mission
        };

        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        let slot = slot_for(&runner);
        let spawned = mgr
            .spawn(
                &mission,
                &runner,
                &slot,
                std::path::Path::new("/tmp"),
                PathBuf::from("/dev/null"),
                Arc::clone(&pool),
                capture(),
            )
            .unwrap();
        mgr.inject_stdin(&spawned.id, b"hello\n").unwrap();
        mgr.inject_stdin(&spawned.id, b"\r").unwrap();

        let writes = fake.bytes_writes();
        assert!(
            writes.iter().any(|(_, bytes)| bytes == b"hello\n"),
            "send_bytes should have captured hello\\n; got = {writes:?}",
        );
        let keys = fake.keys();
        assert!(
            keys.iter().any(|(_, k)| k == "Enter"),
            "bare \\r should route as send_key(Enter); got = {keys:?}",
        );

        mgr.kill(&spawned.id).unwrap();

        // After kill, forwarder thread exits and flips the row.
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let conn = pool.get().unwrap();
            let status: String = conn
                .query_row(
                    "SELECT status FROM sessions WHERE id = ?1",
                    params![spawned.id],
                    |r| r.get(0),
                )
                .unwrap();
            if status != "running" {
                break;
            }
            if Instant::now() > deadline {
                panic!("session never exited after kill");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn inject_stdin_on_unknown_session_errors_cleanly() {
        let mgr = SessionManager::new(None, inert_runtime());
        let err = mgr.inject_stdin("nope", b"x").unwrap_err();
        assert!(format!("{err}").contains("session not found"));
    }

    // `await_pty_output` was deleted in the Step 9 cutover. Tests
    // that previously observed echoed bytes from /bin/cat through
    // a portable-pty master now assert on FakeRuntime's captured
    // pastes / keys / bytes_writes directly — faster and free of
    // shell-timing flakes.

    #[test]
    fn codex_direct_chat_injects_persona_without_preamble() {
        // Direct chats are off-bus: the bundled `runner` CLI is not on
        // PATH (#51) and there's no crew/mission to coordinate over,
        // so the WORKER_COORDINATION_PREAMBLE would advertise verbs
        // that don't work. Direct chat sends ONLY the persona
        // (runner.system_prompt) via the runtime's paste() —
        // `inject_paste` from the manager.
        //
        // Assert (a) `runtime.paste` was called with the persona
        // token and (b) a distinctive substring of
        // WORKER_COORDINATION_PREAMBLE was NOT — regression guard
        // for the bug in #51.
        let pool = pool_with_schema();
        let now = Utc::now().to_rfc3339();
        let runner_id = ulid::Ulid::new().to_string();
        {
            let conn = pool.get().unwrap();
            conn.execute(
                "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, working_dir, system_prompt, env_json,
                     created_at, updated_at)
                 VALUES (?1, 'codex-tester', 'CT', 'codex', '/bin/cat',
                         NULL, NULL, NULL, NULL, ?2, ?2)",
                params![runner_id, now],
            )
            .unwrap();
        }
        let mut runner = runner("/bin/cat", &[]);
        runner.id = runner_id.clone();
        runner.handle = "codex-tester".into();
        runner.runtime = "codex".into();
        runner.system_prompt = Some("CODEX_PERSONA_TOKEN".into());

        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        let spawned = mgr
            .spawn_direct(
                &runner,
                Some("/tmp"),
                None,
                None,
                std::path::Path::new("/tmp"),
                Arc::clone(&pool),
                capture(),
            )
            .unwrap();

        // FIRST_PROMPT_DELAY = ZERO under cfg(test) so the inject
        // happens inline before spawn_direct returns.
        let pastes = fake.pastes();
        let merged: String = pastes
            .iter()
            .map(|(_, p)| String::from_utf8_lossy(p).to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            merged.contains("CODEX_PERSONA_TOKEN"),
            "codex direct chat must paste the persona; got pastes = {merged:?}",
        );
        assert!(
            !merged.contains("in a crew coordinated by the bundled"),
            "direct chat must NOT paste WORKER_COORDINATION_PREAMBLE: {merged:?}",
        );
        // Manager should also follow up with send_key("Enter") to
        // submit the paste.
        assert!(
            fake.keys().iter().any(|(_, k)| k == "Enter"),
            "expected Enter key after paste; got keys = {:?}",
            fake.keys()
        );

        mgr.kill(&spawned.id).unwrap();
    }

    #[test]
    fn claude_code_direct_chat_injects_persona_without_preamble() {
        // Same shape as the codex test, but with claude-code runtime
        // — claude-code's `--append-system-prompt` is SDK-only
        // (silently dropped by the interactive TUI), so stdin is the
        // only persona-delivery path.
        let pool = pool_with_schema();
        let now = Utc::now().to_rfc3339();
        let runner_id = ulid::Ulid::new().to_string();
        {
            let conn = pool.get().unwrap();
            conn.execute(
                "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, working_dir, system_prompt, env_json,
                     created_at, updated_at)
                 VALUES (?1, 'cc-tester', 'CC', 'claude-code', '/bin/sh',
                         ?3, NULL, NULL, NULL, ?2, ?2)",
                params![runner_id, now, r#"["-c","cat"]"#],
            )
            .unwrap();
        }
        let mut runner = runner("/bin/sh", &["-c", "cat"]);
        runner.id = runner_id.clone();
        runner.handle = "cc-tester".into();
        runner.runtime = "claude-code".into();
        runner.system_prompt = Some("CC_PERSONA_TOKEN".into());

        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        let spawned = mgr
            .spawn_direct(
                &runner,
                Some("/tmp"),
                None,
                None,
                std::path::Path::new("/tmp"),
                Arc::clone(&pool),
                capture(),
            )
            .unwrap();

        let merged: String = fake
            .pastes()
            .iter()
            .map(|(_, p)| String::from_utf8_lossy(p).to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            merged.contains("CC_PERSONA_TOKEN"),
            "claude-code direct chat must paste the persona; got = {merged:?}",
        );
        assert!(
            !merged.contains("in a crew coordinated by the bundled"),
            "direct chat must NOT paste WORKER_COORDINATION_PREAMBLE: {merged:?}",
        );

        mgr.kill(&spawned.id).unwrap();
    }

    #[test]
    fn mission_spawn_injects_preamble_for_non_lead_worker() {
        // Regression guard for #45 after the schedule_first_prompt
        // split. Mission spawn (non-lead worker) must STILL get the
        // bus-contract WORKER_COORDINATION_PREAMBLE typed into stdin
        // ahead of the user-authored brief, since workers are
        // expected to call `runner msg post`, `runner status idle`,
        // etc. Mirrors the direct-chat tests but spawns through the
        // mission-flavored `spawn` path with a non-lead slot.
        let pool = pool_with_schema();
        let mission = mission();
        let mut runner = runner("/bin/sh", &["-c", "cat"]);
        runner.runtime = "claude-code".into();
        runner.handle = "worker-tester".into();
        runner.system_prompt = Some("WORKER_PERSONA_TOKEN".into());

        let slot_id = insert_crew_runner(&pool, &mission.id, &runner.id);
        // Override the inserted slot row to non-lead so
        // schedule_mission_first_prompt actually fires (lead path is
        // suppressed because the launch prompt is dispatched separately
        // by the bus's mission_goal handler).
        {
            let conn = pool.get().unwrap();
            conn.execute("UPDATE slots SET lead = 0 WHERE id = ?1", params![slot_id])
                .unwrap();
            // Mirror runner row updates so spawn() reads the test's
            // runtime / handle / system_prompt / args.
            conn.execute(
                "UPDATE runners
                    SET runtime = ?2, handle = ?3, system_prompt = ?4,
                        command = ?5, args_json = ?6
                  WHERE id = ?1",
                params![
                    runner.id,
                    runner.runtime,
                    runner.handle,
                    runner.system_prompt,
                    runner.command,
                    r#"["-c","cat"]"#,
                ],
            )
            .unwrap();
        }
        let fresh_mission_id = {
            let conn = pool.get().unwrap();
            conn.query_row("SELECT id FROM missions LIMIT 1", [], |r| r.get(0))
                .unwrap()
        };
        let mission = Mission {
            id: fresh_mission_id,
            ..mission
        };
        let mut slot = slot_for(&runner);
        slot.id = slot_id;
        slot.lead = false;

        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        let spawned = mgr
            .spawn(
                &mission,
                &runner,
                &slot,
                std::path::Path::new("/tmp"),
                PathBuf::from("/dev/null"),
                Arc::clone(&pool),
                capture(),
            )
            .unwrap();

        // FIRST_PROMPT_DELAY = ZERO under cfg(test) so the inject
        // happens inline. The mission preamble + brief are pasted
        // as a single bracketed-paste through `inject_paste`.
        let merged: String = fake
            .pastes()
            .iter()
            .map(|(_, p)| String::from_utf8_lossy(p).to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            merged.contains("in a crew coordinated by the bundled"),
            "non-lead worker must paste WORKER_COORDINATION_PREAMBLE: {merged:?}",
        );
        assert!(
            merged.contains("WORKER_PERSONA_TOKEN"),
            "non-lead worker must paste the user-authored brief: {merged:?}",
        );

        mgr.kill(&spawned.id).unwrap();
    }

    #[test]
    fn codex_resume_skips_first_prompt_injection() {
        // On a codex resume the agent already has its system context
        // — replaying the brief would either be a no-op (codex
        // resume doesn't replay first turns) or, worse, push a fresh
        // user turn against the existing conversation. Verify the
        // resume path leaves stdin untouched: spawn /bin/cat with
        // codex runtime + a populated `agent_session_key` (so
        // `resume_plan` chooses the resuming branch), wait briefly,
        // and assert no echo arrived. Pairs with
        // `codex_fresh_spawn_injects_brief_via_stdin` — same setup,
        // opposite expectation, locking in the resume guard.
        let pool = pool_with_schema();
        let now = Utc::now().to_rfc3339();
        let runner_id = ulid::Ulid::new().to_string();
        let session_id = ulid::Ulid::new().to_string();
        let prior_key = uuid::Uuid::new_v4().to_string();
        {
            let conn = pool.get().unwrap();
            conn.execute(
                "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, working_dir, system_prompt, env_json,
                     created_at, updated_at)
                 VALUES (?1, 'codex-resumer', 'CR', 'codex', '/bin/cat',
                         NULL, NULL, NULL, NULL, ?2, ?2)",
                params![runner_id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions
                    (id, mission_id, runner_id, cwd, status, started_at,
                     agent_session_key)
                 VALUES (?1, NULL, ?2, '/tmp', 'stopped', ?3, ?4)",
                params![session_id, runner_id, now, prior_key],
            )
            .unwrap();
        }
        // Update the in-memory runner row to mirror the DB so resume()
        // reads what we just inserted.
        {
            let conn = pool.get().unwrap();
            conn.execute(
                "UPDATE runners SET system_prompt = ?2 WHERE id = ?1",
                params![runner_id, "CODEX_BRIEF_TOKEN_RESUME"],
            )
            .unwrap();
        }

        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        let resumed = mgr
            .resume(
                &session_id,
                None,
                None,
                std::path::Path::new("/tmp"),
                Arc::clone(&pool),
                capture(),
            )
            .unwrap();

        // FIRST_PROMPT_DELAY = ZERO under cfg(test); a would-be
        // injection would already be visible in fake.pastes() by
        // the time resume() returns. The contract: codex resume
        // MUST NOT paste anything containing the brief.
        let pasted: String = fake
            .pastes()
            .iter()
            .map(|(_, p)| String::from_utf8_lossy(p).to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !pasted.contains("CODEX_BRIEF_TOKEN_RESUME"),
            "codex resume must NOT paste the brief; got = {pasted:?}"
        );

        mgr.kill(&resumed.id).unwrap();
    }

    #[test]
    fn spawn_failure_after_spawn_command_reaps_the_child() {
        // Force the `sessions` INSERT to fail by dropping the table after the
        // pool is built. Without the post-spawn cleanup, the child would keep
        // running after `spawn` returns Err because nothing knows about it.
        let pool = pool_with_schema();
        let mission = mission();
        let mut runner = runner("/bin/cat", &[]);
        insert_crew_runner(&pool, &mission.id, &runner.id);
        runner.id = {
            let conn = pool.get().unwrap();
            conn.query_row("SELECT id FROM runners LIMIT 1", [], |r| r.get(0))
                .unwrap()
        };
        let fresh_mission_id: String = {
            let conn = pool.get().unwrap();
            conn.query_row("SELECT id FROM missions LIMIT 1", [], |r| r.get(0))
                .unwrap()
        };
        let mission = Mission {
            id: fresh_mission_id,
            ..mission
        };

        // Break the schema so the next INSERT fails.
        pool.get()
            .unwrap()
            .execute("DROP TABLE sessions", [])
            .unwrap();

        let mgr = SessionManager::new(None, inert_runtime());
        let slot = slot_for(&runner);
        let err = mgr
            .spawn(
                &mission,
                &runner,
                &slot,
                std::path::Path::new("/tmp"),
                PathBuf::from("/dev/null"),
                Arc::clone(&pool),
                capture(),
            )
            .unwrap_err();
        // The error must surface the DB failure, not a spawn failure.
        assert!(
            format!("{err}").contains("sessions") || format!("{err}").contains("no such table"),
            "unexpected error: {err}"
        );
        // No live session left behind.
        assert!(mgr.sessions.lock().unwrap().is_empty());
    }

    #[test]
    fn kill_blocks_until_session_row_is_terminal() {
        // mission_stop relies on this contract: kill must return only
        // after the forwarder thread has updated the DB row. With
        // FakeRuntime, `runtime.stop` drops the mpsc Sender so the
        // forwarder sees Disconnected and reconciles immediately;
        // `kill` joins on it before returning.
        let pool = pool_with_schema();
        let mission = mission();
        let mut runner = runner("/bin/cat", &[]);
        insert_crew_runner(&pool, &mission.id, &runner.id);
        runner.id = {
            let conn = pool.get().unwrap();
            conn.query_row("SELECT id FROM runners LIMIT 1", [], |r| r.get(0))
                .unwrap()
        };
        let fresh_mission_id: String = {
            let conn = pool.get().unwrap();
            conn.query_row("SELECT id FROM missions LIMIT 1", [], |r| r.get(0))
                .unwrap()
        };
        let mission = Mission {
            id: fresh_mission_id,
            ..mission
        };

        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        let slot = slot_for(&runner);
        let spawned = mgr
            .spawn(
                &mission,
                &runner,
                &slot,
                std::path::Path::new("/tmp"),
                PathBuf::from("/dev/null"),
                Arc::clone(&pool),
                capture(),
            )
            .unwrap();

        mgr.kill(&spawned.id).unwrap();

        let conn = pool.get().unwrap();
        let status: String = conn
            .query_row(
                "SELECT status FROM sessions WHERE id = ?1",
                params![spawned.id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            status != "running",
            "kill returned while session still running: {status}"
        );
        // killed-set caused the forwarder to classify as `stopped`
        // even though FakeRuntime returns exit_code=0.
        assert_eq!(status, "stopped");
        // The runtime should have observed at least one stop call
        // — two is normal (kill calls stop directly; the
        // forwarder also calls stop on its way out as
        // belt-and-suspenders cleanup once the channel closes).
        assert!(!fake.stops.lock().unwrap().is_empty());
    }

    #[test]
    fn spawn_direct_writes_session_with_null_mission_id_and_emits_activity() {
        // C8.5: a "Chat now" session lives outside any mission. Verify the
        // sessions row has mission_id IS NULL, the session lands in the
        // live map, and the runner_activity emission fires on spawn.
        let pool = pool_with_schema();
        // We don't go through `insert_crew_runner` here because direct
        // chat doesn't need a crew or mission — only a runner row.
        let now = Utc::now().to_rfc3339();
        let runner_id = ulid::Ulid::new().to_string();
        {
            let conn = pool.get().unwrap();
            conn.execute(
                "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, working_dir, system_prompt, env_json,
                     created_at, updated_at)
                 VALUES (?1, 'directrunner', 'D', 'shell', '/bin/sh',
                         NULL, NULL, NULL, NULL, ?2, ?2)",
                params![runner_id, now],
            )
            .unwrap();
        }

        let mut runner = runner("/bin/sh", &["-c", "echo direct"]);
        runner.id = runner_id.clone();
        runner.handle = "directrunner".into();

        let cap = capture();
        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        let spawned = mgr
            .spawn_direct(
                &runner,
                Some("/tmp"),
                None,
                None,
                std::path::Path::new("/tmp"),
                Arc::clone(&pool),
                cap.clone(),
            )
            .unwrap();
        assert_eq!(spawned.mission_id, None);
        assert_eq!(spawned.runner_id, runner_id);

        // Direct chat must NOT have a mission-side shim or
        // bundled-bin in its SpawnSpec — the off-bus invariant.
        let spec = fake.last_spawn_spec().expect("spawn was called");
        assert!(!spec.mission, "spawn_direct must spawn with mission=false");
        assert!(spec.shim_dir.is_none(), "direct chat must not have a shim");
        assert!(
            spec.bundled_bin_dir.is_none(),
            "direct chat must not have the bundled bin on PATH",
        );

        // Simulate clean exit so the activity emission cycle
        // completes (spawn-time emit then reap-time emit).
        fake.close_spawn(0);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let conn = pool.get().unwrap();
            let row: (String, Option<String>) = conn
                .query_row(
                    "SELECT status, mission_id FROM sessions WHERE id = ?1",
                    params![&spawned.id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(
                row.1, None,
                "direct session must persist with NULL mission_id"
            );
            if row.0 != "running" {
                break;
            }
            if Instant::now() > deadline {
                panic!("direct session never exited");
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        // Last activity emission after reap should show zero
        // active sessions for this runner.
        let activity = cap.activity.lock().unwrap();
        assert!(!activity.is_empty(), "runner_activity must fire");
        let last = activity.last().unwrap();
        assert_eq!(last.runner_id, runner_id);
        assert_eq!(
            last.active_sessions, 0,
            "after reap, active_sessions for this runner must be 0"
        );
    }

    #[test]
    fn output_snapshot_replays_live_session_and_clears_after_forget() {
        let pool = pool_with_schema();
        let now = Utc::now().to_rfc3339();
        let runner_id = ulid::Ulid::new().to_string();
        {
            let conn = pool.get().unwrap();
            conn.execute(
                "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, working_dir, system_prompt, env_json,
                     created_at, updated_at)
                 VALUES (?1, 'buffered', 'Buffered', 'shell', '/bin/cat',
                         NULL, NULL, NULL, NULL, ?2, ?2)",
                params![runner_id, now],
            )
            .unwrap();
        }

        let mut runner = runner("/bin/cat", &[]);
        runner.id = runner_id;
        runner.handle = "buffered".into();

        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        let spawned = mgr
            .spawn_direct(
                &runner,
                Some("/tmp"),
                None,
                None,
                std::path::Path::new("/tmp"),
                Arc::clone(&pool),
                capture(),
            )
            .unwrap();

        // Push fake output through the runtime → forwarder
        // chain. The forwarder records it into the manager's
        // output buffer; output_snapshot reads it back.
        fake.push_output(0, b"hello snapshot");
        let deadline = Instant::now() + Duration::from_secs(2);
        let snapshot = loop {
            let snapshot = mgr.output_snapshot(&spawned.id);
            if !snapshot.is_empty() {
                break snapshot;
            }
            if Instant::now() > deadline {
                panic!("session output snapshot never captured live output");
            }
            std::thread::sleep(Duration::from_millis(20));
        };

        assert_eq!(snapshot[0].seq, 1);
        assert!(
            snapshot.iter().all(|ev| ev.session_id == spawned.id),
            "snapshot must only include chunks for the requested session"
        );

        mgr.kill(&spawned.id).unwrap();
        // After kill the buffer is intentionally preserved so a
        // remount can replay the dead session's scrollback. Explicit
        // cleanup is via `purge_session_buffers`.
        assert!(
            !mgr.output_snapshot(&spawned.id).is_empty(),
            "kill must keep the output buffer for snapshot replay"
        );
        mgr.purge_session_buffers(&spawned.id);
        assert!(
            mgr.output_snapshot(&spawned.id).is_empty(),
            "purge_session_buffers must drop the buffer"
        );
    }

    #[test]
    fn resume_reuses_row_and_preserves_agent_session_key() {
        // Multi-chat-per-runner contract: a direct chat IS a
        // sessions row. spawn_direct creates the row and the
        // claude-code adapter persists a UUID under
        // `agent_session_key`. After exit, resume respawns the
        // *same* row (same id, same agent_session_key column
        // populated) and flips status back to running. See
        // docs/impls/0003-direct-chats.md.
        let pool = pool_with_schema();
        let now = Utc::now().to_rfc3339();
        let runner_id = ulid::Ulid::new().to_string();
        {
            let conn = pool.get().unwrap();
            conn.execute(
                "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, working_dir, system_prompt, env_json,
                     created_at, updated_at)
                 VALUES (?1, 'resumer', 'R', 'claude-code', '/bin/sh',
                         NULL, NULL, NULL, NULL, ?2, ?2)",
                params![runner_id, now],
            )
            .unwrap();
        }
        let mut runner = runner("/bin/sh", &["-c", "echo first"]);
        runner.id = runner_id.clone();
        runner.handle = "resumer".into();
        runner.runtime = "claude-code".into();

        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        let spawned = mgr
            .spawn_direct(
                &runner,
                Some("/tmp"),
                None,
                None,
                std::path::Path::new("/tmp"),
                Arc::clone(&pool),
                capture(),
            )
            .unwrap();
        let session_id = spawned.id.clone();

        // Force the spawn to "exit" so the forwarder marks the
        // row stopped; resume() refuses a row that's still
        // running.
        fake.close_spawn(0);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let conn = pool.get().unwrap();
            let status: String = conn
                .query_row(
                    "SELECT status FROM sessions WHERE id = ?1",
                    params![&session_id],
                    |r| r.get(0),
                )
                .unwrap();
            if status != "running" {
                break;
            }
            if Instant::now() > deadline {
                panic!("first spawn never exited");
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        // The claude-code adapter persisted a UUID — capture it.
        let key_before: Option<String> = {
            let conn = pool.get().unwrap();
            conn.query_row(
                "SELECT agent_session_key FROM sessions WHERE id = ?1",
                params![&session_id],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert!(
            key_before.is_some(),
            "claude-code spawn must persist an agent_session_key for later resume",
        );

        // Resume: same id, same row.
        let resumed = mgr
            .resume(
                &session_id,
                None,
                None,
                std::path::Path::new("/tmp"),
                Arc::clone(&pool),
                capture(),
            )
            .unwrap();
        assert_eq!(resumed.id, session_id, "resume must reuse the row id");

        // After resume the status is running again with the
        // agent_session_key still populated. We don't pin the
        // UUID value — the resume_plan logic + missing-
        // conversation-file fallback can rotate it; the
        // manager-level invariant is "row id is preserved and
        // the key column stays populated."
        let key_after: Option<String> = {
            let conn = pool.get().unwrap();
            conn.query_row(
                "SELECT agent_session_key FROM sessions WHERE id = ?1",
                params![&session_id],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert!(
            key_after.is_some(),
            "resume must keep agent_session_key populated; got NULL",
        );

        // Only one row survives: resume must not have INSERTed a
        // duplicate.
        let count: i64 = pool
            .get()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE runner_id = ?1",
                params![runner_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "resume must update in place, not insert");

        mgr.kill(&session_id).unwrap();
    }

    #[test]
    fn resume_refuses_running_and_archived_rows() {
        // Mission rows are no longer rejected — see
        // resume_mission_session_stamps_slot_handle_env. This test
        // covers the gates that remain.
        let pool = pool_with_schema();
        let now = Utc::now().to_rfc3339();
        let runner_id = ulid::Ulid::new().to_string();
        {
            let conn = pool.get().unwrap();
            conn.execute(
                "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     created_at, updated_at)
                 VALUES (?1, 'r', 'R', 'shell', '/bin/sh', ?2, ?2)",
                params![runner_id, now],
            )
            .unwrap();
            // Already-running direct session.
            conn.execute(
                "INSERT INTO sessions
                    (id, mission_id, runner_id, status, started_at)
                 VALUES ('running-sid', NULL, ?1, 'running', ?2)",
                params![runner_id, now],
            )
            .unwrap();
            // Archived direct session.
            conn.execute(
                "INSERT INTO sessions
                    (id, mission_id, runner_id, status, started_at, archived_at)
                 VALUES ('archived-sid', NULL, ?1, 'stopped', ?2, ?2)",
                params![runner_id, now],
            )
            .unwrap();
        }
        let mgr = SessionManager::new(None, inert_runtime());
        for (sid, needle) in [
            ("running-sid", "already running"),
            ("archived-sid", "archived"),
        ] {
            let err = mgr
                .resume(
                    sid,
                    None,
                    None,
                    std::path::Path::new("/tmp"),
                    Arc::clone(&pool),
                    capture(),
                )
                .unwrap_err();
            let msg = format!("{err}");
            assert!(
                msg.contains(needle),
                "resume({sid}) should reject with `{needle}`, got `{msg}`"
            );
        }
    }

    #[test]
    fn resume_mission_session_stamps_slot_handle_env() {
        // Mission resume must look up the slot for the session and
        // use slot.slot_handle as RUNNER_HANDLE, not runner.handle.
        // After the Step 9 cutover the manager hands env to the
        // runtime via SpawnSpec.env; FakeRuntime captures the spec
        // and we assert RUNNER_HANDLE == slot_handle directly.
        let pool = pool_with_schema();
        let now = Utc::now().to_rfc3339();
        let runner_id = ulid::Ulid::new().to_string();
        let mission_id = ulid::Ulid::new().to_string();
        let slot_id = ulid::Ulid::new().to_string();
        {
            let conn = pool.get().unwrap();
            conn.execute(
                "INSERT INTO crews (id, name, created_at, updated_at)
                 VALUES ('c-mr', 'c', ?1, ?1)",
                params![now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, created_at, updated_at)
                 VALUES (?1, 'template-handle', 'R', 'shell', '/bin/sh',
                         '[\"-c\", \"echo HANDLE=$RUNNER_HANDLE && exit\"]',
                         ?2, ?2)",
                params![runner_id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO slots
                    (id, crew_id, runner_id, slot_handle, position, lead, added_at)
                 VALUES (?1, 'c-mr', ?2, 'architect-slot', 0, 1, ?3)",
                params![slot_id, runner_id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO missions
                    (id, crew_id, title, status, started_at)
                 VALUES (?1, 'c-mr', 't', 'running', ?2)",
                params![mission_id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions
                    (id, mission_id, runner_id, slot_id, status, started_at)
                 VALUES ('mr-sid', ?1, ?2, ?3, 'stopped', ?4)",
                params![mission_id, runner_id, slot_id, now],
            )
            .unwrap();
        }

        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        let spawned = mgr
            .resume(
                "mr-sid",
                None,
                None,
                std::path::Path::new("/tmp"),
                Arc::clone(&pool),
                capture(),
            )
            .unwrap();
        // Returned identity is the slot's, not the template's.
        assert_eq!(spawned.handle, "architect-slot");
        assert_eq!(spawned.mission_id.as_deref(), Some(mission_id.as_str()));

        // The SpawnSpec the manager built for the runtime must
        // carry RUNNER_HANDLE = slot_handle (not the template
        // handle), plus the other mission-bus env vars.
        let spec = fake
            .last_spawn_spec()
            .expect("resume should have called spawn");
        assert_eq!(
            spec.env.get("RUNNER_HANDLE").map(String::as_str),
            Some("architect-slot"),
            "RUNNER_HANDLE must be the slot_handle, got env = {:?}",
            spec.env,
        );
        assert_eq!(
            spec.env.get("RUNNER_CREW_ID").map(String::as_str),
            Some("c-mr"),
        );
        assert_eq!(
            spec.env.get("RUNNER_MISSION_ID").map(String::as_str),
            Some(mission_id.as_str()),
        );
        assert!(
            spec.shim_dir.is_some(),
            "mission resume must install the per-slot shim",
        );
        assert!(
            spec.bundled_bin_dir.is_some(),
            "mission resume must put the bundled CLI on PATH",
        );

        mgr.kill("mr-sid").unwrap();
    }

    /// Helper: insert a runner row + a `running` direct-chat
    /// session row with the runtime_* columns populated as if a
    /// prior Runner process had spawned the session through tmux.
    fn insert_running_row_with_runtime_meta(pool: &Arc<DbPool>) -> (String, String) {
        let now = Utc::now().to_rfc3339();
        let runner_id = ulid::Ulid::new().to_string();
        let session_id = ulid::Ulid::new().to_string();
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO runners
                (id, handle, display_name, runtime, command,
                 args_json, working_dir, system_prompt, env_json,
                 created_at, updated_at)
             VALUES (?1, 'reattach', 'R', 'shell', '/bin/sh',
                     NULL, NULL, NULL, NULL, ?2, ?2)",
            params![runner_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
                (id, mission_id, runner_id, cwd, status, started_at,
                 runtime, runtime_socket, runtime_session,
                 runtime_window, runtime_pane)
             VALUES (?1, NULL, ?2, '/tmp', 'running', ?3,
                     'tmux', 'runner', ?4, 'main', ?5)",
            params![
                session_id,
                runner_id,
                now,
                format!("runner-{session_id}"),
                format!("%{session_id}"),
            ],
        )
        .unwrap();
        (session_id, runner_id)
    }

    #[test]
    fn reattach_running_sessions_recovers_live_pane() {
        // Simulate "Runner restarted while a tmux pane survived":
        // a sessions row is `running` with runtime_* populated.
        // FakeRuntime's status() returns alive=true by default,
        // so reattach should rebuild the SessionHandle (the row
        // stays running) and the manager's sessions map gains
        // an entry.
        let pool = pool_with_schema();
        let (session_id, _runner_id) = insert_running_row_with_runtime_meta(&pool);

        let fake = fake_runtime();
        // Override status: alive (the default exit_code=0 still
        // applies but alive=true takes precedence).
        {
            let mut s = fake.status_response.lock().unwrap();
            s.alive = true;
            s.exit_code = None;
        }
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        mgr.reattach_running_sessions(Arc::clone(&pool), capture());

        // Row should still be running.
        let status: String = pool
            .get()
            .unwrap()
            .query_row(
                "SELECT status FROM sessions WHERE id = ?1",
                params![session_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "running");
        // Manager should have the session in its live map.
        assert!(mgr.sessions.lock().unwrap().contains_key(&session_id));

        // Cleanup so the forwarder thread doesn't leak.
        mgr.kill(&session_id).unwrap();
    }

    #[test]
    fn reattach_running_sessions_marks_dead_pane_with_exit_code() {
        // Pane is gone-but-flagged-dead: tmux still has the
        // remain-on-exit row and reports pane_dead=1 with a
        // non-zero exit code. Reattach should mark the row
        // crashed (non-zero) and tear down the dead pane.
        let pool = pool_with_schema();
        let (session_id, _runner_id) = insert_running_row_with_runtime_meta(&pool);

        let fake = fake_runtime();
        {
            let mut s = fake.status_response.lock().unwrap();
            s.alive = false;
            s.exit_code = Some(42);
        }
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        mgr.reattach_running_sessions(Arc::clone(&pool), capture());

        let status: String = pool
            .get()
            .unwrap()
            .query_row(
                "SELECT status FROM sessions WHERE id = ?1",
                params![session_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "crashed");
        // The row must NOT be in the live map — there's no live
        // pane to attach to.
        assert!(!mgr.sessions.lock().unwrap().contains_key(&session_id));
    }

    #[test]
    fn reattach_running_sessions_kills_mission_panes_to_avoid_routing_drift() {
        // Mission sessions don't reattach at startup — an agent
        // appending bus events while Runner is closed and then
        // reattaching without the mission's bus + router mounted
        // would silently miss routing of those events. Kill the
        // pane (so it doesn't keep running), mark the row stopped
        // (so the user can resume from the workspace, which
        // mounts the bus). Direct chats are unaffected.
        let pool = pool_with_schema();
        let now = Utc::now().to_rfc3339();
        let runner_id = ulid::Ulid::new().to_string();
        let session_id = ulid::Ulid::new().to_string();
        let crew_id = "c-mission-reattach".to_string();
        let mission_id = ulid::Ulid::new().to_string();
        {
            let conn = pool.get().unwrap();
            conn.execute(
                "INSERT INTO crews (id, name, created_at, updated_at)
                 VALUES (?1, 'c', ?2, ?2)",
                params![crew_id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO runners
                    (id, handle, display_name, runtime, command,
                     args_json, working_dir, system_prompt, env_json,
                     created_at, updated_at)
                 VALUES (?1, 'mr', 'M', 'shell', '/bin/sh',
                         NULL, NULL, NULL, NULL, ?2, ?2)",
                params![runner_id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO missions (id, crew_id, title, status, started_at)
                 VALUES (?1, ?2, 't', 'running', ?3)",
                params![mission_id, crew_id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions
                    (id, mission_id, runner_id, status, started_at,
                     runtime, runtime_socket, runtime_session,
                     runtime_window, runtime_pane)
                 VALUES (?1, ?2, ?3, 'running', ?4,
                         'tmux', 'runner', ?5, 'main', ?6)",
                params![
                    session_id,
                    mission_id,
                    runner_id,
                    now,
                    format!("runner-{session_id}"),
                    format!("%{session_id}"),
                ],
            )
            .unwrap();
        }

        let fake = fake_runtime();
        // Pane is alive — without the mission carve-out, the
        // current code would happily reattach.
        {
            let mut s = fake.status_response.lock().unwrap();
            s.alive = true;
            s.exit_code = None;
        }
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        mgr.reattach_running_sessions(Arc::clone(&pool), capture());

        // Row marked stopped (the user resumes from the workspace,
        // which is where the bus + router mount).
        let status: String = pool
            .get()
            .unwrap()
            .query_row(
                "SELECT status FROM sessions WHERE id = ?1",
                params![session_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "stopped");
        // Manager should NOT have the session in its live map.
        assert!(!mgr.sessions.lock().unwrap().contains_key(&session_id));
        // The runtime should have observed exactly one stop call
        // — kill the pane so it doesn't keep producing events.
        assert_eq!(fake.stops.lock().unwrap().len(), 1);
    }

    #[test]
    fn reattach_running_sessions_marks_terminal_unavailable_stopped() {
        // Pane is gone entirely (tmux returns Ok(None) — no such
        // session). Mark the row stopped without inventing exit
        // info.
        let pool = pool_with_schema();
        let (session_id, _runner_id) = insert_running_row_with_runtime_meta(&pool);

        // FakeRuntime's status() always returns Ok(Some(...)) by
        // default, so we can't easily express terminal-unavailable
        // through the canned response. Instead, blank out the
        // runtime_* columns to simulate a row that has no usable
        // metadata — the reattach code path goes through
        // `runtime_session()` returning None and immediately marks
        // stopped.
        pool.get()
            .unwrap()
            .execute(
                "UPDATE sessions
                    SET runtime = NULL, runtime_socket = NULL, runtime_session = NULL,
                        runtime_window = NULL, runtime_pane = NULL
                  WHERE id = ?1",
                params![session_id],
            )
            .unwrap();

        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        mgr.reattach_running_sessions(Arc::clone(&pool), capture());

        let status: String = pool
            .get()
            .unwrap()
            .query_row(
                "SELECT status FROM sessions WHERE id = ?1",
                params![session_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "stopped");
    }
}
