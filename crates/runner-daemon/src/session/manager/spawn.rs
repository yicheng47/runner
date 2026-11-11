use super::*;

#[cfg(test)]
pub(super) use super::spawn_spec::{agent_env, ensure_utf8_locale};

pub(super) fn delete_session_row(pool: &DbPool, session_id: &str) -> Result<()> {
    let mut conn = pool.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    crate::ops::session::delete_rows(&tx, &[session_id.to_owned()])?;
    tx.commit()?;
    Ok(())
}

impl SessionManager {
    /// Runs a PTY that is not a session: no `sessions` row, no mission or
    /// role, no busy/idle inference. Clients attach to its terminal by id;
    /// it never reaches the chat surface, the sidebar, the archive,
    /// relaunch or any session list. Input, resize and kill work by id like
    /// any live session until the process exits.
    pub fn spawn_unlisted(
        self: &Arc<Self>,
        spec: SpawnSpec,
        pool: &DbPool,
        events: Arc<dyn SessionEvents>,
    ) -> Result<std::sync::mpsc::Receiver<Option<i32>>> {
        let session_id = spec.session_id.clone();
        let initial_size = spec.initial_size;
        let spawning = self.shutdown.read().unwrap();
        if *spawning {
            return Err(Error::msg("runnerd is stopping"));
        }
        let (rt_session, output) = self.spawn_runtime(spec)?;
        let stop = output.stop_flag();
        self.install_handle(
            &session_id,
            SessionHandle {
                startup: None,
                #[cfg(windows)]
                pending_first_turn: None,
                id: session_id.clone(),
                mission_id: None,
                role_id: None,
                runtime_session: rt_session.clone(),
                rollout_capture: None,
                forwarder: None,
                stop: Arc::clone(&stop),
            },
            None,
            initial_size,
            pool,
            &events,
        );
        let (completed, completion) = std::sync::mpsc::channel();
        let manager = Arc::clone(self);
        let forwarder_id = session_id.clone();
        let forwarder = thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match output.recv_timeout(Duration::from_millis(500)) {
                    Ok(RuntimeOutput::Stream(bytes)) => manager.ingest_output_for_launch(
                        &forwarder_id,
                        None,
                        &bytes,
                        events.as_ref(),
                        Some(&stop),
                    ),
                    Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            let exit_code = manager
                .runtime
                .status(&rt_session)
                .ok()
                .flatten()
                .and_then(|status| status.exit_code);
            let _ = manager.runtime.stop(&rt_session);
            manager.take_killed(&forwarder_id);
            let _ = manager.forget_runtime_handle(&forwarder_id, &rt_session);
            // Finished update tasks keep their terminal until dismissed.
            let retained = manager
                .runtime_updates
                .lock()
                .unwrap()
                .tasks
                .iter()
                .any(|task| task.id == forwarder_id);
            if !retained {
                manager.sessions.lock().unwrap().remove(&forwarder_id);
            }
            events.exit(&ExitEvent {
                session_id: forwarder_id,
                mission_id: None,
                exit_code,
                success: exit_code == Some(0),
            });
            let _ = completed.send(exit_code);
        });
        self.install_forwarder(&session_id, forwarder);
        Ok(completion)
    }
}
