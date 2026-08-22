use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LocalInputClass {
    SetPending,
    ClearPending,
    ActivityOnly,
}

pub(super) fn classify_local_input(bytes: &[u8]) -> Option<LocalInputClass> {
    if bytes.is_empty() {
        return None;
    }
    if bytes == b"\r" || bytes == b"\x03" {
        return Some(LocalInputClass::ClearPending);
    }
    if bytes == b"\x16" || bytes.starts_with(b"\x1b[200~") {
        return Some(LocalInputClass::SetPending);
    }
    if bytes.starts_with(b"\x1b") {
        return Some(LocalInputClass::ActivityOnly);
    }
    if bytes
        .iter()
        .any(|byte| matches!(byte, 0x20..=0x7e | 0x80..=0xff))
    {
        Some(LocalInputClass::SetPending)
    } else {
        Some(LocalInputClass::ActivityOnly)
    }
}

pub(super) fn update_local_input_state(
    state: &mut SessionState,
    input_class: Option<LocalInputClass>,
    now: Instant,
) -> bool {
    match input_class {
        Some(LocalInputClass::SetPending) => {
            state.local_input_pending = true;
            state.last_local_input_at = Some(now);
            false
        }
        Some(LocalInputClass::ClearPending) => {
            state.local_input_pending = false;
            state.last_local_input_at = state
                .observed_input
                .is_some_and(|observed| observed.state == InputState::Idle)
                .then_some(now);
            true
        }
        Some(LocalInputClass::ActivityOnly) => {
            state.last_local_input_at = Some(now);
            false
        }
        None => false,
    }
}

