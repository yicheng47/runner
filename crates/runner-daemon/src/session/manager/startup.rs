use super::*;

// A startup paint can go quiet before the first answer. Give output-only runtimes
// a separate startup grace, then require the normal idle window after their last byte.
const STARTUP_OUTPUT_GRACE: Duration = Duration::from_secs(10);

pub(super) struct StartupDelivery {
    pub(super) runtime: String,
    /// A direct chat's gate holds only reports to it (#748); it has no inbox.
    pub(super) reports_only: bool,
    trust_cleared: bool,
    hooks: bool,
    waits_for_program_status: bool,
    status_hooks_enabled: bool,
    program_status: bool,
    seen_working: bool,
    pending_turn: bool,
    started: Instant,
    first_output: Option<Instant>,
    last_output: Option<Instant>,
    working: bool,
}

impl StartupDelivery {
    pub(super) fn new(runtime: &str, pending_turn: bool, now: Instant) -> Self {
        let adapter = crate::runtimes::for_key(runtime);
        let hooks = adapter.startup_delivery_uses_hooks(pending_turn);
        let waits_for_program_status =
            adapter.startup_delivery_waits_for_program_status(pending_turn);
        Self {
            runtime: runtime.to_owned(),
            reports_only: false,
            trust_cleared: true,
            hooks,
            waits_for_program_status,
            status_hooks_enabled: true,
            program_status: false,
            seen_working: false,
            pending_turn,
            started: now,
            first_output: None,
            last_output: None,
            working: false,
        }
    }

    /// A direct agent chat's gate after a spawn: a report waits until the agent is
    /// ready, as a slot's notice does, so it never lands before the agent reads its
    /// input or in a quiet startup screen such as a folder-trust prompt.
    pub(super) fn for_reports(runtime: &str, pending_turn: bool, now: Instant) -> Self {
        Self {
            reports_only: true,
            trust_cleared: false,
            ..Self::new(runtime, pending_turn, now)
        }
    }

    /// The same gate for a start that meets no folder-trust screen: a conversation
    /// actually resumed or forked, in a folder trusted when it first ran, or a fresh
    /// start whose runtime has already decided the folder's trust. A runtime that
    /// sends no startup hook there opens it once startup output settles (unless a
    /// first turn is pending); from the agent's first hook, the hooks decide, first
    /// turn included.
    pub(super) fn for_trusted_reports(runtime: &str, pending_turn: bool, now: Instant) -> Self {
        let gate = Self::for_reports(runtime, pending_turn, now);
        let reopen_signal = crate::runtimes::for_key(runtime).startup_hook_on_reopen();
        Self {
            trust_cleared: true,
            hooks: gate.hooks && (pending_turn || reopen_signal),
            waits_for_program_status: gate.waits_for_program_status
                && (pending_turn || reopen_signal),
            ..gate
        }
    }

    pub(super) fn with_status_hooks(mut self, enabled: bool) -> Self {
        self.status_hooks_enabled = enabled;
        self.hooks &= enabled;
        self
    }

    pub(super) fn status_hooks_enabled(&self) -> bool {
        self.status_hooks_enabled
    }

    pub(super) fn bridge_failed(&mut self) {
        self.hooks = false;
        self.waits_for_program_status = false;
        self.program_status = false;
        self.working = false;
    }

    pub(super) fn program_session_started(&mut self) {
        self.trust_cleared = true;
        if !self.pending_turn {
            self.waits_for_program_status = false;
        }
    }

    pub(super) fn program_turn_finished(&mut self) -> bool {
        if !self.hooks && !self.program_status && !self.working {
            self.trust_cleared = true;
            self.waits_for_program_status = false;
            return true;
        }
        false
    }

    pub(super) fn identity_bridge_unavailable(&mut self) {
        if self.trust_cleared {
            self.waits_for_program_status = false;
        }
    }

    pub(super) fn observation(
        &mut self,
        observation: &super::super::status::AgentObservation,
    ) -> bool {
        if observation.source == ObservationSource::Hook {
            if self.reports_only {
                self.hooks = true;
            }
            self.working = observation.activity == Activity::Working || observation.needs_you();
            return self.hooks
                && !self.working
                && matches!(observation.activity, Activity::Idle | Activity::Ready)
                && (!self.pending_turn || observation.outcome.is_some());
        }
        if !self.hooks
            && (observation.program_status.is_some()
                || matches!(
                    observation.outcome,
                    Some(
                        super::super::status::TurnOutcome::Interrupted
                            | super::super::status::TurnOutcome::Failed
                    )
                ))
        {
            if observation.program_status.is_some() {
                self.program_status = true;
            }
            if observation.activity == Activity::Working {
                self.seen_working = true;
            }
            self.working = observation.activity == Activity::Working || observation.needs_you();
            return !self.working
                && observation.activity == Activity::Idle
                && (!self.pending_turn || observation.outcome.is_some() || self.seen_working);
        }
        if self.program_status {
            self.program_status = false;
            self.working = false;
        }
        false
    }

    pub(super) fn output(&mut self, bytes: &[u8], now: Instant) {
        if !bytes.is_empty() {
            self.first_output.get_or_insert(now);
            self.last_output = Some(now);
        }
    }

    pub(super) fn output_settled(&self, now: Instant) -> bool {
        !self.hooks
            && !self.waits_for_program_status
            && !self.program_status
            && !self.working
            && self.first_output.is_some_and(|first| {
                now.duration_since(first.max(self.started)) >= STARTUP_OUTPUT_GRACE
            })
            && self.last_output.is_some_and(|last| {
                now.duration_since(last) >= super::super::pty_runtime::DEFAULT_IDLE_THRESHOLD
            })
    }
}

impl SessionManager {
    pub(super) fn startup_output(
        &self,
        session_id: &str,
        launch: &Arc<AtomicBool>,
        bytes: &[u8],
        now: Instant,
    ) {
        let Some(session) = self.session_state(session_id) else {
            return;
        };
        let mut session = session.lock().unwrap();
        if let Some(handle) = session
            .handle
            .as_mut()
            .filter(|handle| Arc::ptr_eq(launch, &handle.stop))
        {
            if let Some(startup) = handle.startup.as_mut() {
                startup.output(bytes, now);
            }
        }
    }

    pub(super) fn release_settled_startup(
        &self,
        session_id: &str,
        launch: &Arc<AtomicBool>,
        now: Instant,
    ) {
        let Some(session) = self.session_state(session_id) else {
            return;
        };
        let ready = {
            let mut session = session.lock().unwrap();
            if session.killed || launch.load(Ordering::Acquire) {
                return;
            }
            let Some(handle) = session
                .handle
                .as_mut()
                .filter(|handle| Arc::ptr_eq(launch, &handle.stop))
            else {
                return;
            };
            if handle
                .startup
                .as_ref()
                .is_some_and(|startup| startup.output_settled(now))
            {
                handle.startup = None;
                true
            } else {
                false
            }
        };
        if ready {
            self.notify_delivery_event(session_id, router::SessionDeliveryEvent::StartupReady);
        }
    }
}
