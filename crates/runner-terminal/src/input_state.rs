use std::time::{Duration, Instant};

use alacritty_terminal::grid::Dimensions as _;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::Color;
pub use runner_core::protocol::terminal::InputEvent;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputState {
    Idle,
    Drafting,
    Submitted,
    /// Text only the router typed, still in the composer after its Enter did not take.
    Unsent,
}

#[derive(Clone, Copy, Debug)]
pub struct InputObservation {
    pub state: InputState,
    pub since: Instant,
    pub composing: bool,
    pub composer_visible: bool,
}

use crate::mappings::InputKind;
use crate::replay::row_to_string;

pub const ECHO_WINDOW: Duration = Duration::from_millis(500);

#[derive(Clone, Debug)]
struct Composer {
    row: usize,
    prefix: String,
    empty_forms: Vec<String>,
    last_form: String,
    visible: bool,
    submitted_form: Option<String>,
    placeholder_style: Option<CellStyle>,
    frame_border: Option<char>,
}

#[derive(Clone, Debug)]
struct Probe {
    typed: String,
    rows_before: Vec<String>,
    styles_before: Vec<Vec<CellStyle>>,
    started: Instant,
    weak_match: bool,
    submitted: Option<SubmitOrigin>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SubmitOrigin {
    Native,
    Delivery,
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
    session_id: String,
    state: TrackerState,
    composing: bool,
    reported: Option<ReportedState>,
    public_since: Instant,
    input_revision: u64,
    delivery_revision: Option<u64>,
    delivery_text: String,
    /// The router's latest line has been held here as its own since it was typed.
    delivery_held: bool,
    /// When text was last typed or pasted since the last Enter, so an empty editor before its
    /// echo releases nothing.
    content_at: Option<Instant>,
}

impl InputTracker {
    pub fn new(now: Instant) -> Self {
        Self::for_session(String::new(), now)
    }

    pub fn for_session(session_id: String, now: Instant) -> Self {
        Self {
            session_id,
            state: TrackerState::Idle,
            composing: false,
            reported: None,
            public_since: now,
            input_revision: 0,
            delivery_revision: None,
            delivery_text: String::new(),
            delivery_held: false,
            content_at: None,
        }
    }

    pub fn initial_observation(&mut self, now: Instant) -> InputObservation {
        self.emit_if_changed(now, "start")
            .expect("a new tracker must report its initial idle state")
    }

    pub fn latest_observation(&self) -> InputObservation {
        let reported = self
            .reported
            .expect("a tracker reports its initial observation first");
        InputObservation {
            state: reported.state,
            since: self.public_since,
            composing: reported.composing,
            composer_visible: reported.composer_visible,
        }
    }

    pub fn observe_input<T>(
        &mut self,
        event: &InputEvent,
        now: Instant,
        term: &Term<T>,
    ) -> Option<InputObservation> {
        // The app ends a composition before every typed key and whenever the IME unmarks, as on
        // a focus change; ending one that never started changes nothing a delivery holds (#1225).
        if !matches!(event, InputEvent::Composing { composing: false } if !self.composing) {
            self.input_revision = self.input_revision.wrapping_add(1);
        }
        self.expire_probe(now);
        match event {
            InputEvent::Composing { composing } => self.composing = *composing,
            InputEvent::Paste { text } => self.observe_content(text, true, now, term),
            InputEvent::Key { kind } => match kind {
                InputKind::Content { text } => self.observe_content(text, false, now, term),
                InputKind::Submit if !self.composing => {
                    self.submit(now, term, SubmitOrigin::Native)
                }
                InputKind::Edit | InputKind::Cancel => {
                    if let TrackerState::Drafting(composer) = &mut self.state {
                        if self.delivery_revision.is_none() {
                            composer.submitted_form = None;
                        }
                        // A redraw such as a Codex rollback can leave a transcript row with the
                        // prompt prefix at the tracked row; edits go to the live editor.
                        if let Some(row) = cursor_prompt_row(term, &composer.prefix) {
                            composer.row = row;
                        }
                    }
                }
                InputKind::Navigate | InputKind::Submit => {}
            },
        }
        let observation = self.emit_if_changed(now, "person input");
        if matches!(event, InputEvent::Composing { composing: true }) {
            observation.or_else(|| {
                let current = self.current_reported_state();
                Some(InputObservation {
                    state: current.state,
                    since: self.public_since,
                    composing: current.composing,
                    composer_visible: current.composer_visible,
                })
            })
        } else {
            observation
        }
    }

