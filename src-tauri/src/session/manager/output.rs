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
            state.last_local_input_at = None;
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
    /// Drains the runtime's `OutputStream` into `session/output`
    /// events, then on channel close queries the runtime for the
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
            // OR `kill` flips the stop flag. Stream chunks flow as
            // `session/output` events. StatusTransition is routed
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
                        // Track terminal modes before recording so
                        // that the very next `output_snapshot` (if
                        // one races in here) reflects the latest
                        // state the agent just emitted.
                        manager_t.update_terminal_mode_state(&session_id, &bytes);
                        let ev = manager_t.record_output(
                            &session_id,
                            mission_id.as_deref(),
                            BASE64.encode(&bytes),
                        );
                        events.output(&ev);
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
        self.write_stdin_bytes(&rt_session, bytes)?;
        drop(session);
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
        while delivery.generation == generation
            && (delivery.in_flight || delivery.next_served != ticket)
        {
            delivery = gate.ready.wait(delivery).unwrap();
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
    pub fn resize(&self, session_id: &str, cols: u16, rows: u16, pool: &DbPool) -> Result<()> {
        let rt_session = self.live_runtime_session(session_id)?;
        self.runtime.resize(&rt_session, cols, rows)?;
        // Full-repaint TUI runtimes (claude-code, codex) redraw the whole
        // frame on SIGWINCH, so bytes buffered before a *width* change
        // describe a stale grid width. Replaying them into the new grid on
        // a later snapshot re-attach wraps their absolute-positioned frames
        // wrong — box-drawing borders shredded into scrollback garbage
        // (seen dogfooding split view, impl 0020). Drop them: the incoming
        // repaint rebuilds the buffer at the new width, and the frontend
        // already hard-clears its local viewport for these runtimes on
        // width changes.
        //
        // Rows-only resizes keep the ring: reflow depends on cols alone,
        // and the frontend's activation dance nudges rows (rows-1 → rows)
        // with width held constant on every tab return — purging there
        // threw away claude-code history that snapshot replay could have
        // restored (the #306 symptom: remount shows only the latest
        // frame). Shells keep their buffer unconditionally — no repaint
        // would arrive, and their history is meaningful.
        let cols_changed = {
            let state = self.session_state_or_insert(session_id);
            let mut state = state.lock().unwrap();
            let changed = state.last_pty_cols != Some(cols);
            state.last_pty_cols = Some(cols);
            changed
        };
        if cols_changed && runtime_clears_on_resize(session_id, pool) {
            self.purge_output_buffer_keep_modes(session_id);
        }
        Ok(())
    }

    /// Return the bounded in-memory PTY output snapshot for a session.
    ///
    /// Tauri events are live-only; without this, a terminal pane mounted after
    /// a session already produced output starts blank until the child redraws.
    /// The snapshot is intentionally process-local and bounded: it covers
    /// webview reloads / chat switching for live sessions without turning the
    /// sessions table into a PTY transcript store.
    pub fn output_snapshot(&self, session_id: &str) -> Vec<OutputEvent> {
        let Some(state) = self.session_state(session_id) else {
            return Vec::new();
        };
        let state = state.lock().unwrap();
        let mut events: Vec<OutputEvent> = state.output_buffer.iter().cloned().collect();
        // For sessions currently inside terminal modes that xterm
        // reset clears, prepend a synthetic chunk restoring them.
        // Long-running TUI sessions (claude-code, codex) lose the
        // original enter-alt-screen escape from the bounded
        // 4096-chunk buffer over time, so a re-attach that just
        // replays the remaining chunks lands mid-alt-screen content
        // into xterm's main screen — visible as stacked redraws in
        // scrollback and a blank alt-screen pane on route remount.
        // Bracketed paste has the same problem after RunnerTerminal
        // calls `reset()`: without replaying `\x1b[?2004h`, xterm
        // sends multiline clipboard text as raw Enter-delimited
        // keystrokes. seq=0 sits below every real event's monotonic
        // seq so the frontend's `seq <= lastWrittenSeq` filter
        // doesn't drop it on re-replay.
        let mut synthetic_prefix = Vec::new();
        if state.alt_screen_on {
            synthetic_prefix.extend_from_slice(b"\x1b[?1049h");
        }
        if state.bracketed_paste_on {
            synthetic_prefix.extend_from_slice(b"\x1b[?2004h");
        }
        if !synthetic_prefix.is_empty() {
            events.insert(
                0,
                OutputEvent {
                    session_id: session_id.into(),
                    mission_id: None,
                    seq: 0,
                    data: BASE64.encode(synthetic_prefix),
                },
            );
        }
        events
    }

    /// Drop the in-memory output buffer + seq counter for a session.
    /// Called when the session is genuinely going away (archive, runner
    /// delete) so the bounded ring buffer doesn't accumulate forever.
    /// Safe to call on a session that's never written output.
    pub fn purge_session_buffers(&self, session_id: &str) {
        if let Some(state) = self.session_state(session_id) {
            let mut state = state.lock().unwrap();
            state.output_buffer.clear();
            state.output_seq = 0;
            state.resume_watermark_seq = 0;
            state.alt_screen_on = false;
            state.bracketed_paste_on = false;
            state.last_pty_cols = None;
        }
        self.prune_empty_session_state(session_id);
    }

    /// Record the resume watermark: the seq the ring had reached when
    /// the current resume started. Called at the top of `resume()`
    /// for every runtime — on the purge path (codex) it equals the
    /// post-purge floor, so the frontend's `seq > watermark` filter
    /// is a no-op there.
    pub fn set_resume_watermark(&self, session_id: &str) {
        if let Some(state) = self.session_state(session_id) {
            let mut state = state.lock().unwrap();
            state.resume_watermark_seq = state.output_seq;
        }
    }

    /// Read back the resume watermark for the pill fast-paths.
    /// Sessions that never resumed (or whose state was purged)
    /// report 0, which degenerates the frontend filter to
    /// "any chunk counts".
    pub fn replay_watermark(&self, session_id: &str) -> u64 {
        self.session_state(session_id)
            .map(|state| state.lock().unwrap().resume_watermark_seq)
            .unwrap_or(0)
    }

    /// Drop only the output buffer for a session, keeping the seq
    /// counter. Used by `resume` for runtimes that repaint their
    /// whole frame on resume (codex — see `runtime_purges_on_resume`):
    /// clearing the buffer means the post-resume snapshot is fresh
    /// (no double banner / stacked agent output on remount), while
    /// preserving the monotonic seq means the new PTY's first chunk
    /// is `last + 1` rather than `1` — which the frontend's
    /// `seq <= lastWrittenSeq` filter would otherwise drop.
    pub fn purge_output_buffer(&self, session_id: &str) {
        if let Some(state) = self.session_state(session_id) {
            let mut state = state.lock().unwrap();
            state.output_buffer.clear();
            // Resume forks a new child; whether it'll be in alt-screen
            // or bracketed-paste mode depends on the new process's own
            // startup. Clear the state so it re-derives from emitted bytes
            // instead of inheriting the prior child's mode.
            state.alt_screen_on = false;
            state.bracketed_paste_on = false;
        }
        self.prune_empty_session_state(session_id);
    }

    /// Buffer-only purge for `resize`: the child process survives, so its
    /// terminal modes persist — a SIGWINCH repaint does not re-emit the
    /// enter-alt-screen / bracketed-paste escapes, and clearing the flags
    /// here would strip the synthetic prefix a later snapshot needs. The
    /// seq counter is likewise untouched.
    fn purge_output_buffer_keep_modes(&self, session_id: &str) {
        if let Some(state) = self.session_state(session_id) {
            let mut state = state.lock().unwrap();
            state.output_buffer.clear();
        }
    }

    /// Update per-session terminal mode flags from a raw runtime
    /// chunk. We take the *latest* match in the chunk as the resulting
    /// state for each tracked mode. No-op when the chunk contains no
    /// tracked mode-switch escape.
    ///
    /// Boundary caveat: an escape could be split across two
    /// adjacent chunks if the PTY reader's buffer slice happens to
    /// land mid-sequence. The sequences are 7-8 bytes and the
    /// reader's reads are kilobyte-scale, so the odds are low and
    /// the worst case is one missed transition — the next emitted
    /// transition picks up the right state. If this turns out to
    /// matter we can carry a small tail buffer across chunks.
    fn update_terminal_mode_state(&self, session_id: &str, bytes: &[u8]) {
        let alt_screen = scan_alt_screen_transition(bytes);
        let bracketed_paste = scan_bracketed_paste_transition(bytes);
        if alt_screen.is_none() && bracketed_paste.is_none() {
            return;
        }
        let state = self.session_state_or_insert(session_id);
        let mut state = state.lock().unwrap();
        if let Some(new_state) = alt_screen {
            state.alt_screen_on = new_state;
        }
        if let Some(new_state) = bracketed_paste {
            state.bracketed_paste_on = new_state;
        }
    }

    fn record_output(
        &self,
        session_id: &str,
        mission_id: Option<&str>,
        data: String,
    ) -> OutputEvent {
        let state = self.session_state_or_insert(session_id);
        let mut state = state.lock().unwrap();
        state.output_seq += 1;
        let seq = state.output_seq;

        let ev = OutputEvent {
            session_id: session_id.into(),
            mission_id: mission_id.map(str::to_string),
            seq,
            data,
        };

        state.output_buffer.push_back(ev.clone());
        while state.output_buffer.len() > MAX_OUTPUT_BUFFER_CHUNKS {
            state.output_buffer.pop_front();
        }
        ev
    }
}

