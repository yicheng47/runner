use super::*;

impl SessionManager {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn install_handle(
        self: &Arc<Self>,
        session_id: &str,
        handle: SessionHandle,
        mission_status_sink: Option<ForwarderEmitCtx>,
        initial_size: Option<(u16, u16)>,
        initial_cell_size: Option<(u16, u16)>,
        pool: &DbPool,
        events: &Arc<dyn SessionEvents>,
    ) {
        let (cols, rows) = initial_size.expect("spawn size must be resolved before handle install");
        let initial_cell_size = initial_cell_size
            .and_then(|(w, h)| runner_core::protocol::terminal::normalize_cell_size(w, h));
        let mut rev = self
            .control_group_revision
            .load(std::sync::atomic::Ordering::Acquire);
        let (binding, mut control_group) = {
            #[cfg(test)]
            self.run_install_checkout_hook(session_id);
            let conn = pool.get().expect("checkout db before handle install");
            let binding = super::terminal::session_terminal_binding(&conn, session_id)
                .expect("read session terminal binding before handle install");
            let group = Self::control_group_on_conn(&conn, session_id)
                .expect("read terminal control group before handle install");
            (binding, group)
        };
        #[cfg(test)]
        self.run_after_install_snapshot_hook(session_id);
        let mut control = loop {
            let control = self.terminal_control.lock().unwrap();
            if self
                .control_group_revision
                .load(std::sync::atomic::Ordering::Acquire)
                == rev
            {
                break control;
            }
            drop(control);
            rev = self
                .control_group_revision
                .load(std::sync::atomic::Ordering::Acquire);
            let conn = pool
                .get()
                .expect("checkout db to refresh terminal control group");
            control_group = Self::control_group_on_conn(&conn, session_id)
                .expect("refresh terminal control group before handle install");
        };
        let prepared = self.session_state(session_id).and_then(|state| {
            let state = state.lock().unwrap();
            state
                .handle
                .is_none()
                .then(|| {
                    state
                        .terminal
                        .clone()
                        .zip(state.terminal_started_at.clone())
                })
                .flatten()
        });
        let (terminal, terminal_started_at) = match prepared {
            Some((terminal, started_at)) => {
                self.rebind_terminal_session_from_binding(
                    session_id,
                    &terminal,
                    &started_at,
                    binding,
                );
                (terminal, started_at)
            }
            None => self
                .create_terminal_from_binding(
                    session_id,
                    cols,
                    rows,
                    initial_cell_size,
                    binding,
                    pool,
                    events,
                )
                .expect("create session terminal model"),
        };
        let previous_worker = {
            let controller =
                self.inherited_controller_for_group(session_id, control_group, &mut control);
            let state = self.session_state_or_insert(session_id);
            let mut state = state.lock().unwrap();
            if state
                .terminal
                .as_ref()
                .is_some_and(|current| !Arc::ptr_eq(current, &terminal))
            {
                state.finish_companion_terminals();
                for sub in state.subscribers.drain(..) {
                    sub.queue.close();
                }
            }
            state.last_exit_was_killed = false;
            control.set_session_controller(session_id, &mut state, controller);
            state.terminal = Some(terminal);
            state.terminal_started_at = Some(terminal_started_at);
            if state.terminal_input.is_none() {
                let (input, worker) = self.start_terminal_input(
                    session_id,
                    Arc::clone(state.terminal.as_ref().unwrap()),
                    Arc::clone(events),
                );
                state.terminal_input = Some(input);
                state.terminal_input_worker.replace(worker)
            } else {
                None
            }
        };
        if let Some(worker) = previous_worker {
            let _ = worker.join();
        }
        let (spawned, pending_persist) = self.install_handle_state(
            session_id,
            handle,
            mission_status_sink,
            initial_size,
            initial_cell_size,
        );
        drop(control);
        if let Some(work) = pending_persist {
            work.run(|cols, rows, is_current| match pool.get() {
                Ok(conn) => {
                    if !is_current() {
                        return false;
                    }
                    match crate::repo::session::update_last_size(&conn, session_id, cols, rows) {
                        Ok(_) => true,
                        Err(error) => {
                            log::warn!("resize persistence after handle install failed: session={session_id} {cols}x{rows}: {error}");
                            false
                        }
                    }
                }
                Err(error) => {
                    log::warn!("resize persistence after handle install pool checkout failed: session={session_id} {cols}x{rows}: {error}");
                    false
                }
            });
        }
        self.publish_handle_install(&spawned, events.as_ref());
    }

