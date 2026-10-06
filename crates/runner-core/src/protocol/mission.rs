use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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

/// A mission start whose project the caller has decided. The CLI's
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

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionFeedOrder {
    #[default]
    NewestFirst,
    OldestFirst,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct MissionFeedArgs {
    /// Mission ID.
    pub mission_id: String,
    /// Maximum number of events to return. Defaults to 50 and is capped at 500.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Sort order for returned events.
    #[serde(default)]
    pub order: MissionFeedOrder,
    /// Optional byte offset into events.ndjson. Use a returned next_offset as the next cursor.
    #[serde(default)]
    pub since_offset: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MissionFeed {
    pub mission_id: String,
    pub events: Vec<MissionFeedEntry>,
    pub next_offset: Option<u64>,
    pub skipped: Vec<SkippedEventLine>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MissionFeedEntry {
    pub next_offset: u64,
    pub event: Event,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SkippedEventLine {
    pub offset: u64,
    pub next_offset: u64,
    pub error: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MissionStatusSnapshot {
    pub mission: Mission,
    pub crew: Crew,
    pub sessions: Vec<SessionRow>,
    pub latest_session_status_by_handle: BTreeMap<String, SessionStatusSnapshot>,
    pub pending_asks: Vec<PendingAskSnapshot>,
    pub pending_ask_count: usize,
    pub live_session_count: usize,
    pub stopped_session_count: usize,
    pub crashed_session_count: usize,
    pub recent_warnings: Vec<MissionWarningSnapshot>,
    pub last_event_id: Option<String>,
    pub last_event_offset: Option<u64>,
    pub skipped_event_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStatusSnapshot {
    pub state: String,
    pub event_id: String,
    pub ts: Timestamp,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingAskSnapshot {
    pub question_id: String,
    pub asker: String,
    pub prompt: String,
    pub choices: Option<serde_json::Value>,
    pub on_behalf_of: Option<String>,
    pub event_id: String,
    pub ts: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionWarningSnapshot {
    pub event_id: String,
    pub ts: Timestamp,
    pub from: String,
    pub message: Option<String>,
    pub payload: serde_json::Value,
}