    pub fn track_delivery_input<T>(&mut self, text: &str, now: Instant, term: &Term<T>) {
        self.expire_probe(now);
        self.delivery_held = false;
        if matches!(self.state, TrackerState::Idle) {
            self.delivery_revision = Some(self.input_revision);
            self.delivery_text = text.to_owned();
        } else if self.delivery_owned() {
            self.delivery_revision = Some(self.input_revision);
            self.delivery_text.push_str(text);
        }
        self.observe_content(text, true, now, term);
    }

    /// The composer row still reads as it did at the router's Enter.
    pub fn unsent_notice<T>(&self, term: &Term<T>) -> bool {
        self.delivery_owned()
            && matches!(
                &self.state,
                TrackerState::Drafting(composer) | TrackerState::Submitted(composer)
                    if composer.row < term.screen_lines()
                        && composer.submitted_form.as_deref()
                            == Some(row_to_string(term, composer.row as i32).as_str())
            )
    }

    fn delivery_owned(&self) -> bool {
        self.delivery_revision.is_some()
            && matches!(
                &self.state,
                TrackerState::Drafting(composer) | TrackerState::Submitted(composer)
                    if composer.submitted_form.as_ref() == Some(&composer.last_form)
            )
    }

    pub fn track_delivery_submit<T>(&mut self, now: Instant, term: &Term<T>) {
        self.submit(now, term, SubmitOrigin::Delivery);
        self.delivery_held |= self.delivery_owned();
    }

    /// The router's latest line was held here as its own after its Enter, even if it has since
    /// been let go of, which tells a line that left the composer from one never located (#1225).
    pub fn delivery_held(&self) -> bool {
        self.delivery_held
    }

    fn content_pending(&self, now: Instant) -> bool {
        self.content_at
            .is_some_and(|at| now.saturating_duration_since(at) <= ECHO_WINDOW)
    }

    /// The router saw its notice submitted, so a row that still repeats it is history. Input
    /// observed after the router's `revision` keeps the composer as the person left it.
    pub fn confirm_delivery(&mut self, revision: u64, now: Instant) -> Option<InputObservation> {
        if self.input_revision != revision {
            return None;
        }
        if self.delivery_owned() {
            self.state = TrackerState::Idle;
        }
        self.delivery_revision = None;
        self.emit_if_changed(now, "ownership cleared")
    }

