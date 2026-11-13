use super::*;

impl SessionManager {
    pub(super) fn session_state(&self, session_id: &str) -> Option<Arc<Mutex<SessionState>>> {
        let session = self.raw_session_state(session_id)?;
        let key_only = session.lock().unwrap().key_only;
        (!key_only).then_some(session)
    }

    pub(super) fn raw_session_state(&self, session_id: &str) -> Option<Arc<Mutex<SessionState>>> {
        self.sessions.lock().unwrap().get(session_id).cloned()
    }

    pub(super) fn session_state_or_insert(&self, session_id: &str) -> Arc<Mutex<SessionState>> {
        loop {
            let session = self
                .sessions
                .lock()
                .unwrap()
                .entry(session_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(SessionState::default())))
                .clone();
            let mut state = session.lock().unwrap();
            if state.key_only {
                let sessions = self.sessions.lock().unwrap();
                if !sessions
                    .get(session_id)
                    .is_some_and(|current| Arc::ptr_eq(current, &session))
                {
                    continue;
                }
                state.key_only = false;
            }
            drop(state);
            return session;
        }
    }

    pub(crate) fn report_key(
        &self,
        session_id: &str,
        report: PersistKey,
        persist: impl FnOnce(&PersistKey) -> Result<bool>,
    ) -> Result<bool> {
        let session = self
            .sessions
            .lock()
            .unwrap()
            .entry(session_id.to_owned())
            .or_insert_with(|| {
                Arc::new(Mutex::new(SessionState {
                    key_only: true,
                    ..Default::default()
                }))
            })
            .clone();
        let effect = session
            .lock()
            .unwrap()
            .model
            .apply(
                SessionEvent::ConversationChanged(report),
                super::super::clock::state_now(),
            )
            .persist_key
            .unwrap();
        let written = persist(&effect);
        if matches!(written, Ok(true)) {
            session.lock().unwrap().model.apply(
                SessionEvent::KeyPersisted(effect),
                super::super::clock::state_now(),
            );
        } else {
            self.prune_key_only_session_state(session_id);
        }
        written
    }

    pub(crate) fn write_key_effect(
        conn: &rusqlite::Connection,
        session_id: &str,
        effect: &PersistKey,
    ) -> Result<bool> {
        match effect.origin {
            KeyOrigin::Assigned => Ok(conn.execute(
                "UPDATE sessions SET agent_session_key = ?1 WHERE id = ?2",
                params![effect.key, session_id],
            )? > 0),
            KeyOrigin::Captured => Ok(crate::repo::session::capture_agent_session_key(
                conn,
                session_id,
                effect.key.as_deref().expect("capture reports a key"),
                &effect.generation,
            )?),
            KeyOrigin::Rekeyed => Ok(crate::repo::session::set_live_agent_session_key(
                conn,
                session_id,
                effect.key.as_deref(),
                &effect.generation,
            )?),
        }
    }

    pub(crate) fn persist_reported_key(
        &self,
        session_id: &str,
        report: PersistKey,
        pool: &DbPool,
    ) -> Result<bool> {
        self.report_key(session_id, report, |effect| {
            let conn = pool.get()?;
            Self::write_key_effect(&conn, session_id, effect)
        })
    }

    #[cfg(test)]
    pub(crate) fn key_test_manager() -> Arc<Self> {
        Self::new(
            Arc::new(RwLock::new(crate::shell_path::LoginShellEnv::default())),
            Arc::new(RwLock::new(crate::shell_path::DiscoveryState::pending())),
            Arc::new(super::super::pty_runtime::PtyRuntime::new()),
        )
    }

    pub(super) fn latest_requested_size(&self, session_id: &str) -> Option<(u16, u16)> {
        self.session_state(session_id)
            .and_then(|state| state.lock().unwrap().last_requested_size)
    }

    pub fn register_delivery_listener(
        &self,
        session_id: &str,
        listener: Weak<dyn router::SessionDeliveryListener>,
    ) {
        let mut listeners = self.delivery_listeners.lock().unwrap();
        let listeners = listeners.entry(session_id.to_string()).or_default();
        if !listeners.iter().any(|existing| existing.ptr_eq(&listener)) {
            listeners.push(listener);
        }
    }

    pub(super) fn notify_delivery_event(
        &self,
        session_id: &str,
        event: router::SessionDeliveryEvent,
    ) {
        let listeners = {
            let mut listeners_by_session = self.delivery_listeners.lock().unwrap();
            let (live, remove_entry) = {
                let Some(listeners) = listeners_by_session.get_mut(session_id) else {
                    return;
                };
                let mut live = Vec::with_capacity(listeners.len());
                listeners.retain(|listener| match listener.upgrade() {
                    Some(listener) => {
                        live.push(listener);
                        true
                    }
                    _ => false,
                });
                (live, listeners.is_empty())
            };
            if remove_entry {
                listeners_by_session.remove(session_id);
            }
            live
        };
        for listener in listeners {
            listener.session_delivery_event(session_id, event);
        }
    }

    pub fn input_quiescent(&self, session_id: &str) -> bool {
        let Some(session) = self.session_state(session_id) else {
            return false;
        };
        let gate = session.lock().unwrap().delivery_gate.clone();
        let delivery = gate.state.lock().unwrap();
        let session = session.lock().unwrap();
        session.handle.as_ref().is_some_and(|handle| {
            handle
                .startup
                .as_ref()
                .is_none_or(|startup| startup.reports_only)
        }) && !session.model.status().observation.needs_you()
            && !delivery.in_flight
            && delivery.next_ticket == delivery.next_served
            && session.model.draft_quiescent(crate::session::clock::now())
    }

    /// Ids of every session whose process is attached right now.
    pub fn live_session_ids(&self) -> Vec<String> {
        let sessions: Vec<_> = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .map(|(id, session)| (id.clone(), Arc::clone(session)))
            .collect();
        sessions
            .into_iter()
            .filter(|(_, session)| session.lock().unwrap().handle.is_some())
            .map(|(id, _)| id)
            .collect()
    }

    pub fn session_live(&self, session_id: &str) -> bool {
        self.session_state(session_id)
            .is_some_and(|session| session.lock().unwrap().handle.is_some())
    }

    pub fn delivery_snapshot(&self, session_id: &str) -> Option<router::DeliverySnapshot> {
        let session = self.session_state(session_id)?;
        let gate = session.lock().unwrap().delivery_gate.clone();
        let delivery = gate.state.lock().unwrap();
        let session = session.lock().unwrap();
        session.handle.as_ref()?;
        Some(router::DeliverySnapshot {
            rows: session
                .terminal
                .as_ref()
                .map(|terminal| terminal.input_rows())
                .unwrap_or_default(),
            generation: delivery.generation,
            input_revision: (
                delivery.next_ticket,
                session
                    .terminal
                    .as_ref()
                    .map(|terminal| terminal.input_reset_guard())
                    .unwrap_or(0),
            ),
            turn_started: session.turn_started,
            prompted: session.delivery_prompted,
            runtime: session.delivery_runtime.clone(),
            unsent: session.model.unsent_notice(),
            held: session
                .terminal
                .as_ref()
                .is_some_and(|terminal| terminal.delivery_held()),
        })
    }

    pub fn resubmit(
        &self,
        session_id: &str,
        snapshot: &router::DeliverySnapshot,
        probe: &runner_terminal::delivery_probe::InputProbe,
    ) -> Result<bool> {
        let Some(session) = self.session_state(session_id) else {
            return Ok(false);
        };
        let gate = session.lock().unwrap().delivery_gate.clone();
        let delivery = gate.state.lock().unwrap();
        let session = session.lock().unwrap();
        if delivery.in_flight
            || delivery.generation != snapshot.generation
            || delivery.next_ticket != snapshot.input_revision.0
            || delivery.next_ticket != delivery.next_served
            || session.model.status().observation.needs_you()
        {
            return Ok(false);
        }
        let Some(handle) = &session.handle else {
            return Ok(false);
        };
        let Some(terminal) = session.terminal.clone() else {
            return Ok(false);
        };
        let runtime_session = handle.runtime_session.clone();
        drop(session);
        terminal
            .with_unchanged_input(snapshot.input_revision.1, probe, || {
                self.write_stdin_bytes(&runtime_session, b"\r")
            })
            .transpose()
            .map(|written| written.is_some())
    }

    pub fn confirm_delivery(&self, session_id: &str, snapshot: &router::DeliverySnapshot) {
        let Some(session) = self.session_state(session_id) else {
            return;
        };
        let gate = session.lock().unwrap().delivery_gate.clone();
        let delivery = gate.state.lock().unwrap();
        let Some(terminal) = session.lock().unwrap().terminal.clone() else {
            return;
        };
        if delivery.generation != snapshot.generation
            || delivery.next_ticket != snapshot.input_revision.0
        {
            return;
        }
        let observation = terminal.confirm_delivery(snapshot.input_revision.1);
        drop(delivery);
        terminal.publish_input(observation);
    }

    pub fn recheck_composer(&self, session_id: &str) {
        let Some(session) = self.session_state(session_id) else {
            return;
        };
        let terminal = {
            let session = session.lock().unwrap();
            if session.model.activity() != Some(SessionActivityState::Idle) {
                return;
            }
            session.terminal.clone()
        };
        if let Some(terminal) = terminal {
            terminal.recheck_delivery();
        }
    }

    pub fn delivery_readiness(&self, session_id: &str) -> router::DeliveryReadiness {
        let Some(session) = self.raw_session_state(session_id) else {
            return router::DeliveryReadiness::Unavailable;
        };
        let gate = session.lock().unwrap().delivery_gate.clone();
        let delivery = gate.state.lock().unwrap();
        let session = session.lock().unwrap();
        session.delivery_readiness(&delivery, crate::session::clock::now())
    }

    pub fn report_readiness(&self, session_id: &str) -> router::DeliveryReadiness {
        let Some(session) = self.raw_session_state(session_id) else {
            return router::DeliveryReadiness::Unavailable;
        };
        let gate = session.lock().unwrap().delivery_gate.clone();
        let delivery = gate.state.lock().unwrap();
        let session = session.lock().unwrap();
        session.report_readiness(&delivery, crate::session::clock::now())
    }

    /// Reserve the input gate for a report, only while the session is idle.
    pub fn reserve_report(&self, session_id: &str) -> Result<Option<u64>> {
        let Some(session) = self.session_state(session_id) else {
            return Ok(None);
        };
        let gate = session.lock().unwrap().delivery_gate.clone();
        let mut delivery = gate.state.lock().unwrap();
        let session = session.lock().unwrap();
        if session.report_readiness(&delivery, crate::session::clock::now())
            != (router::DeliveryReadiness::Ready { idle: true })
        {
            return Ok(None);
        }
        delivery.in_flight = true;
        Ok(Some(delivery.generation))
    }

    pub fn reserve_delivery(&self, session_id: &str) -> Result<router::DeliveryReservation> {
        use router::{DeliveryReadiness as R, DeliveryReservation as D};
        let Some(session) = self.session_state(session_id) else {
            return Ok(D::Unavailable);
        };
        let gate = session.lock().unwrap().delivery_gate.clone();
        let mut delivery = gate.state.lock().unwrap();
        let session = session.lock().unwrap();
        let now = crate::session::clock::now();
        Ok(match session.delivery_readiness(&delivery, now) {
            R::Ready { idle: true } | R::Working => {
                delivery.in_flight = true;
                D::Ready(delivery.generation)
            }
            R::Ready { idle: false } | R::Unconfirmed | R::Pending | R::Unavailable => {
                D::Unavailable
            }
            R::Starting => D::Starting,
            R::NeedsYou => D::HumanInteraction,
            R::InFlight => D::InFlight,
            R::RecentlyTyping(delay) => D::RecentlyTyping(delay),
            R::Drafting | R::Unsent => session.model.draft_hold(now).unwrap(),
        })
    }

    pub fn report_input_state(&self, session_id: &str, observation: InputObservation) {
        let Some(session) = self.session_state(session_id) else {
            return;
        };
        let input_cleared = {
            let mut session = session.lock().unwrap();
            if session.handle.is_none() {
                return;
            }
            session
                .model
                .apply(
                    SessionEvent::Composer {
                        observation,
                        live: true,
                    },
                    super::super::clock::state_now(),
                )
                .input_cleared
        };
        if input_cleared {
            self.notify_delivery_event(session_id, router::SessionDeliveryEvent::InputCleared);
        }
    }

    pub fn finish_delivery(&self, session_id: &str, token: u64) {
        let Some(session) = self.session_state(session_id) else {
            return;
        };
        let gate = session.lock().unwrap().delivery_gate.clone();
        let finished = {
            let mut delivery = gate.state.lock().unwrap();
            if !delivery.in_flight || delivery.generation != token {
                false
            } else {
                delivery.in_flight = false;
                gate.ready.notify_all();
                true
            }
        };
        if finished {
            self.notify_delivery_event(session_id, router::SessionDeliveryEvent::DeliveryFinished);
        }
    }

    pub(super) fn prune_empty_session_state(&self, session_id: &str) {
        self.prune_session_state(session_id, false);
    }

    pub(super) fn prune_key_only_session_state(&self, session_id: &str) {
        self.prune_session_state(session_id, true);
    }

    fn prune_session_state(&self, session_id: &str, key_only: bool) {
        let Some(state) = self.raw_session_state(session_id) else {
            return;
        };
        let state_guard = state.lock().unwrap();
        if !state_guard.is_empty() || (key_only && !state_guard.key_only) {
            return;
        }
        let mut sessions = self.sessions.lock().unwrap();
        if sessions
            .get(session_id)
            .is_some_and(|current| Arc::ptr_eq(current, &state))
        {
            sessions.remove(session_id);
        }
    }

    pub(super) fn live_runtime_session(&self, session_id: &str) -> Result<RuntimeSession> {
        let Some(state) = self.session_state(session_id) else {
            return Err(Error::msg(format!("session not found: {session_id}")));
        };
        let rt_session = state
            .lock()
            .unwrap()
            .handle
            .as_ref()
            .map(|h| h.runtime_session.clone())
            .ok_or_else(|| Error::msg(format!("session not found: {session_id}")))?;
        Ok(rt_session)
    }

    pub(crate) fn try_claim_resume(self: &Arc<Self>, session_id: &str) -> Option<ResumeClaim> {
        let state = self.session_state_or_insert(session_id);
        let mut state = state.lock().unwrap();
        if state.resuming {
            return None;
        }
        state.resuming = true;
        Some(ResumeClaim {
            mgr: Arc::clone(self),
            session_id: session_id.to_owned(),
        })
    }

    pub(super) fn release_resume_claim(&self, session_id: &str) {
        if let Some(state) = self.session_state(session_id) {
            state.lock().unwrap().resuming = false;
        }
        self.prune_empty_session_state(session_id);
    }

    pub(super) fn take_killed(&self, session_id: &str) -> bool {
        let Some(state) = self.session_state(session_id) else {
            return false;
        };
        let was_killed = {
            let mut state = state.lock().unwrap();
            let was_killed = state.killed;
            state.last_exit_was_killed = was_killed;
            state.killed = false;
            was_killed
        };
        self.prune_empty_session_state(session_id);
        was_killed
    }

    pub(super) fn clear_killed(&self, session_id: &str) {
        if let Some(state) = self.session_state(session_id) {
            state.lock().unwrap().killed = false;
        }
        self.prune_empty_session_state(session_id);
    }
}
