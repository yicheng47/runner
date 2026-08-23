use std::time::{Duration, Instant};

use alacritty_terminal::grid::Dimensions as _;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::Color;
use serde::{Deserialize, Serialize};

pub use runner_backend::session::manager::{InputObservation, InputState};

use crate::mappings::InputKind;
use crate::replay::row_to_string;

pub const ECHO_WINDOW: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputEvent {
    Key { kind: InputKind },
    Paste { text: String },
    Composing { composing: bool },
}

#[derive(Clone, Debug)]
struct Composer {
    row: usize,
    prefix: String,
    empty_forms: Vec<String>,
    last_form: String,
    visible: bool,
    submitted_form: Option<String>,
    placeholder_style: Option<CellStyle>,
}

#[derive(Clone, Debug)]
struct Probe {
    typed: String,
    rows_before: Vec<String>,
    styles_before: Vec<Vec<CellStyle>>,
    started: Instant,
    weak_match: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CellStyle {
    fg: Color,
    bg: Color,
    flags: Flags,
}

#[derive(Clone, Debug)]
enum TrackerState {
    Idle,
    Probing(Probe),
    Reprobing { probe: Probe, composer: Composer },
    Drafting(Composer),
    Submitted(Composer),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ReportedState {
    state: InputState,
    composing: bool,
    composer_visible: bool,
}

pub struct InputTracker {
    state: TrackerState,
    composing: bool,
    reported: Option<ReportedState>,
    public_since: Instant,
    input_revision: u64,
}

impl InputTracker {
    pub fn new(now: Instant) -> Self {
        Self {
            state: TrackerState::Idle,
            composing: false,
            reported: None,
            public_since: now,
            input_revision: 0,
        }
    }

    pub fn initial_observation(&mut self, now: Instant) -> InputObservation {
        self.emit_if_changed(now)
            .expect("a new tracker must report its initial idle state")
    }

    pub fn observe_input<T>(
        &mut self,
        event: &InputEvent,
        now: Instant,
        term: &Term<T>,
    ) -> Option<InputObservation> {
        self.input_revision = self.input_revision.wrapping_add(1);
        self.expire_probe(now);
        match event {
            InputEvent::Composing { composing } => self.composing = *composing,
            InputEvent::Paste { text } => self.observe_content(text, true, now, term),
            InputEvent::Key { kind } => match kind {
                InputKind::Content { text } => self.observe_content(text, false, now, term),
                InputKind::Submit if !self.composing => self.submit(now, term),
                InputKind::Edit | InputKind::Cancel => {
                    if let TrackerState::Drafting(composer) = &mut self.state {
                        composer.submitted_form = None;
                    }
                }
                InputKind::Navigate | InputKind::Submit => {}
            },
        }
        self.emit_if_changed(now)
    }

    pub fn observe_output<T>(&mut self, now: Instant, term: &Term<T>) -> Option<InputObservation> {
        let state = std::mem::replace(&mut self.state, TrackerState::Idle);
        self.state = match state {
            TrackerState::Idle => TrackerState::Idle,
            TrackerState::Probing(probe) => {
                if now.duration_since(probe.started) > ECHO_WINDOW {
                    TrackerState::Idle
                } else {
                    let rows_after = visible_rows(term);
                    match locate_echo(
                        &probe.rows_before,
                        &rows_after,
                        &probe.typed,
                        probe.weak_match,
                    ) {
                        Some(matched) => {
                            let placeholder_style = probe_placeholder_style(&probe, &matched, term);
                            TrackerState::Drafting(Composer {
                                row: matched.row,
                                prefix: matched.prefix,
                                empty_forms: vec![probe.rows_before[matched.row].clone()],
                                last_form: rows_after[matched.row].clone(),
                                visible: true,
                                submitted_form: None,
                                placeholder_style,
                            })
                        }
                        None => TrackerState::Probing(probe),
                    }
                }
            }
            TrackerState::Reprobing {
                probe,
                mut composer,
            } => {
                let rows_after = visible_rows(term);
                if let Some(matched) = locate_echo(
                    &probe.rows_before,
                    &rows_after,
                    &probe.typed,
                    probe.weak_match,
                ) {
                    let before = &probe.rows_before[matched.row];
                    if composer_text_is_empty(before, &matched.prefix, &composer.empty_forms)
                        && !composer.empty_forms.contains(before)
                    {
                        composer.empty_forms.push(before.clone());
                    }
                    if let Some(style) = probe_placeholder_style(&probe, &matched, term) {
                        composer.placeholder_style = Some(style);
                    }
                    composer.row = matched.row;
                    composer.prefix = matched.prefix;
                    composer.last_form = rows_after[matched.row].clone();
                    composer.visible = true;
                    composer.submitted_form = None;
                    TrackerState::Drafting(composer)
                } else if now.duration_since(probe.started) <= ECHO_WINDOW {
                    TrackerState::Reprobing { probe, composer }
                } else {
                    self.observe_composer(term, composer, true)
                }
            }
            TrackerState::Drafting(composer) => self.observe_composer(term, composer, false),
            TrackerState::Submitted(composer) => self.observe_composer(term, composer, true),
        };
        self.emit_if_changed(now)
    }

