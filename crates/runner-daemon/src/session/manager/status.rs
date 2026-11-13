use super::*;

impl SessionManager {
    pub(crate) fn note_forwarder_transition(
        &self,
        session_id: &str,
        state: SessionActivityState,
        source: StatusSource,
    ) -> bool {
        let session = self.session_state_or_insert(session_id);
        let mut session = session.lock().unwrap();
        let live = session.handle.is_some() && !session.killed;
        let was_idle = session.model.activity() == Some(SessionActivityState::Idle);
        let published = session
            .model
            .apply(
                SessionEvent::Transition {
                    state,
                    source,
                    live,
                },
                super::super::clock::state_now(),
            )
            .publication
            .is_some();
        let idle = session.model.activity() == Some(SessionActivityState::Idle);
        drop(session);
        if idle && !was_idle {
            self.notify_delivery_event(
                session_id,
                if source == StatusSource::Forwarder {
                    router::SessionDeliveryEvent::SilenceIdle
                } else {
                    router::SessionDeliveryEvent::Idle
                },
            );
        }
        published
    }

    pub(super) fn note_terminal_event(
        &self,
        session_id: &str,
        event: crate::runtimes::TerminalEvent,
    ) -> bool {
        use crate::runtimes::TerminalEvent;
        let session = self.session_state_or_insert(session_id);
        let mut session = session.lock().unwrap();
        let live = session.handle.is_some() && !session.killed;
        let delivery = if matches!(event, TerminalEvent::Activity(_)) {
            router::SessionDeliveryEvent::SilenceIdle
        } else {
            router::SessionDeliveryEvent::Idle
        };
        let event = match event {
            TerminalEvent::CodexAttention(blocked) => {
                SessionEvent::CodexAttention { blocked, live }
            }
            TerminalEvent::Activity(state) => SessionEvent::Transition {
                state,
                source: StatusSource::Forwarder,
                live,
            },
            TerminalEvent::Title(state) => SessionEvent::Title { state, live },
            TerminalEvent::Ready(state) => SessionEvent::Readiness { state, live },
        };
        let was_idle = session.model.activity() == Some(SessionActivityState::Idle);
        let published = session
            .model
            .apply(event, super::super::clock::state_now())
            .publication
            .is_some();
        let idle = session.model.activity() == Some(SessionActivityState::Idle);
        drop(session);
        if idle && !was_idle {
            self.notify_delivery_event(session_id, delivery);
        }
        published
    }

    pub(super) fn publish_direct_terminal_event(
        &self,
        session_id: &str,
        event: crate::runtimes::TerminalEvent,
        events: &dyn SessionEvents,
    ) {
        if let crate::runtimes::TerminalEvent::CodexAttention(blocked) = event {
            self.publish_model_event_for_launch(
                session_id,
                SessionEvent::CodexAttention {
                    blocked,
                    live: true,
                },
                None,
                events,
            );
            return;
        }
        if !self.note_terminal_event(session_id, event) {
            return;
        }
        events.status(&SessionActivityEvent {
            session_id: session_id.to_owned(),
            state: event.state().expect("attention handled above"),
            source: StatusSource::Forwarder,
            status: self.agent_status(session_id),
        });
    }

    #[cfg(test)]
    pub(crate) fn publish_mission_terminal_event(
        &self,
        session_id: &str,
        event: crate::runtimes::TerminalEvent,
        events: &dyn SessionEvents,
    ) {
        if let crate::runtimes::TerminalEvent::CodexAttention(blocked) = event {
            self.publish_model_event_for_launch(
                session_id,
                SessionEvent::CodexAttention {
                    blocked,
                    live: true,
                },
                None,
                events,
            );
            return;
        }
        if !self.note_terminal_event(session_id, event) {
            return;
        }
        let (status, sink) = {
            let session = self.session_state(session_id).unwrap();
            let session = session.lock().unwrap();
            (
                session.model.status().clone(),
                session.mission_status_sink.clone(),
            )
        };
        events.status(&SessionActivityEvent {
            session_id: session_id.to_owned(),
            state: event.state().expect("attention handled above"),
            source: StatusSource::Forwarder,
            status: status.clone(),
        });
        if let Some(sink) = sink {
            if let Err(error) = sink.append_session_status(
                event.state().expect("attention handled above"),
                StatusSource::Forwarder,
                &status,
            ) {
                log::warn!("append spawn session_status failed for {session_id}: {error}");
            }
        }
    }

