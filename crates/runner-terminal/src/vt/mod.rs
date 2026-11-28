//! Safe, Runner-owned wrapper around `ghostty-vt-sys` (`libghostty-vt` `vt-1.0.2`).
//!
//! All `libghostty-vt` FFI handles are single-owner (`!Send` / `!Sync`) and stay
//! strictly internal to `runner-terminal`.

pub(crate) mod encode;
pub(crate) mod render;
pub(crate) mod search;
pub(crate) mod terminal;

#[cfg(test)]
mod tests;

use std::ffi::c_void;
use std::fmt;
use std::io::Cursor;
use std::panic::{self, AssertUnwindSafe};
use std::slice;
use std::sync::Once;

use ghostty_vt_sys as sys;

pub(crate) use encode::{
    encode_color_scheme_report, encode_focus, encode_paste, encode_size_report, paste_is_safe,
    KeyEncoder, MouseEncoder, SizeReportStyle,
};
pub(crate) use render::{RenderColors, RenderCursorInfo, RenderDirty, RenderRow, RenderState};
pub(crate) use search::{Search, SearchScroll, SearchStatus};
pub(crate) use terminal::{
    ActiveScreen, CellContentTag, CellInfo, CellWide, ClipboardLocation, ColorScheme, FormatKind,
    FormatOptions, GridPointTag, GridRef, ProgramStatusKind, ProgramStatusState, PromptKind,
    RawProgramStatus, RowInfo, ScrollViewport, Scrollbar, Selection, SelectionAdjust,
    SelectionOrder, SelectionSpan, SemanticPromptEvent, SemanticPromptKind, SnapshotDecodeOptions,
    Style, StyleColor, Terminal, TerminalConfig, TerminalEffect, TrackedGridRef,
};

pub(crate) const STYLE_FLAG_BOLD: u16 = 1 << 0;
pub(crate) const STYLE_FLAG_ITALIC: u16 = 1 << 1;
pub(crate) const STYLE_FLAG_FAINT: u16 = 1 << 2;
pub(crate) const STYLE_FLAG_BLINK: u16 = 1 << 3;
pub(crate) const STYLE_FLAG_INVERSE: u16 = 1 << 4;
pub(crate) const STYLE_FLAG_INVISIBLE: u16 = 1 << 5;
pub(crate) const STYLE_FLAG_STRIKETHROUGH: u16 = 1 << 6;
pub(crate) const STYLE_FLAG_OVERLINE: u16 = 1 << 7;
pub(crate) const STYLE_UNDERLINE_MASK: u16 = 0x7 << (sys::GHOSTTY_STYLE_UNDERLINE_SHIFT as u16);

/// Matches Ghostty's Kitty graphics limits (`graphics_image.zig`).
pub(crate) const MAX_PNG_DIMENSION: u32 = 10_000;
pub(crate) const MAX_PNG_RGBA_BYTES: usize = 400 * 1024 * 1024;
const MAX_PNG_DECODER_METADATA_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UnderlineStyle {
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

#[inline]
pub(crate) fn style_underline(flags: u16) -> UnderlineStyle {
    let raw = ((flags & STYLE_UNDERLINE_MASK) >> (sys::GHOSTTY_STYLE_UNDERLINE_SHIFT as u16)) as u8;
    match sys::GhosttySgrUnderline::from(raw) {
        sys::GHOSTTY_SGR_UNDERLINE_SINGLE => UnderlineStyle::Single,
        sys::GHOSTTY_SGR_UNDERLINE_DOUBLE => UnderlineStyle::Double,
        sys::GHOSTTY_SGR_UNDERLINE_CURLY => UnderlineStyle::Curly,
        sys::GHOSTTY_SGR_UNDERLINE_DOTTED => UnderlineStyle::Dotted,
        sys::GHOSTTY_SGR_UNDERLINE_DASHED => UnderlineStyle::Dashed,
        _ => UnderlineStyle::None,
    }
}

/// Pack a DEC private mode (`?`-prefixed) into `GhosttyMode`.
#[inline]
pub(crate) const fn dec_mode(value: u16) -> sys::GhosttyMode {
    value & 0x7FFF
}

/// Pack an ANSI mode into `GhosttyMode`.
#[inline]
pub(crate) const fn ansi_mode(value: u16) -> sys::GhosttyMode {
    (value & 0x7FFF) | (1 << 15)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VtError {
    OutOfMemory,
    InvalidValue,
    OutOfSpace,
    NoValue,
    IoError,
    LimitExceeded,
    Rejected,
    Unsupported,
    Unknown(i32),
}

impl fmt::Display for VtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfMemory => f.write_str("libghostty-vt: out of memory"),
            Self::InvalidValue => f.write_str("libghostty-vt: invalid value"),
            Self::OutOfSpace => f.write_str("libghostty-vt: buffer too small"),
            Self::NoValue => f.write_str("libghostty-vt: no value"),
            Self::IoError => f.write_str("libghostty-vt: I/O error"),
            Self::LimitExceeded => f.write_str("libghostty-vt: limit exceeded"),
            Self::Rejected => f.write_str("libghostty-vt: rejected"),
            Self::Unsupported => f.write_str("libghostty-vt: unsupported"),
            Self::Unknown(code) => write!(f, "libghostty-vt: error {code}"),
        }
    }
}

