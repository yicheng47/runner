pub mod agent;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::runtime::SessionActivityState;
use super::status::{
    Activity, AgentObservation, AgentStatus, Lifecycle, ObservationSource, TurnOutcome, WorkDetail,
};

pub const CTRL_C_INTERRUPT: u8 = 1;
pub const ESCAPE_INTERRUPT: u8 = 2;
const RECENT_LOCAL_INPUT_WINDOW: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StatusSource {
    #[serde(rename = "spawn")]
    Spawn,
    #[serde(rename = "fork")]
    Fork,
    #[serde(rename = "resume")]
    Resume,
    #[serde(rename = "forwarder")]
    Forwarder,
    #[serde(rename = "input-submit")]
    InputSubmit,
    #[serde(rename = "input-interrupt")]
    InputInterrupt,
    #[serde(rename = "input-escape")]
    InputEscape,
    #[serde(rename = "hook")]
    Hook,
    #[serde(rename = "baseline")]
    Baseline,
    #[serde(rename = "unavailable")]
    Unavailable,
    #[cfg(test)]
    #[serde(rename = "wake")]
    Wake,
    #[cfg(test)]
    #[serde(rename = "test")]
    Test,
}

impl StatusSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Spawn => "spawn",
            Self::Fork => "fork",
            Self::Resume => "resume",
            Self::Forwarder => "forwarder",
            Self::InputSubmit => "input-submit",
            Self::InputInterrupt => "input-interrupt",
            Self::InputEscape => "input-escape",
            Self::Hook => "hook",
            Self::Baseline => "baseline",
            Self::Unavailable => "unavailable",
            #[cfg(test)]
            Self::Wake => "wake",
            #[cfg(test)]
            Self::Test => "test",
        }
    }
}

impl std::fmt::Display for StatusSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

pub use runner_terminal::input_state::{InputObservation, InputState};

#[derive(Clone, Copy, Debug)]
pub(crate) struct ObservedInput {
    pub state: InputState,
    pub since: Instant,
    pub composer_visible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalInputClass {
    SetPending,
    ClearPending,
    ActivityOnly,
}

#[derive(Clone, Copy)]
pub(crate) struct Now {
    pub monotonic: Instant,
    pub wall_millis: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyOrigin {
    Assigned,
    Captured,
    Rekeyed,
}

#[derive(Clone, Debug)]
pub(crate) struct PersistKey {
    pub key: Option<String>,
    pub generation: String,
    pub origin: KeyOrigin,
}

pub(crate) enum SessionEvent {
    #[cfg(test)]
    TestPending(bool),
    #[cfg(test)]
    TestInputAt(Option<Instant>),
    #[cfg(test)]
    TestObserved(ObservedInput),
    #[cfg(test)]
    TestLifecycle(Lifecycle),
    Attached,
    Detached {
        stopped: bool,
    },
    Transition {
        state: SessionActivityState,
        source: StatusSource,
        live: bool,
    },
    Title {
        state: SessionActivityState,
        live: bool,
    },
    Readiness {
        state: SessionActivityState,
        live: bool,
    },
    Agent {
        event: agent::AgentEvent,
        live: bool,
    },
    BridgeFailed {
        live: bool,
    },
    Input {
        class: Option<LocalInputClass>,
        submitted: bool,
    },
    #[cfg(test)]
    DraftInput(Option<LocalInputClass>),
    InputWriteFailed(InputCheckpoint),
    Composer {
        observation: InputObservation,
        live: bool,
    },
    Delivered {
        revision: u64,
    },
    ArmCompletion,
    TakeCompletion,
    Viewed,
    Unread {
        viewed: bool,
    },
    Exited {
        code: Option<i32>,
        crashed: bool,
    },
    ConversationChanged(PersistKey),
    KeyPersisted(PersistKey),
}

#[derive(Default)]
pub(crate) struct Effects {
    pub agent_feedback: agent::AdapterFeedback,
    pub publication: Option<(SessionActivityState, StatusSource)>,
    pub input_cleared: bool,
    pub completion_consumed: bool,
    pub persist_key: Option<PersistKey>,
}

pub(crate) struct InputCheckpoint {
    activity: Option<SessionActivityState>,
    status: AgentStatus,
    suppression: bool,
    pending: bool,
    input_at: Option<Instant>,
    revision: u64,
    completion: bool,
}

#[derive(Clone, Default)]
pub(crate) struct SessionModel {
    activity: Option<SessionActivityState>,
    status: AgentStatus,
    baseline_activity: Option<SessionActivityState>,
    activity_revision: u64,
    suppress_local_input_busy: bool,
    hook_status_armed: bool,
    provisional_idle: bool,
    local_input_pending: bool,
    observed_input: Option<ObservedInput>,
    native_input: Option<ObservedInput>,
    last_local_input_at: Option<Instant>,
    completion_armed: bool,
    compaction_failed_since: Option<Option<i64>>,
    key: Option<PersistKey>,
    agent: agent::AgentModel,
}

impl SessionModel {
    #[cfg(test)]
    pub fn completion_armed(&self) -> bool {
        self.completion_armed
    }
    #[cfg(test)]
    pub fn suppress_local_input_busy(&self) -> bool {
        self.suppress_local_input_busy
    }
    #[cfg(test)]
    pub fn provisional_idle(&self) -> bool {
        self.provisional_idle
    }
    #[cfg(all(test, unix))]
    pub fn hook_status_armed(&self) -> bool {
        self.hook_status_armed
    }
    #[cfg(test)]
    pub fn observed_input(&self) -> Option<ObservedInput> {
        self.observed_input
    }
    #[cfg(test)]
    pub fn last_local_input_at(&self) -> Option<Instant> {
        self.last_local_input_at
    }
    #[cfg(test)]
    pub fn local_input_pending(&self) -> bool {
        self.local_input_pending
    }
    pub fn status(&self) -> &AgentStatus {
        &self.status
    }
    pub fn activity(&self) -> Option<SessionActivityState> {
        self.activity
    }
    pub fn revision(&self) -> u64 {
        self.activity_revision
    }