    pub(crate) fn synthesize_wake_busy(&self, session_id: &str, draft: EventDraft) -> Result<()> {
        let session = self
            .session_state(session_id)
            .ok_or_else(|| Error::msg(format!("session not found: {session_id}")))?;
        let (sink, activity_revision) = {
            let session = session.lock().unwrap();
            if session.model.activity() == Some(SessionActivityState::Busy) {
                return Ok(());
            }
            let sink = session.mission_status_sink.clone().ok_or_else(|| {
                Error::msg(format!("session has no mission status sink: {session_id}"))
            })?;
            (sink, session.model.revision())
        };
        #[cfg(test)]
        if let Some(baseline_read) = self.wake_baseline_read.lock().unwrap().take() {
            let _ = baseline_read.send(());
        }
        match sink.try_append_with_retry(draft) {
            Ok(()) => {}
            Err(TryAppendError::Contended) => return Err(Error::msg("event log busy")),
            Err(TryAppendError::Failed(error)) => return Err(error.into()),
        }
        let mut session = session.lock().unwrap();
        session.model.apply(
            SessionEvent::Delivered {
                revision: activity_revision,
            },
            super::super::clock::state_now(),
        );
        Ok(())
    }

    /// A confirmed report reads as busy until the chat's agent reports its
    /// turn, or the submit window withdraws it, as a confirmed inbox notice does.
    pub(crate) fn synthesize_report_busy(&self, session_id: &str) {
        let Some(session) = self.session_state(session_id) else {
            return;
        };
        let mut session = session.lock().unwrap();
        if session.model.activity() == Some(SessionActivityState::Busy) {
            return;
        }
        let revision = session.model.revision();
        session.model.apply(
            SessionEvent::Delivered { revision },
            super::super::clock::state_now(),
        );
    }

    /// Signals once the next synthetic wake has read its baseline revision and
    /// released the session lock, just before it appends.
    #[cfg(test)]
    pub(crate) fn notify_wake_baseline_read(&self) -> std::sync::mpsc::Receiver<()> {
        let (tx, rx) = std::sync::mpsc::channel();
        *self.wake_baseline_read.lock().unwrap() = Some(tx);
        rx
    }

    /// A running session with the given activity, for tests outside the manager.
    #[cfg(test)]
    pub(crate) fn test_running_activity(&self, session_id: &str, state: SessionActivityState) {
        let session = self.session_state_or_insert(session_id);
        let mut session = session.lock().unwrap();
        session.model.apply(
            SessionEvent::Transition {
                state,
                source: StatusSource::Forwarder,
                live: true,
            },
            super::super::clock::state_now(),
        );
        session.model.apply(
            SessionEvent::TestLifecycle(crate::session::status::Lifecycle::Running),
            super::super::clock::state_now(),
        );
    }

    /// A running session whose agent reports `event`, for tests outside the manager.
    #[cfg(test)]
    pub(crate) fn test_agent_event(
        &self,
        session_id: &str,
        event: super::super::state::agent::AgentEvent,
    ) {
        let session = self.session_state_or_insert(session_id);
        let mut session = session.lock().unwrap();
        session.model.apply(
            SessionEvent::Agent { event, live: true },
            super::super::clock::state_now(),
        );
        session.model.apply(
            SessionEvent::TestLifecycle(crate::session::status::Lifecycle::Running),
            super::super::clock::state_now(),
        );
    }

