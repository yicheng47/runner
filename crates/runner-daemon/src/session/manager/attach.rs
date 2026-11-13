use super::*;

impl SessionManager {
    pub(super) fn install_handle(
        self: &Arc<Self>,
        session_id: &str,
        handle: SessionHandle,
        mission_status_sink: Option<ForwarderEmitCtx>,
        initial_size: Option<(u16, u16)>,
        pool: &DbPool,
        events: &Arc<dyn SessionEvents>,
    ) {
        let (cols, rows) = initial_size.expect("spawn size must be resolved before handle install");
        let mut control = self.terminal_control.lock().unwrap();
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
                self.rebind_terminal_session(session_id, &terminal, &started_at, pool)
                    .expect("rebind session terminal model");
                (terminal, started_at)
            }
            None => self
                .create_terminal_with_started_at(session_id, cols, rows, pool, events)
                .expect("create session terminal model"),
        };
        let previous_worker = {
            let controller = self
                .inherited_controller(session_id, pool, &mut control)
                .expect("read terminal control group before handle install");
            let state = self.session_state_or_insert(session_id);
            let mut state = state.lock().unwrap();
            if state
                .terminal
                .as_ref()
                .is_some_and(|current| !Arc::ptr_eq(current, &terminal))
            {
                state.finish_companion_terminals();
                for (_, queue) in state.subscribers.drain(..) {
                    queue.close();
                }
            }
            state.last_exit_was_killed = false;
            state.controller = controller;
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
        let spawned = self.install_handle_state(
            session_id,
            handle,
            mission_status_sink,
            initial_size,
            |cols, rows| match pool.get() {
                Ok(conn) => match crate::repo::session::update_last_size(&conn, session_id, cols, rows) {
                    Ok(_) => true,
                    Err(error) => {
                        log::warn!("resize persistence after handle install failed: session={session_id} {cols}x{rows}: {error}");
                        false
                    }
                },
                Err(error) => {
                    log::warn!("resize persistence after handle install pool checkout failed: session={session_id} {cols}x{rows}: {error}");
                    false
                }
            },
        );
        drop(control);
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
        let spawned = self.install_handle_state(
            session_id,
            handle,
            mission_status_sink,
            initial_size,
            persist_size,
        );
        drop(control);
        self.publish_handle_install(&spawned, events);
    }

    fn install_handle_state(
        &self,
        session_id: &str,
        handle: SessionHandle,
        mission_status_sink: Option<ForwarderEmitCtx>,
        initial_size: Option<(u16, u16)>,
        persist_size: impl FnOnce(u16, u16) -> bool,
    ) -> SessionSpawnedEvent {
        let initial_size = initial_size.expect("spawn size must be resolved before handle install");
        let mission_id = handle.mission_id.clone();
        let mut terminal_size = initial_size;
        let state = self.session_state_or_insert(session_id);
        let gate = state.lock().unwrap().delivery_gate.clone();
        {
            let mut delivery = gate.state.lock().unwrap();
            let mut state = state.lock().unwrap();
            delivery.generation = delivery.generation.wrapping_add(1);
            delivery.in_flight = false;
            delivery.next_ticket = 0;
            delivery.next_served = 0;
            delivery.cancelled_tickets.clear();
            gate.ready.notify_all();
            state.pending_spawn = false;
            state.delivery_runtime = handle
                .startup
                .as_ref()
                .filter(|startup| !startup.reports_only)
                .map(|startup| startup.runtime.clone())
                .unwrap_or_default();
            if let Some(startup) = &handle.startup {
                state.model.set_program_status_authority(
                    crate::runtimes::for_key(&startup.runtime)
                        .status_semantics()
                        .program_status_authority,
                );
            }
            state.handle = Some(handle);
            state
                .model
                .apply(SessionEvent::Attached, super::super::clock::state_now());
            state.mission_status_sink = mission_status_sink;
            state.killed = false;

            let requested_size = state.last_requested_size;
            if let Some((cols, rows)) = requested_size.filter(|size| *size != initial_size) {
                let rt_session = state
                    .handle
                    .as_ref()
                    .expect("handle was just installed")
                    .runtime_session
                    .clone();
                match self.runtime.resize(&rt_session, cols, rows) {
                    Ok(()) => {
                        terminal_size = (cols, rows);
                        log::info!(
                            "pty size reconciled after fork: session={session_id} {cols}x{rows} \
                             (pushed mid-fork)"
                        );
                    }
                    Err(error) => log::warn!(
                        "pty size reconcile after fork failed: session={session_id} \
                         {cols}x{rows}: {error}"
                    ),
                }
            }
            if state.last_requested_size_dirty {
                if let Some((cols, rows)) = state.last_requested_size {
                    // This spawn/resume thread owns the state lock so a
                    // newer resize cannot be overwritten by this dirty size.
                    if persist_size(cols, rows) {
                        state.last_requested_size_dirty = false;
                    }
                }
            }
            state.pending_resize = None;
            if let Some(terminal) = &state.terminal {
                terminal.resize(terminal_size.0, terminal_size.1);
            }
        }
        SessionSpawnedEvent {
            session_id: session_id.to_owned(),
            mission_id,
            cols: terminal_size.0,
            rows: terminal_size.1,
        }
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
