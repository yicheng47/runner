//! Replay recorded PTY bytes into a headless Runner-owned terminal and
//! snapshot the visible grid — the regression harness the fixtures exist for.

use std::cell::{OnceCell, RefCell};

use runner_core::protocol::terminal::{SnapshotFormat, TerminalSnapshot};

use crate::fixtures::{decode_chunk, Fixture, FixtureEvent};
use crate::palette::{self, Rgb, TerminalPalette};
use crate::screen::{ScreenData, ScreenView};
use crate::snapshot::TerminalSnapshotState;
use crate::types::{GridPoint, Line, TerminalModeFlags};
use crate::vt::{
    self, RenderState, ScrollViewport, SnapshotDecodeOptions, TerminalConfig, TerminalEffect,
    DEFAULT_CONTINUATION_MAX_BYTES, DEFAULT_SCROLLBACK_LINES,
};

/// Reply emitted by a headless terminal while processing VT queries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalReply {
    ColorQuery { index: usize, reply: String },
    PtyWrite(String),
}

/// Runner-owned headless terminal for fixture replay, snapshot round-trips,
/// and unit/integration tests without exposing underlying VT engine types.
pub struct Terminal {
    term: vt::Terminal,
    render_state: RefCell<RenderState>,
    screen_cache: OnceCell<ScreenData>,
    continuation_cache: OnceCell<Vec<u8>>,
    effects: RefCell<Vec<TerminalEffect>>,
    config: TerminalConfig,
    render_hold_active: bool,
    held_sync_bytes: usize,
}