    pub fn is_empty(&self) -> bool {
        self.activity.is_none()
            && self.status.error_since.is_none()
            && self.status.failed_since.is_none()
            && self.status.unread_since.is_none()
            && !self.suppress_local_input_busy
            && !self.hook_status_armed
            && !self.provisional_idle
            && !self.local_input_pending
            && self.observed_input.is_none()
            && self.native_input.is_none()
            && self.last_local_input_at.is_none()
            && !self.completion_armed
            && self.compaction_failed_since.is_none()
    }

    pub fn input_checkpoint(&self) -> InputCheckpoint {
        InputCheckpoint {
            activity: self.activity,
            status: self.status.clone(),
            suppression: self.suppress_local_input_busy,
            pending: self.local_input_pending,
            input_at: self.last_local_input_at,
            revision: self.activity_revision,
            completion: self.completion_armed,
        }
    }

    pub fn draft_quiescent(&self, now: Instant) -> bool {
        let observed_quiescent = match self.native_input.or(self.observed_input) {
            Some(observed) if observed.state == InputState::Drafting => false,
            Some(observed) if observed.state == InputState::Submitted => {
                now.saturating_duration_since(observed.since) >= RECENT_LOCAL_INPUT_WINDOW
            }
            Some(_) => true,
            None => !self.local_input_pending,
        };
        observed_quiescent
            && self
                .last_local_input_at
                .is_none_or(|last| now.saturating_duration_since(last) >= RECENT_LOCAL_INPUT_WINDOW)
    }

    pub fn draft_hold(&self, now: Instant) -> Option<crate::router::DeliveryReservation> {
        use crate::router::DeliveryReservation;
        match self.native_input.or(self.observed_input) {
            Some(observed) if observed.state == InputState::Drafting => {
                return Some(DeliveryReservation::Drafting {
                    composer_visible: observed.composer_visible,
                })
            }
            Some(observed) if observed.state == InputState::Submitted => {
                let elapsed = now.saturating_duration_since(observed.since);
                if elapsed < RECENT_LOCAL_INPUT_WINDOW {
                    return Some(DeliveryReservation::RecentlyTyping(
                        RECENT_LOCAL_INPUT_WINDOW - elapsed,
                    ));
                }
            }
            Some(_) => {}
            None if self.local_input_pending => {
                return Some(DeliveryReservation::LocalInputPending)
            }
            None => {}
        }
        if let Some(last) = self.last_local_input_at {
            let elapsed = now.saturating_duration_since(last);
            if elapsed < RECENT_LOCAL_INPUT_WINDOW {
                return Some(DeliveryReservation::RecentlyTyping(
                    RECENT_LOCAL_INPUT_WINDOW - elapsed,
                ));
            }
        }
        None
    }

