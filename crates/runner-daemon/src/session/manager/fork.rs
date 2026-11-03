use super::*;
use runner_core::protocol::socket::FORK_MATERIALIZE_TIMEOUT;

pub(super) fn run_headless_fork(
    spec: &SpawnSpec,
    plan: &router::runtime::ForkPlan,
    timeout: Duration,
) -> Result<String> {
    spec.agent_runtime
        .map(crate::runtimes::adapter)
        .unwrap_or(&crate::runtimes::NoAgent)
        .materialize_fork(spec, plan, timeout)
}

fn delete_failed_fork(pool: &DbPool, session_id: &str) -> Result<()> {
    let mut conn = pool.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    crate::repo::node::remove_session(&tx, session_id)?;
    crate::ops::session::delete_rows(&tx, &[session_id.to_owned()])?;
    tx.commit()?;
    Ok(())
}

impl SessionManager {
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

        let role_template = if let Some(role_id) = source.role_id.as_deref() {
            let conn = pool.get()?;
            Some(crate::ops::role::get(&conn, role_id)?)
        } else {
            None
        };
        let definition = source.launch_definition.as_ref().ok_or_else(|| {
            Error::msg(format!(
                "session {source_session_id} has no launch definition"
            ))
        })?;
        let effective_runtime = source.agent_runtime.as_deref().ok_or_else(|| {
            Error::msg(format!("session {source_session_id} has no agent_runtime"))
        })?;
        if !crate::runtimes::for_key(effective_runtime)
            .catalog()
            .is_some_and(|catalog| catalog.native_fork)
        {
            return Err(Error::msg(format!(
                "runtime {effective_runtime} does not support native fork"
            )));
        }
        let identity = match role_template {
            Some(role) => role,
            None => runtime_direct_role(effective_runtime, Some(&definition.command), None, None)?,
        };
        let role =
            crate::session::launch_definition::to_role(definition, effective_runtime, identity);
        let mut role = self.resolve_role_executable(&role, &pool)?;

        // A fork is still an attended chat, so strip permission flags
        // and leave every other arg alone.
        role.args = crate::runtimes::for_key(&role.runtime)
            .permissions()
            .strip(&role.args);