    #[cfg(test)]
    pub(super) fn install_handle_with_size_persistence(
        &self,
        session_id: &str,
        handle: SessionHandle,
        mission_status_sink: Option<ForwarderEmitCtx>,
        initial_size: Option<(u16, u16)>,
        persist_size: impl FnOnce(u16, u16) -> bool,
        events: &dyn SessionEvents,
    ) {
        let control = self.terminal_control.lock().unwrap();
        let (spawned, pending_persist) =
            self.install_handle_state(session_id, handle, mission_status_sink, initial_size, None);
        drop(control);
        if let Some(work) = pending_persist {
            work.run(|cols, rows, _| persist_size(cols, rows));
        }
        self.publish_handle_install(&spawned, events);
    }

    pub(super) fn install_handle_state(
        &self,
        session_id: &str,
        handle: SessionHandle,
        mission_status_sink: Option<ForwarderEmitCtx>,
        initial_size: Option<(u16, u16)>,
        initial_cell_size: Option<(u16, u16)>,
    ) -> (SessionSpawnedEvent, Option<PendingResizePersist>) {
        let initial_size = initial_size.expect("spawn size must be resolved before handle install");
        let initial_cell_size = initial_cell_size
            .and_then(|(w, h)| runner_core::protocol::terminal::normalize_cell_size(w, h));
        let mission_id = handle.mission_id.clone();
        let mut terminal_size = initial_size;
        let mut terminal_cell = initial_cell_size;
        let state = self.session_state_or_insert(session_id);
        let gate = state.lock().unwrap().delivery_gate.clone();
        let (output_seq, pending_persist) = {
            let mut delivery = gate.state.lock().unwrap();
            let mut state_guard = state.lock().unwrap();
            delivery.generation = delivery.generation.wrapping_add(1);
            delivery.in_flight = false;
            delivery.next_ticket = 0;
            delivery.next_served = 0;
            delivery.cancelled_tickets.clear();
            gate.ready.notify_all();
            state_guard.pending_spawn = false;
            state_guard.delivery_runtime = handle
                .startup
                .as_ref()
                .filter(|startup| !startup.reports_only)
                .map(|startup| startup.runtime.clone())
                .unwrap_or_default();
            if let Some(startup) = &handle.startup {
                let semantics = crate::runtimes::for_key(&startup.runtime).status_semantics();
                state_guard.model.set_program_status_authority(
                    semantics.program_status_authoritative(startup.status_hooks_enabled()),
                );
                state_guard
                    .model
                    .set_program_status_root_app(semantics.program_status_root_app);
                state_guard.model.set_program_status_transcript_outcomes(
                    semantics.program_status_transcript_outcomes,
                );
                state_guard
                    .model
                    .set_program_status_cancel_interrupt(semantics.program_status_cancel_interrupt);
                state_guard
                    .model
                    .set_program_status_idle_completes(semantics.program_status_idle_completes);
                state_guard.model.set_program_status_suppress_child_tasks(
                    semantics.program_status_suppress_child_tasks,
                );
                state_guard
                    .model
                    .set_native_attention(semantics.native_attention);
            }
            state_guard.handle = Some(handle);
            state_guard
                .model
                .apply(SessionEvent::Attached, super::super::clock::state_now());
            state_guard.mission_status_sink = mission_status_sink;
            state_guard.killed = false;

            let requested_size = state_guard.last_requested_size;
            let requested_cell = state_guard.last_requested_cell_size;
            let grid_differs = requested_size.is_some_and(|size| size != initial_size);
            let cell_differs = requested_cell.is_some() && requested_cell != initial_cell_size;
            if grid_differs || cell_differs {
                let (cols, rows) = requested_size.unwrap_or(initial_size);
                let target_cell = requested_cell.or(initial_cell_size);
                if cfg!(windows) && !grid_differs {
                    terminal_cell = target_cell;
                } else {
                    let rt_session = state_guard
                        .handle
                        .as_ref()
                        .expect("handle was just installed")
                        .runtime_session
                        .clone();
                    let (pixel_width, pixel_height) =
                        super::super::runtime::pty_pixel_size(cols, rows, target_cell);
                    match self
                        .runtime
                        .resize(&rt_session, cols, rows, pixel_width, pixel_height)
                    {
                        Ok(()) => {
                            terminal_size = (cols, rows);
                            terminal_cell = target_cell;
                            log::info!(
                                "pty size reconciled after fork: session={session_id} {cols}x{rows} \
                                 (pushed mid-fork)"
                            );
                        }
                        Err(error) => {
                            log::warn!(
                                "pty size reconcile after fork failed: session={session_id} \
                                 {cols}x{rows}: {error}"
                            );
                            state_guard.last_requested_size = Some(initial_size);
                            state_guard.last_requested_cell_size = initial_cell_size;
                            state_guard.last_requested_size_dirty = false;
                        }
                    }
                }
            }
            let stamp = state_guard.bump_resize_stamp();
            let pending_persist = if state_guard.last_requested_size_dirty {
                state_guard.last_requested_size.map(|(cols, rows)| {
                    state_guard.prepare_resize_persist(&state, cols, rows, stamp)
                })
            } else {
                None
            };
            state_guard.pending_resize = None;
            if let Some(terminal) = &state_guard.terminal {
                let before_size = terminal.size();
                let before_cell = terminal.cell_size();
                let cell = terminal_cell
                    .map_or(runner_terminal::kitty::CellSize::ZERO, |(width, height)| {
                        runner_terminal::kitty::CellSize { width, height }
                    });
                terminal.set_authoritative_size(terminal_size.0, terminal_size.1, cell);
                let after_size = terminal.size();
                let after_cell = terminal.cell_size();
                if after_size != before_size || after_cell != before_cell {
                    state_guard.output_seq += 1;
                    let seq = state_guard.output_seq;
                    let controller = state_guard.controller.wire();
                    Self::push_plain_frame(
                        &mut state_guard,
                        runner_core::protocol::terminal::TerminalFrame::Resized {
                            seq,
                            cols: after_size.0,
                            rows: after_size.1,
                            cell_width: after_cell.width,
                            cell_height: after_cell.height,
                            controller,
                        },
                    );
                }
            }
            (state_guard.output_seq, pending_persist)
        };
        (
            SessionSpawnedEvent {
                session_id: session_id.to_owned(),
                mission_id,
                cols: terminal_size.0,
                rows: terminal_size.1,
                output_seq,
            },
            pending_persist,
        )
    }

    fn publish_handle_install(&self, spawned: &SessionSpawnedEvent, events: &dyn SessionEvents) {
        if let Ok(terminal) = self.terminal_model(&spawned.session_id) {
            terminal.publish_initial();
        }
        events.spawned(spawned);
        self.notify_delivery_event(&spawned.session_id, router::SessionDeliveryEvent::Respawned);
    }

    pub(super) fn install_forwarder(&self, session_id: &str, forwarder: thread::JoinHandle<()>) {
        if let Some(state) = self.session_state(session_id) {
            if let Some(handle) = state.lock().unwrap().handle.as_mut() {
                handle.forwarder = Some(forwarder);
            }
        }
    }
}