    /// A session launch resume is still bringing back, for tests outside the manager.
    #[cfg(test)]
    pub(crate) fn test_resuming(&self, session_id: &str) {
        self.session_state_or_insert(session_id)
            .lock()
            .unwrap()
            .model
            .apply(
                SessionEvent::TestLifecycle(crate::session::status::Lifecycle::Resuming),
                super::super::clock::state_now(),
            );
    }

    /// A live mission session whose runtime refuses to stop it, for tests outside the manager.
    #[cfg(test)]
    pub(crate) fn test_attach_live(&self, session_id: &str, mission_id: &str) {
        self.session_state_or_insert(session_id)
            .lock()
            .unwrap()
            .handle = Some(super::SessionHandle {
            startup: None,
            #[cfg(windows)]
            pending_first_turn: None,
            id: session_id.to_owned(),
            mission_id: Some(mission_id.to_owned()),
            role_id: None,
            runtime_session: crate::session::runtime::RuntimeSession {
                runtime: "test".into(),
                session_id: session_id.to_owned(),
            },
            rollout_capture: None,
            forwarder: None,
            stop: Default::default(),
        });
    }

    pub(crate) fn publish_direct_activity(
        &self,
        session_id: &str,
        state: SessionActivityState,
        source: StatusSource,
        events: &dyn SessionEvents,
    ) {
        if !self.note_forwarder_transition(session_id, state, source) {
            return;
        }
        events.status(&SessionActivityEvent {
            session_id: session_id.to_string(),
            state,
            source,
            status: self.agent_status(session_id),
        });
    }

    pub(crate) fn publish_mission_activity(
        &self,
        session_id: &str,
        state: SessionActivityState,
        source: StatusSource,
        events: &dyn SessionEvents,
    ) {
        if !self.note_forwarder_transition(session_id, state, source) {
            return;
        }
        let (status, sink) = {
            let session = self.session_state(session_id).unwrap();
            let session = session.lock().unwrap();
            (
                session.model.status().clone(),
                session.mission_status_sink.clone(),
            )
        };
        events.status(&SessionActivityEvent {
            session_id: session_id.to_string(),
            state,
            source,
            status: status.clone(),
        });
        if let Some(sink) = sink {
            if let Err(error) = sink.append_session_status(state, source, &status) {
                log::warn!("append spawn session_status failed for {session_id}: {error}");
            }
        }
    }

    pub(crate) fn arm_completion(&self, session_id: &str) {
        self.session_state_or_insert(session_id)
            .lock()
            .unwrap()
            .model
            .apply(
                SessionEvent::ArmCompletion,
                super::super::clock::state_now(),
            );
    }

    pub(crate) fn take_completion_armed(&self, session_ids: &[String]) -> bool {
        let sessions: Vec<_> = {
            let sessions = self.sessions.lock().unwrap();
            session_ids
                .iter()
                .filter_map(|id| sessions.get(id).cloned())
                .collect()
        };
        let mut armed = false;
        for session in sessions {
            armed |= session
                .lock()
                .unwrap()
                .model
                .apply(
                    SessionEvent::TakeCompletion,
                    super::super::clock::state_now(),
                )
                .completion_consumed;
        }
        armed
    }

    pub fn mark_status_viewed(&self, session_ids: &[String]) {
        for id in session_ids {
            if let Some(session) = self.session_state(id) {
                session
                    .lock()
                    .unwrap()
                    .model
                    .apply(SessionEvent::Viewed, super::super::clock::state_now());
            }
        }
    }

    pub(crate) fn record_unread(&self, session_id: &str, viewed: bool) {
        if let Some(session) = self.session_state(session_id) {
            session.lock().unwrap().model.apply(
                SessionEvent::Unread { viewed },
                super::super::clock::state_now(),
            );
        }
    }

    pub(super) fn record_exit_status(
        &self,
        session_id: &str,
        exit_code: Option<i32>,
        crashed: bool,
    ) {
        if let Some(session) = self.session_state(session_id) {
            let mut session = session.lock().unwrap();
            session.finish_companion_terminals();
            if let Some(terminal) = &session.terminal {
                terminal.clear_program_status();
            }
            session.model.apply(
                SessionEvent::Exited {
                    code: exit_code,
                    crashed,
                },
                super::super::clock::state_now(),
            );
        }
    }

