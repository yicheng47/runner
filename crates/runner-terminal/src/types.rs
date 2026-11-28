//! Runner-owned terminal types shared across the `runner-terminal` boundary.

use std::fmt;
use std::ops::{Add, AddAssign, BitAnd, BitAndAssign, BitOr, BitOrAssign, Not, Sub, SubAssign};
use std::sync::Arc;

use crate::vt::{self, CellWide, UnderlineStyle};

/// A grid line index: `0..rows` on the visible screen and negative lines in scrollback history.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Line(pub i32);

impl Add<i32> for Line {
    type Output = Self;

    #[inline]
    fn add(self, rhs: i32) -> Self::Output {
        Self(self.0 + rhs)
    }
}

impl Sub<i32> for Line {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: i32) -> Self::Output {
        Self(self.0 - rhs)
    }
}

impl Add<usize> for Line {
    type Output = Self;

    #[inline]
    fn add(self, rhs: usize) -> Self::Output {
        Self(self.0 + rhs as i32)
    }
}

impl Sub<usize> for Line {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: usize) -> Self::Output {
        Self(self.0 - rhs as i32)
    }
}

impl AddAssign<i32> for Line {
    #[inline]
    fn add_assign(&mut self, rhs: i32) {
        self.0 += rhs;
    }
}

impl SubAssign<i32> for Line {
    #[inline]
    fn sub_assign(&mut self, rhs: i32) {
        self.0 -= rhs;
    }
}

/// A zero-based column index in the terminal grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Column(pub usize);

impl Add<usize> for Column {
    type Output = Self;

    #[inline]
    fn add(self, rhs: usize) -> Self::Output {
        Self(self.0 + rhs)
    }
}

impl Sub<usize> for Column {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: usize) -> Self::Output {
        Self(self.0 - rhs)
    }
}

impl AddAssign<usize> for Column {
    #[inline]
    fn add_assign(&mut self, rhs: usize) {
        self.0 += rhs;
    }
}

impl SubAssign<usize> for Column {
    #[inline]
    fn sub_assign(&mut self, rhs: usize) {
        self.0 -= rhs;
    }
}

/// A cell coordinate in the terminal grid (including scrollback lines).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GridPoint {
    pub line: Line,
    pub column: Column,
}

impl GridPoint {
    #[inline]
    pub const fn new(line: Line, column: Column) -> Self {
        Self { line, column }
    }
}

/// A cell coordinate relative to the current viewport (`0..rows`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ViewportPoint {
    pub line: usize,
    pub column: Column,
}

impl ViewportPoint {
    #[inline]
    pub const fn new(line: usize, column: Column) -> Self {
        Self { line, column }
    }
}

/// Convert a grid point to a viewport-relative point at `display_offset`.
#[inline]
pub fn point_to_viewport(display_offset: usize, point: GridPoint) -> Option<ViewportPoint> {
    let viewport_line = point.line.0 + display_offset as i32;
    usize::try_from(viewport_line)
        .ok()
        .map(|line| ViewportPoint::new(line, point.column))
}

/// Convert a viewport-relative point at `display_offset` to a grid point.
#[inline]
pub fn viewport_to_point(display_offset: usize, point: ViewportPoint) -> GridPoint {
    let line = Line(point.line as i32) - display_offset;
    GridPoint::new(line, point.column)
}

/// Which half of a cell a pointer gesture targeted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
}

/// Visual shape of the terminal cursor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CursorShape {
    #[default]
    Block,
    HollowBlock,
    Underline,
    Beam,
    Hidden,
}

/// Configured or DECSCUSR cursor shape and blinking state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CursorStyle {
    pub shape: CursorShape,
    pub blinking: bool,
}

/// Cursor geometry and visibility for rendering.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RenderCursor {
    pub point: GridPoint,
    pub shape: CursorShape,
    pub blinking: bool,
}

/// Kind of text selection gesture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SelectionKind {
    Simple,
    Block,
    Semantic,
    Lines,
}

