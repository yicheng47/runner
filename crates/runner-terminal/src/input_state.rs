use std::time::{Duration, Instant};

pub use runner_core::protocol::terminal::InputEvent;
use unicode_width::UnicodeWidthStr;

use crate::mappings::InputKind;
use crate::palette::Color;
use crate::screen::{CellView, ScreenView};
use crate::types::{CellFlags, Column, Line};

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
    draft_style: Option<CellStyle>,
    frame_border: Option<char>,
}

#[derive(Clone, Debug)]
struct Probe {
    typed: String,
    rows_before: Vec<String>,
    styles_before: Vec<Vec<CellStyle>>,
    cursor: Option<(usize, usize)>,
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
    flags: CellFlags,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RecheckHoldDiagnostic {
    composing: bool,
    visible: bool,
    submitted: bool,
    content_pending: bool,
    tracked_row: Option<usize>,
    cursor: Option<(usize, usize)>,
    cursor_prefix_col: bool,
    cursor_prefix_match: bool,
    cursor_empty: bool,
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
    /// Last learned placeholder cell style, retained across Idle transitions so a blank-composer
    /// turn still recognizes a rotated placeholder or prompt suggestion (#1409).
    last_placeholder_style: Option<CellStyle>,
    last_recheck_diag: Option<RecheckHoldDiagnostic>,
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
            last_placeholder_style: None,
            last_recheck_diag: None,
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

    pub fn observe_input(
        &mut self,
        event: &InputEvent,
        now: Instant,
        screen: &ScreenView<'_>,
    ) -> Option<InputObservation> {
        // The app ends a composition before every typed key and whenever the IME unmarks, as on
        // a focus change; ending one that never started changes nothing a delivery holds (#1225).
        if !matches!(event, InputEvent::Composing { composing: false } if !self.composing) {
            self.input_revision = self.input_revision.wrapping_add(1);
        }
        self.expire_probe(now);
        match event {
            InputEvent::Composing { composing } => self.composing = *composing,
            InputEvent::Paste { text } => self.observe_content(text, true, now, screen),
            InputEvent::Key { kind } => match kind {
                InputKind::Content { text } => self.observe_content(text, false, now, screen),
                InputKind::Submit if !self.composing => {
                    self.submit(now, screen, SubmitOrigin::Native)
                }
                InputKind::Edit | InputKind::Cancel => {
                    if let TrackerState::Drafting(composer) = &mut self.state {
                        if self.delivery_revision.is_none() {
                            composer.submitted_form = None;
                        }
                        // A redraw such as a Codex rollback can leave a transcript row with the
                        // prompt prefix at the tracked row; edits go to the live editor.
                        if let Some(row) = cursor_prompt_row(screen, &composer.prefix) {
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

    pub fn track_delivery_input(&mut self, text: &str, now: Instant, screen: &ScreenView<'_>) {
        self.expire_probe(now);
        self.delivery_held = false;
        if matches!(self.state, TrackerState::Idle) {
            self.delivery_revision = Some(self.input_revision);
            self.delivery_text = text.to_owned();
        } else if self.delivery_owned() {
            self.delivery_revision = Some(self.input_revision);
            self.delivery_text.push_str(text);
        }
        self.observe_content(text, true, now, screen);
    }

    /// The composer row still reads as it did at the router's Enter.
    pub fn unsent_notice(&self, screen: &ScreenView<'_>) -> bool {
        self.delivery_owned()
            && matches!(
                &self.state,
                TrackerState::Drafting(composer) | TrackerState::Submitted(composer)
                    if composer.row < screen.rows()
                        && composer.submitted_form.as_deref()
                            == Some(screen.row_to_string(Line(composer.row as i32)).as_str())
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

    pub fn track_delivery_submit(&mut self, now: Instant, screen: &ScreenView<'_>) {
        self.submit(now, screen, SubmitOrigin::Delivery);
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
    pub fn recheck_delivery(
        &mut self,
        now: Instant,
        screen: &ScreenView<'_>,
    ) -> Option<InputObservation> {
        let (live, editor_empty) = match &self.state {
            TrackerState::Drafting(composer) | TrackerState::Submitted(composer) => (
                composer.visible
                    && self.unsent_notice(screen)
                    && cursor_in_notice(screen, composer, &self.delivery_text),
                cursor_at_empty_composer(screen, composer),
            ),
            _ => (false, false),
        };
        if (self.delivery_owned() && self.delivery_revision == Some(self.input_revision) && !live)
            || (editor_empty && !self.composing && !self.content_pending(now))
        {
            self.state = TrackerState::Idle;
            self.delivery_revision = None;
        }
        let observation = self.emit_if_changed(now, "re-check");
        if observation.is_none() {
            self.log_recheck_draft_hold(now, screen);
        }
        observation
    }

    fn log_recheck_draft_hold(&mut self, now: Instant, screen: &ScreenView<'_>) {
        let Some(diag) = self.record_recheck_draft_hold(now, screen) else {
            return;
        };
        log::info!(
            "input tracker recheck kept drafting session={} composing={} visible={} submitted={} content_pending={} tracked_row={:?} cursor={:?} cursor_prefix_col={} cursor_prefix_match={} cursor_empty={}",
            self.session_id,
            diag.composing,
            diag.visible,
            diag.submitted,
            diag.content_pending,
            diag.tracked_row,
            diag.cursor,
            diag.cursor_prefix_col,
            diag.cursor_prefix_match,
            diag.cursor_empty,
        );
    }

    fn record_recheck_draft_hold(
        &mut self,
        now: Instant,
        screen: &ScreenView<'_>,
    ) -> Option<RecheckHoldDiagnostic> {
        if self.current_reported_state().state != InputState::Drafting {
            self.last_recheck_diag = None;
            return None;
        }
        let cursor = cursor_position(screen);
        let (visible, submitted, tracked_row, cursor_prefix_col, cursor_prefix_match, cursor_empty) =
            match &self.state {
                TrackerState::Drafting(composer) | TrackerState::Submitted(composer) => {
                    let prefix_col = cursor.is_some_and(|(_, col)| col == composer.prefix.width());
                    let (prefix_match, empty) = cursor.map_or((false, false), |(row, _)| {
                        let current = screen.row_to_string(Line(row as i32));
                        let matched = row_matches_prefix(&current, &composer.prefix);
                        let empty = matched && composer_is_empty(screen, row, &current, composer);
                        (matched, empty)
                    });
                    (
                        composer.visible,
                        composer.submitted_form.is_some(),
                        Some(composer.row),
                        prefix_col,
                        prefix_match,
                        empty,
                    )
                }
                _ => (true, false, None, false, false, false),
            };
        let diag = RecheckHoldDiagnostic {
            composing: self.composing,
            visible,
            submitted,
            content_pending: self.content_pending(now),
            tracked_row,
            cursor,
            cursor_prefix_col,
            cursor_prefix_match,
            cursor_empty,
        };
        if self.last_recheck_diag == Some(diag) {
            return None;
        }
        self.last_recheck_diag = Some(diag);
        Some(diag)
    }

    pub fn observe_output(
        &mut self,
        now: Instant,
        screen: &ScreenView<'_>,
    ) -> Option<InputObservation> {
        let state = std::mem::replace(&mut self.state, TrackerState::Idle);
        self.state = match state {
            TrackerState::Idle => TrackerState::Idle,
            TrackerState::Probing(probe) => {
                if now.duration_since(probe.started) > ECHO_WINDOW {
                    TrackerState::Idle
                } else {
                    let rows_after = visible_rows(screen);
                    match locate_probe_echo(&probe, &rows_after, screen) {
                        Some(matched) => {
                            let placeholder_style =
                                probe_placeholder_style(&probe, &matched, screen);
                            let draft_style =
                                row_content_style(screen, matched.row, &matched.prefix);
                            if placeholder_style.is_some() {
                                self.last_placeholder_style = placeholder_style;
                            } else if draft_style.is_some()
                                && self.last_placeholder_style == draft_style
                            {
                                self.last_placeholder_style = None;
                            }
                            let cached_placeholder = self
                                .last_placeholder_style
                                .filter(|style| Some(*style) != draft_style);
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
                                placeholder_style: placeholder_style.or(cached_placeholder),
                                draft_style,
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
                let rows_after = visible_rows(screen);
                if let Some(matched) = locate_probe_echo(&probe, &rows_after, screen) {
                    let before = &probe.rows_before[matched.before_row];
                    if composer_text_is_empty(before, &matched.prefix, &composer.empty_forms)
                        && !composer.empty_forms.contains(before)
                    {
                        composer.empty_forms.push(before.clone());
                    }
                    if let Some(style) = row_content_style(screen, matched.row, &matched.prefix) {
                        composer.draft_style = Some(style);
                        if self.last_placeholder_style == Some(style) {
                            self.last_placeholder_style = None;
                        }
                        if composer.placeholder_style == Some(style) {
                            composer.placeholder_style = None;
                        }
                    }
                    if let Some(style) = probe_placeholder_style(&probe, &matched, screen) {
                        composer.placeholder_style = Some(style);
                        self.last_placeholder_style = Some(style);
                    } else if composer.placeholder_style.is_none() {
                        composer.placeholder_style = self
                            .last_placeholder_style
                            .filter(|style| Some(*style) != composer.draft_style);
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
                    self.observe_composer(now, screen, composer, true)
                }
            }
            TrackerState::Drafting(composer) => self.observe_composer(now, screen, composer, false),
            TrackerState::Submitted(composer) => self.observe_composer(now, screen, composer, true),
        };
        self.delivery_held |= self.delivery_owned();
        // Submitted text can remain in history with the same prefix as the live composer; text
        // typed since the Enter may not have reached that composer yet.
        if !self.content_pending(now)
            && matches!(
                &self.state,
                TrackerState::Submitted(composer) | TrackerState::Drafting(composer)
                    if composer.submitted_form.is_some()
                        && cursor_at_empty_composer(screen, composer)
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

    fn observe_content(&mut self, text: &str, pasted: bool, now: Instant, screen: &ScreenView<'_>) {
        if text.is_empty() {
            return;
        }
        self.content_at = Some(now);
        let state = std::mem::replace(&mut self.state, TrackerState::Idle);
        self.state = match state {
            TrackerState::Idle => TrackerState::Probing(new_probe(text, pasted, now, screen)),
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
                probe: new_probe(text, pasted, now, screen),
                composer,
            },
        };
    }

    fn submit(&mut self, _now: Instant, screen: &ScreenView<'_>, origin: SubmitOrigin) {
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
                if composer.row < screen.rows() {
                    let current = screen.row_to_string(Line(composer.row as i32));
                    if row_matches_nonempty_prefix(&current, &composer.prefix) {
                        composer.last_form = current;
                    }
                }
                composer.submitted_form = Some(composer.last_form.clone());
                self.state = TrackerState::Submitted(composer);
            }
            state => self.state = state,
        }
    }

    fn observe_composer(
        &self,
        now: Instant,
        screen: &ScreenView<'_>,
        mut composer: Composer,
        submitted: bool,
    ) -> TrackerState {
        // Submitted prompts or scrolled transcript rows can remain at the old row. A unique
        // learned frame always identifies the current editor; without a frame, follow a non-empty
        // prompt under the live cursor only after the tracked row no longer holds the intact draft.
        if screen.raw_cursor_point().line != Line(composer.row as i32) {
            let tracked_row_intact = composer.row < screen.rows() && {
                let tracked = screen.row_to_string(Line(composer.row as i32));
                tracked == composer.last_form
                    || composer.submitted_form.as_deref() == Some(tracked.as_str())
            };
            if let Some(row) = composer
                .frame_border
                .and_then(|border| framed_composer_row(screen, &composer.prefix, border))
                .or_else(|| {
                    if tracked_row_intact {
                        return None;
                    }
                    let row = cursor_prompt_row(screen, &composer.prefix)?;
                    let text = screen.row_to_string(Line(row as i32));
                    let matches_submitted = composer
                        .submitted_form
                        .as_deref()
                        .is_none_or(|submitted| text == submitted);
                    (matches_submitted && !composer_is_empty(screen, row, &text, &composer))
                        .then_some(row)
                })
            {
                composer.row = row;
            }
        }
        let mut current = if composer.row < screen.rows() {
            screen.row_to_string(Line(composer.row as i32))
        } else if let Some(row) = relocate_row(
            screen,
            &composer.prefix,
            composer.row.min(screen.rows().saturating_sub(1)),
            composer.frame_border,
        ) {
            composer.row = row;
            composer.visible = true;
            screen.row_to_string(Line(row as i32))
        } else {
            composer.visible = false;
            return if submitted {
                TrackerState::Submitted(composer)
            } else {
                TrackerState::Drafting(composer)
            };
        };
        if !row_matches_prefix(&current, &composer.prefix) {
            if let Some(row) = relocate_row(
                screen,
                &composer.prefix,
                composer.row,
                composer.frame_border,
            ) {
                composer.row = row;
                current = screen.row_to_string(Line(row as i32));
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

        if composer_is_empty(screen, composer.row, &current, &composer) {
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
                && (self.content_pending(now) || !cursor_at_empty_composer(screen, &composer))
            {
                composer.submitted_form = None;
                if !row_has_placeholder_style(screen, composer.row, &composer) {
                    composer.last_form = current;
                }
                return TrackerState::Drafting(composer);
            }
            return TrackerState::Idle;
        }
        // Preserve the confirmed draft text while a placeholder-styled row awaits cursor
        // confirmation, so returning the cursor to it still sees `current != last_form`.
        if !row_has_placeholder_style(screen, composer.row, &composer) {
            composer.last_form = current;
        }
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
        if current.state != InputState::Drafting {
            self.last_recheck_diag = None;
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

pub(crate) fn visible_rows(screen: &ScreenView<'_>) -> Vec<String> {
    screen.visible_lines()
}

fn visible_row_styles(screen: &ScreenView<'_>) -> Vec<Vec<CellStyle>> {
    (0..screen.rows())
        .map(|row| {
            let line = Line(row as i32);
            (0..screen.cols())
                .filter_map(|column| {
                    let cell = screen.row_cell(line, Column(column))?;
                    (!cell.flags().intersects(
                        CellFlags::WIDE_CHAR_SPACER | CellFlags::LEADING_WIDE_CHAR_SPACER,
                    ))
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

fn cursor_position(screen: &ScreenView<'_>) -> Option<(usize, usize)> {
    let cursor = screen.raw_cursor_point();
    Some((usize::try_from(cursor.line.0).ok()?, cursor.column.0))
}

fn new_probe(text: &str, pasted: bool, now: Instant, screen: &ScreenView<'_>) -> Probe {
    Probe {
        typed: probe_text(text),
        rows_before: visible_rows(screen),
        styles_before: visible_row_styles(screen),
        cursor: cursor_position(screen),
        started: now,
        weak_match: pasted && weak_paste_match(text),
        submitted: None,
    }
}

fn cell_style(cell: CellView<'_>) -> CellStyle {
    let mut flags = cell.flags();
    flags.remove(
        CellFlags::WIDE_CHAR
            | CellFlags::WIDE_CHAR_SPACER
            | CellFlags::LEADING_WIDE_CHAR_SPACER
            | CellFlags::WRAPLINE,
    );
    CellStyle {
        fg: cell.fg(),
        bg: cell.bg(),
        flags,
    }
}

/// A fake inverse cursor on cell 0 can mask a placeholder's or draft's actual style on that cell
/// alone, so prefer the second non-whitespace cell when cell 0 is inverted.
fn effective_content_style(styles: &[CellStyle]) -> Option<CellStyle> {
    let &first = styles.first()?;
    if first.flags.contains(CellFlags::INVERSE) {
        styles
            .get(1)
            .copied()
            .or_else(|| first.flags.contains(CellFlags::DIM).then_some(first))
    } else {
        Some(first)
    }
}

fn snapshot_content_style(text: &str, styles: &[CellStyle], prefix: &str) -> Option<CellStyle> {
    let content: Vec<CellStyle> = text
        .chars()
        .zip(styles)
        .skip(prefix.chars().count())
        .filter_map(|(character, style)| (!character.is_whitespace()).then_some(*style))
        .take(2)
        .collect();
    effective_content_style(&content)
}

fn row_content_styles(screen: &ScreenView<'_>, row: usize, prefix: &str) -> Vec<CellStyle> {
    let line = Line(row as i32);
    let mut prefix_characters = prefix.chars().count();
    let mut styles = Vec::new();
    for column in 0..screen.cols() {
        let Some(cell) = screen.row_cell(line, Column(column)) else {
            continue;
        };
        if cell
            .flags()
            .intersects(CellFlags::WIDE_CHAR_SPACER | CellFlags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }
        if prefix_characters > 0 {
            prefix_characters =
                prefix_characters.saturating_sub(1 + cell.zerowidth().map_or(0, <[_]>::len));
            continue;
        }
        if !cell.c().is_whitespace() {
            styles.push(cell_style(cell));
            if styles.len() >= 2 {
                break;
            }
        }
    }
    styles
}

fn row_content_style(screen: &ScreenView<'_>, row: usize, prefix: &str) -> Option<CellStyle> {
    effective_content_style(&row_content_styles(screen, row, prefix))
}

fn row_has_placeholder_style(screen: &ScreenView<'_>, row: usize, composer: &Composer) -> bool {
    let Some(style) = row_content_style(screen, row, &composer.prefix) else {
        return false;
    };
    if composer.draft_style == Some(style) {
        return false;
    }
    if composer.placeholder_style == Some(style) {
        return true;
    }
    style.flags.contains(CellFlags::DIM)
        && !composer
            .draft_style
            .is_some_and(|draft| draft.flags.contains(CellFlags::DIM))
}

fn is_placeholder_style(screen: &ScreenView<'_>, row: usize, composer: &Composer) -> bool {
    screen.raw_cursor_point().line == Line(row as i32)
        && row_has_placeholder_style(screen, row, composer)
}

fn probe_placeholder_style(
    probe: &Probe,
    matched: &EchoMatch,
    screen: &ScreenView<'_>,
) -> Option<CellStyle> {
    let placeholder = snapshot_content_style(
        &probe.rows_before[matched.before_row],
        &probe.styles_before[matched.before_row],
        &matched.prefix,
    )?;
    let draft = row_content_style(screen, matched.row, &matched.prefix)?;
    (placeholder != draft).then_some(placeholder)
}

fn locate_probe_echo(
    probe: &Probe,
    after: &[String],
    screen: &ScreenView<'_>,
) -> Option<EchoMatch> {
    let submitted_cursor = probe.submitted.and_then(|_| cursor_position(screen));
    locate_echo_with_cursors(
        &probe.rows_before,
        after,
        &probe.typed,
        probe.weak_match,
        screen.cols(),
        probe.cursor,
        submitted_cursor,
    )
}

#[cfg(test)]
fn locate_echo(
    before: &[String],
    after: &[String],
    typed: &str,
    weak_match: bool,
    columns: usize,
) -> Option<EchoMatch> {
    locate_echo_with_cursors(before, after, typed, weak_match, columns, None, None)
}

fn count_row(rows: &[String], target: &str) -> usize {
    rows.iter().filter(|row| row.as_str() == target).count()
}

fn looks_like_collapsed_paste(remainder: &str) -> bool {
    let trimmed = remainder.trim();
    trimmed.starts_with('[')
        && trimmed.ends_with(']')
        && trimmed.to_ascii_lowercase().contains("pasted")
}

fn submitted_transcript_matches(
    text: &str,
    prefix: &str,
    typed: &str,
    kind: EchoMatchKind,
    weak_match: bool,
    columns: usize,
) -> bool {
    if kind != EchoMatchKind::Weak {
        return true;
    }
    let remainder = text[prefix.len()..].trim_end();
    if weak_match && looks_like_collapsed_paste(remainder) {
        return true;
    }
    let stem = remainder.strip_suffix('…').unwrap_or(remainder).trim_end();
    let max_cell_fit = columns.saturating_sub(prefix.width() + 1).max(1);
    let min_wrapped_width = typed
        .split_whitespace()
        .next()
        .map_or(1, UnicodeWidthStr::width)
        .min(max_cell_fit);
    stem.width() >= min_wrapped_width && typed.starts_with(stem)
}

fn locate_echo_with_cursors(
    before: &[String],
    after: &[String],
    typed: &str,
    weak_match: bool,
    columns: usize,
    cursor_before: Option<(usize, usize)>,
    submitted_cursor: Option<(usize, usize)>,
) -> Option<EchoMatch> {
    let prompt_at_cursor_before = |prefix: &str| {
        let (line, col) = cursor_before?;
        let text = before.get(line)?;
        (row_matches_prefix(text, prefix) && col >= prefix.width()).then_some(line)
    };
    after
        .iter()
        .enumerate()
        .filter(|(row, text)| before.get(*row) != Some(*text))
        .flat_map(|(row, text)| {
            before
                .iter()
                .enumerate()
                .filter_map(move |(before_row, before_text)| {
                    let before_changed = after.get(before_row) != Some(before_text);
                    let (prefix, kind) =
                        match_prompt_prefix(before_text, text, typed, weak_match, columns)?;
                    let cursor_prompt = prompt_at_cursor_before(&prefix);
                    if cursor_prompt.is_some_and(|cursor_row| cursor_row != before_row) {
                        return None;
                    }
                    // A submit resolved in one frame leaves a pinned bottom composer back at its
                    // pre-input row while moving the submitted prompt into transcript history.
                    let cleared_at_cursor = !before_changed
                        && cursor_prompt == Some(before_row)
                        && submitted_cursor == Some((before_row, prefix.width()))
                        && count_row(after, text) > count_row(before, text)
                        && submitted_transcript_matches(
                            text, &prefix, typed, kind, weak_match, columns,
                        );
                    (before_changed || cleared_at_cursor).then_some(EchoMatch {
                        row,
                        before_row,
                        empty_before: before_text.trim_end() == prefix.trim_end(),
                        prefix,
                        kind,
                    })
                })
        })
        .min_by_key(|matched| {
            (
                matched.kind,
                prompt_at_cursor_before(&matched.prefix) != Some(matched.before_row),
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
            if !row_matches_prefix(before, prefix) || !looks_like_prompt_prefix(prefix) {
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

/// Matches a prompt row against a learned prefix, treating trailing whitespace variants (such as
/// Claude Code's NBSP `\u{a0}` in its live composer vs ASCII space in its transcript) as equivalent
/// and accepting trimmed empty prompt rows whose trailing whitespace `row_to_string` stripped.
fn row_matches_prefix(text: &str, prefix: &str) -> bool {
    row_matches_nonempty_prefix(text, prefix) || text.trim_end() == prefix.trim_end()
}

/// Like `row_matches_prefix`, but requires content after the prompt's trailing whitespace so an
/// unframed fallback scan without cursor confirmation cannot jump onto a bare prompt glyph in output.
fn row_matches_nonempty_prefix(text: &str, prefix: &str) -> bool {
    let stem = prefix.trim_end();
    let Some(rest) = text.strip_prefix(stem) else {
        return false;
    };
    let trailing_ws = prefix[stem.len()..].chars().count();
    let mut chars = rest.chars();
    (0..trailing_ws).all(|_| chars.next().is_some_and(char::is_whitespace))
        && !chars.as_str().trim().is_empty()
}

fn composer_text_is_empty(current: &str, prefix: &str, empty_forms: &[String]) -> bool {
    empty_forms.iter().any(|form| current == form) || current.trim_end() == prefix.trim_end()
}

fn composer_is_empty(
    screen: &ScreenView<'_>,
    row: usize,
    current: &str,
    composer: &Composer,
) -> bool {
    composer_text_is_empty(current, &composer.prefix, &composer.empty_forms)
        || (row_matches_prefix(current, &composer.prefix)
            && current != composer.last_form
            && is_placeholder_style(screen, row, composer))
}

fn cursor_at_empty_composer(screen: &ScreenView<'_>, composer: &Composer) -> bool {
    let cursor = screen.raw_cursor_point();
    if cursor.line.0 < 0 || cursor.column.0 != composer.prefix.width() {
        return false;
    }
    let row = cursor.line.0 as usize;
    let current = screen.row_to_string(cursor.line);
    row_matches_prefix(&current, &composer.prefix)
        && composer_is_empty(screen, row, &current, composer)
}

/// The cursor is on the composer row, or below it on rows that only continue the notice.
fn cursor_in_notice(screen: &ScreenView<'_>, composer: &Composer, notice: &str) -> bool {
    let Ok(cursor) = usize::try_from(screen.raw_cursor_point().line.0) else {
        return false;
    };
    cursor >= composer.row
        && (composer.row + 1..=cursor).all(|row| {
            let text = screen.row_to_string(Line(row as i32));
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

fn framed_composer_row(screen: &ScreenView<'_>, prefix: &str, border: char) -> Option<usize> {
    let rows = visible_rows(screen);
    let mut candidates = rows.iter().enumerate().filter_map(|(row, text)| {
        (row_matches_prefix(text, prefix) && composer_frame_border(&rows, row) == Some(border))
            .then_some(row)
    });
    let row = candidates.next()?;
    let cursor = screen.raw_cursor_point();
    (candidates.next().is_none()
        && cursor.line == Line(row as i32)
        && cursor.column.0 >= prefix.width())
    .then_some(row)
}

fn cursor_prompt_row(screen: &ScreenView<'_>, prefix: &str) -> Option<usize> {
    let cursor = screen.raw_cursor_point();
    let row = usize::try_from(cursor.line.0).ok()?;
    let text = screen.row_to_string(cursor.line);
    (row_matches_prefix(&text, prefix) && cursor.column.0 >= prefix.width()).then_some(row)
}

fn relocate_row(
    screen: &ScreenView<'_>,
    prefix: &str,
    old_row: usize,
    frame_border: Option<char>,
) -> Option<usize> {
    if let Some(border) = frame_border {
        // The frame can change shape after a resize or mode switch; the input
        // cursor still marks the live editor where a history row cannot.
        return framed_composer_row(screen, prefix, border)
            .or_else(|| cursor_prompt_row(screen, prefix));
    }
    cursor_prompt_row(screen, prefix).or_else(|| {
        (0..screen.rows())
            .filter(|row| {
                row_matches_nonempty_prefix(&screen.row_to_string(Line(*row as i32)), prefix)
            })
            .min_by_key(|row| row.abs_diff(old_row))
    })
}

fn probe_text(text: &str) -> String {
    text.lines().next().unwrap_or_default().to_owned()
}

fn weak_paste_match(text: &str) -> bool {
    text.contains(['\n', '\r']) || text.chars().count() > 40
}

#[cfg(test)]
mod tests;
