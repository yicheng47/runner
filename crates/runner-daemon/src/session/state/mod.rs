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
        update: Box<runner_terminal::program_status::Update>,
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
        cancel: bool,
    },
    /// Terminal output reached the session.
    Output,
    /// The window for a submit-busy to show a turn has run out.
    SubmitTimeout {
        live: bool,
    },
    #[cfg(test)]
    DraftInput(Option<LocalInputClass>),
    /// An input's bytes never reached the PTY. `after` is where the input left the model; what a
    /// later event has changed since keeps that change.
    InputWriteFailed {
        before: Box<InputCheckpoint>,
        after: Box<InputCheckpoint>,
    },
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
    RestoreUnread(Option<i64>),
    Exited {
        code: Option<i32>,
        crashed: bool,
    },
    StartupError(runner_core::protocol::status::StartupError),
    ClearStartupError,
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
    /// True only when an authoritative OSC 7501 root turn end (`State::Done`,
    /// or `working` → `idle` under `program_status_idle_completes`) just
    /// settled a turn, distinct from wait-overlay or child-detail
    /// republications of an already-settled `Completed` observation.
    pub program_done: bool,
    /// True when `program_done` was optimistically inferred from root
    /// `working` → `idle` (OpenCode) and may be undone if a late native outcome
    /// reports that same turn as failed or interrupted.
    pub program_idle_completed: bool,
    /// True when a late native failure/interruption overrides an unread
    /// completion inferred from `working` → `idle`.
    pub revert_unread: bool,
    /// True when the reducer confirms the pending optimistic idle completion
    /// has been finalized (every participating session succeeded, or the user
    /// viewed the session) so the manager may drop its durable CAS rollback
    /// snapshot.
    pub clear_optimistic_completion: bool,
    pub persist_key: Option<PersistKey>,
}

/// A busy that rests on a submit or a delivery alone, until the agent shows a turn.
#[derive(Clone, Debug, PartialEq)]
struct SubmitBusy {
    since: Instant,
    /// The busy it armed; any later change of activity supersedes it.
    revision: u64,
    /// What the submit overwrote, where the agent reports no status of its own.
    observation: Option<AgentObservation>,
    completion: bool,
}

/// The model fields an input changes.
pub(crate) struct InputCheckpoint {
    activity: Option<SessionActivityState>,
    submit_busy: Option<SubmitBusy>,
    observation: AgentObservation,
    suppression: bool,
    pending: bool,
    input_at: Option<Instant>,
    revision: u64,
    completion: bool,
    cancel_pending: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ProgramTurn {
    /// Whether the native OSC 7501 stream is currently inside a Working/Blocked turn.
    native_working: bool,
    /// Native `Working` starts already counted that have not yet matched a `UserPromptSubmit`.
    pending_native: usize,
    /// `UserPromptSubmit` starts already counted that have not yet matched a native `Working`.
    pending_hook: usize,
}

impl ProgramTurn {
    fn has_unmatched_native(&self) -> bool {
        self.native_working || self.pending_native > 0
    }

    fn on_working(&mut self) -> bool {
        if self.native_working {
            return false;
        }
        self.native_working = true;
        if self.pending_hook > 0 {
            self.pending_hook -= 1;
            false
        } else {
            self.pending_native = self.pending_native.saturating_add(1);
            true
        }
    }

    fn on_native_end(&mut self, terminal: bool) {
        if self.native_working {
            self.native_working = false;
        } else if terminal && self.pending_hook > 0 {
            self.pending_hook -= 1;
        }
    }

    fn on_confirmed(&mut self) -> bool {
        if self.pending_native > 0 {
            self.pending_native -= 1;
            false
        } else if self.native_working {
            // Mid-turn steer attached to an already-paired active native turn.
            true
        } else {
            self.pending_hook = self.pending_hook.saturating_add(1);
            true
        }
    }