/// Active selection span in grid coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SelectionRange {
    pub start: GridPoint,
    pub end: GridPoint,
    pub is_block: bool,
}

impl SelectionRange {
    #[inline]
    pub fn contains(&self, point: GridPoint) -> bool {
        self.start.line <= point.line
            && self.end.line >= point.line
            && (self.start.column <= point.column
                || (self.start.line != point.line && !self.is_block))
            && (self.end.column >= point.column || (self.end.line != point.line && !self.is_block))
    }
}

/// Inclusive grid span of a terminal find match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FindMatch {
    pub start: GridPoint,
    pub end: GridPoint,
}

impl FindMatch {
    #[inline]
    pub const fn new(start: GridPoint, end: GridPoint) -> Self {
        Self { start, end }
    }

    #[inline]
    pub const fn start(&self) -> &GridPoint {
        &self.start
    }

    #[inline]
    pub const fn end(&self) -> &GridPoint {
        &self.end
    }

    #[inline]
    pub fn contains(&self, point: &GridPoint) -> bool {
        self.start <= *point && *point <= self.end
    }
}

/// OSC 8 hyperlink attached to a cell.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Hyperlink {
    id: Arc<str>,
    uri: Arc<str>,
}

impl Hyperlink {
    pub fn new(id: impl Into<Arc<str>>, uri: impl Into<Arc<str>>) -> Self {
        Self {
            id: id.into(),
            uri: uri.into(),
        }
    }

    #[inline]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[inline]
    pub fn uri(&self) -> &str {
        &self.uri
    }
}

/// Per-cell text style and layout flags.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct CellFlags(u16);

impl CellFlags {
    pub const INVERSE: Self = Self(1 << 0);
    pub const BOLD: Self = Self(1 << 1);
    pub const ITALIC: Self = Self(1 << 2);
    pub const UNDERLINE: Self = Self(1 << 3);
    pub const WRAPLINE: Self = Self(1 << 4);
    pub const WIDE_CHAR: Self = Self(1 << 5);
    pub const WIDE_CHAR_SPACER: Self = Self(1 << 6);
    pub const DIM: Self = Self(1 << 7);
    pub const HIDDEN: Self = Self(1 << 8);
    pub const STRIKEOUT: Self = Self(1 << 9);
    pub const LEADING_WIDE_CHAR_SPACER: Self = Self(1 << 10);
    pub const DOUBLE_UNDERLINE: Self = Self(1 << 11);
    pub const UNDERCURL: Self = Self(1 << 12);
    pub const DOTTED_UNDERLINE: Self = Self(1 << 13);
    pub const DASHED_UNDERLINE: Self = Self(1 << 14);
    pub const ALL_UNDERLINES: Self = Self(
        Self::UNDERLINE.0
            | Self::DOUBLE_UNDERLINE.0
            | Self::UNDERCURL.0
            | Self::DOTTED_UNDERLINE.0
            | Self::DASHED_UNDERLINE.0,
    );

    const ALL_BITS: u16 = (1 << 15) - 1;

    #[inline]
    pub const fn empty() -> Self {
        Self(0)
    }

    #[inline]
    pub const fn bits(self) -> u16 {
        self.0
    }

    #[inline]
    pub const fn from_bits_truncate(bits: u16) -> Self {
        Self(bits & Self::ALL_BITS)
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[inline]
    pub const fn intersects(self, other: Self) -> bool {
        (self.0 & other.0) != 0
    }

    #[inline]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[inline]
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    #[inline]
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }

