//! Safe `!Send`/`!Sync` wrapper around `GhosttyTerminal`, its callbacks,
//! grid/selection APIs, binary snapshots and `GhosttyFormatter`.

use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::mem::size_of;
use std::panic::{self, AssertUnwindSafe};
use std::ptr::{self, NonNull};
use std::slice;
use std::sync::atomic::{AtomicU64, Ordering};

use ghostty_vt_sys as sys;

use super::{ansi_mode, check, dec_mode, init_sys, take_ghostty_alloc, Result, VtError};
use crate::palette::Rgb;
use crate::terminal::MAX_CLIPBOARD_BYTES;
use crate::types::CursorShape;

pub(crate) const DEFAULT_CONTINUATION_MAX_BYTES: usize = 64 * 1024;
pub(crate) const DEFAULT_SCROLLBACK_LINES: usize = 10_000;
const TITLE_STACK_MAX_DEPTH: usize = 4096;

static NEXT_TERMINAL_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ColorScheme {
    Light,
    Dark,
}

impl ColorScheme {
    #[inline]
    pub(crate) fn to_raw(self) -> sys::GhosttyColorScheme {
        match self {
            Self::Light => sys::GHOSTTY_COLOR_SCHEME_LIGHT,
            Self::Dark => sys::GHOSTTY_COLOR_SCHEME_DARK,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActiveScreen {
    Primary,
    Alternate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClipboardLocation {
    Standard,
    Selection,
    Primary,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProgramStatusState {
    Idle,
    Working,
    Done,
    Blocked,
    Error,
    Clear,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProgramStatusKind {
    Permission,
    Question,
    Auth,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RawProgramStatus {
    pub(crate) state: ProgramStatusState,
    pub(crate) kind: Option<ProgramStatusKind>,
    pub(crate) progress: Option<u8>,
    pub(crate) id: String,
    pub(crate) app: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) message: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SemanticPromptKind {
    PromptStart,
    InputStart,
    OutputStart,
    CommandEnd,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PromptKind {
    Primary,
    Right,
    Continuation,
    Secondary,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SemanticPromptEvent {
    pub(crate) kind: SemanticPromptKind,
    pub(crate) prompt_kind: PromptKind,
    pub(crate) exit_code: Option<i32>,
    pub(crate) command: String,
    pub(crate) error: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TerminalEffect {
    WritePty(Vec<u8>),
    Bell,
    TitleChanged(String),
    PwdChanged(Vec<u8>),
    ClipboardWrite {
        location: ClipboardLocation,
        text: String,
    },
    ProgramStatus(RawProgramStatus),
    SemanticPrompt(SemanticPromptEvent),
    RenderHold(bool),
    Reset,
}

#[derive(Clone, Debug)]
pub(crate) struct TerminalConfig {
    pub(crate) cols: u16,
    pub(crate) rows: u16,
    pub(crate) scrollback_lines: usize,
    pub(crate) continuation_max_bytes: usize,
    pub(crate) kitty_image_storage_limit: u64,
    pub(crate) clipboard_write_max_bytes: usize,
    pub(crate) color_scheme: ColorScheme,
    pub(crate) cell_size_px: (u32, u32),
    /// Install query/effect callbacks (`WRITE_PTY`, `COLOR_SCHEME`, `SIZE`,
    /// `PROGRAM_STATUS`, `SEMANTIC_PROMPT`, `PWD_CHANGED`, `CLIPBOARD_WRITE`,
    /// `TITLE_CHANGED`, `RESET`). Mirrors disable reply-producing callbacks so
    /// only `TerminalModel` answers queries.
    pub(crate) enable_host_callbacks: bool,
    /// Install only `TITLE_CHANGED` and `RESET` callbacks (used by
    /// `TitleParser` without enabling reply-producing host callbacks).
    pub(crate) enable_title_events: bool,
    /// Install `RENDER_HOLD` callback (needed by both models and mirrors).
    pub(crate) enable_render_hold: bool,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            cols: 80,
            rows: 24,
            scrollback_lines: DEFAULT_SCROLLBACK_LINES,
            continuation_max_bytes: DEFAULT_CONTINUATION_MAX_BYTES,
            kitty_image_storage_limit: 0,
            clipboard_write_max_bytes: MAX_CLIPBOARD_BYTES,
            color_scheme: ColorScheme::Dark,
            cell_size_px: (0, 0),
            enable_host_callbacks: true,
            enable_title_events: false,
            enable_render_hold: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Scrollbar {
    pub(crate) total: u64,
    pub(crate) offset: u64,
    pub(crate) len: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScrollViewport {
    Top,
    Bottom,
    Delta(isize),
    Row(usize),
    DeltaPrompt(isize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GridPointTag {
    Active,
    Viewport,
    Screen,
    History,
}

impl GridPointTag {
    #[inline]
    pub(crate) fn to_raw(self) -> sys::GhosttyPointTag {
        match self {
            Self::Active => sys::GHOSTTY_POINT_TAG_ACTIVE,
            Self::Viewport => sys::GHOSTTY_POINT_TAG_VIEWPORT,
            Self::Screen => sys::GHOSTTY_POINT_TAG_SCREEN,
            Self::History => sys::GHOSTTY_POINT_TAG_HISTORY,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CellContentTag {
    Codepoint,
    CodepointGrapheme,
    BgColorPalette,
    BgColorRgb,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CellWide {
    Narrow,
    Wide,
    SpacerTail,
    SpacerHead,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StyleColor {
    None,
    Palette(u8),
    Rgb(Rgb),
}

impl StyleColor {
    #[inline]
    pub(super) fn from_raw(raw: sys::GhosttyStyleColor) -> Self {
        match sys::GhosttyStyleColorTag::from(raw.tag) {
            sys::GHOSTTY_STYLE_COLOR_PALETTE => Self::Palette(unsafe { raw.value.palette }),
            sys::GHOSTTY_STYLE_COLOR_RGB => {
                let rgb = unsafe { raw.value.rgb };
                Self::Rgb(Rgb {
                    r: rgb.r,
                    g: rgb.g,
                    b: rgb.b,
                })
            }
            _ => Self::None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Style {
    pub(crate) fg: StyleColor,
    pub(crate) bg: StyleColor,
    pub(crate) underline: StyleColor,
    pub(crate) flags: u16,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            fg: StyleColor::None,
            bg: StyleColor::None,
            underline: StyleColor::None,
            flags: 0,
        }
    }
}

impl Style {
    #[inline]
    pub(super) fn from_raw(raw: sys::GhosttyStyle) -> Self {
        Self {
            fg: StyleColor::from_raw(raw.fg_color),
            bg: StyleColor::from_raw(raw.bg_color),
            underline: StyleColor::from_raw(raw.underline_color),
            flags: raw.flags,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CellInfo {
    pub(crate) codepoint: u32,
    pub(crate) content_tag: CellContentTag,
    pub(crate) wide: CellWide,
    pub(crate) has_text: bool,
    pub(crate) has_styling: bool,
    pub(crate) has_hyperlink: bool,
    pub(crate) protected: bool,
    pub(crate) bg_palette: Option<u8>,
    pub(crate) bg_rgb: Option<Rgb>,
}

impl CellInfo {
    pub(super) fn from_raw(raw: sys::GhosttyCell) -> Self {
        let mut codepoint: u32 = 0;
        let mut tag: sys::GhosttyCellContentTag = sys::GHOSTTY_CELL_CONTENT_CODEPOINT;
        let mut wide: sys::GhosttyCellWide = sys::GHOSTTY_CELL_WIDE_NARROW;
        let mut has_text = false;
        let mut has_styling = false;
        let mut has_hyperlink = false;
        let mut protected = false;
        unsafe {
            sys::ghostty_cell_get(
                raw,
                sys::GHOSTTY_CELL_DATA_CODEPOINT,
                (&raw mut codepoint).cast(),
            );
            sys::ghostty_cell_get(
                raw,
                sys::GHOSTTY_CELL_DATA_CONTENT_TAG,
                (&raw mut tag).cast(),
            );
            sys::ghostty_cell_get(raw, sys::GHOSTTY_CELL_DATA_WIDE, (&raw mut wide).cast());
            sys::ghostty_cell_get(
                raw,
                sys::GHOSTTY_CELL_DATA_HAS_TEXT,
                (&raw mut has_text).cast(),
            );
            sys::ghostty_cell_get(
                raw,
                sys::GHOSTTY_CELL_DATA_HAS_STYLING,
                (&raw mut has_styling).cast(),
            );
            sys::ghostty_cell_get(
                raw,
                sys::GHOSTTY_CELL_DATA_HAS_HYPERLINK,
                (&raw mut has_hyperlink).cast(),
            );
            sys::ghostty_cell_get(
                raw,
                sys::GHOSTTY_CELL_DATA_PROTECTED,
                (&raw mut protected).cast(),
            );
        }
        let content_tag = match tag {
            sys::GHOSTTY_CELL_CONTENT_CODEPOINT_GRAPHEME => CellContentTag::CodepointGrapheme,
            sys::GHOSTTY_CELL_CONTENT_BG_COLOR_PALETTE => CellContentTag::BgColorPalette,
            sys::GHOSTTY_CELL_CONTENT_BG_COLOR_RGB => CellContentTag::BgColorRgb,
            _ => CellContentTag::Codepoint,
        };
        let wide = match wide {
            sys::GHOSTTY_CELL_WIDE_WIDE => CellWide::Wide,
            sys::GHOSTTY_CELL_WIDE_SPACER_TAIL => CellWide::SpacerTail,
            sys::GHOSTTY_CELL_WIDE_SPACER_HEAD => CellWide::SpacerHead,
            _ => CellWide::Narrow,
        };
        let bg_palette = if content_tag == CellContentTag::BgColorPalette {
            let mut idx: u8 = 0;
            let rc = unsafe {
                sys::ghostty_cell_get(
                    raw,
                    sys::GHOSTTY_CELL_DATA_COLOR_PALETTE,
                    (&raw mut idx).cast(),
                )
            };
            (rc == sys::GHOSTTY_SUCCESS).then_some(idx)
        } else {
            None
        };
        let bg_rgb = if content_tag == CellContentTag::BgColorRgb {
            let mut rgb = sys::GhosttyColorRgb::default();
            let rc = unsafe {
                sys::ghostty_cell_get(raw, sys::GHOSTTY_CELL_DATA_COLOR_RGB, (&raw mut rgb).cast())
            };
            (rc == sys::GHOSTTY_SUCCESS).then_some(Rgb {
                r: rgb.r,
                g: rgb.g,
                b: rgb.b,
            })
        } else {
            None
        };
        Self {
            codepoint,
            content_tag,
            wide,
            has_text,
            has_styling,
            has_hyperlink,
            protected,
            bg_palette,
            bg_rgb,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RowInfo {
    pub(crate) wrap: bool,
    pub(crate) wrap_continuation: bool,
    pub(crate) grapheme: bool,
    pub(crate) styled: bool,
    pub(crate) hyperlink: bool,
}

impl RowInfo {
    pub(super) fn from_raw(raw: sys::GhosttyRow) -> Self {
        let mut wrap = false;
        let mut wrap_continuation = false;
        let mut grapheme = false;
        let mut styled = false;
        let mut hyperlink = false;
        unsafe {
            sys::ghostty_row_get(raw, sys::GHOSTTY_ROW_DATA_WRAP, (&raw mut wrap).cast());
            sys::ghostty_row_get(
                raw,
                sys::GHOSTTY_ROW_DATA_WRAP_CONTINUATION,
                (&raw mut wrap_continuation).cast(),
            );
            sys::ghostty_row_get(
                raw,
                sys::GHOSTTY_ROW_DATA_GRAPHEME,
                (&raw mut grapheme).cast(),
            );
            sys::ghostty_row_get(raw, sys::GHOSTTY_ROW_DATA_STYLED, (&raw mut styled).cast());
            sys::ghostty_row_get(
                raw,
                sys::GHOSTTY_ROW_DATA_HYPERLINK,
                (&raw mut hyperlink).cast(),
            );
        }
        Self {
            wrap,
            wrap_continuation,
            grapheme,
            styled,
            hyperlink,
        }
    }
}

/// Untracked snapshot reference to a single cell in a terminal's active screen.
///
/// Borrows `&'a Terminal` so no mutating call (`&mut Terminal`) or drop can
/// invalidate its backing page pointer while this reference exists.
#[derive(Clone, Copy)]
pub(crate) struct GridRef<'a> {
    raw: sys::GhosttyGridRef,
    owner_id: u64,
    screen: ActiveScreen,
    _term: PhantomData<&'a Terminal>,
}

impl<'a> GridRef<'a> {
    pub(crate) fn cell(&self) -> Result<CellInfo> {
        let mut raw: sys::GhosttyCell = 0;
        check(unsafe { sys::ghostty_grid_ref_cell(&self.raw, &mut raw) })?;
        Ok(CellInfo::from_raw(raw))
    }

    pub(crate) fn row(&self) -> Result<RowInfo> {
        let mut raw: sys::GhosttyRow = 0;
        check(unsafe { sys::ghostty_grid_ref_row(&self.raw, &mut raw) })?;
        Ok(RowInfo::from_raw(raw))
    }

    pub(crate) fn style(&self) -> Result<Style> {
        let mut raw = sys::GhosttyStyle::default();
        check(unsafe { sys::ghostty_grid_ref_style(&self.raw, &mut raw) })?;
        Ok(Style::from_raw(raw))
    }

    pub(crate) fn graphemes(&self) -> Result<Vec<char>> {
        let mut stack = [0u32; 8];
        let mut written = 0usize;
        let rc = unsafe {
            sys::ghostty_grid_ref_graphemes(
                &self.raw,
                stack.as_mut_ptr(),
                stack.len(),
                &mut written,
            )
        };
        let cps: Vec<u32> = match rc {
            sys::GHOSTTY_SUCCESS => stack[..written].to_vec(),
            sys::GHOSTTY_OUT_OF_SPACE => {
                let mut heap = vec![0u32; written];
                check(unsafe {
                    sys::ghostty_grid_ref_graphemes(
                        &self.raw,
                        heap.as_mut_ptr(),
                        heap.len(),
                        &mut written,
                    )
                })?;
                heap.truncate(written);
                heap
            }
            other => return Err(check(other).unwrap_err()),
        };
        Ok(cps.into_iter().filter_map(char::from_u32).collect())
    }

    pub(crate) fn hyperlink_uri(&self) -> Result<Option<String>> {
        let mut len = 0usize;
        let rc =
            unsafe { sys::ghostty_grid_ref_hyperlink_uri(&self.raw, ptr::null_mut(), 0, &mut len) };
        match rc {
            sys::GHOSTTY_SUCCESS if len == 0 => Ok(None),
            sys::GHOSTTY_OUT_OF_SPACE => {
                let mut buf = vec![0u8; len];
                let mut written = 0usize;
                check(unsafe {
                    sys::ghostty_grid_ref_hyperlink_uri(
                        &self.raw,
                        buf.as_mut_ptr(),
                        buf.len(),
                        &mut written,
                    )
                })?;
                buf.truncate(written);
                Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
            }
            other => Err(check(other).unwrap_err()),
        }
    }
}

/// Owned tracked reference that follows a cell across scrollback and reflow.
pub(crate) struct TrackedGridRef {
    raw: sys::GhosttyTrackedGridRef,
    owner_id: u64,
    screen: ActiveScreen,
    _not_send_sync: PhantomData<*mut ()>,
}

impl TrackedGridRef {
    pub(crate) fn has_value(&self) -> bool {
        unsafe { sys::ghostty_tracked_grid_ref_has_value(self.raw) }
    }

    pub(crate) fn point(&self, tag: GridPointTag) -> Option<(u16, u32)> {
        let mut coord = sys::GhosttyPointCoordinate::default();
        let rc = unsafe { sys::ghostty_tracked_grid_ref_point(self.raw, tag.to_raw(), &mut coord) };
        (rc == sys::GHOSTTY_SUCCESS).then_some((coord.x, coord.y))
    }

    pub(crate) fn set(&mut self, term: &Terminal, tag: GridPointTag, x: u16, y: u32) -> Result<()> {
        if self.owner_id != term.id {
            return Err(VtError::InvalidValue);
        }
        let pt = make_point(tag, x, y);
        check(unsafe { sys::ghostty_tracked_grid_ref_set(self.raw, term.raw, pt) })?;
        self.screen = term.active_screen();
        Ok(())
    }

    /// Snapshot into an untracked `GridRef<'a>` tied to `&'a Terminal`.
    ///
    /// Returns `Err(VtError::InvalidValue)` if `term` is not the originating
    /// terminal, and `Ok(None)` if the tracked cell was evicted or belongs to
    /// the inactive screen (primary vs alternate).
    pub(crate) fn snapshot<'a>(&self, term: &'a Terminal) -> Result<Option<GridRef<'a>>> {
        if self.owner_id != term.id {
            return Err(VtError::InvalidValue);
        }
        let active = term.active_screen();
        if self.screen != active {
            return Ok(None);
        }
        let mut raw = sys::GhosttyGridRef::default();
        let rc = unsafe { sys::ghostty_tracked_grid_ref_snapshot(self.raw, &mut raw) };
        match rc {
            sys::GHOSTTY_SUCCESS => Ok(Some(GridRef {
                raw,
                owner_id: term.id,
                screen: active,
                _term: PhantomData,
            })),
            sys::GHOSTTY_NO_VALUE => Ok(None),
            other => Err(check(other).unwrap_err()),
        }
    }
}

impl Drop for TrackedGridRef {
    fn drop(&mut self) {
        unsafe { sys::ghostty_tracked_grid_ref_free(self.raw) };
    }
}

/// Owned coordinate endpoints of a selection in a specific `GridPointTag` space.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SelectionSpan {
    pub(crate) start: (u16, u32),
    pub(crate) end: (u16, u32),
    pub(crate) rectangle: bool,
}

/// Untracked selection snapshot borrowing `&'a Terminal`.
///
/// Cannot outlive `&'a Terminal`, coexist with `&mut Terminal`, or be passed to
/// a different terminal or screen.
#[derive(Clone, Copy)]
pub(crate) struct Selection<'a> {
    raw: sys::GhosttySelection,
    owner_id: u64,
    screen: ActiveScreen,
    _term: PhantomData<&'a Terminal>,
}

impl<'a> Selection<'a> {
    pub(crate) fn new(start: GridRef<'a>, end: GridRef<'a>, rectangle: bool) -> Result<Self> {
        if start.owner_id != end.owner_id || start.screen != end.screen {
            return Err(VtError::InvalidValue);
        }
        Ok(Self {
            raw: sys::GhosttySelection {
                start: start.raw,
                end: end.raw,
                rectangle,
            },
            owner_id: start.owner_id,
            screen: start.screen,
            _term: PhantomData,
        })
    }

    pub(crate) fn start(&self) -> GridRef<'a> {
        GridRef {
            raw: self.raw.start,
            owner_id: self.owner_id,
            screen: self.screen,
            _term: PhantomData,
        }
    }

    pub(crate) fn end(&self) -> GridRef<'a> {
        GridRef {
            raw: self.raw.end,
            owner_id: self.owner_id,
            screen: self.screen,
            _term: PhantomData,
        }
    }

    pub(crate) fn rectangle(&self) -> bool {
        self.raw.rectangle
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SelectionOrder {
    Forward,
    Reverse,
    MirroredForward,
    MirroredReverse,
}

impl SelectionOrder {
    fn to_raw(self) -> sys::GhosttySelectionOrder {
        match self {
            Self::Forward => sys::GHOSTTY_SELECTION_ORDER_FORWARD,
            Self::Reverse => sys::GHOSTTY_SELECTION_ORDER_REVERSE,
            Self::MirroredForward => sys::GHOSTTY_SELECTION_ORDER_MIRRORED_FORWARD,
            Self::MirroredReverse => sys::GHOSTTY_SELECTION_ORDER_MIRRORED_REVERSE,
        }
    }

    fn from_raw(raw: sys::GhosttySelectionOrder) -> Self {
        match raw {
            sys::GHOSTTY_SELECTION_ORDER_REVERSE => Self::Reverse,
            sys::GHOSTTY_SELECTION_ORDER_MIRRORED_FORWARD => Self::MirroredForward,
            sys::GHOSTTY_SELECTION_ORDER_MIRRORED_REVERSE => Self::MirroredReverse,
            _ => Self::Forward,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SelectionAdjust {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    BeginningOfLine,
    EndOfLine,
}

impl SelectionAdjust {
    fn to_raw(self) -> sys::GhosttySelectionAdjust {
        match self {
            Self::Left => sys::GHOSTTY_SELECTION_ADJUST_LEFT,
            Self::Right => sys::GHOSTTY_SELECTION_ADJUST_RIGHT,
            Self::Up => sys::GHOSTTY_SELECTION_ADJUST_UP,
            Self::Down => sys::GHOSTTY_SELECTION_ADJUST_DOWN,
            Self::Home => sys::GHOSTTY_SELECTION_ADJUST_HOME,
            Self::End => sys::GHOSTTY_SELECTION_ADJUST_END,
            Self::PageUp => sys::GHOSTTY_SELECTION_ADJUST_PAGE_UP,
            Self::PageDown => sys::GHOSTTY_SELECTION_ADJUST_PAGE_DOWN,
            Self::BeginningOfLine => sys::GHOSTTY_SELECTION_ADJUST_BEGINNING_OF_LINE,
            Self::EndOfLine => sys::GHOSTTY_SELECTION_ADJUST_END_OF_LINE,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FormatKind {
    Plain,
    Vt,
    Html,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FormatOptions {
    pub(crate) kind: FormatKind,
    pub(crate) unwrap: bool,
    pub(crate) trim: bool,
    pub(crate) extras: sys::GhosttyFormatterExtras,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            kind: FormatKind::Plain,
            unwrap: false,
            trim: true,
            extras: sys::GHOSTTY_FORMATTER_EXTRAS_NONE,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SnapshotDecodeOptions {
    pub(crate) max_continuation_bytes: usize,
    pub(crate) retain_continuation: bool,
    pub(crate) compress_history: bool,
    pub(crate) kitty_image_storage_limit: u64,
}

impl Default for SnapshotDecodeOptions {
    fn default() -> Self {
        Self {
            max_continuation_bytes: DEFAULT_CONTINUATION_MAX_BYTES,
            retain_continuation: true,
            compress_history: true,
            kitty_image_storage_limit: 0,
        }
    }
}

struct EffectInbox {
    effects: Vec<TerminalEffect>,
    color_scheme: ColorScheme,
    cell_size_px: (u32, u32),
    max_clipboard_bytes: usize,
    title_stack: Vec<Option<String>>,
    emit_title_and_reset: bool,
}

/// Single-owner `!Send`/`!Sync` terminal state backed by `libghostty-vt`.
pub(crate) struct Terminal {
    id: u64,
    raw: sys::GhosttyTerminal,
    /// Heap-allocated `UnsafeCell<EffectInbox>` registered as `GhosttyTerminal`'s
    /// `userdata`. Owned via raw pointer (never `Box`) so no `noalias` unique
    /// pointer conflicts with reentrant C callback access. Freed in `Drop`
    /// strictly after `ghostty_terminal_free(self.raw)`.
    inbox: NonNull<UnsafeCell<EffectInbox>>,
    _not_send_sync: PhantomData<*mut ()>,
}

impl Terminal {
    pub(crate) fn new(config: &TerminalConfig) -> Result<Self> {
        init_sys();
        let mut raw: sys::GhosttyTerminal = ptr::null_mut();
        check(unsafe {
            sys::ghostty_terminal_new(
                ptr::null(),
                &mut raw,
                config.cols.max(1),
                config.rows.max(1),
            )
        })?;
        let mut term = Self::from_raw(raw, config);
        term.configure_instance(config)?;
        Ok(term)
    }

    pub(crate) fn decode_snapshot(
        bytes: &[u8],
        decode_opts: SnapshotDecodeOptions,
        config: &TerminalConfig,
    ) -> Result<Self> {
        init_sys();
        let (ghostty_bytes, title_stack) = strip_title_stack_footer(bytes);
        let mut decoder: sys::GhosttySnapshotDecoder = ptr::null_mut();
        check(unsafe {
            sys::ghostty_snapshot_decoder_new_buf(
                ptr::null(),
                &mut decoder,
                ghostty_bytes.as_ptr(),
                ghostty_bytes.len(),
            )
        })?;
        struct DecoderGuard(sys::GhosttySnapshotDecoder);
        impl Drop for DecoderGuard {
            fn drop(&mut self) {
                unsafe { sys::ghostty_snapshot_decoder_free(self.0) };
            }
        }
        let _guard = DecoderGuard(decoder);

        unsafe {
            check(sys::ghostty_snapshot_decoder_set(
                decoder,
                sys::GHOSTTY_SNAPSHOT_DECODER_OPT_MAX_CONTINUATION_BYTES,
                (&raw const decode_opts.max_continuation_bytes).cast(),
            ))?;
            check(sys::ghostty_snapshot_decoder_set(
                decoder,
                sys::GHOSTTY_SNAPSHOT_DECODER_OPT_RETAIN_CONTINUATION,
                (&raw const decode_opts.retain_continuation).cast(),
            ))?;
            check(sys::ghostty_snapshot_decoder_set(
                decoder,
                sys::GHOSTTY_SNAPSHOT_DECODER_OPT_COMPRESS_HISTORY,
                (&raw const decode_opts.compress_history).cast(),
            ))?;
            check(sys::ghostty_snapshot_decoder_set(
                decoder,
                sys::GHOSTTY_SNAPSHOT_DECODER_OPT_KITTY_IMAGE_STORAGE_LIMIT,
                (&raw const decode_opts.kitty_image_storage_limit).cast(),
            ))?;
        }

        let mut raw: sys::GhosttyTerminal = ptr::null_mut();
        check(unsafe { sys::ghostty_snapshot_decoder_decode(decoder, &mut raw) })?;
        if raw.is_null() {
            return Err(VtError::InvalidValue);
        }

        let mut term = Self::from_raw(raw, config);
        term.configure_after_decode(config)?;
        if !title_stack.is_empty() {
            term.set_title_stack(title_stack);
        }
        Ok(term)
    }

    fn from_raw(raw: sys::GhosttyTerminal, config: &TerminalConfig) -> Self {
        let inbox = NonNull::new(Box::into_raw(Box::new(UnsafeCell::new(EffectInbox {
            effects: Vec::new(),
            color_scheme: config.color_scheme,
            cell_size_px: config.cell_size_px,
            max_clipboard_bytes: config.clipboard_write_max_bytes,
            title_stack: Vec::new(),
            emit_title_and_reset: config.enable_host_callbacks || config.enable_title_events,
        }))))
        .expect("Box::into_raw returned null");
        Self {
            id: NEXT_TERMINAL_ID.fetch_add(1, Ordering::Relaxed),
            raw,
            inbox,
            _not_send_sync: PhantomData,
        }
    }

    fn configure_instance(&mut self, config: &TerminalConfig) -> Result<()> {
        self.set_scrollback_lines(config.scrollback_lines)?;
        self.set_continuation_max_bytes(config.continuation_max_bytes)?;
        self.set_kitty_image_storage_limit(config.kitty_image_storage_limit)?;
        self.set_clipboard_write_max_bytes(config.clipboard_write_max_bytes)?;
        self.install_callbacks(config)?;
        Ok(())
    }

    fn configure_after_decode(&mut self, config: &TerminalConfig) -> Result<()> {
        self.set_scrollback_lines(config.scrollback_lines)?;
        self.set_kitty_image_storage_limit(config.kitty_image_storage_limit)?;
        self.set_clipboard_write_max_bytes(config.clipboard_write_max_bytes)?;
        self.install_callbacks(config)?;
        Ok(())
    }

    fn install_callbacks(&mut self, config: &TerminalConfig) -> Result<()> {
        let ud: *mut c_void = self.inbox.as_ptr().cast();
        unsafe {
            check(sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_USERDATA,
                ud,
            ))?;
            // Always maintain the xterm title stack (`CSI 22/23 t`) and clear
            // it on RIS (`ESC c`) regardless of whether host callbacks run.
            let title_push: unsafe extern "C" fn(sys::GhosttyTerminal, *mut c_void, u16) =
                on_title_push;
            let title_pop: unsafe extern "C" fn(sys::GhosttyTerminal, *mut c_void, u16) =
                on_title_pop;
            let reset: unsafe extern "C" fn(sys::GhosttyTerminal, *mut c_void) = on_reset;
            check(sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_TITLE_PUSH,
                title_push as *const c_void,
            ))?;
            check(sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_TITLE_POP,
                title_pop as *const c_void,
            ))?;
            check(sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_RESET,
                reset as *const c_void,
            ))?;

            if config.enable_render_hold {
                let cb: unsafe extern "C" fn(sys::GhosttyTerminal, *mut c_void, bool) =
                    on_render_hold;
                check(sys::ghostty_terminal_set(
                    self.raw,
                    sys::GHOSTTY_TERMINAL_OPT_RENDER_HOLD,
                    cb as *const c_void,
                ))?;
            }
            if config.enable_host_callbacks || config.enable_title_events {
                let title: unsafe extern "C" fn(sys::GhosttyTerminal, *mut c_void) =
                    on_title_changed;
                check(sys::ghostty_terminal_set(
                    self.raw,
                    sys::GHOSTTY_TERMINAL_OPT_TITLE_CHANGED,
                    title as *const c_void,
                ))?;
            }
            if config.enable_host_callbacks {
                let write_pty: unsafe extern "C" fn(
                    sys::GhosttyTerminal,
                    *mut c_void,
                    *const u8,
                    usize,
                ) = on_write_pty;
                let bell: unsafe extern "C" fn(sys::GhosttyTerminal, *mut c_void) = on_bell;
                let pwd: unsafe extern "C" fn(sys::GhosttyTerminal, *mut c_void) = on_pwd_changed;
                let size_cb: unsafe extern "C" fn(
                    sys::GhosttyTerminal,
                    *mut c_void,
                    *mut sys::GhosttySizeReportSize,
                ) -> bool = on_size;
                let color_scheme: unsafe extern "C" fn(
                    sys::GhosttyTerminal,
                    *mut c_void,
                    *mut sys::GhosttyColorScheme,
                ) -> bool = on_color_scheme;
                let device_attributes: unsafe extern "C" fn(
                    sys::GhosttyTerminal,
                    *mut c_void,
                    *mut sys::GhosttyDeviceAttributes,
                ) -> bool = on_device_attributes;
                let clipboard_write: unsafe extern "C" fn(
                    sys::GhosttyTerminal,
                    *mut c_void,
                    *const sys::GhosttyClipboardWrite,
                ) = on_clipboard_write;
                let program_status: unsafe extern "C" fn(
                    sys::GhosttyTerminal,
                    *mut c_void,
                    *const sys::GhosttyTerminalProgramStatus,
                ) = on_program_status;
                let semantic_prompt: unsafe extern "C" fn(
                    sys::GhosttyTerminal,
                    *mut c_void,
                    *const sys::GhosttyTerminalSemanticPrompt,
                ) = on_semantic_prompt;

                check(sys::ghostty_terminal_set(
                    self.raw,
                    sys::GHOSTTY_TERMINAL_OPT_WRITE_PTY,
                    write_pty as *const c_void,
                ))?;
                check(sys::ghostty_terminal_set(
                    self.raw,
                    sys::GHOSTTY_TERMINAL_OPT_BELL,
                    bell as *const c_void,
                ))?;
                check(sys::ghostty_terminal_set(
                    self.raw,
                    sys::GHOSTTY_TERMINAL_OPT_PWD_CHANGED,
                    pwd as *const c_void,
                ))?;
                check(sys::ghostty_terminal_set(
                    self.raw,
                    sys::GHOSTTY_TERMINAL_OPT_SIZE,
                    size_cb as *const c_void,
                ))?;
                check(sys::ghostty_terminal_set(
                    self.raw,
                    sys::GHOSTTY_TERMINAL_OPT_COLOR_SCHEME,
                    color_scheme as *const c_void,
                ))?;
                check(sys::ghostty_terminal_set(
                    self.raw,
                    sys::GHOSTTY_TERMINAL_OPT_DEVICE_ATTRIBUTES,
                    device_attributes as *const c_void,
                ))?;
                check(sys::ghostty_terminal_set(
                    self.raw,
                    sys::GHOSTTY_TERMINAL_OPT_CLIPBOARD_WRITE,
                    clipboard_write as *const c_void,
                ))?;
                check(sys::ghostty_terminal_set(
                    self.raw,
                    sys::GHOSTTY_TERMINAL_OPT_PROGRAM_STATUS,
                    program_status as *const c_void,
                ))?;
                check(sys::ghostty_terminal_set(
                    self.raw,
                    sys::GHOSTTY_TERMINAL_OPT_SEMANTIC_PROMPT,
                    semantic_prompt as *const c_void,
                ))?;
            }
        }
        Ok(())
    }

    #[inline]
    pub(super) fn raw(&self) -> sys::GhosttyTerminal {
        self.raw
    }

    #[inline]
    pub(super) fn id(&self) -> u64 {
        self.id
    }

    /// Brand a raw `GhosttySelection` with this terminal's identity and active screen.
    ///
    /// # Safety
    /// Both `raw.start` and `raw.end` must be valid, live, untracked
    /// `GhosttyGridRef` snapshots from `self`'s currently active screen, with no
    /// mutating terminal calls since they were produced.
    #[inline]
    pub(super) unsafe fn wrap_selection(&self, raw: sys::GhosttySelection) -> Selection<'_> {
        Selection {
            raw,
            owner_id: self.id,
            screen: self.active_screen(),
            _term: PhantomData,
        }
    }

    fn verify_grid_ref(&self, grid_ref: &GridRef<'_>) -> Result<()> {
        if grid_ref.owner_id != self.id || grid_ref.screen != self.active_screen() {
            return Err(VtError::InvalidValue);
        }
        Ok(())
    }

    fn verify_selection(&self, selection: &Selection<'_>) -> Result<()> {
        if selection.owner_id != self.id || selection.screen != self.active_screen() {
            return Err(VtError::InvalidValue);
        }
        Ok(())
    }

    pub(crate) fn vt_write(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() {
            unsafe { sys::ghostty_terminal_vt_write(self.raw, bytes.as_ptr(), bytes.len()) };
        }
    }

    /// Write `bytes` until the VT stream finishes an OSC sequence or returns to
    /// ground. Returns `(bytes_consumed, reached_boundary)`.
    pub(crate) fn vt_write_until_osc_or_ground(&mut self, bytes: &[u8]) -> Result<(usize, bool)> {
        if bytes.is_empty() {
            return Ok((0, false));
        }
        // Bindgen emits C enums as `c_int` under MSVC and `c_uint` on Unix,
        // while `vt_write_until` takes a fixed `uint32_t` bitmask.
        #[allow(clippy::unnecessary_cast)]
        let mask: u32 =
            (sys::GHOSTTY_TERMINAL_BOUNDARY_GROUND | sys::GHOSTTY_TERMINAL_BOUNDARY_OSC_END) as u32;
        let mut stop = sys::GhosttyTerminalWriteStop {
            size: size_of::<sys::GhosttyTerminalWriteStop>(),
            ..Default::default()
        };
        let rc = unsafe {
            sys::ghostty_terminal_vt_write_until(
                self.raw,
                bytes.as_ptr(),
                bytes.len(),
                mask,
                &mut stop,
            )
        };
        match rc {
            sys::GHOSTTY_SUCCESS => Ok((stop.consumed, stop.reached != 0)),
            sys::GHOSTTY_NO_VALUE => Ok((stop.consumed, false)),
            other => Err(check(other).unwrap_err()),
        }
    }

    /// Phase 1 supports Kitty keyboard bit 0 (`DISAMBIGUATE_ESC_CODES`) only;
    /// clamp any higher bits set on the live top-of-stack when at VT ground.
    pub(crate) fn clamp_kitty_keyboard_flags(&mut self) {
        let flags = self.kitty_keyboard_flags();
        if flags > 1 && self.vt_ground() {
            let clamped = flags & 1;
            let seq = if clamped == 1 {
                b"\x1b[=1;1u".as_slice()
            } else {
                b"\x1b[=0;1u".as_slice()
            };
            unsafe { sys::ghostty_terminal_vt_write(self.raw, seq.as_ptr(), seq.len()) };
        }
    }

    pub(crate) fn title_stack(&self) -> Vec<Option<String>> {
        unsafe { (*self.inbox.as_ref().get()).title_stack.clone() }
    }

    pub(crate) fn clear_title_stack(&mut self) {
        unsafe { (*self.inbox.as_ref().get()).title_stack.clear() };
    }

    pub(crate) fn set_title_stack(&mut self, stack: Vec<Option<String>>) {
        unsafe { (*self.inbox.as_ref().get()).title_stack = stack };
    }

    pub(crate) fn take_effects(&mut self) -> Vec<TerminalEffect> {
        unsafe { std::mem::take(&mut (*self.inbox.as_ref().get()).effects) }
    }

    pub(crate) fn resize(
        &mut self,
        cols: u16,
        rows: u16,
        cell_width_px: u32,
        cell_height_px: u32,
    ) -> Result<()> {
        unsafe {
            (*self.inbox.as_ref().get()).cell_size_px = (cell_width_px, cell_height_px);
        }
        check(unsafe {
            sys::ghostty_terminal_resize(
                self.raw,
                cols.max(1),
                rows.max(1),
                cell_width_px,
                cell_height_px,
            )
        })
    }

    pub(crate) fn reset(&mut self) {
        unsafe { sys::ghostty_terminal_reset(self.raw) };
    }

    pub(crate) fn scroll_viewport(&mut self, action: ScrollViewport) -> Result<()> {
        let raw = match action {
            ScrollViewport::Top => sys::GhosttyTerminalScrollViewport {
                tag: sys::GHOSTTY_TERMINAL_SCROLL_VIEWPORT_TOP,
                value: sys::GhosttyTerminalScrollViewportValue::default(),
            },
            ScrollViewport::Bottom => sys::GhosttyTerminalScrollViewport {
                tag: sys::GHOSTTY_TERMINAL_SCROLL_VIEWPORT_BOTTOM,
                value: sys::GhosttyTerminalScrollViewportValue::default(),
            },
            ScrollViewport::Delta(delta) => sys::GhosttyTerminalScrollViewport {
                tag: sys::GHOSTTY_TERMINAL_SCROLL_VIEWPORT_DELTA,
                value: sys::GhosttyTerminalScrollViewportValue { delta },
            },
            ScrollViewport::Row(row) => sys::GhosttyTerminalScrollViewport {
                tag: sys::GHOSTTY_TERMINAL_SCROLL_VIEWPORT_ROW,
                value: sys::GhosttyTerminalScrollViewportValue { row },
            },
            ScrollViewport::DeltaPrompt(delta_prompt) => sys::GhosttyTerminalScrollViewport {
                tag: sys::GHOSTTY_TERMINAL_SCROLL_VIEWPORT_DELTA_PROMPT,
                value: sys::GhosttyTerminalScrollViewportValue { delta_prompt },
            },
        };
        check(unsafe { sys::ghostty_terminal_scroll_viewport(self.raw, raw) })
    }

    pub(crate) fn set_scrollback_lines(&mut self, lines: usize) -> Result<()> {
        // `SCROLLBACK_MAX_BYTES = 0` disables scrollback and erases retained
        // history (terminal.h:2328); non-zero line limits lift the byte cap so
        // line count governs page eviction.
        let bytes_limit: usize = if lines == 0 { 0 } else { usize::MAX };
        unsafe {
            check(sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_BYTES,
                (&raw const bytes_limit).cast(),
            ))?;
            check(sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_LINES,
                (&raw const lines).cast(),
            ))?;
        }
        Ok(())
    }

    pub(crate) fn set_continuation_max_bytes(&mut self, bytes: usize) -> Result<()> {
        check(unsafe {
            sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_CONTINUATION_MAX_BYTES,
                (&raw const bytes).cast(),
            )
        })
    }

    pub(crate) fn set_kitty_image_storage_limit(&mut self, limit: u64) -> Result<()> {
        check(unsafe {
            sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_STORAGE_LIMIT,
                (&raw const limit).cast(),
            )
        })
    }

    pub(crate) fn set_clipboard_write_max_bytes(&mut self, max_bytes: usize) -> Result<()> {
        unsafe {
            (*self.inbox.as_ref().get()).max_clipboard_bytes = max_bytes;
        }
        check(unsafe {
            sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_CLIPBOARD_WRITE_MAX_BYTES,
                (&raw const max_bytes).cast(),
            )
        })
    }

    pub(crate) fn set_color_scheme(&mut self, scheme: ColorScheme) {
        unsafe {
            (*self.inbox.as_ref().get()).color_scheme = scheme;
        }
    }

    pub(crate) fn color_scheme(&self) -> ColorScheme {
        unsafe { (*self.inbox.as_ref().get()).color_scheme }
    }

    pub(crate) fn set_cell_size_px(&mut self, width: u32, height: u32) {
        unsafe {
            (*self.inbox.as_ref().get()).cell_size_px = (width, height);
        }
    }

    pub(crate) fn set_default_colors(
        &mut self,
        fg: Option<Rgb>,
        bg: Option<Rgb>,
        cursor: Option<Rgb>,
        palette: Option<&[Rgb; 256]>,
    ) -> Result<()> {
        unsafe {
            let set_rgb = |opt, rgb: Option<Rgb>| -> Result<()> {
                match rgb {
                    Some(c) => {
                        let raw = sys::GhosttyColorRgb {
                            r: c.r,
                            g: c.g,
                            b: c.b,
                        };
                        check(sys::ghostty_terminal_set(
                            self.raw,
                            opt,
                            (&raw const raw).cast(),
                        ))
                    }
                    None => check(sys::ghostty_terminal_set(self.raw, opt, ptr::null())),
                }
            };
            set_rgb(sys::GHOSTTY_TERMINAL_OPT_COLOR_FOREGROUND, fg)?;
            set_rgb(sys::GHOSTTY_TERMINAL_OPT_COLOR_BACKGROUND, bg)?;
            set_rgb(sys::GHOSTTY_TERMINAL_OPT_COLOR_CURSOR, cursor)?;
            match palette {
                Some(colors) => {
                    let raw: [sys::GhosttyColorRgb; 256] = std::array::from_fn(|i| {
                        let c = colors[i];
                        sys::GhosttyColorRgb {
                            r: c.r,
                            g: c.g,
                            b: c.b,
                        }
                    });
                    check(sys::ghostty_terminal_set(
                        self.raw,
                        sys::GHOSTTY_TERMINAL_OPT_COLOR_PALETTE,
                        raw.as_ptr().cast(),
                    ))?;
                }
                None => {
                    check(sys::ghostty_terminal_set(
                        self.raw,
                        sys::GHOSTTY_TERMINAL_OPT_COLOR_PALETTE,
                        ptr::null(),
                    ))?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn set_theme_defaults(
        &mut self,
        theme: crate::palette::TerminalPalette,
    ) -> Result<()> {
        let base = crate::palette::base_palette_for(theme);
        self.set_color_scheme(if theme.is_light() {
            ColorScheme::Light
        } else {
            ColorScheme::Dark
        });
        self.set_default_colors(
            Some(theme.foreground),
            Some(theme.background),
            Some(theme.cursor),
            Some(&base),
        )
    }

    pub(crate) fn set_default_cursor_style(
        &mut self,
        shape: CursorShape,
        blink: bool,
    ) -> Result<()> {
        let style = match shape {
            CursorShape::Beam => sys::GHOSTTY_CURSOR_STYLE_BAR,
            CursorShape::Underline => sys::GHOSTTY_CURSOR_STYLE_UNDERLINE,
            CursorShape::HollowBlock => sys::GHOSTTY_CURSOR_STYLE_BLOCK_HOLLOW,
            CursorShape::Block | CursorShape::Hidden => sys::GHOSTTY_CURSOR_STYLE_BLOCK,
        };
        unsafe {
            check(sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_DEFAULT_CURSOR_STYLE,
                (&raw const style).cast(),
            ))?;
            check(sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_DEFAULT_CURSOR_BLINK,
                (&raw const blink).cast(),
            ))?;
        }
        Ok(())
    }

    pub(crate) fn set_mode(&mut self, mode: sys::GhosttyMode, value: bool) -> Result<()> {
        let cfg = sys::GhosttyTerminalModeConfig { mode, value };
        check(unsafe {
            sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_MODE,
                (&raw const cfg).cast(),
            )
        })
    }

    pub(crate) fn set_mode_default(&mut self, mode: sys::GhosttyMode, value: bool) -> Result<()> {
        let cfg = sys::GhosttyTerminalModeConfig { mode, value };
        check(unsafe {
            sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_MODE_DEFAULT,
                (&raw const cfg).cast(),
            )
        })
    }

    pub(crate) fn mode(&self, mode: sys::GhosttyMode) -> bool {
        let mut cfg = sys::GhosttyTerminalModeConfig { mode, value: false };
        let rc = unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_MODE,
                (&raw mut cfg).cast(),
            )
        };
        rc == sys::GHOSTTY_SUCCESS && cfg.value
    }

    pub(crate) fn set_title(&mut self, title: &str) -> Result<()> {
        let raw = sys::GhosttyString {
            ptr: title.as_ptr(),
            len: title.len(),
        };
        check(unsafe {
            sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_TITLE,
                (&raw const raw).cast(),
            )
        })
    }

    pub(crate) fn set_pwd(&mut self, pwd: &str) -> Result<()> {
        let raw = sys::GhosttyString {
            ptr: pwd.as_ptr(),
            len: pwd.len(),
        };
        check(unsafe {
            sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_PWD,
                (&raw const raw).cast(),
            )
        })
    }

    pub(crate) fn set_resize_pull_scrollback(&mut self, enabled: bool) -> Result<()> {
        check(unsafe {
            sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_RESIZE_PULL_SCROLLBACK,
                (&raw const enabled).cast(),
            )
        })
    }

    pub(crate) fn cols(&self) -> u16 {
        let mut cols: u16 = 0;
        unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_COLS,
                (&raw mut cols).cast(),
            );
        }
        cols
    }

    pub(crate) fn rows(&self) -> u16 {
        let mut rows: u16 = 0;
        unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_ROWS,
                (&raw mut rows).cast(),
            );
        }
        rows
    }

    pub(crate) fn size(&self) -> (u16, u16) {
        (self.cols(), self.rows())
    }

    pub(crate) fn cursor_pos(&self) -> (u16, u16) {
        let (mut x, mut y) = (0u16, 0u16);
        unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_CURSOR_X,
                (&raw mut x).cast(),
            );
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_CURSOR_Y,
                (&raw mut y).cast(),
            );
        }
        (x, y)
    }

    pub(crate) fn cursor_visible(&self) -> bool {
        let mut visible = false;
        unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_CURSOR_VISIBLE,
                (&raw mut visible).cast(),
            );
        }
        visible
    }

    pub(crate) fn cursor_pending_wrap(&self) -> bool {
        let mut pending = false;
        unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_CURSOR_PENDING_WRAP,
                (&raw mut pending).cast(),
            );
        }
        pending
    }

    pub(crate) fn active_screen(&self) -> ActiveScreen {
        let mut screen: sys::GhosttyTerminalScreen = sys::GHOSTTY_TERMINAL_SCREEN_PRIMARY;
        unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_ACTIVE_SCREEN,
                (&raw mut screen).cast(),
            );
        }
        if screen == sys::GHOSTTY_TERMINAL_SCREEN_ALTERNATE {
            ActiveScreen::Alternate
        } else {
            ActiveScreen::Primary
        }
    }

    pub(crate) fn kitty_keyboard_flags(&self) -> u8 {
        let mut flags: u8 = 0;
        unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_KITTY_KEYBOARD_FLAGS,
                (&raw mut flags).cast(),
            );
        }
        flags
    }

    pub(crate) fn scrollbar(&self) -> Scrollbar {
        let mut raw = sys::GhosttyTerminalScrollbar::default();
        unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_SCROLLBAR,
                (&raw mut raw).cast(),
            );
        }
        Scrollbar {
            total: raw.total,
            offset: raw.offset,
            len: raw.len,
        }
    }

    pub(crate) fn total_rows(&self) -> usize {
        let mut total: usize = 0;
        unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_TOTAL_ROWS,
                (&raw mut total).cast(),
            );
        }
        total
    }

    pub(crate) fn scrollback_rows(&self) -> usize {
        let mut rows: usize = 0;
        unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_SCROLLBACK_ROWS,
                (&raw mut rows).cast(),
            );
        }
        rows
    }

    pub(crate) fn scrollback_disabled(&self) -> bool {
        let mut bytes: usize = 0;
        let rc = unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_SCROLLBACK_MAX_BYTES,
                (&raw mut bytes).cast(),
            )
        };
        rc == sys::GHOSTTY_SUCCESS && bytes == 0
    }

    pub(crate) fn scrollback_max_lines(&self) -> Option<usize> {
        let mut lines: usize = 0;
        let rc = unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_SCROLLBACK_MAX_LINES,
                (&raw mut lines).cast(),
            )
        };
        (rc == sys::GHOSTTY_SUCCESS).then_some(lines)
    }

    pub(crate) fn scroll_up_rows(&self) -> u64 {
        let mut rows: u64 = 0;
        unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_SCROLL_UP_ROWS,
                (&raw mut rows).cast(),
            );
        }
        rows
    }

    pub(crate) fn screen_generation(&self) -> u64 {
        let mut generation: u64 = 0;
        unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_SCREEN_GENERATION,
                (&raw mut generation).cast(),
            );
        }
        generation
    }

    pub(crate) fn sync_hold_reopens(&self) -> u64 {
        let mut reopens: u64 = 0;
        unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_SYNC_HOLD_REOPENS,
                (&raw mut reopens).cast(),
            );
        }
        reopens
    }

    /// Fast zero-allocation, short-circuiting check whether any cell on the
    /// active screen carries non-whitespace, non-control visible text.
    pub(crate) fn active_has_visible_text(&self) -> bool {
        let cols = self.cols();
        let rows = self.rows();
        for y in 0..u32::from(rows) {
            for x in 0..cols {
                let mut gref = sys::GhosttyGridRef::default();
                if unsafe {
                    sys::ghostty_terminal_grid_ref(
                        self.raw,
                        make_point(GridPointTag::Active, x, y),
                        &mut gref,
                    )
                } != sys::GHOSTTY_SUCCESS
                {
                    continue;
                }
                let mut cell: sys::GhosttyCell = 0;
                if unsafe { sys::ghostty_grid_ref_cell(&gref, &mut cell) } != sys::GHOSTTY_SUCCESS {
                    continue;
                }
                let mut cp: u32 = 0;
                unsafe {
                    sys::ghostty_cell_get(
                        cell,
                        sys::GHOSTTY_CELL_DATA_CODEPOINT,
                        (&raw mut cp).cast(),
                    );
                }
                if cp != 0 && cp != u32::from(b' ') {
                    if let Some(ch) = char::from_u32(cp) {
                        if !ch.is_whitespace() && !ch.is_control() {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Fast zero-allocation scan of the active viewport returning
    /// `(has_visible_text, trimmed_rows_hash)`. Used only on control-only
    /// chunks when hidden (or scrolled back) to detect screen mutations without
    /// invoking `RenderState::update` or allocating a `ScreenData` cell buffer.
    pub(crate) fn active_visible_text_state(&self) -> (bool, u64) {
        const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
        #[inline]
        fn mix_u32(hash: &mut u64, value: u32) {
            for b in value.to_le_bytes() {
                *hash ^= u64::from(b);
                *hash = hash.wrapping_mul(FNV_PRIME);
            }
        }

        let cols = self.cols();
        let rows = self.rows();
        let mut has_visible = false;
        let mut hash = FNV_OFFSET;
        mix_u32(&mut hash, u32::from(cols));
        mix_u32(&mut hash, u32::from(rows));

        for y in 0..u32::from(rows) {
            // Find the last non-blank column on this row so trailing spaces
            // match `ScreenView::visible_lines()` trimming semantics.
            let mut last_non_blank: Option<u16> = None;
            for x in (0..cols).rev() {
                let mut gref = sys::GhosttyGridRef::default();
                if unsafe {
                    sys::ghostty_terminal_grid_ref(
                        self.raw,
                        make_point(GridPointTag::Active, x, y),
                        &mut gref,
                    )
                } != sys::GHOSTTY_SUCCESS
                {
                    continue;
                }
                let mut cell: sys::GhosttyCell = 0;
                if unsafe { sys::ghostty_grid_ref_cell(&gref, &mut cell) } != sys::GHOSTTY_SUCCESS {
                    continue;
                }
                let mut cp: u32 = 0;
                unsafe {
                    sys::ghostty_cell_get(
                        cell,
                        sys::GHOSTTY_CELL_DATA_CODEPOINT,
                        (&raw mut cp).cast(),
                    );
                }
                if cp != 0 && cp != u32::from(b' ') {
                    last_non_blank = Some(x);
                    break;
                }
            }
            if let Some(end_x) = last_non_blank {
                for x in 0..=end_x {
                    let mut gref = sys::GhosttyGridRef::default();
                    if unsafe {
                        sys::ghostty_terminal_grid_ref(
                            self.raw,
                            make_point(GridPointTag::Active, x, y),
                            &mut gref,
                        )
                    } != sys::GHOSTTY_SUCCESS
                    {
                        mix_u32(&mut hash, u32::from(b' '));
                        continue;
                    }
                    let mut cell: sys::GhosttyCell = 0;
                    if unsafe { sys::ghostty_grid_ref_cell(&gref, &mut cell) }
                        != sys::GHOSTTY_SUCCESS
                    {
                        mix_u32(&mut hash, u32::from(b' '));
                        continue;
                    }
                    let mut cp: u32 = 0;
                    unsafe {
                        sys::ghostty_cell_get(
                            cell,
                            sys::GHOSTTY_CELL_DATA_CODEPOINT,
                            (&raw mut cp).cast(),
                        );
                    }
                    let normalized = if cp == 0 { u32::from(b' ') } else { cp };
                    mix_u32(&mut hash, normalized);
                    if !has_visible && cp != 0 {
                        if let Some(ch) = char::from_u32(cp) {
                            if !ch.is_whitespace() && !ch.is_control() {
                                has_visible = true;
                            }
                        }
                    }
                }
            }
            // Row delimiter so adjacent rows cannot collide.
            mix_u32(&mut hash, u32::MAX);
        }
        (has_visible, hash)
    }

    pub(crate) fn viewport_active(&self) -> bool {
        let mut active = true;
        unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_VIEWPORT_ACTIVE,
                (&raw mut active).cast(),
            );
        }
        active
    }

    pub(crate) fn vt_ground(&self) -> bool {
        let mut ground = true;
        unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_VT_GROUND,
                (&raw mut ground).cast(),
            );
        }
        ground
    }

    pub(crate) fn title(&self) -> String {
        read_terminal_string(self.raw, sys::GHOSTTY_TERMINAL_DATA_TITLE)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default()
    }

    pub(crate) fn pwd_bytes(&self) -> Vec<u8> {
        read_terminal_string(self.raw, sys::GHOSTTY_TERMINAL_DATA_PWD).unwrap_or_default()
    }

    pub(crate) fn color_foreground(&self) -> Option<Rgb> {
        read_terminal_rgb(self.raw, sys::GHOSTTY_TERMINAL_DATA_COLOR_FOREGROUND)
    }

    pub(crate) fn color_background(&self) -> Option<Rgb> {
        read_terminal_rgb(self.raw, sys::GHOSTTY_TERMINAL_DATA_COLOR_BACKGROUND)
    }

    pub(crate) fn color_cursor(&self) -> Option<Rgb> {
        read_terminal_rgb(self.raw, sys::GHOSTTY_TERMINAL_DATA_COLOR_CURSOR)
    }

    pub(crate) fn color_foreground_default(&self) -> Option<Rgb> {
        read_terminal_rgb(
            self.raw,
            sys::GHOSTTY_TERMINAL_DATA_COLOR_FOREGROUND_DEFAULT,
        )
    }

    pub(crate) fn color_background_default(&self) -> Option<Rgb> {
        read_terminal_rgb(
            self.raw,
            sys::GHOSTTY_TERMINAL_DATA_COLOR_BACKGROUND_DEFAULT,
        )
    }

    pub(crate) fn color_cursor_default(&self) -> Option<Rgb> {
        read_terminal_rgb(self.raw, sys::GHOSTTY_TERMINAL_DATA_COLOR_CURSOR_DEFAULT)
    }

    pub(crate) fn color_palette(&self) -> [Rgb; 256] {
        read_terminal_palette(self.raw, sys::GHOSTTY_TERMINAL_DATA_COLOR_PALETTE)
    }

    pub(crate) fn color_palette_default(&self) -> [Rgb; 256] {
        read_terminal_palette(self.raw, sys::GHOSTTY_TERMINAL_DATA_COLOR_PALETTE_DEFAULT)
    }

    pub(crate) fn cursor_sgr_style(&self) -> Style {
        let mut raw = sys::GhosttyStyle::default();
        unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_CURSOR_SGR_STYLE,
                (&raw mut raw).cast(),
            );
        }
        Style::from_raw(raw)
    }

    pub(crate) fn mouse_tracking(&self) -> bool {
        let mut active = false;
        unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_MOUSE_TRACKING,
                (&raw mut active).cast(),
            );
        }
        active
    }

    pub(crate) fn display_offset_from_bottom(&self) -> usize {
        let sb = self.scrollbar();
        let max_offset = sb.total.saturating_sub(sb.len);
        max_offset.saturating_sub(sb.offset) as usize
    }

    /// Format VT bytes preserving both primary (with up to `history` scrollback
    /// rows and soft-wrap markers intact for resize reflow) and alternate
    /// screens, saved and active cursors, full bit-0-capped Kitty keyboard
    /// stacks on both screens, modes, palette, tabs, styles, hyperlinks, and
    /// title stack.
    pub(crate) fn format_vt_with_history(&self, history: usize) -> Result<Vec<u8>> {
        let blob = self.encode_snapshot()?;
        let parsed = crate::snapshot::parse_snapshot_blob(&blob).ok_or(VtError::InvalidValue)?;
        let primary_meta = parsed.screen(0).ok_or(VtError::InvalidValue)?;
        let alt_meta = parsed.screen(1);
        let origin_active = self.mode(dec_mode(6));
        let scroll_top = parsed.scroll_top;
        // Omit Ghostty's single-entry `EXTRA_KITTY_KEYBOARD` (`CSI = N;1u`) so
        // we replay the full per-screen Kitty stack depth (capped to bit 0).
        let extras_without_kitty =
            sys::GHOSTTY_FORMATTER_EXTRAS_ALL & !sys::GHOSTTY_FORMATTER_EXTRA_KITTY_KEYBOARD;

        let rows = self.rows();
        let mut out = if self.active_screen() == ActiveScreen::Alternate {
            self.format_dual_screen_vt(
                &blob,
                primary_meta,
                alt_meta,
                history,
                scroll_top,
                parsed.scroll_bottom,
                origin_active,
                extras_without_kitty,
            )?
        } else {
            let mut bytes = Vec::new();
            if let Some(alt) = alt_meta {
                if alt.saved_cursor_present || !alt.kitty_stack.is_empty() {
                    bytes.extend_from_slice(b"\x1b[?1047h");
                    if alt.saved_cursor_present {
                        let alt_saved_pending =
                            alt.saved_cursor
                                .input_needs_wrap
                                .then(|| PendingWrapReplay::Blank {
                                    style: Style::default(),
                                    protected: false,
                                });
                        emit_saved_cursor_vt(
                            &mut bytes,
                            alt,
                            0,
                            rows.saturating_sub(1),
                            rows,
                            false,
                            alt_saved_pending.as_ref(),
                        );
                    }
                    emit_kitty_stack_vt(&mut bytes, &alt.kitty_stack);
                    bytes.extend_from_slice(b"\x1b[?1047l");
                }
            }
            let primary_vt = self.format_active_screen_vt(&blob, history, extras_without_kitty)?;
            bytes.extend_from_slice(&primary_vt);
            let need_wrap_fixup = self.has_blank_soft_wrap();
            if primary_meta.saved_cursor_present || need_wrap_fixup {
                let wrap_off = !self.mode(dec_mode(7));
                let insert_on = self.mode(ansi_mode(4));
                if wrap_off {
                    bytes.extend_from_slice(b"\x1b[?7h");
                }
                if insert_on {
                    bytes.extend_from_slice(b"\x1b[4l");
                }
                if need_wrap_fixup {
                    self.emit_blank_soft_wrap_fixups(
                        &mut bytes,
                        scroll_top,
                        parsed.scroll_bottom,
                        rows,
                        origin_active,
                    );
                }
                if primary_meta.saved_cursor_present {
                    let saved_pending = self.format_pending_wrap_cell_at(
                        primary_meta.saved_cursor.point,
                        primary_meta.saved_cursor.input_needs_wrap,
                    );
                    emit_saved_cursor_vt(
                        &mut bytes,
                        primary_meta,
                        scroll_top,
                        parsed.scroll_bottom,
                        rows,
                        origin_active,
                        saved_pending.as_ref(),
                    );
                }
                let pending_cell = self.format_pending_wrap_cell_at(
                    primary_meta.cursor.point,
                    primary_meta.cursor.input_needs_wrap,
                );
                emit_active_cursor_vt(
                    &mut bytes,
                    primary_meta,
                    scroll_top,
                    origin_active,
                    pending_cell.as_ref(),
                    primary_meta.saved_cursor_present,
                );
                if insert_on {
                    bytes.extend_from_slice(b"\x1b[4h");
                }
                if wrap_off {
                    bytes.extend_from_slice(b"\x1b[?7l");
                }
            }
            emit_kitty_stack_vt(&mut bytes, &primary_meta.kitty_stack);
            bytes
        };
        append_title_state_vt(&mut out, &self.title_stack(), &self.title());
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn format_dual_screen_vt(
        &self,
        blob: &[u8],
        primary_meta: &crate::snapshot::ParsedScreenMeta,
        alt_meta: Option<&crate::snapshot::ParsedScreenMeta>,
        history: usize,
        scroll_top: u16,
        scroll_bottom: u16,
        origin_active: bool,
        extras_without_kitty: sys::GhosttyFormatterExtras,
    ) -> Result<Vec<u8>> {
        let decode_cfg = TerminalConfig {
            cols: self.cols(),
            rows: self.rows(),
            scrollback_lines: usize::MAX,
            continuation_max_bytes: DEFAULT_CONTINUATION_MAX_BYTES,
            kitty_image_storage_limit: 0,
            enable_host_callbacks: false,
            enable_title_events: false,
            enable_render_hold: false,
            ..TerminalConfig::default()
        };
        let mut primary = Terminal::decode_snapshot(
            blob,
            SnapshotDecodeOptions {
                max_continuation_bytes: DEFAULT_CONTINUATION_MAX_BYTES,
                retain_continuation: false,
                compress_history: false,
                kitty_image_storage_limit: 0,
            },
            &decode_cfg,
        )?;
        // Switch clone to primary solely to format primary's grid cells with
        // soft wraps intact; cursor/saved_cursor come directly from `primary_meta`
        // because `1047l` copies the alternate cursor onto primary.
        primary.vt_write(b"\x1b[?1047l");
        let _ = primary.take_effects();
        let _ = primary.set_mode(dec_mode(7), true);

        let rows = self.rows();
        let mut out =
            primary.format_active_screen_vt_inner(history, sys::GHOSTTY_FORMATTER_EXTRAS_NONE)?;
        if primary.has_blank_soft_wrap() {
            primary.emit_blank_soft_wrap_fixups(&mut out, 0, rows.saturating_sub(1), rows, false);
        }
        let alt_1049 = self.mode(dec_mode(1049));
        let primary_pending = primary.format_pending_wrap_cell_at(
            primary_meta.cursor.point,
            primary_meta.cursor.input_needs_wrap,
        );

        if alt_1049 {
            // Replay starts on full-screen default margins (`0..rows-1`), so
            // enabling DECOM here captures `saved_cursor_origin = true` while
            // positioning every absolute row without clamping against margins
            // that the alternate-screen program later changed.
            let primary_origin = primary_meta.saved_cursor_origin;
            if primary_origin {
                out.extend_from_slice(b"\x1b[?6h");
            }
            emit_active_cursor_vt(
                &mut out,
                primary_meta,
                0,
                primary_origin,
                primary_pending.as_ref(),
                false,
            );
            emit_kitty_stack_vt(&mut out, &primary_meta.kitty_stack);
            out.extend_from_slice(b"\x1b[?1049h");
            if primary_origin {
                out.extend_from_slice(b"\x1b[?6l");
            }
        } else {
            if primary_meta.saved_cursor_present {
                let primary_saved_pending = primary.format_pending_wrap_cell_at(
                    primary_meta.saved_cursor.point,
                    primary_meta.saved_cursor.input_needs_wrap,
                );
                emit_saved_cursor_vt(
                    &mut out,
                    primary_meta,
                    0,
                    rows.saturating_sub(1),
                    rows,
                    false,
                    primary_saved_pending.as_ref(),
                );
            }
            emit_active_cursor_vt(
                &mut out,
                primary_meta,
                0,
                false,
                primary_pending.as_ref(),
                primary_meta.saved_cursor_present,
            );
            emit_kitty_stack_vt(&mut out, &primary_meta.kitty_stack);
            if self.mode(dec_mode(1047)) {
                out.extend_from_slice(b"\x1b[?1047h");
            } else {
                out.extend_from_slice(b"\x1b[?47h");
            }
        }
        // Reset alternate cursor and pen defaults so primary's pen/charset/cursor
        // never bleed into the alternate screen formatter output.
        out.extend_from_slice(b"\x1b[0m\x1b]8;;\x1b\\\x1b(B\x0f\x1b[0\"q\x1b[H");

        let alt_vt = self.format_active_screen_vt(blob, usize::MAX, extras_without_kitty)?;
        out.extend_from_slice(&alt_vt);
        if let Some(alt) = alt_meta {
            let need_wrap_fixup = self.has_blank_soft_wrap();
            if alt.saved_cursor_present || need_wrap_fixup {
                let wrap_off = !self.mode(dec_mode(7));
                let insert_on = self.mode(ansi_mode(4));
                if wrap_off {
                    out.extend_from_slice(b"\x1b[?7h");
                }
                if insert_on {
                    out.extend_from_slice(b"\x1b[4l");
                }
                if need_wrap_fixup {
                    self.emit_blank_soft_wrap_fixups(
                        &mut out,
                        scroll_top,
                        scroll_bottom,
                        rows,
                        origin_active,
                    );
                }
                if alt.saved_cursor_present {
                    let alt_saved_pending = self.format_pending_wrap_cell_at(
                        alt.saved_cursor.point,
                        alt.saved_cursor.input_needs_wrap,
                    );
                    emit_saved_cursor_vt(
                        &mut out,
                        alt,
                        scroll_top,
                        scroll_bottom,
                        rows,
                        origin_active,
                        alt_saved_pending.as_ref(),
                    );
                }
                let alt_pending =
                    self.format_pending_wrap_cell_at(alt.cursor.point, alt.cursor.input_needs_wrap);
                emit_active_cursor_vt(
                    &mut out,
                    alt,
                    scroll_top,
                    origin_active,
                    alt_pending.as_ref(),
                    alt.saved_cursor_present,
                );
                if insert_on {
                    out.extend_from_slice(b"\x1b[4h");
                }
                if wrap_off {
                    out.extend_from_slice(b"\x1b[?7l");
                }
            }
            emit_kitty_stack_vt(&mut out, &alt.kitty_stack);
        }
        Ok(out)
    }

    fn has_blank_soft_wrap(&self) -> bool {
        let cols = self.cols();
        let rows = self.rows();
        if cols == 0 || rows < 2 {
            return false;
        }
        for y in 0..u32::from(rows - 1) {
            if let Some(gref) = self.grid_ref(GridPointTag::Active, cols - 1, y) {
                if let (Ok(row), Ok(cell)) = (gref.row(), gref.cell()) {
                    if row.wrap && !cell.has_text && cell.wide == CellWide::Narrow {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Reconstruct blank-ended soft-wrapped rows (`row[y].wrap == true` with an
    /// unwritten right-edge cell) that `libghostty-vt`'s VT formatter skips via
    /// `blank_rows` (`\r\n`), so soft-wrap copy parity holds across attach.
    /// Assumes `DECAWM` is on; caller restores cursor, pen, charsets and `DECAWM`.
    fn emit_blank_soft_wrap_fixups(
        &self,
        out: &mut Vec<u8>,
        scroll_top: u16,
        scroll_bottom: u16,
        rows: u16,
        origin_active: bool,
    ) {
        use std::io::Write as _;
        let cols = self.cols();
        if cols == 0 || rows < 2 {
            return;
        }
        let custom_margins = scroll_top != 0 || scroll_bottom != rows.saturating_sub(1);
        if custom_margins {
            out.extend_from_slice(b"\x1b[r");
        }
        if origin_active {
            out.extend_from_slice(b"\x1b[?6l");
        }

        let fmt_opts = FormatOptions {
            kind: FormatKind::Vt,
            unwrap: false,
            trim: false,
            extras: sys::GHOSTTY_FORMATTER_EXTRAS_NONE,
        };

        for y in 0..u32::from(rows - 1) {
            let Some(last_ref) = self.grid_ref(GridPointTag::Active, cols - 1, y) else {
                continue;
            };
            let (Ok(row), Ok(last_cell)) = (last_ref.row(), last_ref.cell()) else {
                continue;
            };
            if !row.wrap || last_cell.has_text || last_cell.wide != CellWide::Narrow {
                continue;
            }
            let last_style = blank_style_from_grid_ref(&last_ref, &last_cell);
            out.extend_from_slice(b"\x1b]8;;\x1b\\\x1b(B\x0f");
            out.extend_from_slice(if last_cell.protected {
                b"\x1b[1\"q"
            } else {
                b"\x1b[0\"q"
            });
            emit_sgr_vt(out, &last_style);
            let _ = write!(out, "\x1b[{};{}H ", y + 1, cols);

            let Some(next_ref) = self.grid_ref(GridPointTag::Active, 0, y + 1) else {
                continue;
            };
            let Ok(next_cell) = next_ref.cell() else {
                continue;
            };
            if next_cell.has_text || next_cell.wide == CellWide::Wide {
                let end_x = if next_cell.wide == CellWide::Wide && cols > 1 {
                    1
                } else {
                    0
                };
                if let Some(end_ref) = self.grid_ref(GridPointTag::Active, end_x, y + 1) {
                    if let Ok(sel) = Selection::new(next_ref, end_ref, false) {
                        if let Ok(vt_bytes) = self.format_selection(fmt_opts, &sel) {
                            out.extend_from_slice(b"\x1b[0m\x1b]8;;\x1b\\\x1b(B\x0f");
                            out.extend_from_slice(if next_cell.protected {
                                b"\x1b[1\"q"
                            } else {
                                b"\x1b[0\"q"
                            });
                            out.extend_from_slice(&vt_bytes);
                            out.extend_from_slice(b"\x1b]8;;\x1b\\");
                        }
                    }
                }
            } else {
                let next_style = blank_style_from_grid_ref(&next_ref, &next_cell);
                out.extend_from_slice(b"\x1b]8;;\x1b\\\x1b(B\x0f");
                out.extend_from_slice(if next_cell.protected {
                    b"\x1b[1\"q"
                } else {
                    b"\x1b[0\"q"
                });
                emit_sgr_vt(out, &next_style);
                out.push(b' ');
            }
        }

        out.extend_from_slice(b"\x1b[0\"q\x1b]8;;\x1b\\");
        if custom_margins {
            let _ = write!(out, "\x1b[{};{}r", scroll_top + 1, scroll_bottom + 1);
        }
        if origin_active {
            out.extend_from_slice(b"\x1b[?6h");
        }
    }

    fn format_pending_wrap_cell_at(
        &self,
        point: crate::types::GridPoint,
        input_needs_wrap: bool,
    ) -> Option<PendingWrapReplay> {
        if !input_needs_wrap {
            return None;
        }
        let cols = self.cols();
        let rows = self.rows();
        let x = u16::try_from(point.column.0).ok()?;
        let y = u32::try_from(point.line.0).ok()?;
        if x >= cols || y >= u32::from(rows) {
            return None;
        }
        let gref = self.grid_ref(GridPointTag::Active, x, y)?;
        let cell = gref.cell().ok()?;
        if cell.wide != CellWide::SpacerTail && !cell.has_text {
            let mut blank_style = gref.style().unwrap_or_default();
            match cell.content_tag {
                CellContentTag::BgColorPalette => {
                    if let Some(idx) = cell.bg_palette {
                        blank_style.bg = StyleColor::Palette(idx);
                    }
                }
                CellContentTag::BgColorRgb => {
                    if let Some(rgb) = cell.bg_rgb {
                        blank_style.bg = StyleColor::Rgb(rgb);
                    }
                }
                _ => {}
            }
            return Some(PendingWrapReplay::Blank {
                style: blank_style,
                protected: cell.protected,
            });
        }
        let start_x = if cell.wide == CellWide::SpacerTail {
            x.saturating_sub(1)
        } else {
            x
        };
        let start_ref = self.grid_ref(GridPointTag::Active, start_x, y)?;
        let protected = start_ref
            .cell()
            .map(|c| c.protected)
            .unwrap_or(cell.protected);
        let end_ref = self.grid_ref(GridPointTag::Active, x, y)?;
        let sel = Selection::new(start_ref, end_ref, false).ok()?;
        let opts = FormatOptions {
            kind: FormatKind::Vt,
            unwrap: false,
            trim: false,
            extras: sys::GHOSTTY_FORMATTER_EXTRAS_NONE,
        };
        self.format_selection(opts, &sel)
            .ok()
            .map(|vt_bytes| PendingWrapReplay::Cell {
                start_x,
                vt_bytes,
                protected,
            })
    }

    fn format_active_screen_vt(
        &self,
        blob: &[u8],
        history: usize,
        extras: sys::GhosttyFormatterExtras,
    ) -> Result<Vec<u8>> {
        // When DECAWM (mode 7) is disabled or IRM (ANSI mode 4) is enabled,
        // `EXTRA_MODES` emits them before the grid cells and pending-wrap
        // reprint, preventing soft wraps or shifting cells during replay.
        // Format on a clone in replace+wrap mode and restore them at the end.
        let wrap_off = !self.mode(dec_mode(7));
        let insert_on = self.mode(ansi_mode(4));
        if (extras & sys::GHOSTTY_FORMATTER_EXTRA_MODES) != 0 && (wrap_off || insert_on) {
            let decode_cfg = TerminalConfig {
                cols: self.cols(),
                rows: self.rows(),
                scrollback_lines: usize::MAX,
                continuation_max_bytes: DEFAULT_CONTINUATION_MAX_BYTES,
                kitty_image_storage_limit: 0,
                enable_host_callbacks: false,
                enable_title_events: false,
                enable_render_hold: false,
                ..TerminalConfig::default()
            };
            let mut clone = Terminal::decode_snapshot(
                blob,
                SnapshotDecodeOptions {
                    max_continuation_bytes: DEFAULT_CONTINUATION_MAX_BYTES,
                    retain_continuation: false,
                    compress_history: false,
                    kitty_image_storage_limit: 0,
                },
                &decode_cfg,
            )?;
            if wrap_off {
                let _ = clone.set_mode(dec_mode(7), true);
            }
            if insert_on {
                let _ = clone.set_mode(ansi_mode(4), false);
            }
            let mut bytes = clone.format_active_screen_vt_inner(history, extras)?;
            if insert_on {
                bytes.extend_from_slice(b"\x1b[4h");
            }
            if wrap_off {
                bytes.extend_from_slice(b"\x1b[?7l");
            }
            return Ok(bytes);
        }
        self.format_active_screen_vt_inner(history, extras)
    }

    fn format_active_screen_vt_inner(
        &self,
        history: usize,
        extras: sys::GhosttyFormatterExtras,
    ) -> Result<Vec<u8>> {
        let opts = FormatOptions {
            kind: FormatKind::Vt,
            unwrap: true,
            trim: false,
            extras,
        };
        let scrollback = self.scrollback_rows();
        if history >= scrollback {
            return self.format(opts);
        }
        let cols = self.cols();
        let total = self.total_rows();
        if cols == 0 || total == 0 {
            return self.format(opts);
        }
        let start_y = (scrollback - history) as u32;
        let end_y = (total - 1) as u32;
        let Some(start_ref) = self.grid_ref(GridPointTag::Screen, 0, start_y) else {
            return self.format(opts);
        };
        let Some(end_ref) = self.grid_ref(GridPointTag::Screen, cols - 1, end_y) else {
            return self.format(opts);
        };
        let sel = Selection::new(start_ref, end_ref, false)?;
        self.format_selection(opts, &sel)
    }

    /// Format a single row in VT mode without extras (used lazily when a row
    /// carries OSC 8 hyperlinks to recover explicit `id=` parameters).
    pub(crate) fn format_row_vt(&self, tag: GridPointTag, y: u32) -> Option<Vec<u8>> {
        let cols = self.cols();
        if cols == 0 {
            return None;
        }
        let start = self.grid_ref(tag, 0, y)?;
        let end = self.grid_ref(tag, cols - 1, y)?;
        let sel = Selection::new(start, end, false).ok()?;
        let opts = FormatOptions {
            kind: FormatKind::Vt,
            unwrap: false,
            trim: false,
            extras: sys::GHOSTTY_FORMATTER_EXTRAS_NONE,
        };
        self.format_selection(opts, &sel).ok()
    }

    /// Read the last printed character (for `CSI b` REP continuation in
    /// companion snapshots) from Ghostty's binary snapshot TERMINAL header.
    pub(crate) fn previous_char(&self) -> Option<char> {
        let blob = self.encode_snapshot().ok()?;
        parse_snapshot_previous_char(&blob)
    }

    pub(crate) fn memory_usage(&self) -> Result<sys::GhosttyTerminalMemoryUsage> {
        let mut usage = sys::GhosttyTerminalMemoryUsage {
            size: size_of::<sys::GhosttyTerminalMemoryUsage>(),
            ..Default::default()
        };
        check(unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_MEMORY_USAGE,
                (&raw mut usage).cast(),
            )
        })?;
        Ok(usage)
    }

    pub(crate) fn continuation(&self) -> Result<Vec<u8>> {
        unsafe {
            take_ghostty_alloc(|ptr, len| {
                sys::ghostty_terminal_continuation_alloc(self.raw, ptr::null(), ptr, len)
            })
        }
    }

    pub(crate) fn encode_snapshot(&self) -> Result<Vec<u8>> {
        let mut bytes = unsafe {
            take_ghostty_alloc(|ptr, len| {
                sys::ghostty_snapshot_encode_alloc(self.raw, ptr::null(), ptr, len)
            })
        }?;
        let stack = self.title_stack();
        if !stack.is_empty() {
            append_title_stack_footer(&mut bytes, &stack);
        }
        Ok(bytes)
    }

    pub(crate) fn format(&self, opts: FormatOptions) -> Result<Vec<u8>> {
        self.format_raw(opts, None)
    }

    pub(crate) fn format_selection(
        &self,
        opts: FormatOptions,
        selection: &Selection<'_>,
    ) -> Result<Vec<u8>> {
        self.verify_selection(selection)?;
        self.format_raw(opts, Some(&selection.raw))
    }

    pub(crate) fn format_active_selection(&self, opts: FormatOptions) -> Result<Option<Vec<u8>>> {
        let Some(selection) = self.selection() else {
            return Ok(None);
        };
        self.format_raw(opts, Some(&selection.raw)).map(Some)
    }

    fn format_raw(
        &self,
        opts: FormatOptions,
        selection: Option<&sys::GhosttySelection>,
    ) -> Result<Vec<u8>> {
        let mut fmt_handle: sys::GhosttyFormatter = ptr::null_mut();
        check(unsafe {
            sys::ghostty_formatter_terminal_new(ptr::null(), &mut fmt_handle, self.raw)
        })?;
        struct FormatterGuard(sys::GhosttyFormatter);
        impl Drop for FormatterGuard {
            fn drop(&mut self) {
                unsafe { sys::ghostty_formatter_free(self.0) };
            }
        }
        let _guard = FormatterGuard(fmt_handle);

        let emit = match opts.kind {
            FormatKind::Plain => sys::GHOSTTY_FORMATTER_FORMAT_PLAIN,
            FormatKind::Vt => sys::GHOSTTY_FORMATTER_FORMAT_VT,
            FormatKind::Html => sys::GHOSTTY_FORMATTER_FORMAT_HTML,
        };
        unsafe {
            check(sys::ghostty_formatter_set(
                fmt_handle,
                sys::GHOSTTY_FORMATTER_OPT_EMIT,
                (&raw const emit).cast(),
            ))?;
            check(sys::ghostty_formatter_set(
                fmt_handle,
                sys::GHOSTTY_FORMATTER_OPT_UNWRAP,
                (&raw const opts.unwrap).cast(),
            ))?;
            check(sys::ghostty_formatter_set(
                fmt_handle,
                sys::GHOSTTY_FORMATTER_OPT_TRIM,
                (&raw const opts.trim).cast(),
            ))?;
            if opts.kind == FormatKind::Vt {
                check(sys::ghostty_formatter_set(
                    fmt_handle,
                    sys::GHOSTTY_FORMATTER_OPT_EXTRAS,
                    (&raw const opts.extras).cast(),
                ))?;
            }
            if let Some(sel) = selection {
                check(sys::ghostty_formatter_set(
                    fmt_handle,
                    sys::GHOSTTY_FORMATTER_OPT_SELECTION,
                    (sel as *const sys::GhosttySelection).cast(),
                ))?;
            }
        }
        unsafe {
            take_ghostty_alloc(|ptr, len| {
                sys::ghostty_formatter_format_alloc(fmt_handle, ptr::null(), ptr, len)
            })
        }
    }

    pub(crate) fn grid_ref(&self, tag: GridPointTag, x: u16, y: u32) -> Option<GridRef<'_>> {
        let pt = make_point(tag, x, y);
        let mut out = sys::GhosttyGridRef::default();
        let rc = unsafe { sys::ghostty_terminal_grid_ref(self.raw, pt, &mut out) };
        (rc == sys::GHOSTTY_SUCCESS).then(|| GridRef {
            raw: out,
            owner_id: self.id,
            screen: self.active_screen(),
            _term: PhantomData,
        })
    }

    pub(crate) fn row_is_wrapped(&self, screen_y: u32) -> bool {
        let pt = make_point(GridPointTag::Screen, 0, screen_y);
        let mut gref = sys::GhosttyGridRef::default();
        if unsafe { sys::ghostty_terminal_grid_ref(self.raw, pt, &mut gref) }
            != sys::GHOSTTY_SUCCESS
        {
            return false;
        }
        let mut raw_row: sys::GhosttyRow = 0;
        if unsafe { sys::ghostty_grid_ref_row(&gref, &mut raw_row) } != sys::GHOSTTY_SUCCESS {
            return false;
        }
        let mut wrap = false;
        unsafe {
            sys::ghostty_row_get(raw_row, sys::GHOSTTY_ROW_DATA_WRAP, (&raw mut wrap).cast());
        }
        wrap
    }

    pub(crate) fn row_has_hyperlink(&self, screen_y: u32) -> bool {
        let pt = make_point(GridPointTag::Screen, 0, screen_y);
        let mut gref = sys::GhosttyGridRef::default();
        if unsafe { sys::ghostty_terminal_grid_ref(self.raw, pt, &mut gref) }
            != sys::GHOSTTY_SUCCESS
        {
            return false;
        }
        let mut raw_row: sys::GhosttyRow = 0;
        if unsafe { sys::ghostty_grid_ref_row(&gref, &mut raw_row) } != sys::GHOSTTY_SUCCESS {
            return false;
        }
        let mut hyperlink = false;
        unsafe {
            sys::ghostty_row_get(
                raw_row,
                sys::GHOSTTY_ROW_DATA_HYPERLINK,
                (&raw mut hyperlink).cast(),
            );
        }
        hyperlink
    }

    pub(crate) fn row_has_underline(&self, screen_y: u32) -> bool {
        let pt = make_point(GridPointTag::Screen, 0, screen_y);
        let mut gref = sys::GhosttyGridRef::default();
        if unsafe { sys::ghostty_terminal_grid_ref(self.raw, pt, &mut gref) }
            != sys::GHOSTTY_SUCCESS
        {
            return false;
        }
        let mut raw_row: sys::GhosttyRow = 0;
        if unsafe { sys::ghostty_grid_ref_row(&gref, &mut raw_row) } != sys::GHOSTTY_SUCCESS {
            return false;
        }
        let mut styled = false;
        unsafe {
            sys::ghostty_row_get(
                raw_row,
                sys::GHOSTTY_ROW_DATA_STYLED,
                (&raw mut styled).cast(),
            );
        }
        if !styled {
            return false;
        }
        let cols = self.cols();
        for col in 0..cols {
            let Some(cell_ref) = self.grid_ref(GridPointTag::Screen, col, screen_y) else {
                break;
            };
            let Ok(style) = cell_ref.style() else {
                continue;
            };
            if super::style_underline(style.flags) != super::UnderlineStyle::None {
                return true;
            }
        }
        false
    }

    /// Append a single screen row's base text cells (skipping wide spacers) for
    /// lazy regex search without copying styles, colors, or hyperlinks.
    pub(crate) fn append_row_search_cells(
        &self,
        screen_y: u32,
        line: crate::types::Line,
        text: &mut String,
        map: &mut Vec<(usize, crate::types::GridPoint, bool)>,
    ) {
        let cols = self.cols();
        for col in 0..cols {
            let pt = make_point(GridPointTag::Screen, col, screen_y);
            let mut gref = sys::GhosttyGridRef::default();
            if unsafe { sys::ghostty_terminal_grid_ref(self.raw, pt, &mut gref) }
                != sys::GHOSTTY_SUCCESS
            {
                break;
            }
            let mut raw_cell: sys::GhosttyCell = 0;
            if unsafe { sys::ghostty_grid_ref_cell(&gref, &mut raw_cell) } != sys::GHOSTTY_SUCCESS {
                break;
            }
            let mut wide: sys::GhosttyCellWide = sys::GHOSTTY_CELL_WIDE_NARROW;
            unsafe {
                sys::ghostty_cell_get(
                    raw_cell,
                    sys::GHOSTTY_CELL_DATA_WIDE,
                    (&raw mut wide).cast(),
                );
            }
            if wide == sys::GHOSTTY_CELL_WIDE_SPACER_TAIL
                || wide == sys::GHOSTTY_CELL_WIDE_SPACER_HEAD
            {
                continue;
            }
            let mut codepoint: u32 = 0;
            unsafe {
                sys::ghostty_cell_get(
                    raw_cell,
                    sys::GHOSTTY_CELL_DATA_CODEPOINT,
                    (&raw mut codepoint).cast(),
                );
            }
            let ch = if codepoint == 0 {
                ' '
            } else {
                char::from_u32(codepoint).unwrap_or(' ')
            };
            map.push((
                text.len(),
                crate::types::GridPoint::new(line, crate::types::Column(usize::from(col))),
                wide == sys::GHOSTTY_CELL_WIDE_WIDE,
            ));
            text.push(ch);
        }
    }

    pub(crate) fn track_grid_ref(
        &self,
        tag: GridPointTag,
        x: u16,
        y: u32,
    ) -> Result<TrackedGridRef> {
        let pt = make_point(tag, x, y);
        let mut raw: sys::GhosttyTrackedGridRef = ptr::null_mut();
        check(unsafe { sys::ghostty_terminal_grid_ref_track(self.raw, pt, &mut raw) })?;
        Ok(TrackedGridRef {
            raw,
            owner_id: self.id,
            screen: self.active_screen(),
            _not_send_sync: PhantomData,
        })
    }

    pub(crate) fn point_from_grid_ref(
        &self,
        grid_ref: &GridRef<'_>,
        tag: GridPointTag,
    ) -> Result<Option<(u16, u32)>> {
        self.verify_grid_ref(grid_ref)?;
        let mut coord = sys::GhosttyPointCoordinate::default();
        let rc = unsafe {
            sys::ghostty_terminal_point_from_grid_ref(
                self.raw,
                &grid_ref.raw,
                tag.to_raw(),
                &mut coord,
            )
        };
        match rc {
            sys::GHOSTTY_SUCCESS => Ok(Some((coord.x, coord.y))),
            sys::GHOSTTY_NO_VALUE => Ok(None),
            other => Err(check(other).unwrap_err()),
        }
    }

    pub(crate) fn clear_selection(&mut self) -> Result<()> {
        check(unsafe {
            sys::ghostty_terminal_set(self.raw, sys::GHOSTTY_TERMINAL_OPT_SELECTION, ptr::null())
        })
    }

    pub(crate) fn set_selection_points(
        &mut self,
        tag: GridPointTag,
        start: (u16, u32),
        end: (u16, u32),
        rectangle: bool,
    ) -> Result<()> {
        let mut start_ref = sys::GhosttyGridRef::default();
        let mut end_ref = sys::GhosttyGridRef::default();
        check(unsafe {
            sys::ghostty_terminal_grid_ref(
                self.raw,
                make_point(tag, start.0, start.1),
                &mut start_ref,
            )
        })?;
        check(unsafe {
            sys::ghostty_terminal_grid_ref(self.raw, make_point(tag, end.0, end.1), &mut end_ref)
        })?;
        let sel = sys::GhosttySelection {
            start: start_ref,
            end: end_ref,
            rectangle,
        };
        check(unsafe {
            sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_SELECTION,
                (&raw const sel).cast(),
            )
        })
    }

    pub(crate) fn select_and_set_word(
        &mut self,
        tag: GridPointTag,
        x: u16,
        y: u32,
        boundaries: Option<&[u32]>,
    ) -> Result<bool> {
        let mut grid_ref = sys::GhosttyGridRef::default();
        if unsafe { sys::ghostty_terminal_grid_ref(self.raw, make_point(tag, x, y), &mut grid_ref) }
            != sys::GHOSTTY_SUCCESS
        {
            return Ok(false);
        }
        let opts = sys::GhosttyTerminalSelectWordOptions {
            size: size_of::<sys::GhosttyTerminalSelectWordOptions>(),
            ref_: grid_ref,
            boundary_codepoints: boundaries.map_or(ptr::null(), |s| s.as_ptr()),
            boundary_codepoints_len: boundaries.map_or(0, <[u32]>::len),
        };
        let mut sel = sys::GhosttySelection::default();
        let rc = unsafe { sys::ghostty_terminal_select_word(self.raw, &opts, &mut sel) };
        if rc == sys::GHOSTTY_NO_VALUE {
            return Ok(false);
        }
        check(rc)?;
        check(unsafe {
            sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_SELECTION,
                (&raw const sel).cast(),
            )
        })?;
        Ok(true)
    }

    pub(crate) fn select_and_set_line(
        &mut self,
        tag: GridPointTag,
        x: u16,
        y: u32,
        whitespace: Option<&[u32]>,
        semantic_prompt_boundary: bool,
    ) -> Result<bool> {
        let mut grid_ref = sys::GhosttyGridRef::default();
        if unsafe { sys::ghostty_terminal_grid_ref(self.raw, make_point(tag, x, y), &mut grid_ref) }
            != sys::GHOSTTY_SUCCESS
        {
            return Ok(false);
        }
        let opts = sys::GhosttyTerminalSelectLineOptions {
            size: size_of::<sys::GhosttyTerminalSelectLineOptions>(),
            ref_: grid_ref,
            whitespace: whitespace.map_or(ptr::null(), |s| s.as_ptr()),
            whitespace_len: whitespace.map_or(0, <[u32]>::len),
            semantic_prompt_boundary,
        };
        let mut sel = sys::GhosttySelection::default();
        let rc = unsafe { sys::ghostty_terminal_select_line(self.raw, &opts, &mut sel) };
        if rc == sys::GHOSTTY_NO_VALUE {
            return Ok(false);
        }
        check(rc)?;
        check(unsafe {
            sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_SELECTION,
                (&raw const sel).cast(),
            )
        })?;
        Ok(true)
    }

    pub(crate) fn select_and_set_all(&mut self) -> Result<bool> {
        let mut sel = sys::GhosttySelection::default();
        let rc = unsafe { sys::ghostty_terminal_select_all(self.raw, &mut sel) };
        if rc == sys::GHOSTTY_NO_VALUE {
            return Ok(false);
        }
        check(rc)?;
        check(unsafe {
            sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_SELECTION,
                (&raw const sel).cast(),
            )
        })?;
        Ok(true)
    }

    pub(crate) fn adjust_active_selection(&mut self, adjust: SelectionAdjust) -> Result<bool> {
        let mut sel = sys::GhosttySelection::default();
        let rc = unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_SELECTION,
                (&raw mut sel).cast(),
            )
        };
        if rc == sys::GHOSTTY_NO_VALUE {
            return Ok(false);
        }
        check(rc)?;
        check(unsafe {
            sys::ghostty_terminal_selection_adjust(self.raw, &mut sel, adjust.to_raw())
        })?;
        check(unsafe {
            sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_SELECTION,
                (&raw const sel).cast(),
            )
        })?;
        Ok(true)
    }

    pub(crate) fn selection(&self) -> Option<Selection<'_>> {
        let mut raw = sys::GhosttySelection::default();
        let rc = unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_SELECTION,
                (&raw mut raw).cast(),
            )
        };
        (rc == sys::GHOSTTY_SUCCESS).then(|| unsafe { self.wrap_selection(raw) })
    }

    pub(crate) fn selection_span(
        &self,
        selection: &Selection<'_>,
        tag: GridPointTag,
    ) -> Result<Option<SelectionSpan>> {
        self.verify_selection(selection)?;
        let Some(start) = self.point_from_grid_ref(&selection.start(), tag)? else {
            return Ok(None);
        };
        let Some(end) = self.point_from_grid_ref(&selection.end(), tag)? else {
            return Ok(None);
        };
        Ok(Some(SelectionSpan {
            start,
            end,
            rectangle: selection.rectangle(),
        }))
    }

    pub(crate) fn active_selection_span(&self, tag: GridPointTag) -> Option<SelectionSpan> {
        let sel = self.selection()?;
        self.selection_span(&sel, tag).ok().flatten()
    }

    pub(crate) fn select_word<'a>(
        &'a self,
        grid_ref: GridRef<'a>,
        boundaries: Option<&[u32]>,
    ) -> Result<Option<Selection<'a>>> {
        self.verify_grid_ref(&grid_ref)?;
        let opts = sys::GhosttyTerminalSelectWordOptions {
            size: size_of::<sys::GhosttyTerminalSelectWordOptions>(),
            ref_: grid_ref.raw,
            boundary_codepoints: boundaries.map_or(ptr::null(), |s| s.as_ptr()),
            boundary_codepoints_len: boundaries.map_or(0, <[u32]>::len),
        };
        let mut out = sys::GhosttySelection::default();
        let rc = unsafe { sys::ghostty_terminal_select_word(self.raw, &opts, &mut out) };
        match rc {
            sys::GHOSTTY_SUCCESS => Ok(Some(unsafe { self.wrap_selection(out) })),
            sys::GHOSTTY_NO_VALUE => Ok(None),
            other => Err(check(other).unwrap_err()),
        }
    }

    pub(crate) fn select_line<'a>(
        &'a self,
        grid_ref: GridRef<'a>,
        whitespace: Option<&[u32]>,
        semantic_prompt_boundary: bool,
    ) -> Result<Option<Selection<'a>>> {
        self.verify_grid_ref(&grid_ref)?;
        let opts = sys::GhosttyTerminalSelectLineOptions {
            size: size_of::<sys::GhosttyTerminalSelectLineOptions>(),
            ref_: grid_ref.raw,
            whitespace: whitespace.map_or(ptr::null(), |s| s.as_ptr()),
            whitespace_len: whitespace.map_or(0, <[u32]>::len),
            semantic_prompt_boundary,
        };
        let mut out = sys::GhosttySelection::default();
        let rc = unsafe { sys::ghostty_terminal_select_line(self.raw, &opts, &mut out) };
        match rc {
            sys::GHOSTTY_SUCCESS => Ok(Some(unsafe { self.wrap_selection(out) })),
            sys::GHOSTTY_NO_VALUE => Ok(None),
            other => Err(check(other).unwrap_err()),
        }
    }

    pub(crate) fn select_all(&self) -> Option<Selection<'_>> {
        let mut out = sys::GhosttySelection::default();
        let rc = unsafe { sys::ghostty_terminal_select_all(self.raw, &mut out) };
        (rc == sys::GHOSTTY_SUCCESS).then(|| unsafe { self.wrap_selection(out) })
    }

    pub(crate) fn selection_order(&self, selection: &Selection<'_>) -> Result<SelectionOrder> {
        self.verify_selection(selection)?;
        let mut out = sys::GHOSTTY_SELECTION_ORDER_FORWARD;
        check(unsafe {
            sys::ghostty_terminal_selection_order(self.raw, &selection.raw, &mut out)
        })?;
        Ok(SelectionOrder::from_raw(out))
    }

    pub(crate) fn selection_ordered<'a>(
        &'a self,
        selection: &Selection<'a>,
        desired: SelectionOrder,
    ) -> Result<Selection<'a>> {
        self.verify_selection(selection)?;
        let mut out = sys::GhosttySelection::default();
        check(unsafe {
            sys::ghostty_terminal_selection_ordered(
                self.raw,
                &selection.raw,
                desired.to_raw(),
                &mut out,
            )
        })?;
        Ok(unsafe { self.wrap_selection(out) })
    }

    pub(crate) fn selection_contains(
        &self,
        selection: &Selection<'_>,
        tag: GridPointTag,
        x: u16,
        y: u32,
    ) -> Result<bool> {
        self.verify_selection(selection)?;
        let pt = make_point(tag, x, y);
        let mut contains = false;
        check(unsafe {
            sys::ghostty_terminal_selection_contains(self.raw, &selection.raw, pt, &mut contains)
        })?;
        Ok(contains)
    }

    pub(crate) fn selection_adjust<'a>(
        &'a self,
        selection: &mut Selection<'a>,
        adjust: SelectionAdjust,
    ) -> Result<()> {
        self.verify_selection(selection)?;
        check(unsafe {
            sys::ghostty_terminal_selection_adjust(self.raw, &mut selection.raw, adjust.to_raw())
        })
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // Free the C handle before reclaiming `inbox` so no callback can run on
        // freed memory.
        unsafe {
            sys::ghostty_terminal_free(self.raw);
            drop(Box::from_raw(self.inbox.as_ptr()));
        }
    }
}