    pub fn agent_status(&self, session_id: &str) -> AgentStatus {
        self.session_state(session_id)
            .map(|session| session.lock().unwrap().model.status().clone())
            .unwrap_or_default()
    }

    pub fn status_snapshot(&self) -> BTreeMap<String, AgentStatus> {
        let sessions: Vec<_> = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .map(|(id, session)| (id.clone(), Arc::clone(session)))
            .collect();
        sessions
            .into_iter()
            .filter_map(|(id, session)| {
                let session = session.lock().unwrap();
                (!session.key_only).then(|| (id, session.model.status().clone()))
            })
            .collect()
    }

    /// A submit or delivery marks the session busy before its agent has said anything. If no turn
    /// shows within the window the busy is withdrawn, so a Return that started nothing cannot
    /// hold the session undeliverable. A turn that starts later puts the session back to busy.
    pub(super) fn expire_submit_busy(
        &self,
        session_id: &str,
        launch: &Arc<AtomicBool>,
        events: &dyn SessionEvents,
    ) {
        let due = self.session_state(session_id).is_some_and(|session| {
            session
                .lock()
                .unwrap()
                .model
                .submit_busy_due(super::super::clock::now())
        });
        if due {
            self.publish_model_event_for_launch(
                session_id,
                SessionEvent::SubmitTimeout { live: true },
                Some(launch),
                events,
            );
        }
    }

    #[cfg(test)]
    pub(super) fn publish_agent_event(
        &self,
        session_id: &str,
        event: super::super::state::agent::AgentEvent,
        events: &dyn SessionEvents,
    ) -> super::super::state::agent::AdapterFeedback {
        self.publish_model_event(
            session_id,
            SessionEvent::Agent { event, live: true },
            events,
        )
    }

    #[cfg(test)]
    pub(super) fn publish_model_event(
        &self,
        session_id: &str,
        event: SessionEvent,
        events: &dyn SessionEvents,
    ) -> super::super::state::agent::AdapterFeedback {
        self.publish_model_event_for_launch(session_id, event, None, events)
    }