    #[inline]
    pub(crate) fn from_vt_style(style_flags: u16, wide: CellWide, wrapline: bool) -> Self {
        let mut out = 0u16;
        if style_flags & vt::STYLE_FLAG_INVERSE != 0 {
            out |= Self::INVERSE.0;
        }
        if style_flags & vt::STYLE_FLAG_BOLD != 0 {
            out |= Self::BOLD.0;
        }
        if style_flags & vt::STYLE_FLAG_ITALIC != 0 {
            out |= Self::ITALIC.0;
        }
        if style_flags & vt::STYLE_FLAG_FAINT != 0 {
            out |= Self::DIM.0;
        }
        if style_flags & vt::STYLE_FLAG_INVISIBLE != 0 {
            out |= Self::HIDDEN.0;
        }
        if style_flags & vt::STYLE_FLAG_STRIKETHROUGH != 0 {
            out |= Self::STRIKEOUT.0;
        }
        match vt::style_underline(style_flags) {
            UnderlineStyle::None => {}
            UnderlineStyle::Single => out |= Self::UNDERLINE.0,
            UnderlineStyle::Double => out |= Self::DOUBLE_UNDERLINE.0,
            UnderlineStyle::Curly => out |= Self::UNDERCURL.0,
            UnderlineStyle::Dotted => out |= Self::DOTTED_UNDERLINE.0,
            UnderlineStyle::Dashed => out |= Self::DASHED_UNDERLINE.0,
        }
        match wide {
            CellWide::Narrow => {}
            CellWide::Wide => out |= Self::WIDE_CHAR.0,
            CellWide::SpacerTail => out |= Self::WIDE_CHAR_SPACER.0,
            CellWide::SpacerHead => out |= Self::LEADING_WIDE_CHAR_SPACER.0,
        }
        if wrapline {
            out |= Self::WRAPLINE.0;
        }
        Self(out)
    }
}

impl BitOr for CellFlags {
    type Output = Self;

    #[inline]
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for CellFlags {
    #[inline]
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitAnd for CellFlags {
    type Output = Self;

    #[inline]
    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

impl BitAndAssign for CellFlags {
    #[inline]
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}

impl Not for CellFlags {
    type Output = Self;

    #[inline]
    fn not(self) -> Self::Output {
        Self(!self.0 & Self::ALL_BITS)
    }
}

impl fmt::Debug for CellFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = [
            (Self::INVERSE, "INVERSE"),
            (Self::BOLD, "BOLD"),
            (Self::ITALIC, "ITALIC"),
            (Self::UNDERLINE, "UNDERLINE"),
            (Self::WRAPLINE, "WRAPLINE"),
            (Self::WIDE_CHAR, "WIDE_CHAR"),
            (Self::WIDE_CHAR_SPACER, "WIDE_CHAR_SPACER"),
            (Self::DIM, "DIM"),
            (Self::HIDDEN, "HIDDEN"),
            (Self::STRIKEOUT, "STRIKEOUT"),
            (Self::LEADING_WIDE_CHAR_SPACER, "LEADING_WIDE_CHAR_SPACER"),
            (Self::DOUBLE_UNDERLINE, "DOUBLE_UNDERLINE"),
            (Self::UNDERCURL, "UNDERCURL"),
            (Self::DOTTED_UNDERLINE, "DOTTED_UNDERLINE"),
            (Self::DASHED_UNDERLINE, "DASHED_UNDERLINE"),
        ];
        write!(f, "CellFlags(")?;
        let mut first = true;
        for (flag, name) in names {
            if self.contains(flag) {
                if !first {
                    write!(f, " | ")?;
                }
                write!(f, "{name}")?;
                first = false;
            }
        }
        if first {
            write!(f, "empty")?;
        }
        write!(f, ")")
    }
}

/// Active terminal modes exposed across `runner-terminal`'s boundary.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct TerminalModeFlags(u32);

impl TerminalModeFlags {
    pub const NONE: Self = Self(0);
    pub const SHOW_CURSOR: Self = Self(1 << 0);
    pub const APP_CURSOR: Self = Self(1 << 1);
    pub const APP_KEYPAD: Self = Self(1 << 2);
    pub const MOUSE_REPORT_CLICK: Self = Self(1 << 3);
    pub const BRACKETED_PASTE: Self = Self(1 << 4);
    pub const SGR_MOUSE: Self = Self(1 << 5);
    pub const MOUSE_MOTION: Self = Self(1 << 6);
    pub const LINE_WRAP: Self = Self(1 << 7);
    pub const LINE_FEED_NEW_LINE: Self = Self(1 << 8);
    pub const ORIGIN: Self = Self(1 << 9);
    pub const INSERT: Self = Self(1 << 10);
    pub const FOCUS_IN_OUT: Self = Self(1 << 11);
    pub const ALT_SCREEN: Self = Self(1 << 12);
    pub const MOUSE_DRAG: Self = Self(1 << 13);
    pub const UTF8_MOUSE: Self = Self(1 << 14);
    pub const ALTERNATE_SCROLL: Self = Self(1 << 15);
    pub const URGENCY_HINTS: Self = Self(1 << 16);
    pub const DISAMBIGUATE_ESC_CODES: Self = Self(1 << 17);
    pub const REPORT_EVENT_TYPES: Self = Self(1 << 18);
    pub const REPORT_ALTERNATE_KEYS: Self = Self(1 << 19);
    pub const REPORT_ALL_KEYS_AS_ESC: Self = Self(1 << 20);
    pub const REPORT_ASSOCIATED_TEXT: Self = Self(1 << 21);
    pub const MOUSE_MODE: Self =
        Self(Self::MOUSE_REPORT_CLICK.0 | Self::MOUSE_MOTION.0 | Self::MOUSE_DRAG.0);
    pub const KITTY_KEYBOARD_PROTOCOL: Self = Self(
        Self::DISAMBIGUATE_ESC_CODES.0
            | Self::REPORT_EVENT_TYPES.0
            | Self::REPORT_ALTERNATE_KEYS.0
            | Self::REPORT_ALL_KEYS_AS_ESC.0
            | Self::REPORT_ASSOCIATED_TEXT.0,
    );

