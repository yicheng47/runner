//! Runner-owned terminal types shared across the `runner-terminal` boundary.
//! Shaped to map cleanly onto both `alacritty_terminal` and `libghostty-vt`.

use std::fmt;
use std::ops::{Add, AddAssign, BitAnd, BitAndAssign, BitOr, BitOrAssign, Not, Sub, SubAssign};

use alacritty_terminal::index as alacritty_index;
use alacritty_terminal::selection as alacritty_selection;
use alacritty_terminal::term::cell as alacritty_cell;
use alacritty_terminal::term::TermMode as AlacrittyTermMode;
use alacritty_terminal::vte::ansi as alacritty_ansi;

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

impl Line {
    #[inline]
    pub(crate) fn from_alacritty(line: alacritty_index::Line) -> Self {
        Self(line.0)
    }

    #[inline]
    pub(crate) fn to_alacritty(self) -> alacritty_index::Line {
        alacritty_index::Line(self.0)
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

impl Column {
    #[inline]
    pub(crate) fn from_alacritty(column: alacritty_index::Column) -> Self {
        Self(column.0)
    }

    #[inline]
    pub(crate) fn to_alacritty(self) -> alacritty_index::Column {
        alacritty_index::Column(self.0)
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

    #[inline]
    pub(crate) fn from_alacritty(point: alacritty_index::Point) -> Self {
        Self {
            line: Line::from_alacritty(point.line),
            column: Column::from_alacritty(point.column),
        }
    }

    #[inline]
    pub(crate) fn to_alacritty(self) -> alacritty_index::Point {
        alacritty_index::Point::new(self.line.to_alacritty(), self.column.to_alacritty())
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

impl Side {
    #[inline]
    pub(crate) fn to_alacritty(self) -> alacritty_index::Side {
        match self {
            Self::Left => alacritty_index::Side::Left,
            Self::Right => alacritty_index::Side::Right,
        }
    }
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

impl CursorShape {
    #[inline]
    pub(crate) fn from_alacritty(shape: alacritty_ansi::CursorShape) -> Self {
        match shape {
            alacritty_ansi::CursorShape::Block => Self::Block,
            alacritty_ansi::CursorShape::HollowBlock => Self::HollowBlock,
            alacritty_ansi::CursorShape::Underline => Self::Underline,
            alacritty_ansi::CursorShape::Beam => Self::Beam,
            alacritty_ansi::CursorShape::Hidden => Self::Hidden,
        }
    }

    #[inline]
    pub(crate) fn to_alacritty(self) -> alacritty_ansi::CursorShape {
        match self {
            Self::Block => alacritty_ansi::CursorShape::Block,
            Self::HollowBlock => alacritty_ansi::CursorShape::HollowBlock,
            Self::Underline => alacritty_ansi::CursorShape::Underline,
            Self::Beam => alacritty_ansi::CursorShape::Beam,
            Self::Hidden => alacritty_ansi::CursorShape::Hidden,
        }
    }
}

/// Configured or DECSCUSR cursor shape and blinking state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CursorStyle {
    pub shape: CursorShape,
    pub blinking: bool,
}

impl CursorStyle {
    #[inline]
    pub(crate) fn from_alacritty(style: alacritty_ansi::CursorStyle) -> Self {
        Self {
            shape: CursorShape::from_alacritty(style.shape),
            blinking: style.blinking,
        }
    }
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

impl SelectionKind {
    #[inline]
    pub(crate) fn from_alacritty(ty: alacritty_selection::SelectionType) -> Self {
        match ty {
            alacritty_selection::SelectionType::Simple => Self::Simple,
            alacritty_selection::SelectionType::Block => Self::Block,
            alacritty_selection::SelectionType::Semantic => Self::Semantic,
            alacritty_selection::SelectionType::Lines => Self::Lines,
        }
    }

    #[inline]
    pub(crate) fn to_alacritty(self) -> alacritty_selection::SelectionType {
        match self {
            Self::Simple => alacritty_selection::SelectionType::Simple,
            Self::Block => alacritty_selection::SelectionType::Block,
            Self::Semantic => alacritty_selection::SelectionType::Semantic,
            Self::Lines => alacritty_selection::SelectionType::Lines,
        }
    }
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

    #[inline]
    pub(crate) fn from_alacritty(range: alacritty_selection::SelectionRange) -> Self {
        Self {
            start: GridPoint::from_alacritty(range.start),
            end: GridPoint::from_alacritty(range.end),
            is_block: range.is_block,
        }
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

    #[inline]
    pub(crate) fn from_alacritty(matched: alacritty_terminal::term::search::Match) -> Self {
        Self {
            start: GridPoint::from_alacritty(*matched.start()),
            end: GridPoint::from_alacritty(*matched.end()),
        }
    }
}

/// OSC 8 hyperlink attached to a cell.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Hyperlink {
    inner: alacritty_cell::Hyperlink,
}

impl Hyperlink {
    #[inline]
    pub fn id(&self) -> &str {
        self.inner.id()
    }

    #[inline]
    pub fn uri(&self) -> &str {
        self.inner.uri()
    }

    #[inline]
    pub(crate) fn from_alacritty(inner: alacritty_cell::Hyperlink) -> Self {
        Self { inner }
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
    pub(crate) fn from_alacritty(flags: alacritty_cell::Flags) -> Self {
        let mut out = 0u16;
        if flags.contains(alacritty_cell::Flags::INVERSE) {
            out |= Self::INVERSE.0;
        }
        if flags.contains(alacritty_cell::Flags::BOLD) {
            out |= Self::BOLD.0;
        }
        if flags.contains(alacritty_cell::Flags::ITALIC) {
            out |= Self::ITALIC.0;
        }
        if flags.contains(alacritty_cell::Flags::UNDERLINE) {
            out |= Self::UNDERLINE.0;
        }
        if flags.contains(alacritty_cell::Flags::WRAPLINE) {
            out |= Self::WRAPLINE.0;
        }
        if flags.contains(alacritty_cell::Flags::WIDE_CHAR) {
            out |= Self::WIDE_CHAR.0;
        }
        if flags.contains(alacritty_cell::Flags::WIDE_CHAR_SPACER) {
            out |= Self::WIDE_CHAR_SPACER.0;
        }
        if flags.contains(alacritty_cell::Flags::DIM) {
            out |= Self::DIM.0;
        }
        if flags.contains(alacritty_cell::Flags::HIDDEN) {
            out |= Self::HIDDEN.0;
        }
        if flags.contains(alacritty_cell::Flags::STRIKEOUT) {
            out |= Self::STRIKEOUT.0;
        }
        if flags.contains(alacritty_cell::Flags::LEADING_WIDE_CHAR_SPACER) {
            out |= Self::LEADING_WIDE_CHAR_SPACER.0;
        }
        if flags.contains(alacritty_cell::Flags::DOUBLE_UNDERLINE) {
            out |= Self::DOUBLE_UNDERLINE.0;
        }
        if flags.contains(alacritty_cell::Flags::UNDERCURL) {
            out |= Self::UNDERCURL.0;
        }
        if flags.contains(alacritty_cell::Flags::DOTTED_UNDERLINE) {
            out |= Self::DOTTED_UNDERLINE.0;
        }
        if flags.contains(alacritty_cell::Flags::DASHED_UNDERLINE) {
            out |= Self::DASHED_UNDERLINE.0;
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

    #[inline]
    pub(crate) fn from_alacritty(mode: AlacrittyTermMode) -> Self {
        let mut out = 0u32;
        if mode.contains(AlacrittyTermMode::SHOW_CURSOR) {
            out |= Self::SHOW_CURSOR.0;
        }
        if mode.contains(AlacrittyTermMode::APP_CURSOR) {
            out |= Self::APP_CURSOR.0;
        }
        if mode.contains(AlacrittyTermMode::APP_KEYPAD) {
            out |= Self::APP_KEYPAD.0;
        }
        if mode.contains(AlacrittyTermMode::MOUSE_REPORT_CLICK) {
            out |= Self::MOUSE_REPORT_CLICK.0;
        }
        if mode.contains(AlacrittyTermMode::BRACKETED_PASTE) {
            out |= Self::BRACKETED_PASTE.0;
        }
        if mode.contains(AlacrittyTermMode::SGR_MOUSE) {
            out |= Self::SGR_MOUSE.0;
        }
        if mode.contains(AlacrittyTermMode::MOUSE_MOTION) {
            out |= Self::MOUSE_MOTION.0;
        }
        if mode.contains(AlacrittyTermMode::LINE_WRAP) {
            out |= Self::LINE_WRAP.0;
        }
        if mode.contains(AlacrittyTermMode::LINE_FEED_NEW_LINE) {
            out |= Self::LINE_FEED_NEW_LINE.0;
        }
        if mode.contains(AlacrittyTermMode::ORIGIN) {
            out |= Self::ORIGIN.0;
        }
        if mode.contains(AlacrittyTermMode::INSERT) {
            out |= Self::INSERT.0;
        }
        if mode.contains(AlacrittyTermMode::FOCUS_IN_OUT) {
            out |= Self::FOCUS_IN_OUT.0;
        }
        if mode.contains(AlacrittyTermMode::ALT_SCREEN) {
            out |= Self::ALT_SCREEN.0;
        }
        if mode.contains(AlacrittyTermMode::MOUSE_DRAG) {
            out |= Self::MOUSE_DRAG.0;
        }
        if mode.contains(AlacrittyTermMode::UTF8_MOUSE) {
            out |= Self::UTF8_MOUSE.0;
        }
        if mode.contains(AlacrittyTermMode::ALTERNATE_SCROLL) {
            out |= Self::ALTERNATE_SCROLL.0;
        }
        if mode.contains(AlacrittyTermMode::URGENCY_HINTS) {
            out |= Self::URGENCY_HINTS.0;
        }
        if mode.contains(AlacrittyTermMode::DISAMBIGUATE_ESC_CODES) {
            out |= Self::DISAMBIGUATE_ESC_CODES.0;
        }
        if mode.contains(AlacrittyTermMode::REPORT_EVENT_TYPES) {
            out |= Self::REPORT_EVENT_TYPES.0;
        }
        if mode.contains(AlacrittyTermMode::REPORT_ALTERNATE_KEYS) {
            out |= Self::REPORT_ALTERNATE_KEYS.0;
        }
        if mode.contains(AlacrittyTermMode::REPORT_ALL_KEYS_AS_ESC) {
            out |= Self::REPORT_ALL_KEYS_AS_ESC.0;
        }
        if mode.contains(AlacrittyTermMode::REPORT_ASSOCIATED_TEXT) {
            out |= Self::REPORT_ASSOCIATED_TEXT.0;
        }
        Self(out)
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
