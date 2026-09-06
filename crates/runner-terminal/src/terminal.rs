use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions as _, Scroll};
use alacritty_terminal::index::{Boundary, Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle, Processor};
use anyhow::{Context as _, Result};
use regex::Regex;
use runner_backend::session::manager::{
    ExitEvent, OutputEvent, SessionEvents, SessionSpawnedEvent, SessionUpdatedEvent,
};
use runner_backend::AppCore;

use crate::palette;
use crate::{
    fixtures::FixtureRecorder,
    input_state::{InputEvent, InputTracker},
};

pub const XTERM_WORD_SEPARATORS: &str = " ()[]{}',\"`";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LinkTarget {
    Url(String),
    File {
        path: PathBuf,
        line: Option<u32>,
        column: Option<u32>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalLink {
    pub target: LinkTarget,
    pub start: Point,
    pub end: Point,
}

impl TerminalLink {
    pub fn contains(&self, point: Point) -> bool {
        if point.line < self.start.line || point.line > self.end.line {
            return false;
        }
        if self.start.line == self.end.line {
            return point.column >= self.start.column && point.column <= self.end.column;
        }
        (point.line != self.start.line || point.column >= self.start.column)
            && (point.line != self.end.line || point.column <= self.end.column)
    }
}

pub struct EventProxy {
    tx: Sender<Event>,
    waker: Arc<dyn Fn() + Send + Sync>,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        let _ = self.tx.send(event);
        (self.waker)();
    }
}

pub fn query_color<T>(
    term: &Term<T>,
    index: usize,
    base: &[alacritty_terminal::vte::ansi::Rgb; 256],
) -> alacritty_terminal::vte::ansi::Rgb {
    query_color_for(term, index, base, palette::RUNNER)
}

pub fn query_color_for<T>(
    term: &Term<T>,
    index: usize,
    base: &[alacritty_terminal::vte::ansi::Rgb; 256],
    theme: palette::TerminalPalette,
) -> alacritty_terminal::vte::ansi::Rgb {
    let stored = if index < alacritty_terminal::term::color::COUNT {
        term.colors()[index]
    } else {
        None
    };
    stored.unwrap_or_else(|| palette::resolve_index_for(index, base, theme))
}

#[derive(Default)]
struct SequenceState {
    last: u64,
    // No production reader after M6.11; kept as a cheaper mode-level signal should one appear.
    tui_ready_seq: u64,
    first_paint_seq: u64,
    last_output_at: Option<Instant>,
}

#[derive(Clone, Copy)]
struct PaletteState {
    theme: palette::TerminalPalette,
    base: [alacritty_terminal::vte::ansi::Rgb; 256],
}

impl PaletteState {
    fn new(theme: palette::TerminalPalette) -> Self {
        Self {
            theme,
            base: palette::base_palette_for(theme),
        }
    }
}

pub struct TerminalSession {
    pub term: Arc<FairMutex<Term<EventProxy>>>,
    core: AppCore,
    session_id: String,
    parser: Mutex<Processor>,
    sequence: Mutex<SequenceState>,
    size: Arc<Mutex<(u16, u16)>>,
    title: Arc<Mutex<String>>,
    palette: Arc<Mutex<PaletteState>>,
    waker: Arc<dyn Fn() + Send + Sync>,
    viewers: Arc<AtomicUsize>,
    user_input: UserInput,
    input_tracker: Mutex<InputTracker>,
    fixture_recorder: Option<FixtureRecorder>,
    link_cwd: std::sync::OnceLock<Option<PathBuf>>,
}

pub struct TerminalView {
    viewers: Arc<AtomicUsize>,
}

impl Drop for TerminalView {
    fn drop(&mut self) {
        self.viewers.fetch_sub(1, Ordering::Release);
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UserInputMode {
    #[default]
    Inline,
    Queued,
}

enum UserInput {
    Inline,
    Queued(Sender<Vec<u8>>),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TerminalScrollState {
    pub history_lines: usize,
    pub display_offset: usize,
    pub screen_lines: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TerminalOutputActivity {
    pub last_seq: u64,
    pub tui_ready_seq: u64,
    pub first_paint_seq: u64,
    pub last_output_at: Option<Instant>,
}

impl TerminalSession {
    pub fn attach(
        core: AppCore,
        session_id: String,
        cols: u16,
        rows: u16,
        waker: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Arc<Self>> {
        Self::attach_with_input_mode(core, session_id, cols, rows, waker, UserInputMode::Inline)
    }

    pub fn attach_with_input_mode(
        core: AppCore,
        session_id: String,
        cols: u16,
        rows: u16,
        waker: Arc<dyn Fn() + Send + Sync>,
        input_mode: UserInputMode,
    ) -> Result<Arc<Self>> {
        let (tx, rx) = mpsc::channel::<Event>();
        let user_input = match input_mode {
            UserInputMode::Inline => UserInput::Inline,
            UserInputMode::Queued => {
                let (input_tx, input_rx) = mpsc::channel::<Vec<u8>>();
                let input_core = core.clone();
                let input_session_id = session_id.clone();
                thread::Builder::new()
                    .name(format!("native-term-input-{session_id}"))
                    .spawn(move || {
                        while let Ok(bytes) = input_rx.recv() {
                            if let Err(error) = input_core.sessions.inject_direct_stdin(
                                &input_session_id,
                                &bytes,
                                &input_core.session_events(),
                            ) {
                                input_core.events.emit(
                                    "session/input-error",
                                    &serde_json::json!({
                                        "session_id": input_session_id,
                                        "message": error.to_string(),
                                    }),
                                );
                            }
                        }
                    })
                    .context("spawn terminal input thread")?;
                UserInput::Queued(input_tx)
            }
        };
        let proxy = EventProxy {
            tx,
            waker: Arc::clone(&waker),
        };
        let term = Arc::new(FairMutex::new(Term::new(
            Config::default(),
            &TermSize::new(cols as usize, rows as usize),
            proxy,
        )));
        let size = Arc::new(Mutex::new((cols, rows)));
        let title = Arc::new(Mutex::new(String::new()));
        let terminal_palette = Arc::new(Mutex::new(PaletteState::new(palette::RUNNER)));
        let viewers = Arc::new(AtomicUsize::new(0));
        let now = Instant::now();
        let mut input_tracker = InputTracker::new(now);
        let initial_observation = input_tracker.initial_observation(now);
        let fixture_recorder = match FixtureRecorder::from_env(&session_id, cols, rows) {
            Ok(recorder) => recorder,
            Err(error) => {
                log::error!("start input fixture recorder for {session_id} failed: {error:#}");
                None
            }
        };
        let session = Arc::new(Self {
            term: Arc::clone(&term),
            core: core.clone(),
            session_id: session_id.clone(),
            parser: Mutex::new(Processor::new()),
            sequence: Mutex::new(SequenceState::default()),
            size: Arc::clone(&size),
            title: Arc::clone(&title),
            palette: Arc::clone(&terminal_palette),
            waker,
            viewers,
            user_input,
            input_tracker: Mutex::new(input_tracker),
            fixture_recorder,
            link_cwd: std::sync::OnceLock::new(),
        });
        core.sessions
            .report_input_state(&session_id, initial_observation);

        let term_for_events = Arc::downgrade(&term);
        thread::Builder::new()
            .name(format!("native-term-events-{session_id}"))
            .spawn(move || {
                let write = |bytes: &[u8]| {
                    let _ = core.sessions.inject_stdin(&session_id, bytes);
                };
                while let Ok(event) = rx.recv() {
                    match event {
                        Event::PtyWrite(text) => write(text.as_bytes()),
                        Event::ColorRequest(index, format) => {
                            let palette = *terminal_palette.lock().unwrap();
                            let rgb = term_for_events
                                .upgrade()
                                .map(|term| {
                                    query_color_for(
                                        &*term.lock_unfair(),
                                        index,
                                        &palette.base,
                                        palette.theme,
                                    )
                                })
                                .unwrap_or_else(|| {
                                    crate::palette::resolve_index_for(
                                        index,
                                        &palette.base,
                                        palette.theme,
                                    )
                                });
                            write(format(rgb).as_bytes());
                        }
                        Event::TextAreaSizeRequest(format) => {
                            let (cols, rows) = *size.lock().unwrap();
                            let reply = format(WindowSize {
                                num_lines: rows,
                                num_cols: cols,
                                cell_width: 0,
                                cell_height: 0,
                            });
                            write(reply.as_bytes());
                        }
                        Event::Title(new_title) => {
                            *title.lock().unwrap() = new_title;
                        }
                        Event::ResetTitle => {
                            title.lock().unwrap().clear();
                        }
                        _ => {}
                    }
                }
            })
            .context("spawn terminal event thread")?;

        Ok(session)
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn view(&self) -> TerminalView {
        self.viewers.fetch_add(1, Ordering::AcqRel);
        TerminalView {
            viewers: Arc::clone(&self.viewers),
        }
    }

    pub fn title(&self) -> String {
        self.title.lock().unwrap().clone()
    }

    pub fn set_palette(&self, palette: palette::TerminalPalette) {
        let mut current = self.palette.lock().unwrap();
        if current.theme != palette {
            *current = PaletteState::new(palette);
            drop(current);
            (self.waker)();
        }
    }

    pub fn palette(&self) -> palette::TerminalPalette {
        self.palette.lock().unwrap().theme
    }

    pub fn configure(&self, scrollback: usize, cursor_shape: CursorShape) {
        self.term.lock().set_options(Config {
            scrolling_history: scrollback,
            semantic_escape_chars: XTERM_WORD_SEPARATORS.to_owned(),
            default_cursor_style: CursorStyle {
                shape: cursor_shape,
                blinking: true,
            },
            ..Config::default()
        });
        (self.waker)();
    }

    pub fn output_activity(&self) -> TerminalOutputActivity {
        let sequence = self.sequence.lock().unwrap();
        TerminalOutputActivity {
            last_seq: sequence.last,
            tui_ready_seq: sequence.tui_ready_seq,
            first_paint_seq: sequence.first_paint_seq,
            last_output_at: sequence.last_output_at,
        }
    }

    pub fn feed_output(&self, event: &OutputEvent) -> Result<()> {
        let bytes = &event.bytes;
        let now = Instant::now();
        let mut sequence = self.sequence.lock().unwrap();
        if event.seq <= sequence.last {
            return Ok(());
        }
        if let Some(recorder) = self.fixture_recorder.as_ref() {
            recorder.record_output(bytes);
        }
        sequence.last = event.seq;
        sequence.last_output_at = Some(Instant::now());
        if chunk_indicates_tui_ready(bytes) {
            sequence.tui_ready_seq = sequence.tui_ready_seq.max(event.seq);
        }
        let mut parser = self.parser.lock().unwrap();
        let mut input_tracker = self.input_tracker.lock().unwrap();
        let mut term = self.term.lock();
        let previous_mode = *term.mode();
        parser.advance(&mut *term, bytes);
        if sequence.first_paint_seq == 0
            && term
                .grid()
                .display_iter()
                .any(|cell| !cell.c.is_whitespace())
        {
            sequence.first_paint_seq = event.seq;
        }
        let mode = *term.mode();
        if !previous_mode.intersects(TermMode::MOUSE_MODE) && mode.intersects(TermMode::MOUSE_MODE)
        {
            term.selection = None;
        }
        let input_observation = input_tracker.observe_output(now, &term);
        drop(term);
        drop(input_tracker);
        drop(parser);
        drop(sequence);
        if let Some(observation) = input_observation {
            self.core
                .sessions
                .report_input_state(&self.session_id, observation);
        }
        if self.viewers.load(Ordering::Acquire) > 0 {
            (self.waker)();
        }
        Ok(())
    }

    pub fn submit_text(&self, text: &str) -> runner_backend::error::Result<()> {
        self.send_text(text)?;
        self.send_key("enter", false, false, false, None)?;
        Ok(())
    }

    pub fn write_user_bytes(&self, bytes: &[u8]) -> runner_backend::error::Result<()> {
        self.clear_selection();
        match &self.user_input {
            UserInput::Inline => self.core.sessions.inject_direct_stdin(
                &self.session_id,
                bytes,
                &self.core.session_events(),
            ),
            UserInput::Queued(tx) => tx.send(bytes.to_vec()).map_err(|_| {
                runner_backend::error::Error::msg(format!(
                    "terminal input worker stopped: {}",
                    self.session_id
                ))
            }),
        }
    }

    pub fn send_key(
        &self,
        key: &str,
        ctrl: bool,
        alt: bool,
        shift: bool,
        key_char: Option<&str>,
    ) -> runner_backend::error::Result<bool> {
        let input = InputEvent::Key {
            kind: crate::mappings::classify_key(key, ctrl, alt, shift, key_char),
        };
        let app_cursor = self
            .term
            .lock_unfair()
            .mode()
            .contains(TermMode::APP_CURSOR);
        match crate::mappings::encode_key(key, ctrl, alt, shift, key_char, app_cursor) {
            Some(bytes) => {
                self.observe_input(&input);
                self.write_user_bytes(&bytes)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub fn send_text(&self, text: &str) -> runner_backend::error::Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        self.observe_input(&InputEvent::Key {
            kind: crate::mappings::InputKind::Content {
                text: text.to_owned(),
            },
        });
        self.write_user_bytes(text.as_bytes())
    }

    pub fn set_composing(&self, composing: bool) {
        self.observe_input(&InputEvent::Composing { composing });
    }

    pub fn paste(&self, text: &str) -> runner_backend::error::Result<()> {
        let bracketed = self
            .term
            .lock_unfair()
            .mode()
            .contains(TermMode::BRACKETED_PASTE);
        self.observe_input(&InputEvent::Paste {
            text: text.to_owned(),
        });
        self.write_user_bytes(&crate::mappings::encode_paste(text, bracketed))
    }

    fn observe_input(&self, input: &InputEvent) {
        if let Some(recorder) = self.fixture_recorder.as_ref() {
            recorder.record_input(input);
        }
        let observation = {
            let mut tracker = self.input_tracker.lock().unwrap();
            let term = self.term.lock_unfair();
            tracker.observe_input(input, Instant::now(), &term)
        };
        if let Some(observation) = observation {
            self.core
                .sessions
                .report_input_state(&self.session_id, observation);
        }
    }

    pub fn input_reset_guard(&self) -> u64 {
        self.input_tracker.lock().unwrap().reset_guard()
    }

    pub fn reset_input_state(&self, guard: u64) {
        let observation = self
            .input_tracker
            .lock()
            .unwrap()
            .reset_if_unchanged(guard, Instant::now());
        if let Some(observation) = observation {
            self.core
                .sessions
                .report_input_state(&self.session_id, observation);
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        let cols = cols.max(2);
        let rows = rows.max(2);
        let rows_changed = {
            let mut size = self.size.lock().unwrap();
            if *size == (cols, rows) {
                return;
            }
            let rows_changed = size.1 != rows;
            *size = (cols, rows);
            rows_changed
        };
        let _ =
            runner_backend::ops::session::session_resize(&self.core, &self.session_id, cols, rows);
        let mut term = self.term.lock();
        term.resize(TermSize::new(cols as usize, rows as usize));
        if rows_changed {
            term.selection = None;
        }
    }

    pub fn size(&self) -> (u16, u16) {
        *self.size.lock().unwrap()
    }

    pub fn scroll(&self, delta_lines: i32, bypass_reporting: bool) {
        let mode = *self.term.lock_unfair().mode();
        match crate::mappings::encode_scroll(mode, delta_lines, bypass_reporting) {
            Some(bytes) => {
                let _ = self.write_user_bytes(&bytes);
            }
            None => {
                self.term.lock().scroll_display(Scroll::Delta(delta_lines));
                (self.waker)();
            }
        }
    }

    pub fn scroll_to_bottom(&self) {
        self.term.lock().scroll_display(Scroll::Bottom);
    }

    pub fn mode(&self) -> TermMode {
        *self.term.lock_unfair().mode()
    }

    pub fn start_selection(&self, ty: SelectionType, point: Point, side: Side) {
        self.term.lock().selection = Some(Selection::new(ty, point, side));
        (self.waker)();
    }

    pub fn update_selection(&self, point: Point, side: Side) -> bool {
        let mut term = self.term.lock();
        let Some(selection) = term.selection.as_mut() else {
            return false;
        };
        selection.update(point, side);
        drop(term);
        (self.waker)();
        true
    }

    pub fn clear_selection(&self) -> bool {
        let cleared = self.term.lock().selection.take().is_some();
        if cleared {
            (self.waker)();
        }
        cleared
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term
            .lock_unfair()
            .selection_to_string()
            .filter(|text| !text.is_empty())
    }

    pub fn selection_contains(&self, point: Point) -> bool {
        let term = self.term.lock_unfair();
        term.selection
            .as_ref()
            .and_then(|selection| selection.to_range(&*term))
            .is_some_and(|range| range.contains(point))
    }

    pub fn selection_type(&self) -> Option<SelectionType> {
        self.term
            .lock_unfair()
            .selection
            .as_ref()
            .map(|selection| selection.ty)
    }

    pub fn cell_is_whitespace(&self, point: Point) -> bool {
        let term = self.term.lock_unfair();
        let point = point.grid_clamp(&*term, Boundary::Grid);
        let cell = &term.grid()[point];
        !cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            && cell.c.is_whitespace()
    }

    pub fn link_at(&self, point: Point) -> Option<TerminalLink> {
        let cwd = self
            .link_cwd
            .get_or_init(|| resolve_link_cwd(&self.core, &self.session_id));
        terminal_link_at(&*self.term.lock_unfair(), point, cwd.as_deref())
    }

    pub fn move_cursor_to_viewport(&self, column: usize, row: usize) {
        let (cursor, columns, mode, display_offset) = {
            let term = self.term.lock_unfair();
            (
                term.grid().cursor.point,
                term.columns(),
                *term.mode(),
                term.grid().display_offset(),
            )
        };
        if display_offset != 0 || columns == 0 {
            return;
        }

        let app_cursor = mode.contains(TermMode::APP_CURSOR);
        let mut bytes = Vec::new();
        if mode.contains(TermMode::ALT_SCREEN) {
            let vertical = row as i32 - cursor.line.0;
            let key = if vertical < 0 { "up" } else { "down" };
            if let Some(sequence) =
                crate::mappings::encode_key(key, false, false, false, None, app_cursor)
            {
                bytes.extend(sequence.repeat(vertical.unsigned_abs() as usize));
            }
            let horizontal = column as i64 - cursor.column.0 as i64;
            let key = if horizontal < 0 { "left" } else { "right" };
            if let Some(sequence) =
                crate::mappings::encode_key(key, false, false, false, None, app_cursor)
            {
                bytes.extend(sequence.repeat(horizontal.unsigned_abs() as usize));
            }
        } else {
            let cursor_index = cursor.line.0 as i64 * columns as i64 + cursor.column.0 as i64;
            let target_index = row as i64 * columns as i64 + column as i64;
            let delta = target_index - cursor_index;
            let key = if delta < 0 { "left" } else { "right" };
            if let Some(sequence) =
                crate::mappings::encode_key(key, false, false, false, None, app_cursor)
            {
                bytes.extend(sequence.repeat(delta.unsigned_abs() as usize));
            }
        }
        if !bytes.is_empty() {
            let _ = self.write_user_bytes(&bytes);
        }
    }

    pub fn scroll_state(&self) -> TerminalScrollState {
        let term = self.term.lock_unfair();
        TerminalScrollState {
            history_lines: term.history_size(),
            display_offset: term.grid().display_offset(),
            screen_lines: term.screen_lines(),
        }
    }

    pub fn scroll_to_display_offset(&self, display_offset: usize) {
        let mut term = self.term.lock();
        let current = term.grid().display_offset();
        let target = display_offset.min(term.history_size());
        let delta = target as i64 - current as i64;
        term.scroll_display(Scroll::Delta(
            delta.clamp(i32::MIN as i64, i32::MAX as i64) as i32
        ));
        drop(term);
        (self.waker)();
    }
}

struct LinkCell {
    point: Point,
    start: usize,
    end: usize,
    hyperlink: Option<(String, String)>,
}

/// Relative paths in a session's output resolve against the session row's
/// cwd, else its project's directory. Resolved once, on the first link
/// lookup, so a session that is never ⌘-hovered never touches the database.
fn resolve_link_cwd(core: &AppCore, session_id: &str) -> Option<PathBuf> {
    let conn = core.db.get().ok()?;
    let row = runner_backend::repo::session::get_row(&conn, session_id).ok()??;
    if let Some(cwd) = row.cwd.filter(|cwd| !cwd.trim().is_empty()) {
        return Some(PathBuf::from(cwd));
    }
    let project = runner_backend::repo::project::get(&conn, &row.project_id?).ok()??;
    Some(PathBuf::from(project.cwd))
}

fn terminal_link_at<T>(term: &Term<T>, point: Point, cwd: Option<&Path>) -> Option<TerminalLink> {
    let mut point = point.grid_clamp(term, Boundary::Grid);
    if term.grid()[point]
        .flags
        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
    {
        point.column = Column(point.column.0.saturating_sub(1));
    }

    let start = term.line_search_left(point);
    let end = term.line_search_right(point);
    let mut text = String::new();
    let mut cells = Vec::new();
    for line in start.line.0..=end.line.0 {
        for column in 0..term.columns() {
            let cell_point = Point::new(Line(line), Column(column));
            let cell = &term.grid()[cell_point];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            let rendered_start = text.len();
            text.push(cell.c);
            if let Some(zerowidth) = cell.zerowidth() {
                text.extend(zerowidth.iter());
            }
            cells.push(LinkCell {
                point: cell_point,
                start: rendered_start,
                end: text.len(),
                hyperlink: cell
                    .hyperlink()
                    .map(|link| (link.id().to_owned(), link.uri().to_owned())),
            });
        }
    }

    let hit = cells.iter().position(|cell| cell.point == point)?;
    if let Some((id, uri)) = cells[hit].hyperlink.as_ref() {
        let mut first = hit;
        while first > 0
            && cells[first - 1]
                .hyperlink
                .as_ref()
                .is_some_and(|link| link.0 == *id)
        {
            first -= 1;
        }
        let mut last = hit;
        while last + 1 < cells.len()
            && cells[last + 1]
                .hyperlink
                .as_ref()
                .is_some_and(|link| link.0 == *id)
        {
            last += 1;
        }
        return Some(TerminalLink {
            target: file_target_from_uri(uri).unwrap_or_else(|| LinkTarget::Url(uri.clone())),
            start: cells[first].point,
            end: cells[last].point,
        });
    }

    let hit_cell = &cells[hit];
    let overlaps_hit =
        |range: &std::ops::Range<usize>| range.start < hit_cell.end && range.end > hit_cell.start;
    let (range, target) = match url_regex()
        .find_iter(&text)
        .find(|matched| overlaps_hit(&matched.range()))
    {
        Some(matched) => (
            matched.range(),
            LinkTarget::Url(matched.as_str().to_owned()),
        ),
        None => {
            let captures = file_link_regex()
                .captures_iter(&text)
                .find(|captures| overlaps_hit(&captures.get(0).map_or(0..0, |m| m.range())))?;
            (
                captures.get(0)?.range(),
                file_target_from_captures(&captures, cwd)?,
            )
        }
    };
    let first = cells.iter().find(|cell| cell.end > range.start)?;
    let last = cells.iter().rev().find(|cell| cell.start < range.end)?;
    Some(TerminalLink {
        target,
        start: first.point,
        end: last.point,
    })
}

/// A path-shaped token with an optional `:LINE[:COL]`, `(LINE,COL)`, or
/// `#LLINE` suffix. Deliberately loose: the filesystem check in
/// `resolve_file_candidate` is what separates `src/lib.rs` from `and/or`.
fn file_link_regex() -> &'static Regex {
    static FILE_REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    FILE_REGEX.get_or_init(|| {
        Regex::new(
            r##"(?x)
            (?P<path>
                (?: ~/ | \.\.?/ | / )?
                [^\s/:()\[\]{}<>'"`|\\,;=*?!\x23]* [^\s/:()\[\]{}<>'"`|\\,;=*?!\x23.]
                (?: / [^\s/:()\[\]{}<>'"`|\\,;=*?!\x23]* [^\s/:()\[\]{}<>'"`|\\,;=*?!\x23.] )*
            )
            (?:
                : (?P<line>\d+) (?: : (?P<column>\d+) )?
              | \( (?P<pline>\d+) , \s? (?P<pcolumn>\d+) \)
              | \x23 L (?P<hline>\d+)
            )?
            "##,
        )
        .expect("valid terminal file link regex")
    })
}

fn file_target_from_captures(
    captures: &regex::Captures<'_>,
    cwd: Option<&Path>,
) -> Option<LinkTarget> {
    let path = resolve_file_candidate(captures.name("path")?.as_str(), cwd)?;
    let number = |name: &str| {
        captures
            .name(name)
            .and_then(|matched| matched.as_str().parse::<u32>().ok())
    };
    Some(LinkTarget::File {
        path,
        line: number("line")
            .or_else(|| number("pline"))
            .or_else(|| number("hline")),
        column: number("column").or_else(|| number("pcolumn")),
    })
}

fn resolve_file_candidate(candidate: &str, cwd: Option<&Path>) -> Option<PathBuf> {
    let path = if let Some(rest) = candidate.strip_prefix("~/") {
        runner_backend::app_paths::home_dir()?.join(rest)
    } else if candidate.starts_with('/') {
        PathBuf::from(candidate)
    } else {
        cwd?.join(candidate)
    };
    let path = normalize_path(&path);
    std::fs::metadata(&path).ok()?.is_file().then_some(path)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other),
        }
    }
    normalized
}

fn file_target_from_uri(uri: &str) -> Option<LinkTarget> {
    let rest = uri.strip_prefix("file://")?;
    let (rest, fragment) = rest
        .split_once('#')
        .map_or((rest, None), |(rest, fragment)| (rest, Some(fragment)));
    let path = percent_decode(&rest[rest.find('/')?..]);
    let (path, line, column) = split_line_suffix(&path);
    let line = line.or_else(|| {
        fragment?
            .strip_prefix('L')?
            .split(|c: char| !c.is_ascii_digit())
            .next()?
            .parse()
            .ok()
    });
    Some(LinkTarget::File {
        path: PathBuf::from(path),
        line,
        column,
    })
}

fn split_line_suffix(path: &str) -> (&str, Option<u32>, Option<u32>) {
    let mut path = path;
    let mut numbers = Vec::new();
    while numbers.len() < 2 {
        let Some((head, tail)) = path.rsplit_once(':') else {
            break;
        };
        let Ok(number) = tail.parse::<u32>() else {
            break;
        };
        numbers.push(number);
        path = head;
    }
    match numbers.as_slice() {
        [line] => (path, Some(*line), None),
        [column, line] => (path, Some(*line), Some(*column)),
        _ => (path, None, None),
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = (bytes[index] == b'%' && index + 2 < bytes.len())
            .then(|| std::str::from_utf8(&bytes[index + 1..index + 3]).ok())
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                decoded.push(byte);
                index += 3;
            }
            None => {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn url_regex() -> &'static Regex {
    static URL_REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    URL_REGEX.get_or_init(|| {
        Regex::new(r#"(https?|HTTPS?)://[^\s\"'!*(){}|\\\^<>`]*[^\s\"':,.!?{}|\\\^~\[\]`()<>]"#)
            .expect("valid terminal URL regex")
    })
}

fn chunk_indicates_tui_ready(bytes: &[u8]) -> bool {
    [b"\x1b[?2004h".as_slice(), b"\x1b[?1049h", b"\x1b[?47h"]
        .into_iter()
        .any(|signal| bytes.windows(signal.len()).any(|window| window == signal))
}

pub struct TerminalBridge {
    core: AppCore,
    sessions: Mutex<HashMap<String, Arc<TerminalSession>>>,
    waker: Arc<dyn Fn() + Send + Sync>,
}

impl TerminalBridge {
    pub fn new(core: AppCore, waker: Arc<dyn Fn() + Send + Sync>) -> Result<Arc<Self>> {
        let bridge = Arc::new(Self {
            core,
            sessions: Mutex::new(HashMap::new()),
            waker,
        });
        let observer: Arc<dyn SessionEvents> = bridge.clone();
        bridge
            .core
            .session_event_observer
            .install(Arc::downgrade(&observer));
        Ok(bridge)
    }

    pub fn session(&self, session_id: &str) -> Option<Arc<TerminalSession>> {
        self.sessions.lock().unwrap().get(session_id).cloned()
    }

    pub fn live_session_count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }

    fn new_session(
        &self,
        session_id: &str,
        mission_id: Option<&str>,
        cols: u16,
        rows: u16,
    ) -> Result<Arc<TerminalSession>> {
        TerminalSession::attach_with_input_mode(
            self.core.clone(),
            session_id.to_owned(),
            cols,
            rows,
            Arc::clone(&self.waker),
            if mission_id.is_some() {
                UserInputMode::Queued
            } else {
                UserInputMode::Inline
            },
        )
    }

    fn replace_session(&self, event: &SessionSpawnedEvent) -> Result<()> {
        let session = self.new_session(
            &event.session_id,
            event.mission_id.as_deref(),
            event.cols,
            event.rows,
        )?;
        self.sessions
            .lock()
            .unwrap()
            .insert(event.session_id.clone(), session);
        Ok(())
    }

    fn remove_session(&self, session_id: &str) {
        self.sessions.lock().unwrap().remove(session_id);
        (self.waker)();
    }
}

impl SessionEvents for TerminalBridge {
    fn output(&self, event: &OutputEvent) {
        let session = self.session(&event.session_id).or_else(|| {
            let (cols, rows) =
                runner_backend::ops::session::session_last_size(&self.core, &event.session_id)
                    .ok()
                    .flatten()
                    .unwrap_or((80, 24));
            match self.new_session(&event.session_id, event.mission_id.as_deref(), cols, rows) {
                Ok(session) => {
                    self.sessions
                        .lock()
                        .unwrap()
                        .insert(event.session_id.clone(), Arc::clone(&session));
                    Some(session)
                }
                Err(error) => {
                    log::error!(
                        "create terminal for session {} output failed: {error}",
                        event.session_id
                    );
                    None
                }
            }
        });
        if let Some(session) = session {
            if let Err(error) = session.feed_output(event) {
                log::error!(
                    "feed terminal for session {} failed: {error}",
                    event.session_id
                );
            }
        }
    }

    fn spawned(&self, event: &SessionSpawnedEvent) {
        if let Err(error) = self.replace_session(event) {
            log::error!(
                "replace terminal for spawned session {} failed: {error}",
                event.session_id
            );
        }
        (self.waker)();
    }

    fn exit(&self, event: &ExitEvent) {
        self.remove_session(&event.session_id);
    }

    fn archived(&self, event: &SessionUpdatedEvent) {
        self.remove_session(&event.session_id);
    }
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::grid::Dimensions as _;
    use alacritty_terminal::index::{Column, Line, Point, Side};
    use alacritty_terminal::selection::SelectionType;
    use alacritty_terminal::term::cell::Flags;
    use alacritty_terminal::vte::ansi::CursorShape;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    use runner_backend::session::manager::{
        ExitEvent, OutputEvent, SessionEvents, SessionSpawnedEvent, SessionUpdatedEvent,
    };

    use super::{LinkTarget, TerminalBridge, TerminalSession, UserInputMode};
    use crate::replay::visible_lines;
    use runner_backend::AppCore;

    /// Minimal `AppCore` over a temp dir — the pieces `boot_core` wires
    /// in runner-app, minus login-shell discovery and startup cleanup.
    fn test_core(root: &std::path::Path) -> AppCore {
        let app_data_dir = root.join("app-data");
        std::fs::create_dir_all(&app_data_dir).unwrap();
        let pool =
            Arc::new(runner_backend::db::open_pool(&app_data_dir.join("runner.db")).unwrap());
        let runtime: Arc<dyn runner_backend::session::runtime::SessionRuntime> =
            Arc::new(runner_backend::session::pty_runtime::PtyRuntime::new());
        let windows = Arc::new(runner_backend::windows::WindowRegistry::new());
        windows.register("main");
        let runtime_shell_env = Arc::new(std::sync::RwLock::new(
            runner_backend::shell_path::LoginShellEnv::default(),
        ));
        let runtime_discovery = Arc::new(std::sync::RwLock::new(
            runner_backend::shell_path::DiscoveryState::startup(None, None),
        ));
        AppCore {
            db: pool,
            app_data_dir,
            sessions: runner_backend::session::SessionManager::new(
                Arc::clone(&runtime_shell_env),
                Arc::clone(&runtime_discovery),
                runtime,
            ),
            runtime_shell_env,
            runtime_discovery,
            buses: runner_backend::event_bus::BusRegistry::new(),
            routers: runner_backend::router::RouterRegistry::new(),
            mission_grid_hint: Arc::new(std::sync::Mutex::new(None)),
            mcp: Arc::new(runner_backend::mcp::McpHandle::new()),
            windows,
            events: runner_backend::events::EventChannel::new(),
            session_event_observer: Default::default(),
            app_version: "0.0.0-test".into(),
        }
    }

    fn output(seq: u64, text: &str) -> OutputEvent {
        OutputEvent {
            session_id: "replay-race".into(),
            mission_id: None,
            seq,
            bytes: text.as_bytes().to_vec(),
        }
    }

    #[test]
    fn terminal_links_detect_plain_urls_and_osc_8_targets() {
        let temp = tempfile::tempdir().unwrap();
        let core = test_core(temp.path());
        let terminal =
            TerminalSession::attach(core, "replay-race".into(), 80, 4, Arc::new(|| {})).unwrap();
        terminal
            .feed_output(&output(
                1,
                "plain https://example.com/path\r\n\x1b]8;;https://example.com/osc\x1b\\linked label\x1b]8;;\x1b\\",
            ))
            .unwrap();

        let plain = terminal
            .link_at(Point::new(Line(0), Column(10)))
            .expect("plain URL");
        assert_eq!(
            plain.target,
            LinkTarget::Url("https://example.com/path".into())
        );
        assert!(plain.contains(Point::new(Line(0), Column(6))));
        assert!(plain.contains(Point::new(Line(0), Column(29))));

        let osc = terminal
            .link_at(Point::new(Line(1), Column(3)))
            .expect("OSC 8 link");
        assert_eq!(
            osc.target,
            LinkTarget::Url("https://example.com/osc".into())
        );
        assert_eq!(osc.start, Point::new(Line(1), Column(0)));
        assert_eq!(osc.end, Point::new(Line(1), Column(11)));
    }

    #[test]
    fn terminal_url_detection_follows_soft_wraps_but_not_hard_line_breaks() {
        let temp = tempfile::tempdir().unwrap();
        let core = test_core(temp.path());
        let terminal =
            TerminalSession::attach(core, "replay-race".into(), 14, 4, Arc::new(|| {})).unwrap();
        terminal
            .feed_output(&output(1, "xxhttps://example.com/path"))
            .unwrap();

        let wrapped = terminal
            .link_at(Point::new(Line(1), Column(3)))
            .expect("wrapped URL");
        assert_eq!(
            wrapped.target,
            LinkTarget::Url("https://example.com/path".into())
        );
        assert_eq!(wrapped.start, Point::new(Line(0), Column(2)));
        assert_eq!(wrapped.end, Point::new(Line(1), Column(11)));

        {
            let mut term = terminal.term.lock();
            term.grid_mut()[Line(0)][Column(13)]
                .flags
                .remove(Flags::WRAPLINE);
        }
        assert_eq!(
            terminal
                .link_at(Point::new(Line(0), Column(3)))
                .expect("complete URL prefix")
                .target,
            LinkTarget::Url("https://exam".into())
        );
        assert!(terminal.link_at(Point::new(Line(1), Column(3))).is_none());
    }

    fn insert_session_row(
        core: &AppCore,
        id: &str,
        cwd: Option<&std::path::Path>,
        project_id: Option<String>,
    ) {
        let conn = core.db.get().unwrap();
        let mut row = runner_backend::repo::session::SessionRowDb::new_running(id.into());
        row.cwd = cwd.map(|cwd| cwd.to_string_lossy().into_owned());
        row.project_id = project_id;
        runner_backend::repo::session::insert(&conn, &row).unwrap();
    }

    fn file_target(path: std::path::PathBuf, line: Option<u32>, column: Option<u32>) -> LinkTarget {
        LinkTarget::File { path, line, column }
    }

    #[test]
    fn terminal_file_links_resolve_existing_paths_against_the_session_cwd() {
        let temp = tempfile::tempdir().unwrap();
        let core = test_core(temp.path());
        let cwd = temp.path().join("project");
        std::fs::create_dir_all(cwd.join("src")).unwrap();
        for file in ["src/lib.rs", "README.md", "Makefile"] {
            std::fs::write(cwd.join(file), "").unwrap();
        }
        insert_session_row(&core, "file-links", Some(&cwd), None);
        let terminal =
            TerminalSession::attach(core, "file-links".into(), 120, 6, Arc::new(|| {})).unwrap();
        terminal
            .feed_output(&output(
                1,
                "see src/lib.rs:12:5, then ./README.md and Makefile\r\nsrc/lib.rs(3,4) src/lib.rs#L9 src/nope.rs:1 a/b and/or 1.2.3 (src/lib.rs:2).",
            ))
            .unwrap();
        let link = |column: usize| terminal.link_at(Point::new(Line(0), Column(column)));
        let link_1 = |column: usize| terminal.link_at(Point::new(Line(1), Column(column)));

        let with_line = link(6).expect("path with line and column");
        assert_eq!(
            with_line.target,
            file_target(cwd.join("src/lib.rs"), Some(12), Some(5))
        );
        assert_eq!(with_line.start, Point::new(Line(0), Column(4)));
        assert_eq!(with_line.end, Point::new(Line(0), Column(18)));
        assert!(
            link(19).is_none(),
            "the trailing comma is not part of the link"
        );

        let dot_relative = link(30).expect("./ relative path");
        assert_eq!(
            dot_relative.target,
            file_target(cwd.join("README.md"), None, None)
        );
        assert_eq!(dot_relative.start, Point::new(Line(0), Column(26)));
        assert_eq!(dot_relative.end, Point::new(Line(0), Column(36)));

        let bare_name = link(45).expect("extension-less file that exists");
        assert_eq!(
            bare_name.target,
            file_target(cwd.join("Makefile"), None, None)
        );
        assert!(link(39).is_none(), "`and` is not a file");

        let parens = link_1(2).expect("(line,col) suffix");
        assert_eq!(
            parens.target,
            file_target(cwd.join("src/lib.rs"), Some(3), Some(4))
        );
        assert_eq!(parens.end, Point::new(Line(1), Column(14)));

        let hash_line = link_1(20).expect("#L suffix");
        assert_eq!(
            hash_line.target,
            file_target(cwd.join("src/lib.rs"), Some(9), None)
        );
        assert_eq!(hash_line.end, Point::new(Line(1), Column(28)));

        assert!(
            link_1(32).is_none(),
            "a path that does not exist never links"
        );
        assert!(link_1(45).is_none(), "a/b never links");
        assert!(link_1(50).is_none(), "and/or never links");
        assert!(link_1(57).is_none(), "version strings never link");

        let wrapped_in_parens = link_1(65).expect("path inside parentheses");
        assert_eq!(
            wrapped_in_parens.target,
            file_target(cwd.join("src/lib.rs"), Some(2), None)
        );
        assert_eq!(wrapped_in_parens.start, Point::new(Line(1), Column(62)));
        assert_eq!(wrapped_in_parens.end, Point::new(Line(1), Column(73)));
    }

    #[test]
    fn terminal_file_links_fall_back_to_the_project_cwd() {
        let temp = tempfile::tempdir().unwrap();
        let core = test_core(temp.path());
        let project_dir = temp.path().join("proj");
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(project_dir.join("notes.md"), "").unwrap();
        let project = {
            let conn = core.db.get().unwrap();
            runner_backend::repo::project::create(&conn, "Proj", project_dir.to_str().unwrap())
                .unwrap()
        };
        insert_session_row(&core, "project-links", None, Some(project.id));
        let terminal =
            TerminalSession::attach(core, "project-links".into(), 80, 4, Arc::new(|| {})).unwrap();
        terminal.feed_output(&output(1, "edited notes.md")).unwrap();

        let link = terminal
            .link_at(Point::new(Line(0), Column(9)))
            .expect("project-relative path");
        assert_eq!(
            link.target,
            file_target(project_dir.join("notes.md"), None, None)
        );
    }

    #[cfg(unix)]
    #[test]
    fn terminal_file_links_without_a_cwd_only_resolve_absolute_paths() {
        let temp = tempfile::tempdir().unwrap();
        let core = test_core(temp.path());
        let file = temp.path().join("abs.rs");
        std::fs::write(&file, "").unwrap();
        let terminal =
            TerminalSession::attach(core, "no-row".into(), 200, 4, Arc::new(|| {})).unwrap();
        let absolute = file.display().to_string();
        terminal
            .feed_output(&output(1, &format!("{absolute} abs.rs")))
            .unwrap();

        let link = terminal
            .link_at(Point::new(Line(0), Column(1)))
            .expect("absolute path");
        assert_eq!(link.target, file_target(file.clone(), None, None));
        assert!(terminal
            .link_at(Point::new(Line(0), Column(absolute.len() + 2)))
            .is_none());
    }

    #[test]
    fn terminal_osc_8_file_uris_map_to_file_targets() {
        let temp = tempfile::tempdir().unwrap();
        let core = test_core(temp.path());
        let terminal =
            TerminalSession::attach(core, "osc-files".into(), 80, 4, Arc::new(|| {})).unwrap();
        terminal
            .feed_output(&output(
                1,
                "\x1b]8;;file:///tmp/x.rs:7\x1b\\x.rs\x1b]8;;\x1b\\ \x1b]8;;file://localhost/tmp/y%20z.rs#L12\x1b\\y\x1b]8;;\x1b\\",
            ))
            .unwrap();

        let with_suffix = terminal
            .link_at(Point::new(Line(0), Column(1)))
            .expect("file URI with :line");
        assert_eq!(
            with_suffix.target,
            file_target("/tmp/x.rs".into(), Some(7), None)
        );
        let with_fragment = terminal
            .link_at(Point::new(Line(0), Column(5)))
            .expect("file URI with #L fragment");
        assert_eq!(
            with_fragment.target,
            file_target("/tmp/y z.rs".into(), Some(12), None)
        );
    }

    #[test]
    fn queued_user_input_reports_backend_errors_without_blocking_the_caller() {
        let temp = tempfile::tempdir().unwrap();
        let core = test_core(temp.path());
        let mut events = core.events.subscribe();
        let terminal = TerminalSession::attach_with_input_mode(
            core,
            "missing-session".into(),
            80,
            24,
            Arc::new(|| {}),
            UserInputMode::Queued,
        )
        .unwrap();

        terminal.write_user_bytes(b"x").unwrap();

        let event = events.blocking_recv().unwrap();
        assert_eq!(event.name, "session/input-error");
        assert_eq!(event.payload["session_id"], "missing-session");
        assert!(event.payload["message"]
            .as_str()
            .unwrap()
            .contains("session not found"));
    }

    #[test]
    fn registry_creates_on_first_output_and_survives_view_drop() {
        let temp = tempfile::tempdir().unwrap();
        let core = test_core(temp.path());
        let events = core.session_events();
        let mut broadcast = core.events.subscribe();
        let bridge = TerminalBridge::new(core, Arc::new(|| {})).unwrap();
        assert_eq!(bridge.live_session_count(), 0);

        events.output(&output(1, "first-marker"));
        assert!(broadcast.try_recv().is_err());
        let view = bridge.session("replay-race").unwrap();
        assert_eq!(bridge.live_session_count(), 1);
        drop(view);

        events.output(&output(2, "second-marker"));
        let reopened = bridge.session("replay-race").unwrap();
        let rendered = {
            let term = reopened.term.lock();
            visible_lines(&*term).join("\n")
        };
        assert!(rendered.contains("first-markersecond-marker"));
    }

    #[test]
    fn hidden_terminal_output_does_not_wake_the_ui() {
        let temp = tempfile::tempdir().unwrap();
        let core = test_core(temp.path());
        let wakes = Arc::new(AtomicUsize::new(0));
        let wake_count = Arc::clone(&wakes);
        let terminal = TerminalSession::attach(
            core,
            "replay-race".into(),
            80,
            24,
            Arc::new(move || {
                wake_count.fetch_add(1, Ordering::Relaxed);
            }),
        )
        .unwrap();

        terminal.feed_output(&output(1, "hidden")).unwrap();
        assert_eq!(wakes.load(Ordering::Relaxed), 0);

        let view = terminal.view();
        terminal.feed_output(&output(2, "visible")).unwrap();
        assert_eq!(wakes.load(Ordering::Relaxed), 1);

        drop(view);
        terminal.feed_output(&output(3, "hidden-again")).unwrap();
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn registry_removes_exit_and_archive_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let core = test_core(temp.path());
        let events = core.session_events();
        let bridge = TerminalBridge::new(core, Arc::new(|| {})).unwrap();
        events.output(&output(1, "exit-marker"));
        let exited = Arc::downgrade(&bridge.session("replay-race").unwrap());
        events.exit(&ExitEvent {
            session_id: "replay-race".into(),
            mission_id: None,
            exit_code: Some(0),
            success: true,
        });
        assert!(bridge.session("replay-race").is_none());
        assert!(exited.upgrade().is_none());

        events.output(&output(2, "archive-marker"));
        let archived = Arc::downgrade(&bridge.session("replay-race").unwrap());
        events.archived(&SessionUpdatedEvent {
            session_id: "replay-race".into(),
            mission_id: None,
        });
        assert_eq!(bridge.live_session_count(), 0);
        assert!(archived.upgrade().is_none());
    }

    #[test]
    fn registry_replaces_terminal_on_respawn_and_resets_first_paint() {
        let temp = tempfile::tempdir().unwrap();
        let core = test_core(temp.path());
        let events = core.session_events();
        let bridge = TerminalBridge::new(core, Arc::new(|| {})).unwrap();
        events.spawned(&SessionSpawnedEvent {
            session_id: "replay-race".into(),
            mission_id: None,
            cols: 80,
            rows: 24,
        });
        events.output(&output(1, "old-child"));
        let old = bridge.session("replay-race").unwrap();
        assert_eq!(old.output_activity().first_paint_seq, 1);

        events.spawned(&SessionSpawnedEvent {
            session_id: "replay-race".into(),
            mission_id: None,
            cols: 100,
            rows: 30,
        });
        let fresh = bridge.session("replay-race").unwrap();
        assert!(!Arc::ptr_eq(&old, &fresh));
        assert_eq!(fresh.size(), (100, 30));
        assert_eq!(fresh.output_activity().first_paint_seq, 0);
        events.output(&output(2, "\x1b[?2004hnew-child"));

        let activity = fresh.output_activity();
        assert_eq!(activity.last_seq, 2);
        assert_eq!(activity.tui_ready_seq, 2);
        assert_eq!(activity.first_paint_seq, 2);
        let rendered = visible_lines(&*fresh.term.lock()).join("\n");
        assert!(rendered.contains("new-child"));
        assert!(!rendered.contains("old-child"));
    }

    #[test]
    fn registry_has_zero_live_terminals_after_twenty_spawn_exit_cycles() {
        let temp = tempfile::tempdir().unwrap();
        let core = test_core(temp.path());
        let events = core.session_events();
        let bridge = TerminalBridge::new(core, Arc::new(|| {})).unwrap();

        for seq in 1..=20 {
            events.spawned(&SessionSpawnedEvent {
                session_id: "replay-race".into(),
                mission_id: None,
                cols: 80,
                rows: 24,
            });
            events.output(&output(seq, "cycle"));
            assert_eq!(bridge.live_session_count(), 1);
            events.exit(&ExitEvent {
                session_id: "replay-race".into(),
                mission_id: None,
                exit_code: Some(0),
                success: true,
            });
            assert_eq!(bridge.live_session_count(), 0);
        }
    }

    #[test]
    fn terminal_scroll_state_and_absolute_scroll_stay_in_sync() {
        let temp = tempfile::tempdir().unwrap();
        let core = test_core(temp.path());
        let woke = Arc::new(AtomicBool::new(false));
        let wake_flag = Arc::clone(&woke);
        let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            wake_flag.store(true, Ordering::Release);
        });
        let terminal = TerminalSession::attach(core, "replay-race".into(), 80, 24, waker).unwrap();
        terminal
            .feed_output(&output(1, &"scrollback line\r\n".repeat(80)))
            .unwrap();

        let bottom = terminal.scroll_state();
        assert_eq!(bottom.screen_lines, 24);
        assert!(bottom.history_lines > 0);
        assert_eq!(bottom.display_offset, 0);

        woke.store(false, Ordering::Release);
        terminal.scroll_to_display_offset(bottom.history_lines);
        assert_eq!(terminal.scroll_state().display_offset, bottom.history_lines);
        assert!(woke.load(Ordering::Acquire));

        terminal.scroll_to_display_offset(0);
        assert_eq!(terminal.scroll_state().display_offset, 0);
    }

    #[test]
    fn terminal_configuration_applies_scrollback_and_cursor_shape() {
        let temp = tempfile::tempdir().unwrap();
        let core = test_core(temp.path());
        let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        let terminal = TerminalSession::attach(core, "replay-race".into(), 80, 5, waker).unwrap();

        terminal.configure(3, CursorShape::Beam);
        terminal
            .feed_output(&output(1, &"configured line\r\n".repeat(20)))
            .unwrap();

        let term = terminal.term.lock();
        assert_eq!(term.history_size(), 3);
        assert_eq!(term.renderable_content().cursor.shape, CursorShape::Beam);
    }

    #[test]
    fn terminal_selection_matches_xterm_words_wraps_and_line_copy() {
        let temp = tempfile::tempdir().unwrap();
        let core = test_core(temp.path());
        let terminal = TerminalSession::attach_with_input_mode(
            core,
            "replay-race".into(),
            8,
            3,
            Arc::new(|| {}),
            UserInputMode::Queued,
        )
        .unwrap();
        terminal.configure(100, CursorShape::Block);
        assert!(terminal.cell_is_whitespace(Point::new(Line(100), Column(100))));
        {
            let mut term = terminal.term.lock();
            for (column, character) in "foo/bar,".chars().enumerate() {
                term.grid_mut()[Line(0)][Column(column)].c = character;
            }
            for (column, character) in "abcdefgh".chars().enumerate() {
                term.grid_mut()[Line(1)][Column(column)].c = character;
            }
            term.grid_mut()[Line(1)][Column(7)]
                .flags
                .insert(Flags::WRAPLINE);
            for (column, character) in "ijk".chars().enumerate() {
                term.grid_mut()[Line(2)][Column(column)].c = character;
            }
        }

        terminal.start_selection(
            SelectionType::Semantic,
            Point::new(Line(0), Column(2)),
            Side::Left,
        );
        assert_eq!(terminal.selection_text().as_deref(), Some("foo/bar"));
        terminal.feed_output(&output(1, "\x1b[31m")).unwrap();
        terminal.scroll(1, true);
        assert_eq!(terminal.selection_text().as_deref(), Some("foo/bar"));

        terminal.start_selection(
            SelectionType::Simple,
            Point::new(Line(1), Column(5)),
            Side::Left,
        );
        terminal.update_selection(Point::new(Line(2), Column(1)), Side::Right);
        assert_eq!(terminal.selection_text().as_deref(), Some("fghij"));

        terminal.start_selection(
            SelectionType::Lines,
            Point::new(Line(2), Column(1)),
            Side::Left,
        );
        assert_eq!(terminal.selection_text().as_deref(), Some("abcdefghijk\n"));
    }

    #[test]
    fn user_input_vertical_resize_and_mouse_mode_clear_selection() {
        let temp = tempfile::tempdir().unwrap();
        let core = test_core(temp.path());
        let terminal = TerminalSession::attach_with_input_mode(
            core,
            "replay-race".into(),
            8,
            3,
            Arc::new(|| {}),
            UserInputMode::Queued,
        )
        .unwrap();
        let select = || {
            terminal.start_selection(
                SelectionType::Lines,
                Point::new(Line(0), Column(0)),
                Side::Left,
            );
            assert!(terminal.selection_text().is_some());
        };

        select();
        terminal.write_user_bytes(b"x").unwrap();
        assert!(terminal.selection_text().is_none());

        select();
        terminal.resize(8, 4);
        assert!(terminal.selection_text().is_none());

        select();
        terminal
            .feed_output(&output(1, "\x1b[?1000h\x1b[?1006h"))
            .unwrap();
        assert!(terminal.selection_text().is_none());
    }

    #[test]
    fn output_activity_tracks_sequence_idle_time_and_tui_ready_signals() {
        let temp = tempfile::tempdir().unwrap();
        let core = test_core(temp.path());
        let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        let terminal = TerminalSession::attach(core, "replay-race".into(), 80, 24, waker).unwrap();

        assert_eq!(terminal.output_activity().last_seq, 0);
        terminal.feed_output(&output(1, "booting")).unwrap();
        let first = terminal.output_activity();
        assert_eq!(first.last_seq, 1);
        assert_eq!(first.tui_ready_seq, 0);
        assert_eq!(first.first_paint_seq, 1);
        assert!(first.last_output_at.is_some());

        terminal
            .feed_output(&output(2, "\x1b[?2004hready"))
            .unwrap();
        let ready = terminal.output_activity();
        assert_eq!(ready.last_seq, 2);
        assert_eq!(ready.tui_ready_seq, 2);
        assert_eq!(ready.first_paint_seq, 1);
        assert!(ready.last_output_at >= first.last_output_at);
    }

    #[test]
    fn first_paint_requires_a_visible_non_whitespace_cell() {
        let temp = tempfile::tempdir().unwrap();
        let core = test_core(temp.path());
        let terminal =
            TerminalSession::attach(core, "replay-race".into(), 8, 3, Arc::new(|| {})).unwrap();

        terminal
            .feed_output(&output(1, "\x1b[?1049h\x1b[2J\x1b[H"))
            .unwrap();
        let alternate_screen = terminal.output_activity();
        assert_eq!(alternate_screen.tui_ready_seq, 1);
        assert_eq!(alternate_screen.first_paint_seq, 0);

        terminal
            .feed_output(&output(2, "\x1b[31m   \x1b[0m\x1b[2;2H"))
            .unwrap();
        assert_eq!(terminal.output_activity().first_paint_seq, 0);

        terminal.feed_output(&output(3, "x")).unwrap();
        assert_eq!(terminal.output_activity().first_paint_seq, 3);

        terminal.feed_output(&output(4, "later")).unwrap();
        assert_eq!(terminal.output_activity().first_paint_seq, 3);
    }
}