impl std::error::Error for VtError {}

pub(crate) type Result<T> = std::result::Result<T, VtError>;

#[inline]
pub(crate) fn check(rc: sys::GhosttyResult) -> Result<()> {
    match rc {
        sys::GHOSTTY_SUCCESS => Ok(()),
        sys::GHOSTTY_OUT_OF_MEMORY => Err(VtError::OutOfMemory),
        sys::GHOSTTY_INVALID_VALUE => Err(VtError::InvalidValue),
        sys::GHOSTTY_OUT_OF_SPACE => Err(VtError::OutOfSpace),
        sys::GHOSTTY_NO_VALUE => Err(VtError::NoValue),
        sys::GHOSTTY_IO_ERROR => Err(VtError::IoError),
        sys::GHOSTTY_LIMIT_EXCEEDED => Err(VtError::LimitExceeded),
        sys::GHOSTTY_REJECTED => Err(VtError::Rejected),
        sys::GHOSTTY_UNSUPPORTED => Err(VtError::Unsupported),
        other => Err(VtError::Unknown(other)),
    }
}

static SYS_INIT: Once = Once::new();

/// Install process-global `libghostty-vt` system callbacks (PNG decode and
/// `tracing` logging) once before creating any terminal handle.
pub(crate) fn init_sys() {
    SYS_INIT.call_once(|| unsafe {
        let png_cb: unsafe extern "C" fn(
            *mut c_void,
            *const sys::GhosttyAllocator,
            *const u8,
            usize,
            *mut sys::GhosttySysImage,
        ) -> bool = sys_decode_png;
        let rc = sys::ghostty_sys_set(sys::GHOSTTY_SYS_OPT_DECODE_PNG, png_cb as *const c_void);
        debug_assert_eq!(
            rc,
            sys::GHOSTTY_SUCCESS,
            "install libghostty-vt PNG decoder"
        );

        let log_cb: unsafe extern "C" fn(
            *mut c_void,
            sys::GhosttySysLogLevel,
            *const u8,
            usize,
            *const u8,
            usize,
        ) = sys_log;
        let rc = sys::ghostty_sys_set(sys::GHOSTTY_SYS_OPT_LOG, log_cb as *const c_void);
        debug_assert_eq!(rc, sys::GHOSTTY_SUCCESS, "install libghostty-vt logger");
    });
}

/// Returns the terminal display width (`0`, `1`, or `2`) of a Unicode scalar
/// value using `libghostty-vt`'s own width tables.
#[inline]
pub(crate) fn codepoint_width(ch: char) -> u8 {
    unsafe { sys::ghostty_unicode_codepoint_width(ch as u32) }
}

