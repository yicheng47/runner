use std::collections::BTreeMap;

use crate::model::Runtime;
use crate::runtimes::{CompactionPolicy, PermissionPromptPolicy, StatusSemantics, ToolEndPolicy};
use crate::session::status::{
    Activity, AgentObservation, HumanInteraction, ObservationSource, TurnOutcome, WaitReason,
    WorkDetail,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct AdapterFeedback {
    pub(crate) accepted: bool,
    pub(crate) read_transcript: bool,
    pub(crate) compacting: bool,
    pub(crate) activity: Activity,
    pub(crate) detail: Option<WorkDetail>,
    pub(crate) outcome: Option<TurnOutcome>,
}

#[derive(Debug, Clone)]
pub enum AgentEvent {
    ConversationRecovered,
    EditorDraft {
        drafting: bool,
    },
    Ready,
    TurnStarted,
    Working {
        detail: Option<WorkDetail>,
    },
    TurnEnded {
        outcome: TurnOutcome,
    },
    /// Whether the turn that just ended left background work the agent will wake from.
    BackgroundWork {
        pending: bool,
    },
    InteractionOpened {
        reason: WaitReason,
        owners: Vec<String>,
    },
    InteractionClosed {
        owner: String,
        reason: Option<WaitReason>,
    },
    Batch {
        runtime: Runtime,
        events: Vec<AgentEvent>,
    },
    Transcript {
        runtime: Runtime,
        events: Vec<AgentEvent>,
    },
    StartupReady,
    ToolStarted {
        count: usize,
        question: Option<String>,
    },
    ToolEnded {
        owner: Option<String>,
        count: usize,
        interrupted: bool,
        transcript: bool,
    },
    CompactionStarted,
    ManualCompactionStarted,
    CompactionEnded,
    PermissionRequested {
        reason: WaitReason,
        owners: Vec<String>,
    },
    PermissionPrompt {
        candidates: Vec<(String, WaitReason)>,
    },
    ElicitationRequested {
        id: Option<String>,
        server: String,
    },
    ElicitationClosed {
        id: Option<String>,
        server: Option<String>,
    },
    ElicitationPrompt {
        owners: Vec<String>,
    },
    IdlePrompt,
    SessionEnded,
    Outcome {
        outcome: TurnOutcome,
    },
    Settled,
    ReplaceInteraction {
        reason: WaitReason,
        owner: String,
    },
    ClearInteractions,
    RejectedToolResult {
        owner: String,
        count: usize,
        rejected: bool,
    },
    RejectionSettled,
    AbortSettled,
    LocalCancel {
        kind: u8,
    },
    #[cfg(test)]
    Published(AgentObservation),
}

impl AgentEvent {
    pub(crate) fn conversation_recovered(&self) -> bool {
        matches!(self, Self::ConversationRecovered)
            || matches!(self, Self::Batch { events, .. } if events.iter().any(Self::conversation_recovered))
    }

    pub(crate) fn needs_feedback(&self) -> bool {
        match self {
            Self::Batch { runtime, .. } | Self::Transcript { runtime, .. } => {
                crate::runtimes::adapter(*runtime)
                    .status_semantics()
                    .batch_feedback
            }
            _ => false,
        }
    }

    pub(crate) fn startup_ready(&self) -> bool {
        matches!(self, Self::Batch { runtime, events }
            if crate::runtimes::adapter(*runtime).status_semantics().startup_ready_batch
                && matches!(events.first(), Some(Self::StartupReady)))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TurnState {
    pub activity: Activity,
    pub source: ObservationSource,
    pub outcome: Option<TurnOutcome>,
    pub interactions: Vec<HumanInteraction>,
    pub detail: Option<WorkDetail>,
    pub background: bool,
}

impl TurnState {
    pub(crate) fn needs_you(&self) -> bool {
        !self.interactions.is_empty()
    }
    pub(crate) fn published(&self) -> AgentObservation {
        AgentObservation {
            activity: self.activity,
            source: self.source,
            outcome: self.outcome,
            interactions: self.interactions.clone(),
            detail: self.detail,
            program_status: None,
            background: self.background,
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct AgentModel {
    pub(crate) value: TurnState,
    semantics: Option<StatusSemantics>,
    tools: usize,
    compacting: bool,
    pub(crate) compaction_resume: Option<(Activity, Option<TurnOutcome>)>,
    pending_outcome: Option<TurnOutcome>,
    cancelled_tool_result: bool,
    interrupted_invocation: bool,
    permission_owners: Vec<String>,
    elicitations: BTreeMap<String, String>,
    next_interaction: u64,
}

impl AgentModel {
    #[cfg(test)]
    pub(crate) fn reduce(&mut self, event: AgentEvent, now: i64) -> Option<AgentObservation> {
        self.reduce_with_feedback(event, now).0
    }

    pub(crate) fn reduce_with_feedback(
        &mut self,
        event: AgentEvent,
        now: i64,
    ) -> (Option<AgentObservation>, AdapterFeedback) {
        #[cfg(test)]
        if let AgentEvent::Published(observation) = event {
            return (Some(observation), self.feedback(true));
        }
        let changed_only = matches!(event, AgentEvent::Transcript { .. });
        let before = self.value.clone();
        let accepted = if let AgentEvent::Batch { runtime, events }
        | AgentEvent::Transcript { runtime, events } = event
        {
            let semantics = crate::runtimes::adapter(runtime).status_semantics();
            self.semantics = Some(semantics);
            let mut accepted = false;
            for event in events {
                accepted |= self.apply(event, semantics, now);
            }
            accepted
        } else {
            self.apply(event, self.semantics(), now)
        };
        if accepted {
            self.value.source = ObservationSource::Hook;
        }
        (
            (accepted && (!changed_only || before != self.value)).then(|| self.value.published()),
            self.feedback(accepted),
        )
    }

    fn feedback(&self, accepted: bool) -> AdapterFeedback {
        AdapterFeedback {
            accepted,
            read_transcript: self.tools != 0
                || self.value.needs_you()
                || self.cancelled_tool_result,
            compacting: self.compacting,
            activity: self.value.activity,
            detail: self.value.detail,
            outcome: self.value.outcome,
        }
    }

    #[cfg(test)]
    pub(crate) fn cancelled_tool_result(&self) -> bool {
        self.cancelled_tool_result
    }

    fn apply(&mut self, event: AgentEvent, semantics: StatusSemantics, now: i64) -> bool {
        if matches!(event, AgentEvent::ConversationRecovered) {
            *self = Self {
                semantics: Some(semantics),
                ..Default::default()
            };
            return false;
        }
        // The Stop that reports background work is the turn's last word on it, whatever
        // the interruption guards below make of the rest of its batch.
        if let AgentEvent::BackgroundWork { pending } = event {
            let changed = self.value.background != pending;
            self.value.background = pending;
            return changed;
        }
        if semantics.interruption_ends_invocation
            && self.interrupted_invocation
            && !matches!(event, AgentEvent::TurnStarted)
        {
            return false;
        }
        if semantics.reject_events_after_interruption
            && self.value.outcome == Some(TurnOutcome::Interrupted)
            && !matches!(
                event,
                AgentEvent::TurnStarted
                    | AgentEvent::StartupReady
                    | AgentEvent::SessionEnded
                    | AgentEvent::AbortSettled
                    | AgentEvent::ManualCompactionStarted
            )
        {
            return false;
        }
        if semantics.reject_completions_after_outcome
            && self.value.outcome.is_some()
            && matches!(
                event,
                AgentEvent::ToolEnded { .. } | AgentEvent::CompactionEnded
            )
        {
            return false;
        }
        match event {
            AgentEvent::ConversationRecovered | AgentEvent::BackgroundWork { .. } => unreachable!(),
            AgentEvent::EditorDraft { .. } => return false,
            AgentEvent::StartupReady => {
                self.clear_turn(true);
                self.value.activity = Activity::Idle;
                self.value.outcome = None;
                self.value.background = false;
            }
            AgentEvent::Ready => self.value.activity = Activity::Ready,
            AgentEvent::TurnStarted => {
                self.clear_turn(!semantics.preserve_turn_interactions);
                self.interrupted_invocation = false;
                self.value.background = false;
                self.work();
            }
            AgentEvent::Working { detail } => {
                self.work();
                self.value.detail = detail;
            }
            AgentEvent::ToolStarted { count, question } => {
                if semantics.finish_compaction_on_progress {
                    self.finish_compaction(semantics);
                }
                self.tools = count;
                self.work();
                if let Some(owner) = question {
                    self.wait(semantics, WaitReason::Answer, vec![owner], now);
                }
            }
            AgentEvent::ToolEnded {
                owner,
                count,
                interrupted,
                transcript,
            } => {
                let compact_working = if semantics.finish_compaction_on_progress && !transcript {
                    self.finish_compaction(semantics)
                } else {
                    None
                };
                self.tools = count;
                if let Some(owner) = owner {
                    self.close(&owner, None);
                    self.permission_owners.retain(|id| *id != owner);
                }
                if interrupted {
                    self.value.activity = if self.value.needs_you() {
                        Activity::Unavailable
                    } else {
                        Activity::Ready
                    };
                    self.value.outcome = Some(TurnOutcome::Interrupted);
                } else if semantics.tool_end == ToolEndPolicy::ResumeAfterCompaction {
                    if self.value.outcome == Some(TurnOutcome::Interrupted) {
                        if !self.value.needs_you() {
                            self.value.activity = Activity::Ready;
                        }
                    } else if !transcript && compact_working != Some(false) {
                        self.work();
                    }
                } else if semantics.tool_end == ToolEndPolicy::KeepWorking {
                    self.value.activity = Activity::Working;
                } else if !transcript && self.value.outcome.is_none() {
                    self.work();
                }
                self.update_detail(semantics);
            }
            AgentEvent::CompactionStarted | AgentEvent::ManualCompactionStarted => {
                if !self.compacting {
                    self.compaction_resume = Some((self.value.activity, self.value.outcome));
                }
                self.compacting = true;
                self.work();
            }
            AgentEvent::CompactionEnded => {
                if semantics.compaction_requires_start && !self.compacting {
                    return false;
                }
                self.finish_compaction(semantics);
                self.update_detail(semantics);
            }
            AgentEvent::TurnEnded { outcome } => {
                let manual = (semantics.preserve_manual_compaction_outcome)
                    .then(|| {
                        self.compaction_resume.filter(|(activity, outcome)| {
                            *activity != Activity::Working || outcome.is_some()
                        })
                    })
                    .flatten();
                self.clear_turn(!semantics.preserve_turn_interactions);
                self.value.activity = if semantics.interrupted_turn_unavailable
                    && outcome == TurnOutcome::Interrupted
                {
                    Activity::Unavailable
                } else {
                    Activity::Ready
                };
                if let Some((_, outcome)) = manual {
                    self.value.outcome = outcome;
                } else if self.value.outcome != Some(TurnOutcome::Interrupted) {
                    self.value.outcome = Some(outcome);
                }
            }
            AgentEvent::SessionEnded => {
                self.clear_turn(true);
                self.value.background = false;
                self.value.activity =
                    if semantics.ended_session_keeps_ready && self.value.outcome.is_some() {
                        Activity::Ready
                    } else {
                        Activity::Unavailable
                    };
            }
            AgentEvent::LocalCancel { kind: _ } => {
                if self.value.source != ObservationSource::Hook
                    || (self.value.activity != Activity::Working
                        && !(!semantics.interruption_ends_invocation && self.value.needs_you()))
                {
                    return false;
                }
                self.interrupted_invocation = semantics.interruption_ends_invocation;
                self.value.activity = if self.value.needs_you() {
                    Activity::Unavailable
                } else {
                    Activity::Ready
                };
                self.value.outcome = Some(TurnOutcome::Interrupted);
                self.compacting = false;
                self.compaction_resume = None;
                self.update_detail(semantics);
            }
            AgentEvent::AbortSettled => {
                if self.value.outcome != Some(TurnOutcome::Interrupted)
                    || self.value.activity != Activity::Unavailable
                {
                    return false;
                }
                self.value.activity = Activity::Ready;
            }
            AgentEvent::RejectedToolResult {
                owner,
                count,
                rejected,
            } => {
                self.tools = count;
                self.close(&owner, None);
                self.permission_owners.retain(|id| *id != owner);
                if rejected {
                    self.cancelled_tool_result = true;
                    self.value.activity = if self.value.needs_you() {
                        Activity::Unavailable
                    } else {
                        Activity::Ready
                    };
                    self.value.outcome = Some(TurnOutcome::Interrupted);
                }
                self.update_detail(semantics);
            }
            AgentEvent::RejectionSettled => {
                if !self.cancelled_tool_result || self.value.needs_you() {
                    return false;
                }
                self.value.activity = Activity::Ready;
                self.cancelled_tool_result = false;
            }
            AgentEvent::InteractionOpened { reason, owners } => {
                self.wait(semantics, reason, owners, now);
            }
            AgentEvent::InteractionClosed { owner, reason } => {
                self.close(&owner, reason);
                self.permission_owners.retain(|id| *id != owner);
                if semantics.interaction_close_settles_interruption
                    && self.value.outcome == Some(TurnOutcome::Interrupted)
                    && !self.value.needs_you()
                {
                    self.value.activity = Activity::Ready;
                }
                self.update_detail(semantics);
            }
            AgentEvent::PermissionRequested { reason, owners } => {
                if self.value.activity != Activity::Working
                    || self.value.outcome.is_some()
                    || owners.is_empty()
                {
                    return false;
                }
                for owner in &owners {
                    if !self.permission_owners.contains(owner) {
                        self.permission_owners.push(owner.clone());
                    }
                }
                if semantics.permission_prompt == PermissionPromptPolicy::CorrelatedOnly {
                    return false;
                }
                self.wait(semantics, reason, owners, now);
            }
            AgentEvent::PermissionPrompt { candidates } => {
                if self.value.activity != Activity::Working {
                    return false;
                }
                let mut owners: Vec<_> = self
                    .permission_owners
                    .iter()
                    .filter(|owner| candidates.iter().any(|(id, _)| id == *owner))
                    .cloned()
                    .collect();
                if semantics.permission_prompt == PermissionPromptPolicy::VisibleCandidates {
                    for (owner, reason) in &candidates {
                        if *reason != WaitReason::Unknown && !owners.contains(owner) {
                            owners.push(owner.clone());
                        }
                    }
                }
                if semantics.permission_prompt == PermissionPromptPolicy::CorrelatedOnly
                    && owners.is_empty()
                {
                    return false;
                }
                let reason = if !owners.is_empty()
                    && owners.iter().all(|owner| {
                        candidates
                            .iter()
                            .any(|(id, reason)| id == owner && *reason == WaitReason::Answer)
                    }) {
                    WaitReason::Answer
                } else {
                    WaitReason::Approval
                };
                if semantics.deduplicate_wait_events {
                    return self.wait(semantics, reason, owners, now);
                }
                if owners.is_empty()
                    || owners.iter().any(|owner| {
                        !self
                            .value
                            .interactions
                            .iter()
                            .any(|wait| wait.owners.contains(owner))
                    })
                {
                    self.wait(semantics, reason, owners, now);
                } else {
                    return false;
                }
            }
            AgentEvent::ElicitationRequested { id, server } => {
                let id = id.unwrap_or_else(|| {
                    self.next_interaction += 1;
                    format!("server:{server}:{}", self.next_interaction)
                });
                self.elicitations.insert(id, server);
                return false;
            }
            AgentEvent::ElicitationClosed { id, server } => {
                let id = id.or_else(|| {
                    let server = server?;
                    let mut candidates = self
                        .elicitations
                        .iter()
                        .filter(|(_, owner)| **owner == server);
                    let (id, _) = candidates.next()?;
                    candidates.next().is_none().then(|| id.clone())
                });
                let Some(id) = id else {
                    return false;
                };
                if self.elicitations.remove(&id).is_none() {
                    return false;
                }
                self.close(&id, None);
                if self.value.outcome.is_none() {
                    self.work();
                }
            }
            AgentEvent::ElicitationPrompt { owners } => {
                let owners = if semantics.elicitation_uses_requests {
                    if self.value.activity != Activity::Working {
                        return false;
                    }
                    self.elicitations.keys().cloned().collect()
                } else {
                    owners
                };
                if owners.is_empty() {
                    return false;
                }
                if !self.wait(semantics, WaitReason::Answer, owners, now)
                    && semantics.deduplicate_wait_events
                {
                    return false;
                }
            }
            AgentEvent::IdlePrompt => {
                if self.value.needs_you() {
                    return false;
                }
                if semantics.finish_compaction_on_progress {
                    self.finish_compaction(semantics);
                }
                self.value.activity = Activity::Ready;
                if semantics.idle_clears_compaction {
                    self.compacting = false;
                    self.compaction_resume = None;
                }
                self.update_detail(semantics);
            }
            AgentEvent::Outcome { outcome } => {
                self.pending_outcome = Some(outcome);
                return false;
            }
            AgentEvent::Settled => {
                self.tools = 0;
                self.compacting = false;
                self.compaction_resume = None;
                self.value.activity = Activity::Ready;
                self.value.outcome = self.pending_outcome.take();
                self.value.detail = None;
            }
            AgentEvent::ReplaceInteraction { reason, owner } => {
                if self.value.interactions.len() == 1 && self.value.interactions[0].reason == reason
                {
                    return false;
                }
                self.value.interactions.clear();
                self.wait(semantics, reason, vec![owner], now);
            }
            AgentEvent::ClearInteractions => {
                if self.value.interactions.is_empty() {
                    return false;
                }
                self.value.interactions.clear();
            }
            AgentEvent::Batch { .. } | AgentEvent::Transcript { .. } => unreachable!(),
            #[cfg(test)]
            AgentEvent::Published(_) => unreachable!(),
        }
        true
    }

    fn semantics(&self) -> StatusSemantics {
        self.semantics
            .unwrap_or_else(crate::runtimes::unbatched_status_semantics)
    }

    fn work(&mut self) {
        self.value.activity = Activity::Working;
        self.value.outcome = None;
        self.cancelled_tool_result = false;
        self.update_detail(self.semantics());
    }
    fn clear_turn(&mut self, interactions: bool) {
        self.tools = 0;
        self.compacting = false;
        self.compaction_resume = None;
        self.pending_outcome = None;
        self.cancelled_tool_result = false;
        if interactions {
            self.value.interactions.clear();
        }
        self.permission_owners.clear();
        self.elicitations.clear();
        self.value.detail = None;
    }
    fn update_detail(&mut self, semantics: StatusSemantics) {
        self.value.detail = if self.value.activity != Activity::Working
            || (!semantics.detail_after_outcome && self.value.outcome.is_some())
        {
            None
        } else if self.compacting {
            Some(WorkDetail::CompactingContext)
        } else if self.tools != 0 {
            Some(WorkDetail::UsingTools)
        } else {
            None
        };
    }
    fn finish_compaction(&mut self, semantics: StatusSemantics) -> Option<bool> {
        if semantics.finish_compaction_on_progress && !self.compacting {
            return None;
        }
        self.compacting = false;
        let resume = self.compaction_resume.take();
        let working =
            resume.map(|(activity, outcome)| activity == Activity::Working && outcome.is_none());
        match (semantics.compaction, resume) {
            (CompactionPolicy::RestoreExact, Some((activity, outcome))) => {
                self.value.activity = activity;
                self.value.outcome = outcome;
            }
            (_, Some((_, outcome))) if working == Some(false) => {
                self.value.activity = Activity::Ready;
                self.value.outcome = outcome;
            }
            (CompactionPolicy::ResumeRecordedTurn, Some(_)) => self.work(),
            (CompactionPolicy::KeepWorking, _) => self.value.activity = Activity::Working,
            (CompactionPolicy::ResumeUnfinishedTurn, _) if self.value.outcome.is_none() => {
                self.work()
            }
            _ => {}
        }
        self.update_detail(semantics);
        working
    }
    fn close(&mut self, owner: &str, reason: Option<WaitReason>) {
        self.value.interactions.retain_mut(|wait| {
            if reason.is_some_and(|reason| reason != wait.reason) {
                return true;
            }
            wait.owners.retain(|id| id != owner);
            !wait.owners.is_empty()
        });
    }
    fn wait(
        &mut self,
        semantics: StatusSemantics,
        reason: WaitReason,
        owners: Vec<String>,
        now: i64,
    ) -> bool {
        if let Some(wait) = self.value.interactions.iter_mut().find(|wait| {
            wait.reason == reason
                && (wait.owners == owners || wait.owners.iter().any(|owner| owners.contains(owner)))
        }) {
            let before = wait.owners.len();
            for owner in owners {
                if !wait.owners.contains(&owner) {
                    wait.owners.push(owner);
                }
            }
            return before != wait.owners.len();
        }
        self.next_interaction += 1;
        let prefix = semantics
            .interaction_prefix
            .expect("runtime has no interaction events");
        self.value.interactions.push(HumanInteraction {
            id: format!("{prefix}-{}", self.next_interaction),
            reason,
            owners,
            since: now,
        });
        true
    }
}
