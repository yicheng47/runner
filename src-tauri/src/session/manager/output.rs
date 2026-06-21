use super::*;

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
                        // Track alt-screen mode before recording so
                        // that the very next `output_snapshot` (if
                        // one races in here) reflects the latest
                        // state the agent just emitted.
                        manager_t.update_alt_screen_state(&session_id, &bytes);
                        let ev = manager_t.record_output(
                            &session_id,
                            mission_id.as_deref(),
                            BASE64.encode(&bytes),
                        );
                        events.output(&ev);
                    }
                    Ok(RuntimeOutput::StatusTransition { state, source }) => {
                        if let Some(ctx) = emit_ctx.as_ref() {
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
                            events.status(&SessionActivityEvent {
                                session_id: session_id.clone(),
                                state: state.into(),
                                source: source.to_string(),
                            });
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
                "stopped"
            } else {
                "crashed"
            };
            if let Ok(conn) = pool.get() {
                if resume_failed {
                    let _ = conn.execute(
                        "UPDATE sessions
                            SET status = ?1, stopped_at = ?2,
                                agent_session_key = NULL
                          WHERE id = ?3",
                        params!["crashed", Utc::now().to_rfc3339(), session_id],
                    );
                } else {
                    let _ = conn.execute(
                        "UPDATE sessions
                            SET status = ?1, stopped_at = ?2
                          WHERE id = ?3",
                        params![final_status, Utc::now().to_rfc3339(), session_id],
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
                mission_id,
                exit_code,
                success,
            });
            let _ = manager_t.forget_runtime_handle(&session_id, &rt_session);
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
        // ASCII CR (0x0D) is what claude-code's TUI editor reads as
        // "Enter" — bare-byte writes that just contain `\r` map to
        // `send_key("Enter")`. Everything else routes as a literal
        // byte stream.
        if bytes == b"\r" {
            self.runtime
                .send_key(&rt_session, "Enter")
                .map_err(Into::into)
        } else {
            self.runtime
                .send_bytes(&rt_session, bytes)
                .map_err(Into::into)
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
        self.runtime
            .send_key(&rt_session, "Enter")
            .map_err(Into::into)
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
    pub fn resize(&self, session_id: &str, cols: u16, rows: u16) -> Result<()> {
        let rt_session = self.live_runtime_session(session_id)?;
        self.runtime
            .resize(&rt_session, cols, rows)
            .map_err(Into::into)
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
        // For sessions currently inside an alt-screen, prepend a
        // synthetic chunk carrying the `\x1b[?1049h` enter escape.
        // Long-running TUI sessions (claude-code, codex) lose the
        // original enter-alt-screen escape from the bounded
        // 4096-chunk buffer over time, so a re-attach that just
        // replays the remaining chunks lands mid-alt-screen content
        // into xterm's main screen — visible as stacked redraws in
        // scrollback and a blank alt-screen pane on route remount.
        // seq=0 sits below every real event's
        // monotonic seq so the frontend's `seq <= lastWrittenSeq`
        // filter doesn't drop it on re-replay.
        if state.alt_screen_on {
            events.insert(
                0,
                OutputEvent {
                    session_id: session_id.into(),
                    mission_id: None,
                    seq: 0,
                    data: BASE64.encode(b"\x1b[?1049h"),
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
            state.alt_screen_on = false;
        }
        self.prune_empty_session_state(session_id);
    }

    /// Drop only the output buffer for a session, keeping the seq
    /// counter. Used by `resume`: clearing the buffer means the
    /// post-resume snapshot is fresh (no double banner / stacked
    /// agent output on remount), while preserving the monotonic seq
    /// means the new PTY's first chunk is `last + 1` rather than
    /// `1` — which the frontend's `seq <= lastWrittenSeq` filter
    /// would otherwise drop.
    pub fn purge_output_buffer(&self, session_id: &str) {
        if let Some(state) = self.session_state(session_id) {
            let mut state = state.lock().unwrap();
            state.output_buffer.clear();
            // Resume forks a new child; whether it'll be in alt-screen
            // depends on the new process's own startup. Clear the state
            // so it re-derives from the new child's emitted bytes
            // instead of inheriting the prior child's mode.
            state.alt_screen_on = false;
        }
        self.prune_empty_session_state(session_id);
    }

    /// Update the per-session alt-screen flag from a raw runtime
    /// chunk. We scan for the four mode-switch escapes (1049h/l +
    /// 47h/l) and take the *latest* match in the chunk as the
    /// resulting state. No-op when the chunk contains no
    /// mode-switch escape.
    ///
    /// Boundary caveat: an escape could be split across two
    /// adjacent chunks if the PTY reader's buffer slice happens to
    /// land mid-sequence. The sequences are 7-8 bytes and the
    /// reader's reads are kilobyte-scale, so the odds are low and
    /// the worst case is one missed transition — the next emitted
    /// transition picks up the right state. If this turns out to
    /// matter we can carry a small tail buffer across chunks.
    fn update_alt_screen_state(&self, session_id: &str, bytes: &[u8]) {
        if let Some(new_state) = scan_alt_screen_transition(bytes) {
            self.session_state_or_insert(session_id)
                .lock()
                .unwrap()
                .alt_screen_on = new_state;
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