struct GhosttyAllocGuard {
    allocator: *const sys::GhosttyAllocator,
    ptr: *mut u8,
    len: usize,
}

impl GhosttyAllocGuard {
    /// Allocate `len` bytes through Ghostty's `allocator` and zero-initialize
    /// them before any `&mut [u8]` slice is created (`sys.h` returns
    /// uninitialized memory).
    ///
    /// # Safety
    /// `allocator` must be null (default allocator) or point to a valid
    /// `GhosttyAllocator` satisfying `allocator.h`.
    unsafe fn alloc_zeroed(allocator: *const sys::GhosttyAllocator, len: usize) -> Option<Self> {
        let ptr = unsafe { sys::ghostty_alloc(allocator, len) };
        if ptr.is_null() {
            return None;
        }
        unsafe { std::ptr::write_bytes(ptr, 0, len) };
        Some(Self {
            allocator,
            ptr,
            len,
        })
    }

    fn as_slice_mut(&mut self) -> &mut [u8] {
        unsafe { slice::from_raw_parts_mut(self.ptr, self.len) }
    }

    fn into_raw(self) -> (*mut u8, usize) {
        let out = (self.ptr, self.len);
        std::mem::forget(self);
        out
    }
}

impl Drop for GhosttyAllocGuard {
    fn drop(&mut self) {
        unsafe { sys::ghostty_free(self.allocator, self.ptr, self.len) };
    }
}

unsafe extern "C" fn sys_decode_png(
    _userdata: *mut c_void,
    allocator: *const sys::GhosttyAllocator,
    data: *const u8,
    data_len: usize,
    out: *mut sys::GhosttySysImage,
) -> bool {
    if data.is_null() || data_len == 0 || out.is_null() {
        return false;
    }
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        let bytes = unsafe { slice::from_raw_parts(data, data_len) };
        let image = unsafe { decode_png_into_ghostty(allocator, bytes) }?;
        unsafe { *out = image };
        Some(())
    }));
    matches!(result, Ok(Some(())))
}

