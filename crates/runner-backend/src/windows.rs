// Cross-window coordination registry (impl 0018, spec 12).
//
// Runner is a single backend process that can drive several Tauri webview
// windows at once. The backend stays the single source of truth: one
// `SessionManager`, one `RouterRegistry`, one `BusRegistry`. The only thing
// the windows need to *coordinate* is which subject (mission / direct chat)
// each is looking at, so two windows never write to the same PTY stdin.
//
// This module owns that map. Each window reports the `Subject`s it currently
// shows and its focus events; the registry computes, per subject, which
// window is **primary** — the most-recently-focused holder. Everything else
// (the Arc-style overlay, the terminal-mount gate) derives from that single
// rule. Windows report a *list* because the direct-chat surface can split
// into 2–3 panes (impl 0020): every visible pane's session participates in
// ownership arbitration, not just the focused one.

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
/// largest `focused_at` wins. `subjects` is every subject the window has on
/// screen — one for a single-pane surface, up to three for a split chat.
#[derive(Debug, Clone, Serialize)]
pub struct WindowEntry {
    pub label: String,
    pub subjects: Vec<Subject>,
    pub focused_at: DateTime<Utc>,
    pub focused: bool,
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

    /// Update a window's subjects without touching `focused_at` — navigating
    /// between routes (or re-arranging panes) is not a focus change. Upserts
    /// so a report that races ahead of `register` still lands.
    pub fn set_subjects(&self, label: &str, subjects: Vec<Subject>) {
        let mut map = self.entries.lock().unwrap();
        match map.get_mut(label) {
            Some(entry) => entry.subjects = subjects,
            None => {
                map.insert(
                    label.to_string(),
                    WindowEntry {
                        label: label.to_string(),
                        subjects,
                        focused_at: Utc::now(),
                        focused: false,
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

    pub fn mark_blurred(&self, label: &str) {
        if let Some(entry) = self.entries.lock().unwrap().get_mut(label) {
            entry.focused = false;
        }
    }

    /// Demote a window's focus rank to the floor without dropping its subject.
    ///
    /// `main` is hidden (not destroyed) on close, so it never emits
    /// `Destroyed` and stays registered. A *hidden* window must not keep
    /// owning a duplicated subject over a *visible* one — otherwise the
    /// visible window is stuck secondary with no terminal. Demoting
    /// `focused_at` lets any visible holder outrank it, while preserving the
    /// subject so a later reshow + `Focused(true)` reclaims primary. A hidden
    /// sole-holder still wins `primary_for` (nothing visible to hand off to).
    pub fn mark_hidden(&self, label: &str) {
        if let Some(entry) = self.entries.lock().unwrap().get_mut(label) {
            entry.focused_at = DateTime::<Utc>::MIN_UTC;
            entry.focused = false;
        }
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

    /// The primary window for a subject: among windows currently showing it
    /// (in any pane), the one with the max `focused_at`. `None` if no window
    /// holds it.
    pub fn primary_for(&self, subject: &Subject) -> Option<String> {
        self.entries
            .lock()
            .unwrap()
            .values()
            .filter(|e| e.subjects.contains(subject))
            .max_by(|a, b| a.focused_at.cmp(&b.focused_at))
            .map(|e| e.label.clone())
    }

    pub fn focused_direct_sessions(&self, label: &str) -> Vec<String> {
        self.entries
            .lock()
            .unwrap()
            .get(label)
            .filter(|entry| entry.focused)
            .into_iter()
            .flat_map(|entry| entry.subjects.iter())
            .filter_map(|subject| match subject {
                Subject::DirectChat(session_id) => Some(session_id.clone()),
                Subject::Mission(_) => None,
            })
            .collect()
    }

    pub fn any_focused_displaying(&self, session_ids: &[String]) -> bool {
        self.entries.lock().unwrap().values().any(|entry| {
            entry.focused
                && entry.subjects.iter().any(|subject| match subject {
                    Subject::DirectChat(session_id) => session_ids.contains(session_id),
                    Subject::Mission(_) => false,
                })
        })
    }

    // --- internals / test seams -----------------------------------------

    fn register_at(&self, label: &str, focused_at: DateTime<Utc>) {
        self.entries.lock().unwrap().insert(
            label.to_string(),
            WindowEntry {
                label: label.to_string(),
                subjects: Vec::new(),
                focused_at,
                focused: false,
            },
        );
    }

    /// `mark_focused` with an explicit timestamp. Upserts: a focus event for a
    /// window we somehow never registered still produces an entry rather than
    /// being dropped. Split out so tests can assert ordering without sleeping.
    fn mark_focused_at(&self, label: &str, focused_at: DateTime<Utc>) {
        let mut map = self.entries.lock().unwrap();
        match map.get_mut(label) {
            Some(entry) => {
                entry.focused_at = focused_at;
                entry.focused = true;
            }
            None => {
                map.insert(
                    label.to_string(),
                    WindowEntry {
                        label: label.to_string(),
                        subjects: Vec::new(),
                        focused_at,
                        focused: true,
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
        assert!(reg.snapshot()[0].subjects.is_empty());

        reg.unregister("main");
        assert!(reg.snapshot().is_empty());
    }

    #[test]
    fn set_subjects_preserves_focused_at() {
        let reg = WindowRegistry::new();
        reg.register_at("main", ts(100));
        reg.set_subjects("main", vec![Subject::Mission("m1".into())]);

        let snap = reg.snapshot();
        assert_eq!(snap[0].subjects, vec![Subject::Mission("m1".into())]);
        // subject change is not a focus change — timestamp untouched.
        assert_eq!(snap[0].focused_at, ts(100));
    }

    #[test]
    fn mark_focused_bumps_timestamp() {
        let reg = WindowRegistry::new();
        reg.register_at("main", ts(100));
        reg.mark_focused_at("main", ts(200));
        assert_eq!(reg.snapshot()[0].focused_at, ts(200));
        assert!(reg.snapshot()[0].focused);
        reg.mark_blurred("main");
        assert!(!reg.snapshot()[0].focused);
        assert_eq!(reg.snapshot()[0].focused_at, ts(200));
    }

    #[test]
    fn only_focused_windows_count_as_viewing_direct_chats() {
        let reg = WindowRegistry::new();
        reg.register_at("main", ts(100));
        reg.register_at("window-a", ts(100));
        reg.set_subjects(
            "main",
            vec![
                Subject::DirectChat("a".into()),
                Subject::DirectChat("b".into()),
            ],
        );
        reg.set_subjects("window-a", vec![Subject::DirectChat("b".into())]);

        reg.mark_focused_at("main", ts(200));
        assert!(reg.any_focused_displaying(&["a".into()]));
        assert!(reg.any_focused_displaying(&["b".into()]));
        assert_eq!(reg.focused_direct_sessions("main"), ["a", "b"]);

        reg.mark_blurred("main");
        assert!(!reg.any_focused_displaying(&["a".into(), "b".into()]));
        assert!(reg.focused_direct_sessions("main").is_empty());

        reg.mark_focused_at("window-a", ts(300));
        assert!(!reg.any_focused_displaying(&["a".into()]));
        assert!(reg.any_focused_displaying(&["b".into()]));
    }

    #[test]
    fn tab_switch_replaces_focused_visibility_immediately() {
        let reg = WindowRegistry::new();
        reg.register_at("main", ts(100));
        reg.mark_focused_at("main", ts(200));
        reg.set_subjects("main", vec![Subject::DirectChat("a".into())]);
        assert!(reg.any_focused_displaying(&["a".into()]));

        reg.set_subjects(
            "main",
            vec![
                Subject::DirectChat("b".into()),
                Subject::DirectChat("c".into()),
            ],
        );
        assert!(!reg.any_focused_displaying(&["a".into()]));
        assert!(reg.any_focused_displaying(&["b".into()]));
        assert!(reg.any_focused_displaying(&["c".into()]));
    }

    #[test]
    fn primary_for_returns_most_recently_focused_holder() {
        let reg = WindowRegistry::new();
        reg.register_at("main", ts(100));
        reg.register_at("window-a", ts(100));
        let subject = Subject::Mission("m1".into());
        reg.set_subjects("main", vec![subject.clone()]);
        reg.set_subjects("window-a", vec![subject.clone()]);

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
        reg.set_subjects("main", vec![subject.clone()]);
        reg.set_subjects("window-a", vec![subject.clone()]);
        reg.mark_focused_at("main", ts(500)); // main is primary
        reg.mark_focused_at("window-a", ts(200));
        assert_eq!(reg.primary_for(&subject), Some("main".to_string()));

        // Primary closes → the survivor is promoted.
        reg.unregister("main");
        assert_eq!(reg.primary_for(&subject), Some("window-a".to_string()));
    }

    #[test]
    fn mark_hidden_demotes_below_visible_holder_but_keeps_subject() {
        let reg = WindowRegistry::new();
        let subject = Subject::Mission("m1".into());
        reg.register_at("main", ts(500));
        reg.register_at("window-a", ts(300));
        reg.set_subjects("main", vec![subject.clone()]);
        reg.set_subjects("window-a", vec![subject.clone()]);
        // main focused most recently → primary.
        assert_eq!(reg.primary_for(&subject), Some("main".to_string()));

        // main hidden on close → demoted; the visible window-a takes over.
        reg.mark_hidden("main");
        assert_eq!(reg.primary_for(&subject), Some("window-a".to_string()));

        // Subject preserved so a reshow + focus can reclaim ownership.
        let main_entry = reg
            .snapshot()
            .into_iter()
            .find(|e| e.label == "main")
            .expect("main still registered");
        assert_eq!(main_entry.subjects, vec![subject.clone()]);

        // Reshow + focus → main reclaims primary.
        reg.mark_focused_at("main", ts(600));
        assert_eq!(reg.primary_for(&subject), Some("main".to_string()));
    }

    #[test]
    fn mark_hidden_sole_holder_stays_primary() {
        let reg = WindowRegistry::new();
        let subject = Subject::Mission("m1".into());
        reg.register_at("main", ts(500));
        reg.set_subjects("main", vec![subject.clone()]);
        // No visible duplicate to hand off to → hidden main remains primary.
        reg.mark_hidden("main");
        assert_eq!(reg.primary_for(&subject), Some("main".to_string()));
    }

    #[test]
    fn empty_subjects_never_counts_as_primary() {
        let reg = WindowRegistry::new();
        reg.register_at("main", ts(100));
        reg.register_at("window-a", ts(200)); // both subjects: []
                                              // An empty window is never the "primary" of anything — querying any
                                              // subject returns None, so the overlay never fires for blank windows.
        assert_eq!(reg.primary_for(&Subject::Mission("m1".into())), None);
        assert_eq!(reg.primary_for(&Subject::DirectChat("s1".into())), None);
    }

    #[test]
    fn split_window_owns_every_visible_pane_session() {
        // Split-view scenario (impl 0020): one window shows chats A+B in
        // panes and was focused last; another window shows B alone. The
        // split window must be primary for BOTH sessions — reporting only
        // the focused pane's subject is exactly the bug this list model
        // exists to prevent (two windows both believing they own B's PTY).
        let reg = WindowRegistry::new();
        let a = Subject::DirectChat("a".into());
        let b = Subject::DirectChat("b".into());
        reg.register_at("main", ts(200));
        reg.register_at("window-a", ts(100));
        reg.set_subjects("main", vec![a.clone(), b.clone()]);
        reg.set_subjects("window-a", vec![b.clone()]);

        assert_eq!(reg.primary_for(&a), Some("main".to_string()));
        assert_eq!(reg.primary_for(&b), Some("main".to_string()));

        // The single-pane window comes forward → it takes B; the split
        // window keeps A (nobody else shows it).
        reg.mark_focused_at("window-a", ts(300));
        assert_eq!(reg.primary_for(&b), Some("window-a".to_string()));
        assert_eq!(reg.primary_for(&a), Some("main".to_string()));
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
