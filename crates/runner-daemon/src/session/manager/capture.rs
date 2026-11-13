use super::*;

impl SessionManager {
    pub(super) fn capture_prompt_marker(
        runtime_key: &str,
        session_id: &str,
        first_turn: Option<String>,
    ) -> (Option<String>, Option<String>) {
        crate::runtimes::for_key(runtime_key).capture_prompt(session_id, first_turn)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_runtime_capture(
        self: &Arc<Self>,
        role: &Role,
        session_id: &str,
        mission_id: Option<String>,
        cwd: Option<String>,
        initial_key: Option<String>,
        app_data_dir: &Path,
        row_started_at: &str,
        stop: Arc<AtomicBool>,
        pool: &Arc<DbPool>,
        events: &Arc<dyn SessionEvents>,
    ) {
        crate::runtimes::for_key(&role.runtime).start_capture(crate::runtimes::CaptureContext {
            manager: Arc::downgrade(self),
            session_id: session_id.to_owned(),
            mission_id,
            role_args: role.args.clone(),
            role_env: role.env.clone(),
            cwd,
            initial_key,
            app_data_dir: app_data_dir.to_path_buf(),
            row_started_at: row_started_at.to_owned(),
            stop,
            pool: Arc::clone(pool),
            events: Arc::clone(events),
        });
    }

    #[cfg(windows)]
    pub(super) fn queue_windows_batch_first_turn(
        &self,
        session_id: &str,
        role: &Role,
        plan: &router::runtime::ResumePlan,
        first_turn: Option<&str>,
        deadline: Instant,
    ) {
        if Runtime::parse(role.runtime.as_str()).is_some_and(|runtime| !runtime.is_shell())
            && !plan.resuming
            && crate::session::launch::is_windows_batch(&role.command)
        {
            if let Some(body) = first_turn.filter(|body| !body.trim().is_empty()) {
                let state = self.session_state_or_insert(session_id);
                let mut state = state.lock().unwrap();
                if let Some(handle) = state.handle.as_mut() {
                    handle.pending_first_turn = Some(PendingFirstTurn {
                        body: body.to_owned(),
                        deadline,
                        readiness: Default::default(),
                    });
                } else {
                    log::warn!(
                        "cannot queue first turn for {session_id}: session handle is missing"
                    );
                }
            }
        }
    }
}

impl SessionManager {
    pub(super) fn rollout_capture_context(
        &self,
        session_id: &str,
    ) -> Option<RolloutCaptureContext> {
        let state = self.session_state(session_id)?;
        let state = state.lock().unwrap();
        state
            .handle
            .as_ref()
            .and_then(|handle| handle.rollout_capture.clone())
    }

    pub(super) fn spawn_rollout_capture_if_unkeyed(
        &self,
        session_id: &str,
        ctx: &RolloutCaptureContext,
    ) {
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
        crate::runtimes::for_key(&ctx.runtime).start_rollout_capture(
            crate::runtimes::RolloutCaptureRequest {
                manager: ctx.manager.clone(),
                session_id: session_id.to_string(),
                mission_id: ctx.mission_id.clone(),
                sessions_root: ctx.sessions_root.clone(),
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
}
