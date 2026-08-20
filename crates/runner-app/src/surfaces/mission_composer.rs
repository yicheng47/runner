#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RosterEntry {
    pub handle: String,
    pub role: String,
    pub runtime: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ComposerState {
    pub draft: String,
    pub target: Option<String>,
    pub picker_dismissed: bool,
    pub active_index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ComposerPost {
    pub text: String,
    pub to: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct KeyTransition {
    pub state: ComposerState,
    pub prevent_default: bool,
    pub post: Option<ComposerPost>,
}

fn leading_mention_query(draft: &str) -> Option<&str> {
    draft
        .strip_prefix('@')
        .filter(|query| !query.chars().any(char::is_whitespace))
}

pub(crate) fn mention_query(state: &ComposerState) -> Option<&str> {
    if state.target.is_some() || state.picker_dismissed {
        None
    } else {
        leading_mention_query(&state.draft)
    }
}

pub(crate) fn mention_options(state: &ComposerState, roster: &[RosterEntry]) -> Vec<RosterEntry> {
    let Some(query) = mention_query(state) else {
        return Vec::new();
    };
    let query = query.to_lowercase();
    roster
        .iter()
        .filter(|entry| entry.handle.to_lowercase().starts_with(&query))
        .cloned()
        .collect()
}

pub(crate) fn update_draft(state: &ComposerState, draft: String) -> ComposerState {
    let stayed_in_mention =
        leading_mention_query(&state.draft).is_some() && leading_mention_query(&draft).is_some();
    ComposerState {
        draft,
        target: state.target.clone(),
        picker_dismissed: stayed_in_mention && state.picker_dismissed,
        active_index: 0,
    }
}

pub(crate) fn select_target(handle: String) -> ComposerState {
    ComposerState {
        target: Some(handle),
        ..ComposerState::default()
    }
}

pub(crate) fn key_down(
    state: &ComposerState,
    roster: &[RosterEntry],
    key: &str,
    shift: bool,
) -> KeyTransition {
    let query = mention_query(state);
    let options = mention_options(state, roster);
    let picker_open = !options.is_empty();
    let exact = query.and_then(|query| {
        roster
            .iter()
            .find(|entry| entry.handle.eq_ignore_ascii_case(query))
    });

    if picker_open && key == "space" {
        if let Some(exact) = exact {
            return transition(select_target(exact.handle.clone()));
        }
    }
    if picker_open && matches!(key, "down" | "up") {
        let current = state.active_index.min(options.len() - 1);
        let active_index = if key == "down" {
            (current + 1) % options.len()
        } else {
            (current + options.len() - 1) % options.len()
        };
        let mut next = state.clone();
        next.active_index = active_index;
        return transition(next);
    }
    if picker_open && !shift && matches!(key, "enter" | "tab") {
        let option = &options[state.active_index.min(options.len() - 1)];
        return transition(select_target(option.handle.clone()));
    }
    if picker_open && key == "escape" {
        let mut next = state.clone();
        next.picker_dismissed = true;
        return transition(next);
    }
    if key == "backspace" && state.target.is_some() && state.draft.is_empty() {
        let mut next = state.clone();
        next.target = None;
        return transition(next);
    }
    if key == "enter" && !shift {
        let trimmed = state.draft.trim();
        return KeyTransition {
            state: state.clone(),
            prevent_default: true,
            post: (!trimmed.is_empty()).then(|| ComposerPost {
                text: trimmed.to_owned(),
                to: state.target.clone(),
            }),
        };
    }
    KeyTransition {
        state: state.clone(),
        prevent_default: false,
        post: None,
    }
}

fn transition(state: ComposerState) -> KeyTransition {
    KeyTransition {
        state,
        prevent_default: true,
        post: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roster() -> Vec<RosterEntry> {
        vec![
            RosterEntry {
                handle: "coder".into(),
                role: "implementer".into(),
                runtime: "codex".into(),
            },
            RosterEntry {
                handle: "reviewer".into(),
                role: "critic".into(),
                runtime: "claude-code".into(),
            },
        ]
    }

    #[test]
    fn picker_and_send_match_the_react_composer_contract() {
        let state = update_draft(&ComposerState::default(), "@c".into());
        let picked = key_down(&state, &roster(), "enter", false);
        assert_eq!(picked.state.target.as_deref(), Some("coder"));
        assert!(picked.state.draft.is_empty());

        let drafted = update_draft(&picked.state, "  hello crew  ".into());
        let sent = key_down(&drafted, &roster(), "enter", false);
        assert_eq!(
            sent.post,
            Some(ComposerPost {
                text: "hello crew".into(),
                to: Some("coder".into()),
            })
        );
        assert!(!key_down(&drafted, &roster(), "enter", true).prevent_default);
    }

    #[test]
    fn escape_and_target_backspace_match_the_react_composer_contract() {
        let mention = update_draft(&ComposerState::default(), "@".into());
        let dismissed = key_down(&mention, &roster(), "escape", false).state;
        assert!(dismissed.picker_dismissed);
        assert!(mention_options(&dismissed, &roster()).is_empty());

        let targeted = select_target("reviewer".into());
        let cleared = key_down(&targeted, &roster(), "backspace", false).state;
        assert_eq!(cleared.target, None);
    }
}
