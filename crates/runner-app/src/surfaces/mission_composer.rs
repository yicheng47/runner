use std::ops::Range;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RosterEntry {
    pub handle: String,
    pub role: String,
    pub runtime: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ComposerState {
    pub draft: String,
    pub caret: usize,
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
pub(crate) struct DraftEdit {
    pub range: Range<usize>,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct KeyTransition {
    pub state: ComposerState,
    pub edit: Option<DraftEdit>,
    pub prevent_default: bool,
    pub post: Option<ComposerPost>,
}

fn mention_token(draft: &str, caret: usize) -> Option<Range<usize>> {
    let before = draft.get(..caret)?;
    if draft[caret..]
        .chars()
        .next()
        .is_some_and(|character| !character.is_whitespace())
    {
        return None;
    }
    let start = before
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_whitespace())
        .map_or(0, |(offset, character)| offset + character.len_utf8());
    before[start..].starts_with('@').then_some(start..caret)
}

fn mention_range(state: &ComposerState) -> Option<Range<usize>> {
    if state.picker_dismissed {
        None
    } else {
        mention_token(&state.draft, state.caret)
    }
}

pub(crate) fn mention_query(state: &ComposerState) -> Option<&str> {
    let range = mention_range(state)?;
    Some(&state.draft[range.start + 1..range.end])
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

pub(crate) fn update_draft(state: &ComposerState, draft: String, caret: usize) -> ComposerState {
    let stayed_in_mention = mention_token(&state.draft, state.caret)
        .zip(mention_token(&draft, caret))
        .is_some_and(|(before, after)| {
            before.start == after.start && state.draft[..before.start] == draft[..after.start]
        });
    ComposerState {
        draft,
        caret,
        target: state.target.clone(),
        picker_dismissed: stayed_in_mention && state.picker_dismissed,
        active_index: 0,
    }
}

pub(crate) fn select_target(state: &ComposerState, handle: String) -> KeyTransition {
    let Some(token) = mention_range(state) else {
        return transition(state.clone());
    };
    let leading = token.start == 0 && state.target.is_none();
    let text = if leading {
        String::new()
    } else {
        format!("@{handle} ")
    };
    let end = token.end + usize::from(state.draft[token.end..].starts_with(' '));
    let range = token.start..end;
    let mut draft = state.draft.clone();
    draft.replace_range(range.clone(), &text);
    KeyTransition {
        state: ComposerState {
            draft,
            caret: range.start + text.len(),
            target: state.target.clone().or(Some(handle)),
            ..ComposerState::default()
        },
        edit: Some(DraftEdit { range, text }),
        prevent_default: true,
        post: None,
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
            return select_target(state, exact.handle.clone());
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
        return select_target(state, option.handle.clone());
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
            edit: None,
            prevent_default: true,
            post: (!trimmed.is_empty()).then(|| ComposerPost {
                text: trimmed.to_owned(),
                to: state.target.clone(),
            }),
        };
    }
    KeyTransition {
        state: state.clone(),
        edit: None,
        prevent_default: false,
        post: None,
    }
}

fn transition(state: ComposerState) -> KeyTransition {
    KeyTransition {
        state,
        edit: None,
        prevent_default: true,
        post: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pick(state: &ComposerState, handle: &str) -> ComposerState {
        let picked = select_target(state, handle.into());
        let edit = picked.edit.expect("a pick edits the draft");
        let mut draft = state.draft.clone();
        draft.replace_range(edit.range.clone(), &edit.text);
        assert_eq!(draft, picked.state.draft);
        assert_eq!(picked.state.caret, edit.range.start + edit.text.len());
        picked.state
    }

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
        let state = update_draft(&ComposerState::default(), "@c".into(), 2);
        let picked = key_down(&state, &roster(), "enter", false);
        assert_eq!(picked.state.target.as_deref(), Some("coder"));
        assert!(picked.state.draft.is_empty());

        let drafted = update_draft(&picked.state, "  hello crew  ".into(), 14);
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
        let mention = update_draft(&ComposerState::default(), "@".into(), 1);
        let dismissed = key_down(&mention, &roster(), "escape", false).state;
        assert!(dismissed.picker_dismissed);
        assert!(mention_options(&dismissed, &roster()).is_empty());

        let targeted = pick(&mention, "reviewer");
        let cleared = key_down(&targeted, &roster(), "backspace", false).state;
        assert_eq!(cleared.target, None);
    }

    #[test]
    fn mentions_open_at_whitespace_boundaries_and_filter_the_query() {
        for draft in ["please check @", "请检查\t@", "👩‍💻\n@", "hello\u{3000}@"] {
            let state = update_draft(&ComposerState::default(), draft.into(), draft.len());
            assert_eq!(mention_query(&state), Some(""));
            assert_eq!(mention_options(&state, &roster()), roster());
        }
        let draft = "please check @REV";
        let state = update_draft(&ComposerState::default(), draft.into(), draft.len());
        assert_eq!(mention_query(&state), Some("REV"));
        assert_eq!(
            mention_options(&state, &roster()),
            vec![roster()[1].clone()]
        );

        for draft in [
            "me@host",
            "please check me@host",
            "请@rev",
            "hello @rev later",
        ] {
            let state = update_draft(&ComposerState::default(), draft.into(), draft.len());
            assert!(mention_options(&state, &roster()).is_empty());
        }
    }

    #[test]
    fn mid_text_picks_preserve_surrounding_text_and_place_the_caret() {
        let prefix = "请检查 👩‍💻 @rev";
        let draft = format!("{prefix} to check 王菲");
        let state = update_draft(&ComposerState::default(), draft, prefix.len());
        for key in ["enter", "tab"] {
            let picked = key_down(&state, &roster(), key, false);
            assert!(picked.prevent_default);
            assert_eq!(picked.post, None);
            assert_eq!(picked.state.draft, "请检查 👩‍💻 @reviewer to check 王菲");
            assert_eq!(picked.state.caret, "请检查 👩‍💻 @reviewer ".len());
            assert_eq!(picked.state.target.as_deref(), Some("reviewer"));
            assert!(mention_options(&picked.state, &roster()).is_empty());
        }

        let mut targeted = state.clone();
        targeted.target = Some("coder".into());
        let picked = pick(&targeted, "reviewer");
        assert_eq!(picked.target.as_deref(), Some("coder"));
        assert_eq!(picked.draft, "请检查 👩‍💻 @reviewer to check 王菲");
        assert_eq!(picked.caret, "请检查 👩‍💻 @reviewer ".len());

        let draft = "请检查 @rev\nnext";
        let state = update_draft(&ComposerState::default(), draft.into(), "请检查 @rev".len());
        assert_eq!(pick(&state, "reviewer").draft, "请检查 @reviewer \nnext");
    }

    #[test]
    fn picks_after_a_chip_complete_in_place_and_keep_the_chip() {
        let mut state = update_draft(&ComposerState::default(), "@c please".into(), 2);
        state.target = Some("reviewer".into());
        let picked = pick(&state, "coder");
        assert_eq!(picked.target.as_deref(), Some("reviewer"));
        assert_eq!(picked.draft, "@coder please");
        assert_eq!(picked.caret, "@coder ".len());
    }

    #[test]
    fn leading_picks_keep_trailing_text() {
        let state = update_draft(&ComposerState::default(), "@rev 请检查 👋".into(), 4);
        let picked = pick(&state, "reviewer");
        assert_eq!(picked.draft, "请检查 👋");
        assert_eq!(picked.caret, 0);
        assert_eq!(picked.target.as_deref(), Some("reviewer"));
    }

    #[test]
    fn mid_text_picker_navigation_and_exact_space_match_leading_picks() {
        let draft = "please check @";
        let state = update_draft(&ComposerState::default(), draft.into(), draft.len());
        let down = key_down(&state, &roster(), "down", false).state;
        assert_eq!(down.active_index, 1);
        let picked = key_down(&down, &roster(), "tab", false).state;
        assert_eq!(picked.draft, "please check @reviewer ");
        assert_eq!(
            key_down(&down, &roster(), "down", false).state.active_index,
            0
        );
        assert_eq!(
            key_down(&state, &roster(), "up", false).state.active_index,
            1
        );
        assert!(!key_down(&state, &roster(), "enter", true).prevent_default);
        assert!(!key_down(&state, &roster(), "space", false).prevent_default);

        let draft = "please check @REVIEWER";
        let state = update_draft(&state, draft.into(), draft.len());
        let picked = key_down(&state, &roster(), "space", false);
        assert!(picked.prevent_default);
        assert_eq!(picked.state.draft, "please check @reviewer ");
        assert_eq!(picked.state.caret, picked.state.draft.len());
        assert_eq!(
            key_down(&picked.state, &roster(), "enter", false).post,
            Some(ComposerPost {
                text: "please check @reviewer".into(),
                to: Some("reviewer".into())
            })
        );
    }

    #[test]
    fn escape_dismisses_only_the_current_mid_text_query() {
        let draft = "please check @r";
        let state = update_draft(&ComposerState::default(), draft.into(), draft.len());
        let dismissed = key_down(&state, &roster(), "escape", false).state;
        assert!(dismissed.picker_dismissed);
        assert!(mention_options(&dismissed, &roster()).is_empty());
        let draft = "please check @rev";
        let edited = update_draft(&dismissed, draft.into(), draft.len());
        assert!(mention_options(&edited, &roster()).is_empty());
        let draft = "please check @rev and @c";
        let other = update_draft(&edited, draft.into(), draft.len());
        assert_eq!(mention_query(&other), Some("c"));
    }

    #[test]
    fn moving_the_caret_away_from_a_token_closes_the_picker() {
        let draft = "请 👋 @rev later";
        let caret = "请 👋 @rev".len();
        let state = update_draft(&ComposerState::default(), draft.into(), caret);
        assert_eq!(mention_query(&state), Some("rev"));
        for offset in [0, "请 👋 ".len(), caret - 1, caret + 1, draft.len(), 1] {
            let moved = update_draft(&state, draft.into(), offset);
            assert!(mention_options(&moved, &roster()).is_empty());
        }
        let dismissed = key_down(&state, &roster(), "escape", false).state;
        let away = update_draft(&dismissed, draft.into(), 0);
        let back = update_draft(&away, draft.into(), caret);
        assert_eq!(mention_query(&back), Some("rev"));
    }
}
