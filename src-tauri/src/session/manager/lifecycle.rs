use super::*;

impl SessionManager {
    fn runtime_session_matches(a: &RuntimeSession, b: &RuntimeSession) -> bool {
        a.runtime == b.runtime && a.session_id == b.session_id
    }

    /// Kill the child and wait for the reader thread to reap it.
    ///
    /// Sequence:
    ///   1. Mark the session intentionally killed and clone the runtime handle.
    ///   2. Ask the runtime to stop the child. If this fails, leave the live
    ///      handle installed so the caller can retry.
    ///   3. After `runtime.stop` succeeds, clear only the live handle and flip
    ///      the forwarder's stop flag.
    ///   4. Join the forwarder thread. It waits the child, updates the DB row
    ///      to stopped/crashed, emits `session/exit`. Only after this
    ///      returns is the caller allowed to consider the session dead —
    ///      which is what `mission_stop` needs in order to flip the mission
    ///      row without lying about termination.
    pub fn kill(&self, session_id: &str) -> Result<()> {
        // Mark the kill as intentional so the forwarder thread
        // classifies the upcoming non-zero exit as `stopped`, not
        // `crashed`. We roll this back below if `runtime.stop`
        // fails so a future successful kill applies cleanly.
        //
        // Look up the rt_session WITHOUT removing the handle yet.
        // The handle stays present until we know
        // `runtime.stop` succeeded. If it fails (child survived the
        // stop request), bailing here leaves the live handle intact
        // and the caller can retry; if we'd already removed the
        // handle + flipped the cancellation flag, the forwarder
        // thread would reconcile the DB row to `stopped` even
        // though the child is still alive.
        let Some(state) = self.session_state(session_id) else {
            return Ok(());
        };
        let rt_session = {
            let mut state = state.lock().unwrap();
            match state.handle.as_ref().map(|h| h.runtime_session.clone()) {
                Some(rt_session) => {
                    state.killed = true;
                    rt_session
                }
                None => {
                    // Already gone.
                    return Ok(());
                }
            }
        };

        // Stop verifies that the child was signaled. Returns Err if
        // the runtime refuses to reap it.
        if let Err(e) = self.runtime.stop(&rt_session) {
            // Roll back: child is alive, the handle stays
            // in the map, the killed marker is cleared. The
            // caller sees the error.
            state.lock().unwrap().killed = false;
            return Err(e.into());
        }

        // Stop succeeded. Now tear down the handle and reconcile.
        let gate = state.lock().unwrap().delivery_gate.clone();
        let (stop, forwarder) = {
            let mut delivery = gate.state.lock().unwrap();
            let mut state = state.lock().unwrap();
            match state.handle.take() {
                Some(mut h) => {
                    delivery.generation = delivery.generation.wrapping_add(1);
                    delivery.in_flight = false;
                    delivery.next_ticket = 0;
                    delivery.next_served = 0;
                    gate.ready.notify_all();
                    state.activity = None;
                    state.suppress_local_input_busy = false;
                    state.local_input_pending = false;
                    state.last_local_input_at = None;
                    state.mission_status_sink = None;
                    state.completion_armed = false;
                    (h.stop.clone(), h.forwarder.take())
                }
                None => return Ok(()), // raced with another caller; no-op
            }
        };
        self.notify_delivery_event(session_id, router::SessionDeliveryEvent::Exited);

        // Flip the explicit cancellation flag so the consumer
        // breaks out within ~500ms regardless of how the reader EOF
        // and channel-disconnect path progresses.
        stop.store(true, std::sync::atomic::Ordering::SeqCst);

        // Wait for the forwarder to drain + reconcile so the
        // caller (mission_stop) gets the no-live-sessions-after-
        // we-return contract.
        if let Some(h) = forwarder {
            let _ = h.join();
        }
        self.clear_killed(session_id);
        Ok(())
    }

    /// Register a fresh cancellation flag for a mission's background
    /// PTY-spawn task. Called from `mission_start` / `mission_reset`
    /// before dispatching `complete_mission_session_spawn`. Returns
    /// the shared flag the dispatcher and the background task both
    /// hold; setting it (via `cancel_pending_mission_spawns`) is
    /// what aborts queued slots.
    ///
    /// A prior flag for the same mission_id (left over from an
    /// earlier start/reset) is dropped: the new spawn batch
    /// supersedes any stale background task, and the old task's
    /// flag is no longer reachable from anywhere except its own
    /// closure — when it gets cancelled it'll observe the new flag
    /// only via the per-iteration DB lookup, which is fine because
    /// `mission_reset` archives the old session rows up front.
    pub fn register_pending_mission_cancel(&self, mission_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.pending_mission_cancels
            .lock()
            .unwrap()
            .insert(mission_id.to_string(), Arc::clone(&flag));
        flag
    }