/// Whether the session's agent runtime fully repaints on SIGWINCH — the
/// same set the frontend's `runtimeClearsOnResize` gates its local
/// viewport clear on. Runner-backed sessions leave
/// `sessions.agent_runtime` NULL and carry their runtime on the runner
/// row, hence the COALESCE join (same pattern as the direct-chat
/// queries). Best-effort: a DB miss keeps the buffer.
pub(super) fn runtime_clears_on_resize(session_id: &str, pool: &DbPool) -> bool {
    let Ok(conn) = pool.get() else {
        return false;
    };
    let runtime = conn
        .query_row(
            "SELECT COALESCE(s.agent_runtime, r.runtime)
               FROM sessions s
               LEFT JOIN runners r ON r.id = s.runner_id
              WHERE s.id = ?1",
            params![session_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten();
    matches!(runtime.as_deref(), Some("claude-code") | Some("codex"))
}

/// Whether `resume` should drop the session's output ring before
/// spawning the new PTY. Codex repaints its whole frame on resume
/// (and its own resume replay restores a deep conversation tail), so
/// replaying retained scrollback under the new frame stacks garbled
/// content — the artifact class from impls 0009/0011/0020. Claude-code
/// paints inline into the main screen: kept scrollback, then the
/// resume banner, then the tail repaint is exactly what a physical
/// terminal shows, so its ring is kept. Shells and future runtimes
/// keep today's purge purely
/// for scope (extending them is a one-line change here). Best-effort:
/// a DB miss fails toward the purge, i.e. today's behavior.
pub(super) fn runtime_purges_on_resume(session_id: &str, pool: &DbPool) -> bool {
    let Ok(conn) = pool.get() else {
        return true;
    };
    let runtime = conn
        .query_row(
            "SELECT COALESCE(s.agent_runtime, r.runtime)
               FROM sessions s
               LEFT JOIN runners r ON r.id = s.runner_id
              WHERE s.id = ?1",
            params![session_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten();
    !matches!(runtime.as_deref(), Some("claude-code"))
}
