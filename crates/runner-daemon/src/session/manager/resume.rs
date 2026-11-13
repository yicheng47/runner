use super::*;

impl SessionManager {
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
        self.resume_with_fresh_fallback(
            session_id,
            cols,
            rows,
            app_data_dir,
            pool,
            events,
            true,
            false,
        )
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
        self.resume_with_fresh_fallback(
            session_id,
            cols,
            rows,
            app_data_dir,
            pool,
            events,
            false,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn restart(
        self: &Arc<Self>,
        session_id: &str,
        cols: Option<u16>,
        rows: Option<u16>,
        app_data_dir: &Path,
        pool: Arc<DbPool>,
        events: Arc<dyn SessionEvents>,
    ) -> Result<SpawnedSession> {
        self.resume_with_fresh_fallback(
            session_id,
            cols,
            rows,
            app_data_dir,
            pool,
            events,
            true,
            true,
        )
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
        fresh: bool,
    ) -> Result<SpawnedSession> {
        let _claim = self
            .try_claim_resume(session_id)
            .ok_or_else(|| Error::msg(format!("session {session_id} is already being resumed")))?;

        // Validate the row + collect everything we need under a single
        // short-lived connection. We deliberately don't hold the conn
        // across the spawn (which itself grabs a pool slot for the
        // status update).
        let snap = {
            let conn = pool.get()?;
            let row = crate::repo::session::get_row(&conn, session_id)?
                .ok_or_else(|| Error::msg(format!("session not found: {session_id}")))?;
            if !fresh && matches!(row.status, crate::model::SessionStatus::Running) {
                return Err(Error::msg(format!(
                    "session {session_id} is already running — attach instead"
                )));
            }
            if row.archived_at.is_some() {
                return Err(Error::msg(format!(
                    "session {session_id} is archived — un-archive before resuming"
                )));
            }
            if fresh && (row.mission_id.is_none() || row.slot_id.is_none()) {
                return Err(Error::msg("only mission slots can be restarted"));
            }
            row
        };

        // Mission resume: pull the slot + mission so we can stamp the
        // in-mission env (RUNNER_HANDLE = slot_handle, RUNNER_CREW_ID,
        // RUNNER_MISSION_ID). Direct-chat rows skip this lookup —
        // their RUNNER_HANDLE is the role template's globally-unique
        // handle, no slot involved.
        struct MissionCtx {
            crew_id: Option<String>,
            mission_id: String,
            mission_cwd: Option<String>,
            slot_handle: String,
            roster: crate::ops::mission::roster::LoadedRoster,
        }
        let mission_ctx: Option<MissionCtx> = match (
            snap.mission_id.as_deref(),
            snap.slot_id.as_deref(),
        ) {
            (Some(mid), Some(sid)) => {
                let conn = pool.get()?;
                let mission = crate::ops::mission::get(&conn, mid)?;
                if mission.archived_at.is_some()
                    || mission.status != crate::model::MissionStatus::Running
                {
                    return Err(Error::msg("mission is not running"));
                }
                let roster = crate::ops::mission::roster::load(&conn, app_data_dir, &mission)?;
                let slot = &roster.slots.iter().find(|member| member.slot.id == sid)
                    .ok_or_else(|| {
                        let dir = runner_core::event_log::mission_dir(app_data_dir, mission.crew_id.as_deref(), &mission.id);
                        let removed = runner_core::EventLog::open(&dir).and_then(|log| log.read_from_lossy(0)).ok()
                            .and_then(|(entries, _)| entries.into_iter().find(|entry| {
                                let event = &entry.event;
                                event.kind == runner_core::EventKind::Signal && event.from == "human"
                                    && event.signal_type.as_ref().is_some_and(|kind| kind.as_str() == "role_removed")
                                    && event.payload["slot_id"] == sid
                            }));
                        match removed.and_then(|entry| entry.event.payload["handle"].as_str().map(str::to_owned)) {
                            Some(handle) => Error::msg(format!("@{handle} was removed from this mission")),
                            None => Error::msg(format!("slot {sid} referenced by session {session_id} is not in the mission roster")),
                        }
                    })?.slot;
                Some(MissionCtx {
                    crew_id: mission.crew_id,
                    mission_id: mission.id,
                    mission_cwd: mission.cwd,
                    slot_handle: slot.slot_handle.clone(),
                    roster,
                })
            }
            _ => None,
        };

        let mut role = if fresh {
            let ctx = mission_ctx
                .as_ref()
                .ok_or_else(|| Error::msg("only mission slots can be restarted"))?;
            let member = ctx
                .roster
                .slots
                .iter()
                .find(|member| Some(&member.slot.id) == snap.slot_id.as_ref())
                .ok_or_else(|| Error::msg("mission slot is missing from roster"))?;
            let mut role = resolve_runtime_override(
                &member.role,
                member.slot.runtime_override.as_deref(),
                member.slot.model_override.as_deref(),
                member.slot.effort_override.as_deref(),
            )?
            .unwrap_or_else(|| member.role.clone());
            if crate::runtimes::for_key(&role.runtime)
                .capabilities()
                .codex_speed
            {
                role.codex_speed = member.slot.codex_speed_override.or(role.codex_speed);
            }
            role
        } else {
            let definition = snap.launch_definition.as_ref().ok_or_else(|| {
                Error::msg(format!("session {session_id} has no launch definition"))
            })?;
            let runtime = snap
                .agent_runtime
                .as_deref()
                .ok_or_else(|| Error::msg(format!("session {session_id} has no agent_runtime")))?;
            let identity = if let Some(role_id) = snap.role_id.as_deref() {
                crate::ops::role::get(&*pool.get()?, role_id)?
            } else {
                runtime_direct_role(runtime, Some(&definition.command), None, None)?
            };
            crate::session::launch_definition::to_role(definition, runtime, identity)
        };
        let mut new_definition =
            fresh.then(|| crate::session::launch_definition::from_effective(&role));
        if mission_ctx.is_some() {
            role.args = crate::runtimes::for_key(&role.runtime)
                .permissions()
                .apply_mission(&role.args);
        }
        role = self.resolve_role_executable(&role, &pool)?;

        if snap.mission_id.is_none() {
            role.args = crate::runtimes::for_key(&role.runtime)
                .permissions()
                .strip(&role.args);
        }
        let shell = Runtime::parse(&role.runtime).is_some_and(Runtime::is_shell);

        // Resume plan: hand the prior agent_session_key back to the
        // runtime adapter so claude-code uses `--resume <uuid>` and
        // codex (once capture lands) uses `codex resume <uuid>`.
        //
        // claude-code only: if the conversation file for this
        // (cwd, uuid) was never persisted, `--resume <uuid>` would
        // print "No conversation found" and leave the TUI half-broken.
        // Detect the missing file up front and degrade to a fresh
        // spawn with a newly self-assigned uuid via `--session-id`.
        let resolved_cwd_for_check = resolve_spawn_cwd(
            snap.cwd.as_deref(),
            snap.role_id.as_ref().and(role.working_dir.as_deref()),
        );
        let adapter = crate::runtimes::for_key(&role.runtime);
        let missing = adapter.missing_conversation();
        let probe_env = self.probe_env(adapter, &role.env);
        let previous_account = snap
            .launch_definition
            .as_ref()
            .and_then(|definition| definition.claude_account.as_ref());
        let mut claude_account = crate::claude_accounts::launch_account(
            &pool,
            &mut role,
            &self.shell_env.read().unwrap().vars,
            previous_account,
            fresh,
        )?;
        if let Some(account) = &mut claude_account {
            if let Some(key) = snap.agent_session_key.as_deref() {
                account.permission_mode = crate::claude_accounts::last_permission(
                    &account.origin_folder,
                    resolved_cwd_for_check.as_deref(),
                    key,
                    &account.permission_mode,
                );
            }
        }
        let recovery_env = probe_env.clone();
        let conversation_exists = {
            let origin_folder = claude_account
                .as_ref()
                .map(|account| account.origin_folder.clone());
            let cwd = resolved_cwd_for_check.clone();
            move |key: &str| {
                super::output::conversation_exists(
                    adapter,
                    origin_folder.as_deref(),
                    cwd.as_deref(),
                    &probe_env,
                    key,
                )
            }
        };
        let conversation_missing = snap
            .agent_session_key
            .as_deref()
            .is_some_and(|key| conversation_exists(key) == Some(false));
        let recovered_key = snap
            .agent_session_key
            .as_deref()
            .filter(|_| conversation_missing && !fresh)
            .and_then(|key| {
                let recovered = adapter.recover_conversation(
                    session_id,
                    key,
                    &crate::runtimes::ProbeContext {
                        cwd: resolved_cwd_for_check.as_deref(),
                        role_env: &recovery_env,
                    },
                )?;
                log::warn!(
                    "session {session_id} conversation {key} is missing; resuming its original conversation {recovered}"
                );
                Some(recovered)
            });
        let conversation_missing = conversation_missing && recovered_key.is_none();
        if conversation_missing && !allow_fresh_fallback && !missing.resume_on_launch {
            return Err(Error::msg(format!(
                "session {session_id} conversation is unavailable; resume it manually to start fresh"
            )));
        }
        if let Some(key) = snap
            .agent_session_key
            .as_deref()
            .filter(|_| conversation_missing && !fresh)
        {
            let folder = claude_account
                .as_ref()
                .map(|account| PathBuf::from(&account.origin_folder))
                .or_else(|| {
                    adapter.conversation_folder(&crate::runtimes::ProbeContext {
                        cwd: resolved_cwd_for_check.as_deref(),
                        role_env: &recovery_env,
                    })
                });
            super::output::warn_missing_conversation(session_id, key, folder.as_deref());
        }
        let effective_prior_key = if fresh || (conversation_missing && !missing.reuse_key) {
            None
        } else {
            recovered_key
                .as_deref()
                .or(snap.agent_session_key.as_deref())
        };
        let mut plan = adapter.resume_plan(effective_prior_key);
        if conversation_missing {
            plan.resuming = false;
        }
        if let Some(account) = &mut claude_account {
            if let Some(key) = effective_prior_key {
                if account.origin_folder != account.folder {
                    role.args = adapter.permissions().strip(&role.args);
                }
                crate::claude_accounts::resume_args(
                    &mut plan.args,
                    account,
                    &account.origin_folder,
                    resolved_cwd_for_check.as_deref(),
                    key,
                );
            }
            if !plan.resuming {
                account.origin_folder = account.folder.clone();
            }
            let definition = new_definition.get_or_insert_with(|| {
                snap.launch_definition
                    .clone()
                    .expect("recorded launch definition")
            });
            definition.claude_account = Some(account.clone());
        }
        if !allow_fresh_fallback
            && !plan.resuming
            && !shell
            && !(missing.resume_on_launch && conversation_missing)
        {
            return Err(Error::msg(format!(
                "session {session_id} cannot resume its prior conversation; resume it manually to start fresh"
            )));
        }
        if mission_ctx.is_some() && adapter.permissions().strip_on_mission_resume && plan.resuming {
            // A restored Trae thread already supplies its approvals reviewer.
            // Adding permission_mode makes thread/resume reject the config.
            role.args = adapter.permissions().strip(&role.args);
        }

        // Direct chats keep `spawn_direct`'s hard error for an explicitly
        // missing cwd; mission slots retain their existing resume behavior.
        // Shells are safe to relaunch at a fallback directory because a human
        // remains at the prompt, so resolve that fallback before portable-pty
        // can silently substitute HOME.
        let (resolved_cwd, shell_cwd_notice) =
            if Runtime::parse(&role.runtime).is_some_and(Runtime::is_shell) {
                let project_cwd = match snap.project_id.as_deref() {
                    Some(project_id) => {
                        let conn = pool.get()?;
                        crate::repo::project::get(&conn, project_id)?.map(|project| project.cwd)
                    }
                    None => None,
                };
                let home = runner_core::app_paths::home_dir();
                let (cwd, notice) = resolve_shell_resume_cwd(
                    snap.cwd.as_deref(),
                    project_cwd.as_deref(),
                    home.as_deref(),
                )?;
                (Some(cwd), notice)
            } else {
                let cwd = resolved_cwd_for_check;
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

        // Refresh the per-slot role shim before composing PATH —
        // mission cwd may have been edited since the last spawn.
        let shim_dir = mission_ctx.as_ref().and_then(|ctx| {
            let event_log_path = runner_core::event_log::path::events_path(
                app_data_dir,
                ctx.crew_id.as_deref(),
                &ctx.mission_id,
            );
            crate::cli_install::install_session_runner_shim(
                app_data_dir,
                ctx.crew_id.as_deref(),
                &ctx.mission_id,
                &ctx.slot_handle,
                &event_log_path,
                ctx.mission_cwd.as_deref(),
            )
            .ok()
        });
        let bundled_bin_dir = Some(app_data_dir.join("bin"));

        // Mission resume stamps the slot's in-mission identity; direct
        // chat resume falls through to the template handle.
        let mut env_extra: BTreeMap<String, String> = BTreeMap::new();
        if let Some(ctx) = mission_ctx.as_ref() {
            if let Some(crew_id) = &ctx.crew_id {
                env_extra.insert("RUNNER_CREW_ID".into(), crew_id.clone());
            }
            env_extra.insert("RUNNER_MISSION_ID".into(), ctx.mission_id.clone());
            env_extra.insert("RUNNER_HANDLE".into(), ctx.slot_handle.clone());
            let event_log_path = runner_core::event_log::path::events_path(
                app_data_dir,
                ctx.crew_id.as_deref(),
                &ctx.mission_id,
            );
            env_extra.insert(
                "RUNNER_EVENT_LOG".into(),
                event_log_path.to_string_lossy().to_string(),
            );
            if let Some(wd) = ctx.mission_cwd.as_deref() {
                env_extra.insert("MISSION_CWD".into(), wd.to_string());
            }
        } else if !Runtime::parse(&role.runtime).is_some_and(Runtime::is_shell) {
            env_extra.insert("RUNNER_HANDLE".into(), role.handle.clone());
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
            &role,
            resolved_cwd.clone(),
            mission_ctx.is_some(),
            shim_dir,
            bundled_bin_dir,
            Some(initial_size),
            env_extra,
        );
        let mission_bus_dir = mission_ctx.as_ref().map(|ctx| {
            runner_core::event_log::path::mission_dir(
                app_data_dir,
                ctx.crew_id.as_deref(),
                &ctx.mission_id,
            )
        });
        let (system_prompt, first_turn) = if !plan.resuming
            || adapter.prompt_channels().system_prompt
        {
            if let Some(ctx) = mission_ctx.as_ref() {
                let mut roster = ctx.roster.slots.clone();
                let member = roster
                    .iter_mut()
                    .find(|member| member.slot.slot_handle == ctx.slot_handle)
                    .ok_or_else(|| Error::msg("mission member missing from roster"))?;
                member.role.system_prompt.clone_from(&role.system_prompt);
                let member = member.clone();
                let log =
                    open_mission_event_log(app_data_dir, ctx.crew_id.as_deref(), &ctx.mission_id)
                        .ok_or_else(|| Error::msg("mission event log unavailable"))?;
                router::prompt::member_prompt_channels(
                    adapter,
                    &member,
                    &ctx.roster.file,
                    &log,
                    plan.resuming,
                )
            } else {
                // A fresh agy conversation has no persona unless it is resent: agy
                // takes no system prompt, and its lost conversation held the first turn.
                adapter.prompt_channels().split(
                    router::prompt::SessionPromptKind::Direct,
                    adapter
                        .prompt_channels()
                        .resend_persona_on_fresh
                        .then(|| {
                            router::prompt::compose_direct_first_turn(role.system_prompt.as_deref())
                        })
                        .flatten(),
                )
            }
        } else {
            (None, None)
        };
        if let (Some(ctx), Some(body)) = (mission_ctx.as_ref(), first_turn.as_deref()) {
            crate::ops::mission::ensure_first_turn_fits(&ctx.slot_handle, body)?;
        }
        // A new marker prevents the reused row from capturing its previous rollout.
        let marker_id = ulid::Ulid::new().to_string();
        let (first_turn, capture_prompt_marker) =
            Self::capture_prompt_marker(&role.runtime, &marker_id, first_turn);

        if fresh && matches!(snap.status, crate::model::SessionStatus::Running) {
            self.kill(session_id)?;
        }

        let system_prompt_path = system_prompt
            .as_deref()
            .map(|body| crate::session::system_prompt::write(app_data_dir, session_id, body))
            .transpose()?;
        let first_turn_delivered_via_argv = self.apply_runtime_args(
            &mut spec,
            &role,
            &plan,
            app_data_dir,
            system_prompt_path.as_deref(),
            first_turn.as_deref(),
            mission_bus_dir.as_deref(),
            Some(&pool),
        );
        crate::claude_accounts::sharing::launch(&pool, claude_account.as_ref(), &role, &mut spec)?;

        let started_at_dt = Utc::now();
        let started_at = started_at_dt.to_rfc3339();

        // UPDATE in place: same id, same conversation thread.
        let resume_result = self.report_key(
            session_id,
            PersistKey {
                key: plan.assigned_key.clone(),
                generation: started_at.clone(),
                origin: KeyOrigin::Assigned,
            },
            |effect| {
                let conn = pool.get()?;
                crate::repo::session::resume_in_place(
                    &conn,
                    session_id,
                    started_at_dt,
                    effect.key.as_deref(),
                    !plan.resuming,
                    initial_size,
                    new_definition
                        .as_ref()
                        .map(|definition| (role.runtime.as_str(), definition)),
                )?;
                Ok(true)
            },
        );
        if let Err(error) = resume_result {
            crate::session::system_prompt::remove(app_data_dir, session_id);
            return Err(error);
        }

        if !plan.resuming {
            self.enter_launch_gate(session_id, &role.runtime);
        }
        let spawn_started_at_dt = Utc::now();
        #[cfg(windows)]
        let first_turn_deadline = Instant::now() + WINDOWS_FIRST_TURN_TIMEOUT;
        self.seed_runtime_project_trust(
            session_id,
            &role.runtime,
            spec.cwd.as_deref(),
            role.env.get("COPILOT_HOME").map(String::as_str),
            crate::runtimes::claude_code::spawned_config_dir(&spec.env),
        );
        let starts_trusted = crate::runtimes::for_key(&role.runtime).starts_trusted(
            spec.cwd.as_deref(),
            &spec.args,
            &spec.env,
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
        let status_hooks_enabled = crate::runtimes::status_hooks_enabled(&spec.env);
        let (rt_session, output) = match self.spawn_runtime(spec) {
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
                crate::session::system_prompt::remove(app_data_dir, session_id);
                return Err(Error::msg(format!("spawn {}: {e}", role.command)));
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
            role.runtime,
            initial_size.0,
            initial_size.1,
        );

        let spawn_pid = self.runtime_pid(&rt_session);

        if let Ok(conn) = pool.get() {
            let _ =
                crate::repo::session::record_process(&conn, session_id, spawn_pid, &role.command);
        }

        let rollout_capture = if plan.assigned_key.is_none() {
            capture_root.and_then(|sessions_root| {
                resolved_cwd.clone().map(|cwd| RolloutCaptureContext {
                    runtime: role.runtime.clone(),
                    manager: Arc::downgrade(self),
                    mission_id: snap.mission_id.clone(),
                    sessions_root,
                    spawn_cwd: cwd,
                    started_at: spawn_started_at_dt,
                    row_started_at: started_at.clone(),
                    spawn_pid,
                    prompt_marker: capture_prompt_marker.clone(),
                    pool: Arc::clone(&pool),
                    events: Arc::clone(&events),
                })
            })
        } else {
            None
        };

        let resume_emit_ctx = mission_ctx.as_ref().and_then(|ctx| {
            open_mission_event_log(app_data_dir, ctx.crew_id.as_deref(), &ctx.mission_id).map(
                |event_log| ForwarderEmitCtx {
                    crew_id: ctx.crew_id.clone(),
                    mission_id: ctx.mission_id.clone(),
                    handle: ctx.slot_handle.clone(),
                    event_log,
                },
            )
        });
        self.start_runtime_capture(
            &role,
            session_id,
            snap.mission_id.clone(),
            resolved_cwd.clone(),
            plan.assigned_key.clone(),
            app_data_dir,
            &started_at,
            output.stop_flag(),
            &pool,
            &events,
        );
        self.install_handle(
            session_id,
            SessionHandle {
                // A slot's inbox, and a chat's reports (#748), wait until the agent is ready.
                startup: {
                    let pending_turn = !plan.resuming && first_turn.is_some();
                    let now = super::super::clock::now();
                    if snap.mission_id.is_some() {
                        Some(startup::StartupDelivery::new(
                            &role.runtime,
                            pending_turn,
                            now,
                        ))
                    } else if Runtime::parse(&role.runtime).is_some_and(Runtime::is_shell) {
                        None
                    } else {
                        // A resume that falls back to a fresh start can meet a trust prompt again,
                        // unless the runtime's config already records the folder as trusted.
                        let gate = if plan.resuming || (starts_trusted && !pending_turn) {
                            startup::StartupDelivery::for_trusted_reports
                        } else {
                            startup::StartupDelivery::for_reports
                        };
                        Some(
                            gate(&role.runtime, pending_turn, now)
                                .with_status_hooks(status_hooks_enabled),
                        )
                    }
                },
                #[cfg(windows)]
                pending_first_turn: None,
                id: session_id.to_string(),
                mission_id: snap.mission_id.clone(),
                role_id: snap.role_id.clone(),
                runtime_session: rt_session.clone(),
                rollout_capture: rollout_capture.clone(),
                forwarder: None,
                stop: output.stop_flag(),
            },
            resume_emit_ctx.clone(),
            Some(initial_size),
            &pool,
            &events,
        );
        if first_turn_delivered_via_argv {
            self.arm_completion(session_id);
        }
        #[cfg(windows)]
        self.queue_windows_batch_first_turn(
            session_id,
            &role,
            &plan,
            first_turn.as_deref(),
            first_turn_deadline,
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
                StatusSource::Resume,
                events.as_ref(),
            );
        }

        let resume_check = effective_prior_key.filter(|_| plan.resuming).map(|key| {
            let key = key.to_owned();
            Box::new(move || conversation_exists(&key)) as super::output::ConversationCheck
        });
        let forwarder = self.start_forwarder_thread(
            session_id.to_string(),
            started_at,
            snap.mission_id.clone(),
            rt_session,
            output,
            Arc::clone(&pool),
            Arc::clone(&events),
            role.clone(),
            resume_check,
            snap.role_id.is_some(),
            resume_emit_ctx,
            app_data_dir.to_path_buf(),
        );
        self.install_forwarder(session_id, forwarder);

        if let Some(ctx) = rollout_capture.as_ref() {
            self.spawn_rollout_capture_if_unkeyed(session_id, ctx);
        }

        if snap.role_id.is_some() {
            emit_role_activity(&pool, &role, events.as_ref());
        }

        // Return the slot's in-mission identity for mission rows so the
        // frontend (and the router, which keys on slot_handle) sees the
        // identity the resumed PTY actually stamps onto its events.
        let resumed_handle = mission_ctx
            .as_ref()
            .map(|c| c.slot_handle.clone())
            .unwrap_or_else(|| role.handle.clone());
        Ok(SpawnedSession {
            id: session_id.to_string(),
            mission_id: snap.mission_id.clone(),
            role_id: snap.role_id.clone(),
            handle: resumed_handle,
            pid: None,
        })
    }
}
