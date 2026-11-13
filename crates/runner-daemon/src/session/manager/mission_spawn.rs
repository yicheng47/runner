use super::*;

impl SessionManager {
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
        role: &Role,
        slot: &crate::model::Slot,
        app_data_dir: &Path,
        events_log_path: PathBuf,
        pool: Arc<DbPool>,
        system_prompt: Option<String>,
        first_turn: Option<String>,
        initial_size: Option<(u16, u16)>,
        size_source: &'static str,
    ) -> Result<PendingMissionSpawn> {
        let initial_size = Some(initial_size.unwrap_or(DEFAULT_PTY_SIZE));

        // Slot-level runtime override (feature 41): the effective
        // runtime is `slot.runtime_override ?? role.runtime`. On a
        // differing override the spawn uses registry command, default
        // args, model, and effort; persona fields carry over. Slot
        // model/effort overrides apply last.
        let effective = resolve_runtime_override(
            role,
            slot.runtime_override.as_deref(),
            slot.model_override.as_deref(),
            slot.effort_override.as_deref(),
        )?;
        let mut role = effective.unwrap_or_else(|| role.clone());
        if crate::runtimes::for_key(&role.runtime)
            .capabilities()
            .codex_speed
        {
            role.codex_speed = slot.codex_speed_override.or(role.codex_speed);
        }
        let mut launch_definition = crate::session::launch_definition::from_effective(&role);
        role.args = crate::runtimes::for_key(&role.runtime)
            .permissions()
            .apply_mission(&role.args);
        let mut role = self.resolve_role_executable(&role, &pool)?;
        launch_definition.claude_account = crate::claude_accounts::launch_account(
            &pool,
            &mut role,
            &self.shell_env.read().unwrap().vars,
            None,
            true,
        )?;
        if let Some(account) = &mut launch_definition.claude_account {
            account.permission_mode = crate::runtimes::for_key(&role.runtime)
                .native_permission_mode(&role.args)
                .expect("account runtime has a native permission mode");
        }

        // Agent-native session resume: this is a *fresh* session row, so
        // there's no prior key to inherit. The runtime adapter still
        // self-assigns a UUID for claude-code (`--session-id <uuid>`) so
        // a future `SessionManager::resume` can hand it back.
        let plan = crate::runtimes::for_key(&role.runtime).resume_plan(None);

        // Working directory: mission cwd if set, else role override, else
        // inherit parent's. The mission-level cwd is what the operator typed
        // into the Start-mission modal and the modal's helper text promises
        // it wins ("Each role's PTY starts in this directory"). Capture the
        // resolved cwd so we can persist it on the session row — `resume`
        // reads it back to spawn the same dir on respawn, which matters for
        // claude-code (its conversation files are keyed under
        // `~/.claude/projects/<encoded-cwd>/<uuid>.jsonl`; resuming with a
        // different cwd makes `--resume` fail).
        let resolved_cwd = resolve_spawn_cwd(mission.cwd.as_deref(), role.working_dir.as_deref());

        // Per-slot role shim: hardcodes the RUNNER_* env vars + exec's
        // the real bundled CLI. claude-code's Bash tool spawns
        // non-login shells that don't inherit the PTY's env, so a CLI
        // call like `runner msg post …` would otherwise see the vars
        // as unset. The shim sits in front of the bundled `runner` on
        // PATH so `runner` resolves to it regardless of shell context.
        let shim_dir = crate::cli_install::install_session_runner_shim(
            app_data_dir,
            mission.crew_id.as_deref(),
            &mission.id,
            &slot.slot_handle,
            &events_log_path,
            mission.cwd.as_deref(),
        )
        .inspect_err(|error| log::warn!("install session runner shim: {error}"))
        .ok();
        let bundled_bin_dir = Some(app_data_dir.join("bin"));

        let mut mission_env: BTreeMap<String, String> = BTreeMap::new();
        if let Some(crew_id) = &mission.crew_id {
            mission_env.insert("RUNNER_CREW_ID".into(), crew_id.clone());
        }
        mission_env.insert("RUNNER_MISSION_ID".into(), mission.id.clone());
        // RUNNER_HANDLE is the slot's in-mission identity, not the
        // role template's handle.
        mission_env.insert("RUNNER_HANDLE".into(), slot.slot_handle.clone());
        mission_env.insert(
            "RUNNER_EVENT_LOG".into(),
            events_log_path.to_string_lossy().to_string(),
        );
        if let Some(wd) = mission.cwd.as_deref() {
            mission_env.insert("MISSION_CWD".into(), wd.to_string());
        }

        let session_id = ulid::Ulid::new().to_string();
        let system_prompt_path = system_prompt
            .as_deref()
            .map(|body| crate::session::system_prompt::write(app_data_dir, &session_id, body))
            .transpose()?;
        let (first_turn, capture_prompt_marker) =
            Self::capture_prompt_marker(&role.runtime, &session_id, first_turn);
        let mut spec = self.base_spawn_spec(
            session_id.clone(),
            &role,
            resolved_cwd.clone(),
            true,
            shim_dir,
            bundled_bin_dir,
            initial_size,
            mission_env,
        );
        let mission_bus_dir = runner_core::event_log::path::mission_dir(
            app_data_dir,
            mission.crew_id.as_deref(),
            &mission.id,
        );
        let first_turn_delivered_via_argv = self.apply_runtime_args(
            &mut spec,
            &role,
            &plan,
            app_data_dir,
            system_prompt_path.as_deref(),
            first_turn.as_deref(),
            Some(&mission_bus_dir),
            Some(&pool),
        );
        crate::claude_accounts::sharing::launch(
            &pool,
            launch_definition.claude_account.as_ref(),
            &role,
            &mut spec,
        )?;

        // Insert the row first (status=running with no runtime_*
        // metadata yet) so a fast-failing runtime spawn doesn't leave
        // a half-row. We update with runtime metadata once the
        // runtime hands them back.
        let started_at_dt = Utc::now();
        let started_at = started_at_dt.to_rfc3339();
        let insert_result = self.report_key(
            &session_id,
            PersistKey {
                key: plan.assigned_key.clone(),
                generation: started_at.clone(),
                origin: KeyOrigin::Assigned,
            },
            |effect| {
                let conn = pool.get()?;
                let mut row = crate::repo::session::SessionRowDb::new_running(session_id.clone());
                row.mission_id = Some(mission.id.clone());
                row.project_id = mission.project_id.clone();
                row.role_id = Some(role.id.clone());
                row.slot_id = Some(slot.id.clone());
                row.cwd = resolved_cwd.clone();
                row.started_at = Some(started_at_dt);
                row.agent_session_key = effect.key.clone();
                row.last_cols = initial_size.map(|(cols, _)| cols);
                row.last_rows = initial_size.map(|(_, rows)| rows);
                // Record the spawned engine so the session keeps its
                // identity and its resume engine even if the slot's
                // override or the role template's runtime is edited later.
                row.agent_runtime = Some(role.runtime.clone());
                row.agent_command = Some(role.command.clone());
                row.launch_definition = Some(launch_definition.clone());
                crate::repo::session::insert(&conn, &row)?;
                Ok(true)
            },
        );
        if let Err(error) = insert_result {
            crate::session::system_prompt::remove(app_data_dir, &session_id);
            return Err(error);
        }

        self.raw_session_state(&session_id)
            .expect("registered mission session")
            .lock()
            .unwrap()
            .pending_spawn = true;

        Ok(PendingMissionSpawn {
            pre_attachment: PreAttachmentState {
                manager: Arc::downgrade(self),
                session_id: session_id.clone(),
            },
            session_id,
            launch_definition,
            spec,
            mission: mission.clone(),
            role: role.clone(),
            slot_handle: slot.slot_handle.clone(),
            size_source,
            plan,
            first_turn_delivered_via_argv,
            first_turn,
            resolved_cwd,
            row_started_at: started_at,
            capture_prompt_marker,
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
            launch_definition: _,
            pre_attachment: _pre_attachment,
            mut spec,
            mission,
            role,
            slot_handle,
            mut size_source,
            plan,
            first_turn_delivered_via_argv,
            first_turn,
            resolved_cwd,
            row_started_at,
            capture_prompt_marker,
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
                "mission session spawn cancelled pre-gate: mission={} session={} role={}",
                mission.id,
                session_id,
                slot_handle,
            );
            crate::session::system_prompt::remove(&app_data_dir, &session_id);
            return Ok(CompleteSpawnOutcome::Cancelled);
        }

        // Gate claude-code spawns so N parallel mission slots don't
        // race the OAuth refresh-token rotation. No-op for other
        // runtimes; zero-wait for the first claude through. See
        // `enter_launch_gate` + issue #171.
        self.enter_launch_gate(&session_id, &role.runtime);

        // Post-gate cancellation: covers a Stop that fires while we
        // were asleep in the gate. The wake-up still races with the
        // cancel — flagging it here means we observe it before the
        // expensive `runtime.spawn`. Also covers the case where
        // `role_delete` removed the row along with the role's other
        // sessions — surfaces the same way (we have no
        // row to attach a PTY to).
        if cancel.load(Ordering::Acquire) || !Self::session_row_exists(&pool, &session_id) {
            log::info!(
                "mission session spawn cancelled post-gate: mission={} session={} role={}",
                mission.id,
                session_id,
                slot_handle,
            );
            crate::session::system_prompt::remove(&app_data_dir, &session_id);
            return Ok(CompleteSpawnOutcome::Cancelled);
        }

        let spawn_started_at_dt = Utc::now();
        #[cfg(windows)]
        let first_turn_deadline = Instant::now() + WINDOWS_FIRST_TURN_TIMEOUT;
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
        self.seed_runtime_project_trust(
            &session_id,
            &role.runtime,
            spec.cwd.as_deref(),
            role.env.get("COPILOT_HOME").map(String::as_str),
            crate::runtimes::claude_code::spawned_config_dir(&spec.env),
        );
        let capture_root =
            match crate::runtimes::for_key(&role.runtime).key_capture_for_spawn(&spec) {
                crate::runtimes::KeyCapture::RolloutScan { sessions_root } => sessions_root,
                _ => None,
            };
        let spawning = self.shutdown.read().unwrap();
        if *spawning {
            return Err(Error::msg("runnerd is stopping"));
        }
        let (rt_session, output) = match self.spawn_runtime(spec) {
            Ok(spawned) => spawned,
            Err(error) => {
                crate::session::system_prompt::remove(&app_data_dir, &session_id);
                return Err(Error::msg(format!("spawn {}: {error}", role.command)));
            }
        };

        // Post-spawn cancellation. Two triggers reach this branch:
        //   1. A `Stop`/`Archive` that fired while the
        //      runtime was mid-fork — `kill_all_for_mission` can't
        //      see the PTY yet (no `SessionHandle` in `sessions`
        //      until the insert below). Flagged by `cancel`.
        //   2. A `role_delete` that removed the row
        //      while runtime was mid-fork. Flagged by the row check.
        // Either way the PTY exists with no DB anchor; tear it down
        // before any further bookkeeping. The dropped output stream
        // triggers EOF in the reader thread and reaps the child.
        if cancel.load(Ordering::Acquire) || !Self::session_row_exists(&pool, &session_id) {
            log::info!(
                "mission session spawn cancelled post-runtime-spawn: \
                 mission={} session={} role={}",
                mission.id,
                session_id,
                slot_handle,
            );
            if let Err(e) = self.runtime.stop(&rt_session) {
                log::warn!(
                    "failed to stop just-spawned PTY for cancelled session {session_id}: {e}"
                );
            }
            crate::session::system_prompt::remove(&app_data_dir, &session_id);
            return Ok(CompleteSpawnOutcome::Cancelled);
        }

        // The PTY exists and survived both cancellation windows — this
        // reports a fork that actually happened, not an attempt.
        if let Some((cols, rows)) = initial_size {
            log::info!(
                "mission slot fork: session={session_id} runtime={} \
                 size={cols}x{rows} source={size_source}",
                role.runtime,
            );
        }

        let spawn_pid = self.runtime_pid(&rt_session);

        // Persist the runtime-side identity for diagnostics and for
        // the current runtime session row.
        if let Ok(conn) = pool.get() {
            let _ =
                crate::repo::session::record_process(&conn, &session_id, spawn_pid, &role.command);
        }

        let rollout_capture = if plan.assigned_key.is_none() {
            capture_root.and_then(|sessions_root| {
                resolved_cwd.clone().map(|cwd| RolloutCaptureContext {
                    runtime: role.runtime.clone(),
                    manager: Arc::downgrade(self),
                    mission_id: Some(mission.id.clone()),
                    sessions_root,
                    spawn_cwd: cwd,
                    started_at: spawn_started_at_dt,
                    row_started_at: row_started_at.clone(),
                    spawn_pid,
                    prompt_marker: capture_prompt_marker.clone(),
                    pool: Arc::clone(&pool),
                    events: Arc::clone(&events),
                })
            })
        } else {
            None
        };

        let spawn_emit_ctx =
            open_mission_event_log(&app_data_dir, mission.crew_id.as_deref(), &mission.id).map(
                |event_log| ForwarderEmitCtx {
                    crew_id: mission.crew_id.clone(),
                    mission_id: mission.id.clone(),
                    handle: slot_handle.clone(),
                    event_log,
                },
            );
        let stop = output.stop_flag();
        self.start_runtime_capture(
            &role,
            &session_id,
            Some(mission.id.clone()),
            resolved_cwd.clone(),
            plan.assigned_key.clone(),
            &app_data_dir,
            &row_started_at,
            Arc::clone(&stop),
            &pool,
            &events,
        );
        let runtime_session_for_log = rt_session.session_id.clone();
        self.install_handle(
            &session_id,
            SessionHandle {
                startup: Some(startup::StartupDelivery::new(
                    &role.runtime,
                    !plan.resuming && first_turn.is_some(),
                    super::super::clock::now(),
                )),
                #[cfg(windows)]
                pending_first_turn: None,
                id: session_id.clone(),
                mission_id: Some(mission.id.clone()),
                role_id: Some(role.id.clone()),
                runtime_session: rt_session.clone(),
                rollout_capture: rollout_capture.clone(),
                forwarder: None,
                stop,
            },
            spawn_emit_ctx.clone(),
            initial_size,
            &pool,
            &events,
        );
        if first_turn_delivered_via_argv {
            self.arm_completion(&session_id);
        }
        self.publish_mission_activity(
            &session_id,
            SessionActivityState::Busy,
            StatusSource::Spawn,
            events.as_ref(),
        );

        #[cfg(windows)]
        self.queue_windows_batch_first_turn(
            &session_id,
            &role,
            &plan,
            first_turn.as_deref(),
            first_turn_deadline,
        );

        let forwarder = self.start_forwarder_thread(
            session_id.clone(),
            row_started_at,
            Some(mission.id.clone()),
            rt_session,
            output,
            Arc::clone(&pool),
            Arc::clone(&events),
            role.clone(),
            None,
            true,
            spawn_emit_ctx,
            app_data_dir.clone(),
        );
        self.install_forwarder(&session_id, forwarder);

        if let Some(ctx) = rollout_capture.as_ref() {
            self.spawn_rollout_capture_if_unkeyed(&session_id, ctx);
        }

        emit_role_activity(&pool, &role, events.as_ref());
        log::info!(
            "session spawn: mission={} session={} role={} runtime_session={}",
            mission.id,
            session_id,
            slot_handle,
            runtime_session_for_log,
        );

        Ok(CompleteSpawnOutcome::Spawned)
    }

    /// Spawn one PTY child for `role` as part of `mission`. Persists a
    /// `sessions` row, starts the reader thread, and returns a summary for
    /// the frontend.
    ///
    /// `app_data_dir` is the root of `$APPDATA/role/` so we can prepend
    /// `<app_data_dir>/bin` onto the child's PATH — arch §5.3 Layer 2 and
    /// 0001-v0-mvp.md C9 both require the bundled `role` CLI to win over any
    /// system binary with the same name.
    /// `first_turn` is the composed first-user-turn body to deliver
    /// at spawn (team preamble, shared goal, conventions and brief).
    /// When the runtime accepts the
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
        role: &Role,
        slot: &crate::model::Slot,
        app_data_dir: &Path,
        events_log_path: PathBuf,
        pool: Arc<DbPool>,
        events: Arc<dyn SessionEvents>,
        first_turn: Option<String>,
    ) -> Result<SpawnedSession> {
        let (system_prompt, first_turn) = crate::runtimes::for_key(&role.runtime)
            .prompt_channels()
            .split(router::prompt::SessionPromptKind::Member, first_turn);
        self.spawn_with_prompt_channels(
            mission,
            role,
            slot,
            app_data_dir,
            events_log_path,
            pool,
            events,
            system_prompt,
            first_turn,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn_with_prompt_channels(
        self: &Arc<Self>,
        mission: &Mission,
        role: &Role,
        slot: &crate::model::Slot,
        app_data_dir: &Path,
        events_log_path: PathBuf,
        pool: Arc<DbPool>,
        events: Arc<dyn SessionEvents>,
        system_prompt: Option<String>,
        first_turn: Option<String>,
    ) -> Result<SpawnedSession> {
        let pending = self.register_mission_session(
            mission,
            role,
            slot,
            app_data_dir,
            events_log_path,
            Arc::clone(&pool),
            system_prompt,
            first_turn,
            None,
            "DEFAULT_PTY_SIZE",
        )?;
        let session_id = pending.session_id.clone();
        let mission_id = pending.mission.id.clone();
        let role_id = pending.role.id.clone();
        let handle = pending.role.handle.clone();
        // No batch context in the sync wrapper, so cancellation is
        // never set externally — pass a fresh flag so the cancel
        // checks are a no-op for this path.
        let noop_cancel = Arc::new(AtomicBool::new(false));
        if let Err(e) = self.complete_mission_session_spawn(pending, events, noop_cancel) {
            // Match the historical sync-spawn contract: if the runtime
            // can't bring the PTY up, delete the half-row so retries
            // start from a clean slate. The async mission_start path
            // takes a softer line and marks the row crashed instead.
            let _ = delete_session_row(&pool, &session_id);
            return Err(e);
        }
        Ok(SpawnedSession {
            id: session_id,
            mission_id: Some(mission_id),
            role_id: Some(role_id),
            handle,
            // PTY child pid is populated lazily via runtime.status()
            // when the manager needs it; the SpawnedSession field is
            // informational and the frontend doesn't rely on it.
            pid: None,
        })
    }
}
