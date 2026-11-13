pub mod agent;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::runtime::SessionActivityState;
use super::status::{
    Activity, AgentObservation, AgentStatus, HumanInteraction, Lifecycle, ObservationSource,
    TurnOutcome, WaitReason, WorkDetail,
};

pub const CTRL_C_INTERRUPT: u8 = 1;
pub const ESCAPE_INTERRUPT: u8 = 2;
const RECENT_LOCAL_INPUT_WINDOW: Duration = Duration::from_secs(2);
const OBSERVED_DRAFT_HOLD_CAP: Duration = Duration::from_secs(10 * 60);
/// How long a busy that rests on a submit alone waits for evidence of a turn before it reverts to
/// idle. A hook arrives within a second of a real submit; the margin covers a loaded machine.
pub(crate) const SUBMIT_TURN_START_WINDOW: Duration = Duration::from_secs(10);

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
    #[serde(rename = "codex-attention")]
    CodexAttention,
    #[serde(rename = "program-status")]
    ProgramStatus,
    #[serde(rename = "input-submit")]
    InputSubmit,
    #[serde(rename = "input-timeout")]
    InputTimeout,
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
            Self::CodexAttention => "codex-attention",
            Self::ProgramStatus => "program-status",
            Self::InputSubmit => "input-submit",
            Self::InputTimeout => "input-timeout",
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
    ProgramStatus {
        seq: u64,
        subseq: usize,
        report: Option<runner_terminal::program_status::Report>,
        live: bool,
    },
    CodexAttention {
        blocked: bool,
        live: bool,
    },
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
    /// Terminal output reached the session.
    Output,
    /// The window for a submit-busy to show a turn has run out.
    SubmitTimeout {
        live: bool,
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
    StartupError(runner_core::protocol::status::StartupError),
    ConversationChanged(PersistKey),
    KeyPersisted(PersistKey),
}

#[derive(Default)]
pub(crate) struct Effects {
    pub agent_feedback: agent::AdapterFeedback,
    pub publication: Option<(SessionActivityState, StatusSource)>,
    pub input_cleared: bool,
    pub completion_consumed: bool,
    pub program_turn_started: bool,
    pub persist_key: Option<PersistKey>,
}

/// A busy that rests on a submit or a delivery alone, until the agent shows a turn.
#[derive(Clone, Debug)]
struct SubmitBusy {
    since: Instant,
    /// The busy it armed; any later change of activity supersedes it.
    revision: u64,
    /// What the submit overwrote, where the agent reports no status of its own.
    observation: Option<AgentObservation>,
    completion: bool,
}

pub(crate) struct InputCheckpoint {
    activity: Option<SessionActivityState>,
    submit_busy: Option<SubmitBusy>,
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
    submit_busy: Option<SubmitBusy>,
    local_input_pending: bool,
    observed_input: Option<ObservedInput>,
    native_input: Option<ObservedInput>,
    last_local_input_at: Option<Instant>,
    completion_armed: bool,
    compaction_failed_since: Option<Option<i64>>,
    key: Option<PersistKey>,
    agent: agent::AgentModel,
    codex_attention: Option<HumanInteraction>,
    next_codex_attention: u64,
    program_status_authority: bool,
    program_turn_active: bool,
    program_status_seq: (u64, usize),
    program_status_wait: Option<(String, HumanInteraction)>,
    next_program_status_wait: u64,
}

