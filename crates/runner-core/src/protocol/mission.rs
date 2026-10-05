use super::*;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct StartMissionInput {
    pub crew_id: String,
    /// Optional project membership. Its cwd is used when cwd is omitted.
    #[serde(default)]
    pub project_id: Option<String>,
    pub title: String,
    /// The mission's goal. When `None` the mission starts with an empty-goal
    /// event (valid — the human may post a `human_said` signal later instead
    /// of setting a goal up front).
    #[serde(default)]
    pub goal_override: Option<String>,
    /// Working directory exposed to every session as `$MISSION_CWD`.
    /// An explicit value overrides the project's bound cwd.
    #[serde(default)]
    pub cwd: Option<String>,
}

/// A mission start whose project the caller has decided. The socket tool's
/// `StartMissionInput` converts with its project inferred from `cwd` when
/// none is named.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionStart {
    pub crew_id: String,
    pub scope: ProjectScope,
    pub title: String,
    pub goal_override: Option<String>,
    pub cwd: Option<String>,
}

impl From<StartMissionInput> for MissionStart {
    fn from(input: StartMissionInput) -> Self {
        Self {
            crew_id: input.crew_id,
            scope: ProjectScope::or_infer(input.project_id),
            title: input.title,
            goal_override: input.goal_override,
            cwd: input.cwd,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartMissionOutput {
    pub mission: Mission,
    /// Effective goal: the override, else empty.
    /// The frontend uses this to render the first event in the workspace
    /// without making a second round-trip.
    pub goal: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MissionActivityState {
    Busy,
    Idle,
}

/// One row in the Missions page list — the mission's own fields denormalized
/// with the crew name and the pending-ask count. The count comes from the
/// live `RouterRegistry` when the mission is mounted; otherwise it's
/// reconstructed from the event log (unmatched `human_question` /
/// `human_response` pairs) so post-restart and terminal-status missions
/// still surface unanswered cards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionSummary {
    pub session_statuses: Vec<(String, super::status::AgentStatus)>,
    #[serde(flatten)]
    pub mission: Mission,
    pub crew_name: String,
    pub pending_ask_count: usize,
    /// True iff at least one of the mission's session rows is `status =
    /// 'running'`. A mission without live sessions never shows the sidebar
    /// working indicator.
    pub any_session_live: bool,
    /// True iff every current, unarchived session row is `running`.
    /// Partial crash/resume states keep the sidebar mission icon muted.
    pub all_sessions_live: bool,
    /// Optional live activity projection derived from per-slot
    /// `session_status` events. `None` means the mission has no live
    /// sessions and keeps the sidebar attention slot clear.
    pub activity: Option<MissionActivityState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeMissionOutput {
    pub mission_id: String,
    pub resumed_session_ids: Vec<String>,
    pub sessions: Vec<super::session::SessionRow>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PostSignalInput {
    pub mission_id: String,
    /// Omit for the person at the app; otherwise a mission roster handle.
    #[serde(default)]
    pub from: Option<String>,
    /// Signal type. Human-originated calls keep the workspace whitelist;
    /// roster handles may emit any known type except person/app-only types.
    pub signal_type: String,
    /// Free-form JSON object carried with the signal. Annotated so schemars
    /// emits a schema with an explicit `type`; a bare `serde_json::Value`
    /// derives a typeless schema (`{}`), which strict MCP clients reject —
    /// and the rejection drops the entire advertised tool list (#240).
    #[cfg_attr(
        feature = "schemars",
        schemars(with = "std::collections::HashMap<String, serde_json::Value>")
    )]
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PostMessageInput {
    pub mission_id: String,
    /// Omit for the person at the app; otherwise a mission roster handle.
    #[serde(default)]
    pub from: Option<String>,
    pub text: String,
    /// Omit for a crew-wide channel post; set to a slot handle for a
    /// targeted message.
    #[serde(default)]
    pub to: Option<String>,
}
