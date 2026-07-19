use super::*;

impl SessionManager {
    /// Gate a fresh `claude-code` spawn before calling
    /// `runtime.spawn()`. No-op for any other runtime — those
    /// bypass the gate.
    ///
    /// Only the *fresh-spawn* call sites (`spawn`, `spawn_direct`)
    /// invoke this. The resume path is intentionally unguarded:
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
        // Bottom layer: login-shell vars (proxy quartet, both cases)
        // captured at app start. A runner row can override any of these
        // by setting the same name in its own env map — the runner row
        // is the most specific configuration surface.
        let mut env: BTreeMap<String, String> = self.shell_env.vars.clone();
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
        SpawnSpec {
            session_id,
            cwd: cwd.map(PathBuf::from),
            command: runner.command.clone(),
            args: runner.args.clone(),
            env,
            mission,
            shim_dir,
            bundled_bin_dir,
            shell_path: self.shell_env.path.clone(),
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
    fn apply_runtime_args(
        spec: &mut SpawnSpec,
        runner: &Runner,
        plan: &router::runtime::ResumePlan,
        first_turn: Option<&str>,
        mission_bus_dir: Option<&Path>,
    ) -> bool {
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

    fn codex_capture_prompt_marker(
        runtime: &str,
        session_id: &str,
        first_turn: Option<String>,
    ) -> (Option<String>, Option<String>) {
        if runtime != "codex" {
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
    /// router/bus mount synchronously and return its Tauri command
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
    ) -> Result<PendingMissionSpawn> {
        // Slot-level runtime override (feature 41): the effective
        // runtime is `slot.runtime_override ?? runner.runtime`. On a
        // differing override the spawn uses registry command/default
        // args and drops model/effort; persona fields carry over. A
        // matching override spawns byte-identically but still pins.
        let resolution = resolve_runtime_override(runner, slot.runtime_override.as_deref())?;
        let pinned = resolution.pinned;
        let runner = resolution.effective.as_ref().unwrap_or(runner);

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
            runner,
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
            runner,
            &plan,
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
            if pinned {
                // Record the effective runtime so respawn/resume
                // keeps this session's engine even if the slot's
                // override — or the runner template's runtime — is
                // edited later. No-override rows stay NULL.
                row.agent_runtime = Some(runner.runtime.clone());
                row.agent_command = Some(runner.command.clone());
            }
            crate::repo::session::insert(&conn, &row)?;
        }

        Ok(PendingMissionSpawn {
            session_id,
            spec,
            mission: mission.clone(),
            runner: runner.clone(),
            slot_handle: slot.slot_handle.clone(),
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
            spec,
            mission,
            runner,
            slot_handle,
            plan,
            first_turn_delivered_via_argv,
            resolved_cwd,
            row_started_at,
            codex_prompt_marker,
            app_data_dir,
            pool,
        } = pending;

        // Pre-gate cancellation: a user who clicked Stop/Archive/Reset
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
        let initial_size = spec.initial_size;
        let (rt_session, output) = self
            .runtime
            .spawn(spec)
            .map_err(|e| Error::msg(format!("spawn {}: {e}", runner.command)))?;

        // Post-spawn cancellation. Two triggers reach this branch:
        //   1. A `Stop`/`Archive`/`Reset` that fired while the
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

        let codex_capture = if runner.runtime == "codex" && plan.assigned_key.is_none() {
            capture_cwd(resolved_cwd.clone()).map(|cwd| CodexCaptureContext {
                mission_id: Some(mission.id.clone()),
                spawn_cwd: cwd,
                started_at: spawn_started_at_dt,
                row_started_at: row_started_at.clone(),
                spawn_pid,
                prompt_marker: codex_prompt_marker.clone(),
                pool: Arc::clone(&pool),
                events: Arc::clone(&events),
            })
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
        if matches!(runner.runtime.as_str(), "claude-code" | "codex")
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
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_direct(
        self: &Arc<Self>,
        runner: &Runner,
        runtime_override: Option<&str>,
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
        let _ = app_data_dir; // direct chats don't get the bundled CLI on PATH

        // Chat-level runtime override (feature 41) — same resolution
        // rule as mission spawns.
        let resolution = resolve_runtime_override(runner, runtime_override)?;
        let pinned = resolution.pinned;
        let runner = resolution.effective.as_ref().unwrap_or(runner);

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
        let (first_turn, codex_prompt_marker) =
            Self::codex_capture_prompt_marker(&runner.runtime, &session_id, first_turn);
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
        let first_turn_delivered_via_argv =
            Self::apply_runtime_args(&mut spec, runner, &plan, first_turn.as_deref(), None);

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
            if persisted_runner_id.is_none() || pinned {
                row.agent_runtime = Some(runner.runtime.clone());
                row.agent_command = Some(runner.command.clone());
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

        let codex_capture = if runner.runtime == "codex" && plan.assigned_key.is_none() {
            capture_cwd(resolved_cwd.clone()).map(|cwd| CodexCaptureContext {
                mission_id: None,
                spawn_cwd: cwd,
                started_at: spawn_started_at_dt,
                row_started_at: started_at.clone(),
                spawn_pid,
                prompt_marker: codex_prompt_marker.clone(),
                pool: Arc::clone(&pool),
                events: Arc::clone(&events),
            })
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
            emit_runner_activity(&pool, runner, events.as_ref());
        }
        if matches!(runner.runtime.as_str(), "claude-code" | "codex")
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

        // Stamp the resume watermark (and, for full-frame-repaint
        // runtimes, purge the prior output buffer) up front. Two
        // properties depend on this happening *before* any
        // long-running step (gate, runtime.spawn, mission/runner
        // re-lookup):
        //
        // 1. The frontend's resuming-pill effect calls
        //    `session_output_snapshot` to catch a TUI-ready escape
        //    that fired before its live listener attached. The old
        //    PTY's chunks include the pre-stop `\x1b[?2004h`, so the
        //    pill only honors ready escapes in chunks with
        //    `seq > session_replay_watermark`. Stamping the watermark
        //    at the top closes the window where the snapshot could
        //    clear the resuming overlay before the new PTY exists.
        // 2. The seq counter is never touched (see
        //    `purge_output_buffer`'s contract) so the new PTY's first
        //    chunk continues at `last + 1` and the frontend's
        //    `seq <= lastWrittenSeq` filter doesn't drop it.
        //
        // Whether the ring itself survives is per-runtime: claude-code
        // keeps it (a terminal emulator would; scrolling up after
        // resume shows the prior conversation), codex/shells purge —
        // see `runtime_purges_on_resume`.
        self.set_resume_watermark(session_id);
        if output::runtime_purges_on_resume(session_id, &pool) {
            self.purge_output_buffer(session_id);
        }

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
            match resolve_runtime_override(&runner, snap.agent_runtime.as_deref())?.effective {
                Some(effective) => effective,
                None => runner,
            }
        } else {
            let runtime = snap.agent_runtime.as_deref().ok_or_else(|| {
                Error::msg(format!(
                    "runtime-only session {session_id} missing agent_runtime"
                ))
            })?;
            runtime_direct_runner(runtime, snap.agent_command.as_deref())?
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
        let resolved_cwd_for_check: Option<String> = snap.cwd.clone().or_else(|| {
            snap.runner_id
                .as_ref()
                .and_then(|_| runner.working_dir.clone())
        });
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
        let resolved_cwd: Option<String> = snap.cwd.clone().or_else(|| {
            snap.runner_id
                .as_ref()
                .and_then(|_| runner.working_dir.clone())
        });

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
        let mission_bus_dir = mission_ctx.as_ref().map(|ctx| {
            runner_core::event_log::path::mission_dir(app_data_dir, &ctx.crew_id, &ctx.mission_id)
        });
        // Resume never delivers a first-turn via argv: a real resume
        // restores prior context via the agent CLI's own session
        // resume, and the rare fresh-fallback case routes its launch
        // prompt through paste-and-verify via the caller in
        // `ops::session::session_resume`. `first_turn = None`
        // here so the argv path stays inert.
        let _ =
            Self::apply_runtime_args(&mut spec, &runner, &plan, None, mission_bus_dir.as_deref());

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

        let codex_capture = if runner.runtime == "codex" && plan.assigned_key.is_none() {
            capture_cwd(resolved_cwd.clone()).map(|cwd| CodexCaptureContext {
                mission_id: snap.mission_id.clone(),
                spawn_cwd: cwd,
                started_at: spawn_started_at_dt,
                row_started_at: started_at.clone(),
                spawn_pid,
                prompt_marker: None,
                pool: Arc::clone(&pool),
                events: Arc::clone(&events),
            })
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
            initial_size,
        );
        if snap.mission_id.is_none() {
            self.publish_direct_activity(
                session_id,
                SessionActivityState::Busy,
                "resume",
                events.as_ref(),
            );
        }

        // (The pre-runtime.spawn purge moved up to right after
        // snapshot collection so the frontend's snapshot fast-path
        // can't read the stopped session's chunks during the resume
        // window. See the call site above this function's mission
        // lookup.)

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
        if matches!(runner.runtime.as_str(), "claude-code" | "codex") && !plan.resuming {
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

    fn runtime_pid(&self, rt_session: &RuntimeSession) -> Option<i32> {
        self.runtime
            .status(rt_session)
            .ok()
            .flatten()
            .and_then(|status| status.pid)
    }
}