    fn on_compacted(&mut self) {
        if !self.native_working {
            self.pending_native = 0;
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct IdleCompletedTurn {
    sessions: std::collections::BTreeSet<String>,
    /// Earliest timestamp (`wall_millis` of the previous turn's root `idle`,
    /// if any) that a DB outcome for this turn can legitimately carry; rejects
    /// delayed outcomes belonging to an earlier turn on the same session.
    min_ended_at: Option<i64>,
    prev_unread: Option<i64>,
    written_unread: Option<i64>,
    /// Set once the user views this session: suppresses unread rollback while
    /// preserving turn identity so a late Failed/Interrupted DB record still
    /// corrects the displayed outcome.
    viewed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PendingSessionOutcome {
    outcome: TurnOutcome,
    ended_at_ms: i64,
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
    native_attention: bool,
    program_status_authority: bool,
    program_status_root_app: Option<&'static str>,
    program_status_transcript_outcomes: bool,
    program_status_cancel_interrupt: bool,
    program_status_idle_completes: bool,
    program_status_suppress_child_tasks: bool,
    program_cancel_pending: bool,
    program_error_is_abort: bool,
    program_idle_completed: bool,
    program_idle_turn: Option<IdleCompletedTurn>,
    program_cancelled_sessions: std::collections::BTreeSet<String>,
    program_pending_session_outcomes: std::collections::BTreeMap<String, PendingSessionOutcome>,
    program_active_children: std::collections::BTreeSet<String>,
    program_turn_sessions: std::collections::BTreeSet<String>,
    program_turn_started_at: Option<i64>,
    program_last_turn_idle_at: Option<i64>,
    program_status_children: usize,
    program_turn: ProgramTurn,
    program_turn_outcome: Option<TurnOutcome>,
    program_status_seq: (u64, usize),
    program_status_wait: Option<(String, HumanInteraction)>,
    program_wait_overlay: Option<HumanInteraction>,
    next_program_status_wait: u64,
}

impl SessionModel {
    #[cfg(test)]
    pub fn program_status_authority(&self) -> bool {
        self.program_status_authority
    }
    pub fn set_program_status_authority(&mut self, enabled: bool) {
        self.program_status_authority = enabled;
    }
    pub fn set_program_status_root_app(&mut self, app: Option<&'static str>) {
        self.program_status_root_app = app;
    }
    pub fn set_program_status_transcript_outcomes(&mut self, enabled: bool) {
        self.program_status_transcript_outcomes = enabled;
    }
    pub fn set_program_status_cancel_interrupt(&mut self, enabled: bool) {
        self.program_status_cancel_interrupt = enabled;
    }
    pub fn set_program_status_idle_completes(&mut self, enabled: bool) {
        self.program_status_idle_completes = enabled;
    }
    pub fn set_program_status_suppress_child_tasks(&mut self, enabled: bool) {
        self.program_status_suppress_child_tasks = enabled;
    }
    pub fn set_native_attention(&mut self, enabled: bool) {
        self.native_attention = enabled;
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
            observation: self.status.observation.clone(),
            suppression: self.suppress_local_input_busy,
            pending: self.local_input_pending,
            input_at: self.last_local_input_at,
            revision: self.activity_revision,
            completion: self.completion_armed,
            cancel_pending: self.program_cancel_pending,
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

    /// True when the person currently has unsubmitted local or composer draft input.
    pub fn has_active_draft(&self) -> bool {
        self.local_input_pending
            || self
                .native_input
                .or(self.observed_input)
                .is_some_and(|observed| {
                    matches!(observed.state, InputState::Drafting | InputState::Unsent)
                })
    }

    pub fn draft_hold(&self, now: Instant) -> Option<crate::router::DeliveryReservation> {
        use crate::router::DeliveryReservation;
        match self.native_input.or(self.observed_input) {
            Some(observed) if observed.state == InputState::Drafting => {
                if self.native_input.is_some()
                    || self.last_local_input_at.is_some_and(|last| {
                        now.saturating_duration_since(last) < OBSERVED_DRAFT_HOLD_CAP
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

    /// A report to a linked chat shares the screen-observed draft cap as a fallback when
    /// empty-composer detection misses (#1409); native editor drafts keep holding without a cap.
    pub fn report_draft_hold(&self, now: Instant) -> Option<crate::router::DeliveryReservation> {
        self.draft_hold(now)
    }

    pub fn apply(&mut self, event: SessionEvent, now: Now) -> Effects {
        let mut effects = Effects::default();
        match event {
            SessionEvent::ProgramStatus {
                seq,
                subseq,
                update,
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
                // Codex with status hooks enabled overlays native waits onto
                // live hook or baseline status rather than taking over full
                // ProgramStatus (which would suppress baseline Idle before
                // hook takeover or after bridge loss).
                if self.native_attention && !self.program_status_authority {
                    return self.apply_native_attention(update.trigger, live, now);
                }
                if self.hook_status_armed {
                    return effects;
                }
                use runner_terminal::program_status::State;
                let suppress_child_tasks = self.program_status_suppress_child_tasks;
                let active_children = |app: &str| -> usize {
                    if suppress_child_tasks {
                        return 0;
                    }
                    update
                        .active_children
                        .iter()
                        .filter(|entry| entry.app.as_deref() == Some(app))
                        .map(|entry| entry.active)
                        .sum()
                };
                let report = if let Some(app) = self.program_status_root_app {
                    match update.trigger {
                        Some(trigger) => {
                            if !trigger.id.is_empty() {
                                if trigger.state == State::Clear {
                                    if trigger.app.is_some() && trigger.app.as_deref() != Some(app)
                                    {
                                        return effects;
                                    }
                                    self.program_active_children.remove(&trigger.id);
                                } else {
                                    if trigger.app.as_deref() != Some(app) {
                                        return effects;
                                    }
                                    if matches!(trigger.state, State::Working | State::Blocked) {
                                        self.program_active_children.insert(trigger.id.clone());
                                        self.program_turn_sessions.insert(trigger.id.clone());
                                    } else {
                                        self.program_active_children.remove(&trigger.id);
                                    }
                                }
                                // Child records only contribute background-task detail text and
                                // never alter root status, unread badges or crew delivery (#1051).
                                let count = active_children(app);
                                if count == self.program_status_children {
                                    return effects;
                                }
                                self.program_status_children = count;
                                if let Some(evidence) =
                                    self.status.observation.program_status.as_mut()
                                {
                                    evidence.background_tasks = count;
                                    let state = self.activity.unwrap_or(SessionActivityState::Idle);
                                    effects.publication =
                                        Some((state, StatusSource::ProgramStatus));
                                }
                                return effects;
                            }
                            if trigger.state == State::Clear {
                                if trigger.app.is_some() && trigger.app.as_deref() != Some(app) {
                                    return effects;
                                }
                                self.program_status_children = 0;
                                self.program_active_children.clear();
                                None
                            } else {
                                if trigger.app.as_deref() != Some(app) {
                                    return effects;
                                }
                                self.program_status_children = active_children(app);
                                Some(trigger)
                            }
                        }
                        None => match update.root {
                            Some(root) if root.app.as_deref() == Some(app) => {
                                // Root survived a prompt/reset boundary (e.g. Done/Error across
                                // shell 133;D+133;A) while active children may have been cleared.
                                let count = active_children(app);
                                if count != self.program_status_children {
                                    self.program_status_children = count;
                                    if let Some(evidence) =
                                        self.status.observation.program_status.as_mut()
                                    {
                                        evidence.background_tasks = count;
                                        let state =
                                            self.activity.unwrap_or(SessionActivityState::Idle);
                                        effects.publication =
                                            Some((state, StatusSource::ProgramStatus));
                                    }
                                }
                                return effects;
                            }
                            _ => {
                                self.program_status_children = 0;
                                None
                            }
                        },
                    }
                } else {
                    self.program_status_children = 0;
                    update.current
                };
                let Some(report) = report else {
                    self.program_status_wait = None;
                    self.program_turn = ProgramTurn::default();
                    self.program_turn_outcome = None;
                    self.program_cancel_pending = false;
                    self.program_error_is_abort = false;
                    self.program_idle_completed = false;
                    self.program_idle_turn = None;
                    self.program_cancelled_sessions.clear();
                    self.program_pending_session_outcomes.clear();
                    self.program_active_children.clear();
                    self.program_turn_sessions.clear();
                    self.program_turn_started_at = None;
                    self.program_last_turn_idle_at = None;
                    self.program_status_children = 0;
                    if self.status.observation.program_status.is_some() {
                        let completion_armed = self.completion_armed;
                        let mut fallback = AgentObservation {
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
                        if let Some(overlay) = &self.program_wait_overlay {
                            fallback.interactions.push(overlay.clone());
                        }
                        effects = self.publish_snapshot(fallback, live, now);
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
                let report_state = report.state;
                let was_working = self.program_turn.native_working;
                let was_native_active =
                    self.program_turn.native_working || self.program_status_wait.is_some();
                let started_turn = match report_state {
                    State::Working => self.program_turn.on_working(),
                    State::Blocked => false,
                    State::Idle => {
                        self.program_turn.on_native_end(false);
                        false
                    }
                    State::Done | State::Error => {
                        self.program_turn.on_native_end(true);
                        false
                    }
                    State::Clear => {
                        self.program_turn = ProgramTurn::default();
                        false
                    }
                };
                let is_abort_message =
                    report.message.as_deref() == Some("This operation was aborted");
                let cancelled_abort_error = report_state == State::Error
                    && self.program_status_cancel_interrupt
                    && is_abort_message
                    && (self.program_cancel_pending
                        || (was_native_active
                            && self.program_turn_outcome == Some(TurnOutcome::Interrupted)));
                let mut idle_completed_now = false;
                match report_state {
                    State::Idle => {
                        let prev_turn_idle_at = self.program_last_turn_idle_at;
                        if was_working {
                            self.program_last_turn_idle_at = Some(now.wall_millis);
                        }
                        if self.program_status_cancel_interrupt
                            && std::mem::take(&mut self.program_cancel_pending)
                        {
                            let mut sessions = std::mem::take(&mut self.program_turn_sessions);
                            sessions.extend(self.program_active_children.iter().cloned());
                            self.program_turn_outcome = Some(TurnOutcome::Interrupted);
                            self.program_error_is_abort = false;
                            self.program_idle_completed = false;
                            self.program_idle_turn = None;
                            self.program_cancelled_sessions = sessions;
                            self.program_pending_session_outcomes
                                .retain(|_, pending| pending.ended_at_ms >= now.wall_millis);
                        } else if was_working {
                            self.program_cancelled_sessions.clear();
                            let mut sessions = std::mem::take(&mut self.program_turn_sessions);
                            sessions.extend(self.program_active_children.iter().cloned());
                            if self.program_turn_outcome.is_none() && !sessions.is_empty() {
                                let mut matched: Option<TurnOutcome> = None;
                                for sid in &sessions {
                                    if let Some(pending) =
                                        self.program_pending_session_outcomes.remove(sid)
                                    {
                                        if prev_turn_idle_at
                                            .is_none_or(|prev| pending.ended_at_ms >= prev)
                                        {
                                            matched = match (matched, pending.outcome) {
                                                (Some(TurnOutcome::Failed), _)
                                                | (_, TurnOutcome::Failed) => {
                                                    Some(TurnOutcome::Failed)
                                                }
                                                (_, other) => Some(other),
                                            };
                                        }
                                    }
                                }
                                if let Some(outcome) = matched {
                                    self.program_turn_outcome = Some(outcome);
                                }
                            }
                            self.program_pending_session_outcomes
                                .retain(|_, pending| pending.ended_at_ms >= now.wall_millis);
                            if self.program_status_idle_completes
                                && self.program_turn_outcome.is_none()
                            {
                                idle_completed_now = true;
                                self.program_idle_completed = true;
                                self.program_idle_turn = Some(IdleCompletedTurn {
                                    sessions,
                                    min_ended_at: prev_turn_idle_at,
                                    prev_unread: self.status.unread_since,
                                    written_unread: None,
                                    viewed: false,
                                });
                            } else {
                                self.program_idle_completed = false;
                                self.program_idle_turn = None;
                            }
                        }
                    }
                    State::Blocked => {}
                    // Delayed Working pairing with an already-confirmed turn must not erase
                    // that same turn's transcript outcome (e.g. UserPromptSubmit -> turn_aborted
                    // ahead of PTY Working -> Idle) or an in-flight Esc/Ctrl+C cancel.
                    State::Working if !started_turn => {
                        if self.program_turn_started_at.is_none() {
                            self.program_turn_started_at = Some(now.wall_millis);
                        }
                    }
                    State::Working => {
                        self.program_cancel_pending = false;
                        self.program_error_is_abort = false;
                        self.program_idle_completed = false;
                        self.program_turn_outcome = None;
                        self.program_turn_started_at = Some(now.wall_millis);
                        self.program_turn_sessions = self.program_active_children.clone();
                    }
                    State::Error if cancelled_abort_error => {
                        self.program_cancel_pending = false;
                        self.program_error_is_abort = false;
                        self.program_idle_completed = false;
                        self.program_idle_turn = None;
                        self.program_turn_sessions.clear();
                        self.program_turn_outcome = Some(TurnOutcome::Interrupted);
                    }
                    State::Error if self.program_status_transcript_outcomes => {
                        self.program_cancel_pending = false;
                        self.program_error_is_abort = is_abort_message;
                        self.program_idle_completed = false;
                        self.program_idle_turn = None;
                        self.program_turn_sessions.clear();
                        self.program_turn_outcome = Some(TurnOutcome::Failed);
                    }
                    State::Error => {
                        self.program_cancel_pending = false;
                        self.program_error_is_abort = is_abort_message;
                        self.program_idle_completed = false;
                        self.program_idle_turn = None;
                        self.program_turn_sessions.clear();
                        self.program_turn_outcome = None;
                    }
                    _ => {
                        self.program_cancel_pending = false;
                        self.program_error_is_abort = false;
                        self.program_idle_completed = false;
                        self.program_idle_turn = None;
                        self.program_turn_sessions.clear();
                        self.program_turn_outcome = None;
                    }
                }
                let mut observation = AgentObservation {
                    activity: if report_state == State::Working {
                        Activity::Working
                    } else {
                        Activity::Idle
                    },
                    source: ObservationSource::Baseline,
                    outcome: match report_state {
                        State::Done => Some(TurnOutcome::Completed),
                        State::Error if cancelled_abort_error => Some(TurnOutcome::Interrupted),
                        State::Error => Some(TurnOutcome::Failed),
                        State::Idle if idle_completed_now || self.program_idle_completed => {
                            Some(TurnOutcome::Completed)
                        }
                        State::Idle => self.program_turn_outcome,
                        _ => None,
                    },
                    program_status: Some(runner_core::protocol::status::ProgramStatusEvidence {
                        message: report.message.map(|message| {
                            message.chars().filter(|c| !matches!(*c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')).collect()
                        }),
                        background_tasks: self.program_status_children,
                    }),
                    ..Default::default()
                };
                if report_state == State::Blocked {
                    let reason = Self::wait_reason(report.kind.as_deref());
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
                    if let Some(overlay) = &self.program_wait_overlay {
                        observation.interactions.push(overlay.clone());
                    }
                }
                let completion_armed = self.completion_armed;
                effects = self.publish_snapshot(observation, live, now);
                if self.program_status_authority {
                    if report_state == State::Idle && !idle_completed_now {
                        self.completion_armed = false;
                    }
                } else {
                    self.completion_armed = completion_armed;
                }
                effects.program_turn_started = started_turn && self.program_status_authority;
                effects.program_done = self.program_status_authority
                    && (report_state == State::Done || idle_completed_now);
                effects.program_idle_completed =
                    self.program_status_authority && idle_completed_now;
                effects.publication = effects
                    .publication
                    .map(|(state, _)| (state, StatusSource::ProgramStatus));
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
                self.program_turn = ProgramTurn::default();
                self.program_turn_outcome = None;
                self.program_cancel_pending = false;
                self.program_error_is_abort = false;
                self.program_idle_completed = false;
                self.program_idle_turn = None;
                self.program_cancelled_sessions.clear();
                self.program_pending_session_outcomes.clear();
                self.program_active_children.clear();
                self.program_turn_sessions.clear();
                self.program_turn_started_at = None;
                self.program_last_turn_idle_at = None;
                self.program_status_children = 0;
                self.program_status_seq = (0, 0);
                self.program_status_wait = None;
                self.program_wait_overlay = None;
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
                self.program_turn = ProgramTurn::default();
                self.program_turn_outcome = None;
                self.program_cancel_pending = false;
                self.program_error_is_abort = false;
                self.program_idle_completed = false;
                self.program_idle_turn = None;
                self.program_cancelled_sessions.clear();
                self.program_pending_session_outcomes.clear();
                self.program_active_children.clear();
                self.program_turn_sessions.clear();
                self.program_turn_started_at = None;
                self.program_last_turn_idle_at = None;
                self.program_status_children = 0;
                self.program_status_wait = None;
                self.program_wait_overlay = None;
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
                if let agent::AgentEvent::ProgramSessionStarted {
                    conversation_changed,
                    compacted,
                } = event
                {
                    if compacted {
                        self.program_turn.on_compacted();
                    }
                    if conversation_changed {
                        let cleared = self.apply_program_wait_overlay(false, live, now);
                        if cleared.publication.is_some() || cleared.input_cleared {
                            effects = cleared;
                        }
                    }
                    if conversation_changed
                        && (!self.program_turn.has_unmatched_native()
                            || self.program_turn_outcome == Some(TurnOutcome::Interrupted))
                    {
                        self.program_cancel_pending = false;
                        self.program_error_is_abort = false;
                        self.program_idle_completed = false;
                        self.program_turn_outcome = None;
                        if self.program_status_authority
                            && !self.hook_status_armed
                            && self.status.observation.outcome.is_some()
                            && !matches!(
                                self.status.lifecycle,
                                Lifecycle::Stopped | Lifecycle::Error
                            )
                        {
                            let mut observation = self.status.observation.clone();
                            observation.outcome = None;
                            let input_cleared = effects.input_cleared;
                            effects = self.publish_snapshot(observation, live, now);
                            effects.input_cleared |= input_cleared;
                            self.completion_armed = false;
                            effects.publication = effects
                                .publication
                                .map(|(state, _)| (state, StatusSource::ProgramStatus));
                        }
                    }
                    return effects;
                }
                if let agent::AgentEvent::ProgramTurnConfirmed { .. } = event {
                    let started = self.program_turn.on_confirmed();
                    if started {
                        self.program_cancel_pending = false;
                        self.program_error_is_abort = false;
                        self.program_idle_completed = false;
                        self.program_turn_outcome = None;
                        self.program_turn_started_at = Some(now.wall_millis);
                    }
                    if self.program_status_authority
                        && (started || self.program_turn.native_working)
                    {
                        self.submit_busy = None;
                    }
                    effects.program_turn_started = started && self.program_status_authority;
                    return effects;
                }
                if matches!(event, agent::AgentEvent::ProgramTurnFinished) {
                    self.program_idle_completed = false;
                    self.program_idle_turn = None;
                    effects.clear_optimistic_completion = true;
                    return effects;
                }
                if let agent::AgentEvent::ProgramSessionFinished {
                    session_id,
                    ended_at_ms,
                    superseded,
                } = event
                {
                    if superseded {
                        return effects;
                    }
                    self.program_cancelled_sessions.remove(&session_id);
                    if self
                        .program_pending_session_outcomes
                        .get(&session_id)
                        .is_some_and(|pending| pending.ended_at_ms <= ended_at_ms)
                    {
                        self.program_pending_session_outcomes.remove(&session_id);
                    }
                    if let Some(idle_turn) = self.program_idle_turn.as_mut() {
                        if idle_turn.min_ended_at.is_some_and(|min| ended_at_ms < min) {
                            return effects;
                        }
                        if !idle_turn.sessions.is_empty() && !idle_turn.sessions.remove(&session_id)
                        {
                            return effects;
                        }
                        if idle_turn.sessions.is_empty() {
                            self.program_idle_turn = None;
                            self.program_idle_completed = false;
                            effects.clear_optimistic_completion = true;
                        }
                    } else if !self.program_turn.native_working
                        && self.program_status_wait.is_none()
                    {
                        self.program_idle_completed = false;
                        effects.clear_optimistic_completion = true;
                    }
                    return effects;
                }
                if matches!(event, agent::AgentEvent::ProgramIdentityUnavailable) {
                    return self.apply_program_wait_overlay(false, live, now);
                }
                if matches!(event, agent::AgentEvent::ProgramOutcomeUnavailable) {
                    return effects;
                }
                if let agent::AgentEvent::ProgramWaitOverlay { active } = event {
                    return self.apply_program_wait_overlay(active, live, now);
                }
                if let agent::AgentEvent::ProgramSessionOutcome {
                    outcome,
                    policy,
                    session_id,
                    ended_at_ms,
                    superseded,
                } = event
                {
                    if !self.program_status_authority
                        || self.hook_status_armed
                        || matches!(self.status.lifecycle, Lifecycle::Stopped | Lifecycle::Error)
                    {
                        return effects;
                    }
                    let has_native = self.status.observation.program_status.is_some();
                    if has_native
                        && (self.program_turn.native_working || self.program_status_wait.is_some())
                    {
                        // Only treat the session as a confirmed participant of the active turn
                        // once its child record has identified it; when program_turn_sessions is
                        // empty (between root working and the first child record), buffer the
                        // outcome per session_id so foreign sessions cannot poison the turn.
                        let matches_current_session =
                            self.program_turn_sessions.contains(&session_id);
                        let matches_idle = self.program_idle_turn.as_ref().is_some_and(|turn| {
                            (turn.sessions.is_empty() || turn.sessions.contains(&session_id))
                                && turn.min_ended_at.is_none_or(|min| ended_at_ms >= min)
                        });
                        let predates_current_turn = self
                            .program_turn_started_at
                            .is_some_and(|started| ended_at_ms < started);
                        if outcome == TurnOutcome::Interrupted
                            && predates_current_turn
                            && self.program_cancelled_sessions.remove(&session_id)
                        {
                            return effects;
                        }
                        let belongs_to_current = !superseded
                            && matches_current_session
                            && !(matches_idle && predates_current_turn);
                        if belongs_to_current {
                            self.program_cancelled_sessions.remove(&session_id);
                            if policy != agent::ProgramOutcomePolicy::NativeFallbackOnly {
                                self.program_turn_outcome = Some(outcome);
                            }
                            return effects;
                        }
                        // Only roll back the previous idle-completed turn when the DB record
                        // clearly belongs to that earlier turn (superseded by newer work or
                        // predating the current turn's start); otherwise buffer ambiguous
                        // current-session evidence until child membership identifies the turn.
                        if matches_idle && (superseded || predates_current_turn) {
                            if let Some(idle_turn) = self.program_idle_turn.take() {
                                if !idle_turn.viewed {
                                    if idle_turn.written_unread.is_some()
                                        && self.status.unread_since == idle_turn.written_unread
                                    {
                                        self.status.unread_since = idle_turn.prev_unread;
                                    }
                                    effects.revert_unread = true;
                                }
                            }
                            return effects;
                        }
                        if !superseded
                            && policy != agent::ProgramOutcomePolicy::NativeFallbackOnly
                            && self
                                .program_last_turn_idle_at
                                .is_none_or(|prev| ended_at_ms >= prev)
                        {
                            self.program_pending_session_outcomes.insert(
                                session_id,
                                PendingSessionOutcome {
                                    outcome,
                                    ended_at_ms,
                                },
                            );
                        }
                        return effects;
                    }
                    if has_native {
                        match policy {
                            agent::ProgramOutcomePolicy::NativeFallbackOnly => return effects,
                            agent::ProgramOutcomePolicy::DeferWhileNativeActive => {
                                if superseded {
                                    return effects;
                                }
                                // Reconcile immediately only against an observed idle-completed
                                // turn on this TUI. Consume a matching DB Interrupted confirmation
                                // against an already-settled cancelled turn instead of buffering it
                                // onto the next turn; otherwise buffer per session_id so genuinely
                                // early outcomes for a newer turn reconcile when its OSC arrives.
                                let matches_idle_turn = self.program_idle_completed
                                    && self.program_idle_turn.as_ref().is_some_and(|idle_turn| {
                                        (idle_turn.sessions.is_empty()
                                            || idle_turn.sessions.contains(&session_id))
                                            && idle_turn
                                                .min_ended_at
                                                .is_none_or(|min| ended_at_ms >= min)
                                    });
                                if !matches_idle_turn {
                                    if outcome == TurnOutcome::Interrupted
                                        && self.program_cancelled_sessions.remove(&session_id)
                                    {
                                        return effects;
                                    }
                                    self.program_cancelled_sessions.remove(&session_id);
                                    if self
                                        .program_last_turn_idle_at
                                        .is_none_or(|prev| ended_at_ms >= prev)
                                    {
                                        self.program_pending_session_outcomes.insert(
                                            session_id,
                                            PendingSessionOutcome {
                                                outcome,
                                                ended_at_ms,
                                            },
                                        );
                                    }
                                    return effects;
                                }
                            }
                            agent::ProgramOutcomePolicy::Immediate => {}
                        }
                    } else if superseded {
                        return effects;
                    }
                    let reverted_idle = self.program_idle_completed;
                    self.program_idle_completed = false;
                    let idle_turn = self.program_idle_turn.take();
                    self.program_turn.native_working = false;
                    self.program_status_wait = None;
                    self.program_cancel_pending = false;
                    self.program_error_is_abort = false;
                    self.program_turn_outcome = Some(outcome);
                    let mut observation = self.status.observation.clone();
                    observation.activity = Activity::Idle;
                    observation.source = ObservationSource::Baseline;
                    observation.outcome = Some(outcome);
                    observation.interactions.clear();
                    if let Some(overlay) = &self.program_wait_overlay {
                        observation.interactions.push(overlay.clone());
                    }
                    observation.detail = None;
                    effects = self.publish_snapshot(observation, live, now);
                    if reverted_idle {
                        if let Some(idle_turn) = idle_turn {
                            if !idle_turn.viewed {
                                if idle_turn.written_unread.is_some()
                                    && self.status.unread_since == idle_turn.written_unread
                                {
                                    self.status.unread_since = idle_turn.prev_unread;
                                }
                                effects.revert_unread = true;
                            }
                        } else {
                            effects.revert_unread = true;
                        }
                    }
                    self.completion_armed = false;
                    effects.publication = effects
                        .publication
                        .map(|(state, _)| (state, StatusSource::ProgramStatus));
                    return effects;
                }
                if let agent::AgentEvent::ProgramTurnOutcome { outcome, policy } = event {
                    if !self.program_status_authority
                        || self.hook_status_armed
                        || matches!(self.status.lifecycle, Lifecycle::Stopped | Lifecycle::Error)
                    {
                        return effects;
                    }
                    let has_native = self.status.observation.program_status.is_some();
                    match policy {
                        agent::ProgramOutcomePolicy::NativeFallbackOnly if has_native => {
                            return effects;
                        }
                        agent::ProgramOutcomePolicy::DeferWhileNativeActive if has_native => {
                            if self.program_turn.native_working
                                || self.program_status_wait.is_some()
                            {
                                self.program_turn_outcome = Some(outcome);
                                return effects;
                            }
                            if (self.status.observation.outcome == Some(TurnOutcome::Completed)
                                && !self.program_idle_completed)
                                || (self.status.observation.outcome == Some(TurnOutcome::Failed)
                                    && !self.program_error_is_abort)
                            {
                                return effects;
                            }
                        }
                        _ => {}
                    }
                    let reverted_idle_completion = std::mem::take(&mut self.program_idle_completed);
                    let idle_turn = self.program_idle_turn.take();
                    self.program_turn.native_working = false;
                    self.program_status_wait = None;
                    self.program_cancel_pending = false;
                    self.program_error_is_abort = false;
                    self.program_turn_outcome = Some(outcome);
                    let mut observation = self.status.observation.clone();
                    observation.activity = Activity::Idle;
                    observation.source = ObservationSource::Baseline;
                    observation.outcome = Some(outcome);
                    observation.interactions.clear();
                    if let Some(overlay) = &self.program_wait_overlay {
                        observation.interactions.push(overlay.clone());
                    }
                    observation.detail = None;
                    effects = self.publish_snapshot(observation, live, now);
                    if reverted_idle_completion {
                        if let Some(idle_turn) = idle_turn {
                            if idle_turn.written_unread.is_some()
                                && self.status.unread_since == idle_turn.written_unread
                            {
                                self.status.unread_since = idle_turn.prev_unread;
                            }
                        }
                        effects.revert_unread = true;
                    }
                    self.completion_armed = false;
                    effects.publication = effects
                        .publication
                        .map(|(state, _)| (state, StatusSource::ProgramStatus));
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
                self.program_turn = ProgramTurn::default();
                self.program_turn_outcome = None;
                self.program_cancel_pending = false;
                self.program_error_is_abort = false;
                self.program_idle_completed = false;
                self.program_idle_turn = None;
                self.program_cancelled_sessions.clear();
                self.program_pending_session_outcomes.clear();
                self.program_active_children.clear();
                self.program_turn_sessions.clear();
                self.program_turn_started_at = None;
                self.program_last_turn_idle_at = None;
                self.program_status_children = 0;
                self.program_status_wait = None;
                self.program_wait_overlay = None;
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
            SessionEvent::Input {
                class,
                submitted,
                cancel,
            } => {
                let completion_before = self.completion_armed;
                if cancel
                    && self.program_status_authority
                    && self.program_status_cancel_interrupt
                    && !self.hook_status_armed
                    && (self.program_turn.native_working
                        || self.program_turn.pending_hook > 0
                        || self.status.observation.activity == Activity::Working)
                {
                    self.program_cancel_pending = true;
                }
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
            SessionEvent::InputWriteFailed { before, after } => {
                let before = *before;
                fn undo<T: PartialEq>(field: &mut T, before: T, after: &T) {
                    if field == after {
                        *field = before;
                    }
                }
                // Its draft marks go unless a later event changed them.
                undo(
                    &mut self.suppress_local_input_busy,
                    before.suppression,
                    &after.suppression,
                );
                undo(
                    &mut self.local_input_pending,
                    before.pending,
                    &after.pending,
                );
                undo(
                    &mut self.last_local_input_at,
                    before.input_at,
                    &after.input_at,
                );
                undo(
                    &mut self.program_cancel_pending,
                    before.cancel_pending,
                    &after.cancel_pending,
                );
                // Every later activity report advances the revision, even one that agrees with
                // the input's busy, and then owns the turn.
                if self.activity_revision == after.revision {
                    undo(&mut self.activity, before.activity, &after.activity);
                    undo(
                        &mut self.submit_busy,
                        before.submit_busy,
                        &after.submit_busy,
                    );
                    undo(
                        &mut self.status.observation,
                        before.observation,
                        &after.observation,
                    );
                    undo(
                        &mut self.completion_armed,
                        before.completion,
                        &after.completion,
                    );
                    self.activity_revision = before.revision;
                }
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
                if let Some(idle_turn) = self.program_idle_turn.as_mut() {
                    idle_turn.prev_unread = None;
                    idle_turn.written_unread = None;
                    idle_turn.viewed = true;
                }
                effects.clear_optimistic_completion = true;
                if self.compaction_failed_since.is_some() {
                    self.compaction_failed_since = Some(None);
                }
            }
            SessionEvent::Unread { viewed } => {
                if !viewed {
                    self.status.unread_since = Some(now.wall_millis);
                    if let Some(idle_turn) = self.program_idle_turn.as_mut() {
                        idle_turn.written_unread = Some(now.wall_millis);
                    }
                }
            }
            SessionEvent::RestoreUnread(unread_since) => {
                self.status.unread_since = unread_since;
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
                self.program_turn = ProgramTurn::default();
                self.program_turn_outcome = None;
                self.program_cancel_pending = false;
                self.program_error_is_abort = false;
                self.program_idle_completed = false;
                self.program_idle_turn = None;
                self.program_cancelled_sessions.clear();
                self.program_pending_session_outcomes.clear();
                self.program_active_children.clear();
                self.program_turn_sessions.clear();
                self.program_turn_started_at = None;
                self.program_last_turn_idle_at = None;
                self.program_status_children = 0;
                self.program_status_wait = None;
                self.program_wait_overlay = None;
                self.status.observation.program_status = None;
                self.hook_status_armed = false;
                self.submit_busy = None;
                self.status.observation.interactions.clear();
            }
            SessionEvent::StartupError(error) => self.status.startup_error = Some(error),
            SessionEvent::ClearStartupError => {
                if self.status.startup_error.take().is_some() {
                    let state = self.activity.unwrap_or(SessionActivityState::Idle);
                    effects.publication = Some((state, StatusSource::Unavailable));
                }
            }
            SessionEvent::ConversationChanged(report) => effects.persist_key = Some(report),
            SessionEvent::KeyPersisted(report) => {
                self.key = Some(report);
            }
        }
        effects
    }

    fn apply_program_wait_overlay(&mut self, active: bool, live: bool, now: Now) -> Effects {
        if !self.program_status_authority
            || self.hook_status_armed
            || matches!(self.status.lifecycle, Lifecycle::Stopped | Lifecycle::Error)
        {
            return Effects::default();
        }
        if active {
            if self.program_wait_overlay.is_some() {
                return Effects::default();
            }
            self.next_program_status_wait += 1;
            let overlay = HumanInteraction {
                id: format!("program-wait:{}", self.next_program_status_wait),
                reason: WaitReason::Unknown,
                owners: Vec::new(),
                since: now.wall_millis,
            };
            self.program_wait_overlay = Some(overlay.clone());
            if self.program_status_wait.is_some() {
                return Effects::default();
            }
            let mut observation = self.status.observation.clone();
            observation.interactions = vec![overlay];
            let completion_armed = self.completion_armed;
            let mut effects = self.publish_snapshot(observation, live, now);
            self.completion_armed = completion_armed;
            effects.publication = effects
                .publication
                .map(|(state, _)| (state, StatusSource::ProgramStatus));
            return effects;
        }
        if self.program_wait_overlay.take().is_none() || self.program_status_wait.is_some() {
            return Effects::default();
        }
        let mut observation = self.status.observation.clone();
        observation.interactions.clear();
        if observation.program_status.is_none() && observation.outcome.is_none() {
            observation.activity = match self.baseline_activity {
                Some(SessionActivityState::Busy) => Activity::Working,
                Some(SessionActivityState::Idle) => Activity::Idle,
                None => Activity::Unavailable,
            };
        }
        let completion_armed = self.completion_armed;
        let mut effects = self.publish_snapshot(observation, live, now);
        self.completion_armed = completion_armed;
        effects.publication = effects
            .publication
            .map(|(state, _)| (state, StatusSource::ProgramStatus));
        effects
    }

    fn wait_reason(kind: Option<&str>) -> WaitReason {
        match kind {
            Some("permission") => WaitReason::Approval,
            Some("question") => WaitReason::Answer,
            _ => WaitReason::Unknown,
        }
    }

    fn apply_native_attention(
        &mut self,
        trigger: Option<runner_terminal::program_status::Report>,
        live: bool,
        now: Now,
    ) -> Effects {
        use runner_terminal::program_status::State;
        let Some(trigger) = trigger else {
            return Effects::default();
        };
        if !trigger.id.is_empty() {
            return Effects::default();
        }
        let reason = if trigger.state == State::Clear {
            if trigger.app.is_some() && trigger.app.as_deref() != Some("codex") {
                return Effects::default();
            }
            None
        } else {
            if trigger.app.as_deref() != Some("codex") {
                return Effects::default();
            }
            (trigger.state == State::Blocked).then(|| Self::wait_reason(trigger.kind.as_deref()))
        };
        if self.codex_attention.as_ref().map(|wait| wait.reason) == reason {
            return Effects::default();
        }
        let mut observation = self.status.observation.clone();
        if let Some(wait) = self.codex_attention.take() {
            observation.interactions.retain(|entry| entry.id != wait.id);
        }
        if reason.is_none() && observation.source != ObservationSource::Hook {
            observation.activity = match self.baseline_activity {
                Some(SessionActivityState::Busy) => Activity::Working,
                Some(SessionActivityState::Idle) => Activity::Idle,
                None => Activity::Unavailable,
            };
            observation.source = if self.baseline_activity.is_some() {
                ObservationSource::Baseline
            } else {
                ObservationSource::Unavailable
            };
        }
        if let Some(reason) = reason {
            self.next_codex_attention += 1;
            self.codex_attention = Some(HumanInteraction {
                id: format!("codex-native:{}", self.next_codex_attention),
                reason,
                owners: Vec::new(),
                since: now.wall_millis,
            });
        }
        let completion_armed = self.completion_armed;
        let mut effects = self.publish_snapshot(observation, live, now);
        self.completion_armed = completion_armed;
        effects.publication = effects
            .publication
            .map(|(state, _)| (state, StatusSource::CodexAttention));
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
            self.program_turn = ProgramTurn::default();
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
        if (self.codex_attention.is_some()
            || self.program_wait_overlay.is_some()
            || self.status.observation.program_status.is_some())
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
            self.program_turn = ProgramTurn::default();
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
        } else {
            self.program_turn_outcome = None;
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
                } else if state == SessionActivityState::Idle {
                    self.program_turn_outcome
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