    /// Clear the cancellation flag for a mission once its background
    /// spawn task drains. Per-batch identity-checked: only the task
    /// that owns `expected` may unregister it. Without this guard, a
    /// slow draining task could outlive a subsequent `mission_reset`
    /// and remove the *new* batch's flag — leaving the next
    /// Stop/Archive/Reset with no flag to flip and pending slots
    /// uncancellable. Callers pass an `Arc::clone(&cancel)` of the
    /// flag they received from `register_pending_mission_cancel`.
    pub fn drop_pending_mission_cancel(&self, mission_id: &str, expected: &Arc<AtomicBool>) {
        let mut map = self.pending_mission_cancels.lock().unwrap();
        if let Some(current) = map.get(mission_id) {
            if Arc::ptr_eq(current, expected) {
                map.remove(mission_id);
            }
        }
    }

    /// Flip the cancellation flag for `mission_id` (if one is
    /// registered) so any queued background spawns observe it and
    /// abort before their PTYs come up. Safe to call when no flag
    /// is registered. Invoked from `kill_all_for_mission`, which is
    /// in the call path of `mission_stop`, `mission_archive`, and
    /// `mission_reset`.
    pub fn cancel_pending_mission_spawns(&self, mission_id: &str) {
        if let Some(flag) = self.pending_mission_cancels.lock().unwrap().get(mission_id) {
            flag.store(true, Ordering::Release);
        }
    }

    /// Kill every live session; used on mission_stop and at app shutdown.
    /// Returns only after all reader threads have joined — callers rely on
    /// that for the "no live sessions after we return" contract.
    ///
    /// Also flips the per-mission cancellation flag so any pending
    /// `complete_mission_session_spawn` tasks abort before their PTYs
    /// come up — without this, slots that were still asleep in the
    /// claude-code launch gate would spawn into a now-stopped mission.
    pub fn kill_all_for_mission(&self, mission_id: &str) -> Result<()> {
        self.cancel_pending_mission_spawns(mission_id);
        let ids: Vec<String> = {
            let sessions = self.sessions.lock().unwrap();
            sessions
                .values()
                .filter_map(|state| {
                    let state = state.lock().unwrap();
                    let handle = state.handle.as_ref()?;
                    (handle.mission_id.as_deref() == Some(mission_id)).then(|| handle.id.clone())
                })
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
                .filter_map(|state| {
                    let state = state.lock().unwrap();
                    let handle = state.handle.as_ref()?;
                    (handle.runner_id.as_deref() == Some(runner_id)).then(|| handle.id.clone())
                })
                .collect()
        };
        for id in ids {
            self.kill(&id)?;
        }
        Ok(())
    }

    pub(super) fn forget_runtime_handle(
        &self,
        session_id: &str,
        runtime_session: &RuntimeSession,
    ) -> Result<()> {
        // Only the live PTY handle is dropped here. We deliberately keep
        // retained output and seq state alive so that:
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
        if let Some(state) = self.session_state(session_id) {
            let gate = state.lock().unwrap().delivery_gate.clone();
            let mut delivery = gate.state.lock().unwrap();
            let mut state = state.lock().unwrap();
            if state
                .handle
                .as_ref()
                .is_some_and(|h| Self::runtime_session_matches(&h.runtime_session, runtime_session))
            {
                delivery.generation = delivery.generation.wrapping_add(1);
                delivery.in_flight = false;
                delivery.next_ticket = 0;
                delivery.next_served = 0;
                gate.ready.notify_all();
                state.handle = None;
                state.activity = None;
                state.suppress_local_input_busy = false;
                state.local_input_pending = false;
                state.last_local_input_at = None;
                state.mission_status_sink = None;
                state.completion_armed = false;
                drop(state);
                drop(delivery);
                self.notify_delivery_event(session_id, router::SessionDeliveryEvent::Exited);
            }
        }
        self.prune_empty_session_state(session_id);
        Ok(())
    }
}