    pub fn apply(&mut self, event: SessionEvent, now: Now) -> Effects {
        let mut effects = Effects::default();
        match event {
            #[cfg(test)]
            SessionEvent::TestPending(pending) => self.local_input_pending = pending,
            #[cfg(test)]
            SessionEvent::TestInputAt(at) => self.last_local_input_at = at,
            #[cfg(test)]
            SessionEvent::TestObserved(observed) => self.observed_input = Some(observed),
            #[cfg(test)]
            SessionEvent::TestLifecycle(lifecycle) => self.status.lifecycle = lifecycle,
            SessionEvent::Attached => {
                self.local_input_pending = false;
                self.observed_input = None;
                self.native_input = None;
                self.last_local_input_at = None;
                self.status = AgentStatus {
                    unread_since: self.status.unread_since,
                    ..Default::default()
                };
                self.compaction_failed_since = None;
                self.agent = Default::default();
                self.baseline_activity = None;
                self.hook_status_armed = false;
                self.provisional_idle = false;
                self.activity_revision = self.activity_revision.wrapping_add(1);
            }
            SessionEvent::Detached { stopped } => {
                if stopped {
                    self.status.lifecycle = Lifecycle::Stopped;
                }
                self.status.observation.interactions.clear();
                self.activity = None;
                self.activity_revision = self.activity_revision.wrapping_add(1);
                self.suppress_local_input_busy = false;
                self.hook_status_armed = false;
                self.provisional_idle = false;
                self.local_input_pending = false;
                self.observed_input = None;
                self.native_input = None;
                self.last_local_input_at = None;
                self.completion_armed = false;
            }
            SessionEvent::Transition {
                state,
                source,
                live,
            } => {
                if self.transition(state, source, live) {
                    effects.publication = Some((state, source));
                }
            }
            SessionEvent::Title { state, live } | SessionEvent::Readiness { state, live } => {
                if self.transition(state, StatusSource::Forwarder, live) {
                    effects.publication = Some((state, StatusSource::Forwarder));
                }
            }
            SessionEvent::Agent { event, live } => {
                if !live {
                    return effects;
                }
                if let agent::AgentEvent::EditorDraft { drafting } = event {
                    effects.input_cleared = !drafting
                        && self
                            .native_input
                            .is_some_and(|previous| previous.state == InputState::Drafting);
                    self.native_input = Some(ObservedInput {
                        state: if drafting {
                            InputState::Drafting
                        } else {
                            InputState::Idle
                        },
                        since: now.monotonic,
                        composer_visible: true,
                    });
                    return effects;
                }
                let (observation, feedback) =
                    self.agent.reduce_with_feedback(event, now.wall_millis);
                if let Some(observation) = observation {
                    effects = self.publish_snapshot(observation, live, now);
                }
                effects.agent_feedback = feedback;
            }
            SessionEvent::BridgeFailed { live } => {
                self.native_input = None;
                self.completion_armed = false;
                let fallback = AgentObservation {
                    activity: match self.baseline_activity {
                        Some(SessionActivityState::Busy) => Activity::Working,
                        Some(SessionActivityState::Idle) => Activity::Idle,
                        None => Activity::Unavailable,
                    },
                    source: if self.baseline_activity.is_some() {
                        ObservationSource::Baseline
                    } else {
                        ObservationSource::Unavailable
                    },
                    ..Default::default()
                };
                effects = self.publish_snapshot(fallback, live, now);
            }
            SessionEvent::Input { class, submitted } => {
                if self.activity.is_some() && submitted {
                    self.suppress_local_input_busy = false;
                    if self.activity == Some(SessionActivityState::Idle) {
                        self.activity = Some(SessionActivityState::Busy);
                        if !self.hook_status_armed {
                            self.status.observation.activity = Activity::Working;
                            self.status.observation.source = ObservationSource::Baseline;
                            self.status.observation.outcome = None;
                        }
                        effects.publication =
                            Some((SessionActivityState::Busy, StatusSource::InputSubmit));
                        self.activity_revision = self.activity_revision.wrapping_add(1);
                    }
                } else if self.activity == Some(SessionActivityState::Idle) {
                    self.suppress_local_input_busy = true;
                }
                effects.input_cleared = self.draft_input(class, now.monotonic);
                if submitted {
                    self.completion_armed = true;
                }
            }
            #[cfg(test)]
            SessionEvent::DraftInput(class) => {
                effects.input_cleared = self.draft_input(class, now.monotonic)
            }
            SessionEvent::InputWriteFailed(previous) => {
                self.activity = previous.activity;
                self.status = previous.status;
                self.suppress_local_input_busy = previous.suppression;
                self.local_input_pending = previous.pending;
                self.last_local_input_at = previous.input_at;
                self.activity_revision = previous.revision;
                self.completion_armed = previous.completion;
            }
            SessionEvent::Composer { observation, live } => {
                if !live {
                    return effects;
                }
                effects.input_cleared = self.native_input.is_none()
                    && self.observed_input.is_some_and(|previous| {
                        previous.state != InputState::Idle && observation.state == InputState::Idle
                    });
                if effects.input_cleared {
                    self.last_local_input_at = None;
                }
                self.observed_input = Some(ObservedInput {
                    state: observation.state,
                    since: observation.since,
                    composer_visible: observation.composer_visible,
                });
            }
            SessionEvent::Delivered { revision } => {
                if self.activity_revision == revision {
                    self.activity = Some(SessionActivityState::Busy);
                    self.activity_revision = self.activity_revision.wrapping_add(1);
                }
            }
            SessionEvent::ArmCompletion => self.completion_armed = true,
            SessionEvent::TakeCompletion => {
                if !self.provisional_idle {
                    effects.completion_consumed = self.completion_armed;
                    self.completion_armed = false;
                }
            }
            SessionEvent::Viewed => {
                self.status.error_since = None;
                self.status.failed_since = None;
                self.status.unread_since = None;
                if self.compaction_failed_since.is_some() {
                    self.compaction_failed_since = Some(None);
                }
            }
            SessionEvent::Unread { viewed } => {
                if !viewed {
                    self.status.unread_since = Some(now.wall_millis);
                }
            }
            SessionEvent::Exited { code, crashed } => {
                self.native_input = None;
                self.status.lifecycle = if crashed {
                    Lifecycle::Error
                } else {
                    Lifecycle::Stopped
                };
                self.status.exit_code = code;
                self.status.error_since = crashed.then_some(now.wall_millis);
                self.status.failed_since = None;
                self.compaction_failed_since = None;
                self.status.observation.interactions.clear();
            }
            SessionEvent::ConversationChanged(report) => effects.persist_key = Some(report),
            SessionEvent::KeyPersisted(report) => {
                self.key = Some(report);
            }
        }
        effects
    }