impl Terminal {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self::with_scrollback(cols, rows, DEFAULT_SCROLLBACK_LINES)
    }

    pub fn with_scrollback(cols: u16, rows: u16, scrollback: usize) -> Self {
        let config = TerminalConfig {
            cols: cols.max(1),
            rows: rows.max(1),
            scrollback_lines: scrollback,
            continuation_max_bytes: DEFAULT_CONTINUATION_MAX_BYTES,
            kitty_image_storage_limit: 0,
            enable_host_callbacks: true,
            enable_render_hold: true,
            ..TerminalConfig::default()
        };
        Self::from_vt_config(config)
    }

    pub fn with_kitty_keyboard(cols: u16, rows: u16) -> Self {
        Self::new(cols, rows)
    }

    fn from_vt_config(config: TerminalConfig) -> Self {
        let mut term = vt::Terminal::new(&config).expect("create libghostty-vt terminal");
        let _ = term.set_theme_defaults(palette::RUNNER);
        let render_state = RenderState::new().expect("create libghostty-vt render state");
        Self {
            term,
            render_state: RefCell::new(render_state),
            screen_cache: OnceCell::new(),
            continuation_cache: OnceCell::new(),
            effects: RefCell::new(Vec::new()),
            config,
            render_hold_active: false,
            held_sync_bytes: 0,
        }
    }

    /// Reconstruct a headless terminal from a local (`GhosttyBinary`) or
    /// companion (`Vt`) [`TerminalSnapshot`].
    pub fn from_snapshot(snapshot: &TerminalSnapshot) -> anyhow::Result<Self> {
        if snapshot.unavailable {
            anyhow::bail!("terminal snapshot unavailable");
        }
        match snapshot.format {
            SnapshotFormat::GhosttyBinary => {
                if snapshot.bytes.is_empty() {
                    anyhow::bail!("empty ghostty binary snapshot");
                }
                Self::from_binary_snapshot(&snapshot.bytes, DEFAULT_SCROLLBACK_LINES)
            }
            SnapshotFormat::Vt => {
                let mut term = Self::new(snapshot.cols, snapshot.rows);
                let split = snapshot.bytes.len().saturating_sub(snapshot.unfinished_len);
                term.advance(&snapshot.bytes[..split]);
                term.restore_preceding_char(snapshot.preceding_char);
                term.advance(&snapshot.bytes[split..]);
                Ok(term)
            }
        }
    }

    /// Decode a `libghostty-vt` binary snapshot blob into a headless terminal.
    pub fn from_binary_snapshot(bytes: &[u8], scrollback: usize) -> anyhow::Result<Self> {
        let config = TerminalConfig {
            scrollback_lines: scrollback,
            continuation_max_bytes: DEFAULT_CONTINUATION_MAX_BYTES,
            kitty_image_storage_limit: 0,
            enable_host_callbacks: true,
            enable_render_hold: true,
            ..TerminalConfig::default()
        };
        let mut term = vt::Terminal::decode_snapshot(
            bytes,
            SnapshotDecodeOptions {
                max_continuation_bytes: DEFAULT_CONTINUATION_MAX_BYTES,
                retain_continuation: true,
                compress_history: true,
                kitty_image_storage_limit: 0,
            },
            &config,
        )
        .map_err(|err| anyhow::anyhow!("decode ghostty snapshot: {err}"))?;
        let _ = term.set_theme_defaults(palette::RUNNER);
        term.clamp_kitty_keyboard_flags();
        let render_state = RenderState::new()
            .map_err(|err| anyhow::anyhow!("create ghostty render state: {err}"))?;
        let render_hold_active = term.mode(vt::dec_mode(2026));
        let (cols, rows) = (term.cols(), term.rows());
        Ok(Self {
            term,
            render_state: RefCell::new(render_state),
            screen_cache: OnceCell::new(),
            continuation_cache: OnceCell::new(),
            effects: RefCell::new(Vec::new()),
            config: TerminalConfig {
                cols,
                rows,
                ..config
            },
            render_hold_active,
            held_sync_bytes: 0,
        })
    }

    /// Encode this terminal's complete state (both screens, scrollback, modes,
    /// palette, previous char, unfinished sequence continuation) as a binary
    /// `libghostty-vt` snapshot.
    pub fn encode_binary_snapshot(&self) -> Vec<u8> {
        self.term
            .encode_snapshot()
            .expect("encode libghostty-vt snapshot")
    }

    fn invalidate_caches(&mut self) {
        self.screen_cache.take();
        self.continuation_cache.take();
    }

    /// Feed bytes into the terminal.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.advance(bytes);
    }

    /// Advance the terminal parser, tracking `DEC 2026` synchronized update
    /// state and buffering query replies.
    pub fn advance(&mut self, bytes: &[u8]) {
        let _ = self.advance_inner(bytes, false);
    }

    #[cfg(test)]
    pub(crate) fn advance_tracking_scroll(&mut self, bytes: &[u8]) -> Option<usize> {
        self.advance_inner(bytes, true)
    }

    fn advance_inner(&mut self, bytes: &[u8], track_scroll: bool) -> Option<usize> {
        if bytes.is_empty() {
            return Some(0);
        }
        self.invalidate_caches();
        let was_held = self.render_hold_active;
        let moved = if track_scroll {
            crate::terminal::find::ScreenScrollTracker::measure_write(
                &mut self.term,
                bytes,
                |term, slice| term.vt_write(slice),
            )
        } else {
            self.term.vt_write(bytes);
            Some(0)
        };
        self.term.clamp_kitty_keyboard_flags();
        let drained = self.term.take_effects();
        let mut held_now = was_held;
        let mut effects = self.effects.borrow_mut();
        for effect in drained {
            match effect {
                TerminalEffect::RenderHold(held) => {
                    held_now = held;
                    if !held {
                        self.held_sync_bytes = 0;
                    }
                }
                other => effects.push(other),
            }
        }
        self.render_hold_active = held_now;
        if self.render_hold_active {
            if !was_held {
                // Count payload bytes after the last `ESC[?2026h` marker in this chunk.
                let marker = b"\x1b[?2026h";
                let tail_len = bytes
                    .windows(marker.len())
                    .rposition(|w| w == marker)
                    .map_or(bytes.len(), |pos| bytes.len() - (pos + marker.len()));
                self.held_sync_bytes = tail_len;
            } else {
                self.held_sync_bytes = self.held_sync_bytes.saturating_add(bytes.len());
            }
        } else {
            self.held_sync_bytes = 0;
        }
        moved
    }

    /// Whether a `DEC 2026` synchronized update is currently open.
    pub fn is_sync_held(&self) -> bool {
        self.render_hold_active || self.term.mode(vt::dec_mode(2026))
    }

    /// End any active `DEC 2026` synchronized-output hold immediately.
    pub fn stop_sync(&mut self) {
        if self.render_hold_active || self.term.mode(vt::dec_mode(2026)) {
            self.invalidate_caches();
            let _ = self.term.set_mode(vt::dec_mode(2026), false);
            self.render_hold_active = false;
            self.held_sync_bytes = 0;
        }
    }

    pub fn sync_bytes_count(&self) -> usize {
        self.held_sync_bytes
    }

    pub fn preceding_char(&self) -> Option<char> {
        self.term.previous_char()
    }

    /// Seed the terminal's `previous_codepoint` (used by `CSI b` REP) after
    /// restoring a VT-formatted companion snapshot without disturbing cells or
    /// cursor state.
    pub fn restore_preceding_char(&mut self, preceding: Option<char>) {
        if self.term.previous_char() == preceding {
            return;
        }
        if let Ok(mut blob) = self.term.encode_snapshot() {
            if crate::snapshot::patch_snapshot_previous_char(&mut blob, preceding) {
                if let Ok(mut decoded) = vt::Terminal::decode_snapshot(
                    &blob,
                    SnapshotDecodeOptions {
                        max_continuation_bytes: self.config.continuation_max_bytes,
                        retain_continuation: true,
                        compress_history: true,
                        kitty_image_storage_limit: 0,
                    },
                    &self.config,
                ) {
                    let _ = decoded.set_theme_defaults(palette::RUNNER);
                    self.term = decoded;
                    self.invalidate_caches();
                }
            }
        }
    }

    pub fn unfinished(&self) -> &[u8] {
        self.continuation_cache
            .get_or_init(|| self.term.continuation().unwrap_or_default())
            .as_slice()
    }

    pub fn end_sync(&mut self) {
        self.render_hold_active = false;
        self.held_sync_bytes = 0;
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.invalidate_caches();
        let _ = self.term.resize(cols, rows, 0, 0);
        // Drain any RenderHold(false) emitted by ghostty_terminal_resize without
        // disturbing an active synchronized-update marker tracked by tests.
        let drained = self.term.take_effects();
        let mut effects = self.effects.borrow_mut();
        for effect in drained {
            if !matches!(effect, TerminalEffect::RenderHold(_)) {
                effects.push(effect);
            }
        }
    }

    pub fn scroll_top(&mut self) {
        self.invalidate_caches();
        let _ = self.term.scroll_viewport(ScrollViewport::Top);
    }

    pub fn scroll_bottom(&mut self) {
        self.invalidate_caches();
        let _ = self.term.scroll_viewport(ScrollViewport::Bottom);
    }

    #[inline]
    pub fn screen(&self) -> ScreenView<'_> {
        self.screen_cache
            .get_or_init(|| {
                let mut rs = self.render_state.borrow_mut();
                let _ = rs.update(&self.term);
                let cursor = rs.cursor().ok();
                ScreenData::from_terminal_full(&self.term, cursor, false)
            })
            .view()
    }

    #[inline]
    pub fn mode(&self) -> TerminalModeFlags {
        TerminalModeFlags::from_vt(&self.term)
    }

    #[inline]
    pub fn cols(&self) -> usize {
        usize::from(self.term.cols())
    }

    #[inline]
    pub fn rows(&self) -> usize {
        usize::from(self.term.rows())
    }

    #[inline]
    pub fn history_size(&self) -> usize {
        self.term.scrollback_rows()
    }

    #[inline]
    pub fn display_offset(&self) -> usize {
        self.term.display_offset_from_bottom()
    }

    pub fn row_cells(&self, line: i32) -> String {
        self.screen().row_cells(Line(line))
    }

    pub fn row_to_string(&self, line: i32) -> String {
        self.screen().row_to_string(Line(line))
    }

    pub fn visible_lines(&self) -> Vec<String> {
        self.screen().visible_lines()
    }

    pub fn screen_snapshot(&self) -> String {
        self.screen().screen_snapshot()
    }

    pub fn bounds_to_string(&self, start: GridPoint, end: GridPoint) -> String {
        self.screen().bounds_to_string(start, end)
    }

    pub fn snapshot_state(&self) -> TerminalSnapshotState {
        crate::snapshot::inspect_vt(&self.term)
    }

    /// Drain queued query replies, preserving `TerminalReply::ColorQuery`
    /// classification for OSC 4/10/11/12 replies and resolving against `base`
    /// and `theme` when a caller passes a non-default theme.
    pub fn drain_replies(&self, base: &[Rgb; 256], theme: TerminalPalette) -> Vec<TerminalReply> {
        let mut replies = Vec::new();
        for effect in self.effects.borrow_mut().drain(..) {
            if let TerminalEffect::WritePty(bytes) = effect {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                if let Some((index, reply)) = classify_color_reply(&text, self, base, theme) {
                    replies.push(TerminalReply::ColorQuery { index, reply });
                } else {
                    replies.push(TerminalReply::PtyWrite(text));
                }
            }
        }
        replies
    }

    #[inline]
    pub(crate) fn vt(&self) -> &vt::Terminal {
        &self.term
    }
}