fn make_point(tag: GridPointTag, x: u16, y: u32) -> sys::GhosttyPoint {
    sys::GhosttyPoint {
        tag: tag.to_raw(),
        value: sys::GhosttyPointValue {
            coordinate: sys::GhosttyPointCoordinate { x, y },
        },
    }
}

fn read_terminal_string(
    raw: sys::GhosttyTerminal,
    data: sys::GhosttyTerminalData,
) -> Option<Vec<u8>> {
    let mut s = sys::GhosttyString::default();
    let rc = unsafe { sys::ghostty_terminal_get(raw, data, (&raw mut s).cast()) };
    if rc != sys::GHOSTTY_SUCCESS || s.ptr.is_null() || s.len == 0 {
        return Some(Vec::new());
    }
    Some(unsafe { slice::from_raw_parts(s.ptr, s.len) }.to_vec())
}

fn read_terminal_rgb(raw: sys::GhosttyTerminal, data: sys::GhosttyTerminalData) -> Option<Rgb> {
    let mut c = sys::GhosttyColorRgb::default();
    let rc = unsafe { sys::ghostty_terminal_get(raw, data, (&raw mut c).cast()) };
    (rc == sys::GHOSTTY_SUCCESS).then_some(Rgb {
        r: c.r,
        g: c.g,
        b: c.b,
    })
}

