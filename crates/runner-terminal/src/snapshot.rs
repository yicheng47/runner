use std::fmt::Write as _;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::{Cursor, Dimensions, Grid};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{
    CharsetIndex, Color, CursorShape, Handler, KeyboardModes, Processor, StandardCharset,
    StdSyncHandler,
};

#[derive(Default)]
pub struct TrackedProcessor {
    processor: Processor,
    capture: Processor,
    preceding: PrecedingChar,
}

#[derive(Default)]
struct PrecedingChar(Option<char>);

impl Handler for PrecedingChar {
    fn input(&mut self, character: char) {
        self.0 = Some(character);
    }
}

impl TrackedProcessor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn advance<H: Handler>(&mut self, handler: &mut H, bytes: &[u8]) {
        self.processor.advance(handler, bytes);
        self.capture.advance(&mut self.preceding, bytes);
    }

    pub fn stop_sync<H: Handler>(&mut self, handler: &mut H) {
        self.processor.stop_sync(handler);
        self.capture.stop_sync(&mut self.preceding);
    }

    pub fn sync_timeout(&self) -> &StdSyncHandler {
        self.processor.sync_timeout()
    }

    pub fn sync_bytes_count(&self) -> usize {
        self.processor.sync_bytes_count()
    }

    pub fn preceding_char(&self) -> Option<char> {
        self.preceding.0
    }

    pub fn restore_preceding_char(&mut self, preceding: Option<char>) {
        *self = Self::new();
        if let Some(character) = preceding {
            let mut bytes = [0; 4];
            let bytes = character.encode_utf8(&mut bytes).as_bytes();
            // Seed REP without painting over the restored terminal or mapping through its charset.
            self.processor.advance(&mut PrecedingChar::default(), bytes);
            self.capture.advance(&mut self.preceding, bytes);
        }
    }
}

pub fn serialize<T: EventListener>(term: &Term<T>, unfinished: &[u8]) -> Vec<u8> {
    let state = term.snapshot_state();
    let alt = term.mode().contains(TermMode::ALT_SCREEN);
    let (primary, primary_keyboard) = if alt {
        (state.inactive_grid, state.inactive_keyboard_mode_stack)
    } else {
        (term.grid(), state.keyboard_mode_stack)
    };
    let mut out = String::from("\x1bc");
    if !alt {
        out.push_str("\x1b[?1049h");
        cursor(
            &mut out,
            state.inactive_grid,
            &state.inactive_grid.saved_cursor,
            0,
        );
        out.push_str("\x1b7");
        keyboard_stack(&mut out, state.inactive_keyboard_mode_stack);
        out.push_str("\x1b[?1049l");
    }
    grid(&mut out, primary);
    cursor(&mut out, primary, &primary.saved_cursor, 0);
    out.push_str("\x1b7");
    cursor(&mut out, primary, &primary.cursor, 0);
    keyboard_stack(&mut out, primary_keyboard);
    if alt {
        out.push_str("\x1b[?1049h");
        grid(&mut out, term.grid());
        cursor(&mut out, term.grid(), &term.grid().saved_cursor, 0);
        out.push_str("\x1b7");
        keyboard_stack(&mut out, state.keyboard_mode_stack);
    }
    write!(
        out,
        "\x1b[{};{}r",
        state.scroll_region.start.0 + 1,
        state.scroll_region.end.0
    )
    .unwrap();
    for (mode, code, private) in [
        (TermMode::APP_CURSOR, 1, true),
        (TermMode::BRACKETED_PASTE, 2004, true),
        (TermMode::MOUSE_REPORT_CLICK, 1000, true),
        (TermMode::MOUSE_DRAG, 1002, true),
        (TermMode::MOUSE_MOTION, 1003, true),
        (TermMode::SGR_MOUSE, 1006, true),
        (TermMode::UTF8_MOUSE, 1005, true),
        (TermMode::FOCUS_IN_OUT, 1004, true),
        (TermMode::ALTERNATE_SCROLL, 1007, true),
        (TermMode::URGENCY_HINTS, 1042, true),
        (TermMode::ORIGIN, 6, true),
        (TermMode::LINE_WRAP, 7, true),
        (TermMode::SHOW_CURSOR, 25, true),
        (TermMode::INSERT, 4, false),
        (TermMode::LINE_FEED_NEW_LINE, 20, false),
    ] {
        write!(
            out,
            "\x1b[{}{code}{}",
            if private { "?" } else { "" },
            if term.mode().contains(mode) { 'h' } else { 'l' }
        )
        .unwrap();
    }
    out.push_str(if term.mode().contains(TermMode::APP_KEYPAD) {
        "\x1b="
    } else {
        "\x1b>"
    });
    if let Some(style) = state.cursor_style {
        let shape = match style.shape {
            CursorShape::Block | CursorShape::HollowBlock | CursorShape::Hidden => 2,
            CursorShape::Underline => 4,
            CursorShape::Beam => 6,
        } - u8::from(style.blinking);
        write!(out, "\x1b[{shape} q").unwrap();
    }
    let origin = if term.mode().contains(TermMode::ORIGIN) {
        state.scroll_region.start.0
    } else {
        0
    };
    cursor(&mut out, term.grid(), &term.grid().cursor, origin);
    out.push_str("\x1b[3g");
    for (column, enabled) in state.tabs.iter().take(term.columns()).enumerate() {
        if *enabled {
            write!(out, "\x1b[{}G\x1bH", column + 1).unwrap();
        }
    }
    cursor(&mut out, term.grid(), &term.grid().cursor, origin);
    for title in state.title_stack {
        if let Some(title) = title {
            write!(out, "\x1b]2;{title}\x1b\\").unwrap();
        }
        out.push_str("\x1b[22;0t");
    }
    if let Some(title) = state.title {
        write!(out, "\x1b]2;{title}\x1b\\").unwrap();
    }
    for index in 0..259 {
        if let Some(color) = term.colors()[index] {
            let prefix = match index {
                0..=255 => format!("4;{index}"),
                _ => (index - 246).to_string(),
            };
            write!(
                out,
                "\x1b]{prefix};#{:02x}{:02x}{:02x}\x1b\\",
                color.r, color.g, color.b
            )
            .unwrap();
        }
    }
    let flags = KeyboardModes::from_bits_truncate(
        ((term.mode().bits() & TermMode::KITTY_KEYBOARD_PROTOCOL.bits()) >> 18) as u8,
    );
    write!(out, "\x1b[={}u", flags.bits()).unwrap();
    out.push(if *state.active_charset == CharsetIndex::G1 {
        '\x0e'
    } else {
        '\x0f'
    });
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(unfinished);
    bytes
}

