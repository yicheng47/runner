// Event envelope and supporting types, shared between the app and the CLI.
//
// SQLite row types (Crew, Runner, Mission, Session) live in the app — the CLI
// never touches the DB. Only the on-the-wire Event shape lives here.

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

    /// `session_status` under either its current or its pre-#632 name.
    pub fn is_session_status(&self) -> bool {
        KnownSignalType::from_name(&self.0) == Some(KnownSignalType::SessionStatus)
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

pub const ASK_LEAD_REMOVED_HINT: &str =
    "ask the lead with `runner msg post --to <lead>`, or the person with `runner ask --human`";

/// Closed set of signal types the CLI recognizes for `runner signal
/// <type>`. The router still dispatches on `SignalType` (the open
/// string newtype) so future user-defined types can pass through the
/// wire envelope unchanged; this enum is just the CLI-input guardrail
/// against typos and the source of truth for the launch-prompt's
/// "Allowed signal types" line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KnownSignalType {
    MissionGoal,
    HumanSaid,
    AskHuman,
    HumanQuestion,
    HumanResponse,
    SessionStatus,
    InboxRead,
}

impl KnownSignalType {
    pub const ALL: &'static [KnownSignalType] = &[
        KnownSignalType::MissionGoal,
        KnownSignalType::HumanSaid,
        KnownSignalType::AskHuman,
        KnownSignalType::HumanQuestion,
        KnownSignalType::HumanResponse,
        KnownSignalType::SessionStatus,
        KnownSignalType::InboxRead,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            KnownSignalType::MissionGoal => "mission_goal",
            KnownSignalType::HumanSaid => "human_said",
            KnownSignalType::AskHuman => "ask_human",
            KnownSignalType::HumanQuestion => "human_question",
            KnownSignalType::HumanResponse => "human_response",
            KnownSignalType::SessionStatus => "session_status",
            KnownSignalType::InboxRead => "inbox_read",
        }
    }

    /// The pre-#632 name of `session_status`. Mission logs and briefs
    /// written before the rename still carry it, so readers accept both.
    pub const LEGACY_SESSION_STATUS: &'static str = "runner_status";

    pub fn from_name(s: &str) -> Option<Self> {
        if s == Self::LEGACY_SESSION_STATUS {
            return Some(Self::SessionStatus);
        }
        Self::ALL.iter().copied().find(|k| k.as_str() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    Signal,
    Message,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schemars", schemars(with = "Option<RecipientsWire>"))]
pub struct Recipients(Vec<String>);

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(untagged)]
enum RecipientsWire {
    One(String),
    Several(Vec<String>),
}

impl Recipients {
    pub fn new(handles: Vec<String>) -> Self {
        let mut unique = Vec::new();
        for handle in handles {
            if !unique.contains(&handle) {
                unique.push(handle);
            }
        }
        Self(unique)
    }

    pub fn as_slice(&self) -> &[String] {
        &self.0
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Option<String>> for Recipients {
    fn from(handle: Option<String>) -> Self {
        Self(handle.into_iter().collect())
    }
}

impl Serialize for Recipients {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0.as_slice() {
            [] => serializer.serialize_none(),
            [handle] => serializer.serialize_str(handle),
            handles => handles.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Recipients {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Option::<RecipientsWire>::deserialize(deserializer)? {
            None => Self::default(),
            Some(RecipientsWire::One(handle)) => Self::new(vec![handle]),
            Some(RecipientsWire::Several(handles)) => Self::new(handles),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: Ulid,
    pub ts: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crew_id: Option<String>,
    pub mission_id: String,
    pub kind: EventKind,
    pub from: String,
    #[serde(default)]
    pub to: Recipients,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub signal_type: Option<SignalType>,
    #[serde(default)]
    pub payload: serde_json::Value,
}

impl Event {
    pub fn recipients(&self) -> &[String] {
        self.to.as_slice()
    }

    pub fn addresses(&self, handle: &str) -> bool {
        self.kind == EventKind::Message
            && handle != "human"
            && self.from != handle
            && (self.recipients().is_empty()
                || self.recipients().iter().any(|target| target == handle))
    }
}

/// All of `Event`'s fields except `id` and `ts`. `EventLog::append` takes a
/// draft and assigns `id` + `ts` inside the flock so cross-process appends
/// stay monotonic.
#[derive(Debug, Clone)]
pub struct EventDraft {
    pub crew_id: Option<String>,
    pub mission_id: String,
    pub kind: EventKind,
    pub from: String,
    pub to: Recipients,
    pub signal_type: Option<SignalType>,
    pub payload: serde_json::Value,
}

impl EventDraft {
    pub fn signal(
        crew_id: Option<&str>,
        mission_id: impl Into<String>,
        from: impl Into<String>,
        signal_type: impl Into<SignalType>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            crew_id: crew_id.map(str::to_owned),
            mission_id: mission_id.into(),
            kind: EventKind::Signal,
            from: from.into(),
            to: Default::default(),
            signal_type: Some(signal_type.into()),
            payload,
        }
    }

    pub fn message(
        crew_id: Option<&str>,
        mission_id: impl Into<String>,
        from: impl Into<String>,
        to: Option<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            crew_id: crew_id.map(str::to_owned),
            mission_id: mission_id.into(),
            kind: EventKind::Message,
            from: from.into(),
            to: to.into(),
            signal_type: None,
            payload: serde_json::json!({ "text": text.into() }),
        }
    }
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
        assert!(evt.recipients().is_empty());

        let back = serde_json::to_value(&evt).unwrap();
        assert_eq!(back, json);
    }

    #[test]
    fn known_signal_type_roundtrips_through_from_name() {
        for kind in KnownSignalType::ALL {
            assert_eq!(KnownSignalType::from_name(kind.as_str()), Some(*kind));
        }
        assert_eq!(KnownSignalType::from_name("not_a_real_type"), None);
        assert_eq!(
            KnownSignalType::from_name("runner_status"),
            Some(KnownSignalType::SessionStatus)
        );
        assert!(SignalType::new("runner_status").is_session_status());
        assert!(SignalType::new("session_status").is_session_status());
        assert!(!SignalType::new("ask_human").is_session_status());
    }

    #[test]
    fn message_event_omits_type_when_serialized() {
        let evt = Event {
            id: "01HGMSG".into(),
            ts: Utc::now(),
            crew_id: Some("c".into()),
            mission_id: "m".into(),
            kind: EventKind::Message,
            from: "lead".into(),
            to: Some("impl".into()).into(),
            signal_type: None,
            payload: serde_json::json!({ "text": "hi" }),
        };
        let v = serde_json::to_value(&evt).unwrap();
        assert!(v.get("type").is_none(), "messages must omit `type`");
        assert_eq!(v["to"], "impl");
    }
}

#[cfg(test)]
mod recipients_tests {
    use super::*;

    #[test]
    fn legacy_single_and_broadcast_lines_roundtrip_and_arrays_address_all_targets() {
        for value in [
            serde_json::json!(null),
            serde_json::json!("qa"),
            serde_json::json!(["qa", "coder"]),
        ] {
            let event_json = serde_json::json!({"id":"01HG3K1YRG7RQ3N9ABCDEFGHJK","ts":"2026-04-21T12:34:56.123Z","mission_id":"m","kind":"message","from":"reviewer","to":value,"payload":{"text":"hello"}});
            let event: Event = serde_json::from_value(event_json.clone()).unwrap();
            assert_eq!(serde_json::to_value(&event).unwrap(), event_json);
            assert!(event.addresses("qa"));
            assert!(!event.addresses("reviewer"));
            assert!(!event.addresses("human"));
            assert_eq!(
                event.addresses("coder"),
                value.is_null() || value.is_array()
            );
        }
        let legacy: Event = serde_json::from_value(serde_json::json!({"id":"old","ts":"2026-04-21T12:34:56Z","mission_id":"m","kind":"message","from":"qa","payload":{"text":"omitted to"}})).unwrap();
        assert!(legacy.addresses("coder"));
        let recipients: Recipients =
            serde_json::from_value(serde_json::json!(["qa", "qa"])).unwrap();
        assert_eq!(serde_json::to_value(recipients).unwrap(), "qa");
    }
}