fn read_terminal_palette(raw: sys::GhosttyTerminal, data: sys::GhosttyTerminalData) -> [Rgb; 256] {
    let mut buf = [sys::GhosttyColorRgb::default(); 256];
    unsafe {
        sys::ghostty_terminal_get(raw, data, buf.as_mut_ptr().cast());
    }
    std::array::from_fn(|i| Rgb {
        r: buf[i].r,
        g: buf[i].g,
        b: buf[i].b,
    })
}

/// Extract `previous_codepoint` (offset 25..29 of the TERMINAL payload) from a
/// v1 Ghostty binary snapshot (`envelope.zig`, `record.zig`, `terminal.zig`).
pub(crate) fn parse_snapshot_previous_char(blob: &[u8]) -> Option<char> {
    if blob.len() < 10 + 10 + 103 || &blob[..8] != b"GHOSTSNP" {
        return None;
    }
    let version = u16::from_le_bytes([blob[8], blob[9]]);
    if version != 1 {
        return None;
    }
    let tag = u16::from_le_bytes([blob[10], blob[11]]);
    let payload_len = u32::from_le_bytes([blob[12], blob[13], blob[14], blob[15]]) as usize;
    if tag != 1 || payload_len < 103 || blob.len() < 20 + payload_len {
        return None;
    }
    let payload = &blob[20..20 + payload_len];
    let cp = u32::from_le_bytes([payload[25], payload[26], payload[27], payload[28]]);
    if cp == u32::MAX {
        None
    } else {
        char::from_u32(cp)
    }
}