fn keyboard_stack(out: &mut String, stack: &[KeyboardModes]) {
    for mode in stack {
        write!(out, "\x1b[>{}u", mode.bits()).unwrap();
    }
}

fn grid(out: &mut String, grid: &Grid<Cell>) {
    out.push_str("\x1b[3g");
    for column in 1..grid.columns() {
        write!(out, "\x1b[{}G\x1bH", column + 1).unwrap();
    }
    out.push_str("\x1b[?7h\x1b[H\x1b[0m");
    let mut previous = None;
    for line in -(grid.history_size() as i32)..grid.screen_lines() as i32 {
        let row = &grid[Line(line)];
        for column in 0..grid.columns() {
            let cell = &row[Column(column)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            let style = (
                cell.fg,
                cell.bg,
                cell.flags & !(Flags::WIDE_CHAR | Flags::WRAPLINE),
                cell.underline_color(),
            );
            if previous != Some(style) {
                sgr(out, cell);
                previous = Some(style);
            }
            if cell.c == '\t' {
                write!(out, " \x1b[{}G\t", column + 1).unwrap();
            } else {
                out.push(cell.c);
            }
            if let Some(marks) = cell.zerowidth() {
                out.extend(marks);
            }
        }
        if line + 1 < grid.screen_lines() as i32
            && !row[Column(grid.columns() - 1)]
                .flags
                .contains(Flags::WRAPLINE)
        {
            out.push_str("\r\n");
        }
    }
}

fn cursor(out: &mut String, grid: &Grid<Cell>, cursor: &Cursor<Cell>, origin: i32) {
    out.push_str("\x0f\x1b(B");
    write!(
        out,
        "\x1b[{};{}H",
        cursor.point.line.0 - origin + 1,
        cursor.point.column.0 + 1
    )
    .unwrap();
    if cursor.input_needs_wrap {
        let mut point = cursor.point;
        if grid[point].flags.contains(Flags::WIDE_CHAR_SPACER) {
            point.column.0 -= 1;
            write!(out, "\x1b[{}G", point.column.0 + 1).unwrap();
        }
        let cell = &grid[point];
        sgr(out, cell);
        out.push(cell.c);
        if let Some(marks) = cell.zerowidth() {
            out.extend(marks);
        }
    }
    sgr(out, &cursor.template);
    for (index, introducer) in [
        CharsetIndex::G0,
        CharsetIndex::G1,
        CharsetIndex::G2,
        CharsetIndex::G3,
    ]
    .into_iter()
    .zip(['(', ')', '*', '+'])
    {
        let code = match cursor.charsets[index] {
            StandardCharset::Ascii => 'B',
            StandardCharset::SpecialCharacterAndLineDrawing => '0',
        };
        write!(out, "\x1b{introducer}{code}").unwrap();
    }
}

fn sgr(out: &mut String, cell: &Cell) {
    out.push_str("\x1b[0");
    for (flag, code) in [
        (Flags::BOLD, "1"),
        (Flags::DIM, "2"),
        (Flags::ITALIC, "3"),
        (Flags::UNDERLINE, "4"),
        (Flags::DOUBLE_UNDERLINE, "4:2"),
        (Flags::UNDERCURL, "4:3"),
        (Flags::DOTTED_UNDERLINE, "4:4"),
        (Flags::DASHED_UNDERLINE, "4:5"),
        (Flags::INVERSE, "7"),
        (Flags::HIDDEN, "8"),
        (Flags::STRIKEOUT, "9"),
    ] {
        if cell.flags.contains(flag) {
            write!(out, ";{code}").unwrap();
        }
    }
    color(out, cell.fg, 38);
    color(out, cell.bg, 48);
    if let Some(underline) = cell.underline_color() {
        color(out, underline, 58);
    }
    out.push('m');
}

fn color(out: &mut String, color: Color, channel: u8) {
    match color {
        Color::Spec(rgb) => {
            write!(out, ";{channel};2;{};{};{}", rgb.r, rgb.g, rgb.b).unwrap();
        }
        Color::Indexed(index) => {
            write!(out, ";{channel};5;{index}").unwrap();
        }
        Color::Named(named) => {
            let index = named as usize;
            let code = match index {
                0..=7 => (if channel == 38 { 30 } else { 40 }) + index,
                8..=15 => (if channel == 38 { 90 } else { 100 }) + index - 8,
                _ => {
                    if channel == 38 {
                        39
                    } else {
                        49
                    }
                }
            };
            write!(out, ";{code}").unwrap();
        }
    }
}

#[derive(Clone, Copy, Default)]
enum Boundary {
    #[default]
    Ground,
    Escape,
    Csi,
    Osc,
    DcsHeader,
    String,
    StringEscape,
    Utf8(u8),
}

#[derive(Default)]
pub struct BoundaryScanner {
    state: Boundary,
    pending: Vec<u8>,
    synchronized: Option<Vec<u8>>,
}

const SYNC_BEGIN: &[u8] = b"\x1b[?2026h";

impl BoundaryScanner {
    pub fn advance(&mut self, bytes: &[u8], held_bytes: Option<usize>) {
        if let Some(held) = held_bytes {
            let sync = self.synchronized.get_or_insert_with(|| SYNC_BEGIN.to_vec());
            sync.extend_from_slice(bytes);
            // vte can flush and restart inside one advance; only its held suffix is still pending.
            let start = sync.len() - held;
            sync.copy_within(start.., SYNC_BEGIN.len());
            sync.truncate(SYNC_BEGIN.len() + held);
        } else {
            self.end_sync();
        }
        for &byte in bytes {
            self.byte(byte);
        }
    }

    pub fn unfinished(&self) -> &[u8] {
        self.synchronized.as_deref().unwrap_or(&self.pending)
    }

    pub fn end_sync(&mut self) {
        self.synchronized = None;
    }

    fn byte(&mut self, byte: u8) {
        if matches!(self.state, Boundary::Utf8(_)) && !(0x80..=0xbf).contains(&byte) {
            self.pending.clear();
            self.state = Boundary::Ground;
        }
        if matches!(byte, 0x18 | 0x1a) {
            self.pending.clear();
            self.state = Boundary::Ground;
            return;
        }
        if matches!(
            self.state,
            Boundary::Escape | Boundary::Csi | Boundary::DcsHeader
        ) && matches!(byte, 0..=0x1f | 0x7f)
            && byte != 0x1b
        {
            return;
        }
        match self.state {
            Boundary::Ground => {
                self.state = match byte {
                    0x1b => Boundary::Escape,
                    0xc2..=0xdf => Boundary::Utf8(1),
                    0xe0..=0xef => Boundary::Utf8(2),
                    0xf0..=0xf4 => Boundary::Utf8(3),
                    _ => return,
                };
                self.pending.push(byte);
            }
            Boundary::Escape => {
                if byte == 0x1b {
                    self.pending.clear();
                }
                self.pending.push(byte);
                self.state = match byte {
                    b'[' => Boundary::Csi,
                    b']' => Boundary::Osc,
                    b'P' => Boundary::DcsHeader,
                    b'X' | b'^' | b'_' => Boundary::String,
                    0..=0x2f => Boundary::Escape,
                    _ => Boundary::Ground,
                };
            }
            Boundary::Csi | Boundary::DcsHeader => {
                self.pending.push(byte);
                if byte == 0x1b {
                    self.pending.clear();
                    self.pending.push(byte);
                    self.state = Boundary::Escape;
                } else if (0x40..=0x7e).contains(&byte) {
                    if matches!(self.state, Boundary::DcsHeader) {
                        self.state = Boundary::String;
                    } else {
                        self.state = Boundary::Ground;
                    }
                }
            }
            Boundary::Osc | Boundary::String => {
                self.pending.push(byte);
                if byte == 0x1b {
                    self.state = Boundary::StringEscape;
                } else if byte == 7 && matches!(self.state, Boundary::Osc) {
                    self.state = Boundary::Ground;
                }
            }
            Boundary::StringEscape => {
                self.pending.push(byte);
                if byte == b'\\' {
                    self.state = Boundary::Ground;
                } else {
                    self.pending.clear();
                    self.pending.push(0x1b);
                    self.state = Boundary::Escape;
                    self.byte(byte);
                }
            }
            Boundary::Utf8(left) => {
                self.pending.push(byte);
                self.state = if left == 1 {
                    Boundary::Ground
                } else {
                    Boundary::Utf8(left - 1)
                };
            }
        }
        if matches!(self.state, Boundary::Ground) {
            self.pending.clear();
        }
    }
}