    pub fn reset_guard(&self) -> u64 {
        self.input_revision
    }

    pub fn reset_if_unchanged(&mut self, guard: u64, now: Instant) -> Option<InputObservation> {
        if self.input_revision != guard {
            return None;
        }
        self.state = TrackerState::Idle;
        self.composing = false;
        self.input_revision = self.input_revision.wrapping_add(1);
        self.emit_if_changed(now)
    }

    fn observe_content<T>(&mut self, text: &str, pasted: bool, now: Instant, term: &Term<T>) {
        if text.is_empty() {
            return;
        }
        let state = std::mem::replace(&mut self.state, TrackerState::Idle);
        self.state = match state {
            TrackerState::Idle => TrackerState::Probing(new_probe(text, pasted, now, term)),
            TrackerState::Probing(mut probe) => {
                probe.typed.push_str(&probe_text(text));
                probe.weak_match |= pasted && weak_paste_match(text);
                TrackerState::Probing(probe)
            }
            TrackerState::Reprobing {
                mut probe,
                composer,
            } => {
                probe.typed.push_str(&probe_text(text));
                probe.weak_match |= pasted && weak_paste_match(text);
                TrackerState::Reprobing { probe, composer }
            }
            TrackerState::Drafting(mut composer) => {
                composer.submitted_form = None;
                TrackerState::Drafting(composer)
            }
            TrackerState::Submitted(composer) => TrackerState::Reprobing {
                probe: new_probe(text, pasted, now, term),
                composer,
            },
        };
    }

    fn submit<T>(&mut self, _now: Instant, term: &Term<T>) {
        let state = std::mem::replace(&mut self.state, TrackerState::Idle);
        match state {
            TrackerState::Drafting(mut composer) => {
                if composer.row < term.screen_lines() {
                    let current = row_to_string(term, composer.row as i32);
                    if current.starts_with(&composer.prefix) {
                        composer.last_form = current;
                    }
                }
                composer.submitted_form = Some(composer.last_form.clone());
                self.state = TrackerState::Submitted(composer);
            }
            state => self.state = state,
        }
    }

    fn observe_composer<T>(
        &self,
        term: &Term<T>,
        mut composer: Composer,
        submitted: bool,
    ) -> TrackerState {
        let mut current = if composer.row < term.screen_lines() {
            row_to_string(term, composer.row as i32)
        } else if let Some(row) = relocate_row(
            term,
            &composer.prefix,
            composer.row.min(term.screen_lines().saturating_sub(1)),
        ) {
            composer.row = row;
            composer.visible = true;
            row_to_string(term, row as i32)
        } else {
            composer.visible = false;
            return if submitted {
                TrackerState::Submitted(composer)
            } else {
                TrackerState::Drafting(composer)
            };
        };
        if composer_is_empty(term, composer.row, &current, &composer) {
            return TrackerState::Idle;
        }
        if !current.starts_with(&composer.prefix) {
            if let Some(row) = relocate_row(term, &composer.prefix, composer.row) {
                composer.row = row;
                current = row_to_string(term, row as i32);
                composer.visible = true;
            } else {
                composer.visible = false;
                return if submitted {
                    TrackerState::Submitted(composer)
                } else {
                    TrackerState::Drafting(composer)
                };
            }
        } else {
            composer.visible = true;
        }

        if composer_is_empty(term, composer.row, &current, &composer) {
            return TrackerState::Idle;
        }

        if submitted {
            let submitted_form = composer
                .submitted_form
                .as_ref()
                .expect("submitted composer keeps its submitted form");
            if current == *submitted_form {
                return TrackerState::Drafting(composer);
            }
            return TrackerState::Idle;
        }

        if let Some(submitted_form) = composer.submitted_form.as_ref() {
            if current == *submitted_form {
                return TrackerState::Drafting(composer);
            }
            return TrackerState::Idle;
        }
        composer.last_form = current;
        TrackerState::Drafting(composer)
    }

