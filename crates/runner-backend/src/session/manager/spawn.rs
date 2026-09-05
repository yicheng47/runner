use super::*;

use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};

const FORK_MATERIALIZE_TIMEOUT: Duration = Duration::from_secs(120);
const FORK_MATERIALIZE_POLL: Duration = Duration::from_millis(25);
const CODEX_FORK_ROLLOUT_TIMEOUT: Duration = Duration::from_secs(5);

const LOCALE_VARS: [&str; 3] = ["LANG", "LC_ALL", "LC_CTYPE"];

/// Dock-launched GUI apps inherit no locale, so children run in the
/// POSIX C locale and macOS clipboard tools (claude's `pbpaste`)
/// decode UTF-8 as Mac Roman — CJK text arrives as mojibake (#461).
/// Mirror alacritty's minimal macOS fallback: set `LC_CTYPE=UTF-8`
/// only when no layer supplies a locale (a full `LANG` would also
/// switch tool message languages). Children inherit the app's own
/// process env on top of the spec, so a locale there counts too.
pub(super) fn ensure_utf8_locale(env: &mut BTreeMap<String, String>, process_has_locale: bool) {
    if process_has_locale || LOCALE_VARS.iter().any(|var| env.contains_key(*var)) {
        return;
    }
    env.insert("LC_CTYPE".into(), "UTF-8".into());
}

