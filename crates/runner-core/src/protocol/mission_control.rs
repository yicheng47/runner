//! Read-only, project-local evidence for the Mission Control board.
//!
//! This is intentionally a snapshot rather than persisted workflow state: the
//! daemon reads the project and joins it with Runner's existing mission state.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionControlLane {
    Closed,
    NeedsYou,
    Running,
    VerifyLand,
    Blocked,
    Ready,
    Backlog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionControlSourceKind {
    Ticket,
    Triage,
    Runner,
}

/// A local path which supplied a field. Paths are evidence links, never a
/// copied ticket body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionControlEvidence {
    pub kind: MissionControlSourceKind,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionControlFieldProvenance {
    pub field: String,
    pub evidence: MissionControlEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionControlSourceHealth {
    pub kind: MissionControlSourceKind,
    pub path: String,
    pub warning: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionControlMission {
    pub id: String,
    pub title: String,
    pub active_slots: usize,
    /// The current Runner question, if one remains unanswered.
    pub pending_question: Option<String>,
    /// The newest explicit handoff in the mission feed, retained as Runner
    /// evidence rather than ticket content.
    pub newest_handoff: Option<String>,
    pub branch: Option<String>,
    pub worktree: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionControlCard {
    pub ticket_id: String,
    pub summary: String,
    pub severity: Option<String>,
    pub blockers: Vec<String>,
    pub triage_state: Option<String>,
    pub lane: MissionControlLane,
    pub mission: Option<MissionControlMission>,
    pub evidence: Vec<MissionControlEvidence>,
    pub provenance: Vec<MissionControlFieldProvenance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionControlSnapshot {
    pub project_id: String,
    pub project_path: String,
    pub cards: Vec<MissionControlCard>,
    pub health: Vec<MissionControlSourceHealth>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_round_trips_without_ticket_content() {
        let snapshot = MissionControlSnapshot {
            project_id: "project".into(),
            project_path: "/tmp/project".into(),
            cards: vec![MissionControlCard {
                ticket_id: "CR-1".into(),
                summary: "Keep only the ticket summary".into(),
                severity: Some("high".into()),
                blockers: vec!["waiting for review".into()],
                triage_state: Some("ready".into()),
                lane: MissionControlLane::Ready,
                mission: None,
                evidence: vec![MissionControlEvidence {
                    kind: MissionControlSourceKind::Ticket,
                    path: "/tmp/project/issues/CR-1.md".into(),
                }],
                provenance: vec![],
            }],
            health: vec![],
        };

        let wire = serde_json::to_value(&snapshot).unwrap();
        assert!(wire["cards"][0].get("body").is_none());
        assert_eq!(
            serde_json::from_value::<MissionControlSnapshot>(wire).unwrap(),
            snapshot
        );
    }
}