    fn publish_snapshot(
        &mut self,
        mut observation: AgentObservation,
        live: bool,
        now: Now,
    ) -> Effects {
        let mut effects = Effects::default();
        if observation.source != ObservationSource::Hook {
            observation.detail = None;
        }
        if !live
            || (self.status.lifecycle == Lifecycle::Running
                && self.status.observation == observation)
        {
            return effects;
        }
        let source = match observation.source {
            ObservationSource::Hook => StatusSource::Hook,
            ObservationSource::Baseline => StatusSource::Baseline,
            ObservationSource::Unavailable => StatusSource::Unavailable,
        };
        let released = self.status.observation.needs_you() && !observation.needs_you();
        if matches!(
            observation.outcome,
            Some(TurnOutcome::Interrupted | TurnOutcome::Failed)
        ) {
            self.completion_armed = false;
        }
        if observation.activity == Activity::Working
            && observation.outcome.is_none()
            && observation.detail != Some(WorkDetail::CompactingContext)
        {
            self.completion_armed = true;
        }
        self.hook_status_armed = observation.source == ObservationSource::Hook;
        self.status.lifecycle = Lifecycle::Running;
        let old_failed = self.status.observation.outcome == Some(TurnOutcome::Failed);
        let new_failed = observation.outcome == Some(TurnOutcome::Failed);
        let compacting = observation.detail == Some(WorkDetail::CompactingContext);
        if old_failed && compacting && self.compaction_failed_since.is_none() {
            self.compaction_failed_since = Some(self.status.failed_since);
        }
        self.status.failed_since = if new_failed
            && self.status.observation.detail == Some(WorkDetail::CompactingContext)
        {
            self.compaction_failed_since
                .take()
                .unwrap_or(Some(now.wall_millis))
        } else {
            match (old_failed, new_failed) {
                (false, true) => Some(now.wall_millis),
                (true, true) => self.status.failed_since,
                (_, false) => None,
            }
        };
        if !compacting && !new_failed {
            self.compaction_failed_since = None;
        }
        self.status.observation = observation;
        let state = if self.status.observation.activity == Activity::Working
            || self.status.observation.needs_you()
        {
            SessionActivityState::Busy
        } else {
            SessionActivityState::Idle
        };
        self.activity = Some(state);
        self.activity_revision = self.activity_revision.wrapping_add(1);

        effects.publication = Some((state, source));
        effects.input_cleared = released;
        effects
    }