fn ghostty_string_bytes<'a>(s: sys::GhosttyString) -> &'a [u8] {
    if s.ptr.is_null() || s.len == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(s.ptr, s.len) }
    }
}

fn ghostty_string_owned(s: sys::GhosttyString) -> String {
    String::from_utf8_lossy(ghostty_string_bytes(s)).into_owned()
}

fn ghostty_string_opt(s: sys::GhosttyString) -> Option<String> {
    let bytes = ghostty_string_bytes(s);
    (!bytes.is_empty()).then(|| String::from_utf8_lossy(bytes).into_owned())
}

fn is_utf8_plain_text_mime(mime: &[u8]) -> bool {
    let mut parts = mime.split(|&b| b == b';');
    let Some(media_type) = parts.next() else {
        return false;
    };
    if !media_type.trim_ascii().eq_ignore_ascii_case(b"text/plain") {
        return false;
    }
    for param in parts {
        let param = param.trim_ascii();
        if param.is_empty() {
            continue;
        }
        let Some(eq) = param.iter().position(|&b| b == b'=') else {
            return false;
        };
        let key = param[..eq].trim_ascii();
        let val = param[eq + 1..].trim_ascii();
        let val = val
            .strip_prefix(b"\"")
            .and_then(|v| v.strip_suffix(b"\""))
            .unwrap_or(val);
        if !key.eq_ignore_ascii_case(b"charset") || !val.eq_ignore_ascii_case(b"utf-8") {
            return false;
        }
    }
    true
}