    fn expire_probe(&mut self, now: Instant) {
        if matches!(
            &self.state,
            TrackerState::Probing(probe) if now.duration_since(probe.started) > ECHO_WINDOW
        ) {
            self.state = TrackerState::Idle;
        }
    }

    fn emit_if_changed(&mut self, now: Instant) -> Option<InputObservation> {
        let current = self.current_reported_state();
        if self.reported == Some(current) {
            return None;
        }
        if self
            .reported
            .is_none_or(|previous| previous.state != current.state)
        {
            self.public_since = now;
        }
        self.reported = Some(current);
        Some(InputObservation {
            state: current.state,
            since: self.public_since,
            composing: current.composing,
            composer_visible: current.composer_visible,
        })
    }

    fn current_reported_state(&self) -> ReportedState {
        if self.composing {
            return ReportedState {
                state: InputState::Drafting,
                composing: true,
                composer_visible: true,
            };
        }
        match &self.state {
            TrackerState::Idle | TrackerState::Probing(_) => ReportedState {
                state: InputState::Idle,
                composing: false,
                composer_visible: true,
            },
            TrackerState::Reprobing { composer, .. } => ReportedState {
                state: InputState::Submitted,
                composing: false,
                composer_visible: composer.visible,
            },
            TrackerState::Drafting(composer) => ReportedState {
                state: InputState::Drafting,
                composing: false,
                composer_visible: composer.visible,
            },
            TrackerState::Submitted(composer) => ReportedState {
                state: InputState::Submitted,
                composing: false,
                composer_visible: composer.visible,
            },
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct EchoMatch {
    row: usize,
    prefix: String,
    empty_before: bool,
    kind: EchoMatchKind,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum EchoMatchKind {
    Exact,
    Decorated,
    Weak,
}

fn visible_rows<T>(term: &Term<T>) -> Vec<String> {
    (0..term.screen_lines())
        .map(|row| row_to_string(term, row as i32))
        .collect()
}

fn visible_row_styles<T>(term: &Term<T>) -> Vec<Vec<CellStyle>> {
    (0..term.screen_lines())
        .map(|row| {
            let grid = term.grid();
            let row = &grid[Line(row as i32)];
            (0..grid.columns())
                .filter_map(|column| {
                    let cell = &row[Column(column)];
                    (!cell
                        .flags
                        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER))
                    .then(|| {
                        std::iter::repeat_n(
                            cell_style(cell),
                            1 + cell.zerowidth().map_or(0, <[_]>::len),
                        )
                    })
                })
                .flatten()
                .collect()
        })
        .collect()
}

fn new_probe<T>(text: &str, pasted: bool, now: Instant, term: &Term<T>) -> Probe {
    Probe {
        typed: probe_text(text),
        rows_before: visible_rows(term),
        styles_before: visible_row_styles(term),
        started: now,
        weak_match: pasted && weak_paste_match(text),
    }
}

fn cell_style(cell: &alacritty_terminal::term::cell::Cell) -> CellStyle {
    let mut flags = cell.flags;
    flags.remove(
        Flags::WIDE_CHAR
            | Flags::WIDE_CHAR_SPACER
            | Flags::LEADING_WIDE_CHAR_SPACER
            | Flags::WRAPLINE,
    );
    CellStyle {
        fg: cell.fg,
        bg: cell.bg,
        flags,
    }
}

fn snapshot_content_style(text: &str, styles: &[CellStyle], prefix: &str) -> Option<CellStyle> {
    text.chars()
        .zip(styles)
        .skip(prefix.chars().count())
        .find_map(|(character, style)| (!character.is_whitespace()).then_some(*style))
}

fn row_content_style<T>(term: &Term<T>, row: usize, prefix: &str) -> Option<CellStyle> {
    let grid = term.grid();
    let row = &grid[Line(row as i32)];
    let mut prefix_characters = prefix.chars().count();
    for column in 0..grid.columns() {
        let cell = &row[Column(column)];
        if cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }
        if prefix_characters > 0 {
            prefix_characters =
                prefix_characters.saturating_sub(1 + cell.zerowidth().map_or(0, <[_]>::len));
            continue;
        }
        if !cell.c.is_whitespace() {
            return Some(cell_style(cell));
        }
    }
    None
}

fn probe_placeholder_style<T>(
    probe: &Probe,
    matched: &EchoMatch,
    term: &Term<T>,
) -> Option<CellStyle> {
    let placeholder = snapshot_content_style(
        &probe.rows_before[matched.row],
        &probe.styles_before[matched.row],
        &matched.prefix,
    );
    let draft = row_content_style(term, matched.row, &matched.prefix);
    placeholder.filter(|style| Some(*style) != draft)
}

fn locate_echo(
    before: &[String],
    after: &[String],
    typed: &str,
    weak_match: bool,
) -> Option<EchoMatch> {
    before
        .iter()
        .zip(after)
        .enumerate()
        .filter(|(_, (before, after))| before != after)
        .filter_map(|(row, (before, after))| {
            match_prompt_prefix(before, after, typed, weak_match).map(|(prefix, kind)| EchoMatch {
                row,
                empty_before: before.trim_end() == prefix.trim_end(),
                prefix,
                kind,
            })
        })
        .min_by_key(|matched| {
            (
                matched.kind,
                !matched.empty_before,
                matched.prefix.chars().count(),
                matched.row,
            )
        })
}

fn match_prompt_prefix(
    before: &str,
    after: &str,
    typed: &str,
    weak_match: bool,
) -> Option<(String, EchoMatchKind)> {
    if before == after {
        return None;
    }
    let mut decorated = None;
    if !typed.is_empty() {
        for (index, _) in after.match_indices(typed) {
            let prefix = &after[..index];
            let remainder = &after[index..];
            if !(before.starts_with(prefix) || before.trim_end() == prefix.trim_end())
                || !looks_like_prompt_prefix(prefix)
            {
                continue;
            }
            let Some(suffix) = remainder.strip_prefix(typed) else {
                continue;
            };
            if suffix.chars().all(|character| !character.is_alphanumeric()) {
                return Some((prefix.to_owned(), EchoMatchKind::Exact));
            }
            let stable_suffix = common_suffix(before, after);
            if stable_suffix.len() <= suffix.len() && !stable_suffix.trim().is_empty() {
                decorated = Some((prefix.to_owned(), EchoMatchKind::Decorated));
            }
        }
    }
    if decorated.is_some() {
        return decorated;
    }
    if !weak_match {
        return None;
    }
    let prefix = common_prefix(before, after);
    let remainder = &after[prefix.len()..];
    (looks_like_prompt_prefix(prefix) && !remainder.trim().is_empty())
        .then(|| (prefix.to_owned(), EchoMatchKind::Weak))
}

fn common_prefix<'a>(left: &'a str, right: &str) -> &'a str {
    let mut end = 0;
    for ((left_index, left_char), right_char) in left.char_indices().zip(right.chars()) {
        if left_char != right_char {
            break;
        }
        end = left_index + left_char.len_utf8();
    }
    &left[..end]
}

fn common_suffix<'a>(left: &str, right: &'a str) -> &'a str {
    let mut start = right.len();
    for ((right_index, right_char), left_char) in right.char_indices().rev().zip(left.chars().rev())
    {
        if left_char != right_char {
            break;
        }
        start = right_index;
    }
    &right[start..]
}