impl SessionManager {
    /// Forwarder thread shared by `spawn`, `spawn_direct`, and `resume`.
    /// Drains the runtime's `OutputStream` into the terminal sink,
    /// then on channel close queries the runtime for the
    /// final exit code, flips the DB row, emits `session/exit`, and
    /// clears the live handle. `kill` joins this handle so
    /// `mission_stop` gets the no-lying-about-termination contract.
    // The thread genuinely needs every one of these — session_id /
    // mission_id for event payloads, runtime_session for status
    // queries, output for the input stream, pool for the DB row
    // update, events for emitter dispatch, runner for the
    // post-reap activity recompute, emit_ctx for the synthetic
    // runner_status events the forwarder appends to the mission's
    // event log (issue #124). Bundling into a Context struct just
    // moves the same arity to the call site without buying clarity.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_forwarder_thread(
        self: &Arc<Self>,
        session_id: String,
        mission_id: Option<String>,
        rt_session: RuntimeSession,
        output: OutputStream,
        pool: Arc<DbPool>,
        events: Arc<dyn SessionEvents>,
        runner: Runner,
        resuming: bool,
        emit_activity: bool,
        emit_ctx: Option<ForwarderEmitCtx>,
    ) -> thread::JoinHandle<()> {
        let manager_t: Arc<SessionManager> = Arc::clone(self);
        let started_at = std::time::Instant::now();
        // Capture the cancellation flag before moving `output` into
        // the thread. `kill` flips this flag so the consumer
        // breaks out within ~500ms even if the reader/EOF
        // disconnect path stalls.
        let stop = output.stop_flag();
        thread::spawn(move || {
            // Drain PTY output until the runtime closes the channel
            // OR `kill` flips the stop flag. Stream chunks flow to
            // the terminal sink. StatusTransition is routed
            // into either the mission event log or a direct-chat
            // live status event so the UI sees busy/idle flips.
            //
            // Failure bookkeeping for `runner_status` emission lives
            // here on the consumer's stack — single-threaded access,
            // no atomics. `drop_streak` resets on each successful
            // append; `drop_total` is a lifetime counter logged at
            // recovery.
            let mut drop_streak: u64 = 0;
            let mut drop_total: u64 = 0;
            loop {
                if stop.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                match output.recv_timeout(Duration::from_millis(500)) {
                    Ok(RuntimeOutput::Stream(bytes)) => {
                        manager_t.ingest_output_chunk(
                            &session_id,
                            mission_id.as_deref(),
                            &bytes,
                            events.as_ref(),
                        );
                    }
                    Ok(RuntimeOutput::StatusTransition { state, source }) => {
                        if let Some(ctx) = emit_ctx.as_ref() {
                            if !manager_t.note_forwarder_transition(
                                &session_id,
                                state.into(),
                                source,
                            ) {
                                continue;
                            }
                            let outcome = ctx.try_append_runner_status(state, source);
                            match outcome {
                                AppendOutcome::Ok => {
                                    if drop_streak > 0 {
                                        log::info!(
                                            "runner_status emit recovered for {session_id} \
                                             after {drop_streak} dropped events \
                                             ({drop_total} total this session)",
                                        );
                                    }
                                    drop_streak = 0;
                                }
                                AppendOutcome::Contended | AppendOutcome::Failed => {
                                    drop_streak += 1;
                                    drop_total += 1;
                                    if drop_streak_is_loggable(drop_streak) {
                                        log::error!(
                                            "runner_status emit failing for {session_id}; \
                                             {drop_streak} events dropped in a row \
                                             ({drop_total} total this session)",
                                        );
                                    }
                                }
                            }
                        } else {
                            manager_t.publish_direct_activity(
                                &session_id,
                                state.into(),
                                source,
                                events.as_ref(),
                            );
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }

            // Channel closed — query the runtime for the final child
            // status to recover an exit code. `Ok(None)` means the
            // runtime session is gone; we still need to flip the DB
            // row, just without an exit code.
            let status = manager_t.runtime.status(&rt_session).ok().flatten();
            let exit_code = status.as_ref().and_then(|s| s.exit_code);
            let success = exit_code == Some(0);

            // Best-effort: tear down the PTY child now that the
            // output channel closed. Skipped if `kill` already did it.
            let _ = manager_t.runtime.stop(&rt_session);

            let was_killed = manager_t.take_killed(&session_id);
            // Resume failure heuristic: prior conversation rejected
            // and the agent died fast.
            let resume_failed = resuming
                && !success
                && !was_killed
                && started_at.elapsed() < std::time::Duration::from_secs(3);
            let final_status = if success || was_killed {
                crate::model::SessionStatus::Stopped
            } else {
                crate::model::SessionStatus::Crashed
            };
            match pool.get() {
                Ok(conn) => {
                    let result = if resume_failed {
                        crate::repo::session::set_crashed_clearing_key(
                            &conn,
                            &session_id,
                            Utc::now(),
                        )
                    } else {
                        crate::repo::session::set_exit_status(
                            &conn,
                            &session_id,
                            final_status,
                            Utc::now(),
                        )
                    };
                    if let Err(error) = result {
                        log::warn!("session exit reconciliation failed for {session_id}: {error}");
                    }
                }
                Err(error) => {
                    log::warn!(
                        "session exit reconciliation pool checkout failed for {session_id}: {error}"
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
            if emit_activity {
                emit_runner_activity(&pool, &runner, events.as_ref());
            }
            events.exit(&ExitEvent {
                session_id: session_id.clone(),
                mission_id: mission_id.clone(),
                exit_code,
                success,
            });
            let _ = manager_t.forget_runtime_handle(&session_id, &rt_session);
            if !was_killed {
                if let Some(mission_id) = mission_id.as_deref() {
                    manager_t.reap_live_mission_siblings(mission_id, &session_id, &pool);
                }
            }
        })
    }

    /// Write raw bytes to the session's stdin. Used for keystroke
    /// passthrough from xterm.js — small chunks, no embedded
    /// newlines. Routed through `runtime.send_bytes` so each byte
    /// lands without bracketed-paste markers.
    ///
    /// Multi-line prompt blocks (the system_prompt injection on
    /// fresh spawn) should go through `inject_paste` instead so the
    /// agent's TUI sees them as one paste rather than 50
    /// keystrokes that might trigger an early submit on the first
    /// `\n`.
    pub fn inject_stdin(&self, session_id: &str, bytes: &[u8]) -> Result<()> {
        let rt_session = self.live_runtime_session(session_id)?;
        self.write_stdin(session_id, &rt_session, bytes)
    }

    pub fn inject_reserved(&self, session_id: &str, token: u64, bytes: &[u8]) -> Result<bool> {
        let Some(session) = self.session_state(session_id) else {
            return Ok(false);
        };
        let gate = session.lock().unwrap().delivery_gate.clone();
        let delivery = gate.state.lock().unwrap();
        let session = session.lock().unwrap();
        if !delivery.in_flight || delivery.generation != token {
            return Ok(false);
        }
        let Some(rt_session) = session
            .handle
            .as_ref()
            .map(|handle| handle.runtime_session.clone())
        else {
            return Ok(false);
        };
        drop(session);
        self.write_stdin_bytes(&rt_session, bytes)?;
        drop(delivery);
        if bytes == b"\r" {
            self.capture_codex_session_key(session_id);
        }
        Ok(true)
    }

    pub fn inject_direct_stdin(
        &self,
        session_id: &str,
        bytes: &[u8],
        events: &dyn SessionEvents,
    ) -> Result<()> {
        self.inject_direct_stdin_with_wait_timeout(
            session_id,
            bytes,
            events,
            DIRECT_INPUT_GATE_TIMEOUT,
        )
    }

    pub(super) fn inject_direct_stdin_with_wait_timeout(
        &self,
        session_id: &str,
        bytes: &[u8],
        events: &dyn SessionEvents,
        wait_timeout: Duration,
    ) -> Result<()> {
        let submitted = bytes == b"\r";
        let input_class = classify_local_input(bytes);
        let session = self
            .session_state(session_id)
            .ok_or_else(|| Error::msg(format!("session not found: {session_id}")))?;
        let gate = session.lock().unwrap().delivery_gate.clone();
        let mut delivery = gate.state.lock().unwrap();
        let generation = delivery.generation;
        let ticket = delivery.next_ticket;
        delivery.next_ticket = delivery.next_ticket.wrapping_add(1);
        let deadline = Instant::now() + wait_timeout;
        while delivery.generation == generation
            && (delivery.in_flight || delivery.next_served != ticket)
        {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                delivery.cancelled_tickets.insert(ticket);
                delivery.skip_cancelled_tickets();
                gate.ready.notify_all();
                return Err(Error::DirectInputTimeout {
                    session_id: session_id.to_string(),
                    timeout_ms: wait_timeout.as_millis() as u64,
                });
            }
            let (next, wait) = gate.ready.wait_timeout(delivery, remaining).unwrap();
            delivery = next;
            if wait.timed_out()
                && delivery.generation == generation
                && (delivery.in_flight || delivery.next_served != ticket)
            {
                delivery.cancelled_tickets.insert(ticket);
                delivery.skip_cancelled_tickets();
                gate.ready.notify_all();
                return Err(Error::DirectInputTimeout {
                    session_id: session_id.to_string(),
                    timeout_ms: wait_timeout.as_millis() as u64,
                });
            }
        }
        if delivery.generation != generation {
            return Err(Error::msg(format!(
                "session changed while input was queued: {session_id}"
            )));
        }

        let outcome = (|| {
            let mut session = session.lock().unwrap();
            let rt_session = session
                .handle
                .as_ref()
                .map(|handle| handle.runtime_session.clone())
                .ok_or_else(|| Error::msg(format!("session not found: {session_id}")))?;
            let previous_activity = session.activity;
            let previous_suppression = session.suppress_local_input_busy;
            let previous_input_pending = session.local_input_pending;
            let previous_input_at = session.last_local_input_at;
            let mission_status_sink = session.mission_status_sink.clone();
            let mission_scoped = session
                .handle
                .as_ref()
                .is_some_and(|handle| handle.mission_id.is_some());
            let transition = if previous_activity.is_some() && submitted {
                session.suppress_local_input_busy = false;
                if previous_activity == Some(SessionActivityState::Idle) {
                    session.activity = Some(SessionActivityState::Busy);
                    Some(SessionActivityEvent {
                        session_id: session_id.to_string(),
                        state: SessionActivityState::Busy,
                        source: "input-submit".to_string(),
                    })
                } else {
                    None
                }
            } else {
                if previous_activity == Some(SessionActivityState::Idle) {
                    session.suppress_local_input_busy = true;
                }
                None
            };
            let input_cleared = update_local_input_state(&mut session, input_class, Instant::now());
            if let Err(error) = self.write_stdin_bytes(&rt_session, bytes) {
                session.activity = previous_activity;
                session.suppress_local_input_busy = previous_suppression;
                session.local_input_pending = previous_input_pending;
                session.last_local_input_at = previous_input_at;
                return Err(error);
            }
            if transition.is_some() {
                session.activity_revision = session.activity_revision.wrapping_add(1);
            }
            if submitted {
                session.completion_armed = true;
            }
            Ok((
                transition,
                mission_status_sink,
                mission_scoped,
                input_cleared,
            ))
        })();

        delivery.next_served = delivery.next_served.wrapping_add(1);
        delivery.skip_cancelled_tickets();
        let input_queue_drained = delivery.next_served == delivery.next_ticket;
        let successful_clear = outcome
            .as_ref()
            .is_ok_and(|(_, _, _, input_cleared)| *input_cleared);
        gate.ready.notify_all();
        drop(delivery);
        if input_queue_drained && !successful_clear {
            self.notify_delivery_event(session_id, router::SessionDeliveryEvent::InputQueueDrained);
        }
        let (transition, mission_status_sink, mission_scoped, input_cleared) = outcome?;
        if submitted {
            self.capture_codex_session_key(session_id);
        }
        if input_cleared {
            self.notify_delivery_event(session_id, router::SessionDeliveryEvent::InputCleared);
        }
        if let Some(transition) = transition.as_ref() {
            if let Some(sink) = mission_status_sink.as_ref() {
                if let Err(error) = sink.append_runner_status(RunnerStatus::Busy, "input-submit") {
                    log::error!(
                        "append input-submit runner_status failed for {session_id}: {error}"
                    );
                }
            } else if !mission_scoped {
                events.status(transition);
            }
        }
        Ok(())
    }

    fn write_stdin(
        &self,
        session_id: &str,
        rt_session: &RuntimeSession,
        bytes: &[u8],
    ) -> Result<()> {
        self.write_stdin_bytes(rt_session, bytes)?;
        if bytes == b"\r" {
            self.capture_codex_session_key(session_id);
        }
        Ok(())
    }

    fn write_stdin_bytes(&self, rt_session: &RuntimeSession, bytes: &[u8]) -> Result<()> {
        // ASCII CR (0x0D) is what claude-code's TUI editor reads as
        // "Enter" — bare-byte writes that just contain `\r` map to
        // `send_key("Enter")`. Everything else routes as a literal
        // byte stream.
        if bytes == b"\r" {
            self.runtime
                .send_key(rt_session, "Enter")
                .map_err(Into::into)
        } else {
            self.runtime
                .send_bytes(rt_session, bytes)
                .map_err(Into::into)
        }
    }

    fn capture_codex_session_key(&self, session_id: &str) {
        if let Some(ctx) = self.codex_capture_context(session_id) {
            self.spawn_codex_capture_if_unkeyed(session_id, &ctx);
        }
    }

    /// Paste a multi-line prompt block into the session, then submit
    /// with Enter. This preserves the old runtime paste behavior:
    /// write the payload bytes unchanged, then send Enter.
    ///
    /// Sleeps 120ms between paste and Enter. Without this gap,
    /// Claude Code v2.1.x's input editor sometimes leaves pasted
    /// content sitting in the input box unsubmitted. `cfg(test)`
    /// keeps the same constant — fake runtimes complete instantly so
    /// the wait is harmless.
    pub fn inject_paste(&self, session_id: &str, payload: &[u8]) -> Result<()> {
        let rt_session = self.live_runtime_session(session_id)?;
        self.runtime.send_bytes(&rt_session, payload)?;
        std::thread::sleep(std::time::Duration::from_millis(120));
        let result = if let Some(session) = self.session_state(session_id) {
            let mut session = session.lock().unwrap();
            let result = self
                .runtime
                .send_key(&rt_session, "Enter")
                .map_err(Into::into);
            if result.is_ok() {
                session.completion_armed = true;
            }
            result
        } else {
            self.runtime
                .send_key(&rt_session, "Enter")
                .map_err(Into::into)
        };
        if result.is_ok() {
            if let Some(ctx) = self.codex_capture_context(session_id) {
                self.spawn_codex_capture_if_unkeyed(session_id, &ctx);
            }
        }
        result
    }

    /// Paste a first-turn body and submit it once we've verified the
    /// pane actually rendered the paste — covers the agent-readiness
    /// race that the bare `inject_paste` path leaves open
    /// (FIRST_PROMPT_DELAY blind wait isn't enough under contention).
    ///
    /// Loop shape: sleep `initial_wait`, take a baseline capture, then
    /// up to `max_attempts` rounds of paste → sleep `render_wait` →
    /// capture → if any of head/tail-marker delta or (body ≥
    /// `PLACEHOLDER_MIN_BODY_LEN`) placeholder delta ≥ 1 vs the
    /// Resize the session's pane. The frontend calls this after
    /// xterm fits its container — without it, claude-code stays at
    /// the spawn-time grid regardless of how big the visible grid
    /// is.
    pub fn resize(&self, session_id: &str, cols: u16, rows: u16, pool: &Arc<DbPool>) -> Result<()> {
        let state = self.session_state_or_insert(session_id);
        let settle_ms = self
            .resize_settle_ms
            .load(std::sync::atomic::Ordering::Relaxed);
        let (settle_generation, resize_result) = {
            let mut session = state.lock().unwrap();
            session.last_requested_size = Some((cols, rows));
            session.last_requested_size_dirty = true;

            let mut ioctl_count = 0;
            let resize_result = if session.killed || session.resuming {
                Ok(())
            } else if let Some(rt_session) = session
                .handle
                .as_ref()
                .map(|handle| handle.runtime_session.clone())
            {
                ioctl_count = 1;
                self.runtime
                    .resize(&rt_session, cols, rows)
                    .map_err(Into::into)
            } else {
                Ok(())
            };

            let settle_generation = match session.pending_resize.as_mut() {
                Some(pending) => {
                    pending.cols = cols;
                    pending.rows = rows;
                    pending.deadline = Instant::now() + Duration::from_millis(settle_ms);
                    pending.suppressed += 1;
                    pending.ioctl_count += ioctl_count;
                    None
                }
                None => {
                    let generation = self
                        .resize_generation
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                        .wrapping_add(1);
                    session.pending_resize = Some(PendingResize {
                        generation,
                        cols,
                        rows,
                        deadline: Instant::now() + Duration::from_millis(settle_ms),
                        suppressed: 0,
                        ioctl_count,
                        pool: Arc::clone(pool),
                    });
                    Some(generation)
                }
            };
            (settle_generation, resize_result)
        };
        if let Some(generation) = settle_generation {
            let session_id = session_id.to_string();
            let state = Arc::clone(&state);
            thread::spawn(move || loop {
                let wait = {
                    let state = state.lock().unwrap();
                    let Some(pending) = state
                        .pending_resize
                        .as_ref()
                        .filter(|pending| pending.generation == generation)
                    else {
                        return;
                    };
                    pending.deadline.saturating_duration_since(Instant::now())
                };
                if !wait.is_zero() {
                    thread::sleep(wait);
                    continue;
                }
                Self::settle_pending_resize(&session_id, &state, Some(generation));
                return;
            });
        }
        resize_result
    }

    fn settle_pending_resize(
        session_id: &str,
        state: &Arc<Mutex<SessionState>>,
        expected_generation: Option<u64>,
    ) {
        // The runtime resolves a reusable session id to the live child. Keep
        // the state lock through the ioctl so kill/resume cannot retarget it.
        let mut state = state.lock().unwrap();
        if expected_generation.is_some_and(|generation| {
            state
                .pending_resize
                .as_ref()
                .is_none_or(|pending| pending.generation != generation)
        }) {
            return;
        }
        let Some(pending) = state.pending_resize.take() else {
            return;
        };
        if state.killed || state.resuming {
            log::info!(
                "cols-gate settle abandoned: session={session_id} {}x{} ({} coalesced) — \
                 kill/resume in flight",
                pending.cols,
                pending.rows,
                pending.suppressed,
            );
            return;
        }

        // Serialize this settled write with resize/install so an older storm
        // can never overwrite a newer in-memory measurement.
        let persisted = match pending.pool.get() {
            Ok(conn) => match crate::repo::session::update_last_size(
                &conn,
                session_id,
                pending.cols,
                pending.rows,
            ) {
                Ok(0) => {
                    log::warn!("resize persistence found no session row: session={session_id}");
                    false
                }
                Ok(_) => {
                    if state.last_requested_size == Some((pending.cols, pending.rows)) {
                        state.last_requested_size_dirty = false;
                    }
                    true
                }
                Err(error) => {
                    log::warn!(
                        "resize persistence failed: session={session_id} {}x{}: {error}",
                        pending.cols,
                        pending.rows,
                    );
                    false
                }
            },
            Err(error) => {
                log::warn!(
                    "resize persistence pool checkout failed: session={session_id} \
                     {}x{}: {error}",
                    pending.cols,
                    pending.rows,
                );
                false
            }
        };

        log::debug!(
            "resize trace: session={session_id} pushes={} immediate_ioctls={} \
             settle_ioctls=0 total_ioctls={} persisted={persisted}",
            pending.suppressed + 1,
            pending.ioctl_count,
            pending.ioctl_count,
        );
    }

    #[cfg(test)]
    pub(crate) fn settle_pending_resize_now(&self, session_id: &str) {
        if let Some(state) = self.session_state(session_id) {
            Self::settle_pending_resize(session_id, &state, None);
        }
    }

    #[cfg(test)]
    pub(crate) fn settle_pending_resize_generation_now(&self, session_id: &str, generation: u64) {
        if let Some(state) = self.session_state(session_id) {
            Self::settle_pending_resize(session_id, &state, Some(generation));
        }
    }

    pub fn forget_session_state(&self, session_id: &str) {
        if let Some(state) = self.session_state(session_id) {
            let mut state = state.lock().unwrap();
            state.output_seq = 0;
            state.last_requested_size = None;
            state.last_requested_size_dirty = false;
            state.pending_resize = None;
        }
        self.prune_empty_session_state(session_id);
    }

    fn ingest_output_chunk(
        &self,
        session_id: &str,
        mission_id: Option<&str>,
        bytes: &[u8],
        events: &dyn SessionEvents,
    ) {
        let ev = self.record_output(session_id, mission_id, bytes);
        events.output(&ev);
    }

    pub(super) fn record_output(
        &self,
        session_id: &str,
        mission_id: Option<&str>,
        bytes: &[u8],
    ) -> OutputEvent {
        let state = self.session_state_or_insert(session_id);
        let mut state = state.lock().unwrap();
        state.output_seq += 1;
        let seq = state.output_seq;

        OutputEvent {
            session_id: session_id.into(),
            mission_id: mission_id.map(str::to_string),
            seq,
            bytes: bytes.to_vec(),
        }
    }
}