    const ALL_BITS: u32 = (1 << 22) - 1;

    #[inline]
    pub const fn empty() -> Self {
        Self(0)
    }

    #[inline]
    pub const fn bits(self) -> u32 {
        self.0
    }

    #[inline]
    pub const fn from_bits_truncate(bits: u32) -> Self {
        Self(bits & Self::ALL_BITS)
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[inline]
    pub const fn intersects(self, other: Self) -> bool {
        (self.0 & other.0) != 0
    }

    #[inline]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[inline]
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    #[inline]
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }

    pub(crate) fn from_vt(term: &vt::Terminal) -> Self {
        let mut out = 0u32;
        if term.cursor_visible() {
            out |= Self::SHOW_CURSOR.0;
        }
        if term.mode(vt::dec_mode(1)) {
            out |= Self::APP_CURSOR.0;
        }
        if term.mode(vt::dec_mode(66)) {
            out |= Self::APP_KEYPAD.0;
        }
        if term.mode(vt::dec_mode(1000)) || term.mode(vt::dec_mode(9)) {
            out |= Self::MOUSE_REPORT_CLICK.0;
        }
        if term.mode(vt::dec_mode(2004)) {
            out |= Self::BRACKETED_PASTE.0;
        }
        if term.mode(vt::dec_mode(1006)) {
            out |= Self::SGR_MOUSE.0;
        }
        if term.mode(vt::dec_mode(1003)) {
            out |= Self::MOUSE_MOTION.0;
        }
        if term.mode(vt::dec_mode(7)) {
            out |= Self::LINE_WRAP.0;
        }
        if term.mode(vt::ansi_mode(20)) {
            out |= Self::LINE_FEED_NEW_LINE.0;
        }
        if term.mode(vt::dec_mode(6)) {
            out |= Self::ORIGIN.0;
        }
        if term.mode(vt::ansi_mode(4)) {
            out |= Self::INSERT.0;
        }
        if term.mode(vt::dec_mode(1004)) {
            out |= Self::FOCUS_IN_OUT.0;
        }
        if term.active_screen() == vt::ActiveScreen::Alternate {
            out |= Self::ALT_SCREEN.0;
        }
        if term.mode(vt::dec_mode(1002)) {
            out |= Self::MOUSE_DRAG.0;
        }
        if term.mode(vt::dec_mode(1005)) {
            out |= Self::UTF8_MOUSE.0;
        }
        if term.mode(vt::dec_mode(1007)) {
            out |= Self::ALTERNATE_SCROLL.0;
        }
        // Phase 1 supports only the Kitty disambiguation bit (bit 0); releases,
        // repeats, alternate keys and associated text wait for key-up wiring.
        let kitty = term.kitty_keyboard_flags() & 1;
        if kitty & (1 << 0) != 0 {
            out |= Self::DISAMBIGUATE_ESC_CODES.0;
        }
        Self(out)
    }