fn looks_like_prompt_prefix(prefix: &str) -> bool {
    let Some(last) = prefix.trim_end().chars().last() else {
        return false;
    };
    prefix.chars().last().is_some_and(char::is_whitespace) && !last.is_alphanumeric()
}

fn composer_text_is_empty(current: &str, prefix: &str, empty_forms: &[String]) -> bool {
    empty_forms.iter().any(|form| current == form) || current.trim_end() == prefix.trim_end()
}

fn composer_is_empty<T>(term: &Term<T>, row: usize, current: &str, composer: &Composer) -> bool {
    composer_text_is_empty(current, &composer.prefix, &composer.empty_forms)
        || (current != composer.last_form
            && composer.placeholder_style.is_some_and(|placeholder| {
                row_content_style(term, row, &composer.prefix) == Some(placeholder)
            }))
}

fn relocate_row<T>(term: &Term<T>, prefix: &str, old_row: usize) -> Option<usize> {
    (0..term.screen_lines())
        .filter(|row| row_to_string(term, *row as i32).starts_with(prefix))
        .min_by_key(|row| row.abs_diff(old_row))
}

fn probe_text(text: &str) -> String {
    text.lines().next().unwrap_or_default().to_owned()
}

fn weak_paste_match(text: &str) -> bool {
    text.contains(['\n', '\r']) || text.chars().count() > 40
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay::{feed, new_term};

    #[test]
    fn matcher_requires_a_prompt_prefix_and_prefers_the_empty_form() {
        let before = vec!["assistant ".into(), "❯ Try a task".into(), "$ ".into()];
        let after = vec!["assistant h".into(), "❯ h".into(), "$ h".into()];
        assert_eq!(
            locate_echo(&before, &after, "h", false),
            Some(EchoMatch {
                row: 2,
                prefix: "$ ".into(),
                empty_before: true,
                kind: EchoMatchKind::Exact,
            })
        );
    }

    #[test]
    fn matcher_accepts_stable_right_side_composer_decorations() {
        for (before, after) in [
            (
                "> Try a task            <ret> send",
                "> h                     <ret> send",
            ),
            ("| > Ask Codex           |", "| > h                   |"),
        ] {
            assert!(locate_echo(&[before.into()], &[after.into()], "h", false).is_some());
        }

        let before = vec!["  └ ".into(), "> Try a task            <ret> send".into()];
        let after = vec![
            "  └ hooked 12 files".into(),
            "> h                     <ret> send".into(),
        ];
        assert_eq!(locate_echo(&before, &after, "h", false).unwrap().row, 1);
    }

    #[test]
    fn transcript_repaint_cannot_steal_the_composer_echo() {
        let start = Instant::now();
        let mut term = new_term(40, 5);
        feed(
            &mut term,
            b"\x1b[2;1H\xe2\x9d\xaf Try a task\x1b[4;1H  \xe2\x94\x94 ",
        );
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content { text: "h".into() },
            },
            start,
            &term,
        );
        feed(
            &mut term,
            b"\x1b[2;1H\x1b[K\xe2\x9d\xaf h\x1b[4;1H\x1b[K  \xe2\x94\x94 hooked 12 files",
        );
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(1), &term)
                .unwrap()
                .state,
            InputState::Drafting
        );
        feed(&mut term, b"\x1b[2;1H\x1b[K\xe2\x9d\xaf Try a task");
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(2), &term)
                .unwrap()
                .state,
            InputState::Idle
        );
    }

    #[test]
    fn matcher_accepts_collapsed_multiline_paste_but_not_a_transcript_row() {
        assert_eq!(
            match_prompt_prefix(
                "❯ Try a task",
                "❯ [Pasted text #1 +19 lines]",
                "line one",
                true,
            ),
            Some(("❯ ".into(), EchoMatchKind::Weak))
        );
        assert_eq!(
            match_prompt_prefix("assistant ", "assistant [Pasted text]", "line", true),
            None
        );
    }

    #[test]
    fn typed_then_deleted_returns_to_idle_from_the_grid() {
        let start = Instant::now();
        let mut term = new_term(40, 4);
        feed(&mut term, b"\x1b[2;1H\xe2\x9d\xaf Try a task");
        let mut tracker = InputTracker::new(start);
        assert_eq!(tracker.initial_observation(start).state, InputState::Idle);
        assert!(tracker
            .observe_input(
                &InputEvent::Key {
                    kind: InputKind::Content { text: "h".into() }
                },
                start,
                &term,
            )
            .is_none());
        feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf h");
        let drafting = tracker
            .observe_output(start + Duration::from_millis(5), &term)
            .unwrap();
        assert_eq!(drafting.state, InputState::Drafting);
        feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf Try a task");
        let idle = tracker
            .observe_output(start + Duration::from_millis(10), &term)
            .unwrap();
        assert_eq!(idle.state, InputState::Idle);
    }

    #[test]
    fn claude_styled_rotated_placeholder_returns_drafting_to_idle() {
        let start = Instant::now();
        let mut term = new_term(50, 4);
        feed(
            &mut term,
            b"\x1b[2;1H\xe2\x9d\xaf \x1b[2mTry refactoring this file\x1b[22m",
        );
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content { text: "h".into() },
            },
            start,
            &term,
        );
        feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf h");
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(1), &term)
                .unwrap()
                .state,
            InputState::Drafting
        );

        feed(
            &mut term,
            b"\r\x1b[K\xe2\x9d\xaf \x1b[2mTry writing a test\x1b[22m",
        );
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(2), &term)
                .unwrap()
                .state,
            InputState::Idle
        );
    }

    #[test]
    fn restyled_live_draft_stays_drafting() {
        let start = Instant::now();
        let mut term = new_term(50, 4);
        feed(
            &mut term,
            b"\x1b[2;1H\xe2\x9d\xaf \x1b[2mTry refactoring this file\x1b[22m",
        );
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content {
                    text: "hello world".into(),
                },
            },
            start,
            &term,
        );
        feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf hello world");
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(1), &term)
                .unwrap()
                .state,
            InputState::Drafting
        );

        feed(
            &mut term,
            b"\r\x1b[K\xe2\x9d\xaf \x1b[2mhello world\x1b[22m",
        );
        assert!(tracker
            .observe_output(start + Duration::from_millis(2), &term)
            .is_none());
        assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
    }

    #[test]
    fn codex_styled_rotated_placeholder_returns_drafting_to_idle() {
        let start = Instant::now();
        let mut term = new_term(50, 4);
        feed(
            &mut term,
            b"\x1b[2;1H\xe2\x94\x82 \xe2\x80\xba \x1b[38;5;8mAsk Codex\x1b[39m             \xe2\x94\x82",
        );
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content { text: "h".into() },
            },
            start,
            &term,
        );
        feed(
            &mut term,
            b"\r\x1b[K\xe2\x94\x82 \xe2\x80\xba h                     \xe2\x94\x82",
        );
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(1), &term)
                .unwrap()
                .state,
            InputState::Drafting
        );

        feed(
            &mut term,
            b"\r\x1b[K\xe2\x94\x82 \xe2\x80\xba \x1b[38;5;8mReview this repository\x1b[39m\xe2\x94\x82",
        );
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(2), &term)
                .unwrap()
                .state,
            InputState::Idle
        );
    }

    #[test]
    fn explicit_reset_forces_a_draft_idle_once() {
        let start = Instant::now();
        let mut term = new_term(40, 4);
        feed(&mut term, b"\x1b[2;1H\xe2\x9d\xaf Try a task");
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content { text: "h".into() },
            },
            start,
            &term,
        );
        feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf h");
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(1), &term)
                .unwrap()
                .state,
            InputState::Drafting
        );

        let guard = tracker.reset_guard();
        let reset = tracker
            .reset_if_unchanged(guard, start + Duration::from_millis(2))
            .unwrap();
        assert_eq!(reset.state, InputState::Idle);
        assert!(!reset.composing);
        assert!(tracker
            .reset_if_unchanged(guard, start + Duration::from_millis(3))
            .is_none());
    }

    #[test]
    fn explicit_reset_does_not_clear_input_observed_after_the_guard() {
        let start = Instant::now();
        let mut term = new_term(40, 4);
        feed(&mut term, b"\x1b[2;1H\xe2\x9d\xaf Try a task");
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content { text: "h".into() },
            },
            start,
            &term,
        );
        feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf h");
        tracker.observe_output(start + Duration::from_millis(1), &term);
        let guard = tracker.reset_guard();
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content { text: "i".into() },
            },
            start + Duration::from_millis(2),
            &term,
        );

        assert!(tracker
            .reset_if_unchanged(guard, start + Duration::from_millis(3))
            .is_none());
        assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
    }

    #[test]
    fn submitted_accepts_a_rotated_empty_form() {
        let start = Instant::now();
        let mut term = new_term(40, 4);
        feed(&mut term, b"\x1b[2;1H\xe2\x9d\xaf First placeholder");
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content {
                    text: "hello".into(),
                },
            },
            start,
            &term,
        );
        feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf hello");
        tracker.observe_output(start + Duration::from_millis(5), &term);
        let submitted = tracker
            .observe_input(
                &InputEvent::Key {
                    kind: InputKind::Submit,
                },
                start + Duration::from_millis(6),
                &term,
            )
            .unwrap();
        assert_eq!(submitted.state, InputState::Submitted);
        feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf Different placeholder");
        let idle = tracker
            .observe_output(start + Duration::from_millis(9), &term)
            .unwrap();
        assert_eq!(idle.state, InputState::Idle);
    }

    #[test]
    fn hidden_composer_stays_drafting_until_the_grid_restores_it() {
        let start = Instant::now();
        let mut term = new_term(30, 3);
        feed(&mut term, b"\x1b[2;1H$ ");
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content { text: "x".into() },
            },
            start,
            &term,
        );
        feed(&mut term, b"\r\x1b[K$ x");
        let drafting = tracker
            .observe_output(start + Duration::from_millis(1), &term)
            .unwrap();
        assert_eq!(drafting.state, InputState::Drafting);
        feed(&mut term, b"\r\x1b[Kmenu");
        let hidden = tracker
            .observe_output(start + Duration::from_millis(2), &term)
            .unwrap();
        assert_eq!(hidden.state, InputState::Drafting);
        assert!(!hidden.composer_visible);
        feed(&mut term, b"\x1b[1;1H\x1b[K$ x");
        let visible = tracker
            .observe_output(start + Duration::from_millis(3), &term)
            .unwrap();
        assert!(visible.composer_visible);
        feed(&mut term, b"\r\x1b[K$ ");
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(4), &term)
                .unwrap()
                .state,
            InputState::Idle
        );
    }

    #[test]
    fn resize_below_the_tracked_row_relocates_and_observes_the_clear() {
        use alacritty_terminal::term::test::TermSize;

        let start = Instant::now();
        let mut term = new_term(30, 30);
        feed(&mut term, b"\x1b[25;1H$ ");
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content { text: "x".into() },
            },
            start,
            &term,
        );
        feed(&mut term, b"\r\x1b[K$ x");
        tracker.observe_output(start + Duration::from_millis(1), &term);

        term.resize(TermSize::new(30, 10));
        feed(&mut term, b"\x1b[2J\x1b[8;1H$ x");
        assert!(tracker
            .observe_output(start + Duration::from_millis(2), &term)
            .is_none());
        feed(&mut term, b"\r\x1b[K$ ");
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(3), &term)
                .unwrap()
                .state,
            InputState::Idle
        );
    }

    #[test]
    fn enter_over_a_hidden_composer_does_not_replace_the_submitted_form() {
        let start = Instant::now();
        let mut term = new_term(30, 3);
        feed(&mut term, b"\x1b[2;1H$ ");
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content { text: "x".into() },
            },
            start,
            &term,
        );
        feed(&mut term, b"\r\x1b[K$ x");
        tracker.observe_output(start + Duration::from_millis(1), &term);
        feed(&mut term, b"\r\x1b[Kmenu");
        tracker.observe_output(start + Duration::from_millis(2), &term);
        assert_eq!(
            tracker
                .observe_input(
                    &InputEvent::Key {
                        kind: InputKind::Submit,
                    },
                    start + Duration::from_millis(3),
                    &term,
                )
                .unwrap()
                .state,
            InputState::Submitted
        );
        feed(&mut term, b"\r\x1b[K$ x");
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(4), &term)
                .unwrap()
                .state,
            InputState::Drafting
        );
    }

    #[test]
    fn content_before_submit_redraw_rearms_drafting_without_an_idle_transition() {
        let start = Instant::now();
        let mut term = new_term(40, 4);
        feed(&mut term, b"\x1b[2;1H\xe2\x9d\xaf Try a task");
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content {
                    text: "hello".into(),
                },
            },
            start,
            &term,
        );
        feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf hello");
        tracker.observe_output(start + Duration::from_millis(1), &term);
        assert_eq!(
            tracker
                .observe_input(
                    &InputEvent::Key {
                        kind: InputKind::Submit,
                    },
                    start + Duration::from_millis(2),
                    &term,
                )
                .unwrap()
                .state,
            InputState::Submitted
        );
        assert!(tracker
            .observe_input(
                &InputEvent::Key {
                    kind: InputKind::Content { text: "n".into() },
                },
                start + Duration::from_millis(3),
                &term,
            )
            .is_none());
        feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf n");
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(4), &term)
                .unwrap()
                .state,
            InputState::Drafting
        );
    }

    #[test]
    fn submit_redraw_before_the_next_echo_keeps_the_reprobe_armed() {
        let start = Instant::now();
        let mut term = new_term(40, 4);
        feed(&mut term, b"\x1b[2;1H\xe2\x9d\xaf Try a task");
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content {
                    text: "hello".into(),
                },
            },
            start,
            &term,
        );
        feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf hello");
        tracker.observe_output(start + Duration::from_millis(1), &term);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Submit,
            },
            start + Duration::from_millis(2),
            &term,
        );
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content { text: "w".into() },
            },
            start + Duration::from_millis(3),
            &term,
        );

        feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf Try a task");
        assert!(tracker
            .observe_output(start + Duration::from_millis(4), &term)
            .is_none());
        feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf w");
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(5), &term)
                .unwrap()
                .state,
            InputState::Drafting
        );
    }

    #[test]
    fn submit_while_probing_preserves_the_probe() {
        let start = Instant::now();
        let mut term = new_term(30, 3);
        feed(&mut term, b"\x1b[2;1H$ ");
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content { text: "x".into() },
            },
            start,
            &term,
        );
        assert!(tracker
            .observe_input(
                &InputEvent::Key {
                    kind: InputKind::Submit,
                },
                start + Duration::from_millis(1),
                &term,
            )
            .is_none());
        feed(&mut term, b"\r\x1b[K$ x");
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(2), &term)
                .unwrap()
                .state,
            InputState::Drafting
        );
    }

    #[test]
    fn relocation_tracks_the_nearest_prompt_row_then_observes_its_clear() {
        let start = Instant::now();
        let mut term = new_term(30, 4);
        feed(&mut term, b"\x1b[2;1H$ ");
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content { text: "x".into() },
            },
            start,
            &term,
        );
        feed(&mut term, b"\r\x1b[K$ x");
        tracker.observe_output(start + Duration::from_millis(1), &term);
        feed(&mut term, b"\r\x1b[Kpopup\x1b[3;1H$ x");
        assert!(tracker
            .observe_output(start + Duration::from_millis(2), &term)
            .is_none());
        feed(&mut term, b"\r\x1b[K$ ");
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(3), &term)
                .unwrap()
                .state,
            InputState::Idle
        );
    }

    #[test]
    fn composition_reports_drafting_and_masks_enter_submission() {
        let start = Instant::now();
        let term = new_term(30, 3);
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        let composing = tracker
            .observe_input(
                &InputEvent::Composing { composing: true },
                start + Duration::from_millis(1),
                &term,
            )
            .unwrap();
        assert_eq!(composing.state, InputState::Drafting);
        assert!(composing.composing);
        assert!(tracker
            .observe_input(
                &InputEvent::Key {
                    kind: InputKind::Submit,
                },
                start + Duration::from_millis(2),
                &term,
            )
            .is_none());
        let idle = tracker
            .observe_input(
                &InputEvent::Composing { composing: false },
                start + Duration::from_millis(3),
                &term,
            )
            .unwrap();
        assert_eq!(idle.state, InputState::Idle);
    }

    #[test]
    #[ignore = "manual M6.1 handoff measurement"]
    fn measure_tracker_cost_on_a_200_by_60_grid() {
        const ITERATIONS: u32 = 20_000;
        let start = Instant::now();
        let mut term = new_term(200, 60);
        feed(&mut term, b"\x1b[59;1H\xe2\x9d\xaf Placeholder");
        let mut idle = InputTracker::new(start);
        idle.initial_observation(start);
        let measured_at = Instant::now();
        for _ in 0..ITERATIONS {
            std::hint::black_box(idle.observe_output(Instant::now(), &term));
        }
        let idle_us = measured_at.elapsed().as_secs_f64() * 1_000_000. / ITERATIONS as f64;

        let mut drafting = InputTracker::new(start);
        drafting.initial_observation(start);
        drafting.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content { text: "x".into() },
            },
            start,
            &term,
        );
        feed(&mut term, b"\r\x1b[K\xe2\x9d\xaf x");
        drafting.observe_output(start + Duration::from_millis(1), &term);
        let measured_at = Instant::now();
        for _ in 0..ITERATIONS {
            std::hint::black_box(drafting.observe_output(Instant::now(), &term));
        }
        let drafting_us = measured_at.elapsed().as_secs_f64() * 1_000_000. / ITERATIONS as f64;

        let measured_at = Instant::now();
        for _ in 0..ITERATIONS {
            std::hint::black_box(relocate_row(&term, "missing-prefix ", 30));
        }
        let relocation_us = measured_at.elapsed().as_secs_f64() * 1_000_000. / ITERATIONS as f64;
        eprintln!(
            "input tracker µs/chunk: idle={idle_us:.3} drafting={drafting_us:.3} relocation={relocation_us:.3}"
        );
    }
}
