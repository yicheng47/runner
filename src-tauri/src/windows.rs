// Cross-window coordination registry (impl 0018, spec 12).
//
// Runner is a single backend process that can drive several Tauri webview
// windows at once. The backend stays the single source of truth: one
// `SessionManager`, one `RouterRegistry`, one `BusRegistry`. The only thing
// the windows need to *coordinate* is which subject (mission / direct chat)
// each is looking at, so two windows never write to the same PTY stdin.
//
// This module owns that map. Each window reports its current `Subject` and
// its focus events; the registry computes, per subject, which window is
// **primary** — the most-recently-focused holder. Everything else (the
// Arc-style overlay, the terminal-mount gate) derives from that single rule.

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// What a window is currently looking at. Granularity is mission-id /
/// session-id, not full URL — two windows on the same mission but different
/// inner tabs are still "looking at the same mission" (spec decision 1).
///
/// Serialized adjacently-tagged so the frontend sees
/// `{ "type": "Mission", "value": "<id>" }`. Mirrored by `Subject` in
/// `src/lib/types.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum Subject {
    Mission(String),
    DirectChat(String),
}

/// One window's row in the registry. `focused_at` is the tiebreak that
/// decides primary ownership: among windows holding the same subject, the
/// largest `focused_at` wins.
#[derive(Debug, Clone, Serialize)]
pub struct WindowEntry {
    pub label: String,
    pub subject: Option<Subject>,
    pub focused_at: DateTime<Utc>,
}

/// `Mutex<HashMap<label, WindowEntry>>`, mirroring the shape used by the
/// other in-memory registries in this crate (`BusRegistry`, `RouterRegistry`).
pub struct WindowRegistry {
    entries: Mutex<HashMap<String, WindowEntry>>,
}