#[inline]
unsafe fn push_effect(userdata: *mut c_void, effect: TerminalEffect) {
    let cell = unsafe { &*userdata.cast::<UnsafeCell<EffectInbox>>() };
    unsafe { (*cell.get()).effects.push(effect) };
}

unsafe extern "C" fn on_write_pty(
    _terminal: sys::GhosttyTerminal,
    userdata: *mut c_void,
    data: *const u8,
    len: usize,
) {
    if userdata.is_null() || data.is_null() || len == 0 {
        return;
    }
    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        let bytes = unsafe { slice::from_raw_parts(data, len) }.to_vec();
        unsafe { push_effect(userdata, TerminalEffect::WritePty(bytes)) };
    }));
}

unsafe extern "C" fn on_bell(_terminal: sys::GhosttyTerminal, userdata: *mut c_void) {
    if userdata.is_null() {
        return;
    }
    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        unsafe { push_effect(userdata, TerminalEffect::Bell) };
    }));
}

const TITLE_STACK_FOOTER_MAGIC: &[u8; 4] = b"RNTS";

fn append_title_stack_footer(out: &mut Vec<u8>, stack: &[Option<String>]) {
    let mut payload = Vec::new();
    payload.extend_from_slice(&(stack.len() as u32).to_le_bytes());
    for entry in stack {
        match entry {
            None => payload.push(0),
            Some(title) => {
                payload.push(1);
                payload.extend_from_slice(&(title.len() as u32).to_le_bytes());
                payload.extend_from_slice(title.as_bytes());
            }
        }
    }
    out.extend_from_slice(TITLE_STACK_FOOTER_MAGIC);
    out.extend_from_slice(&payload);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(TITLE_STACK_FOOTER_MAGIC);
}