pub(super) fn run_headless_fork(
    spec: &SpawnSpec,
    plan: &router::runtime::ForkPlan,
    timeout: Duration,
) -> Result<String> {
    let router::runtime::ForkPlan::Headless { args, source_key } = plan else {
        return Err(Error::msg(
            "direct fork plan cannot run as a headless materializer",
        ));
    };
    let codex_sessions_root = codex_fork_sessions_root(spec)?;
    let inherited_path = std::env::var("PATH").ok();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let path = crate::session::launch::compose_path(
        spec.shim_dir.as_deref(),
        spec.bundled_bin_dir.as_deref(),
        spec.shell_path.as_deref(),
        home.as_deref(),
        inherited_path.as_deref(),
    );
    let mut command = Command::new(&spec.command);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PATH", path);
    if let Some(cwd) = spec.cwd.as_deref() {
        command.current_dir(cwd);
    }
    for (name, value) in &spec.env {
        if crate::session::launch::is_reserved_env_name(name) {
            continue;
        }
        if !crate::session::launch::is_valid_env_name(name) {
            return Err(Error::msg(format!(
                "invalid env var name {name:?}: must match [A-Za-z_][A-Za-z0-9_]*"
            )));
        }
        command.env(name, value);
    }
    if let Some((cols, rows)) = spec.initial_size {
        command.env("COLUMNS", cols.to_string());
        command.env("LINES", rows.to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let mut child = command
        .spawn()
        .map_err(|error| Error::msg(format!("fork materialization spawn: {error}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::msg("fork materialization stdout unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::msg("fork materialization stderr unavailable"))?;

    run_thread_started_headless_fork(
        child,
        stdout,
        stderr,
        codex_sessions_root,
        source_key,
        timeout,
    )
}

fn run_thread_started_headless_fork(
    mut child: std::process::Child,
    stdout: std::process::ChildStdout,
    stderr: std::process::ChildStderr,
    sessions_root: PathBuf,
    source_key: &str,
    timeout: Duration,
) -> Result<String> {
    let (key_tx, key_rx) = std::sync::mpsc::sync_channel(1);
    let stdout_reader = thread::spawn(move || {
        let mut stdout = BufReader::new(stdout);
        let _ = key_tx.send(read_thread_started_event(&mut stdout));
        let _ = std::io::copy(&mut stdout, &mut std::io::sink());
    });
    let stderr_reader = thread::spawn(move || {
        let mut stderr = stderr;
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });

    let deadline = Instant::now() + timeout;
    let mut key = None;
    let mut rollout_deadline = None;
    let mut child_status = None;
    let mut failed_status = None;
    let mut terminate = false;
    let result = loop {
        if key.is_none() {
            match key_rx.try_recv() {
                Ok(Ok(received)) => {
                    key = Some(received);
                    rollout_deadline = Some(std::cmp::min(
                        deadline,
                        Instant::now() + CODEX_FORK_ROLLOUT_TIMEOUT,
                    ));
                }
                Ok(Err(error)) => {
                    terminate = child_status.is_none();
                    break Err(error);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    terminate = child_status.is_none();
                    break Err(Error::msg(
                        "fork materialization stdout reader stopped before thread.started",
                    ));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }

        if child_status.is_none() {
            match child.try_wait() {
                Ok(Some(status)) => child_status = Some(status),
                Ok(None) => {}
                Err(error) => {
                    terminate = true;
                    break Err(Error::msg(format!(
                        "fork materialization wait failed: {error}"
                    )));
                }
            }
        }
        if child_status
            .as_ref()
            .is_some_and(|status| !status.success())
        {
            failed_status = child_status.take();
            break Err(Error::msg("fork materialization exited unsuccessfully"));
        }

        if let Some(key) = key.as_deref() {
            if crate::session::codex_capture::fork_rollout_is_ready(&sessions_root, key, source_key)
            {
                terminate = child_status.is_none();
                break Ok(key.to_string());
            }
            if rollout_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                terminate = child_status.is_none();
                break Err(Error::msg(format!(
                    "fork materialization rollout for thread {key} did not appear before the deadline"
                )));
            }
        }

        if Instant::now() >= deadline {
            terminate = child_status.is_none();
            break Err(Error::msg(format!(
                "fork materialization timed out after {} seconds",
                timeout.as_secs()
            )));
        }

        thread::sleep(FORK_MATERIALIZE_POLL);
    };

    if terminate {
        terminate_headless_fork(&mut child);
    }
    stdout_reader
        .join()
        .map_err(|_| Error::msg("fork materialization stdout reader panicked"))?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| Error::msg("fork materialization stderr reader panicked"))?;
    if let Some(status) = failed_status {
        let stderr = String::from_utf8_lossy(&stderr);
        return Err(Error::msg(format!(
            "fork materialization exited with {status}: {}",
            stderr.trim()
        )));
    }
    result
}

fn read_thread_started_event(reader: &mut impl BufRead) -> Result<String> {
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| Error::msg(format!("fork materialization output: {error}")))?;
        if read == 0 {
            return Err(Error::msg(
                "fork materialization missing thread.started event",
            ));
        }
        if !line.trim().is_empty() {
            break;
        }
    }
    let event: serde_json::Value = serde_json::from_str(line.trim_end()).map_err(|error| {
        Error::msg(format!(
            "fork materialization first event is invalid JSON: {error}"
        ))
    })?;
    if event.get("type").and_then(serde_json::Value::as_str) != Some("thread.started") {
        return Err(Error::msg(
            "fork materialization first event is not thread.started",
        ));
    }
    let key = event
        .get("thread_id")
        .and_then(serde_json::Value::as_str)
        .filter(|key| uuid::Uuid::parse_str(key).is_ok())
        .ok_or_else(|| Error::msg("fork materialization thread.started has no valid thread_id"))?;
    Ok(key.to_string())
}

fn codex_fork_sessions_root(spec: &SpawnSpec) -> Result<PathBuf> {
    let codex_home = spec
        .env
        .get("CODEX_HOME")
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("CODEX_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
        .ok_or_else(|| Error::msg("fork materialization cannot resolve Codex sessions root"))?;
    let codex_home = if codex_home.is_absolute() {
        codex_home
    } else if let Some(cwd) = spec.cwd.as_deref() {
        cwd.join(codex_home)
    } else {
        std::env::current_dir()
            .map_err(|error| Error::msg(format!("fork materialization cwd: {error}")))?
            .join(codex_home)
    };
    Ok(codex_home.join("sessions"))
}

fn terminate_headless_fork(child: &mut std::process::Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn delete_failed_fork(pool: &DbPool, session_id: &str) -> Result<()> {
    let mut conn = pool.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    crate::repo::node::remove_session(&tx, session_id)?;
    crate::repo::session::delete(&tx, session_id)?;
    tx.commit()?;
    Ok(())
}

impl SessionManager {
    fn resolve_runner_executable(&self, runner: &Runner, pool: &DbPool) -> Result<Runner> {
        let Some(definition) = router::runtime::runtime_definition(&runner.runtime) else {
            return Ok(runner.clone());
        };
        if runner.command != definition.command {
            return Ok(runner.clone());
        }
        let effective = crate::runtime_status::effective_runtime_command(
            definition.name,
            pool,
            &self.shell_env,
            &self.discovery_state,
        )?;
        let mut resolved = runner.clone();
        resolved.command = effective.command;
        Ok(resolved)
    }

    fn resolve_runtime_only_resume_runner(
        &self,
        runtime: &str,
        recorded_command: Option<&str>,
        model: Option<&str>,
        effort: Option<&str>,
        pool: &DbPool,
    ) -> Result<Runner> {
        if runtime == "shell" {
            let command = recorded_command
                .map(str::trim)
                .filter(|command| !command.is_empty())
                .ok_or_else(|| Error::msg("shell session missing agent_command"))?;
            return runtime_direct_runner("shell", Some(command), None, None);
        }
        let definition = router::runtime::runtime_definition(runtime)
            .ok_or_else(|| Error::msg(format!("unknown runtime: {runtime}")))?;
        let recorded = recorded_command
            .map(str::trim)
            .filter(|command| !command.is_empty());
        if let Some(command) = recorded {
            let path = Path::new(command);
            if path.is_absolute() && crate::runtime_status::executable_path_is_valid(path) {
                return runtime_direct_runner(runtime, Some(command), model, effort);
            }
            if !path.is_absolute() && command != definition.command {
                return runtime_direct_runner(runtime, Some(command), model, effort);
            }
        }
        let runner = runtime_direct_runner(runtime, None, model, effort)?;
        self.resolve_runner_executable(&runner, pool)
    }

    /// Gate a fresh `claude-code` spawn before calling
    /// `runtime.spawn()`. No-op for any other runtime — those
    /// bypass the gate.
    ///
    /// Fresh and fork spawn call sites invoke this. The ordinary resume
    /// path is intentionally unguarded:
    /// `claude --resume` / `--session-id` loads the local
    /// conversation file and puts up the TUI without touching the
    /// network until the user's next turn, so concurrent resumes
    /// can't race on the refresh-token rotation.
    ///
    /// Deadline-based: reads `last_spawn_at`, sleeps the remainder
    /// of `CLAUDE_LAUNCH_GATE_GRACE`, updates the timestamp, then
    /// releases the mutex. The first claude-code spawn through (or
    /// any spawn arriving after the grace window has elapsed) pays
    /// zero — so single direct chats and cold mission starts feel
    /// instant. Subsequent concurrent claudes serialize 1.5s apart,
    /// which is what prevents the OAuth refresh-token race.
    ///
    /// The mutex is held across the sleep so concurrent callers
    /// queue up correctly: B arrives mid-A-sleep → blocks on mutex
    /// → after A wakes and updates `last`, B observes A's
    /// just-recorded timestamp and waits its own full grace.
    pub(super) fn enter_claude_launch_gate(&self, session_id: &str, runtime: &str) {
        if runtime != "claude-code" {
            return;
        }
        let mut last = self
            .claude_launch_gate
            .lock()
            .expect("claude_launch_gate poisoned");
        let wait = compute_gate_wait(*last, Instant::now(), CLAUDE_LAUNCH_GATE_GRACE);
        if !wait.is_zero() {
            log::info!(
                "claude-code launch gate: session={session_id} sleep_ms={}",
                wait.as_millis()
            );
            thread::sleep(wait);
        }
        *last = Some(Instant::now());
    }

    fn seed_codex_project_trust(&self, session_id: &str, runtime: &str, cwd: Option<&Path>) {
        if runtime != "codex" {
            return;
        }
        let Some(cwd) = cwd else {
            log::debug!("skipping codex project trust seed without cwd: session={session_id}");
            return;
        };
        if let Err(e) = crate::session::codex_trust::seed_project_trust(cwd) {
            log::warn!(
                "failed to seed codex project trust: session={session_id} cwd={} error={e}",
                cwd.display()
            );
        }
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
        let shell_env = self
            .shell_env
            .read()
            .expect("runtime shell environment lock poisoned")
            .clone();
        // Bottom layer: login-shell vars (proxy quartet, both cases)
        // captured at app start. A runner row can override any of these
        // by setting the same name in its own env map — the runner row
        // is the most specific configuration surface.
        let mut env: BTreeMap<String, String> = shell_env.vars;
        for (k, v) in &runner.env {
            env.insert(k.clone(), v.clone());
        }
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
        let process_has_locale = LOCALE_VARS
            .iter()
            .any(|var| std::env::var_os(var).is_some());
        ensure_utf8_locale(&mut env, process_has_locale);
        SpawnSpec {
            session_id,
            cwd: cwd.map(PathBuf::from),
            command: runner.command.clone(),
            args: runner.args.clone(),
            env,
            mission,
            shim_dir,
            bundled_bin_dir,
            shell_path: shell_env.path,
            initial_size,
        }
    }

    /// Apply the runtime adapter's resume + trailing args to a
    /// `SpawnSpec`. Mirrors what the portable-pty `spawn` paths
    /// did inline; factored out so spawn / spawn_direct / resume
    /// can share the argv composition.
    ///
    /// `first_turn` is the composed first-user-turn body (mission
    /// launch prompt for a lead, worker preamble for non-leads,
    /// persona for direct chats). When the runtime accepts the
    /// positional `[PROMPT]` argv and the body fits in
    /// `FIRST_TURN_ARGV_MAX_BYTES`, the body lands as the trailing
    /// positional. Returns whether the body was delivered via argv
    /// so the caller can warn if a supported runtime somehow missed
    /// the deterministic path.
    pub(super) fn apply_runtime_args(
        spec: &mut SpawnSpec,
        runner: &Runner,
        plan: &router::runtime::ResumePlan,
        app_data_dir: &Path,
        first_turn: Option<&str>,
        mission_bus_dir: Option<&Path>,
    ) -> bool {
        if runner.runtime == "claude-code" {
            let _ = std::fs::remove_file(crate::session::claude_rekey::drop_path(
                app_data_dir,
                &spec.session_id,
            ));
        }
        let mut composed: Vec<String> = Vec::new();
        if plan.prepend {
            composed.extend(plan.args.iter().cloned());
            composed.append(&mut spec.args);
        } else {
            composed.append(&mut spec.args);
            composed.extend(plan.args.iter().cloned());
        }
        let first_turn_for_argv = router::runtime::first_turn_argv(&runner.runtime, first_turn);
        let delivered_via_argv = !first_turn_for_argv.is_empty();
        composed.extend(router::runtime::mission_bus_sandbox_args(
            &runner.runtime,
            mission_bus_dir,
        ));
        for extra in router::runtime::trailing_runtime_args(
            &runner.runtime,
            &runner.args,
            app_data_dir,
            &spec.session_id,
            plan.resuming,
            runner.model.as_deref(),
            runner.effort.as_deref(),
            runner.system_prompt.as_deref(),
            first_turn,
        ) {
            composed.push(extra);
        }
        spec.args = composed;
        delivered_via_argv
    }

    pub(super) fn codex_capture_prompt_marker(
        runtime: &str,
        session_id: &str,
        first_turn: Option<String>,
    ) -> (Option<String>, Option<String>) {
        if !matches!(runtime, "codex" | "trae") {
            return (first_turn, None);
        }
        let Some(first_turn) = first_turn else {
            return (None, None);
        };
        let marker = crate::session::codex_capture::prompt_marker(session_id);
        let marked_first_turn = format!("{first_turn}\n\n{marker}");
        if marked_first_turn.len() > router::runtime::FIRST_TURN_ARGV_MAX_BYTES {
            return (Some(first_turn), None);
        }
        (Some(marked_first_turn), Some(marker))
    }

    /// Sync part of a mission-slot spawn: validates inputs, composes
    /// the `SpawnSpec`, generates the session id, and INSERTs the
    /// `sessions` row. Returns a `PendingMissionSpawn` that
    /// `complete_mission_session_spawn` consumes (after the gate
    /// sleep) to actually bring the PTY up.
    ///
    /// Split out of the original monolithic `spawn` so
    /// `ops::mission::mission_start` can finish row inserts +
    /// router/bus mount synchronously and return to the GPUI task
    /// in ~milliseconds, then drive the slow PTY-spawn phase in a
    /// background task. Without the split, the modal Start button
    /// blocks ~1500ms per claude-code worker (gate cost) before the
    /// workspace loads. See issue #171.
    #[allow(clippy::too_many_arguments)]
    pub fn register_mission_session(
        self: &Arc<Self>,
        mission: &Mission,
        runner: &Runner,
        slot: &crate::model::Slot,
        app_data_dir: &Path,
        events_log_path: PathBuf,
        pool: Arc<DbPool>,
        first_turn: Option<String>,
        initial_size: Option<(u16, u16)>,
        size_source: &'static str,
    ) -> Result<PendingMissionSpawn> {
        let initial_size = Some(initial_size.unwrap_or(DEFAULT_PTY_SIZE));

        // Slot-level runtime override (feature 41): the effective
        // runtime is `slot.runtime_override ?? runner.runtime`. On a
        // differing override the spawn uses registry command, default
        // args, model, and effort; persona fields carry over. Slot
        // model/effort overrides apply last and do not themselves pin
        // the engine. A matching runtime override still pins.
        let resolution = resolve_runtime_override(
            runner,
            slot.runtime_override.as_deref(),
            slot.model_override.as_deref(),
            slot.effort_override.as_deref(),
        )?;
        let pinned = resolution.pinned;
        let agent_options_overridden = resolution.effective.is_some();
        let runner =
            self.resolve_runner_executable(resolution.effective.as_ref().unwrap_or(runner), &pool)?;

        // Agent-native session resume: this is a *fresh* session row, so
        // there's no prior key to inherit. The runtime adapter still
        // self-assigns a UUID for claude-code (`--session-id <uuid>`) so
        // a future `SessionManager::resume` can hand it back.
        let plan = router::runtime::resume_plan(&runner.runtime, None);

        // Working directory: mission cwd if set, else runner override, else
        // inherit parent's. The mission-level cwd is what the operator typed
        // into the Start-mission modal and the modal's helper text promises
        // it wins ("Each runner's PTY starts in this directory"). Capture the
        // resolved cwd so we can persist it on the session row — `resume`
        // reads it back to spawn the same dir on respawn, which matters for
        // claude-code (its conversation files are keyed under
        // `~/.claude/projects/<encoded-cwd>/<uuid>.jsonl`; resuming with a
        // different cwd makes `--resume` fail).
        let resolved_cwd: Option<String> =
            mission.cwd.clone().or_else(|| runner.working_dir.clone());

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
        let (first_turn, codex_prompt_marker) =
            Self::codex_capture_prompt_marker(&runner.runtime, &session_id, first_turn);
        let mut spec = self.base_spawn_spec(
            session_id.clone(),
            &runner,
            resolved_cwd.clone(),
            true,
            shim_dir,
            bundled_bin_dir,
            initial_size,
            mission_env,
        );
        let mission_bus_dir =
            runner_core::event_log::path::mission_dir(app_data_dir, &mission.crew_id, &mission.id);
        let first_turn_delivered_via_argv = Self::apply_runtime_args(
            &mut spec,
            &runner,
            &plan,
            app_data_dir,
            first_turn.as_deref(),
            Some(&mission_bus_dir),
        );

        // Insert the row first (status=running with no runtime_*
        // metadata yet) so a fast-failing runtime spawn doesn't leave
        // a half-row. We update with runtime metadata once the
        // runtime hands them back.
        let started_at_dt = Utc::now();
        let started_at = started_at_dt.to_rfc3339();
        {
            let conn = pool.get()?;
            let mut row = crate::repo::session::SessionRowDb::new_running(session_id.clone());
            row.mission_id = Some(mission.id.clone());
            row.project_id = mission.project_id.clone();
            row.runner_id = Some(runner.id.clone());
            row.slot_id = Some(slot.id.clone());
            row.cwd = resolved_cwd.clone();
            row.started_at = Some(started_at_dt);
            row.agent_session_key = plan.assigned_key.clone();
            row.last_cols = initial_size.map(|(cols, _)| cols);
            row.last_rows = initial_size.map(|(_, rows)| rows);
            if pinned {
                // Record the effective runtime so respawn/resume
                // keeps this session's engine even if the slot's
                // override — or the runner template's runtime — is
                // edited later. No-override rows stay NULL.
                row.agent_runtime = Some(runner.runtime.clone());
                row.agent_command = Some(runner.command.clone());
            }
            if agent_options_overridden {
                row.agent_model = runner.model.clone();
                row.agent_effort = runner.effort.clone();
            }
            crate::repo::session::insert(&conn, &row)?;
        }

        Ok(PendingMissionSpawn {
            session_id,
            spec,
            mission: mission.clone(),
            runner: runner.clone(),
            slot_handle: slot.slot_handle.clone(),
            size_source,
            plan,
            first_turn_delivered_via_argv,
            resolved_cwd,
            row_started_at: started_at,
            codex_prompt_marker,
            app_data_dir: app_data_dir.to_path_buf(),
            pool,
        })
    }

    /// Async/blocking part of a mission-slot spawn: takes the gate,
    /// forks the PTY, persists runtime metadata, installs the
    /// forwarder thread, schedules first-turn delivery. May block
    /// `CLAUDE_LAUNCH_GATE_GRACE` (1500ms) when other claude-code
    /// spawns are in flight.
    ///
    /// `cancel` is the per-mission abort flag from
    /// `register_pending_mission_cancel`. Checked twice: before the
    /// gate sleep (so a cancel that fires while the slot is still
    /// in the queue returns immediately) and after (so a cancel
    /// that fires during sleep still skips `runtime.spawn`). A
    /// cancelled spawn returns `Ok(CompleteSpawnOutcome::Cancelled)`
    /// — the caller marks the row stopped and continues. Pass a
    /// fresh `Arc::new(AtomicBool::new(false))` from the sync
    /// `SessionManager::spawn` wrapper where there's no batch to
    /// cancel against.
    ///
    /// Errors leave the session row in `running` status (with no
    /// `runtime_*` metadata) so the caller can decide whether to
    /// `DELETE` (legacy sync `spawn`) or mark `crashed` (async
    /// `mission_start` path).
    pub fn complete_mission_session_spawn(
        self: &Arc<Self>,
        pending: PendingMissionSpawn,
        events: Arc<dyn SessionEvents>,
        cancel: Arc<AtomicBool>,
    ) -> Result<CompleteSpawnOutcome> {
        let PendingMissionSpawn {
            session_id,
            mut spec,
            mission,
            runner,
            slot_handle,
            mut size_source,
            plan,
            first_turn_delivered_via_argv,
            resolved_cwd,
            row_started_at,
            codex_prompt_marker,
            app_data_dir,
            pool,
        } = pending;

        // Pre-gate cancellation: a user who clicked Stop/Archive
        // while this slot was sitting in the spawn queue gets the
        // expected behavior — the queued slot never forks. Without
        // this, the slot would sleep through the gate and spawn into
        // a stopped mission.
        if cancel.load(Ordering::Acquire) {
            log::info!(
                "mission session spawn cancelled pre-gate: mission={} session={} runner={}",
                mission.id,
                session_id,
                slot_handle,
            );
            return Ok(CompleteSpawnOutcome::Cancelled);
        }

        // Gate claude-code spawns so N parallel mission slots don't
        // race the OAuth refresh-token rotation. No-op for other
        // runtimes; zero-wait for the first claude through. See
        // `enter_claude_launch_gate` + issue #171.
        self.enter_claude_launch_gate(&session_id, &runner.runtime);

        // Post-gate cancellation: covers a Stop that fires while we
        // were asleep in the gate. The wake-up still races with the
        // cancel — flagging it here means we observe it before the
        // expensive `runtime.spawn`. Also covers the case where
        // `runner_delete` cascade-removed the row through the FK on
        // `sessions.runner_id` — surfaces the same way (we have no
        // row to attach a PTY to).
        if cancel.load(Ordering::Acquire) || !Self::session_row_exists(&pool, &session_id) {
            log::info!(
                "mission session spawn cancelled post-gate: mission={} session={} runner={}",
                mission.id,
                session_id,
                slot_handle,
            );
            return Ok(CompleteSpawnOutcome::Cancelled);
        }

        let spawn_started_at_dt = Utc::now();
        // The row has been visible since registration. A measurement can
        // arrive before its trailing persistence settle, so manager memory
        // wins over the row and the registration hint.
        let latest_size = self
            .latest_requested_size(&session_id)
            .or_else(|| Self::persisted_size(&pool, &session_id));
        if let Some(latest_size) = latest_size {
            if spec.initial_size != Some(latest_size) {
                spec.initial_size = Some(latest_size);
                size_source = "latest-measured-size";
            }
        }
        let initial_size = spec.initial_size;
        self.seed_codex_project_trust(&session_id, &runner.runtime, spec.cwd.as_deref());
        let (rt_session, output) = self
            .runtime
            .spawn(spec)
            .map_err(|e| Error::msg(format!("spawn {}: {e}", runner.command)))?;

        // Post-spawn cancellation. Two triggers reach this branch:
        //   1. A `Stop`/`Archive` that fired while the
        //      runtime was mid-fork — `kill_all_for_mission` can't
        //      see the PTY yet (no `SessionHandle` in `sessions`
        //      until the insert below). Flagged by `cancel`.
        //   2. A `runner_delete` whose FK cascade dropped the row
        //      while runtime was mid-fork. Flagged by the row check.
        // Either way the PTY exists with no DB anchor; tear it down
        // before any further bookkeeping. The dropped output stream
        // triggers EOF in the reader thread and reaps the child.
        if cancel.load(Ordering::Acquire) || !Self::session_row_exists(&pool, &session_id) {
            log::info!(
                "mission session spawn cancelled post-runtime-spawn: \
                 mission={} session={} runner={}",
                mission.id,
                session_id,
                slot_handle,
            );
            if let Err(e) = self.runtime.stop(&rt_session) {
                log::warn!(
                    "failed to stop just-spawned PTY for cancelled session {session_id}: {e}"
                );
            }
            return Ok(CompleteSpawnOutcome::Cancelled);
        }

        // The PTY exists and survived both cancellation windows — this
        // reports a fork that actually happened, not an attempt.
        if let Some((cols, rows)) = initial_size {
            log::info!(
                "mission slot fork: session={session_id} runtime={} \
                 size={cols}x{rows} source={size_source}",
                runner.runtime,
            );
        }

        let spawn_pid = self.runtime_pid(&rt_session);

        // Persist the runtime-side identity for diagnostics and for
        // the current runtime session row.
        if let Ok(conn) = pool.get() {
            let _ = crate::repo::session::update_runtime_metadata(
                &conn,
                &session_id,
                &rt_session.runtime,
                &rt_session.session_id,
                spawn_pid,
            );
        }

        let codex_capture =
            if matches!(runner.runtime.as_str(), "codex" | "trae") && plan.assigned_key.is_none() {
                crate::session::codex_capture::sessions_root_for(&runner.runtime).and_then(
                    |sessions_root| {
                        capture_cwd(resolved_cwd.clone()).map(|cwd| CodexCaptureContext {
                            mission_id: Some(mission.id.clone()),
                            sessions_root,
                            spawn_cwd: cwd,
                            started_at: spawn_started_at_dt,
                            row_started_at: row_started_at.clone(),
                            spawn_pid,
                            prompt_marker: codex_prompt_marker.clone(),
                            pool: Arc::clone(&pool),
                            events: Arc::clone(&events),
                        })
                    },
                )
            } else {
                None
            };

        let spawn_emit_ctx = open_mission_event_log(&app_data_dir, &mission.crew_id, &mission.id)
            .map(|event_log| ForwarderEmitCtx {
                crew_id: mission.crew_id.clone(),
                mission_id: mission.id.clone(),
                handle: slot_handle.clone(),
                event_log,
            });
        let stop = output.stop_flag();
        let runtime_session_for_log = rt_session.session_id.clone();
        self.install_handle(
            &session_id,
            SessionHandle {
                id: session_id.clone(),
                mission_id: Some(mission.id.clone()),
                runner_id: Some(runner.id.clone()),
                runtime_session: rt_session.clone(),
                codex_capture: codex_capture.clone(),
                forwarder: None,
                stop,
            },
            spawn_emit_ctx.clone(),
            initial_size,
            &pool,
            events.as_ref(),
        );
        if first_turn_delivered_via_argv {
            self.arm_completion(&session_id);
        }

        let forwarder = self.start_forwarder_thread(
            session_id.clone(),
            Some(mission.id.clone()),
            rt_session,
            output,
            Arc::clone(&pool),
            Arc::clone(&events),
            runner.clone(),
            plan.resuming,
            true,
            spawn_emit_ctx,
        );
        self.install_forwarder(&session_id, forwarder);

        if let Some(ctx) = codex_capture.as_ref() {
            self.spawn_codex_capture_if_unkeyed(&session_id, ctx);
        }

        emit_runner_activity(&pool, &runner, events.as_ref());
        if matches!(runner.runtime.as_str(), "claude-code" | "codex" | "trae")
            && !plan.resuming
            && !first_turn_delivered_via_argv
        {
            log::warn!(
                "first-turn argv not delivered for {session_id} (runtime {}); skipping post-spawn injection",
                runner.runtime,
            );
        }

        log::info!(
            "session spawn: mission={} session={} runner={} runtime_session={}",
            mission.id,
            session_id,
            slot_handle,
            runtime_session_for_log,
        );

        Ok(CompleteSpawnOutcome::Spawned)
    }

    /// Spawn one PTY child for `runner` as part of `mission`. Persists a
    /// `sessions` row, starts the reader thread, and returns a summary for
    /// the frontend.
    ///
    /// `app_data_dir` is the root of `$APPDATA/runner/` so we can prepend
    /// `<app_data_dir>/bin` onto the child's PATH — arch §5.3 Layer 2 and
    /// 0001-v0-mvp.md C9 both require the bundled `runner` CLI to win over any
    /// system binary with the same name.
    /// `first_turn` is the composed first-user-turn body to deliver
    /// at spawn (lead launch prompt for a lead slot, worker preamble
    /// plus brief for a non-lead). When the runtime accepts the
    /// positional `[PROMPT]` argv and the body fits
    /// `FIRST_TURN_ARGV_MAX_BYTES`, it lands as the trailing
    /// positional during process init. Pass `None` to skip
    /// first-turn delivery entirely, for tests that don't care about
    /// boot context.
    ///
    /// Synchronous, all-or-nothing wrapper: row insert + PTY spawn +
    /// reader thread happen on the calling thread. Rolls back the
    /// row if the runtime spawn fails. Used by tests and by the
    /// resume / direct-chat paths where the caller awaits a fully
    /// initialized session. Mission start uses the split form
    /// (`register_mission_session` + `complete_mission_session_spawn`)
    /// to keep the Start-mission RPC snappy.
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
        first_turn: Option<String>,
    ) -> Result<SpawnedSession> {
        let pending = self.register_mission_session(
            mission,
            runner,
            slot,
            app_data_dir,
            events_log_path,
            Arc::clone(&pool),
            first_turn,
            None,
            "DEFAULT_PTY_SIZE",
        )?;
        let session_id = pending.session_id.clone();
        let mission_id = pending.mission.id.clone();
        let runner_id = pending.runner.id.clone();
        let handle = pending.runner.handle.clone();
        // No batch context in the sync wrapper, so cancellation is
        // never set externally — pass a fresh flag so the cancel
        // checks are a no-op for this path.
        let noop_cancel = Arc::new(AtomicBool::new(false));
        if let Err(e) = self.complete_mission_session_spawn(pending, events, noop_cancel) {
            // Match the historical sync-spawn contract: if the runtime
            // can't bring the PTY up, delete the half-row so retries
            // start from a clean slate. The async mission_start path
            // takes a softer line and marks the row crashed instead.
            if let Ok(conn) = pool.get() {
                let _ = crate::repo::session::delete(&conn, &session_id);
            }
            return Err(e);
        }
        Ok(SpawnedSession {
            id: session_id,
            mission_id: Some(mission_id),
            runner_id: Some(runner_id),
            handle,
            // PTY child pid is populated lazily via runtime.status()
            // when the manager needs it; the SpawnedSession field is
            // informational and the frontend doesn't rely on it.
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
    ///
    /// `first_turn` is the composed persona body for the direct chat
    /// (no preamble — direct chats are off-bus). When the runtime
    /// supports argv-based delivery the persona lands as the
    /// trailing positional at spawn. Pass `None` when there's no
    /// persona to deliver, or for tests that don't care about boot
    /// context.
    /// `runtime_override` is the chat-level engine choice (feature 41):
    /// `None` spawns the runner's own runtime unchanged; a differing
    /// registry runtime spawns that engine with registry command /
    /// default args while the runner's persona fields carry over.
    /// `model_override` / `effort_override` apply after runtime
    /// resolution and can also customize the runner's own engine.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_direct(
        self: &Arc<Self>,
        runner: &Runner,
        runtime_override: Option<&str>,
        model_override: Option<&str>,
        effort_override: Option<&str>,
        project_id: Option<&str>,
        cwd: Option<&str>,
        cols: Option<u16>,
        rows: Option<u16>,
        app_data_dir: &Path,
        pool: Arc<DbPool>,
        events: Arc<dyn SessionEvents>,
        first_turn: Option<String>,
    ) -> Result<SpawnedSession> {
        self.spawn_direct_inner(
            runner,
            runtime_override,
            model_override,
            effort_override,
            Some(runner.id.as_str()),
            project_id,
            cwd,
            cols,
            rows,
            app_data_dir,
            pool,
            events,
            first_turn,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_runtime_direct(
        self: &Arc<Self>,
        runner: &Runner,
        project_id: Option<&str>,
        cwd: Option<&str>,
        cols: Option<u16>,
        rows: Option<u16>,
        app_data_dir: &Path,
        pool: Arc<DbPool>,
        events: Arc<dyn SessionEvents>,
    ) -> Result<SpawnedSession> {
        self.spawn_direct_inner(
            runner,
            None,
            None,
            None,
            None,
            project_id,
            cwd,
            cols,
            rows,
            app_data_dir,
            pool,
            events,
            None,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_direct_inner(
        self: &Arc<Self>,
        runner: &Runner,
        runtime_override: Option<&str>,
        model_override: Option<&str>,
        effort_override: Option<&str>,
        persisted_runner_id: Option<&str>,
        project_id: Option<&str>,
        cwd: Option<&str>,
        cols: Option<u16>,
        rows: Option<u16>,
        app_data_dir: &Path,
        pool: Arc<DbPool>,
        events: Arc<dyn SessionEvents>,
        first_turn: Option<String>,
        emit_activity: bool,
    ) -> Result<SpawnedSession> {
        // Chat-level runtime override (feature 41) — same resolution
        // rule as mission spawns.
        let resolution =
            resolve_runtime_override(runner, runtime_override, model_override, effort_override)?;
        let pinned = resolution.pinned;
        let agent_options_overridden = resolution.effective.is_some();
        let runner =
            self.resolve_runner_executable(resolution.effective.as_ref().unwrap_or(runner), &pool)?;

        // Agent-native session resume: `spawn_direct` always opens a *new*
        // chat. The runtime adapter self-assigns a fresh
        // `agent_session_key` (claude-code) or leaves it NULL (codex).
        let plan = router::runtime::resume_plan(&runner.runtime, None);

        // Working directory precedence: explicit `cwd` arg (Chat now
        // dialog folder) ► runner's `working_dir`. A direct chat must
        // name a real directory: portable-pty silently substitutes
        // $HOME for a missing or nonexistent cwd, which strands the
        // agent in the wrong place and breaks codex session-key
        // capture (the rollout cwd can never match the row).
        let resolved_cwd: Option<String> = cwd
            .map(|s| s.to_string())
            .or_else(|| runner.working_dir.clone());
        let Some(chat_cwd) = resolved_cwd.as_deref().filter(|c| !c.is_empty()) else {
            return Err(Error::msg(
                "select a working directory before starting a chat",
            ));
        };
        if !std::path::Path::new(chat_cwd).is_dir() {
            return Err(Error::msg(format!(
                "working directory does not exist: {chat_cwd}"
            )));
        }

        // Direct chats are off-bus: RUNNER_HANDLE is the runner template's
        // own handle, no slot/mission env vars.
        let mut direct_env: BTreeMap<String, String> = BTreeMap::new();
        if runner.runtime != "shell" {
            direct_env.insert("RUNNER_HANDLE".into(), runner.handle.clone());
        }

        let initial_size = Some(cols.zip(rows).unwrap_or(DEFAULT_PTY_SIZE));

        let session_id = ulid::Ulid::new().to_string();
        let (first_turn, codex_prompt_marker) =
            Self::codex_capture_prompt_marker(&runner.runtime, &session_id, first_turn);
        let started_at_dt = Utc::now();
        let started_at = started_at_dt.to_rfc3339();

        let mut spec = self.base_spawn_spec(
            session_id.clone(),
            &runner,
            resolved_cwd.clone(),
            false,
            None, // shim_dir — off-bus
            None, // bundled_bin_dir — off-bus
            initial_size,
            direct_env,
        );
        let first_turn_delivered_via_argv = Self::apply_runtime_args(
            &mut spec,
            &runner,
            &plan,
            app_data_dir,
            first_turn.as_deref(),
            None,
        );

        // Insert the row first so a fast-failing spawn doesn't leave
        // a half-row. Runtime-only chats (no persisted runner template)
        // carry their agent identity on the row via agent_runtime /
        // agent_command; runner-backed chats leave those NULL unless a
        // runtime override was explicitly requested — then the row
        // records the effective runtime so resume respawns the same
        // engine even if the runner template is edited later.
        {
            let conn = pool.get()?;
            let mut row = crate::repo::session::SessionRowDb::new_running(session_id.clone());
            row.project_id = project_id.map(str::to_string);
            row.runner_id = persisted_runner_id.map(str::to_string);
            row.cwd = resolved_cwd.clone();
            row.started_at = Some(started_at_dt);
            row.agent_session_key = plan.assigned_key.clone();
            row.last_cols = initial_size.map(|(cols, _)| cols);
            row.last_rows = initial_size.map(|(_, rows)| rows);
            if persisted_runner_id.is_none() || pinned {
                row.agent_runtime = Some(runner.runtime.clone());
                row.agent_command = Some(runner.command.clone());
            }
            if persisted_runner_id.is_none() || agent_options_overridden {
                row.agent_model = runner.model.clone();
                row.agent_effort = runner.effort.clone();
            }
            crate::repo::session::insert(&conn, &row)?;
        }

        // Same gate as the mission spawn path — direct chats are
        // also fresh claude-code spawns and proactively refresh the
        // OAuth token, so a rapid burst of new chats can race. See
        // `enter_claude_launch_gate` + issue #171.
        self.enter_claude_launch_gate(&session_id, &runner.runtime);

        // Post-gate row check: `runner_delete` can cascade through
        // `sessions.runner_id` while we were asleep in the gate. The
        // session row is gone; spawning a PTY now would attach to
        // nothing.
        if !Self::session_row_exists(&pool, &session_id) {
            return Err(Error::msg(format!(
                "direct-chat session {session_id} row vanished before spawn — runner deleted?"
            )));
        }

        let spawn_started_at_dt = Utc::now();
        self.seed_codex_project_trust(&session_id, &runner.runtime, spec.cwd.as_deref());
        let (rt_session, output) = match self.runtime.spawn(spec) {
            Ok(p) => p,
            Err(e) => {
                if let Ok(conn) = pool.get() {
                    let _ = crate::repo::session::delete(&conn, &session_id);
                }
                return Err(Error::msg(format!("spawn {}: {e}", runner.command)));
            }
        };

        // Post-spawn row check: `runner_delete` can also fire while
        // `runtime.spawn` was mid-fork. The PTY is alive; tear it
        // down before we install a `SessionHandle` that points at a
        // row that no longer exists.
        if !Self::session_row_exists(&pool, &session_id) {
            if let Err(e) = self.runtime.stop(&rt_session) {
                log::warn!(
                    "failed to stop just-spawned direct-chat PTY for vanished session \
                     {session_id}: {e}"
                );
            }
            return Err(Error::msg(format!(
                "direct-chat session {session_id} row vanished mid-spawn — runner deleted?"
            )));
        }

        let spawn_pid = self.runtime_pid(&rt_session);

        if let Ok(conn) = pool.get() {
            let _ = crate::repo::session::update_runtime_metadata(
                &conn,
                &session_id,
                &rt_session.runtime,
                &rt_session.session_id,
                spawn_pid,
            );
        }

        let codex_capture =
            if matches!(runner.runtime.as_str(), "codex" | "trae") && plan.assigned_key.is_none() {
                crate::session::codex_capture::sessions_root_for(&runner.runtime).and_then(
                    |sessions_root| {
                        capture_cwd(resolved_cwd.clone()).map(|cwd| CodexCaptureContext {
                            mission_id: None,
                            sessions_root,
                            spawn_cwd: cwd,
                            started_at: spawn_started_at_dt,
                            row_started_at: started_at.clone(),
                            spawn_pid,
                            prompt_marker: codex_prompt_marker.clone(),
                            pool: Arc::clone(&pool),
                            events: Arc::clone(&events),
                        })
                    },
                )
            } else {
                None
            };

        self.install_handle(
            &session_id,
            SessionHandle {
                id: session_id.clone(),
                mission_id: None,
                runner_id: persisted_runner_id.map(str::to_string),
                runtime_session: rt_session.clone(),
                codex_capture: codex_capture.clone(),
                forwarder: None,
                stop: output.stop_flag(),
            },
            None,
            initial_size,
            &pool,
            events.as_ref(),
        );
        if first_turn_delivered_via_argv {
            self.arm_completion(&session_id);
        }
        self.publish_direct_activity(
            &session_id,
            SessionActivityState::Busy,
            "spawn",
            events.as_ref(),
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
            emit_activity,
            None, // direct chats are off-bus — no log to append runner_status to
        );
        self.install_forwarder(&session_id, forwarder);

        if let Some(ctx) = codex_capture.as_ref() {
            self.spawn_codex_capture_if_unkeyed(&session_id, ctx);
        }

        if emit_activity {
            emit_runner_activity(&pool, &runner, events.as_ref());
        }
        if matches!(runner.runtime.as_str(), "claude-code" | "codex" | "trae")
            && !plan.resuming
            && !first_turn_delivered_via_argv
        {
            log::warn!(
                "first-turn argv not delivered for direct chat {session_id} (runtime {}); skipping post-spawn injection",
                runner.runtime,
            );
        }

        Ok(SpawnedSession {
            id: session_id,
            mission_id: None,
            runner_id: persisted_runner_id.map(str::to_string),
            handle: runner.handle.clone(),
            pid: None,
            fresh_fallback_lead: false,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_fork(
        self: &Arc<Self>,
        source_session_id: &str,
        title: Option<String>,
        cols: Option<u16>,
        rows: Option<u16>,
        app_data_dir: &Path,
        pool: Arc<DbPool>,
        events: Arc<dyn SessionEvents>,
    ) -> Result<SpawnedSession> {
        let fork_started_at = Instant::now();
        let source = {
            let conn = pool.get()?;
            crate::repo::session::get_row(&conn, source_session_id)?
                .ok_or_else(|| Error::msg(format!("session not found: {source_session_id}")))?
        };
        if source.mission_id.is_some() {
            return Err(Error::msg("only direct chats can be forked"));
        }
        if source.archived_at.is_some() {
            return Err(Error::msg(format!(
                "session {source_session_id} is archived — un-archive before forking"
            )));
        }
        let source_key = source.agent_session_key.as_deref().ok_or_else(|| {
            Error::msg(format!(
                "session {source_session_id} has no captured agent session key"
            ))
        })?;

        let runner_template = if let Some(runner_id) = source.runner_id.as_deref() {
            let conn = pool.get()?;
            Some(crate::ops::runner::get(&conn, runner_id)?)
        } else {
            None
        };
        let effective_runtime = source
            .agent_runtime
            .clone()
            .or_else(|| {
                runner_template
                    .as_ref()
                    .map(|runner| runner.runtime.clone())
            })
            .ok_or_else(|| {
                Error::msg(format!(
                    "runtime-only session {source_session_id} missing agent_runtime"
                ))
            })?;
        if !router::runtime::supports_native_fork(&effective_runtime) {
            return Err(Error::msg(format!(
                "runtime {effective_runtime} does not support native fork"
            )));
        }

        let runner = if let Some(runner) = runner_template {
            let runner = match resolve_runtime_override(
                &runner,
                source.agent_runtime.as_deref(),
                source.agent_model.as_deref(),
                source.agent_effort.as_deref(),
            )?
            .effective
            {
                Some(effective) => effective,
                None => runner,
            };
            self.resolve_runner_executable(&runner, &pool)?
        } else {
            self.resolve_runtime_only_resume_runner(
                &effective_runtime,
                source.agent_command.as_deref(),
                source.agent_model.as_deref(),
                source.agent_effort.as_deref(),
                &pool,
            )?
        };

        let resolved_cwd = source.cwd.clone().or_else(|| runner.working_dir.clone());
        let Some(chat_cwd) = resolved_cwd.as_deref().filter(|cwd| !cwd.is_empty()) else {
            return Err(Error::msg(
                "select a working directory before forking a chat",
            ));
        };
        if !Path::new(chat_cwd).is_dir() {
            return Err(Error::msg(format!(
                "working directory does not exist: {chat_cwd}"
            )));
        }
        let source_label = source
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                if source.runner_id.is_some() {
                    format!("@{}", runner.handle)
                } else {
                    runner.display_name.clone()
                }
            });
        let plan = router::runtime::fork_plan(&runner.runtime, source_key, &source_label)
            .ok_or_else(|| {
                Error::msg(format!(
                    "could not build fork plan for runtime {}",
                    runner.runtime
                ))
            })?;

        let mut direct_env = BTreeMap::new();
        if runner.runtime != "shell" {
            direct_env.insert("RUNNER_HANDLE".into(), runner.handle.clone());
        }
        let initial_size = Some(cols.zip(rows).unwrap_or(DEFAULT_PTY_SIZE));
        let session_id = ulid::Ulid::new().to_string();
        let started_at_dt = Utc::now();
        let started_at = started_at_dt.to_rfc3339();
        let mut spec = self.base_spawn_spec(
            session_id.clone(),
            &runner,
            resolved_cwd.clone(),
            false,
            None,
            None,
            initial_size,
            direct_env,
        );

        {
            let conn = pool.get()?;
            let mut row = crate::repo::session::SessionRowDb::new_running(session_id.clone());
            row.project_id.clone_from(&source.project_id);
            row.runner_id.clone_from(&source.runner_id);
            row.cwd.clone_from(&source.cwd);
            row.started_at = Some(started_at_dt);
            row.agent_session_key = match &plan {
                router::runtime::ForkPlan::Direct(plan) => plan.assigned_key.clone(),
                router::runtime::ForkPlan::Headless { .. } => None,
            };
            row.title = title.and_then(|title| {
                let title = title.trim();
                (!title.is_empty()).then(|| title.to_string())
            });
            row.agent_runtime.clone_from(&source.agent_runtime);
            row.agent_command.clone_from(&source.agent_command);
            row.agent_model.clone_from(&source.agent_model);
            row.agent_effort.clone_from(&source.agent_effort);
            row.last_cols = initial_size.map(|(cols, _)| cols);
            row.last_rows = initial_size.map(|(_, rows)| rows);
            crate::repo::session::insert(&conn, &row)?;
        }
        events.updated(&SessionUpdatedEvent {
            session_id: session_id.clone(),
            mission_id: None,
        });
        events.fork_started(&super::SessionForkStartedEvent {
            source_session_id: source_session_id.to_owned(),
            session_id: session_id.clone(),
        });

        // Claude forks are direct TUI spawns, so they use the same launch gate
        // as a fresh chat. This is a no-op for Codex; its visible phase-2
        // process is an ordinary resume and remains deliberately ungated.
        let gate_started_at = Instant::now();
        self.enter_claude_launch_gate(&session_id, &runner.runtime);
        let gate_elapsed = gate_started_at.elapsed();
        if !Self::session_row_exists(&pool, &session_id) {
            return Err(Error::msg(format!(
                "forked session {session_id} row vanished before spawn — runner deleted?"
            )));
        }

        match plan {
            router::runtime::ForkPlan::Direct(plan) => {
                let _ =
                    Self::apply_runtime_args(&mut spec, &runner, &plan, app_data_dir, None, None);
                let direct_spawn_started_at = Instant::now();
                let (rt_session, output) = match self.runtime.spawn(spec) {
                    Ok(spawned) => spawned,
                    Err(error) => {
                        log::info!(
                            "fork timing: session={session_id} runtime={} gate_ms={} materialize_ms=not-applicable direct_spawn_ms={} total_ms={} outcome=spawn-error",
                            runner.runtime,
                            gate_elapsed.as_millis(),
                            direct_spawn_started_at.elapsed().as_millis(),
                            fork_started_at.elapsed().as_millis(),
                        );
                        if let Err(cleanup_error) = delete_failed_fork(&pool, &session_id) {
                            return Err(Error::msg(format!(
                                "spawn {}: {error}; failed to roll back forked session {session_id}: {cleanup_error}",
                                runner.command,
                            )));
                        }
                        events.updated(&SessionUpdatedEvent {
                            session_id: session_id.clone(),
                            mission_id: None,
                        });
                        return Err(Error::msg(format!("spawn {}: {error}", runner.command)));
                    }
                };

                if !Self::session_row_exists(&pool, &session_id) {
                    if let Err(error) = self.runtime.stop(&rt_session) {
                        log::warn!(
                            "failed to stop just-spawned fork PTY for vanished session {session_id}: {error}"
                        );
                    }
                    let _ = delete_failed_fork(&pool, &session_id);
                    events.updated(&SessionUpdatedEvent {
                        session_id: session_id.clone(),
                        mission_id: None,
                    });
                    return Err(Error::msg(format!(
                        "forked session {session_id} row vanished mid-spawn — runner deleted?"
                    )));
                }

                let spawn_pid = self.runtime_pid(&rt_session);
                if let Ok(conn) = pool.get() {
                    let _ = crate::repo::session::update_runtime_metadata(
                        &conn,
                        &session_id,
                        &rt_session.runtime,
                        &rt_session.session_id,
                        spawn_pid,
                    );
                }

                self.install_handle(
                    &session_id,
                    SessionHandle {
                        id: session_id.clone(),
                        mission_id: None,
                        runner_id: source.runner_id.clone(),
                        runtime_session: rt_session.clone(),
                        codex_capture: None,
                        forwarder: None,
                        stop: output.stop_flag(),
                    },
                    None,
                    initial_size,
                    &pool,
                    events.as_ref(),
                );
                self.publish_direct_activity(
                    &session_id,
                    SessionActivityState::Busy,
                    "fork",
                    events.as_ref(),
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
                    source.runner_id.is_some(),
                    None,
                );
                self.install_forwarder(&session_id, forwarder);
                if source.runner_id.is_some() {
                    emit_runner_activity(&pool, &runner, events.as_ref());
                }
                log::info!(
                    "fork timing: session={session_id} runtime={} gate_ms={} materialize_ms=not-applicable direct_spawn_ms={} total_ms={} outcome=ok",
                    runner.runtime,
                    gate_elapsed.as_millis(),
                    direct_spawn_started_at.elapsed().as_millis(),
                    fork_started_at.elapsed().as_millis(),
                );
                Ok(SpawnedSession {
                    id: session_id,
                    mission_id: None,
                    runner_id: source.runner_id,
                    handle: runner.handle,
                    pid: None,
                    fresh_fallback_lead: false,
                })
            }
            headless @ router::runtime::ForkPlan::Headless { .. } => {
                self.seed_codex_project_trust(&session_id, &runner.runtime, spec.cwd.as_deref());
                let materialize_started_at = Instant::now();
                let fork_key = match run_headless_fork(&spec, &headless, FORK_MATERIALIZE_TIMEOUT) {
                    Ok(key) => key,
                    Err(error) => {
                        log::info!(
                            "fork timing: session={session_id} runtime={} gate_ms={} materialize_ms={} total_ms={} outcome=materialize-error",
                            runner.runtime,
                            gate_elapsed.as_millis(),
                            materialize_started_at.elapsed().as_millis(),
                            fork_started_at.elapsed().as_millis(),
                        );
                        if let Err(cleanup_error) = delete_failed_fork(&pool, &session_id) {
                            return Err(Error::msg(format!(
                                "{error}; failed to roll back forked session {session_id}: {cleanup_error}"
                            )));
                        }
                        events.updated(&SessionUpdatedEvent {
                            session_id: session_id.clone(),
                            mission_id: None,
                        });
                        return Err(error);
                    }
                };
                let materialize_elapsed = materialize_started_at.elapsed();

                let persist_result = (|| -> Result<bool> {
                    let conn = pool.get()?;
                    let captured = crate::repo::session::capture_agent_session_key(
                        &conn,
                        &session_id,
                        &fork_key,
                        &started_at,
                    )?;
                    let stopped = crate::repo::session::set_exit_status(
                        &conn,
                        &session_id,
                        crate::model::SessionStatus::Stopped,
                        Utc::now(),
                    )?;
                    Ok(captured && stopped > 0)
                })();
                match persist_result {
                    Ok(true) => {}
                    Ok(false) => {
                        delete_failed_fork(&pool, &session_id)?;
                        events.updated(&SessionUpdatedEvent {
                            session_id: session_id.clone(),
                            mission_id: None,
                        });
                        return Err(Error::msg(format!(
                            "forked session {session_id} row vanished during materialization"
                        )));
                    }
                    Err(error) => {
                        if let Err(cleanup_error) = delete_failed_fork(&pool, &session_id) {
                            return Err(Error::msg(format!(
                                "{error}; failed to roll back forked session {session_id}: {cleanup_error}"
                            )));
                        }
                        events.updated(&SessionUpdatedEvent {
                            session_id: session_id.clone(),
                            mission_id: None,
                        });
                        return Err(error);
                    }
                }

                let resume_started_at = Instant::now();
                let resume_result = self.resume_on_launch(
                    &session_id,
                    cols,
                    rows,
                    app_data_dir,
                    Arc::clone(&pool),
                    Arc::clone(&events),
                );
                log::info!(
                    "fork timing: session={session_id} runtime={} gate_ms={} materialize_ms={} resume_spawn_ms={} total_ms={} outcome={}",
                    runner.runtime,
                    gate_elapsed.as_millis(),
                    materialize_elapsed.as_millis(),
                    resume_started_at.elapsed().as_millis(),
                    fork_started_at.elapsed().as_millis(),
                    if resume_result.is_ok() { "ok" } else { "resume-error" },
                );
                match resume_result {
                    Ok(spawned) => Ok(spawned),
                    Err(error) => {
                        if let Err(cleanup_error) = delete_failed_fork(&pool, &session_id) {
                            return Err(Error::msg(format!(
                                "{error}; failed to roll back forked session {session_id}: {cleanup_error}"
                            )));
                        }
                        events.updated(&SessionUpdatedEvent {
                            session_id: session_id.clone(),
                            mission_id: None,
                        });
                        Err(error)
                    }
                }
            }
        }
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
        self.resume_with_fresh_fallback(session_id, cols, rows, app_data_dir, pool, events, true)
    }

    /// Launch-time resume shares the normal resume path but refuses the
    /// manual flow's fresh fallback when the prior conversation is gone.
    #[allow(clippy::too_many_arguments)]
    pub fn resume_on_launch(
        self: &Arc<Self>,
        session_id: &str,
        cols: Option<u16>,
        rows: Option<u16>,
        app_data_dir: &Path,
        pool: Arc<DbPool>,
        events: Arc<dyn SessionEvents>,
    ) -> Result<SpawnedSession> {
        self.resume_with_fresh_fallback(session_id, cols, rows, app_data_dir, pool, events, false)
    }

    #[allow(clippy::too_many_arguments)]
    fn resume_with_fresh_fallback(
        self: &Arc<Self>,
        session_id: &str,
        cols: Option<u16>,
        rows: Option<u16>,
        app_data_dir: &Path,
        pool: Arc<DbPool>,
        events: Arc<dyn SessionEvents>,
        allow_fresh_fallback: bool,
    ) -> Result<SpawnedSession> {
        // Atomically claim this session id for the resume. If another
        // resume is already in flight (e.g. two fast clicks, two
        // windows), refuse rather than racing two PTY spawns against
        // the same row. The claim guard releases on every exit path
        // via Drop.
        let _claim = {
            let state = self.session_state_or_insert(session_id);
            let mut state = state.lock().unwrap();
            if state.resuming {
                return Err(Error::msg(format!(
                    "session {session_id} is already being resumed"
                )));
            }
            state.resuming = true;
            ResumeClaim {
                mgr: Arc::clone(self),
                session_id: session_id.to_string(),
            }
        };

        // Validate the row + collect everything we need under a single
        // short-lived connection. We deliberately don't hold the conn
        // across the spawn (which itself grabs a pool slot for the
        // status update).
        let snap = {
            let conn = pool.get()?;
            let row = crate::repo::session::get_row(&conn, session_id)?
                .ok_or_else(|| Error::msg(format!("session not found: {session_id}")))?;
            if matches!(row.status, crate::model::SessionStatus::Running) {
                return Err(Error::msg(format!(
                    "session {session_id} is already running — attach instead"
                )));
            }
            if row.archived_at.is_some() {
                return Err(Error::msg(format!(
                    "session {session_id} is archived — un-archive before resuming"
                )));
            }
            row
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
                    let mission = crate::ops::mission::get(&conn, mid)?;
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

        // Pull the runner config fresh for runner-backed rows; rebuild
        // the default runtime config for runtime-only direct chats.
        // Runner-backed rows that recorded an effective runtime at
        // spawn (runtime override, feature 41) re-apply it here so the
        // respawn keeps this session's engine regardless of later
        // slot/override edits.
        let runner = if let Some(runner_id) = snap.runner_id.as_deref() {
            let conn = pool.get()?;
            let runner = crate::ops::runner::get(&conn, runner_id)?;
            let runner = match resolve_runtime_override(
                &runner,
                snap.agent_runtime.as_deref(),
                snap.agent_model.as_deref(),
                snap.agent_effort.as_deref(),
            )?
            .effective
            {
                Some(effective) => effective,
                None => runner,
            };
            self.resolve_runner_executable(&runner, &pool)?
        } else {
            let runtime = snap.agent_runtime.as_deref().ok_or_else(|| {
                Error::msg(format!(
                    "runtime-only session {session_id} missing agent_runtime"
                ))
            })?;
            self.resolve_runtime_only_resume_runner(
                runtime,
                snap.agent_command.as_deref(),
                snap.agent_model.as_deref(),
                snap.agent_effort.as_deref(),
                &pool,
            )?
        };

        // Resume plan: hand the prior agent_session_key back to the
        // runtime adapter so claude-code uses `--resume <uuid>` and
        // codex (once capture lands) uses `codex resume <uuid>`.
        //
        // claude-code only: if the conversation file for this
        // (cwd, uuid) was never persisted, `--resume <uuid>` would
        // print "No conversation found" and leave the TUI half-broken.
        // Detect the missing file up front and degrade to a fresh
        // spawn with a newly self-assigned uuid via `--session-id`.
        let resolved_cwd_for_check: Option<String> = snap.cwd.clone().or_else(|| {
            snap.runner_id
                .as_ref()
                .and_then(|_| runner.working_dir.clone())
        });
        let is_lead_slot = mission_ctx.as_ref().is_some_and(|c| c.lead);
        let conversation_missing =
            match (runner.runtime.as_str(), snap.agent_session_key.as_deref()) {
                ("claude-code", Some(key)) => !router::runtime::claude_code_conversation_exists(
                    resolved_cwd_for_check.as_deref(),
                    key,
                ),
                _ => false,
            };
        if conversation_missing && !allow_fresh_fallback {
            return Err(Error::msg(format!(
                "session {session_id} conversation is unavailable; resume it manually to start fresh"
            )));
        }
        let fresh_fallback_lead = conversation_missing && is_lead_slot;
        let effective_prior_key = match (runner.runtime.as_str(), snap.agent_session_key.as_deref())
        {
            ("claude-code", Some(_)) if conversation_missing => None,
            (_, k) => k,
        };
        let plan = router::runtime::resume_plan(&runner.runtime, effective_prior_key);
        if !allow_fresh_fallback && !plan.resuming && runner.runtime != "shell" {
            return Err(Error::msg(format!(
                "session {session_id} cannot resume its prior conversation; resume it manually to start fresh"
            )));
        }

        // Direct chats keep `spawn_direct`'s hard error for an explicitly
        // missing cwd; mission slots retain their existing resume behavior.
        // Shells are safe to relaunch at a fallback directory because a human
        // remains at the prompt, so resolve that fallback before portable-pty
        // can silently substitute HOME.
        let (resolved_cwd, shell_cwd_notice) = if runner.runtime == "shell" {
            let project_cwd = match snap.project_id.as_deref() {
                Some(project_id) => {
                    let conn = pool.get()?;
                    crate::repo::project::get(&conn, project_id)?.map(|project| project.cwd)
                }
                None => None,
            };
            let home = std::env::var_os("HOME").map(PathBuf::from);
            let (cwd, notice) = resolve_shell_resume_cwd(
                snap.cwd.as_deref(),
                project_cwd.as_deref(),
                home.as_deref(),
            )?;
            (Some(cwd), notice)
        } else {
            let cwd = snap.cwd.clone().or_else(|| {
                snap.runner_id
                    .as_ref()
                    .and_then(|_| runner.working_dir.clone())
            });
            if mission_ctx.is_none() {
                if let Some(missing_cwd) = cwd
                    .as_deref()
                    .filter(|cwd| !cwd.is_empty() && !Path::new(cwd).is_dir())
                {
                    return Err(Error::msg(format!(
                        "working directory does not exist: {missing_cwd}"
                    )));
                }
            }
            (cwd, None)
        };

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
        } else if runner.runtime != "shell" {
            env_extra.insert("RUNNER_HANDLE".into(), runner.handle.clone());
        }

        // Caller-supplied size wins; else the row's persisted last size
        // (migration 0016); else the default. Resuming at the pane's
        // real width keeps full-frame TUIs from repainting at 80 cols.
        let (initial_size, size_source) = match (cols.zip(rows), snap.last_cols.zip(snap.last_rows))
        {
            (Some(size), _) => (size, "caller-supplied"),
            (None, Some(size)) => (size, "persisted-last-size"),
            (None, None) => (super::DEFAULT_PTY_SIZE, "DEFAULT_PTY_SIZE"),
        };
        let mut spec = self.base_spawn_spec(
            session_id.to_string(),
            &runner,
            resolved_cwd.clone(),
            mission_ctx.is_some(),
            shim_dir,
            bundled_bin_dir,
            Some(initial_size),
            env_extra,
        );
        let mission_bus_dir = mission_ctx.as_ref().map(|ctx| {
            runner_core::event_log::path::mission_dir(app_data_dir, &ctx.crew_id, &ctx.mission_id)
        });
        // Resume never delivers a first-turn via argv: a real resume
        // restores prior context via the agent CLI's own session
        // resume, and the rare fresh-fallback case routes its launch
        // prompt through paste-and-verify via the caller in
        // `ops::session::session_resume`. `first_turn = None`
        // here so the argv path stays inert.
        let _ = Self::apply_runtime_args(
            &mut spec,
            &runner,
            &plan,
            app_data_dir,
            None,
            mission_bus_dir.as_deref(),
        );

        let started_at_dt = Utc::now();
        let started_at = started_at_dt.to_rfc3339();

        // UPDATE in place: same id, same conversation thread.
        {
            let conn = pool.get()?;
            crate::repo::session::resume_in_place(
                &conn,
                session_id,
                started_at_dt,
                plan.assigned_key.as_deref(),
                initial_size.0,
                initial_size.1,
            )?;
        }

        // No gate on the resume path: `claude --resume <uuid>` /
        // `--session-id <uuid>` loads the local conversation file and
        // puts up the TUI without touching the network until the
        // user's next turn. No proactive OAuth refresh at resume
        // means no concurrent refresh-token race, so Resume-all over
        // N stopped slots can spawn as fast as the runtime allows.
        // See issue #171.
        let spawn_started_at_dt = Utc::now();
        self.seed_codex_project_trust(session_id, &runner.runtime, spec.cwd.as_deref());
        let (rt_session, output) = match self.runtime.spawn(spec) {
            Ok(p) => p,
            Err(e) => {
                // Roll the row back to stopped so the user can retry.
                if let Ok(conn) = pool.get() {
                    let _ = crate::repo::session::set_exit_status(
                        &conn,
                        session_id,
                        crate::model::SessionStatus::Stopped,
                        Utc::now(),
                    );
                }
                return Err(Error::msg(format!("spawn {}: {e}", runner.command)));
            }
        };

        // The PTY exists — this line reports a fork that actually
        // happened, not an attempt. One per resume so a production log
        // shows the width every resumed session started at and where it
        // came from (#366).
        log::info!(
            "{} fork: session={session_id} runtime={} size={}x{} source={size_source}",
            if allow_fresh_fallback {
                "resume"
            } else {
                "resume-on-launch"
            },
            runner.runtime,
            initial_size.0,
            initial_size.1,
        );

        let spawn_pid = self.runtime_pid(&rt_session);

        if let Ok(conn) = pool.get() {
            let _ = crate::repo::session::update_runtime_metadata(
                &conn,
                session_id,
                &rt_session.runtime,
                &rt_session.session_id,
                spawn_pid,
            );
        }

        let codex_capture =
            if matches!(runner.runtime.as_str(), "codex" | "trae") && plan.assigned_key.is_none() {
                crate::session::codex_capture::sessions_root_for(&runner.runtime).and_then(
                    |sessions_root| {
                        capture_cwd(resolved_cwd.clone()).map(|cwd| CodexCaptureContext {
                            mission_id: snap.mission_id.clone(),
                            sessions_root,
                            spawn_cwd: cwd,
                            started_at: spawn_started_at_dt,
                            row_started_at: started_at.clone(),
                            spawn_pid,
                            prompt_marker: None,
                            pool: Arc::clone(&pool),
                            events: Arc::clone(&events),
                        })
                    },
                )
            } else {
                None
            };

        let resume_emit_ctx = mission_ctx.as_ref().and_then(|ctx| {
            open_mission_event_log(app_data_dir, &ctx.crew_id, &ctx.mission_id).map(|event_log| {
                ForwarderEmitCtx {
                    crew_id: ctx.crew_id.clone(),
                    mission_id: ctx.mission_id.clone(),
                    handle: ctx.slot_handle.clone(),
                    event_log,
                }
            })
        });
        self.install_handle(
            session_id,
            SessionHandle {
                id: session_id.to_string(),
                mission_id: snap.mission_id.clone(),
                runner_id: snap.runner_id.clone(),
                runtime_session: rt_session.clone(),
                codex_capture: codex_capture.clone(),
                forwarder: None,
                stop: output.stop_flag(),
            },
            resume_emit_ctx.clone(),
            Some(initial_size),
            &pool,
            events.as_ref(),
        );
        if let Some(notice) = shell_cwd_notice.as_deref() {
            self.ingest_output_chunk(
                session_id,
                snap.mission_id.as_deref(),
                notice,
                events.as_ref(),
            );
        }
        if snap.mission_id.is_none() {
            self.publish_direct_activity(
                session_id,
                SessionActivityState::Busy,
                "resume",
                events.as_ref(),
            );
        }

        let forwarder = self.start_forwarder_thread(
            session_id.to_string(),
            snap.mission_id.clone(),
            rt_session,
            output,
            Arc::clone(&pool),
            Arc::clone(&events),
            runner.clone(),
            plan.resuming,
            snap.runner_id.is_some(),
            resume_emit_ctx,
        );
        self.install_forwarder(session_id, forwarder);

        if let Some(ctx) = codex_capture.as_ref() {
            self.spawn_codex_capture_if_unkeyed(session_id, ctx);
        }

        if snap.runner_id.is_some() {
            emit_runner_activity(&pool, &runner, events.as_ref());
        }

        // First-turn warning for fresh claude-code / codex spawns.
        // `plan.resuming` is true on any resume against a real
        // prior_key — those skip naturally (the agent already has its
        // system context). For mission resume, the lead always
        // suppresses the worker preamble: when the lead's
        // conversation file is missing and the resume degrades to a
        // fresh spawn, the *launch prompt* (composed by the router
        // with crew / roster / goal context) is the right thing to
        // inject — the ops::session::session_resume caller fires
        // that path when it sees `fresh_fallback_lead = true` on the
        // returned SpawnedSession. For direct-chat resume there's no
        // slot/lead concept; if that degrades to fresh and argv
        // delivery was unavailable, we log the skipped injection.
        if matches!(runner.runtime.as_str(), "claude-code" | "codex" | "trae") && !plan.resuming {
            if mission_ctx.is_some() {
                log::warn!(
                    "first-turn argv not delivered for {session_id} (runtime {}); skipping post-spawn injection",
                    runner.runtime,
                );
            } else {
                log::warn!(
                    "first-turn argv not delivered for direct chat {session_id} (runtime {}); skipping post-spawn injection",
                    runner.runtime,
                );
            }
        }

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
            runner_id: snap.runner_id.clone(),
            handle: resumed_handle,
            pid: None,
            fresh_fallback_lead,
        })
    }

    /// True iff the `sessions` row for `session_id` is still in the
    /// DB. False if the row was deleted out from under an in-flight
    /// spawn — most commonly `runner_delete` triggering the foreign
    /// key cascade on `sessions.runner_id`, but also covers manual
    /// DB cleanup or any other path that drops the row while a
    /// gated spawn was asleep. Returns false on pool errors so the
    /// caller treats "can't tell" the same as "deleted" and bails
    /// out of the spawn — losing a session on a transient DB hiccup
    /// is preferable to leaving an orphan PTY attached to no row.
    fn session_row_exists(pool: &DbPool, session_id: &str) -> bool {
        let Ok(conn) = pool.get() else { return false };
        let count: rusqlite::Result<i64> = conn.query_row(
            "SELECT COUNT(*) FROM sessions WHERE id = ?1",
            params![session_id],
            |r| r.get(0),
        );
        count.map(|n| n > 0).unwrap_or(false)
    }

    /// The row's persisted last size: the hint from registration until a
    /// frontend push lands, then whatever `resize` recorded.
    pub(super) fn persisted_size(pool: &DbPool, session_id: &str) -> Option<(u16, u16)> {
        let conn = pool.get().ok()?;
        let row = crate::repo::session::get_row(&conn, session_id).ok()??;
        row.last_cols.zip(row.last_rows)
    }

    fn runtime_pid(&self, rt_session: &RuntimeSession) -> Option<i32> {
        self.runtime
            .status(rt_session)
            .ok()
            .flatten()
            .and_then(|status| status.pid)
    }
}

fn resolve_shell_resume_cwd(
    recorded_cwd: Option<&str>,
    project_cwd: Option<&str>,
    home: Option<&Path>,
) -> Result<(String, Option<Vec<u8>>)> {
    let recorded_path = recorded_cwd
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty())
        .map(Path::new);
    let resolved = recorded_path
        .and_then(|cwd| cwd.ancestors().find(|candidate| candidate.is_dir()))
        .or_else(|| {
            project_cwd
                .map(str::trim)
                .filter(|cwd| !cwd.is_empty())
                .map(Path::new)
                .filter(|cwd| cwd.is_dir())
        })
        .or_else(|| home.filter(|home| home.is_dir()))
        .ok_or_else(|| Error::msg("shell session has no existing working directory fallback"))?;
    let resolved = resolved.to_string_lossy().into_owned();
    let notice = recorded_path
        .filter(|recorded| *recorded != Path::new(&resolved))
        .map(|recorded| {
            let recorded = display_terminal_path(recorded, home);
            let resolved = display_terminal_path(Path::new(&resolved), home);
            format!(
                "\x1b[33mrunner: {recorded} no longer exists\r\n        opened {resolved} instead\x1b[0m\r\n"
            )
            .into_bytes()
        });
    Ok((resolved, notice))
}

fn display_terminal_path(path: &Path, home: Option<&Path>) -> String {
    let Some(home) = home else {
        return path.to_string_lossy().into_owned();
    };
    if path == home {
        return "~".into();
    }
    match path.strip_prefix(home) {
        Ok(relative) => format!("~/{}", relative.to_string_lossy()),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}
