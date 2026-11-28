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

use super::{check, init_sys, take_ghostty_alloc, Result, VtError};
use crate::palette::Rgb;
use crate::terminal::MAX_CLIPBOARD_BYTES;
use crate::types::CursorShape;

pub(crate) const DEFAULT_CONTINUATION_MAX_BYTES: usize = 64 * 1024;
pub(crate) const DEFAULT_SCROLLBACK_LINES: usize = 10_000;

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
}

/// Single-owner `!Send`/`!Sync` terminal state backed by `libghostty-vt`.
pub(crate) struct Terminal {
    id: u64,
    mutation_gen: u64,
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
        let mut decoder: sys::GhosttySnapshotDecoder = ptr::null_mut();
        check(unsafe {
            sys::ghostty_snapshot_decoder_new_buf(
                ptr::null(),
                &mut decoder,
                bytes.as_ptr(),
                bytes.len(),
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
        Ok(term)
    }

    fn from_raw(raw: sys::GhosttyTerminal, config: &TerminalConfig) -> Self {
        let inbox = NonNull::new(Box::into_raw(Box::new(UnsafeCell::new(EffectInbox {
            effects: Vec::new(),
            color_scheme: config.color_scheme,
            cell_size_px: config.cell_size_px,
            max_clipboard_bytes: config.clipboard_write_max_bytes,
        }))))
        .expect("Box::into_raw returned null");
        Self {
            id: NEXT_TERMINAL_ID.fetch_add(1, Ordering::Relaxed),
            mutation_gen: 0,
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
            if config.enable_render_hold {
                let cb: unsafe extern "C" fn(sys::GhosttyTerminal, *mut c_void, bool) =
                    on_render_hold;
                check(sys::ghostty_terminal_set(
                    self.raw,
                    sys::GHOSTTY_TERMINAL_OPT_RENDER_HOLD,
                    cb as *const c_void,
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
                let title: unsafe extern "C" fn(sys::GhosttyTerminal, *mut c_void) =
                    on_title_changed;
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
                let reset: unsafe extern "C" fn(sys::GhosttyTerminal, *mut c_void) = on_reset;

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
                    sys::GHOSTTY_TERMINAL_OPT_TITLE_CHANGED,
                    title as *const c_void,
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
                check(sys::ghostty_terminal_set(
                    self.raw,
                    sys::GHOSTTY_TERMINAL_OPT_RESET,
                    reset as *const c_void,
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

    #[inline]
    pub(super) fn mutation_gen(&self) -> u64 {
        self.mutation_gen
    }

    #[inline]
    pub(super) fn bump_mutation_gen(&mut self) {
        self.mutation_gen = self.mutation_gen.wrapping_add(1);
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
            self.bump_mutation_gen();
            unsafe { sys::ghostty_terminal_vt_write(self.raw, bytes.as_ptr(), bytes.len()) };
        }
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
        self.bump_mutation_gen();
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
        self.bump_mutation_gen();
        unsafe { sys::ghostty_terminal_reset(self.raw) };
    }

    pub(crate) fn scroll_viewport(&mut self, action: ScrollViewport) -> Result<()> {
        self.bump_mutation_gen();
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
        self.bump_mutation_gen();
        let unlimited_bytes: usize = usize::MAX;
        unsafe {
            check(sys::ghostty_terminal_set(
                self.raw,
                sys::GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_BYTES,
                (&raw const unlimited_bytes).cast(),
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

    pub(crate) fn color_palette(&self) -> [Rgb; 256] {
        let mut raw = [sys::GhosttyColorRgb::default(); 256];
        unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::GHOSTTY_TERMINAL_DATA_COLOR_PALETTE,
                raw.as_mut_ptr().cast(),
            );
        }
        std::array::from_fn(|i| Rgb {
            r: raw[i].r,
            g: raw[i].g,
            b: raw[i].b,
        })
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
        unsafe {
            take_ghostty_alloc(|ptr, len| {
                sys::ghostty_snapshot_encode_alloc(self.raw, ptr::null(), ptr, len)
            })
        }
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
        self.bump_mutation_gen();
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
        self.bump_mutation_gen();
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
        self.bump_mutation_gen();
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
        self.bump_mutation_gen();
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
        self.bump_mutation_gen();
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
        self.bump_mutation_gen();
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
        // Free the C handle before reclaiming `inbox` so no callback can run on freed memory.
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

unsafe extern "C" fn on_title_changed(terminal: sys::GhosttyTerminal, userdata: *mut c_void) {
    if userdata.is_null() {
        return;
    }
    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        let title = read_terminal_string(terminal, sys::GHOSTTY_TERMINAL_DATA_TITLE)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        unsafe { push_effect(userdata, TerminalEffect::TitleChanged(title)) };
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
        unsafe { push_effect(userdata, TerminalEffect::Reset) };
    }));
}