fn classify_color_reply(
    text: &str,
    term: &Terminal,
    base: &[Rgb; 256],
    theme: TerminalPalette,
) -> Option<(usize, String)> {
    let body = text.strip_prefix("\x1b]")?;
    let (body, terminator) = if let Some(b) = body.strip_suffix('\x07') {
        (b, "\x07")
    } else {
        (body.strip_suffix("\x1b\\")?, "\x1b\\")
    };
    let (prefix, index) = if let Some(rest) = body.strip_prefix("4;") {
        let (idx_str, payload) = rest.split_once(';')?;
        if !payload.starts_with("rgb:") {
            return None;
        }
        let idx = idx_str.parse::<usize>().ok()?;
        (format!("4;{idx}"), idx)
    } else if body.starts_with("10;rgb:") {
        ("10".to_owned(), 256)
    } else if body.starts_with("11;rgb:") {
        ("11".to_owned(), 257)
    } else if body.starts_with("12;rgb:") {
        ("12".to_owned(), 258)
    } else {
        return None;
    };
    if theme == palette::RUNNER {
        Some((index, text.to_owned()))
    } else {
        let color = crate::terminal::query_color_for(&term.screen(), index, base, theme);
        Some((
            index,
            format!(
                "\x1b]{prefix};rgb:{:02x}{:02x}/{:02x}{:02x}/{:02x}{:02x}{terminator}",
                color.r, color.r, color.g, color.g, color.b, color.b
            ),
        ))
    }
}

