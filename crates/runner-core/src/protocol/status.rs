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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkDetail {
    UsingTools,
    CompactingContext,
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
    #[serde(default)]
    pub detail: Option<WorkDetail>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_status: Option<ProgramStatusEvidence>,
    /// The agent ended its turn with background work it will wake from still pending (#1183).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub background: bool,
}

/// Additive evidence keeps the existing source vocabulary readable by older clients.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramStatusEvidence {
    pub message: Option<String>,
    /// Active OSC 7501 child records (e.g. Claude Code `id=` background tasks or subagents).
    /// Surfaced as detail text only; never alters root status, unread badges or crew delivery.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub background_tasks: usize,
}

fn is_zero(value: &usize) -> bool {
    *value == 0
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_error: Option<StartupError>,
    pub lifecycle: Lifecycle,
    pub observation: AgentObservation,
    pub exit_code: Option<i32>,
    pub error_since: Option<i64>,
    #[serde(default)]
    pub failed_since: Option<i64>,
    pub unread_since: Option<i64>,
    /// A mission member mid-turn when runnerd stopped, until its next turn:
    /// when runnerd stopped (#1187).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interrupted_since: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_evidence_is_additive_and_preserves_existing_source_values() {
        let old = serde_json::json!({
            "activity": "working", "source": "baseline", "outcome": null,
            "interactions": [], "detail": null
        });
        let mut observation: AgentObservation = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(observation.program_status, None);
        observation.program_status = Some(ProgramStatusEvidence {
            message: Some("Hello".into()),
            background_tasks: 0,
        });
        let mut wire = serde_json::to_value(&observation).unwrap();
        assert_eq!(wire["source"], "baseline");
        assert_eq!(wire["program_status"]["message"], "Hello");
        assert!(wire["program_status"].get("background_tasks").is_none());
        let roundtrip: AgentObservation = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(roundtrip, observation);
        wire.as_object_mut().unwrap().remove("program_status");
        assert_eq!(wire, old);

        observation.program_status = Some(ProgramStatusEvidence {
            message: None,
            background_tasks: 2,
        });
        let with_children = serde_json::to_value(&observation).unwrap();
        assert_eq!(with_children["program_status"]["background_tasks"], 2);
        let roundtrip_children: AgentObservation = serde_json::from_value(with_children).unwrap();
        assert_eq!(roundtrip_children, observation);
    }

    #[test]
    fn background_work_is_additive() {
        let old = serde_json::json!({
            "activity": "ready", "source": "hook", "outcome": "completed",
            "interactions": [], "detail": null
        });
        let mut observation: AgentObservation = serde_json::from_value(old.clone()).unwrap();
        assert!(!observation.background);
        assert_eq!(serde_json::to_value(&observation).unwrap(), old);
        observation.background = true;
        let mut wire = serde_json::to_value(observation).unwrap();
        assert_eq!(wire["background"], true);
        wire.as_object_mut().unwrap().remove("background");
        assert_eq!(wire, old);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartupError {
    pub runtime: super::Runtime,
    pub message: String,
    pub output: String,
}