impl WindowRegistry {
    pub fn new() -> Self {
        WindowRegistry {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Insert a freshly-created window with no subject. Called the moment a
    /// window is built (Rust-side) so the snapshot reflects it before the
    /// webview's frontend has reported anything.
    pub fn register(&self, label: &str) {
        self.register_at(label, Utc::now());
    }

    /// Remove a window. Keyed off `WindowEvent::Destroyed` so it fires for the
    /// genuine close of a secondary window. The surviving most-recent holder
    /// of that subject (if any) becomes primary automatically.
    pub fn unregister(&self, label: &str) {
        self.entries.lock().unwrap().remove(label);
    }

    /// Update a window's subject without touching `focused_at` — navigating
    /// between routes is not a focus change. Upserts so a report that races
    /// ahead of `register` still lands.
    pub fn set_subject(&self, label: &str, subject: Option<Subject>) {
        let mut map = self.entries.lock().unwrap();
        match map.get_mut(label) {
            Some(entry) => entry.subject = subject,
            None => {
                map.insert(
                    label.to_string(),
                    WindowEntry {
                        label: label.to_string(),
                        subject,
                        focused_at: Utc::now(),
                    },
                );
            }
        }
    }

    /// Bump a window's `focused_at` to now — the OS told us this window came
    /// forward, so it now owns whatever subject it's on (spec decision 2).
    pub fn mark_focused(&self, label: &str) {
        self.mark_focused_at(label, Utc::now());
    }

    /// Snapshot of every registered window, for the `window_focus_map`
    /// broadcast and the `window_list_subjects` hydrate-on-mount path.
    pub fn snapshot(&self) -> Vec<WindowEntry> {
        let mut out: Vec<WindowEntry> = self.entries.lock().unwrap().values().cloned().collect();
        // Stable order so the broadcast payload is deterministic across
        // mutations (HashMap iteration order is not).
        out.sort_by(|a, b| a.label.cmp(&b.label));
        out
    }

    /// The primary window for a subject: among windows currently holding it,
    /// the one with the max `focused_at`. `None` if no window holds it.
    pub fn primary_for(&self, subject: &Subject) -> Option<String> {
        self.entries
            .lock()
            .unwrap()
            .values()
            .filter(|e| e.subject.as_ref() == Some(subject))
            .max_by(|a, b| a.focused_at.cmp(&b.focused_at))
            .map(|e| e.label.clone())
    }

    // --- internals / test seams -----------------------------------------

    fn register_at(&self, label: &str, focused_at: DateTime<Utc>) {
        self.entries.lock().unwrap().insert(
            label.to_string(),
            WindowEntry {
                label: label.to_string(),
                subject: None,
                focused_at,
            },
        );
    }

    /// `mark_focused` with an explicit timestamp. Upserts: a focus event for a
    /// window we somehow never registered still produces an entry rather than
    /// being dropped. Split out so tests can assert ordering without sleeping.
    fn mark_focused_at(&self, label: &str, focused_at: DateTime<Utc>) {
        let mut map = self.entries.lock().unwrap();
        match map.get_mut(label) {
            Some(entry) => entry.focused_at = focused_at,
            None => {
                map.insert(
                    label.to_string(),
                    WindowEntry {
                        label: label.to_string(),
                        subject: None,
                        focused_at,
                    },
                );
            }
        }
    }
}

impl Default for WindowRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).single().expect("valid ts")
    }

    #[test]
    fn register_unregister_round_trip() {
        let reg = WindowRegistry::new();
        reg.register("main");
        assert_eq!(reg.snapshot().len(), 1);
        assert_eq!(reg.snapshot()[0].label, "main");
        assert!(reg.snapshot()[0].subject.is_none());

        reg.unregister("main");
        assert!(reg.snapshot().is_empty());
    }

    #[test]
    fn set_subject_preserves_focused_at() {
        let reg = WindowRegistry::new();
        reg.register_at("main", ts(100));
        reg.set_subject("main", Some(Subject::Mission("m1".into())));

        let snap = reg.snapshot();
        assert_eq!(snap[0].subject, Some(Subject::Mission("m1".into())));
        // subject change is not a focus change — timestamp untouched.
        assert_eq!(snap[0].focused_at, ts(100));
    }

    #[test]
    fn mark_focused_bumps_timestamp() {
        let reg = WindowRegistry::new();
        reg.register_at("main", ts(100));
        reg.mark_focused_at("main", ts(200));
        assert_eq!(reg.snapshot()[0].focused_at, ts(200));
    }

    #[test]
    fn primary_for_returns_most_recently_focused_holder() {
        let reg = WindowRegistry::new();
        reg.register_at("main", ts(100));
        reg.register_at("window-a", ts(100));
        let subject = Subject::Mission("m1".into());
        reg.set_subject("main", Some(subject.clone()));
        reg.set_subject("window-a", Some(subject.clone()));

        // main focused later → main is primary.
        reg.mark_focused_at("main", ts(300));
        reg.mark_focused_at("window-a", ts(200));
        assert_eq!(reg.primary_for(&subject), Some("main".to_string()));

        // window-a now focused later → it takes over.
        reg.mark_focused_at("window-a", ts(400));
        assert_eq!(reg.primary_for(&subject), Some("window-a".to_string()));
    }

    #[test]
    fn primary_promotes_survivor_when_primary_closes() {
        let reg = WindowRegistry::new();
        let subject = Subject::DirectChat("s1".into());
        reg.register_at("main", ts(100));
        reg.register_at("window-a", ts(100));
        reg.set_subject("main", Some(subject.clone()));
        reg.set_subject("window-a", Some(subject.clone()));
        reg.mark_focused_at("main", ts(500)); // main is primary
        reg.mark_focused_at("window-a", ts(200));
        assert_eq!(reg.primary_for(&subject), Some("main".to_string()));

        // Primary closes → the survivor is promoted.
        reg.unregister("main");
        assert_eq!(reg.primary_for(&subject), Some("window-a".to_string()));
    }

    #[test]
    fn none_subject_never_counts_as_primary() {
        let reg = WindowRegistry::new();
        reg.register_at("main", ts(100));
        reg.register_at("window-a", ts(200)); // both subject: None
                                              // An empty window is never the "primary" of anything — querying any
                                              // subject returns None, so the overlay never fires for blank windows.
        assert_eq!(reg.primary_for(&Subject::Mission("m1".into())), None);
        assert_eq!(reg.primary_for(&Subject::DirectChat("s1".into())), None);
    }

    #[test]
    fn subject_serializes_adjacently_tagged() {
        let json = serde_json::to_value(Subject::Mission("m1".into())).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "type": "Mission", "value": "m1" })
        );
        let json = serde_json::to_value(Subject::DirectChat("s1".into())).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "type": "DirectChat", "value": "s1" })
        );
    }
}
