use super::*;
use crate::model::CodexSpeed;

impl SessionManager {
    /// Spawn a "direct chat" PTY: a role process with **no parent
    /// mission**. Schema-supported since C5.5a (`sessions.mission_id` is
    /// nullable); C8.5 surfaces it as the "Chat now" affordance on the
    /// Role Detail page.
    ///
    /// Differences vs. the mission-flavored `spawn`:
    ///   - No `RUNNER_MISSION_ID`, `RUNNER_EVENT_LOG`, or
    ///     `RUNNER_CREW_ID` env vars. The bundled `runner` CLI is also
    ///     deliberately NOT on PATH for direct chats: direct sessions
    ///     have no event log or crew/mission identity for the local
    ///     `runner msg`, `runner signal`, and `runner ask` paths. Direct
    ///     chats are off-bus.
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
    /// `None` spawns the role's own runtime unchanged; a differing
    /// registry runtime spawns that engine with registry command /
    /// default args while the role's persona fields carry over.
    /// `model_override` / `effort_override` apply after runtime
    /// resolution and can also customize the role's own engine.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_direct(
        self: &Arc<Self>,
        role: &Role,
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
        self.spawn_direct_with_speed(
            role,
            runtime_override,
            model_override,
            effort_override,
            None,
            project_id,
            cwd,
            cols,
            rows,
            app_data_dir,
            pool,
            events,
            first_turn,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_direct_with_speed(
        self: &Arc<Self>,
        role: &Role,
        runtime_override: Option<&str>,
        model_override: Option<&str>,
        effort_override: Option<&str>,
        speed_override: Option<CodexSpeed>,
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
            role,
            runtime_override,
            model_override,
            effort_override,
            speed_override,
            Some(role.id.as_str()),
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
        role: &Role,
        project_id: Option<&str>,
        cwd: Option<&str>,
        cols: Option<u16>,
        rows: Option<u16>,
        app_data_dir: &Path,
        pool: Arc<DbPool>,
        events: Arc<dyn SessionEvents>,
    ) -> Result<SpawnedSession> {
        self.spawn_direct_inner(
            role,
            None,
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
        role: &Role,
        runtime_override: Option<&str>,
        model_override: Option<&str>,
        effort_override: Option<&str>,
        speed_override: Option<CodexSpeed>,
        persisted_role_id: Option<&str>,
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
        let effective =
            resolve_runtime_override(role, runtime_override, model_override, effort_override)?;
        let mut role = effective.unwrap_or_else(|| role.clone());
        let speed_override = (crate::runtimes::for_key(&role.runtime)
            .capabilities()
            .codex_speed)
            .then_some(speed_override)
            .flatten();
        role.codex_speed = speed_override.or(role.codex_speed);
        let mut launch_definition = crate::session::launch_definition::from_effective(&role);
        let mut role = self.resolve_role_executable(&role, &pool)?;
        role.args = crate::runtimes::for_key(&role.runtime)
            .permissions()
            .strip(&role.args);

        launch_definition.claude_account = crate::claude_accounts::launch_account(
            &pool,
            &mut role,
            &self.shell_env.read().unwrap().vars,
            None,
            true,
        )?;
        if let Some(account) = &mut launch_definition.claude_account {
            let options = crate::db::runtime_launch_options(&pool)?
                .get(&role.runtime)
                .copied()
                .unwrap_or_default();
            let effective = crate::runtimes::apply_launch_options(&role, false, options);
            account.permission_mode = crate::runtimes::for_key(&role.runtime)
                .native_permission_mode(&effective.args)
                .expect("account runtime has a native permission mode");
        }

        // Agent-native session resume: `spawn_direct` always opens a *new*
        // chat. The runtime adapter self-assigns a fresh
        // `agent_session_key` (claude-code) or leaves it NULL (codex).
        let plan = crate::runtimes::for_key(&role.runtime).resume_plan(None);

        // Reject explicitly missing folders before portable-pty can substitute another cwd.
        let resolved_cwd = resolve_spawn_cwd(cwd, role.working_dir.as_deref());
        let Some(chat_cwd) = resolved_cwd.as_deref() else {
            return Err(Error::msg(
                "home directory is unavailable; select a working directory before starting a chat",
            ));
        };
        if !std::path::Path::new(chat_cwd).is_dir() {
            return Err(Error::msg(format!(
                "working directory does not exist: {chat_cwd}"
            )));
        }

        // Direct chats are off-bus: RUNNER_HANDLE is the role template's
        // own handle, no slot/mission env vars.
        let mut direct_env: BTreeMap<String, String> = BTreeMap::new();
        if !Runtime::parse(&role.runtime).is_some_and(Runtime::is_shell) {
            direct_env.insert("RUNNER_HANDLE".into(), role.handle.clone());
        }

        let initial_size = Some(cols.zip(rows).unwrap_or(DEFAULT_PTY_SIZE));

        let session_id = ulid::Ulid::new().to_string();
        let _pre_attachment = PreAttachmentState {
            manager: Arc::downgrade(self),
            session_id: session_id.clone(),
        };
        let (system_prompt, first_turn) = crate::runtimes::for_key(&role.runtime)
            .prompt_channels()
            .split(router::prompt::SessionPromptKind::Direct, first_turn);
        let system_prompt_path = system_prompt
            .as_deref()
            .map(|body| crate::session::system_prompt::write(app_data_dir, &session_id, body))
            .transpose()?;
        let (first_turn, capture_prompt_marker) =
            Self::capture_prompt_marker(&role.runtime, &session_id, first_turn);
        let started_at_dt = Utc::now();
        let started_at = started_at_dt.to_rfc3339();

        let mut spec = self.base_spawn_spec(
            session_id.clone(),
            &role,
            resolved_cwd.clone(),
            false,
            None, // shim_dir — off-bus
            Some(app_data_dir.join("bin")),
            initial_size,
            direct_env,
        );
        let first_turn_delivered_via_argv = self.apply_runtime_args(
            &mut spec,
            &role,
            &plan,
            app_data_dir,
            system_prompt_path.as_deref(),
            first_turn.as_deref(),
            None,
            Some(&pool),
        );
        crate::claude_accounts::sharing::launch(
            &pool,
            launch_definition.claude_account.as_ref(),
            &role,
            &mut spec,
        )?;

        // Insert the row first so a fast-failing spawn doesn't leave
        // a half-row. Every chat carries its spawned engine on the row
        // via agent_runtime / agent_command: runtime-only chats have no
        // role template to rebuild from, and role-backed chats keep
        // their identity and resume engine even if the role template's
        // runtime is edited later.
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
                row.project_id = project_id.map(str::to_string);
                row.role_id = persisted_role_id.map(str::to_string);
                row.cwd = resolved_cwd.clone();
                row.started_at = Some(started_at_dt);
                row.agent_session_key = effect.key.clone();
                row.last_cols = initial_size.map(|(cols, _)| cols);
                row.last_rows = initial_size.map(|(_, rows)| rows);
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

        // Same gate as the mission spawn path — direct chats are
        // also fresh claude-code spawns and proactively refresh the
        // OAuth token, so a rapid burst of new chats can race. See
        // `enter_launch_gate` + issue #171.
        self.enter_launch_gate(&session_id, &role.runtime);

        // Post-gate row check: `role_delete` can remove the role's
        // sessions while we were asleep in the gate. The
        // session row is gone; spawning a PTY now would attach to
        // nothing.
        if !Self::session_row_exists(&pool, &session_id) {
            crate::session::system_prompt::remove(app_data_dir, &session_id);
            return Err(Error::msg(format!(
                "direct-chat session {session_id} row vanished before spawn — role deleted?"
            )));
        }

        let spawn_started_at_dt = Utc::now();
        #[cfg(windows)]
        let first_turn_deadline = Instant::now() + WINDOWS_FIRST_TURN_TIMEOUT;
        self.seed_runtime_project_trust(
            &session_id,
            &role.runtime,
            spec.cwd.as_deref(),
            role.env.get("COPILOT_HOME").map(String::as_str),
            crate::runtimes::claude_code::spawned_config_dir(&spec.env),
        );
        let starts_trusted = first_turn.is_none()
            && crate::runtimes::for_key(&role.runtime).starts_trusted(
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
                let _ = delete_session_row(&pool, &session_id);
                crate::session::system_prompt::remove(app_data_dir, &session_id);
                return Err(Error::msg(format!("spawn {}: {e}", role.command)));
            }
        };

        // Post-spawn row check: `role_delete` can also fire while
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
            crate::session::system_prompt::remove(app_data_dir, &session_id);
            return Err(Error::msg(format!(
                "direct-chat session {session_id} row vanished mid-spawn — role deleted?"
            )));
        }

        let spawn_pid = self.runtime_pid(&rt_session);

        if let Ok(conn) = pool.get() {
            let _ =
                crate::repo::session::record_process(&conn, &session_id, spawn_pid, &role.command);
        }

        let rollout_capture = if plan.assigned_key.is_none() {
            capture_root.and_then(|sessions_root| {
                resolved_cwd.clone().map(|cwd| RolloutCaptureContext {
                    runtime: role.runtime.clone(),
                    manager: Arc::downgrade(self),
                    mission_id: None,
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

        self.start_runtime_capture(
            &role,
            &session_id,
            None,
            resolved_cwd.clone(),
            plan.assigned_key.clone(),
            app_data_dir,
            &started_at,
            output.stop_flag(),
            &pool,
            &events,
        );
        self.install_handle(
            &session_id,
            SessionHandle {
                // A report to the chat waits until the agent is ready (#748).
                startup: (!Runtime::parse(&role.runtime).is_some_and(Runtime::is_shell)).then(
                    || {
                        let gate = if starts_trusted {
                            startup::StartupDelivery::for_trusted_reports
                        } else {
                            startup::StartupDelivery::for_reports
                        };
                        gate(
                            &role.runtime,
                            first_turn.is_some(),
                            super::super::clock::now(),
                        )
                        .with_status_hooks(status_hooks_enabled)
                    },
                ),
                #[cfg(windows)]
                pending_first_turn: None,
                id: session_id.clone(),
                mission_id: None,
                role_id: persisted_role_id.map(str::to_string),
                runtime_session: rt_session.clone(),
                rollout_capture: rollout_capture.clone(),
                forwarder: None,
                stop: output.stop_flag(),
            },
            None,
            initial_size,
            &pool,
            &events,
        );
        if first_turn_delivered_via_argv {
            self.arm_completion(&session_id);
        }
        self.publish_direct_activity(
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
            started_at,
            None,
            rt_session,
            output,
            Arc::clone(&pool),
            Arc::clone(&events),
            role.clone(),
            None,
            emit_activity,
            None, // direct chats are off-bus — no log to append session_status to
            app_data_dir.to_path_buf(),
        );
        self.install_forwarder(&session_id, forwarder);

        if let Some(ctx) = rollout_capture.as_ref() {
            self.spawn_rollout_capture_if_unkeyed(&session_id, ctx);
        }

        if emit_activity {
            emit_role_activity(&pool, &role, events.as_ref());
        }
        let missing_first_turn = Runtime::parse(role.runtime.as_str())
            .is_some_and(|runtime| !runtime.is_shell())
            && !plan.resuming
            && first_turn.is_some()
            && !first_turn_delivered_via_argv;
        #[cfg(windows)]
        let missing_first_turn =
            missing_first_turn && !crate::session::launch::is_windows_batch(&role.command);
        if missing_first_turn {
            log::warn!(
                "first-turn argv not delivered for direct chat {session_id} (runtime {}); skipping post-spawn injection",
                role.runtime,
            );
        }

        Ok(SpawnedSession {
            id: session_id,
            mission_id: None,
            role_id: persisted_role_id.map(str::to_string),
            handle: role.handle.clone(),
            pid: None,
        })
    }
}