/// Decode a PNG directly into memory allocated through Ghostty's `allocator`,
/// enforcing Ghostty's pixel and byte budget before allocating any frame data.
///
/// # Safety
/// `allocator` must be null (Ghostty's default allocator) or point to a valid
/// `GhosttyAllocator` whose vtable satisfies `allocator.h` (`alloc` and `free`
/// required).
unsafe fn decode_png_into_ghostty(
    allocator: *const sys::GhosttyAllocator,
    bytes: &[u8],
) -> Option<sys::GhosttySysImage> {
    let mut decoder = png::Decoder::new_with_limits(
        Cursor::new(bytes),
        png::Limits {
            bytes: MAX_PNG_DECODER_METADATA_BYTES,
        },
    );
    decoder.set_transformations(
        png::Transformations::EXPAND | png::Transformations::STRIP_16 | png::Transformations::ALPHA,
    );
    let mut reader = decoder.read_info().ok()?;
    let header = reader.info();
    let (width, height) = (header.width, header.height);
    if width == 0 || height == 0 || width > MAX_PNG_DIMENSION || height > MAX_PNG_DIMENSION {
        return None;
    }
    let pixels = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?;
    let rgba_len = pixels.checked_mul(4)?;
    if rgba_len > MAX_PNG_RGBA_BYTES {
        return None;
    }
    let (out_color, out_depth) = reader.output_color_type();
    if out_depth != png::BitDepth::Eight || out_color == png::ColorType::Indexed {
        return None;
    }
    let output_size = reader.output_buffer_size()?;
    if output_size == 0 || output_size > rgba_len {
        return None;
    }

    // Allocate the final RGBA buffer directly from Ghostty's (limited) allocator,
    // zero-initializing before forming a mutable slice, and decode + expand in
    // place so no frame-sized Rust heap buffer is created.
    let mut buf = unsafe { GhosttyAllocGuard::alloc_zeroed(allocator, rgba_len) }?;
    let frame_info = reader
        .next_frame(&mut buf.as_slice_mut()[..output_size])
        .ok()?;
    if frame_info.width != width
        || frame_info.height != height
        || frame_info.bit_depth != png::BitDepth::Eight
    {
        return None;
    }
    let frame_len = frame_info.buffer_size();
    let dst = buf.as_slice_mut();
    match frame_info.color_type {
        png::ColorType::Rgba => {
            if frame_len != rgba_len {
                return None;
            }
        }
        png::ColorType::Rgb => {
            if frame_len != pixels.checked_mul(3)? {
                return None;
            }
            for i in (0..pixels).rev() {
                let (r, g, b) = (dst[i * 3], dst[i * 3 + 1], dst[i * 3 + 2]);
                dst[i * 4..i * 4 + 4].copy_from_slice(&[r, g, b, 0xFF]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            if frame_len != pixels.checked_mul(2)? {
                return None;
            }
            for i in (0..pixels).rev() {
                let (g, a) = (dst[i * 2], dst[i * 2 + 1]);
                dst[i * 4..i * 4 + 4].copy_from_slice(&[g, g, g, a]);
            }
        }
        png::ColorType::Grayscale => {
            if frame_len != pixels {
                return None;
            }
            for i in (0..pixels).rev() {
                let g = dst[i];
                dst[i * 4..i * 4 + 4].copy_from_slice(&[g, g, g, 0xFF]);
            }
        }
        png::ColorType::Indexed => return None,
    }
    let (ptr, data_len) = buf.into_raw();
    Some(sys::GhosttySysImage {
        width,
        height,
        data: ptr,
        data_len,
    })
}

unsafe extern "C" fn sys_log(
    _userdata: *mut c_void,
    level: sys::GhosttySysLogLevel,
    scope: *const u8,
    scope_len: usize,
    message: *const u8,
    message_len: usize,
) {
    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        let scope_bytes = if scope.is_null() || scope_len == 0 {
            &[][..]
        } else {
            unsafe { slice::from_raw_parts(scope, scope_len) }
        };
        let msg_bytes = if message.is_null() || message_len == 0 {
            &[][..]
        } else {
            unsafe { slice::from_raw_parts(message, message_len) }
        };
        let scope = String::from_utf8_lossy(scope_bytes);
        let msg = String::from_utf8_lossy(msg_bytes);
        match level {
            sys::GHOSTTY_SYS_LOG_LEVEL_ERROR => {
                tracing::error!(target: "libghostty_vt", scope = %scope, "{msg}");
            }
            sys::GHOSTTY_SYS_LOG_LEVEL_WARNING => {
                tracing::warn!(target: "libghostty_vt", scope = %scope, "{msg}");
            }
            sys::GHOSTTY_SYS_LOG_LEVEL_INFO => {
                tracing::info!(target: "libghostty_vt", scope = %scope, "{msg}");
            }
            _ => {
                tracing::debug!(target: "libghostty_vt", scope = %scope, "{msg}");
            }
        }
    }));
}

/// Copy and free a buffer allocated by Ghostty's default allocator.
///
/// # Safety
/// On `GHOSTTY_SUCCESS`, `call` must either leave `*ptr` null / `*len == 0` or
/// write a valid pointer and byte length allocated by Ghostty's default
/// allocator (suitable for `slice::from_raw_parts` and `ghostty_free(NULL, ...)`).
pub(super) unsafe fn take_ghostty_alloc(
    call: impl FnOnce(*mut *mut u8, *mut usize) -> sys::GhosttyResult,
) -> Result<Vec<u8>> {
    let mut ptr: *mut u8 = std::ptr::null_mut();
    let mut len: usize = 0;
    check(call(&mut ptr, &mut len))?;
    if ptr.is_null() || len == 0 {
        unsafe { sys::ghostty_free(std::ptr::null(), ptr, len) };
        return Ok(Vec::new());
    }
    let out = unsafe { slice::from_raw_parts(ptr, len) }.to_vec();
    unsafe { sys::ghostty_free(std::ptr::null(), ptr, len) };
    Ok(out)
}