fn strip_title_stack_footer(bytes: &[u8]) -> (&[u8], Vec<Option<String>>) {
    if bytes.len() < 12 || &bytes[bytes.len() - 4..] != TITLE_STACK_FOOTER_MAGIC {
        return (bytes, Vec::new());
    }
    let len_off = bytes.len() - 8;
    let payload_len = u32::from_le_bytes([
        bytes[len_off],
        bytes[len_off + 1],
        bytes[len_off + 2],
        bytes[len_off + 3],
    ]) as usize;
    let Some(header_off) = len_off.checked_sub(payload_len + 4) else {
        return (bytes, Vec::new());
    };
    if &bytes[header_off..header_off + 4] != TITLE_STACK_FOOTER_MAGIC {
        return (bytes, Vec::new());
    }
    let payload = &bytes[header_off + 4..len_off];
    if payload.len() < 4 {
        return (bytes, Vec::new());
    }
    let count = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
    if count > TITLE_STACK_MAX_DEPTH {
        return (bytes, Vec::new());
    }
    let mut pos = 4usize;
    let mut stack = Vec::with_capacity(count);
    for _ in 0..count {
        let Some(&tag) = payload.get(pos) else {
            return (bytes, Vec::new());
        };
        pos += 1;
        match tag {
            0 => stack.push(None),
            1 => {
                if pos + 4 > payload.len() {
                    return (bytes, Vec::new());
                }
                let slen = u32::from_le_bytes([
                    payload[pos],
                    payload[pos + 1],
                    payload[pos + 2],
                    payload[pos + 3],
                ]) as usize;
                pos += 4;
                let Some(slice) = payload.get(pos..pos + slen) else {
                    return (bytes, Vec::new());
                };
                pos += slen;
                let Ok(s) = std::str::from_utf8(slice) else {
                    return (bytes, Vec::new());
                };
                stack.push(Some(s.to_owned()));
            }
            _ => return (bytes, Vec::new()),
        }
    }
    if pos != payload.len() {
        return (bytes, Vec::new());
    }
    (&bytes[..header_off], stack)
}