    #[inline]
    pub(crate) fn kitty_keyboard_bits(self) -> u8 {
        let mut bits = 0u8;
        if self.contains(Self::DISAMBIGUATE_ESC_CODES) {
            bits |= 1 << 0;
        }
        if self.contains(Self::REPORT_EVENT_TYPES) {
            bits |= 1 << 1;
        }
        if self.contains(Self::REPORT_ALTERNATE_KEYS) {
            bits |= 1 << 2;
        }
        if self.contains(Self::REPORT_ALL_KEYS_AS_ESC) {
            bits |= 1 << 3;
        }
        if self.contains(Self::REPORT_ASSOCIATED_TEXT) {
            bits |= 1 << 4;
        }
        bits
    }
}

impl BitOr for TerminalModeFlags {
    type Output = Self;

    #[inline]
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for TerminalModeFlags {
    #[inline]
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitAnd for TerminalModeFlags {
    type Output = Self;

    #[inline]
    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

impl BitAndAssign for TerminalModeFlags {
    #[inline]
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}

impl Not for TerminalModeFlags {
    type Output = Self;

    #[inline]
    fn not(self) -> Self::Output {
        Self(!self.0 & Self::ALL_BITS)
    }
}

impl fmt::Debug for TerminalModeFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = [
            (Self::SHOW_CURSOR, "SHOW_CURSOR"),
            (Self::APP_CURSOR, "APP_CURSOR"),
            (Self::APP_KEYPAD, "APP_KEYPAD"),
            (Self::MOUSE_REPORT_CLICK, "MOUSE_REPORT_CLICK"),
            (Self::BRACKETED_PASTE, "BRACKETED_PASTE"),
            (Self::SGR_MOUSE, "SGR_MOUSE"),
            (Self::MOUSE_MOTION, "MOUSE_MOTION"),
            (Self::LINE_WRAP, "LINE_WRAP"),
            (Self::LINE_FEED_NEW_LINE, "LINE_FEED_NEW_LINE"),
            (Self::ORIGIN, "ORIGIN"),
            (Self::INSERT, "INSERT"),
            (Self::FOCUS_IN_OUT, "FOCUS_IN_OUT"),
            (Self::ALT_SCREEN, "ALT_SCREEN"),
            (Self::MOUSE_DRAG, "MOUSE_DRAG"),
            (Self::UTF8_MOUSE, "UTF8_MOUSE"),
            (Self::ALTERNATE_SCROLL, "ALTERNATE_SCROLL"),
            (Self::URGENCY_HINTS, "URGENCY_HINTS"),
            (Self::DISAMBIGUATE_ESC_CODES, "DISAMBIGUATE_ESC_CODES"),
            (Self::REPORT_EVENT_TYPES, "REPORT_EVENT_TYPES"),
            (Self::REPORT_ALTERNATE_KEYS, "REPORT_ALTERNATE_KEYS"),
            (Self::REPORT_ALL_KEYS_AS_ESC, "REPORT_ALL_KEYS_AS_ESC"),
            (Self::REPORT_ASSOCIATED_TEXT, "REPORT_ASSOCIATED_TEXT"),
        ];
        write!(f, "TerminalModeFlags(")?;
        let mut first = true;
        for (flag, name) in names {
            if self.contains(flag) {
                if !first {
                    write!(f, " | ")?;
                }
                write!(f, "{name}")?;
                first = false;
            }
        }
        if first {
            write!(f, "NONE")?;
        }
        write!(f, ")")
    }
}
