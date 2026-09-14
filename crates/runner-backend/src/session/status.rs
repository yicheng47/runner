use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activity {
    Working,
    Idle,
    Ready,
    #[default]
    Unavailable,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationSource {
    Hook,
    Baseline,
    #[default]
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    Completed,
    Interrupted,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitReason {
    Approval,
    Answer,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanInteraction {
    pub id: String,
    pub reason: WaitReason,
    pub owners: Vec<String>,
    pub since: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentObservation {
    pub activity: Activity,
    pub source: ObservationSource,
    pub outcome: Option<TurnOutcome>,
    pub interactions: Vec<HumanInteraction>,
}

impl AgentObservation {
    pub fn needs_you(&self) -> bool {
        !self.interactions.is_empty()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    #[default]
    Starting,
    Resuming,
    Running,
    Stopped,
    Error,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentStatus {
    pub lifecycle: Lifecycle,
    pub observation: AgentObservation,
    pub exit_code: Option<i32>,
    pub error_since: Option<i64>,
    pub unread_since: Option<i64>,
}
