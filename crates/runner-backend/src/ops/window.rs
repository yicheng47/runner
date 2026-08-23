use crate::error::Result;
use crate::ops::node::mark_direct_sessions_viewed;
use crate::windows::{Subject, WindowEntry};
use crate::AppCore;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
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

pub fn report_subjects(core: &AppCore, label: &str, subjects: Vec<Subject>) -> Result<()> {
    core.windows.set_subjects(label, subjects);
    let visible = core.windows.focused_direct_sessions(label);
    mark_direct_sessions_viewed(core, &visible)?;
    core.broadcast_focus_map();
    Ok(())
}

pub fn mark_focused(core: &AppCore, label: &str) -> Result<()> {
    core.windows.mark_focused(label);
    let visible = core.windows.focused_direct_sessions(label);
    mark_direct_sessions_viewed(core, &visible)?;
    core.broadcast_focus_map();
    Ok(())
}

pub fn mark_blurred(core: &AppCore, label: &str) {
    core.windows.mark_blurred(label);
    core.broadcast_focus_map();
}

pub fn unregister(core: &AppCore, label: &str) {
    core.windows.unregister(label);
    core.broadcast_focus_map();
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

#[cfg(test)]
mod tests {
    use chrono::{TimeZone as _, Utc};

    use super::*;

    fn entry(label: &str, seconds: i64, subjects: Vec<Subject>) -> WindowEntry {
        WindowEntry {
            label: label.to_owned(),
            subjects,
            focused_at: Utc.timestamp_opt(seconds, 0).single().unwrap(),
            focused: true,
        }
    }

    #[test]
    fn labels_use_the_secondary_window_prefix() {
        let label = allocate_label();
        assert!(label.starts_with("window-"));
        assert!(ulid::Ulid::from_string(label.trim_start_matches("window-")).is_ok());
    }

    #[test]
    fn cascade_uses_the_most_recent_existing_window() {
        let entries = vec![
            entry("older", 100, Vec::new()),
            entry("new-window", 500, Vec::new()),
            entry("newer", 300, Vec::new()),
        ];
        assert_eq!(
            cascade_reference(&entries, "new-window"),
            Some("newer".into())
        );
    }

    #[test]
    fn equal_focus_timestamps_do_not_create_a_secondary() {
        let subject = Subject::Mission("mission".into());
        let entries = vec![
            entry("mine", 100, vec![subject.clone()]),
            entry("other", 100, vec![subject.clone()]),
        ];
        assert_eq!(
            is_secondary_for(&entries, "mine", &subject),
            SecondaryState::default()
        );
    }

    #[test]
    fn unknown_self_loses_to_an_existing_holder() {
        let subject = Subject::DirectChat("chat".into());
        let entries = vec![entry("other", 100, vec![subject.clone()])];
        assert_eq!(
            is_secondary_for(&entries, "mine", &subject),
            SecondaryState {
                secondary: true,
                primary_label: Some("other".into()),
            }
        );
    }

    #[test]
    fn strictly_later_holder_wins() {
        let subject = Subject::DirectChat("chat".into());
        let entries = vec![
            entry("mine", 100, vec![subject.clone()]),
            entry("later", 300, vec![subject.clone()]),
            entry("middle", 200, vec![subject.clone()]),
        ];
        assert_eq!(
            is_secondary_for(&entries, "mine", &subject),
            SecondaryState {
                secondary: true,
                primary_label: Some("later".into()),
            }
        );
    }

    #[test]
    fn equal_other_timestamps_keep_the_first_later_holder() {
        let subject = Subject::DirectChat("chat".into());
        let entries = vec![
            entry("mine", 100, vec![subject.clone()]),
            entry("first", 300, vec![subject.clone()]),
            entry("second", 300, vec![subject.clone()]),
        ];
        assert_eq!(
            is_secondary_for(&entries, "mine", &subject).primary_label,
            Some("first".into())
        );
    }
}