    fn draft_input(&mut self, class: Option<LocalInputClass>, now: Instant) -> bool {
        match class {
            Some(LocalInputClass::SetPending) => {
                self.local_input_pending = true;
                self.last_local_input_at = Some(now);
                false
            }
            Some(LocalInputClass::ClearPending) => {
                self.local_input_pending = false;
                self.last_local_input_at = self
                    .observed_input
                    .is_some_and(|observed| observed.state == InputState::Idle)
                    .then_some(now);
                true
            }
            Some(LocalInputClass::ActivityOnly) => {
                self.last_local_input_at = Some(now);
                false
            }
            None => false,
        }
    }

    fn transition(
        &mut self,
        state: SessionActivityState,
        source: StatusSource,
        live: bool,
    ) -> bool {
        if source == StatusSource::Forwarder {
            self.baseline_activity = Some(state);
        }
        let old_status = self.status.clone();
        if source == StatusSource::Hook {
            if !live {
                return false;
            }
            self.hook_status_armed = true;
        }
        if matches!(
            source,
            StatusSource::InputInterrupt | StatusSource::InputEscape
        ) {
            if !self.hook_status_armed
                || !live
                || (self.activity != Some(SessionActivityState::Busy)
                    && !(source == StatusSource::InputInterrupt && self.provisional_idle))
            {
                return false;
            }
            self.completion_armed = false;
        }
        if self.hook_status_armed && source == StatusSource::Forwarder {
            return false;
        }
        if source == StatusSource::Forwarder
            && state == SessionActivityState::Busy
            && self.suppress_local_input_busy
        {
            return false;
        }
        let resolved_provisional_idle = self.provisional_idle
            && source == StatusSource::Hook
            && state == SessionActivityState::Idle;
        self.provisional_idle = source == StatusSource::InputEscape;
        if state == SessionActivityState::Idle {
            self.suppress_local_input_busy = false;
        }
        self.status.lifecycle = Lifecycle::Running;
        if matches!(
            source,
            StatusSource::InputInterrupt | StatusSource::InputEscape
        ) {
            self.status.observation.activity = Activity::Unavailable;
            self.status.observation.outcome = Some(TurnOutcome::Interrupted);
            self.status.observation.detail = None;
        } else if !self.hook_status_armed || source == StatusSource::Hook {
            self.status.observation.source = if source == StatusSource::Hook {
                ObservationSource::Hook
            } else {
                ObservationSource::Baseline
            };
            self.status.observation.activity = match (state, self.hook_status_armed) {
                (SessionActivityState::Busy, _) => Activity::Working,
                (SessionActivityState::Idle, true) => Activity::Ready,
                (SessionActivityState::Idle, false) => Activity::Idle,
            };
            self.status.observation.outcome =
                if source == StatusSource::Hook && state == SessionActivityState::Idle {
                    Some(TurnOutcome::Completed)
                } else {
                    None
                };
            self.status.observation.detail = None;
        }
        if self.status.observation.outcome != Some(TurnOutcome::Failed) {
            self.status.failed_since = None;
        }
        if self.status.observation.detail != Some(WorkDetail::CompactingContext) {
            self.compaction_failed_since = None;
        }
        if self.activity == Some(state) && !resolved_provisional_idle && self.status == old_status {
            return false;
        }
        self.activity = Some(state);
        self.activity_revision = self.activity_revision.wrapping_add(1);
        true
    }
}

#[cfg(test)]
mod tests;