    pub(super) fn publish_model_event_for_launch(
        &self,
        session_id: &str,
        mut event: SessionEvent,
        launch: Option<&Arc<AtomicBool>>,
        events: &dyn SessionEvents,
    ) -> super::super::state::agent::AdapterFeedback {
        let Some(session) = self.session_state(session_id) else {
            return Default::default();
        };
        let composer_cleared = matches!(
            &event,
            SessionEvent::Agent {
                event: super::super::state::agent::AgentEvent::EditorDraft { drafting: false },
                ..
            }
        );
        // A recovered conversation drops status to the output baseline, so the
        // startup hold stops waiting for a hook turn end that may never come.
        let bridge_failed = match &event {
            SessionEvent::BridgeFailed { .. } => true,
            SessionEvent::Agent { event, .. } => event.conversation_recovered(),
            _ => false,
        };
        let (status, sink, effects, startup_ready, became_idle) = {
            let mut session = session.lock().unwrap();
            if launch.is_some_and(|launch| {
                launch.load(Ordering::Acquire)
                    || session
                        .handle
                        .as_ref()
                        .is_none_or(|handle| !Arc::ptr_eq(launch, &handle.stop))
            }) {
                return Default::default();
            }
            let is_live = session.handle.is_some() && !session.killed;
            if bridge_failed && is_live {
                if let Some(terminal) = &session.terminal {
                    terminal.invalidate_program_status();
                }
            }
            match &mut event {
                SessionEvent::Agent { live, .. }
                | SessionEvent::BridgeFailed { live }
                | SessionEvent::SubmitTimeout { live }
                | SessionEvent::CodexAttention { live, .. } => *live = is_live,
                SessionEvent::ProgramStatus { live, .. } => *live = is_live,
                _ => {}
            }
            let agent_turn = matches!(&event, SessionEvent::Agent { event, live: true } if agent_turn_started(event));
            let prompted = matches!(&event, SessionEvent::Agent { event, live: true }
                if session.delivery_line.as_deref().is_some_and(|line| agent_prompted(event, line)));
            let was_idle = session.model.activity() == Some(SessionActivityState::Idle);
            let effects = session.model.apply(event, super::super::clock::state_now());
            if (agent_turn && effects.agent_feedback.accepted) || effects.program_turn_started {
                session.turn_started = session.turn_started.wrapping_add(1);
                session.delivery_prompted |= prompted;
            }
            let became_idle =
                !was_idle && session.model.activity() == Some(SessionActivityState::Idle);
            let status = session.model.status().clone();
            let startup_ready = session.handle.as_mut().is_some_and(|handle| {
                let Some(startup) = handle.startup.as_mut() else {
                    return false;
                };
                if bridge_failed {
                    startup.bridge_failed();
                }
                if startup.observation(&status.observation) {
                    handle.startup = None;
                    true
                } else {
                    false
                }
            });
            (
                status,
                session.mission_status_sink.clone(),
                effects,
                startup_ready,
                became_idle,
            )
        };
        if let Some((state, source)) = effects.publication {
            events.status(&SessionActivityEvent {
                session_id: session_id.to_owned(),
                state,
                source,
                status,
            });
            let status = self.agent_status(session_id);
            if let Some(sink) = sink {
                let draft = sink.session_status_draft(state, source, &status);
                if let Err(error) = sink.try_append_with_retry(draft) {
                    log::warn!("publish hook observation: {error:?}");
                }
            }
        }
        if became_idle {
            self.notify_delivery_event(session_id, router::SessionDeliveryEvent::Idle);
        }
        if startup_ready {
            self.notify_delivery_event(session_id, router::SessionDeliveryEvent::StartupReady);
        }
        if effects.input_cleared {
            self.notify_delivery_event(
                session_id,
                if composer_cleared {
                    router::SessionDeliveryEvent::InputCleared
                } else {
                    router::SessionDeliveryEvent::ReadinessChanged
                },
            );
        }
        effects.agent_feedback
    }

    #[cfg(test)]
    pub(super) fn publish_observation(
        &self,
        session_id: &str,
        observation: AgentObservation,
        events: &dyn SessionEvents,
    ) {
        self.publish_agent_event(
            session_id,
            super::super::state::agent::AgentEvent::Published(observation),
            events,
        );
    }

    #[cfg(test)]
    pub(super) fn status_bridge_failed(&self, session_id: &str, events: &dyn SessionEvents) {
        self.publish_model_event(
            session_id,
            SessionEvent::BridgeFailed { live: true },
            events,
        );
    }

    pub fn activity_snapshot(&self) -> BTreeMap<String, SessionActivityState> {
        let sessions: Vec<_> = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .map(|(id, session)| (id.clone(), Arc::clone(session)))
            .collect();
        sessions
            .into_iter()
            .filter_map(|(id, session)| {
                session
                    .lock()
                    .unwrap()
                    .model
                    .activity()
                    .map(|activity| (id, activity))
            })
            .collect()
    }
}

fn agent_prompted(event: &super::super::state::agent::AgentEvent, line: &str) -> bool {
    use super::super::state::agent::AgentEvent;
    match event {
        AgentEvent::PromptSubmitted { prompt } => prompt.trim() == line.trim(),
        AgentEvent::Batch { events, .. } | AgentEvent::Transcript { events, .. } => {
            events.iter().any(|event| agent_prompted(event, line))
        }
        _ => false,
    }
}

fn agent_turn_started(event: &super::super::state::agent::AgentEvent) -> bool {
    use super::super::state::agent::AgentEvent;
    match event {
        AgentEvent::TurnStarted => true,
        AgentEvent::Batch { events, .. } | AgentEvent::Transcript { events, .. } => {
            events.iter().any(agent_turn_started)
        }
        _ => false,
    }
}