        let resolved_cwd = resolve_spawn_cwd(source.cwd.as_deref(), role.working_dir.as_deref());
        let Some(chat_cwd) = resolved_cwd.as_deref() else {
            return Err(Error::msg(
                "home directory is unavailable; select a working directory before forking a chat",
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
                if source.role_id.is_some() {
                    format!("@{}", role.handle)
                } else {
                    role.display_name.clone()
                }
            });
        let source_pin =
            crate::claude_accounts::config_pin(&role, &self.shell_env.read().unwrap().vars);
        let mut fork_definition = definition.clone();
        let previous_account = definition.claude_account.as_ref();
        let mut account = crate::claude_accounts::launch_account(
            &pool,
            &mut role,
            &self.shell_env.read().unwrap().vars,
            previous_account,
            true,
        )?;
        let mut plan = crate::runtimes::for_key(&role.runtime)
            .fork_plan(source_key, &source_label)
            .ok_or_else(|| {
                Error::msg(format!(
                    "could not build fork plan for runtime {}",
                    role.runtime
                ))
            })?;

        if let Some(account) = &mut account {
            let native_origin = runner_core::app_paths::home_dir().unwrap().join(".claude");
            let origin = previous_account
                .map(|previous| previous.origin_folder.as_str())
                .or(source_pin.as_deref())
                .unwrap_or_else(|| native_origin.to_str().unwrap());
            account.permission_mode = crate::claude_accounts::last_permission(
                origin,
                resolved_cwd.as_deref(),
                source_key,
                previous_account
                    .map(|previous| previous.permission_mode.as_str())
                    .unwrap_or("default"),
            );
            if let router::runtime::ForkPlan::Direct(plan) = &mut plan {
                crate::claude_accounts::resume_args(
                    &mut plan.args,
                    account,
                    origin,
                    resolved_cwd.as_deref(),
                    source_key,
                );
            }
        }
        fork_definition.claude_account = account;
        let mut direct_env = BTreeMap::new();
        if !Runtime::parse(&role.runtime).is_some_and(Runtime::is_shell) {
            direct_env.insert("RUNNER_HANDLE".into(), role.handle.clone());
        }
        let initial_size = Some(cols.zip(rows).unwrap_or(DEFAULT_PTY_SIZE));
        let session_id = ulid::Ulid::new().to_string();
        let _pre_attachment = PreAttachmentState {
            manager: Arc::downgrade(self),
            session_id: session_id.clone(),
        };
        let started_at_dt = Utc::now();
        let started_at = started_at_dt.to_rfc3339();
        let mut spec = self.base_spawn_spec(
            session_id.clone(),
            &role,
            resolved_cwd.clone(),
            false,
            None,
            Some(app_data_dir.join("bin")),
            initial_size,
            direct_env,
        );

        let assigned_key = match &plan {
            router::runtime::ForkPlan::Direct(plan) => plan.assigned_key.clone(),
            router::runtime::ForkPlan::Headless { .. } => None,
        };
        self.report_key(
            &session_id,
            PersistKey {
                key: assigned_key,
                generation: started_at.clone(),
                origin: KeyOrigin::Assigned,
            },
            |effect| {
                let conn = pool.get()?;
                let mut row = crate::repo::session::SessionRowDb::new_running(session_id.clone());
                row.project_id.clone_from(&source.project_id);
                row.role_id.clone_from(&source.role_id);
                row.cwd = resolved_cwd.clone();
                row.started_at = Some(started_at_dt);
                row.agent_session_key = effect.key.clone();
                row.title = title.and_then(|title| {
                    let title = title.trim();
                    (!title.is_empty()).then(|| title.to_string())
                });
                row.agent_runtime.clone_from(&source.agent_runtime);
                row.agent_command.clone_from(&source.agent_command);
                row.launch_definition = Some(fork_definition.clone());
                row.last_cols = initial_size.map(|(cols, _)| cols);
                row.last_rows = initial_size.map(|(_, rows)| rows);
                crate::repo::session::insert(&conn, &row)?;
                Ok(true)
            },
        )?;
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
        self.enter_launch_gate(&session_id, &role.runtime);
        let gate_elapsed = gate_started_at.elapsed();
        if !Self::session_row_exists(&pool, &session_id) {
            return Err(Error::msg(format!(
                "forked session {session_id} row vanished before spawn — role deleted?"
            )));
        }

        match plan {
            router::runtime::ForkPlan::Direct(plan) => {
                let system_prompt = if crate::runtimes::for_key(&role.runtime)
                    .prompt_channels()
                    .system_prompt
                {
                    router::prompt::compose_direct_first_turn(role.system_prompt.as_deref())
                } else {
                    None
                };
                let system_prompt_path = match system_prompt
                    .as_deref()
                    .map(|body| {
                        crate::session::system_prompt::write(app_data_dir, &session_id, body)
                    })
                    .transpose()
                {
                    Ok(path) => path,
                    Err(error) => {
                        let _ = delete_failed_fork(&pool, &session_id);
                        events.updated(&SessionUpdatedEvent {
                            session_id: session_id.clone(),
                            mission_id: None,
                        });
                        return Err(error);
                    }
                };
                let _ = self.apply_runtime_args(
                    &mut spec,
                    &role,
                    &plan,
                    app_data_dir,
                    system_prompt_path.as_deref(),
                    None,
                    None,
                    Some(&pool),
                );
                if let Err(error) = crate::claude_accounts::sharing::launch(
                    &pool,
                    fork_definition.claude_account.as_ref(),
                    &role,
                    &mut spec,
                ) {
                    let _ = delete_failed_fork(&pool, &session_id);
                    events.updated(&SessionUpdatedEvent {
                        session_id: session_id.clone(),
                        mission_id: None,
                    });
                    return Err(error);
                }
                let direct_spawn_started_at = Instant::now();
                let spawning = self.shutdown.read().unwrap();
                if *spawning {
                    return Err(Error::msg("runnerd is stopping"));
                }
                let (rt_session, output) = match self.spawn_runtime(spec) {
                    Ok(spawned) => spawned,
                    Err(error) => {
                        log::info!(
                            "fork timing: session={session_id} runtime={} gate_ms={} materialize_ms=not-applicable direct_spawn_ms={} total_ms={} outcome=spawn-error",
                            role.runtime,
                            gate_elapsed.as_millis(),
                            direct_spawn_started_at.elapsed().as_millis(),
                            fork_started_at.elapsed().as_millis(),
                        );
                        if let Err(cleanup_error) = delete_failed_fork(&pool, &session_id) {
                            return Err(Error::msg(format!(
                                "spawn {}: {error}; failed to roll back forked session {session_id}: {cleanup_error}",
                                role.command,
                            )));
                        }
                        crate::session::system_prompt::remove(app_data_dir, &session_id);
                        events.updated(&SessionUpdatedEvent {
                            session_id: session_id.clone(),
                            mission_id: None,
                        });
                        return Err(Error::msg(format!("spawn {}: {error}", role.command)));
                    }
                };

                if !Self::session_row_exists(&pool, &session_id) {
                    if let Err(error) = self.runtime.stop(&rt_session) {
                        log::warn!(
                            "failed to stop just-spawned fork PTY for vanished session {session_id}: {error}"
                        );
                    }
                    let _ = delete_failed_fork(&pool, &session_id);
                    crate::session::system_prompt::remove(app_data_dir, &session_id);
                    events.updated(&SessionUpdatedEvent {
                        session_id: session_id.clone(),
                        mission_id: None,
                    });
                    return Err(Error::msg(format!(
                        "forked session {session_id} row vanished mid-spawn — role deleted?"
                    )));
                }

                let spawn_pid = self.runtime_pid(&rt_session);
                if let Ok(conn) = pool.get() {
                    let _ = crate::repo::session::record_process(
                        &conn,
                        &session_id,
                        spawn_pid,
                        &role.command,
                    );
                }

                self.install_handle(
                    &session_id,
                    SessionHandle {
                        // A report to the chat waits until the agent is ready (#748).
                        startup: (!Runtime::parse(&role.runtime).is_some_and(Runtime::is_shell))
                            .then(|| {
                                startup::StartupDelivery::for_trusted_reports(
                                    &role.runtime,
                                    false,
                                    super::super::clock::now(),
                                )
                            }),
                        #[cfg(windows)]
                        pending_first_turn: None,
                        id: session_id.clone(),
                        mission_id: None,
                        role_id: source.role_id.clone(),
                        runtime_session: rt_session.clone(),
                        rollout_capture: None,
                        forwarder: None,
                        stop: output.stop_flag(),
                    },
                    None,
                    initial_size,
                    &pool,
                    &events,
                );
                self.publish_direct_activity(
                    &session_id,
                    SessionActivityState::Busy,
                    StatusSource::Fork,
                    events.as_ref(),
                );
                let resume_check = plan
                    .assigned_key
                    .clone()
                    .filter(|_| plan.resuming)
                    .map(|key| {
                        let runtime = role.runtime.clone();
                        let account_folder = fork_definition
                            .claude_account
                            .as_ref()
                            .map(|account| account.origin_folder.clone());
                        let cwd = resolved_cwd.clone();
                        let env = role.env.clone();
                        Box::new(move || {
                            super::output::conversation_exists(
                                crate::runtimes::for_key(&runtime),
                                account_folder.as_deref(),
                                cwd.as_deref(),
                                &env,
                                &key,
                            )
                        }) as super::output::ConversationCheck
                    });
                let forwarder = self.start_forwarder_thread(
                    session_id.clone(),
                    started_at,
                    None,
                    rt_session,
                    output,
                    Arc::clone(&pool),
                    Arc::clone(&events),
                    role.clone(),
                    resume_check,
                    source.role_id.is_some(),
                    None,
                    app_data_dir.to_path_buf(),
                );
                self.install_forwarder(&session_id, forwarder);
                if source.role_id.is_some() {
                    emit_role_activity(&pool, &role, events.as_ref());
                }
                log::info!(
                    "fork timing: session={session_id} runtime={} gate_ms={} materialize_ms=not-applicable direct_spawn_ms={} total_ms={} outcome=ok",
                    role.runtime,
                    gate_elapsed.as_millis(),
                    direct_spawn_started_at.elapsed().as_millis(),
                    fork_started_at.elapsed().as_millis(),
                );
                Ok(SpawnedSession {
                    id: session_id,
                    mission_id: None,
                    role_id: source.role_id,
                    handle: role.handle,
                    pid: None,
                })
            }
            headless @ router::runtime::ForkPlan::Headless { .. } => {
                self.seed_runtime_project_trust(
                    &session_id,
                    &role.runtime,
                    spec.cwd.as_deref(),
                    role.env.get("COPILOT_HOME").map(String::as_str),
                );
                let materialize_started_at = Instant::now();
                let fork_key = match run_headless_fork(&spec, &headless, FORK_MATERIALIZE_TIMEOUT) {
                    Ok(key) => key,
                    Err(error) => {
                        log::info!(
                            "fork timing: session={session_id} runtime={} gate_ms={} materialize_ms={} total_ms={} outcome=materialize-error",
                            role.runtime,
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

                let persist_result = self.report_key(
                    &session_id,
                    PersistKey {
                        key: Some(fork_key.clone()),
                        generation: started_at.clone(),
                        origin: KeyOrigin::Captured,
                    },
                    |effect| {
                        let conn = pool.get()?;
                        let captured = Self::write_key_effect(&conn, &session_id, effect)?;
                        let stopped = crate::repo::session::set_exit_status(
                            &conn,
                            &session_id,
                            crate::model::SessionStatus::Stopped,
                            Utc::now(),
                        )?;
                        Ok(captured && stopped > 0)
                    },
                );
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
                    role.runtime,
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
}