    /// With no router attempt pending, keep the notice only where the input cursor still
    /// sits on it or on the rows it wrapped onto, or once a person has used the terminal since.
    /// A notice or draft is dropped wherever the input cursor sits at an empty composer instead.
    pub fn recheck_delivery<T>(
        &mut self,
        now: Instant,
        term: &Term<T>,
    ) -> Option<InputObservation> {
        let (live, editor_empty) = match &self.state {
            TrackerState::Drafting(composer) | TrackerState::Submitted(composer) => (
                composer.visible
                    && self.unsent_notice(term)
                    && cursor_in_notice(term, composer, &self.delivery_text),
                composer.visible && cursor_at_empty_composer(term, composer),
            ),
            _ => (false, false),
        };
        if (self.delivery_owned() && self.delivery_revision == Some(self.input_revision) && !live)
            || (editor_empty && !self.composing && !self.content_pending(now))
        {
            self.state = TrackerState::Idle;
            self.delivery_revision = None;
        }
        self.emit_if_changed(now, "re-check")
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
                        term.columns(),
                    ) {
                        Some(matched) => {
                            let placeholder_style = probe_placeholder_style(&probe, &matched, term);
                            let composer = Composer {
                                row: matched.row,
                                prefix: matched.prefix,
                                empty_forms: vec![probe.rows_before[matched.before_row].clone()],
                                last_form: rows_after[matched.row].clone(),
                                visible: true,
                                submitted_form: probe
                                    .submitted
                                    .is_some()
                                    .then(|| rows_after[matched.row].clone()),
                                placeholder_style,
                                frame_border: composer_frame_border(
                                    &probe.rows_before,
                                    matched.before_row,
                                ),
                            };
                            // Native Return can be rejected while leaving this first echo intact.
                            if probe.submitted == Some(SubmitOrigin::Delivery) {
                                TrackerState::Submitted(composer)
                            } else {
                                TrackerState::Drafting(composer)
                            }
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
                    term.columns(),
                ) {
                    let before = &probe.rows_before[matched.before_row];
                    if composer_text_is_empty(before, &matched.prefix, &composer.empty_forms)
                        && !composer.empty_forms.contains(before)
                    {
                        composer.empty_forms.push(before.clone());
                    }
                    if let Some(style) = probe_placeholder_style(&probe, &matched, term) {
                        composer.placeholder_style = Some(style);
                    }
                    if let Some(border) =
                        composer_frame_border(&probe.rows_before, matched.before_row)
                    {
                        composer.frame_border = Some(border);
                    }
                    composer.row = matched.row;
                    composer.prefix = matched.prefix;
                    composer.last_form = rows_after[matched.row].clone();
                    composer.visible = true;
                    composer.submitted_form = probe
                        .submitted
                        .is_some()
                        .then(|| composer.last_form.clone());
                    if probe.submitted == Some(SubmitOrigin::Delivery) {
                        TrackerState::Submitted(composer)
                    } else {
                        TrackerState::Drafting(composer)
                    }
                } else if now.duration_since(probe.started) <= ECHO_WINDOW {
                    TrackerState::Reprobing { probe, composer }
                } else {
                    self.observe_composer(now, term, composer, true)
                }
            }
            TrackerState::Drafting(composer) => self.observe_composer(now, term, composer, false),
            TrackerState::Submitted(composer) => self.observe_composer(now, term, composer, true),
        };
        self.delivery_held |= self.delivery_owned();
        // Submitted text can remain in history with the same prefix as the live composer; text
        // typed since the Enter may not have reached that composer yet.
        if !self.content_pending(now)
            && matches!(
                &self.state,
                TrackerState::Submitted(composer) | TrackerState::Drafting(composer)
                    if composer.submitted_form.is_some() && cursor_at_empty_composer(term, composer)
            )
        {
            self.state = TrackerState::Idle;
        }
        self.emit_if_changed(now, "composer changed")
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
        self.emit_if_changed(now, "reset")
    }

    fn observe_content<T>(&mut self, text: &str, pasted: bool, now: Instant, term: &Term<T>) {
        if text.is_empty() {
            return;
        }
        self.content_at = Some(now);
        let state = std::mem::replace(&mut self.state, TrackerState::Idle);
        self.state = match state {
            TrackerState::Idle => TrackerState::Probing(new_probe(text, pasted, now, term)),
            TrackerState::Probing(mut probe) => {
                probe.typed.push_str(&probe_text(text));
                probe.weak_match |= pasted && weak_paste_match(text);
                probe.started = now;
                probe.submitted = None;
                TrackerState::Probing(probe)
            }
            TrackerState::Reprobing {
                mut probe,
                composer,
            } => {
                probe.typed.push_str(&probe_text(text));
                probe.weak_match |= pasted && weak_paste_match(text);
                probe.started = now;
                probe.submitted = None;
                TrackerState::Reprobing { probe, composer }
            }
            TrackerState::Drafting(mut composer) => {
                if self.delivery_revision.is_none() {
                    composer.submitted_form = None;
                }
                TrackerState::Drafting(composer)
            }
            TrackerState::Submitted(composer) if self.delivery_revision.is_some() => {
                TrackerState::Drafting(composer)
            }
            TrackerState::Submitted(composer) => TrackerState::Reprobing {
                probe: new_probe(text, pasted, now, term),
                composer,
            },
        };
    }

    fn submit<T>(&mut self, _now: Instant, term: &Term<T>, origin: SubmitOrigin) {
        self.content_at = None;
        if origin == SubmitOrigin::Native && !self.delivery_owned() {
            self.delivery_revision = None;
        }
        let state = std::mem::replace(&mut self.state, TrackerState::Idle);
        match state {
            TrackerState::Probing(mut probe) => {
                probe.submitted = Some(origin);
                self.state = TrackerState::Probing(probe);
            }
            TrackerState::Reprobing {
                mut probe,
                composer,
            } => {
                probe.submitted = Some(origin);
                self.state = TrackerState::Reprobing { probe, composer };
            }
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
        now: Instant,
        term: &Term<T>,
        mut composer: Composer,
        submitted: bool,
    ) -> TrackerState {
        // Submitted prompts can remain in history at the old row. A unique
        // learned frame plus the input cursor distinguishes the current editor.
        if term.grid().cursor.point.line != Line(composer.row as i32) {
            if let Some(row) = composer
                .frame_border
                .and_then(|border| framed_composer_row(term, &composer.prefix, border))
            {
                composer.row = row;
            }
        }
        let mut current = if composer.row < term.screen_lines() {
            row_to_string(term, composer.row as i32)
        } else if let Some(row) = relocate_row(
            term,
            &composer.prefix,
            composer.row.min(term.screen_lines().saturating_sub(1)),
            composer.frame_border,
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
            if let Some(row) =
                relocate_row(term, &composer.prefix, composer.row, composer.frame_border)
            {
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

        if let Some(submitted_form) = composer.submitted_form.as_ref() {
            if current == *submitted_form {
                return TrackerState::Drafting(composer);
            }
            // The person's input reaches the editor under the cursor: while that one is empty and
            // nothing typed awaits its echo, the changed row is transcript, not their draft (#1225).
            if self
                .delivery_revision
                .is_some_and(|revision| revision != self.input_revision)
                && (self.content_pending(now) || !cursor_at_empty_composer(term, &composer))
            {
                composer.submitted_form = None;
                composer.last_form = current;
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

    fn emit_if_changed(&mut self, now: Instant, released_by: &str) -> Option<InputObservation> {
        let current = self.current_reported_state();
        if self.reported == Some(current) {
            return None;
        }
        let was_unsent = self
            .reported
            .is_some_and(|previous| previous.state == InputState::Unsent);
        if current.state == InputState::Unsent && !was_unsent {
            let reason = if current.composer_visible {
                "row match"
            } else {
                "composer hidden"
            };
            log::info!(
                "input tracker reports unsent session={} reason={reason}",
                self.session_id
            );
        } else if was_unsent && current.state != InputState::Unsent {
            log::info!(
                "input tracker stopped reporting unsent session={} reason={released_by} state={:?}",
                self.session_id,
                current.state
            );
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
                state: if self.delivery_owned() {
                    InputState::Unsent
                } else {
                    InputState::Drafting
                },
                composing: false,
                composer_visible: composer.visible,
            },
            // An ignored Enter may never be followed by output that would turn this into Drafting.
            TrackerState::Submitted(composer) => ReportedState {
                state: if self.delivery_owned() {
                    InputState::Unsent
                } else {
                    InputState::Submitted
                },
                composing: false,
                composer_visible: composer.visible,
            },
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct EchoMatch {
    row: usize,
    before_row: usize,
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

pub(crate) fn visible_rows<T>(term: &Term<T>) -> Vec<String> {
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
        submitted: None,
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
        &probe.rows_before[matched.before_row],
        &probe.styles_before[matched.before_row],
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
    columns: usize,
) -> Option<EchoMatch> {
    after
        .iter()
        .enumerate()
        .filter(|(row, text)| before.get(*row) != Some(*text))
        .flat_map(|(row, text)| {
            before
                .iter()
                .enumerate()
                .filter(move |(before_row, before)| after.get(*before_row) != Some(*before))
                .filter_map(move |(before_row, before)| {
                    match_prompt_prefix(before, text, typed, weak_match, columns).map(
                        |(prefix, kind)| EchoMatch {
                            row,
                            before_row,
                            empty_before: before.trim_end() == prefix.trim_end(),
                            prefix,
                            kind,
                        },
                    )
                })
        })
        .min_by_key(|matched| {
            (
                matched.kind,
                matched.row != matched.before_row,
                !matched.empty_before,
                matched.prefix.chars().count(),
                matched.row.abs_diff(matched.before_row),
                matched.row,
            )
        })
}

fn match_prompt_prefix(
    before: &str,
    after: &str,
    typed: &str,
    weak_match: bool,
    columns: usize,
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
    let prefix = common_prefix(before, after);
    let prefix = if looks_like_prompt_prefix(prefix) {
        prefix
    } else {
        prefix.char_indices().rev().find_map(|(index, character)| {
            let candidate = &prefix[..index + character.len_utf8()];
            looks_like_prompt_prefix(candidate).then_some(candidate)
        })?
    };
    if !weak_match && typed.width() <= columns.saturating_sub(prefix.width()) {
        return None;
    }
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

fn cursor_at_empty_composer<T>(term: &Term<T>, composer: &Composer) -> bool {
    let cursor = term.grid().cursor.point;
    if cursor.line.0 < 0 || cursor.column.0 != composer.prefix.width() {
        return false;
    }
    let row = cursor.line.0 as usize;
    let current = row_to_string(term, cursor.line.0);
    (current.starts_with(&composer.prefix) || current.trim_end() == composer.prefix.trim_end())
        && composer_is_empty(term, row, &current, composer)
}

/// The cursor is on the composer row, or below it on rows that only continue the notice.
fn cursor_in_notice<T>(term: &Term<T>, composer: &Composer, notice: &str) -> bool {
    let Ok(cursor) = usize::try_from(term.grid().cursor.point.line.0) else {
        return false;
    };
    cursor >= composer.row
        && (composer.row + 1..=cursor).all(|row| {
            let text = row_to_string(term, row as i32);
            let text = text.trim();
            !text.is_empty() && notice.contains(text)
        })
}

fn composer_frame_border(rows: &[String], row: usize) -> Option<char> {
    let border = rows.get(row.checked_sub(1)?)?;
    let glyph = border.chars().next()?;
    (matches!(glyph, '─' | '━' | '═')
        && border.chars().count() >= 3
        && border.chars().all(|character| character == glyph)
        && rows.get(row + 1) == Some(border))
    .then_some(glyph)
}

fn framed_composer_row<T>(term: &Term<T>, prefix: &str, border: char) -> Option<usize> {
    let rows = visible_rows(term);
    let mut candidates = rows.iter().enumerate().filter_map(|(row, text)| {
        ((text.starts_with(prefix) || text.trim_end() == prefix.trim_end())
            && composer_frame_border(&rows, row) == Some(border))
        .then_some(row)
    });
    let row = candidates.next()?;
    let cursor = term.grid().cursor.point;
    (candidates.next().is_none()
        && cursor.line == Line(row as i32)
        && cursor.column.0 >= prefix.width())
    .then_some(row)
}

fn cursor_prompt_row<T>(term: &Term<T>, prefix: &str) -> Option<usize> {
    let cursor = term.grid().cursor.point;
    let row = usize::try_from(cursor.line.0).ok()?;
    let text = row_to_string(term, cursor.line.0);
    ((text.starts_with(prefix) || text.trim_end() == prefix.trim_end())
        && cursor.column.0 >= prefix.width())
    .then_some(row)
}

fn relocate_row<T>(
    term: &Term<T>,
    prefix: &str,
    old_row: usize,
    frame_border: Option<char>,
) -> Option<usize> {
    if let Some(border) = frame_border {
        // The frame can change shape after a resize or mode switch; the input
        // cursor still marks the live editor where a history row cannot.
        return framed_composer_row(term, prefix, border)
            .or_else(|| cursor_prompt_row(term, prefix));
    }
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
mod tests;