fn append_title_state_vt(out: &mut Vec<u8>, stack: &[Option<String>], current: &str) {
    if stack.is_empty() && current.is_empty() {
        return;
    }
    for entry in stack {
        out.extend_from_slice(b"\x1b]2;");
        if let Some(title) = entry {
            out.extend_from_slice(title.as_bytes());
        }
        out.extend_from_slice(b"\x1b\\\x1b[22;2t");
    }
    out.extend_from_slice(b"\x1b]2;");
    out.extend_from_slice(current.as_bytes());
    out.extend_from_slice(b"\x1b\\");
}

fn emit_kitty_stack_vt(out: &mut Vec<u8>, stack: &[u8]) {
    if stack.is_empty() {
        return;
    }
    if stack.len() >= 8 {
        let bottom = stack[0] & 1;
        let _ = std::io::Write::write_fmt(out, format_args!("\x1b[={bottom};1u"));
        for &entry in &stack[1..8] {
            let flags = entry & 1;
            let _ = std::io::Write::write_fmt(out, format_args!("\x1b[>{flags}u"));
        }
    } else {
        for &entry in stack {
            let flags = entry & 1;
            let _ = std::io::Write::write_fmt(out, format_args!("\x1b[>{flags}u"));
        }
    }
}

enum PendingWrapReplay {
    Cell {
        start_x: u16,
        vt_bytes: Vec<u8>,
        protected: bool,
    },
    Blank {
        style: Style,
        protected: bool,
    },
}

fn emit_sgr_vt(out: &mut Vec<u8>, style: &Style) {
    use std::io::Write as _;
    out.extend_from_slice(b"\x1b[0");
    if style.flags & super::STYLE_FLAG_BOLD != 0 {
        out.extend_from_slice(b";1");
    }
    if style.flags & super::STYLE_FLAG_FAINT != 0 {
        out.extend_from_slice(b";2");
    }
    if style.flags & super::STYLE_FLAG_ITALIC != 0 {
        out.extend_from_slice(b";3");
    }
    match super::style_underline(style.flags) {
        super::UnderlineStyle::None => {}
        super::UnderlineStyle::Single => out.extend_from_slice(b";4"),
        super::UnderlineStyle::Double => out.extend_from_slice(b";4:2"),
        super::UnderlineStyle::Curly => out.extend_from_slice(b";4:3"),
        super::UnderlineStyle::Dotted => out.extend_from_slice(b";4:4"),
        super::UnderlineStyle::Dashed => out.extend_from_slice(b";4:5"),
    }
    if style.flags & super::STYLE_FLAG_BLINK != 0 {
        out.extend_from_slice(b";5");
    }
    if style.flags & super::STYLE_FLAG_INVERSE != 0 {
        out.extend_from_slice(b";7");
    }
    if style.flags & super::STYLE_FLAG_INVISIBLE != 0 {
        out.extend_from_slice(b";8");
    }
    if style.flags & super::STYLE_FLAG_STRIKETHROUGH != 0 {
        out.extend_from_slice(b";9");
    }
    if style.flags & super::STYLE_FLAG_OVERLINE != 0 {
        out.extend_from_slice(b";53");
    }
    match style.fg {
        StyleColor::None => {}
        StyleColor::Palette(idx) => {
            let _ = write!(out, ";38;5;{idx}");
        }
        StyleColor::Rgb(rgb) => {
            let _ = write!(out, ";38;2;{};{};{}", rgb.r, rgb.g, rgb.b);
        }
    }
    match style.bg {
        StyleColor::None => {}
        StyleColor::Palette(idx) => {
            let _ = write!(out, ";48;5;{idx}");
        }
        StyleColor::Rgb(rgb) => {
            let _ = write!(out, ";48;2;{};{};{}", rgb.r, rgb.g, rgb.b);
        }
    }
    match style.underline {
        StyleColor::None => {}
        StyleColor::Palette(idx) => {
            let _ = write!(out, ";58;5;{idx}");
        }
        StyleColor::Rgb(rgb) => {
            let _ = write!(out, ";58;2;{};{};{}", rgb.r, rgb.g, rgb.b);
        }
    }
    out.push(b'm');
}

fn emit_charsets_vt(
    out: &mut Vec<u8>,
    raw_charsets: &[u8; 4],
    active: crate::snapshot::SnapshotCharsetIndex,
) {
    use crate::snapshot::SnapshotCharsetIndex as Idx;
    for (slot, &cs) in (*b"()*+").into_iter().zip(raw_charsets.iter()) {
        // Ghostty `charsets.Charset`: 0 = utf8, 1 = ascii, 2 = british, 3 = dec_special.
        let code = match cs {
            2 => b'A',
            3 => b'0',
            _ => b'B',
        };
        out.extend_from_slice(&[0x1b, slot, code]);
    }
    match active {
        Idx::G0 => out.push(0x0f),
        Idx::G1 => out.push(0x0e),
        Idx::G2 => out.extend_from_slice(b"\x1bn"),
        Idx::G3 => out.extend_from_slice(b"\x1bo"),
    }
}

fn emit_cursor_position_vt(
    out: &mut Vec<u8>,
    x: u16,
    rel_y: u16,
    pending_wrap: Option<&PendingWrapReplay>,
    preserve_saved_cursor: bool,
) {
    use std::io::Write as _;
    match pending_wrap {
        Some(PendingWrapReplay::Cell {
            start_x,
            vt_bytes,
            protected,
        }) => {
            // Reset SGR, close any open OSC 8 hyperlink, reset GL to ASCII, and
            // set DECSCA to the cell's own protection bit before reprinting so
            // the existing margin cell keeps its exact attributes.
            out.extend_from_slice(b"\x1b[0m\x1b]8;;\x1b\\\x1b(B\x0f");
            out.extend_from_slice(if *protected {
                b"\x1b[1\"q"
            } else {
                b"\x1b[0\"q"
            });
            let _ = write!(out, "\x1b[{};{}H", rel_y + 1, start_x + 1);
            out.extend_from_slice(vt_bytes);
            out.extend_from_slice(b"\x1b]8;;\x1b\\");
        }
        Some(PendingWrapReplay::Blank { style, protected }) => {
            out.extend_from_slice(b"\x1b]8;;\x1b\\\x1b(B\x0f");
            out.extend_from_slice(if *protected {
                b"\x1b[1\"q"
            } else {
                b"\x1b[0\"q"
            });
            emit_sgr_vt(out, style);
            let _ = write!(out, "\x1b[{};{}H", rel_y + 1, x + 1);
            if preserve_saved_cursor {
                out.push(b' ');
            } else {
                out.extend_from_slice(b" \x1b7\x1b[X\x1b8");
            }
        }
        None => {
            let _ = write!(out, "\x1b[{};{}H", rel_y + 1, x + 1);
        }
    }
}

