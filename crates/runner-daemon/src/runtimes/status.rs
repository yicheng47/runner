#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum CompactionPolicy {
    #[default]
    Passive,
    RestoreExact,
    ResumeRecordedTurn,
    KeepWorking,
    ResumeUnfinishedTurn,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum ToolEndPolicy {
    #[default]
    ResumeUnfinishedTurn,
    ResumeAfterCompaction,
    KeepWorking,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum PermissionPromptPolicy {
    #[default]
    RequestedOwners,
    VisibleCandidates,
    CorrelatedOnly,
}

#[derive(Clone, Copy, Default)]
pub struct StatusSemantics {
    pub interaction_prefix: Option<&'static str>,
    pub batch_feedback: bool,
    pub startup_ready_batch: bool,
    pub native_attention: bool,
    pub program_status_authority: bool,
    pub interruption_ends_invocation: bool,
    pub reject_events_after_interruption: bool,
    pub reject_completions_after_outcome: bool,
    pub preserve_turn_interactions: bool,
    pub tool_end: ToolEndPolicy,
    pub compaction: CompactionPolicy,
    pub compaction_requires_start: bool,
    pub finish_compaction_on_progress: bool,
    pub preserve_manual_compaction_outcome: bool,
    pub interrupted_turn_unavailable: bool,
    pub ended_session_keeps_ready: bool,
    pub interaction_close_settles_interruption: bool,
    pub permission_prompt: PermissionPromptPolicy,
    pub elicitation_uses_requests: bool,
    pub deduplicate_wait_events: bool,
    pub idle_clears_compaction: bool,
    pub detail_after_outcome: bool,
}
