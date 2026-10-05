use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
/// What a window is currently looking at. Granularity is mission-id /
/// session-id, not full URL — two windows on the same mission but different
/// inner tabs are still "looking at the same mission" (spec decision 1).
///
/// Serialized adjacently-tagged as
/// `{ "type": "Mission", "value": "<id>" }` so the shape survives in
/// persisted window state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum Subject {
    Mission(String),
    DirectChat(String),
}

/// One window's row in the registry. `focused_at` is the tiebreak that
/// decides primary ownership: among windows holding the same subject, the
/// largest `focused_at` wins. `subjects` is every subject the window has on
/// screen — one for a single-pane surface, one per pane in a split tab.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowEntry {
    pub label: String,
    pub subjects: Vec<Subject>,
    #[serde(default)]
    pub viewed_session_id: Option<String>,
    pub focused_at: DateTime<Utc>,
    pub focused: bool,
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SecondaryState {
    pub secondary: bool,
    pub primary_label: Option<String>,
}

pub fn allocate_label() -> String {
    format!("window-{}", ulid::Ulid::new())
}

pub fn cascade_reference(entries: &[WindowEntry], new_label: &str) -> Option<String> {
    entries
        .iter()
        .filter(|entry| entry.label != new_label)
        .max_by(|left, right| left.focused_at.cmp(&right.focused_at))
        .map(|entry| entry.label.clone())
}

pub fn is_secondary_for(
    entries: &[WindowEntry],
    my_label: &str,
    subject: &Subject,
) -> SecondaryState {
    let mut primary_focus = entries
        .iter()
        .find(|entry| entry.label == my_label)
        .map(|entry| entry.focused_at);
    let mut primary_label = None;
    for entry in entries {
        if entry.label == my_label || !entry.subjects.contains(subject) {
            continue;
        }
        if primary_focus.is_none_or(|focused_at| entry.focused_at > focused_at) {
            primary_focus = Some(entry.focused_at);
            primary_label = Some(entry.label.clone());
        }
    }
    SecondaryState {
        secondary: primary_label.is_some(),
        primary_label,
    }
}
