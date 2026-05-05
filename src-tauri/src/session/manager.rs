// Per-runner PTY session runtime.
//
// One `Session` = one `portable_pty` child running the runner's CLI agent. The
// SessionManager holds the map of live sessions so Tauri commands can look
// them up by id (for stdin injection, pause/resume, kill). Each session owns:
//
//   - A `MasterPty` handle (Tauri process side). The slave end is closed
//     immediately after spawn — we never read from it.
//   - A reader thread that drains the PTY and emits `session/output` Tauri
//     events. When the reader hits EOF (child exited, signaled, or we killed
//     it), it reaps the child, emits `session/exit`, and updates the DB row.
//   - A writer behind a Mutex for `inject_stdin`.
//
// Drop behavior: killing the app process drops the SessionManager, which
// drops every `SessionHandle`, which drops each `Child`. `portable-pty`'s
// Child wrappers on Unix do not SIGKILL on drop — we take care of this in
// `SessionManager::kill_all` at app shutdown (future work; for MVP the
// child inherits our process group and dies when we exit).

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::Utc;
use portable_pty::{CommandBuilder, MasterPty, PtySize};
use rusqlite::params;
use serde::Serialize;

use crate::db::DbPool;
use crate::error::{Error, Result};
use crate::model::{Mission, Runner};
use crate::router;

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
    /// Optionally holds the master PTY. `kill` takes it to drop-close the
    /// terminal (signals the child's SIGHUP) before signaling/joining.
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    /// OS process id of the spawned child. Used by `kill` to escalate
    /// SIGTERM → SIGKILL if the PTY hangup alone doesn't reap the child.
    pid: Option<u32>,
    /// Handle for the reader thread that drains the PTY + reaps the child.
    /// `kill` joins on it so the caller is guaranteed the `sessions` row is
    /// in a terminal status by the time we return.
    reader: Option<thread::JoinHandle<()>>,
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
    pub fn new(shell_path: Option<String>) -> Arc<Self> {
        Arc::new(Self {
            sessions: Mutex::new(HashMap::new()),
            killed: Mutex::new(HashSet::new()),
            output_buffers: Mutex::new(HashMap::new()),
            output_seq: Mutex::new(HashMap::new()),
            resuming_claims: Mutex::new(HashSet::new()),
            shell_path,
        })
    }

    /// Compose a child PTY's PATH from the bundled-bin dir, an optional
    /// per-slot shim dir, the captured login-shell PATH, and the
    /// inherited (launchd-default) PATH. Order: shim ► bin ► shell ► parent.
    /// `shim` and `bin` come first so our `runner` resolves to the env-baked
    /// shim before any system-installed `runner` binary; `shell` comes
    /// before the inherited launchd PATH because launchd's PATH on macOS
    /// is the stripped default we're trying to extend.
    fn compose_path(&self, shim_dir: Option<&Path>, bin_dir: Option<&Path>) -> std::ffi::OsString {
        let sep: &str = if cfg!(windows) { ";" } else { ":" };
        let parent_path = std::env::var_os("PATH").unwrap_or_default();
        let mut parts: Vec<std::ffi::OsString> = Vec::with_capacity(4);
        if let Some(shim) = shim_dir {
            parts.push(shim.as_os_str().into());
        }
        // `bin_dir = None` is the direct-chat path: the bundled `runner`
        // CLI is intentionally excluded from PATH so the agent can't
        // call bus verbs against an empty event log. Mission spawns
        // always pass `Some(bin_dir)`.
        if let Some(bin) = bin_dir {
            parts.push(bin.as_os_str().into());
        }
        if let Some(sp) = self.shell_path.as_deref() {
            if !sp.is_empty() {
                parts.push(std::ffi::OsString::from(sp));
            }
        }
        if !parent_path.is_empty() {
            parts.push(parent_path);
        }
        let mut new_path = std::ffi::OsString::new();
        for (i, p) in parts.iter().enumerate() {
            if i > 0 {
                new_path.push(sep);
            }
            new_path.push(p);
        }
        new_path
    }

    /// Spawn one PTY child for `runner` as part of `mission`. Persists a
    /// `sessions` row, starts the reader thread, and returns a summary for
    /// the frontend.
    ///
    /// `app_data_dir` is the root of `$APPDATA/runner/` so we can prepend
    /// `<app_data_dir>/bin` onto the child's PATH — arch §5.3 Layer 2 and
    /// v0-mvp.md C9 both require the bundled `runner` CLI to win over any
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
        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| Error::msg(format!("openpty: {e}")))?;

        // Agent-native session resume: this is a *fresh* session row, so
        // there's no prior key to inherit. The runtime adapter still
        // self-assigns a UUID for claude-code (`--session-id <uuid>`) so
        // a future `SessionManager::resume` can hand it back. See
        // docs/impls/direct-chats.md for why mission spawn no longer
        // chains across mission_stop/start.
        let plan = crate::router::runtime::resume_plan(&runner.runtime, None);

        let mut cmd = CommandBuilder::new(&runner.command);
        // codex resume is a subcommand, not a flag — it must precede any
        // user-supplied args. Other runtimes append their args.
        if plan.prepend {
            for extra in &plan.args {
                cmd.arg(extra);
            }
        }
        cmd.args(&runner.args);
        if !plan.prepend {
            for extra in &plan.args {
                cmd.arg(extra);
            }
        }
        // Append the runtime-specific trailing args: model/effort flags
        // followed by the `system_prompt` argv. Without this the
        // user-authored brief on the runner row is dropped on the floor
        // (arch §4.2 / §4.3). Codex requires flags before the positional
        // `[PROMPT]`; the helper enforces that ordering and the codex
        // carve-out that drops the prompt argv on resume so it isn't
        // replayed as a fresh user turn against an existing conversation.
        for extra in crate::router::runtime::trailing_runtime_args(
            &runner.runtime,
            plan.resuming,
            runner.model.as_deref(),
            runner.effort.as_deref(),
            runner.system_prompt.as_deref(),
        ) {
            cmd.arg(extra);
        }

        // Working directory: runner override if set, else mission cwd, else
        // inherit parent's. `CommandBuilder::cwd` requires a concrete path.
        // Capture the resolved cwd so we can persist it on the session row
        // — `resume` reads it back to spawn the same dir on respawn, which
        // matters for claude-code (its conversation files are keyed under
        // `~/.claude/projects/<encoded-cwd>/<uuid>.jsonl`; resuming with a
        // different cwd makes `--resume` fail with "No conversation found").
        let resolved_cwd: Option<String> =
            runner.working_dir.clone().or_else(|| mission.cwd.clone());
        if let Some(wd) = resolved_cwd.as_deref() {
            cmd.cwd(wd);
        }

        // Env — start from the runner's map (so the user can override /
        // clear things they need), then layer the system-assigned vars on
        // top so they can't be accidentally shadowed.
        for (k, v) in &runner.env {
            cmd.env(k, v);
        }
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

        // Prepend (shim, fallback bundled bin) to PATH so `runner` on the
        // child's PATH resolves first to our env-baked shim, then to
        // the raw CLI for verbs (`runner help`) that don't need
        // env. The captured login-shell PATH (Homebrew, mise, etc.)
        // is merged in next so GUI-launched apps can find third-party
        // agent CLIs that aren't on launchd's default PATH; the
        // inherited PATH is the tail.
        let bin_dir = app_data_dir.join("bin");
        let new_path = self.compose_path(shim_dir.as_deref(), Some(&bin_dir));
        cmd.env("PATH", new_path);

        cmd.env("RUNNER_CREW_ID", &mission.crew_id);
        cmd.env("RUNNER_MISSION_ID", &mission.id);
        // RUNNER_HANDLE is the slot's in-mission identity (slot_handle),
        // not the runner template's handle. The bundled `runner` CLI
        // stamps this into event envelopes so two slots referencing the
        // same template appear as distinct senders.
        cmd.env("RUNNER_HANDLE", &slot.slot_handle);
        cmd.env(
            "RUNNER_EVENT_LOG",
            events_log_path.to_string_lossy().to_string(),
        );
        if let Some(wd) = mission.cwd.as_deref() {
            cmd.env("MISSION_CWD", wd);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| Error::msg(format!("spawn {}: {e}", runner.command)))?;
        // Closing the slave on our side means child is the only holder and
        // our reader sees EOF the moment the child dies.
        drop(pair.slave);

        let pid = child.process_id();

        // Everything between `spawn_command` and the live-map insert is
        // fallible (`try_clone_reader`, `take_writer`, `sessions` INSERT).
        // If any of it errors we'd otherwise leak the running child — the
        // session isn't in the map yet, so `mission_start`'s rollback can't
        // see it and nothing else ever reaps it. Group the fallible work in
        // an IIFE so a single error handler can kill + wait the child on
        // every post-spawn failure path.
        let session_id = ulid::Ulid::new().to_string();
        let started_at = Utc::now().to_rfc3339();
        let setup_res: Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> = (|| {
            let reader = pair
                .master
                .try_clone_reader()
                .map_err(|e| Error::msg(format!("clone reader: {e}")))?;
            let writer = pair
                .master
                .take_writer()
                .map_err(|e| Error::msg(format!("take writer: {e}")))?;
            let conn = pool.get()?;
            conn.execute(
                "INSERT INTO sessions
                    (id, mission_id, runner_id, slot_id, cwd, status, pid, started_at,
                     agent_session_key)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6, ?7, ?8)",
                params![
                    session_id,
                    mission.id,
                    runner.id,
                    slot.id,
                    resolved_cwd,
                    pid,
                    started_at,
                    plan.assigned_key
                ],
            )?;
            Ok((reader, writer))
        })();
        let (reader, writer) = match setup_res {
            Ok(rw) => rw,
            Err(e) => {
                // Reap the orphan. `kill` signals SIGTERM/Windows equivalent;
                // `wait` blocks until the child is gone so the caller isn't
                // racing against a live PID when it retries.
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };

        // Insert into the live map BEFORE starting the reader thread.
        // A short-lived child (e.g. `sh -c "echo hi"`) can exit within
        // microseconds — if we spawned the thread first, its `forget()`
        // call could run before the insert and leave a stale live handle
        // for an already-dead session. Handle parts that the reader thread
        // needs ownership of (child, reader pipe) stay out of the map;
        // parts the Tauri commands need (master, writer, pid) go in.
        self.sessions.lock().unwrap().insert(
            session_id.clone(),
            SessionHandle {
                id: session_id.clone(),
                mission_id: Some(mission.id.clone()),
                runner_id: runner.id.clone(),
                master: Some(pair.master),
                writer: Mutex::new(writer),
                pid,
                reader: None, // populated immediately below
            },
        );

        // Spawn the reader thread. On EOF it reaps the child, updates the
        // DB row, removes the session from the in-memory map, and emits
        // the `exit` event. `kill` joins this handle to guarantee the
        // mission_stop → mission_completed transition never races ahead of
        // the actual child reap.
        let reader_handle = self.start_reader_thread(
            session_id.clone(),
            Some(mission.id.clone()),
            child,
            reader,
            Arc::clone(&pool),
            Arc::clone(&events),
            runner.clone(),
            plan.resuming,
        );

        // Attach the reader handle. We raced to insert-first so the reader
        // may already be draining by the time we land here — that's fine,
        // it doesn't touch this slot.
        if let Some(h) = self.sessions.lock().unwrap().get_mut(&session_id) {
            h.reader = Some(reader_handle);
        }

        // Notify subscribers (Runners page, Runner Detail) that this
        // runner's activity counters changed. Don't fail the spawn if the
        // counter query hits a transient error — the spawn itself
        // succeeded; activity badges will reconcile on the next event.
        emit_runner_activity(&pool, runner, events.as_ref());

        // Deliver the runner's brief as a first user turn via stdin.
        // claude-code's interactive TUI ignores `--append-system-prompt`,
        // and codex's positional `[PROMPT]` argv loses races with its
        // startup permission / approval dialog — see
        // `router::runtime::system_prompt_args`. Skipped for the lead
        // — the `mission_goal` handler injects a richer launch prompt
        // that already embeds system_prompt.
        schedule_mission_first_prompt(self, session_id.clone(), runner, &plan, slot.lead);

        Ok(SpawnedSession {
            id: session_id,
            mission_id: Some(mission.id.clone()),
            runner_id: runner.id.clone(),
            handle: runner.handle.clone(),
            pid,
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
        let pty_system = portable_pty::native_pty_system();
        // Spawn at the caller's reported xterm grid when known. TUIs like
        // claude-code lay out their input frame on first paint and don't
        // gracefully redraw on later SIGWINCH, so booting at the wrong
        // size leaves a stale 80-col frame stranded in the buffer.
        let opened = PtySize {
            rows: rows.unwrap_or(24),
            cols: cols.unwrap_or(80),
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = pty_system
            .openpty(opened)
            .map_err(|e| Error::msg(format!("openpty: {e}")))?;

        // Agent-native session resume: `spawn_direct` always opens a *new*
        // chat. To resume a prior chat the caller goes through
        // `SessionManager::resume(session_id)`, which respawns the PTY
        // for an existing row instead of creating a new one. Here we
        // just let the runtime adapter self-assign a fresh
        // `agent_session_key` (claude-code) or leave it NULL (codex).
        // See docs/impls/direct-chats.md.
        let plan = crate::router::runtime::resume_plan(&runner.runtime, None);

        let mut cmd = CommandBuilder::new(&runner.command);
        if plan.prepend {
            for extra in &plan.args {
                cmd.arg(extra);
            }
        }
        cmd.args(&runner.args);
        if !plan.prepend {
            for extra in &plan.args {
                cmd.arg(extra);
            }
        }
        // Apply the same runtime adapter as the mission spawn so direct chat
        // sessions also receive the runner's `system_prompt`. Direct chats
        // get only the brief — no roster, no goal, no coordination notes —
        // so this is strictly the per-runner default. `plan.resuming` is
        // always false here (spawn_direct opens a new chat); pass it
        // through anyway for parity with the other spawn sites.
        for extra in crate::router::runtime::trailing_runtime_args(
            &runner.runtime,
            plan.resuming,
            runner.model.as_deref(),
            runner.effort.as_deref(),
            runner.system_prompt.as_deref(),
        ) {
            cmd.arg(extra);
        }

        // Working directory precedence: explicit `cwd` arg (the user picked
        // a folder in the Chat now dialog) ► runner's own `working_dir`
        // override ► inherit parent's. Mirrors `spawn`'s precedence so
        // behavior is consistent across mission and direct flavors.
        let resolved_cwd: Option<String> = cwd
            .map(|s| s.to_string())
            .or_else(|| runner.working_dir.clone());
        if let Some(wd) = resolved_cwd.as_deref() {
            cmd.cwd(wd);
        }

        for (k, v) in &runner.env {
            cmd.env(k, v);
        }
        // PATH for direct chat: the captured login-shell PATH only.
        // The bundled `runner` CLI is intentionally NOT prepended —
        // direct chats are off-bus (no `RUNNER_CREW_ID` /
        // `RUNNER_MISSION_ID` / `RUNNER_EVENT_LOG`), so verbs like
        // `runner msg post` would have no event log to write to and
        // no crew/mission to attribute against. Excluding the shim
        // keeps the agent from being tempted to call bus verbs that
        // would silently no-op or fail. Mission spawn keeps the
        // bundled CLI on PATH (see `spawn`).
        let _ = app_data_dir;
        let new_path = self.compose_path(None, None);
        cmd.env("PATH", new_path);
        cmd.env("RUNNER_HANDLE", &runner.handle);
        // Pass the spawn-time grid via COLUMNS/LINES too. portable-pty
        // sets the kernel winsize via TIOCSWINSZ at openpty time, but
        // some Node-based TUIs (claude-code, anything using ink) read
        // these env vars on startup as a fallback / hint and lay out
        // their initial UI from them, ignoring SIGWINCH that arrives
        // mid-render. Without this, claude-code paints its input frame
        // at whatever stale size it picked up.
        cmd.env("COLUMNS", opened.cols.to_string());
        cmd.env("LINES", opened.rows.to_string());
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        // Deliberately NOT setting RUNNER_CREW_ID, RUNNER_MISSION_ID,
        // RUNNER_EVENT_LOG, MISSION_CWD — direct chats are off-bus.

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| Error::msg(format!("spawn {}: {e}", runner.command)))?;
        drop(pair.slave);

        let pid = child.process_id();
        let session_id = ulid::Ulid::new().to_string();
        let started_at_dt = Utc::now();
        let started_at = started_at_dt.to_rfc3339();
        let setup_res: Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> = (|| {
            let reader = pair
                .master
                .try_clone_reader()
                .map_err(|e| Error::msg(format!("clone reader: {e}")))?;
            let writer = pair
                .master
                .take_writer()
                .map_err(|e| Error::msg(format!("take writer: {e}")))?;
            let conn = pool.get()?;
            // mission_id is NULL; cwd lives on the session row.
            conn.execute(
                "INSERT INTO sessions
                    (id, mission_id, runner_id, cwd, status, pid, started_at,
                     agent_session_key)
                 VALUES (?1, NULL, ?2, ?3, 'running', ?4, ?5, ?6)",
                params![
                    session_id,
                    runner.id,
                    resolved_cwd,
                    pid,
                    started_at,
                    plan.assigned_key
                ],
            )?;
            Ok((reader, writer))
        })();
        let (reader, writer) = match setup_res {
            Ok(rw) => rw,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };

        self.sessions.lock().unwrap().insert(
            session_id.clone(),
            SessionHandle {
                id: session_id.clone(),
                mission_id: None,
                runner_id: runner.id.clone(),
                master: Some(pair.master),
                writer: Mutex::new(writer),
                pid,
                reader: None,
            },
        );

        let reader_handle = self.start_reader_thread(
            session_id.clone(),
            None,
            child,
            reader,
            Arc::clone(&pool),
            Arc::clone(&events),
            runner.clone(),
            plan.resuming,
        );
        if let Some(h) = self.sessions.lock().unwrap().get_mut(&session_id) {
            h.reader = Some(reader_handle);
        }

        // Codex doesn't accept a caller-assigned session id at spawn,
        // so the runtime adapter leaves `assigned_key = None` for
        // fresh codex spawns. Kick off a short-lived watcher that
        // captures codex's auto-generated id from the rollout file
        // and writes it to `agent_session_key` so the *next* resume
        // can drive `codex resume <uuid>`.
        //
        // When `resolved_cwd` is None the child inherits the parent
        // process's cwd, which is what codex stamps into the rollout
        // file's `payload.cwd`. Match the watcher's expected cwd to
        // the same fallback so chats without an explicit cwd still
        // become resumable.
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

        // First-turn prompt injection for fresh claude-code / codex
        // direct chats. Direct chats are off-bus, so we send only the
        // persona (runner.system_prompt) — NO bus-contract preamble.
        // See `schedule_direct_first_prompt` for the rationale.
        schedule_direct_first_prompt(self, session_id.clone(), runner, &plan);

        Ok(SpawnedSession {
            id: session_id,
            mission_id: None,
            runner_id: runner.id.clone(),
            handle: runner.handle.clone(),
            pid,
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

        let pty_system = portable_pty::native_pty_system();
        let opened = PtySize {
            rows: rows.unwrap_or(24),
            cols: cols.unwrap_or(80),
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = pty_system
            .openpty(opened)
            .map_err(|e| Error::msg(format!("openpty: {e}")))?;

        // Resume plan: hand the prior agent_session_key back to the
        // runtime adapter so claude-code uses `--resume <uuid>` and
        // codex (once capture lands) uses `codex resume <uuid>`. If the
        // row's key is NULL (e.g. shell runtime, or codex pre-capture)
        // we just respawn fresh — same agent, no conversation state.
        //
        // claude-code only: if the conversation file for this
        // (cwd, uuid) was never persisted (e.g. the lead PTY was
        // reset before its first turn landed), `--resume <uuid>`
        // would print "No conversation found …" and leave the TUI
        // half-broken. Detect the missing file up front and degrade
        // to a fresh spawn that *keeps* the same uuid via
        // `--session-id`, so the row's existing key still binds to
        // the new conversation.
        let resolved_cwd_for_check: Option<String> =
            snap.cwd.clone().or_else(|| runner.working_dir.clone());
        let is_lead_slot = mission_ctx.as_ref().is_some_and(|c| c.lead);
        let conversation_missing = matches!(
            (runner.runtime.as_str(), snap.agent_session_key.as_deref()),
            ("claude-code", Some(key))
                if !crate::router::runtime::claude_code_conversation_exists(
                    resolved_cwd_for_check.as_deref(),
                    key,
                )
        );
        // Lead-only signal back to the caller: when a lead's prior
        // conversation file is missing, the resume degrades to a
        // fresh spawn and the bus's mission_goal handler will NOT
        // fire (mission_attach's watermark suppresses replay), so
        // the lead would come up with no system context. The caller
        // in commands/session.rs uses this flag to ask the router
        // to fire the launch prompt manually after the resume
        // returns.
        let fresh_fallback_lead = conversation_missing && is_lead_slot;
        let effective_prior_key = match (runner.runtime.as_str(), snap.agent_session_key.as_deref())
        {
            ("claude-code", Some(_)) if conversation_missing => None,
            (_, k) => k,
        };
        let plan = crate::router::runtime::resume_plan(&runner.runtime, effective_prior_key);

        let mut cmd = CommandBuilder::new(&runner.command);
        if plan.prepend {
            for extra in &plan.args {
                cmd.arg(extra);
            }
        }
        cmd.args(&runner.args);
        if !plan.prepend {
            for extra in &plan.args {
                cmd.arg(extra);
            }
        }
        // Both supported runtimes deliver the system prompt via stdin
        // (`schedule_first_prompt`), so `system_prompt_args` is empty
        // for both and the helper's `plan_resuming` carve-out is a
        // no-op today — kept on the call so the contract stays
        // self-describing for any future runtime that opts back into
        // positional argv.
        for extra in crate::router::runtime::trailing_runtime_args(
            &runner.runtime,
            plan.resuming,
            runner.model.as_deref(),
            runner.effort.as_deref(),
            runner.system_prompt.as_deref(),
        ) {
            cmd.arg(extra);
        }

        // Working directory: same precedence as `spawn_direct` — the
        // row's stored cwd (per-chat override the user picked when
        // starting the chat) wins; otherwise fall back to the
        // runner's current `working_dir`. NULL on both means inherit
        // parent's cwd. Without the fallback, sessions originally
        // spawned with no explicit cwd would land in the dev server's
        // cwd on every resume, ignoring later edits to the runner.
        let resolved_cwd: Option<String> = snap.cwd.clone().or_else(|| runner.working_dir.clone());
        if let Some(wd) = resolved_cwd.as_deref() {
            cmd.cwd(wd);
        }

        for (k, v) in &runner.env {
            cmd.env(k, v);
        }
        // Refresh the per-slot runner shim before computing PATH so it
        // picks up any post-spawn env changes (mission cwd edited,
        // etc.) for THIS resume cycle. Direct chats skip — no
        // mission_ctx, no shim.
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

        // Direct-chat resume keeps the bundled `runner` CLI off PATH —
        // same off-bus rationale as `spawn_direct`. Mission resume
        // re-prepends `<app_data_dir>/bin` so `runner` verbs in the
        // PTY behave the same as the original mission spawn.
        let bin_dir = mission_ctx.as_ref().map(|_| app_data_dir.join("bin"));
        let new_path = self.compose_path(shim_dir.as_deref(), bin_dir.as_deref());
        cmd.env("PATH", new_path);
        // Mission resume stamps the slot's in-mission identity so the
        // bundled `runner` CLI in this PTY attributes events as the
        // slot, not the runner template. Direct chat falls through to
        // the template handle. Crew/mission ids surface for
        // `runner signal` / `runner msg post` calls inside the PTY,
        // and RUNNER_EVENT_LOG / MISSION_CWD parity the original
        // mission spawn so the bundled CLI can find the event log
        // and tools that read $MISSION_CWD keep working post-resume.
        if let Some(ctx) = mission_ctx.as_ref() {
            cmd.env("RUNNER_CREW_ID", &ctx.crew_id);
            cmd.env("RUNNER_MISSION_ID", &ctx.mission_id);
            cmd.env("RUNNER_HANDLE", &ctx.slot_handle);
            let event_log_path = runner_core::event_log::path::events_path(
                app_data_dir,
                &ctx.crew_id,
                &ctx.mission_id,
            );
            cmd.env(
                "RUNNER_EVENT_LOG",
                event_log_path.to_string_lossy().to_string(),
            );
            if let Some(wd) = ctx.mission_cwd.as_deref() {
                cmd.env("MISSION_CWD", wd);
            }
        } else {
            cmd.env("RUNNER_HANDLE", &runner.handle);
        }
        cmd.env("COLUMNS", opened.cols.to_string());
        cmd.env("LINES", opened.rows.to_string());
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| Error::msg(format!("spawn {}: {e}", runner.command)))?;
        drop(pair.slave);

        let pid = child.process_id();
        let started_at_dt = Utc::now();
        let started_at = started_at_dt.to_rfc3339();

        let setup_res: Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> = (|| {
            let reader = pair
                .master
                .try_clone_reader()
                .map_err(|e| Error::msg(format!("clone reader: {e}")))?;
            let writer = pair
                .master
                .take_writer()
                .map_err(|e| Error::msg(format!("take writer: {e}")))?;
            let conn = pool.get()?;
            // UPDATE in place: same id, same conversation thread,
            // refreshed runtime metadata. agent_session_key is rewritten
            // to whatever the adapter chose — claude-code preserves the
            // prior UUID; codex's adapter only assigns a key when
            // resuming with a known one (`codex resume <uuid>`). For a
            // fresh codex spawn (no prior key), assigned_key is None
            // and the watcher kicked off below captures codex's
            // auto-generated id post-spawn.
            conn.execute(
                "UPDATE sessions
                    SET status = 'running',
                        pid = ?2,
                        started_at = ?3,
                        stopped_at = NULL,
                        agent_session_key = COALESCE(?4, agent_session_key)
                  WHERE id = ?1",
                params![session_id, pid, started_at, plan.assigned_key],
            )?;
            Ok((reader, writer))
        })();
        let (reader, writer) = match setup_res {
            Ok(rw) => rw,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };

        self.sessions.lock().unwrap().insert(
            session_id.to_string(),
            SessionHandle {
                id: session_id.to_string(),
                mission_id: snap.mission_id.clone(),
                runner_id: runner.id.clone(),
                master: Some(pair.master),
                writer: Mutex::new(writer),
                pid,
                reader: None,
            },
        );

        // Drop the prior session's output buffer just before the
        // reader thread starts pumping chunks for the new PTY. The
        // monotonic seq counter is intentionally kept (so the new
        // chunk seq continues at `last + 1` and the frontend's
        // seq-merge filter doesn't drop the head of post-resume
        // output). Purging here — after the spawn + DB UPDATE
        // succeeded — ensures we don't wipe the buffer on a path
        // that ends up returning Err.
        self.purge_output_buffer(session_id);

        let reader_handle = self.start_reader_thread(
            session_id.to_string(),
            snap.mission_id.clone(),
            child,
            reader,
            Arc::clone(&pool),
            Arc::clone(&events),
            runner.clone(),
            plan.resuming,
        );
        if let Some(h) = self.sessions.lock().unwrap().get_mut(session_id) {
            h.reader = Some(reader_handle);
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
            pid,
            fresh_fallback_lead,
        })
    }

    /// Common reader-thread machinery used by both `spawn` (mission) and
    /// `spawn_direct`. Drains the PTY, reaps the child, flips the DB row,
    /// removes the live-map entry, and emits `session/exit`. Whatever
    /// invoked spawn doesn't get a return until `kill` joins this handle,
    /// which is what mission_stop relies on for the no-lying-about-
    /// termination contract.
    // The reader thread genuinely needs every one of these — session_id /
    // mission_id for event payloads, child + reader for the PTY drain, pool
    // for the DB row update, events for emitter dispatch, runner for the
    // post-reap activity recompute. Bundling into a Context struct just
    // moves the same arity to the call site without buying clarity.
    #[allow(clippy::too_many_arguments)]
    fn start_reader_thread(
        self: &Arc<Self>,
        session_id: String,
        mission_id: Option<String>,
        mut child: Box<dyn portable_pty::Child + Send + Sync>, // portable-pty's Child is Send + Sync; both needed for thread::spawn move + the &mut reborrow inside drain_pty_and_reap.
        reader: Box<dyn Read + Send>,
        pool: Arc<DbPool>,
        events: Arc<dyn SessionEvents>,
        runner: Runner,
        resuming: bool,
    ) -> thread::JoinHandle<()> {
        let manager_t: Arc<SessionManager> = Arc::clone(self);
        let started_at = std::time::Instant::now();
        thread::spawn(move || {
            let exit = drain_pty_and_reap(
                reader,
                &mut *child,
                manager_t.as_ref(),
                &session_id,
                mission_id.as_deref(),
                events.as_ref(),
            );
            let _ = manager_t.forget(&session_id);
            // Was the user-initiated kill the cause of this exit?
            // Drain the killed-set entry here so subsequent spawns of
            // the same id (resume cycles) don't inherit a stale
            // "intentional" flag.
            let was_killed = manager_t.killed.lock().unwrap().remove(&session_id);
            // Resume failure heuristic: we asked the agent to resume a
            // prior conversation, but the child died fast and unhappy.
            // Either the agent rejected the prior id, or the runtime
            // doesn't actually have that conversation on disk anymore.
            // Wipe `agent_session_key` on this row so the next lookup
            // skips it and the next spawn falls back to a fresh
            // conversation; surface a banner so the user knows. An
            // explicit kill is never a resume failure — the user
            // pulled the plug on purpose.
            let resume_failed = resuming
                && !exit.success
                && !was_killed
                && started_at.elapsed() < std::time::Duration::from_secs(3);
            // Status classification:
            //   - exit.success → `stopped` (clean child exit)
            //   - was_killed → `stopped` (intentional kill via Stop /
            //     Archive / mission teardown — SIGTERM is non-zero by
            //     design but isn't a crash)
            //   - else → `crashed`
            let final_status = if exit.success || was_killed {
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
            // Activity dropped — emit before `exit` so the Runners list
            // sees the new counts before any session_id-keyed UI cleans up.
            emit_runner_activity(&pool, &runner, events.as_ref());
            events.exit(&exit);
        })
    }

    /// Write raw bytes to the session's stdin.
    pub fn inject_stdin(&self, session_id: &str, bytes: &[u8]) -> Result<()> {
        let sessions = self.sessions.lock().unwrap();
        let handle = sessions
            .get(session_id)
            .ok_or_else(|| Error::msg(format!("session not found: {session_id}")))?;
        let mut writer = handle.writer.lock().unwrap();
        writer.write_all(bytes)?;
        writer.flush()?;
        Ok(())
    }

    /// Resize the session's PTY. Issues the equivalent of an SIGWINCH so
    /// the child re-renders into the new grid. Frontend calls this after
    /// xterm fits to the container — without it, claude-code stays at
    /// the spawn-time 80×24 regardless of how big the visible grid is.
    pub fn resize(&self, session_id: &str, cols: u16, rows: u16) -> Result<()> {
        let sessions = self.sessions.lock().unwrap();
        let handle = sessions
            .get(session_id)
            .ok_or_else(|| Error::msg(format!("session not found: {session_id}")))?;
        if let Some(master) = handle.master.as_ref() {
            master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| Error::msg(format!("pty resize failed: {e}")))?;
        }
        Ok(())
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
        let (pid, master, reader) = {
            let mut sessions = self.sessions.lock().unwrap();
            match sessions.remove(session_id) {
                Some(mut h) => (h.pid, h.master.take(), h.reader.take()),
                None => return Ok(()),
            }
        };

        // Mark the kill as intentional so the reader thread classifies
        // the upcoming non-zero exit as `stopped`, not `crashed`. SIGTERM
        // typically produces exit code 143; without this flag every
        // user-initiated stop would surface as a crash in the UI.
        self.killed.lock().unwrap().insert(session_id.to_string());

        // Step 2: hang up the terminal. For most children this alone is
        // enough. We drop before sending signals so the child's next I/O
        // fails instead of blocking indefinitely.
        drop(master);

        // Step 3: Unix-only hard-kill escalation.
        #[cfg(unix)]
        if let Some(pid) = pid {
            // SAFETY: `pid` came from `Child::process_id()` on a child we
            // just started; it hasn't been reaped yet because the reader
            // thread holds the only `Child` reference. `kill(2)` with an
            // unknown pid is a no-op returning ESRCH which we ignore.
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGTERM);
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
        }
        #[cfg(not(unix))]
        let _ = pid; // Windows path lands with a future chunk.

        // Step 4: wait for the reader to reap + update the DB + emit exit.
        if let Some(h) = reader {
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
/// swallowed by a dialog still on screen. `cfg(test)` zeros it so unit
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
    let body: String = prompt.chars().filter(|c| *c != '\r').collect();
    let delay = FIRST_PROMPT_DELAY;
    if delay.is_zero() {
        // Inline path used by unit tests (`FIRST_PROMPT_DELAY = ZERO`
        // under `cfg(test)`) so synchronous output assertions can
        // observe the injection without waiting on a background
        // thread. Production never hits this branch.
        let _ = mgr.inject_stdin(&session_id, body.as_bytes());
        let _ = mgr.inject_stdin(&session_id, b"\r");
        return;
    }
    let mgr = Arc::clone(mgr);
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        let _ = mgr.inject_stdin(&session_id, body.as_bytes());
        std::thread::sleep(std::time::Duration::from_millis(80));
        let _ = mgr.inject_stdin(&session_id, b"\r");
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
        // Same 2.5s budget as `schedule_first_prompt` — claude-code
        // shows the prior conversation history first, and we want
        // the editor bound before typing.
        std::thread::sleep(std::time::Duration::from_millis(2500));
        let _ = mgr.inject_stdin(&session_id, b"continue");
        std::thread::sleep(std::time::Duration::from_millis(80));
        let _ = mgr.inject_stdin(&session_id, b"\r");
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
fn drain_pty_and_reap(
    mut reader: Box<dyn Read + Send>,
    child: &mut (dyn portable_pty::Child + Send),
    manager: &SessionManager,
    session_id: &str,
    mission_id: Option<&str>,
    events: &dyn SessionEvents,
) -> ExitEvent {
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let ev = manager.record_output(session_id, mission_id, BASE64.encode(&buf[..n]));
                events.output(&ev);
            }
            Err(_) => break,
        }
    }
    let (exit_code, success) = match child.wait() {
        Ok(status) => {
            let code = status.exit_code() as i32;
            (Some(code), status.success())
        }
        Err(_) => (None, false),
    };
    ExitEvent {
        session_id: session_id.into(),
        mission_id: mission_id.map(str::to_string),
        exit_code,
        success,
    }
}

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
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

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

    #[test]
    fn compose_path_orders_segments_and_skips_empties() {
        // shim ► bin ► shell ► parent — and any None / empty segment
        // is skipped without leaving stray separators. The shell PATH
        // segment is the v0.1.x fix for GUI-launched apps not seeing
        // Homebrew/mise/etc.
        let prior = std::env::var_os("PATH");
        // Deterministic parent for the duration of the test.
        std::env::set_var("PATH", "/usr/bin:/bin");

        let with_shell = SessionManager::new(Some(
            "/opt/homebrew/bin:/Users/x/.npm-global/bin".to_string(),
        ));
        let bin = std::path::PathBuf::from("/app/bin");
        let shim = std::path::PathBuf::from("/app/shim/m1/s1");

        let p = with_shell.compose_path(Some(&shim), Some(&bin));
        assert_eq!(
            p.to_string_lossy(),
            "/app/shim/m1/s1:/app/bin:/opt/homebrew/bin:/Users/x/.npm-global/bin:/usr/bin:/bin"
        );

        // No shim, no shell PATH — only bin + parent.
        let bare = SessionManager::new(None);
        let p = bare.compose_path(None, Some(&bin));
        assert_eq!(p.to_string_lossy(), "/app/bin:/usr/bin:/bin");

        // Empty shell PATH is treated as None (no doubled separator).
        let empty_shell = SessionManager::new(Some(String::new()));
        let p = empty_shell.compose_path(None, Some(&bin));
        assert_eq!(p.to_string_lossy(), "/app/bin:/usr/bin:/bin");

        // Empty parent PATH (unset) — no trailing separator.
        std::env::remove_var("PATH");
        let p = with_shell.compose_path(None, Some(&bin));
        assert_eq!(
            p.to_string_lossy(),
            "/app/bin:/opt/homebrew/bin:/Users/x/.npm-global/bin"
        );

        // Direct-chat path: `bin_dir = None` skips the bundled CLI
        // entirely. The bundled `runner` shim is intentionally not on
        // PATH for direct chat (#51) — assertion mirrors the
        // production caller in `spawn_direct`.
        std::env::set_var("PATH", "/usr/bin:/bin");
        let p = with_shell.compose_path(None, None);
        assert_eq!(
            p.to_string_lossy(),
            "/opt/homebrew/bin:/Users/x/.npm-global/bin:/usr/bin:/bin",
            "direct-chat PATH must not contain the bundled bin_dir"
        );

        match prior {
            Some(v) => std::env::set_var("PATH", v),
            None => std::env::remove_var("PATH"),
        }
    }

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

        let mgr = SessionManager::new(None);
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
    fn spawn_echo_roundtrip() {
        // Spawn `sh -c "echo hi && exit"`; assert the exit event fires with
        // success=true. We skip output inspection because the Tauri mock app
        // doesn't let us subscribe to events from a test.
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

        let mgr = SessionManager::new(None);
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
        assert!(spawned.pid.is_some());

        // Poll the DB until the reader thread has marked the session stopped.
        let deadline = Instant::now() + Duration::from_secs(5);
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
    }

    #[test]
    fn inject_stdin_roundtrip() {
        // Spawn `cat`, inject "hello\n", then kill. `cat` reads until stdin
        // closes; killing the session drops the master PTY, which on Unix
        // hangs up and `cat` sees EOF.
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

        let mgr = SessionManager::new(None);
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
        // Brief wait so `cat` echoes before we hang up.
        std::thread::sleep(Duration::from_millis(100));
        mgr.kill(&spawned.id).unwrap();

        // After kill, reader thread exits and updates the row.
        let deadline = Instant::now() + Duration::from_secs(5);
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
        let mgr = SessionManager::new(None);
        let err = mgr.inject_stdin("nope", b"x").unwrap_err();
        assert!(format!("{err}").contains("session not found"));
    }

    /// Drain the bounded output buffer until either `predicate` returns
    /// true on the merged decoded text or the deadline elapses.
    /// Returns the merged text seen at the point of break, plus the
    /// boolean predicate result. Used by direct-chat / mission first-
    /// turn injection tests that need to wait on `/bin/cat` echoing
    /// the typed bytes back through the PTY.
    fn await_pty_output<F>(
        mgr: &Arc<SessionManager>,
        session_id: &str,
        predicate: F,
        timeout: Duration,
    ) -> (String, bool)
    where
        F: Fn(&str) -> bool,
    {
        let deadline = Instant::now() + timeout;
        loop {
            let snapshot = mgr.output_snapshot(session_id);
            let merged: String = snapshot
                .iter()
                .filter_map(|ev| {
                    BASE64
                        .decode(ev.data.as_bytes())
                        .ok()
                        .and_then(|b| String::from_utf8(b).ok())
                })
                .collect();
            if predicate(&merged) {
                return (merged, true);
            }
            if Instant::now() > deadline {
                return (merged, false);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn codex_direct_chat_injects_persona_without_preamble() {
        // Direct chats are off-bus: the bundled `runner` CLI is not on
        // PATH (#51) and there's no crew/mission to coordinate over,
        // so the WORKER_COORDINATION_PREAMBLE would advertise verbs
        // that don't work. Direct chat sends ONLY the persona
        // (runner.system_prompt) into stdin.
        //
        // Assert (a) the persona token DOES appear in the captured
        // output (echoed back by /bin/cat) and (b) a distinctive
        // substring of WORKER_COORDINATION_PREAMBLE does NOT —
        // regression guard for the bug in #51.
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

        let mgr = SessionManager::new(None);
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

        let (merged, saw_persona) = await_pty_output(
            &mgr,
            &spawned.id,
            |text| text.contains("CODEX_PERSONA_TOKEN"),
            Duration::from_secs(5),
        );
        mgr.kill(&spawned.id).unwrap();
        assert!(
            saw_persona,
            "codex direct chat must inject the persona via stdin: {merged:?}",
        );
        // Distinctive opening of the preamble — picked because no
        // persona prompt would naturally contain the literal "in a
        // crew coordinated by the bundled" substring.
        assert!(
            !merged.contains("in a crew coordinated by the bundled"),
            "direct chat must NOT inject WORKER_COORDINATION_PREAMBLE: {merged:?}",
        );
    }

    #[test]
    fn claude_code_direct_chat_injects_persona_without_preamble() {
        // Same shape as the codex test, but with claude-code runtime
        // — claude-code's `--append-system-prompt` is SDK-only
        // (silently dropped by the interactive TUI), so stdin is the
        // only persona-delivery path.
        //
        // claude-code's `resume_plan` self-assigns `--session-id <uuid>`
        // for fresh spawns, which gets appended to the spawn argv. We
        // wrap with `/bin/sh -c 'cat'` so those extras land as the
        // shell's positional params (consumed by `sh`, not passed to
        // `cat`) and don't make the test child error out.
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

        let mgr = SessionManager::new(None);
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

        let (merged, saw_persona) = await_pty_output(
            &mgr,
            &spawned.id,
            |text| text.contains("CC_PERSONA_TOKEN"),
            Duration::from_secs(5),
        );
        mgr.kill(&spawned.id).unwrap();
        assert!(
            saw_persona,
            "claude-code direct chat must inject the persona via stdin: {merged:?}",
        );
        assert!(
            !merged.contains("in a crew coordinated by the bundled"),
            "direct chat must NOT inject WORKER_COORDINATION_PREAMBLE: {merged:?}",
        );
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
        //
        // Wrapped in `/bin/sh -c 'cat'` so claude-code's self-assigned
        // `--session-id <uuid>` fresh-spawn args don't crash the test
        // child (cat would reject the unknown flag); the shell
        // swallows them as positional params.
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

        let mgr = SessionManager::new(None);
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

        // Wait for BOTH halves before killing — the preamble lands
        // first (it's typed first), and the brief follows. Killing
        // the PTY between the two breaks cat's read loop and the
        // brief never makes it back through the master. The shorter
        // PERSONA_TOK sentinel reliably lands in a single chunk
        // even though the full WORKER_PERSONA_TOKEN can split
        // across kernel-echo / cat-read boundaries.
        let (merged, saw_both) = await_pty_output(
            &mgr,
            &spawned.id,
            |text| {
                text.contains("in a crew coordinated by the bundled")
                    && text.contains("PERSONA_TOK")
            },
            Duration::from_secs(5),
        );
        mgr.kill(&spawned.id).unwrap();
        assert!(
            saw_both,
            "mission spawn must inject WORKER_COORDINATION_PREAMBLE + the user-authored brief \
             for a non-lead worker: {merged:?}",
        );
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

        let mgr = SessionManager::new(None);
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

        // Give the (would-be) injection thread a chance to fire.
        std::thread::sleep(Duration::from_millis(100));
        let snapshot = mgr.output_snapshot(&resumed.id);
        let merged: String = snapshot
            .iter()
            .filter_map(|ev| {
                BASE64
                    .decode(ev.data.as_bytes())
                    .ok()
                    .and_then(|b| String::from_utf8(b).ok())
            })
            .collect();
        mgr.kill(&resumed.id).unwrap();
        assert!(
            !merged.contains("CODEX_BRIEF_TOKEN_RESUME"),
            "codex resume must NOT replay the brief over stdin: {merged:?}"
        );
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

        let mgr = SessionManager::new(None);
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
        // mission_stop relies on this contract: kill must return only after
        // the reader thread has updated the DB row to stopped/crashed.
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

        let mgr = SessionManager::new(None);
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

        // kill must synchronize on the reader; immediately after it returns,
        // the DB row should already be terminal (no polling).
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
        let mgr = SessionManager::new(None);
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

        // Wait for the child to exit so the test isn't racing with the
        // reader thread for the activity drop.
        let deadline = Instant::now() + Duration::from_secs(5);
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

        // Buffer survives the PTY exit so a remount of the chat (or
        // a navigate-away-and-back) can still replay the dead
        // session's scrollback via `session_output_snapshot`. The
        // explicit cleanup path is `purge_session_buffers`.

        // Activity emissions: at least one on spawn (count=1), and one on
        // reap (count=0). We don't pin exact counts — the spawn-time emit
        // could race the reap if the child is fast — but the *last*
        // emission must show zero active sessions for this runner.
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

        let mgr = SessionManager::new(None);
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

        mgr.inject_stdin(&spawned.id, b"hello snapshot\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
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
        // Multi-chat-per-runner contract: a direct chat IS a sessions
        // row. spawn_direct creates the row and the agent CLI's UUID
        // (for claude-code; shell here just exits). After kill, resume
        // respawns the *same* row — same id, same agent_session_key —
        // and flips status back to running. See docs/impls/direct-chats.md.
        let pool = pool_with_schema();
        let now = Utc::now().to_rfc3339();
        let runner_id = ulid::Ulid::new().to_string();
        {
            let conn = pool.get().unwrap();
            // Use claude-code runtime so resume_plan self-assigns a
            // UUID and persists it. We don't actually exec claude here —
            // the spawn path uses runner.command (set to /bin/sh) so
            // the test runs without external deps.
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

        let mgr = SessionManager::new(None);
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

        // Wait for the child to exit naturally so the row's status
        // flips to stopped before we attempt resume.
        let deadline = Instant::now() + Duration::from_secs(5);
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

        // The claude-code adapter persisted a UUID under
        // `--session-id`; capture it for the resume comparison.
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
            "claude-code spawn must persist an agent_session_key for later resume"
        );

        // Resume: same row, same id. Use a runner pointing at a
        // different cmd to confirm the resume reads the *current*
        // runner config from the row.
        {
            let conn = pool.get().unwrap();
            conn.execute(
                "UPDATE runners SET command = '/bin/sh',
                                    args_json = ?2
                  WHERE id = ?1",
                params![runner_id, "[\"-c\",\"echo resumed\"]"],
            )
            .unwrap();
        }
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

        // Wait for the resumed child to exit, then assert the key
        // survived. claude-code's resume_plan re-uses the same UUID, so
        // the column must be non-null and match the prior value.
        let deadline = Instant::now() + Duration::from_secs(5);
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
                panic!("resumed spawn never exited");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let key_after: Option<String> = {
            let conn = pool.get().unwrap();
            conn.query_row(
                "SELECT agent_session_key FROM sessions WHERE id = ?1",
                params![&session_id],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(
            key_after, key_before,
            "resume must preserve agent_session_key for claude-code"
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
        let mgr = SessionManager::new(None);
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
        // Mission resume must look up the slot for the session and use
        // slot.slot_handle as RUNNER_HANDLE, not runner.handle. The
        // bundled CLI relies on this env var to attribute events to the
        // in-mission identity. We verify by spawning a shell that
        // echoes the var, then reading the captured output buffer.
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

        let mgr = SessionManager::new(None);
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

        // Wait for the child to exit so the buffer is fully drained.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let status: String = pool
                .get()
                .unwrap()
                .query_row("SELECT status FROM sessions WHERE id = 'mr-sid'", [], |r| {
                    r.get(0)
                })
                .unwrap();
            if status != "running" {
                break;
            }
            if Instant::now() > deadline {
                panic!("mission resume never exited");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let snapshot = mgr.output_snapshot("mr-sid");
        // Output chunks carry base64'd payloads (IPC-friendly). Decode
        // and concatenate to verify the env-echo landed.
        use base64::Engine;
        let combined: String = snapshot
            .iter()
            .filter_map(|c| {
                base64::engine::general_purpose::STANDARD
                    .decode(&c.data)
                    .ok()
            })
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .collect();
        assert!(
            combined.contains("HANDLE=architect-slot"),
            "RUNNER_HANDLE must be the slot_handle, got: {combined:?}"
        );
    }
}