impl SessionModel {
    pub fn program_status_authority(&self) -> bool {
        self.program_status_authority
    }
    pub fn set_program_status_authority(&mut self, enabled: bool) {
        self.program_status_authority = enabled;
    }
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
    pub fn codex_attention_active(&self) -> bool {
        self.codex_attention.is_some()
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

    /// Busy because a submit or delivery said so, with no turn reported by the agent.
    pub fn unconfirmed_busy(&self) -> bool {
        self.activity == Some(SessionActivityState::Busy)
            && self.status.observation.activity != Activity::Working
            && !self.status.observation.needs_you()
    }

    /// Whether an armed Busy that nothing has superseded has outlived its window.
    pub fn submit_busy_due(&self, now: Instant) -> bool {
        self.submit_busy.as_ref().is_some_and(|submit| {
            self.activity == Some(SessionActivityState::Busy)
                && self.activity_revision == submit.revision
                && now.saturating_duration_since(submit.since) >= SUBMIT_TURN_START_WINDOW
        })
    }

    #[cfg(test)]
    pub fn submit_armed(&self) -> bool {
        self.submit_busy.is_some()
    }

    fn arm_submit_busy(
        &mut self,
        now: Instant,
        observation: Option<AgentObservation>,
        completion: bool,
    ) {
        self.submit_busy = Some(SubmitBusy {
            since: now,
            revision: self.activity_revision,
            observation,
            completion,
        });
    }

    pub fn is_empty(&self) -> bool {
        self.activity.is_none()
            && self.status.error_since.is_none()
            && self.status.failed_since.is_none()
            && self.status.unread_since.is_none()
            && !self.suppress_local_input_busy
            && !self.hook_status_armed
            && !self.provisional_idle
            && self.submit_busy.is_none()
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
            submit_busy: self.submit_busy.clone(),
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
            Some(observed)
                if matches!(observed.state, InputState::Drafting | InputState::Unsent) =>
            {
                false
            }
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

    /// The tracker still sees the router's own notice, even where native editor state wins readiness.
    pub fn unsent_notice(&self) -> bool {
        self.observed_input
            .is_some_and(|observed| observed.state == InputState::Unsent)
    }

    pub fn draft_hold(&self, now: Instant) -> Option<crate::router::DeliveryReservation> {
        self.draft_hold_capped(now, Some(OBSERVED_DRAFT_HOLD_CAP))
    }

    /// A report to a linked chat waits for a screen-observed draft for as long
    /// as it stays (#748), unlike an inbox notice.
    pub fn report_draft_hold(&self, now: Instant) -> Option<crate::router::DeliveryReservation> {
        self.draft_hold_capped(now, None)
    }

    fn draft_hold_capped(
        &self,
        now: Instant,
        cap: Option<Duration>,
    ) -> Option<crate::router::DeliveryReservation> {
        use crate::router::DeliveryReservation;
        match self.native_input.or(self.observed_input) {
            Some(observed) if observed.state == InputState::Drafting => {
                if self.native_input.is_some()
                    || self.last_local_input_at.is_some_and(|last| {
                        cap.is_none_or(|cap| now.saturating_duration_since(last) < cap)
                    })
                {
                    return Some(DeliveryReservation::Drafting {
                        composer_visible: observed.composer_visible,
                    });
                }
            }
            Some(observed) if observed.state == InputState::Unsent => {
                return Some(DeliveryReservation::Unsent)
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
            SessionEvent::ProgramStatus {
                seq,
                subseq,
                report,
                live,
            } => {
                if !live
                    || seq == 0
                    || (seq, subseq) <= self.program_status_seq
                    || matches!(self.status.lifecycle, Lifecycle::Stopped | Lifecycle::Error)
                {
                    return effects;
                }
                self.program_status_seq = (seq, subseq);
                if self.hook_status_armed {
                    return effects;
                }
                let Some(report) = report else {
                    self.program_status_wait = None;
                    self.program_turn_active = false;
                    if self.status.observation.program_status.is_some() {
                        let completion_armed = self.completion_armed;
                        effects = self.apply(SessionEvent::BridgeFailed { live }, now);
                        self.completion_armed = if self.program_status_authority {
                            false
                        } else {
                            completion_armed
                        };
                        effects.publication = effects
                            .publication
                            .map(|(state, _)| (state, StatusSource::ProgramStatus));
                    }
                    return effects;
                };
                use runner_terminal::program_status::State;
                let report_state = report.state;
                let started_turn = match report_state {
                    State::Working => !std::mem::replace(&mut self.program_turn_active, true),
                    State::Blocked => false,
                    State::Idle | State::Done | State::Error | State::Clear => {
                        self.program_turn_active = false;
                        false
                    }
                };
                let mut observation = AgentObservation {
                    activity: if report_state == State::Working {
                        Activity::Working
                    } else {
                        Activity::Idle
                    },
                    source: ObservationSource::Baseline,
                    outcome: match report_state {
                        State::Done => Some(TurnOutcome::Completed),
                        State::Error => Some(TurnOutcome::Failed),
                        _ => None,
                    },
                    program_status: Some(runner_core::protocol::status::ProgramStatusEvidence {
                        message: report.message.map(|message| {
                            message.chars().filter(|c| !matches!(*c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')).collect()
                        }),
                    }),
                    ..Default::default()
                };
                if report_state == State::Blocked {
                    let reason = match report.kind.as_deref() {
                        Some("permission") => WaitReason::Approval,
                        Some("question") => WaitReason::Answer,
                        _ => WaitReason::Unknown,
                    };
                    let same_wait = self
                        .program_status_wait
                        .as_ref()
                        .is_some_and(|(id, wait)| *id == report.id && wait.reason == reason);
                    if !same_wait {
                        self.next_program_status_wait += 1;
                        self.program_status_wait = Some((
                            report.id,
                            HumanInteraction {
                                id: format!("program-status:{}", self.next_program_status_wait),
                                reason,
                                owners: Vec::new(),
                                since: now.wall_millis,
                            },
                        ));
                    }
                    observation
                        .interactions
                        .push(self.program_status_wait.as_ref().unwrap().1.clone());
                } else {
                    self.program_status_wait = None;
                }
                let completion_armed = self.completion_armed;
                effects = self.publish_snapshot(observation, live, now);
                if self.program_status_authority {
                    if report_state == State::Idle {
                        self.completion_armed = false;
                    }
                } else {
                    self.completion_armed = completion_armed;
                }
                effects.program_turn_started = started_turn;
                effects.publication = effects
                    .publication
                    .map(|(state, _)| (state, StatusSource::ProgramStatus));
            }
            SessionEvent::CodexAttention { blocked, live } => {
                if !live || blocked == self.codex_attention.is_some() {
                    return effects;
                }
                let mut observation = self.status.observation.clone();
                if let Some(wait) = self.codex_attention.take() {
                    observation.interactions.retain(|entry| entry.id != wait.id);
                }
                if !blocked && observation.source != ObservationSource::Hook {
                    observation.activity = match self.baseline_activity {
                        Some(SessionActivityState::Busy) => Activity::Working,
                        Some(SessionActivityState::Idle) => Activity::Idle,
                        None => Activity::Unavailable,
                    };
                }
                if blocked {
                    self.next_codex_attention += 1;
                    self.codex_attention = Some(HumanInteraction {
                        id: format!("codex-native:{}", self.next_codex_attention),
                        reason: WaitReason::Unknown,
                        owners: Vec::new(),
                        since: now.wall_millis,
                    });
                }
                let completion_armed = self.completion_armed;
                effects = self.publish_snapshot(observation, live, now);
                self.completion_armed = completion_armed;
                effects.publication = effects
                    .publication
                    .map(|(state, _)| (state, StatusSource::CodexAttention));
            }
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
                self.codex_attention = None;
                self.program_turn_active = false;
                self.program_status_seq = (0, 0);
                self.program_status_wait = None;
                self.baseline_activity = None;
                self.hook_status_armed = false;
                self.provisional_idle = false;
                self.submit_busy = None;
                self.activity_revision = self.activity_revision.wrapping_add(1);
            }
            SessionEvent::Detached { stopped } => {
                if stopped {
                    self.status.lifecycle = Lifecycle::Stopped;
                }
                self.codex_attention = None;
                self.program_turn_active = false;
                self.program_status_wait = None;
                self.status.observation.program_status = None;
                self.status.observation.interactions.clear();
                self.activity = None;
                self.activity_revision = self.activity_revision.wrapping_add(1);
                self.suppress_local_input_busy = false;
                self.hook_status_armed = false;
                self.provisional_idle = false;
                self.submit_busy = None;
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
                if event.conversation_recovered() {
                    effects = self.apply(SessionEvent::BridgeFailed { live }, now);
                    self.completion_armed = false;
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
                self.program_turn_active = false;
                self.program_status_wait = None;
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
                let completion_before = self.completion_armed;
                if self.activity.is_some() && submitted {
                    self.suppress_local_input_busy = false;
                    if self.activity == Some(SessionActivityState::Idle) {
                        self.activity = Some(SessionActivityState::Busy);
                        let mut overwritten = None;
                        if !self.hook_status_armed
                            && self.status.observation.program_status.is_none()
                        {
                            overwritten = Some(self.status.observation.clone());
                            self.status.observation.activity = Activity::Working;
                            self.status.observation.source = ObservationSource::Baseline;
                            self.status.observation.outcome = None;
                        }
                        effects.publication =
                            Some((SessionActivityState::Busy, StatusSource::InputSubmit));
                        self.activity_revision = self.activity_revision.wrapping_add(1);
                        self.arm_submit_busy(now.monotonic, overwritten, completion_before);
                    } else {
                        // A second Return while the first has shown no turn restarts the wait for
                        // its turn. A Busy the agent confirmed has no armed submit to restart.
                        let revision = self.activity_revision;
                        if let Some(submit) = self
                            .submit_busy
                            .as_mut()
                            .filter(|submit| submit.revision == revision)
                        {
                            submit.since = now.monotonic;
                        }
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
                self.submit_busy = previous.submit_busy;
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
                if observation.composing {
                    self.last_local_input_at = Some(now.monotonic);
                }
                self.observed_input = Some(ObservedInput {
                    state: observation.state,
                    since: observation.since,
                    composer_visible: observation.composer_visible,
                });
            }
            SessionEvent::Delivered { revision } => {
                if self.activity_revision == revision {
                    let was_idle = self.activity == Some(SessionActivityState::Idle);
                    self.activity = Some(SessionActivityState::Busy);
                    self.activity_revision = self.activity_revision.wrapping_add(1);
                    if was_idle {
                        self.arm_submit_busy(now.monotonic, None, self.completion_armed);
                    }
                }
            }
            SessionEvent::Output => {
                // Output-only runtimes have no other evidence of a turn: from here the idle
                // detector returns them to idle once the output falls silent. Under hooks or a
                // native status report the output proves nothing, since Codex redraws its
                // composer on an Enter it ignores.
                if !self.hook_status_armed && self.status.observation.program_status.is_none() {
                    self.submit_busy = None;
                }
            }
            SessionEvent::SubmitTimeout { live } => {
                let Some(submit) = &self.submit_busy else {
                    return effects;
                };
                if self.activity != Some(SessionActivityState::Busy)
                    || self.activity_revision != submit.revision
                {
                    self.submit_busy = None;
                    return effects;
                }
                if !self.submit_busy_due(now.monotonic) {
                    return effects;
                }
                let submit = self.submit_busy.take().expect("checked above");
                if !live || matches!(self.status.lifecycle, Lifecycle::Stopped | Lifecycle::Error) {
                    return effects;
                }
                if let Some(observation) = submit.observation {
                    self.status.observation = observation;
                }
                self.completion_armed = submit.completion;
                self.activity = Some(SessionActivityState::Idle);
                self.activity_revision = self.activity_revision.wrapping_add(1);
                effects.publication =
                    Some((SessionActivityState::Idle, StatusSource::InputTimeout));
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
                self.codex_attention = None;
                self.program_turn_active = false;
                self.program_status_wait = None;
                self.status.observation.program_status = None;
                self.hook_status_armed = false;
                self.submit_busy = None;
                self.status.observation.interactions.clear();
            }
            SessionEvent::StartupError(error) => self.status.startup_error = Some(error),
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
        if let Some(wait) = &self.codex_attention {
            if !observation
                .interactions
                .iter()
                .any(|entry| entry.id == wait.id)
            {
                observation.interactions.push(wait.clone());
            }
        }
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
        if self.hook_status_armed {
            self.program_turn_active = false;
        }
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
                    .is_some_and(|observed| {
                        observed.state == InputState::Idle || self.native_input.is_none()
                    })
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
        // Native attention outlives byte activity and local cancellation; only
        // a subsequent Codex status report or process lifecycle can release it.
        if (self.codex_attention.is_some() || self.status.observation.program_status.is_some())
            && source == StatusSource::Forwarder
        {
            return false;
        }
        let old_status = self.status.clone();
        if source == StatusSource::Hook {
            if !live {
                return false;
            }
            self.hook_status_armed = true;
            self.program_turn_active = false;
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
