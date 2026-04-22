// Shared domain types. Hand-synced with src/lib/types.ts — change one, change the other.
//
// Covers the four SQLite row shapes (arch §7.1) plus the event envelope (arch §5.2).

#![allow(dead_code)] // Types land in C1 but get consumed by C2+.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub type Timestamp = DateTime<Utc>;

pub type Ulid = String;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SignalType(pub String);

impl SignalType {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for SignalType {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for SignalType {
    fn from(s: String) -> Self {
        Self(s)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Crew {
    pub id: String,
    pub name: String,
    pub purpose: Option<String>,
    pub goal: Option<String>,
    pub orchestrator_policy: Option<serde_json::Value>,
    pub signal_types: Vec<SignalType>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Runner {
    pub id: String,
    pub crew_id: String,
    pub handle: String,
    pub display_name: String,
    pub role: String,
    pub runtime: String,
    pub command: String,
    pub args: Vec<String>,
    pub working_dir: Option<String>,
    pub system_prompt: Option<String>,
    pub env: HashMap<String, String>,
    pub lead: bool,
    pub position: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MissionStatus {
    Running,
    Completed,
    Aborted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mission {
    pub id: String,
    pub crew_id: String,
    pub title: String,
    pub status: MissionStatus,
    pub goal_override: Option<String>,
    pub cwd: Option<String>,
    pub started_at: Timestamp,
    pub stopped_at: Option<Timestamp>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Running,
    Stopped,
    Crashed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub mission_id: String,
    pub runner_id: String,
    pub status: SessionStatus,
    pub pid: Option<i64>,
    pub started_at: Option<Timestamp>,
    pub stopped_at: Option<Timestamp>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    Signal,
    Message,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: Ulid,
    pub ts: Timestamp,
    pub crew_id: String,
    pub mission_id: String,
    pub kind: EventKind,
    pub from: String,
    pub to: Option<String>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub signal_type: Option<SignalType>,
    #[serde(default)]
    pub payload: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_event_roundtrips_as_documented_envelope() {
        let json = serde_json::json!({
            "id": "01HG3K1YRG7RQ3N9ABCDEFGHJK",
            "ts": "2026-04-21T12:34:56.123Z",
            "crew_id": "01HGCREW",
            "mission_id": "01HGMSN",
            "kind": "signal",
            "from": "coder",
            "to": null,
            "type": "ask_lead",
            "payload": { "question": "?", "context": "..." }
        });

        let evt: Event = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(evt.kind, EventKind::Signal);
        assert_eq!(evt.signal_type.as_ref().unwrap().as_str(), "ask_lead");
        assert_eq!(evt.to, None);

        let back = serde_json::to_value(&evt).unwrap();
        assert_eq!(back, json);
    }

    #[test]
    fn message_event_omits_type_when_serialized() {
        let evt = Event {
            id: "01HGMSG".into(),
            ts: Utc::now(),
            crew_id: "c".into(),
            mission_id: "m".into(),
            kind: EventKind::Message,
            from: "lead".into(),
            to: Some("impl".into()),
            signal_type: None,
            payload: serde_json::json!({ "text": "hi" }),
        };
        let v = serde_json::to_value(&evt).unwrap();
        assert!(v.get("type").is_none(), "messages must omit `type`");
        assert_eq!(v["to"], "impl");
    }
}
