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
mod tests {
    use super::*;
    use crate::replay::{feed, new_term};

    #[test]
    fn matcher_requires_a_prompt_prefix_and_prefers_the_empty_form() {
        let before = vec!["assistant ".into(), "❯ Try a task".into(), "$ ".into()];
        let after = vec!["assistant h".into(), "❯ h".into(), "$ h".into()];
        assert_eq!(
            locate_echo(&before, &after, "h", false, 40),
            Some(EchoMatch {
                row: 2,
                before_row: 2,
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
            assert!(locate_echo(&[before.into()], &[after.into()], "h", false, 40).is_some());
        }

        let before = vec!["  └ ".into(), "> Try a task            <ret> send".into()];
        let after = vec![
            "  └ hooked 12 files".into(),
            "> h                     <ret> send".into(),
        ];
        assert_eq!(locate_echo(&before, &after, "h", false, 40).unwrap().row, 1);
    }

    #[test]
    fn a_transcript_echo_cannot_reuse_an_unchanged_prompt_on_another_row() {
        let before = vec!["old output".into(), "› Ask Codex".into()];
        let after = vec!["› hello".into(), "› Ask Codex".into()];
        assert!(locate_echo(&before, &after, "hello", false, 40).is_none());
    }

    #[test]
    fn coalesced_native_input_holds_across_the_prompt_wrap_boundary() {
        use crate::replay::screen_snapshot;

        for (prefix, prompt_width) in [("› ", 2), ("界› ", 4)] {
            let available = 12 - prompt_width;
            for length in [available, available + 1, 12, 13] {
                let start = Instant::now();
                let mut term = new_term(12, 4);
                feed(&mut term, format!("{prefix}Ask").as_bytes());
                let mut tracker = InputTracker::new(start);
                tracker.initial_observation(start);
                let text = &"abcdefghijklm"[..length];
                for (ms, character) in text.chars().enumerate() {
                    tracker.observe_input(
                        &InputEvent::Key {
                            kind: InputKind::Content {
                                text: character.to_string(),
                            },
                        },
                        start + Duration::from_millis(ms as u64),
                        &term,
                    );
                }
                let split = text.len().min(available);
                feed(
                    &mut term,
                    format!("\x1b[H\x1b[J{prefix}{}", &text[..split]).as_bytes(),
                );
                if split < text.len() {
                    feed(
                        &mut term,
                        format!("\r\n{}{}", " ".repeat(prompt_width), &text[split..]).as_bytes(),
                    );
                }
                tracker.observe_output(start + Duration::from_millis(30), &term);
                tracker.observe_output(start + Duration::from_secs(3), &term);
                assert_eq!(
                    tracker.current_reported_state().state,
                    InputState::Drafting,
                    "prefix={prefix:?} length={length}\n{}",
                    screen_snapshot(&term)
                );
                feed(&mut term, format!("\x1b[H\x1b[J{prefix}Ask").as_bytes());
                assert_eq!(
                    tracker
                        .observe_output(start + Duration::from_secs(4), &term)
                        .unwrap()
                        .state,
                    InputState::Idle
                );
            }
        }
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
    fn coalesced_composer_echo_and_clear_stays_idle() {
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
        let burst = [
            b"\x1b[2;1H\x1b[K\xe2\x9d\xaf h\x1b[4;1H\x1b[K  \xe2\x94\x94 hooked 12 files"
                .as_slice(),
            b"\x1b[2;1H\x1b[K\xe2\x9d\xaf Try a task",
        ]
        .concat();
        feed(&mut term, &burst);
        assert!(tracker
            .observe_output(start + Duration::from_millis(2), &term)
            .is_none());
        assert_eq!(tracker.current_reported_state().state, InputState::Idle);
    }

    #[test]
    fn matcher_accepts_collapsed_multiline_paste_but_not_a_transcript_row() {
        assert_eq!(
            match_prompt_prefix(
                "❯ Try a task",
                "❯ [Pasted text #1 +19 lines]",
                "line one",
                true,
                40,
            ),
            Some(("❯ ".into(), EchoMatchKind::Weak))
        );
        assert_eq!(
            match_prompt_prefix("assistant ", "assistant [Pasted text]", "line", true, 40),
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
    fn clearing_a_draft_after_a_rollback_redraw_releases_the_hold() {
        for kind in [InputKind::Edit, InputKind::Cancel] {
            let start = Instant::now();
            let mut term = new_term(60, 12);
            feed(
                &mut term,
                "\x1b[4;1H› \x1b[90mAsk Codex to do anything\x1b[39m\x1b[4;3H".as_bytes(),
            );
            let mut tracker = InputTracker::new(start);
            tracker.initial_observation(start);
            tracker.observe_input(
                &InputEvent::Key {
                    kind: InputKind::Content {
                        text: "fix it".into(),
                    },
                },
                start,
                &term,
            );
            feed(&mut term, "\x1b[4;1H\x1b[K› fix it".as_bytes());
            assert_eq!(
                tracker
                    .observe_output(start + Duration::from_millis(1), &term)
                    .unwrap()
                    .state,
                InputState::Drafting
            );

            // Codex redraws the trimmed transcript; a user row now sits at the tracked row.
            feed(
                &mut term,
                "\x1b[H\x1b[2J› first question\r\n\r\n• answer one\r\n› second question\r\n\r\n• answer two\x1b[10;1H› fix it".as_bytes(),
            );
            tracker.observe_output(start + Duration::from_millis(2), &term);
            assert_eq!(tracker.current_reported_state().state, InputState::Drafting);

            tracker.observe_input(&InputEvent::Key { kind: kind.clone() }, start, &term);
            if kind == InputKind::Edit {
                feed(&mut term, "\x1b[10;1H\x1b[K› fix".as_bytes());
                tracker.observe_output(start + Duration::from_millis(3), &term);
                assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
                tracker.observe_input(&InputEvent::Key { kind: kind.clone() }, start, &term);
            }
            feed(
                &mut term,
                "\x1b[10;1H\x1b[K› \x1b[90mAsk Codex to do anything\x1b[39m\x1b[10;3H".as_bytes(),
            );
            tracker.observe_output(start + Duration::from_millis(4), &term);
            assert_eq!(
                tracker.current_reported_state().state,
                InputState::Idle,
                "{kind:?}"
            );
        }
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
    fn antigravity_submitted_history_does_not_hold_the_empty_composer() {
        use crate::fixtures::{decode_chunk, Fixture, FixtureEvent};

        for injected in [false, true] {
            let start = Instant::now();
            let mut term = new_term(110, 36);
            let border = "─".repeat(110);
            feed(
                &mut term,
                format!("\x1b[?1049h\x1b[8;1H{border}\r\n> \r\n{border}").as_bytes(),
            );
            let mut tracker = InputTracker::new(start);
            tracker.initial_observation(start);
            let text = "Reply with exactly OK.";
            if injected {
                tracker.track_delivery_input(text, start, &term);
            } else {
                tracker.observe_input(
                    &InputEvent::Key {
                        kind: InputKind::Content { text: text.into() },
                    },
                    start,
                    &term,
                );
            }
            feed(&mut term, format!("\x1b[9;1H> {text}").as_bytes());
            assert_eq!(
                tracker
                    .observe_output(start + Duration::from_millis(1), &term)
                    .unwrap()
                    .state,
                InputState::Drafting
            );
            if injected {
                tracker.track_delivery_submit(start + Duration::from_millis(2), &term);
            } else {
                tracker.observe_input(
                    &InputEvent::Key {
                        kind: InputKind::Submit,
                    },
                    start + Duration::from_millis(2),
                    &term,
                );
            }

            let fixture = Fixture::load(
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("fixtures/agy-first-turn.ndjson"),
            )
            .unwrap();
            for event in fixture.events {
                if let FixtureEvent::Data { data, .. } = event {
                    feed(&mut term, &decode_chunk(&data).unwrap());
                }
            }
            assert_eq!(row_to_string(&term, 8), "> Reply with exactly OK.");
            assert_eq!(row_to_string(&term, 13), ">");
            tracker.observe_output(start + Duration::from_millis(3), &term);
            assert_eq!(
                tracker.current_reported_state().state,
                InputState::Idle,
                "injected={injected}"
            );
        }
    }

    #[test]
    fn framed_composer_relocation_keeps_a_live_draft_held() {
        let start = Instant::now();
        let mut term = new_term(40, 12);
        let border = "─".repeat(40);
        feed(
            &mut term,
            format!("\x1b[2;1H{border}\r\n> \r\n{border}").as_bytes(),
        );
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content {
                    text: "draft".into(),
                },
            },
            start,
            &term,
        );
        feed(&mut term, b"\x1b[3;1H> draft");
        tracker.observe_output(start + Duration::from_millis(1), &term);
        feed(
            &mut term,
            format!("\x1b[4;1H\x1b[K\x1b[7;1H{border}\r\n> draft\r\n{border}\x1b[8;8H").as_bytes(),
        );
        tracker.observe_output(start + Duration::from_millis(2), &term);
        assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
        feed(&mut term, b"\x1b[8;1H> \x1b[K");
        tracker.observe_output(start + Duration::from_millis(3), &term);
        assert_eq!(tracker.current_reported_state().state, InputState::Idle);
    }

    #[test]
    fn framed_composer_keeps_multiline_drafts_held_and_releases_after_resize() {
        use alacritty_terminal::term::test::TermSize;

        let start = Instant::now();
        let mut term = new_term(40, 12);
        let border = "─".repeat(40);
        feed(
            &mut term,
            format!("\x1b[2;1H{border}\r\n> \r\n{border}").as_bytes(),
        );
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Paste {
                text: "first\nsecond".into(),
            },
            start,
            &term,
        );
        feed(
            &mut term,
            format!("\x1b[3;1H> first\r\nsecond\r\n{border}").as_bytes(),
        );
        tracker.observe_output(start + Duration::from_millis(1), &term);
        assert_eq!(tracker.current_reported_state().state, InputState::Drafting);

        term.resize(TermSize::new(30, 12));
        let border = "─".repeat(30);
        feed(
            &mut term,
            format!(
                "\x1b[H\x1b[J> first\r\nsecond\x1b[7;1H{border}\r\n> first\r\nsecond\r\n{border}"
            )
            .as_bytes(),
        );
        tracker.observe_output(start + Duration::from_millis(2), &term);
        assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
        feed(
            &mut term,
            format!("\x1b[8;1H> \x1b[K\r\n{border}\r\n\x1b[K\x1b[8;3H").as_bytes(),
        );
        tracker.observe_output(start + Duration::from_millis(3), &term);
        assert_eq!(tracker.current_reported_state().state, InputState::Idle);
    }

    #[test]
    fn an_unrelated_frame_cannot_release_a_multiline_or_hidden_draft() {
        for hidden in [false, true] {
            let start = Instant::now();
            let mut term = new_term(40, 12);
            let border = "─".repeat(40);
            feed(
                &mut term,
                format!("\x1b[2;1H{border}\r\n> Ask\r\n{border}").as_bytes(),
            );
            let mut tracker = InputTracker::new(start);
            tracker.initial_observation(start);
            tracker.observe_input(
                &InputEvent::Paste {
                    text: "first\nsecond".into(),
                },
                start,
                &term,
            );
            feed(
                &mut term,
                format!("\x1b[3;1H> first\r\nsecond\r\n{border}").as_bytes(),
            );
            tracker.observe_output(start + Duration::from_millis(1), &term);
            if hidden {
                feed(&mut term, b"\x1b[3;1Hmenu\x1b[K\r\nchoice\x1b[K");
            }
            feed(
                &mut term,
                format!("\x1b[8;1H{border}\r\n> Ask\r\n{border}\x1b[4;7H").as_bytes(),
            );
            tracker.observe_output(start + Duration::from_millis(2), &term);
            assert_eq!(
                tracker.current_reported_state().state,
                InputState::Drafting,
                "hidden={hidden}"
            );
        }
    }

    #[test]
    fn ambiguous_framed_prompts_cannot_release_a_live_draft() {
        let start = Instant::now();
        let mut term = new_term(40, 12);
        let border = "─".repeat(40);
        feed(
            &mut term,
            format!("\x1b[2;1H{border}\r\n> \r\n{border}").as_bytes(),
        );
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content {
                    text: "draft".into(),
                },
            },
            start,
            &term,
        );
        feed(&mut term, b"\x1b[3;1H> draft");
        tracker.observe_output(start + Duration::from_millis(1), &term);
        feed(
            &mut term,
            format!("\x1b[7;1H{border}\r\n> \r\n{border}").as_bytes(),
        );
        tracker.observe_output(start + Duration::from_millis(2), &term);
        assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
    }

    #[test]
    fn claude_framed_composer_releases_after_output_moves_it() {
        let start = Instant::now();
        let mut term = new_term(40, 12);
        let border = "─".repeat(40);
        feed(
            &mut term,
            format!("\x1b[2;1H{border}\r\n❯ \r\n{border}").as_bytes(),
        );
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content {
                    text: "draft".into(),
                },
            },
            start,
            &term,
        );
        feed(&mut term, "\x1b[3;1H❯ draft".as_bytes());
        tracker.observe_output(start + Duration::from_millis(1), &term);
        feed(
            &mut term,
            format!(
                "\x1b[2;1H● working\x1b[K\r\n\x1b[K\r\n\x1b[K\x1b[5;1H{border}\r\n❯ draft\r\n{border}\x1b[6;8H"
            )
            .as_bytes(),
        );
        tracker.observe_output(start + Duration::from_millis(2), &term);
        assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
        feed(&mut term, "\x1b[6;1H❯ \x1b[K\x1b[6;3H".as_bytes());
        tracker.observe_output(start + Duration::from_millis(3), &term);
        assert_eq!(tracker.current_reported_state().state, InputState::Idle);
    }

    #[test]
    fn a_reshaped_frame_still_releases_the_cleared_composer_under_the_cursor() {
        let start = Instant::now();
        let mut term = new_term(40, 12);
        let border = "─".repeat(40);
        feed(
            &mut term,
            format!("\x1b[2;1H{border}\r\n> \r\n{border}").as_bytes(),
        );
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content {
                    text: "draft".into(),
                },
            },
            start,
            &term,
        );
        feed(&mut term, b"\x1b[3;1H> draft");
        tracker.observe_output(start + Duration::from_millis(1), &term);
        feed(
            &mut term,
            format!(
                "\x1b[2;1H\x1b[K\r\n\x1b[K\r\n\x1b[K\x1b[7;1H── plan ──\x1b[K\r\n> \x1b[K\r\n{border}\x1b[8;3H"
            )
            .as_bytes(),
        );
        tracker.observe_output(start + Duration::from_millis(2), &term);
        assert_eq!(tracker.current_reported_state().state, InputState::Idle);
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
    fn delivery_submit_before_echo_accepts_a_rotated_empty_form_without_local_input() {
        let start = Instant::now();
        let mut term = new_term(80, 4);
        feed(&mut term, "\x1b[2;1H❯ First placeholder".as_bytes());
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.track_delivery_input("[inbox] task", start, &term);
        tracker.track_delivery_submit(start, &term);
        feed(&mut term, "\r\x1b[K❯ [inbox] task".as_bytes());
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(5), &term)
                .unwrap()
                .state,
            InputState::Unsent
        );
        // An ignored Enter with no later output leaves the notice where the guard can resubmit it.
        assert!(tracker.unsent_notice(&term));
        feed(&mut term, "\r\x1b[K❯ Different placeholder".as_bytes());
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(9), &term)
                .unwrap()
                .state,
            InputState::Idle
        );
        assert_eq!(tracker.reset_guard(), 0);
    }

    #[test]
    fn local_content_after_a_generated_submit_remains_a_draft() {
        let start = Instant::now();
        let mut term = new_term(80, 4);
        feed(&mut term, "\x1b[2;1H❯ First placeholder".as_bytes());
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.track_delivery_input("[inbox] task", start, &term);
        tracker.track_delivery_submit(start, &term);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content {
                    text: " draft".into(),
                },
            },
            start,
            &term,
        );
        feed(&mut term, "\r\x1b[K❯ [inbox] task draft".as_bytes());
        assert_eq!(
            tracker
                .observe_output(start + Duration::from_millis(5), &term)
                .unwrap()
                .state,
            InputState::Drafting
        );
    }

    #[test]
    fn an_ignored_delivery_enter_leaves_the_routers_notice_until_a_person_types() {
        let text = "[inbox] 1 new messages — run `runner msg read` to view.";
        for person in [false, true] {
            let start = Instant::now();
            let mut term = new_term(80, 4);
            feed(&mut term, "\x1b[2;1H› Ask Codex".as_bytes());
            let mut tracker = InputTracker::new(start);
            tracker.initial_observation(start);
            if person {
                tracker.observe_input(&InputEvent::Paste { text: text.into() }, start, &term);
            } else {
                tracker.track_delivery_input(text, start, &term);
            }
            feed(&mut term, format!("\r\x1b[K› {text}").as_bytes());
            tracker.observe_output(start + Duration::from_millis(5), &term);
            if person {
                tracker.observe_input(
                    &InputEvent::Key {
                        kind: InputKind::Submit,
                    },
                    start + Duration::from_millis(300),
                    &term,
                );
            } else {
                tracker.track_delivery_submit(start + Duration::from_millis(300), &term);
            }
            feed(&mut term, format!("\r\x1b[K› {text}").as_bytes());
            tracker.observe_output(start + Duration::from_millis(400), &term);
            tracker.observe_output(start + Duration::from_secs(26 * 60), &term);
            assert_eq!(
                tracker.current_reported_state().state,
                if person {
                    InputState::Drafting
                } else {
                    InputState::Unsent
                },
                "person={person}"
            );
            assert_eq!(tracker.unsent_notice(&term), !person, "person={person}");
            if person {
                continue;
            }
            tracker.observe_input(
                &InputEvent::Key {
                    kind: InputKind::Content { text: "!".into() },
                },
                start + Duration::from_secs(26 * 60 + 1),
                &term,
            );
            feed(&mut term, format!("\r\x1b[K› {text}!").as_bytes());
            let observation = tracker
                .observe_output(start + Duration::from_secs(26 * 60 + 2), &term)
                .unwrap();
            assert_eq!(observation.state, InputState::Drafting);
            assert!(!tracker.unsent_notice(&term));
        }
    }

    const NOTICE: &str = "[inbox] 1 new messages — run `runner msg read` to view.";

    /// The router types its notice into the composer at row 4 and presses Enter.
    fn submitted_notice(
        cols: u16,
        echo: &str,
    ) -> (
        Instant,
        Term<alacritty_terminal::event::VoidListener>,
        InputTracker,
    ) {
        let start = Instant::now();
        let mut term = new_term(cols, 8);
        feed(&mut term, b"\x1b[5;1H> Try a task");
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.track_delivery_input(NOTICE, start, &term);
        feed(&mut term, echo.as_bytes());
        tracker.observe_output(start + Duration::from_millis(1), &term);
        tracker.track_delivery_submit(start + Duration::from_millis(80), &term);
        (start, term, tracker)
    }

    #[test]
    fn a_confirmed_notice_is_not_unsent_where_a_history_row_repeats_it() {
        let echo = format!("\x1b[5;1H\x1b[K> {NOTICE}");
        let (start, mut term, mut tracker) = submitted_notice(80, &echo);
        // The prompt stays in history at the composer row; the cursor is parked below the output.
        let history =
            format!("{echo}\x1b[6;1H\x1b[K⏺ Read 1 message\x1b[7;1H\x1b[K> \x1b[8;1H\x1b[K");
        feed(&mut term, history.as_bytes());
        tracker.observe_output(start + Duration::from_millis(100), &term);
        assert_eq!(tracker.current_reported_state().state, InputState::Unsent);

        let revision = tracker.reset_guard();
        assert_eq!(
            tracker
                .confirm_delivery(revision, start + Duration::from_millis(1600))
                .unwrap()
                .state,
            InputState::Idle
        );
        feed(&mut term, history.as_bytes());
        tracker.observe_output(start + Duration::from_secs(60), &term);
        assert_eq!(tracker.current_reported_state().state, InputState::Idle);
        assert!(!tracker.unsent_notice(&term));
    }

    #[test]
    fn person_input_after_the_routers_revision_survives_confirmation_and_recheck() {
        for input in [
            InputEvent::Key {
                kind: InputKind::Content {
                    text: "draft".into(),
                },
            },
            InputEvent::Paste {
                text: "draft".into(),
            },
        ] {
            let echo = format!("\x1b[5;1H\x1b[K> {NOTICE}");
            let (start, mut term, mut tracker) = submitted_notice(80, &echo);
            feed(
                &mut term,
                format!("{echo}\x1b[6;1H\x1b[K⏺ Read 1 message\x1b[7;1H\x1b[K> \x1b[8;1H\x1b[K")
                    .as_bytes(),
            );
            tracker.observe_output(start + Duration::from_millis(100), &term);
            let revision = tracker.reset_guard();
            // A person types before the router's confirmation and before the echo.
            tracker.observe_input(&input, start + Duration::from_millis(200), &term);
            assert!(tracker
                .confirm_delivery(revision, start + Duration::from_millis(1600))
                .is_none());
            feed(&mut term, b"\x1b[7;1H\x1b[K> draft");
            tracker.observe_output(start + Duration::from_millis(1700), &term);
            tracker.recheck_delivery(start + Duration::from_secs(60), &term);
            assert_ne!(
                tracker.current_reported_state().state,
                InputState::Idle,
                "{input:?}"
            );
        }
    }

    #[test]
    fn a_recheck_keeps_only_a_notice_under_the_input_cursor() {
        let echo = format!("\x1b[5;1H\x1b[K> {NOTICE}");
        let wrapped =
            "\x1b[5;1H\x1b[K> [inbox] 1 new messages — run\r\n\x1b[K  `runner msg read` to view.";
        for (case, cols, echo, after, unsent) in [
            ("in the composer", 80, echo.as_str(), echo.clone(), true),
            ("wrapped", 40, wrapped, wrapped.to_owned(), true),
            (
                "history above the live prompt",
                80,
                echo.as_str(),
                format!("{echo}\x1b[6;1H\x1b[K⏺ Read 1 message\x1b[7;1H\x1b[K> \x1b[7;1H"),
                false,
            ),
            (
                "cursor on output below the notice",
                80,
                echo.as_str(),
                format!("{echo}\x1b[6;1H\x1b[K⏺ Read 1 message\x1b[7;1H\x1b[K> \x1b[6;5H"),
                false,
            ),
            (
                "cursor parked below the live prompt",
                80,
                echo.as_str(),
                format!("{echo}\x1b[6;1H\x1b[K⏺ Read 1 message\x1b[7;1H\x1b[K> \x1b[8;1H"),
                false,
            ),
            (
                "composer lost",
                80,
                echo.as_str(),
                "\x1b[2J\x1b[5;1H⏺ Replied\x1b[7;1H> \x1b[8;1H".to_owned(),
                false,
            ),
        ] {
            let (start, mut term, mut tracker) = submitted_notice(cols, echo);
            feed(&mut term, after.as_bytes());
            tracker.observe_output(start + Duration::from_millis(100), &term);
            assert_eq!(
                tracker.current_reported_state().state,
                InputState::Unsent,
                "{case}"
            );
            tracker.recheck_delivery(start + Duration::from_secs(60), &term);
            assert_eq!(
                tracker.current_reported_state().state,
                if unsent {
                    InputState::Unsent
                } else {
                    InputState::Idle
                },
                "{case}"
            );
            assert_eq!(tracker.unsent_notice(&term), unsent, "{case}");
        }
    }

    const LINKED: &str = "[runner] Mission m1 is now linked to this chat.";

    /// #1225: the router's line is submitted, and Claude Code redraws it as a transcript row at
    /// the composer's row while the turn runs, with the input cursor parked below the frame.
    fn linked_line_in_transcript() -> (
        Instant,
        Term<alacritty_terminal::event::VoidListener>,
        InputTracker,
    ) {
        let start = Instant::now();
        let mut term = new_term(60, 10);
        feed(
            &mut term,
            "\x1b[1;1H✻ Welcome\x1b[3;1H❯ \x1b[3;3H".as_bytes(),
        );
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.track_delivery_input(LINKED, start, &term);
        feed(&mut term, format!("\x1b[3;1H\x1b[K❯ {LINKED}").as_bytes());
        tracker.observe_output(start + Duration::from_millis(1), &term);
        tracker.track_delivery_submit(start + Duration::from_millis(80), &term);
        feed(
            &mut term,
            format!("\x1b[3;1H\x1b[K❯ {LINKED}\x1b[5;1H✻ Contemplating…\x1b[7;1H❯ \x1b[10;1H")
                .as_bytes(),
        );
        tracker.observe_output(start + Duration::from_millis(89), &term);
        assert_eq!(tracker.current_reported_state().state, InputState::Unsent);
        (start, term, tracker)
    }

    /// The turn ends and the transcript is redrawn: the tracked row now holds an earlier prompt,
    /// and the input cursor rests at the empty composer.
    const TURN_END: &str = "\x1b[2J\x1b[1;1H⏺ Started.\x1b[3;1H❯ start the mission\x1b[4;1H⏺ Mission started.\x1b[5;1H❯ [runner] Mission m1 is now linked to this chat.\x1b[6;1H⏺ Linked.\x1b[8;1H❯ \x1b[8;3H";

    #[test]
    fn an_ended_composition_does_not_hold_a_submitted_line_as_unsent() {
        let (start, mut term, mut tracker) = linked_line_in_transcript();
        let revision = tracker.reset_guard();
        tracker.observe_input(
            &InputEvent::Composing { composing: false },
            start + Duration::from_millis(300),
            &term,
        );
        assert_eq!(tracker.reset_guard(), revision, "no composition to end");
        assert_eq!(
            tracker
                .confirm_delivery(revision, start + Duration::from_millis(1580))
                .map(|observation| observation.state),
            Some(InputState::Idle),
            "the router saw the turn start"
        );
        feed(&mut term, TURN_END.as_bytes());
        tracker.observe_output(start + Duration::from_secs(14), &term);
        assert_eq!(tracker.current_reported_state().state, InputState::Idle);

        // A composition that did start still counts as input.
        tracker.observe_input(&InputEvent::Composing { composing: true }, start, &term);
        let composing = tracker.reset_guard();
        assert_ne!(composing, revision);
        tracker.observe_input(&InputEvent::Composing { composing: false }, start, &term);
        assert_ne!(tracker.reset_guard(), composing);
    }

    #[test]
    fn a_key_during_the_turn_cannot_make_the_transcript_a_draft() {
        let (start, mut term, mut tracker) = linked_line_in_transcript();
        let revision = tracker.reset_guard();
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Navigate,
            },
            start + Duration::from_millis(300),
            &term,
        );
        assert!(tracker
            .confirm_delivery(revision, start + Duration::from_millis(1580))
            .is_none());
        feed(&mut term, TURN_END.as_bytes());
        tracker.observe_output(start + Duration::from_secs(14), &term);
        assert_eq!(tracker.current_reported_state().state, InputState::Idle);
        assert!(!tracker.unsent_notice(&term));
    }

    #[test]
    fn a_recheck_drops_a_draft_the_empty_editor_does_not_show() {
        let (start, mut term, mut tracker) = linked_line_in_transcript();
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Navigate,
            },
            start + Duration::from_millis(300),
            &term,
        );
        // The turn-end frame arrives before Claude Code returns the cursor to its composer.
        feed(&mut term, TURN_END.as_bytes());
        feed(&mut term, b"\x1b[10;1H");
        tracker.observe_output(start + Duration::from_secs(14), &term);
        assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
        tracker.recheck_delivery(start + Duration::from_secs(15), &term);
        assert_eq!(
            tracker.current_reported_state().state,
            InputState::Drafting,
            "no editor under the cursor yet"
        );
        feed(&mut term, b"\x1b[8;3H");
        tracker.recheck_delivery(start + Duration::from_secs(16), &term);
        assert_eq!(tracker.current_reported_state().state, InputState::Idle);
    }

    fn typed_before_echo(pasted: bool) -> InputEvent {
        if pasted {
            InputEvent::Paste {
                text: "draft".into(),
            }
        } else {
            InputEvent::Key {
                kind: InputKind::Content {
                    text: "draft".into(),
                },
            }
        }
    }

    #[test]
    fn a_recheck_keeps_text_typed_before_its_echo() {
        for pasted in [false, true] {
            let (start, mut term, mut tracker) = linked_line_in_transcript();
            tracker.observe_input(
                &InputEvent::Key {
                    kind: InputKind::Navigate,
                },
                start + Duration::from_millis(300),
                &term,
            );
            feed(&mut term, TURN_END.as_bytes());
            feed(&mut term, b"\x1b[10;1H");
            tracker.observe_output(start + Duration::from_secs(14), &term);
            feed(&mut term, b"\x1b[8;3H");
            let typed = start + Duration::from_secs(15);
            tracker.observe_input(&typed_before_echo(pasted), typed, &term);
            tracker.recheck_delivery(typed + Duration::from_millis(10), &term);
            assert_eq!(
                tracker.current_reported_state().state,
                InputState::Drafting,
                "pasted={pasted}: the echo is still on its way"
            );
            feed(&mut term, "\x1b[8;1H\x1b[K❯ draft".as_bytes());
            tracker.observe_output(typed + Duration::from_millis(30), &term);
            tracker.recheck_delivery(typed + Duration::from_secs(5), &term);
            assert_eq!(
                tracker.current_reported_state().state,
                InputState::Drafting,
                "pasted={pasted}"
            );
        }
    }

    #[test]
    fn text_typed_before_its_echo_is_not_lost_to_a_frame_with_an_empty_editor() {
        // The frame either keeps the transcript row at the tracked row or ends the turn.
        for (changed_row, frame, echo) in [
            (false, "\x1b[7;3H", "\x1b[7;1H\x1b[K❯ draft"),
            (true, TURN_END, "\x1b[8;1H\x1b[K❯ draft"),
        ] {
            for pasted in [false, true] {
                let (start, mut term, mut tracker) = linked_line_in_transcript();
                tracker.observe_input(
                    &InputEvent::Key {
                        kind: InputKind::Navigate,
                    },
                    start + Duration::from_millis(300),
                    &term,
                );
                let typed = start + Duration::from_secs(14);
                tracker.observe_input(&typed_before_echo(pasted), typed, &term);
                feed(&mut term, frame.as_bytes());
                tracker.observe_output(typed + Duration::from_millis(10), &term);
                feed(&mut term, echo.as_bytes());
                tracker.observe_output(typed + Duration::from_millis(30), &term);
                tracker.recheck_delivery(typed + Duration::from_secs(5), &term);
                assert_ne!(
                    tracker.current_reported_state().state,
                    InputState::Idle,
                    "changed_row={changed_row} pasted={pasted}"
                );
            }
        }
    }

    #[test]
    fn the_tracker_remembers_holding_a_line_it_let_go_of_until_the_next_one() {
        let start = Instant::now();
        let mut term = new_term(60, 10);
        feed(&mut term, "\x1b[3;1H❯ \x1b[3;3H".as_bytes());
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.track_delivery_input(LINKED, start, &term);
        feed(&mut term, format!("\x1b[3;1H\x1b[K❯ {LINKED}").as_bytes());
        tracker.observe_output(start + Duration::from_millis(1), &term);
        assert!(!tracker.delivery_held(), "not submitted yet");
        tracker.track_delivery_submit(start + Duration::from_millis(80), &term);
        // The first frame after Enter already shows the empty composer below the transcript row.
        feed(
            &mut term,
            format!("\x1b[3;1H\x1b[K❯ {LINKED}\x1b[5;1H❯ \x1b[5;3H").as_bytes(),
        );
        tracker.observe_output(start + Duration::from_millis(89), &term);
        assert_eq!(tracker.current_reported_state().state, InputState::Idle);
        assert!(tracker.delivery_held());

        // A line whose echo was never located is not one the tracker held.
        tracker.track_delivery_input("[inbox] task", start + Duration::from_secs(5), &term);
        assert!(!tracker.delivery_held());
        tracker.track_delivery_submit(start + Duration::from_secs(5), &term);
        tracker.observe_output(start + Duration::from_secs(6), &term);
        assert!(!tracker.delivery_held());
    }

    #[test]
    fn a_recheck_keeps_a_real_draft_before_and_after_a_turn_end() {
        let start = Instant::now();
        let mut term = new_term(60, 10);
        feed(&mut term, "\x1b[1;1H⏺ Done.\x1b[3;1H❯ \x1b[3;3H".as_bytes());
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        tracker.observe_input(
            &InputEvent::Key {
                kind: InputKind::Content {
                    text: "half a thought".into(),
                },
            },
            start,
            &term,
        );
        feed(&mut term, "\x1b[3;1H\x1b[K❯ half a thought".as_bytes());
        tracker.observe_output(start + Duration::from_millis(5), &term);
        tracker.recheck_delivery(start + Duration::from_secs(5), &term);
        assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
        // A turn end redraws the transcript above the draft and returns the cursor to it.
        feed(
            &mut term,
            "\x1b[1;1H⏺ Done again.\x1b[K\x1b[3;1H\x1b[K❯ half a thought\x1b[3;17H".as_bytes(),
        );
        tracker.observe_output(start + Duration::from_secs(10), &term);
        tracker.recheck_delivery(start + Duration::from_secs(11), &term);
        assert_eq!(tracker.current_reported_state().state, InputState::Drafting);
    }

    #[test]
    fn delivery_typed_over_person_text_keeps_it_a_draft() {
        // The trailing space defeats the echo match, so that person text is still probing.
        for typed in ["draft", "draft "] {
            let start = Instant::now();
            let mut term = new_term(80, 4);
            feed(&mut term, "\x1b[2;1H❯ Try a task".as_bytes());
            let mut tracker = InputTracker::new(start);
            tracker.initial_observation(start);
            tracker.observe_input(
                &InputEvent::Key {
                    kind: InputKind::Content { text: typed.into() },
                },
                start,
                &term,
            );
            feed(&mut term, format!("\r\x1b[K❯ {typed}").as_bytes());
            tracker.observe_output(start + Duration::from_millis(5), &term);
            tracker.track_delivery_input("[inbox] task", start + Duration::from_millis(6), &term);
            feed(
                &mut term,
                format!("\r\x1b[K❯ {typed}[inbox] task").as_bytes(),
            );
            tracker.observe_output(start + Duration::from_millis(9), &term);
            assert_eq!(
                tracker.current_reported_state().state,
                InputState::Drafting,
                "{typed:?}"
            );
        }
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
    fn submit_while_probing_holds_a_nonempty_first_echo() {
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
    fn every_preedit_update_reports_activity_even_when_the_state_is_unchanged() {
        let start = Instant::now();
        let term = new_term(30, 3);
        let mut tracker = InputTracker::new(start);
        tracker.initial_observation(start);
        for seconds in [600, 601] {
            let observation = tracker
                .observe_input(
                    &InputEvent::Composing { composing: true },
                    start + Duration::from_secs(seconds),
                    &term,
                )
                .unwrap();
            assert!(observation.composing);
            assert_eq!(observation.state, InputState::Drafting);
        }
    }

    #[test]
    fn person_keys_keep_notice_ownership_until_the_composer_text_changes() {
        for kind in [
            InputKind::Navigate,
            InputKind::Edit,
            InputKind::Cancel,
            InputKind::Submit,
        ] {
            let start = Instant::now();
            let mut term = new_term(80, 4);
            feed(&mut term, "\x1b[2;1H› Ask Codex".as_bytes());
            let mut tracker = InputTracker::new(start);
            tracker.initial_observation(start);
            tracker.track_delivery_input("[inbox] task", start, &term);
            feed(&mut term, "\r\x1b[K› [inbox] task".as_bytes());
            tracker.observe_output(start, &term);
            tracker.track_delivery_submit(start, &term);
            tracker.observe_output(start, &term);
            let revision = tracker.reset_guard();
            tracker.observe_input(&InputEvent::Key { kind }, start, &term);
            feed(&mut term, b"\x1b[D");
            tracker.observe_output(start + Duration::from_secs(11 * 60), &term);
            assert_eq!(tracker.current_reported_state().state, InputState::Unsent);
            assert!(tracker.unsent_notice(&term));
            assert_ne!(tracker.reset_guard(), revision);
            assert!(tracker.reset_if_unchanged(revision, start).is_none());
            feed(&mut term, "\r\x1b[K› recalled prompt".as_bytes());
            assert_eq!(
                tracker
                    .observe_output(start + Duration::from_secs(11 * 60 + 1), &term)
                    .unwrap()
                    .state,
                InputState::Drafting
            );
            assert!(!tracker.unsent_notice(&term));
        }
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
            std::hint::black_box(relocate_row(&term, "missing-prefix ", 30, None));
        }
        let relocation_us = measured_at.elapsed().as_secs_f64() * 1_000_000. / ITERATIONS as f64;

        let measured_at = Instant::now();
        for _ in 0..ITERATIONS {
            std::hint::black_box(relocate_row(&term, "missing-prefix ", 30, Some('─')));
        }
        let framed_us = measured_at.elapsed().as_secs_f64() * 1_000_000. / ITERATIONS as f64;
        eprintln!(
            "input tracker µs/chunk: idle={idle_us:.3} drafting={drafting_us:.3} relocation={relocation_us:.3} framed={framed_us:.3}"
        );
    }
}