fn emit_saved_cursor_vt(
    out: &mut Vec<u8>,
    meta: &crate::snapshot::ParsedScreenMeta,
    scroll_top: u16,
    scroll_bottom: u16,
    rows: u16,
    origin_active: bool,
    pending_wrap: Option<&PendingWrapReplay>,
) {
    use std::io::Write as _;
    let sc_origin = meta.saved_cursor_origin;
    let y = meta.saved_cursor.point.line.0.max(0) as u16;
    let x = meta.saved_cursor.point.column.0 as u16;
    let custom_margins = scroll_top != 0 || scroll_bottom != rows.saturating_sub(1);
    let relax_margins = sc_origin && custom_margins;
    if relax_margins {
        // Temporarily clear DECSTBM so a cursor saved with DECOM on can be
        // reconstructed at its exact absolute row even if margins changed later.
        out.extend_from_slice(b"\x1b[r");
    }
    if sc_origin != origin_active {
        out.extend_from_slice(if sc_origin { b"\x1b[?6h" } else { b"\x1b[?6l" });
    }
    let rel_y = if sc_origin && !relax_margins {
        y.saturating_sub(scroll_top)
    } else {
        y
    };
    emit_cursor_position_vt(out, x, rel_y, pending_wrap, false);
    emit_sgr_vt(out, &meta.saved_cursor_style_vt);
    emit_charsets_vt(
        out,
        &meta.saved_raw_charsets,
        meta.saved_cursor_active_charset,
    );
    out.extend_from_slice(if meta.saved_cursor_protected {
        b"\x1b[1\"q"
    } else {
        b"\x1b[0\"q"
    });
    out.extend_from_slice(b"\x1b7");
    if relax_margins {
        let _ = write!(out, "\x1b[{};{}r", scroll_top + 1, scroll_bottom + 1);
    }
    if sc_origin != origin_active {
        out.extend_from_slice(if origin_active {
            b"\x1b[?6h"
        } else {
            b"\x1b[?6l"
        });
    }
}

fn blank_style_from_grid_ref(gref: &GridRef<'_>, cell: &CellInfo) -> Style {
    let mut style = gref.style().unwrap_or_default();
    match cell.content_tag {
        CellContentTag::BgColorPalette => {
            if let Some(idx) = cell.bg_palette {
                style.bg = StyleColor::Palette(idx);
            }
        }
        CellContentTag::BgColorRgb => {
            if let Some(rgb) = cell.bg_rgb {
                style.bg = StyleColor::Rgb(rgb);
            }
        }
        _ => {}
    }
    style
}

fn emit_active_cursor_vt(
    out: &mut Vec<u8>,
    meta: &crate::snapshot::ParsedScreenMeta,
    scroll_top: u16,
    origin_active: bool,
    pending_wrap: Option<&PendingWrapReplay>,
    preserve_saved_cursor: bool,
) {
    use std::io::Write as _;
    let y = meta.cursor.point.line.0.max(0) as u16;
    let x = meta.cursor.point.column.0 as u16;
    let rel_y = if origin_active {
        y.saturating_sub(scroll_top)
    } else {
        y
    };
    emit_cursor_position_vt(out, x, rel_y, pending_wrap, preserve_saved_cursor);
    emit_sgr_vt(out, &meta.cursor_style_vt);
    if let Some(link) = meta.cursor.template.hyperlink.as_ref() {
        if link.id().is_empty() {
            let _ = write!(out, "\x1b]8;;{}\x1b\\", link.uri());
        } else {
            let _ = write!(out, "\x1b]8;id={};{}\x1b\\", link.id(), link.uri());
        }
    } else {
        out.extend_from_slice(b"\x1b]8;;\x1b\\");
    }
    out.extend_from_slice(if meta.cursor_protected {
        b"\x1b[1\"q"
    } else {
        b"\x1b[0\"q"
    });
    emit_charsets_vt(out, &meta.raw_charsets, meta.active_charset);
}

unsafe extern "C" fn on_title_changed(terminal: sys::GhosttyTerminal, userdata: *mut c_void) {
    if userdata.is_null() {
        return;
    }
    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        let title = read_terminal_string(terminal, sys::GHOSTTY_TERMINAL_DATA_TITLE)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        let cell = unsafe { &*userdata.cast::<UnsafeCell<EffectInbox>>() };
        let inbox = unsafe { &mut *cell.get() };
        if inbox.emit_title_and_reset {
            inbox.effects.push(TerminalEffect::TitleChanged(title));
        }
    }));
}

unsafe extern "C" fn on_title_push(
    terminal: sys::GhosttyTerminal,
    userdata: *mut c_void,
    param: u16,
) {
    if userdata.is_null() || (param != 0 && param != 2) {
        return;
    }
    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        let raw_bytes =
            read_terminal_string(terminal, sys::GHOSTTY_TERMINAL_DATA_TITLE).unwrap_or_default();
        let current =
            (!raw_bytes.is_empty()).then(|| String::from_utf8_lossy(&raw_bytes).into_owned());
        let cell = unsafe { &*userdata.cast::<UnsafeCell<EffectInbox>>() };
        let inbox = unsafe { &mut *cell.get() };
        inbox.title_stack.push(current);
        if inbox.title_stack.len() > TITLE_STACK_MAX_DEPTH {
            inbox.title_stack.remove(0);
        }
    }));
}

unsafe extern "C" fn on_title_pop(
    terminal: sys::GhosttyTerminal,
    userdata: *mut c_void,
    param: u16,
) {
    if userdata.is_null() || (param != 0 && param != 2) {
        return;
    }
    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        let (popped, emit) = {
            let cell = unsafe { &*userdata.cast::<UnsafeCell<EffectInbox>>() };
            let inbox = unsafe { &mut *cell.get() };
            (inbox.title_stack.pop(), inbox.emit_title_and_reset)
        };
        let Some(restored) = popped else {
            return;
        };
        let title_str = restored.as_deref().unwrap_or("");
        let raw = sys::GhosttyString {
            ptr: title_str.as_ptr(),
            len: title_str.len(),
        };
        // Restore Ghostty's live title with no Rust borrow on `EffectInbox` live.
        let _ = unsafe {
            sys::ghostty_terminal_set(
                terminal,
                sys::GHOSTTY_TERMINAL_OPT_TITLE,
                (&raw const raw).cast(),
            )
        };
        if emit {
            unsafe { push_effect(userdata, TerminalEffect::TitleChanged(title_str.to_owned())) };
        }
    }));
}

unsafe extern "C" fn on_pwd_changed(terminal: sys::GhosttyTerminal, userdata: *mut c_void) {
    if userdata.is_null() {
        return;
    }
    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        let pwd =
            read_terminal_string(terminal, sys::GHOSTTY_TERMINAL_DATA_PWD).unwrap_or_default();
        unsafe { push_effect(userdata, TerminalEffect::PwdChanged(pwd)) };
    }));
}

unsafe extern "C" fn on_size(
    terminal: sys::GhosttyTerminal,
    userdata: *mut c_void,
    out: *mut sys::GhosttySizeReportSize,
) -> bool {
    if userdata.is_null() || out.is_null() {
        return false;
    }
    let res = panic::catch_unwind(AssertUnwindSafe(|| {
        let (cell_width, cell_height) = {
            let cell = unsafe { &*userdata.cast::<UnsafeCell<EffectInbox>>() };
            unsafe { (*cell.get()).cell_size_px }
        };
        let (mut cols, mut rows) = (0u16, 0u16);
        unsafe {
            sys::ghostty_terminal_get(
                terminal,
                sys::GHOSTTY_TERMINAL_DATA_COLS,
                (&raw mut cols).cast(),
            );
            sys::ghostty_terminal_get(
                terminal,
                sys::GHOSTTY_TERMINAL_DATA_ROWS,
                (&raw mut rows).cast(),
            );
            *out = sys::GhosttySizeReportSize {
                rows,
                columns: cols,
                cell_width,
                cell_height,
            };
        }
    }));
    res.is_ok()
}

unsafe extern "C" fn on_color_scheme(
    _terminal: sys::GhosttyTerminal,
    userdata: *mut c_void,
    out: *mut sys::GhosttyColorScheme,
) -> bool {
    if userdata.is_null() || out.is_null() {
        return false;
    }
    let res = panic::catch_unwind(AssertUnwindSafe(|| {
        let scheme = {
            let cell = unsafe { &*userdata.cast::<UnsafeCell<EffectInbox>>() };
            unsafe { (*cell.get()).color_scheme }
        };
        unsafe { *out = scheme.to_raw() };
    }));
    res.is_ok()
}

unsafe extern "C" fn on_device_attributes(
    _terminal: sys::GhosttyTerminal,
    _userdata: *mut c_void,
    out: *mut sys::GhosttyDeviceAttributes,
) -> bool {
    if out.is_null() {
        return false;
    }
    let attrs = unsafe { &mut *out };
    // DA1: VT102 with no feature list -> `ESC[?6c`.
    attrs.primary.conformance_level = sys::GHOSTTY_DA_CONFORMANCE_VT102 as u16;
    attrs.primary.num_features = 0;
    // DA2: Runner's established secondary identity -> `ESC[>0;2600;1c`.
    attrs.secondary.device_type = 0;
    attrs.secondary.firmware_version = 2600;
    attrs.secondary.rom_cartridge = 1;
    // DA3: native Ghostty tertiary identity -> `DCS !|00000000 ST`.
    attrs.tertiary.unit_id = 0;
    true
}

unsafe extern "C" fn on_clipboard_write(
    _terminal: sys::GhosttyTerminal,
    userdata: *mut c_void,
    write: *const sys::GhosttyClipboardWrite,
) {
    if userdata.is_null() || write.is_null() {
        return;
    }
    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        let max_clipboard_bytes = {
            let cell = unsafe { &*userdata.cast::<UnsafeCell<EffectInbox>>() };
            unsafe { (*cell.get()).max_clipboard_bytes }
        };
        let req = unsafe { &*write };
        let location = match req.location {
            sys::GHOSTTY_CLIPBOARD_LOCATION_SELECTION => ClipboardLocation::Selection,
            sys::GHOSTTY_CLIPBOARD_LOCATION_PRIMARY => ClipboardLocation::Primary,
            _ => ClipboardLocation::Standard,
        };

        // Runner's host clipboard commits a single non-empty UTF-8 `text/plain`
        // value up to `max_clipboard_bytes`. Reject clears, multi-representation
        // writes (cannot be committed atomically), non-text MIME types, oversize
        // writes, and invalid UTF-8 before queuing or acknowledging.
        let outcome: std::result::Result<String, sys::GhosttyClipboardWriteResult> =
            if req.contents_len == 0 {
                Err(sys::GHOSTTY_CLIPBOARD_WRITE_RESULT_UNSUPPORTED)
            } else if req.contents.is_null() {
                Err(sys::GHOSTTY_CLIPBOARD_WRITE_RESULT_INVALID_DATA)
            } else if req.contents_len != 1 {
                Err(sys::GHOSTTY_CLIPBOARD_WRITE_RESULT_UNSUPPORTED)
            } else {
                let items = unsafe { slice::from_raw_parts(req.contents, req.contents_len) };
                let item = &items[0];
                if !is_utf8_plain_text_mime(ghostty_string_bytes(item.mime)) {
                    Err(sys::GHOSTTY_CLIPBOARD_WRITE_RESULT_UNSUPPORTED)
                } else {
                    let data = ghostty_string_bytes(item.data);
                    if data.is_empty() {
                        Err(sys::GHOSTTY_CLIPBOARD_WRITE_RESULT_UNSUPPORTED)
                    } else if data.len() > max_clipboard_bytes {
                        Err(sys::GHOSTTY_CLIPBOARD_WRITE_RESULT_DENIED)
                    } else {
                        match std::str::from_utf8(data) {
                            Ok(text) => Ok(text.to_owned()),
                            Err(_) => Err(sys::GHOSTTY_CLIPBOARD_WRITE_RESULT_INVALID_DATA),
                        }
                    }
                }
            };

        let reply_result = match outcome {
            Ok(text) => {
                unsafe { push_effect(userdata, TerminalEffect::ClipboardWrite { location, text }) };
                sys::GHOSTTY_CLIPBOARD_WRITE_RESULT_SUCCESS
            }
            Err(code) => code,
        };

        // Call `reply` with no Rust borrow on `EffectInbox` live, because OSC 5522
        // replies synchronously invoke `on_write_pty`.
        if let Some(reply_fn) = req.reply {
            let reply = sys::GhosttyClipboardWriteReply {
                size: size_of::<sys::GhosttyClipboardWriteReply>(),
                result: reply_result,
                remember: false,
            };
            unsafe { reply_fn(write, &reply) };
        }
    }));
}

unsafe extern "C" fn on_program_status(
    _terminal: sys::GhosttyTerminal,
    userdata: *mut c_void,
    report: *const sys::GhosttyTerminalProgramStatus,
) {
    if userdata.is_null() || report.is_null() {
        return;
    }
    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        let r = unsafe { &*report };
        let state = match r.state {
            sys::GHOSTTY_PROGRAM_STATUS_STATE_IDLE => ProgramStatusState::Idle,
            sys::GHOSTTY_PROGRAM_STATUS_STATE_WORKING => ProgramStatusState::Working,
            sys::GHOSTTY_PROGRAM_STATUS_STATE_DONE => ProgramStatusState::Done,
            sys::GHOSTTY_PROGRAM_STATUS_STATE_BLOCKED => ProgramStatusState::Blocked,
            sys::GHOSTTY_PROGRAM_STATUS_STATE_ERROR => ProgramStatusState::Error,
            sys::GHOSTTY_PROGRAM_STATUS_STATE_CLEAR => ProgramStatusState::Clear,
            _ => return,
        };
        let kind = match r.kind {
            sys::GHOSTTY_PROGRAM_STATUS_KIND_PERMISSION => Some(ProgramStatusKind::Permission),
            sys::GHOSTTY_PROGRAM_STATUS_KIND_QUESTION => Some(ProgramStatusKind::Question),
            sys::GHOSTTY_PROGRAM_STATUS_KIND_AUTH => Some(ProgramStatusKind::Auth),
            _ => None,
        };
        let progress = u8::try_from(r.progress).ok();
        let status = RawProgramStatus {
            state,
            kind,
            progress,
            id: ghostty_string_owned(r.id),
            app: ghostty_string_opt(r.app),
            title: ghostty_string_opt(r.title),
            message: ghostty_string_opt(r.message),
        };
        unsafe { push_effect(userdata, TerminalEffect::ProgramStatus(status)) };
    }));
}

unsafe extern "C" fn on_semantic_prompt(
    _terminal: sys::GhosttyTerminal,
    userdata: *mut c_void,
    event: *const sys::GhosttyTerminalSemanticPrompt,
) {
    if userdata.is_null() || event.is_null() {
        return;
    }
    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        let ev = unsafe { &*event };
        let kind = match ev.kind {
            sys::GHOSTTY_SEMANTIC_PROMPT_PROMPT_START => SemanticPromptKind::PromptStart,
            sys::GHOSTTY_SEMANTIC_PROMPT_INPUT_START => SemanticPromptKind::InputStart,
            sys::GHOSTTY_SEMANTIC_PROMPT_OUTPUT_START => SemanticPromptKind::OutputStart,
            sys::GHOSTTY_SEMANTIC_PROMPT_COMMAND_END => SemanticPromptKind::CommandEnd,
            _ => return,
        };
        let prompt_kind = match ev.prompt_kind {
            sys::GHOSTTY_SEMANTIC_PROMPT_PROMPT_RIGHT => PromptKind::Right,
            sys::GHOSTTY_SEMANTIC_PROMPT_PROMPT_CONTINUATION => PromptKind::Continuation,
            sys::GHOSTTY_SEMANTIC_PROMPT_PROMPT_SECONDARY => PromptKind::Secondary,
            _ => PromptKind::Primary,
        };
        let sem = SemanticPromptEvent {
            kind,
            prompt_kind,
            exit_code: ev.has_exit_code.then_some(ev.exit_code),
            command: ghostty_string_owned(ev.command),
            error: ghostty_string_owned(ev.error),
        };
        unsafe { push_effect(userdata, TerminalEffect::SemanticPrompt(sem)) };
    }));
}

unsafe extern "C" fn on_render_hold(
    _terminal: sys::GhosttyTerminal,
    userdata: *mut c_void,
    held: bool,
) {
    if userdata.is_null() {
        return;
    }
    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        unsafe { push_effect(userdata, TerminalEffect::RenderHold(held)) };
    }));
}

unsafe extern "C" fn on_reset(_terminal: sys::GhosttyTerminal, userdata: *mut c_void) {
    if userdata.is_null() {
        return;
    }
    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        let cell = unsafe { &*userdata.cast::<UnsafeCell<EffectInbox>>() };
        let inbox = unsafe { &mut *cell.get() };
        inbox.title_stack.clear();
        if inbox.emit_title_and_reset {
            inbox.effects.push(TerminalEffect::Reset);
        }
    }));
}