pub fn new_term(cols: u16, rows: u16) -> Terminal {
    Terminal::new(cols, rows)
}

pub fn replay_bytes(cols: u16, rows: u16, bytes: &[u8]) -> Terminal {
    let mut term = new_term(cols, rows);
    feed(&mut term, bytes);
    term
}

pub fn replay_fixture(fixture: &Fixture) -> anyhow::Result<Terminal> {
    let mut term = new_term(fixture.header.cols, fixture.header.rows);
    for event in &fixture.events {
        match event {
            FixtureEvent::Data { data, .. } => term.advance(&decode_chunk(data)?),
            FixtureEvent::Resize { cols, rows, .. } => term.resize(*cols, *rows),
            _ => {}
        }
    }
    Ok(term)
}

pub fn feed(term: &mut Terminal, bytes: &[u8]) {
    term.feed(bytes);
}

/// One visible row as a string: wide chars keep their glyph, spacer
/// cells are skipped, zero-width combining marks are re-attached.
pub fn row_to_string(term: &Terminal, line: i32) -> String {
    term.row_to_string(line)
}

/// Preserve trailing space cells for probes that join a wrapped line.
pub fn row_cells(term: &Terminal, line: i32) -> String {
    term.row_cells(line)
}

pub fn visible_lines(term: &Terminal) -> Vec<String> {
    term.visible_lines()
}

/// Deterministic textual snapshot of the visible screen, compared
/// against `.snapshot.txt` files in the fixture corpus.
pub fn screen_snapshot(term: &Terminal) -> String {
    term.screen_snapshot()
}

#[cfg(test)]
mod tests {
    //! Why terminals attach at the PTY's recorded size rather than a
    //! layout estimate: the alternate screen never reflows, so output
    //! replayed into a grid of the wrong width keeps its wrap damage
    //! through every later resize.
    use super::*;
    use crate::palette::{Color, Rgb};
    use crate::types::{Column, GridPoint, Line};

    const GRAY: Color = Color::Rgb(Rgb {
        r: 50,
        g: 50,
        b: 56,
    });

    /// One full-width row as a TUI paints it for a `width`-column PTY:
    /// background-filled cells, then a newline.
    fn painted_row(width: usize) -> Vec<u8> {
        let mut bytes = b"\x1b[48;2;50;50;56m".to_vec();
        bytes.extend(std::iter::repeat_n(b' ', width));
        bytes.extend_from_slice(b"\x1b[0m\r\n");
        bytes
    }

    fn column_zero_painted(term: &Terminal) -> Vec<bool> {
        let screen = term.screen();
        (0..6)
            .map(|line| {
                screen
                    .cell(GridPoint::new(Line(line), Column(0)))
                    .is_some_and(|cell| cell.bg() == GRAY)
            })
            .collect()
    }

    fn alt_screen_replay(grid_cols: u16, pty_cols: usize) -> Terminal {
        let mut term = new_term(grid_cols, 10);
        feed(&mut term, b"\x1b[?1049h");
        for _ in 0..3 {
            feed(&mut term, &painted_row(pty_cols));
        }
        term
    }

    #[test]
    fn alt_screen_replay_narrower_than_the_pty_keeps_its_wrap_tails_after_growing() {
        let mut term = alt_screen_replay(111, 112);
        assert_eq!(
            column_zero_painted(&term),
            [true; 6],
            "every row wrapped its last cell onto the next line"
        );
        term.resize(112, 10);
        assert_eq!(
            column_zero_painted(&term),
            [true; 6],
            "the alt screen does not reflow, so growing cannot rejoin them"
        );
    }

    #[test]
    fn alt_screen_replay_at_the_pty_width_is_clean() {
        let term = alt_screen_replay(112, 112);
        assert_eq!(
            column_zero_painted(&term),
            [true, true, true, false, false, false]
        );
    }
}
