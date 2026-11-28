//! Terminal snapshotting and state inspection over `libghostty-vt`.
//!
//! Local desktop snapshots use Ghostty's lossless binary snapshot codec
//! (`encode_binary` / `decode_binary`). Companion / browser snapshots use
//! Ghostty's `Formatter` in VT mode (`serialize` / `serialize_with_history`) so
//! the browser `companion::VERSION = 1` wire format stays unchanged.

use std::ops::Range;
use std::sync::Arc;

use crate::palette::{Color as RunnerColor, NamedColor, Rgb};
use crate::screen::{
    cell_bg_to_runner, style_color_to_runner, style_underline_color_to_runner, ScreenData,
};
use crate::types::{
    CellFlags, Column, CursorShape, CursorStyle as RunnerCursorStyle, GridPoint,
    Hyperlink as RunnerHyperlink, Line as RunnerLine, TerminalModeFlags,
};
use crate::vt::{
    self, CellWide, GridPointTag, SnapshotDecodeOptions, StyleColor, TerminalConfig,
    DEFAULT_CONTINUATION_MAX_BYTES,
};

/// Runner-owned character set slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotCharsetIndex {
    G0,
    G1,
    G2,
    G3,
}

/// Runner-owned character set designation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotStandardCharset {
    Ascii,
    SpecialCharacterAndLineDrawing,
}

/// Owned snapshot of one cell for round-trip verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellSnapshot {
    pub c: char,
    pub fg: RunnerColor,
    pub bg: RunnerColor,
    pub flags: CellFlags,
    pub zerowidth: Vec<char>,
    pub underline_color: Option<RunnerColor>,
    pub hyperlink: Option<RunnerHyperlink>,
}

impl Default for CellSnapshot {
    fn default() -> Self {
        Self {
            c: ' ',
            fg: RunnerColor::Named(NamedColor::Foreground),
            bg: RunnerColor::Named(NamedColor::Background),
            flags: CellFlags::empty(),
            zerowidth: Vec::new(),
            underline_color: None,
            hyperlink: None,
        }
    }
}

/// Owned snapshot of cursor state for round-trip verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorSnapshot {
    pub point: GridPoint,
    pub charsets: [SnapshotStandardCharset; 4],
    pub input_needs_wrap: bool,
    pub template: CellSnapshot,
}

impl Default for CursorSnapshot {
    fn default() -> Self {
        Self {
            point: GridPoint::default(),
            charsets: [SnapshotStandardCharset::Ascii; 4],
            input_needs_wrap: false,
            template: CellSnapshot::default(),
        }
    }
}

/// Owned snapshot of a grid (history + screen) for round-trip verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridSnapshot {
    pub history_size: usize,
    pub screen_lines: usize,
    pub columns: usize,
    pub cursor: CursorSnapshot,
    pub saved_cursor: CursorSnapshot,
    pub rows: Vec<Vec<CellSnapshot>>,
}

/// Owned Runner-typed view of complete terminal state for snapshot round-trip verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSnapshotState {
    pub mode: TerminalModeFlags,
    pub active_grid: GridSnapshot,
    pub inactive_grid: Option<GridSnapshot>,
    pub inactive_saved_cursor: CursorSnapshot,
    pub scroll_region: Range<RunnerLine>,
    pub title: Option<String>,
    pub title_stack: Vec<Option<String>>,
    pub tabs: Vec<bool>,
    pub keyboard_mode_stack: Vec<u8>,
    pub inactive_keyboard_mode_stack: Vec<u8>,
    pub cursor_style: Option<RunnerCursorStyle>,
    pub active_charset: SnapshotCharsetIndex,
}

/// Encode a VT-format snapshot for companion consumers (all scrollback).
pub fn serialize(term: &crate::replay::Terminal, unfinished: &[u8]) -> Vec<u8> {
    serialize_vt_with_history(term.vt(), unfinished, usize::MAX)
}

/// Encode a VT-format snapshot for companion consumers with at most `history`
/// scrollback lines.
pub fn serialize_with_history(
    term: &crate::replay::Terminal,
    unfinished: &[u8],
    history: usize,
) -> Vec<u8> {
    serialize_vt_with_history(term.vt(), unfinished, history)
}

pub(crate) fn try_serialize_vt_with_history(
    term: &vt::Terminal,
    unfinished: &[u8],
    history: usize,
) -> vt::Result<Vec<u8>> {
    let mut bytes = term.format_vt_with_history(history)?;
    bytes.extend_from_slice(unfinished);
    Ok(bytes)
}

pub(crate) fn serialize_vt_with_history(
    term: &vt::Terminal,
    unfinished: &[u8],
    history: usize,
) -> Vec<u8> {
    try_serialize_vt_with_history(term, unfinished, history).unwrap_or_default()
}

pub(crate) fn encode_binary(term: &vt::Terminal) -> vt::Result<Vec<u8>> {
    term.encode_snapshot()
}

/// Inspect complete terminal state from a live `vt::Terminal` by reading its
/// active grid plus the structured v1 binary snapshot headers (for both
/// screens' cursors, saved cursors, charsets, tab stops, scroll region, and
/// Kitty keyboard stacks).
pub(crate) fn inspect_vt(term: &vt::Terminal) -> TerminalSnapshotState {
    let mode = TerminalModeFlags::from_vt(term);
    let cols = usize::from(term.cols());
    let rows = usize::from(term.rows());
    let blob = term.encode_snapshot().unwrap_or_default();
    let parsed = parse_snapshot_blob(&blob);

    let active_screen_key: u16 = if mode.contains(TerminalModeFlags::ALT_SCREEN) {
        1
    } else {
        0
    };
    let inactive_screen_key: u16 = if active_screen_key == 1 { 0 } else { 1 };

    let active_meta = parsed
        .as_ref()
        .and_then(|p| p.screen(active_screen_key))
        .cloned()
        .unwrap_or_default();
    let inactive_meta = parsed
        .as_ref()
        .and_then(|p| p.screen(inactive_screen_key))
        .cloned()
        .unwrap_or_default();

    let active_rows =
        grid_rows_from_screen_data(&ScreenData::from_terminal_full(term, None, false));
    let active_grid = GridSnapshot {
        history_size: term.scrollback_rows(),
        screen_lines: rows,
        columns: cols,
        cursor: active_meta.cursor.clone(),
        saved_cursor: active_meta.saved_cursor.clone(),
        rows: active_rows,
    };

    let inactive_grid = if mode.contains(TerminalModeFlags::ALT_SCREEN) && !blob.is_empty() {
        let decode_cfg = TerminalConfig {
            cols: term.cols(),
            rows: term.rows(),
            scrollback_lines: usize::MAX,
            continuation_max_bytes: DEFAULT_CONTINUATION_MAX_BYTES,
            kitty_image_storage_limit: 0,
            enable_host_callbacks: false,
            enable_render_hold: false,
            ..TerminalConfig::default()
        };
        vt::Terminal::decode_snapshot(
            &blob,
            SnapshotDecodeOptions {
                max_continuation_bytes: DEFAULT_CONTINUATION_MAX_BYTES,
                retain_continuation: false,
                compress_history: false,
                kitty_image_storage_limit: 0,
            },
            &decode_cfg,
        )
        .ok()
        .map(|mut tmp| {
            // Mode 1047 switches back to the primary screen without restoring the
            // saved cursor, leaving primary grid cells untouched.
            tmp.vt_write(b"\x1b[?1047l");
            let data = ScreenData::from_terminal_full(&tmp, None, false);
            GridSnapshot {
                history_size: tmp.scrollback_rows(),
                screen_lines: usize::from(tmp.rows()),
                columns: usize::from(tmp.cols()),
                cursor: inactive_meta.cursor.clone(),
                saved_cursor: inactive_meta.saved_cursor.clone(),
                rows: grid_rows_from_screen_data(&data),
            }
        })
    } else {
        None
    };

    let (scroll_region, tabs, cursor_style) = match parsed.as_ref() {
        Some(p) => {
            let sr =
                RunnerLine(i32::from(p.scroll_top))..RunnerLine(i32::from(p.scroll_bottom) + 1);
            let style = if p.cursor_is_default {
                None
            } else {
                Some(RunnerCursorStyle {
                    shape: active_meta.visual_style,
                    blinking: term.mode(vt::dec_mode(12)),
                })
            };
            (sr, p.tabs.clone(), style)
        }
        None => (
            RunnerLine(0)..RunnerLine(rows as i32),
            vec![false; cols],
            None,
        ),
    };

    let title = term.title();
    TerminalSnapshotState {
        mode,
        active_grid,
        inactive_grid,
        inactive_saved_cursor: inactive_meta.saved_cursor,
        scroll_region,
        title: (!title.is_empty()).then_some(title),
        title_stack: term.title_stack(),
        tabs,
        keyboard_mode_stack: active_meta.kitty_stack,
        inactive_keyboard_mode_stack: inactive_meta.kitty_stack,
        cursor_style,
        active_charset: active_meta.active_charset,
    }
}

fn grid_rows_from_screen_data(data: &ScreenData) -> Vec<Vec<CellSnapshot>> {
    let view = data.view();
    let cols = view.cols();
    let history = view.history_size();
    let total = history + view.rows();
    let mut rows = Vec::with_capacity(total);
    for line_i in -(history as i32)..view.rows() as i32 {
        let mut row = Vec::with_capacity(cols);
        for col in 0..cols {
            let cell = view
                .cell(GridPoint::new(RunnerLine(line_i), Column(col)))
                .map(|c| CellSnapshot {
                    c: c.c(),
                    fg: c.fg(),
                    bg: c.bg(),
                    flags: c.flags(),
                    zerowidth: c.zerowidth().map(<[char]>::to_vec).unwrap_or_default(),
                    underline_color: c.underline_color(),
                    hyperlink: c.hyperlink(),
                })
                .unwrap_or_default();
            row.push(cell);
        }
        rows.push(row);
    }
    rows
}

/// Patch `previous_codepoint` (offset `25..29` of record 0 `TERMINAL` payload)
/// and update record 0's CRC32C checksum in a v1 Ghostty binary snapshot.
pub(crate) fn patch_snapshot_previous_char(blob: &mut [u8], ch: Option<char>) -> bool {
    if blob.len() < 10 + 10 + 103 || &blob[..8] != b"GHOSTSNP" {
        return false;
    }
    let version = u16::from_le_bytes([blob[8], blob[9]]);
    if version != 1 {
        return false;
    }
    let tag = u16::from_le_bytes([blob[10], blob[11]]);
    let payload_len = u32::from_le_bytes([blob[12], blob[13], blob[14], blob[15]]) as usize;
    if tag != 1 || payload_len < 103 || blob.len() < 20 + payload_len {
        return false;
    }
    let cp = ch.map_or(u32::MAX, |c| c as u32);
    blob[20 + 25..20 + 29].copy_from_slice(&cp.to_le_bytes());
    let crc = crc32c_record(&blob[10..16], &blob[20..20 + payload_len]);
    blob[16..20].copy_from_slice(&crc.to_le_bytes());
    true
}

/// Castagnoli CRC-32C over the 6-byte record header prefix (`tag` + `payload_len`)
/// followed by the payload bytes, matching Ghostty's `snapshot/record.zig`.
fn crc32c_record(header_prefix: &[u8], payload: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in header_prefix.iter().chain(payload.iter()) {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0x82F6_3B78 & mask);
        }
    }
    !crc
}

#[derive(Clone)]
pub(crate) struct ParsedScreenMeta {
    pub(crate) screen_key: u16,
    pub(crate) cursor: CursorSnapshot,
    pub(crate) cursor_protected: bool,
    pub(crate) cursor_style_vt: vt::Style,
    pub(crate) raw_charsets: [u8; 4],
    pub(crate) saved_cursor: CursorSnapshot,
    pub(crate) saved_cursor_present: bool,
    pub(crate) saved_cursor_origin: bool,
    pub(crate) saved_cursor_protected: bool,
    pub(crate) saved_cursor_active_charset: SnapshotCharsetIndex,
    pub(crate) saved_raw_charsets: [u8; 4],
    pub(crate) saved_cursor_style_vt: vt::Style,
    pub(crate) visual_style: CursorShape,
    pub(crate) active_charset: SnapshotCharsetIndex,
    pub(crate) kitty_stack: Vec<u8>,
}

impl Default for ParsedScreenMeta {
    fn default() -> Self {
        Self {
            screen_key: 0,
            cursor: CursorSnapshot::default(),
            cursor_protected: false,
            cursor_style_vt: vt::Style::default(),
            raw_charsets: [0; 4],
            saved_cursor: CursorSnapshot::default(),
            saved_cursor_present: false,
            saved_cursor_origin: false,
            saved_cursor_protected: false,
            saved_cursor_active_charset: SnapshotCharsetIndex::G0,
            saved_raw_charsets: [0; 4],
            saved_cursor_style_vt: vt::Style::default(),
            visual_style: CursorShape::Block,
            active_charset: SnapshotCharsetIndex::G0,
            kitty_stack: Vec::new(),
        }
    }
}

pub(crate) struct ParsedSnapshot {
    pub(crate) scroll_top: u16,
    pub(crate) scroll_bottom: u16,
    pub(crate) cursor_is_default: bool,
    pub(crate) tabs: Vec<bool>,
    pub(crate) screens: Vec<ParsedScreenMeta>,
}

impl ParsedSnapshot {
    pub(crate) fn screen(&self, key: u16) -> Option<&ParsedScreenMeta> {
        self.screens.iter().find(|s| s.screen_key == key)
    }
}

pub(crate) fn parse_snapshot_blob(blob: &[u8]) -> Option<ParsedSnapshot> {
    if blob.len() < 10 || &blob[..8] != b"GHOSTSNP" {
        return None;
    }
    let version = u16::from_le_bytes([blob[8], blob[9]]);
    if version != 1 {
        return None;
    }
    let mut pos = 10usize;
    let mut scroll_top = 0u16;
    let mut scroll_bottom = 0u16;
    let mut cursor_is_default = true;
    let mut tabs = Vec::new();
    let mut screens = Vec::new();

    while pos + 10 <= blob.len() {
        let tag = u16::from_le_bytes([blob[pos], blob[pos + 1]]);
        let payload_len =
            u32::from_le_bytes([blob[pos + 2], blob[pos + 3], blob[pos + 4], blob[pos + 5]])
                as usize;
        pos += 10;
        if pos + payload_len > blob.len() {
            break;
        }
        let payload = &blob[pos..pos + payload_len];
        pos += payload_len;

        match tag {
            1 if payload.len() >= 103 => {
                let cols = usize::from(u16::from_le_bytes([payload[0], payload[1]]));
                scroll_top = u16::from_le_bytes([payload[12], payload[13]]);
                scroll_bottom = u16::from_le_bytes([payload[14], payload[15]]);
                cursor_is_default = payload[29] != 0;
                let tab_bytes = cols.div_ceil(8);
                if payload.len() >= 103 + tab_bytes {
                    tabs = (0..cols)
                        .map(|col| (payload[103 + col / 8] & (1 << (col % 8))) != 0)
                        .collect();
                }
            }
            2 if payload.len() >= 53 => {
                if let Some(meta) = parse_screen_record(payload) {
                    screens.push(meta);
                }
            }
            6 => break,
            _ => {}
        }
    }

    Some(ParsedSnapshot {
        scroll_top,
        scroll_bottom,
        cursor_is_default,
        tabs,
        screens,
    })
}

fn parse_screen_record(payload: &[u8]) -> Option<ParsedScreenMeta> {
    if payload.len() < 53 {
        return None;
    }
    let screen_key = u16::from_le_bytes([payload[0], payload[1]]);
    let cursor_x = u16::from_le_bytes([payload[12], payload[13]]);
    let cursor_y = u16::from_le_bytes([payload[14], payload[15]]);
    let visual_style = match payload[16] {
        0 => CursorShape::Beam,
        2 => CursorShape::Underline,
        3 => CursorShape::HollowBlock,
        _ => CursorShape::Block,
    };
    let cursor_flags = payload[17];
    let pending_wrap = (cursor_flags & 1) != 0;
    let cursor_protected = (cursor_flags & (1 << 1)) != 0;
    let (pen, cursor_style_vt) = parse_style_entry(&payload[18..34]);
    let charset_raw = u16::from_le_bytes([payload[38], payload[39]]);
    let (charsets, raw_charsets, active_charset) = decode_charset_state(charset_raw);
    let kitty_index = usize::from(payload[41]).min(7);
    let kitty_raw = &payload[42..50];
    let kitty_stack = parse_kitty_ring(kitty_index, kitty_raw);
    let saved_present = payload[52] != 0;
    let (
        saved_cursor,
        saved_cursor_present,
        saved_cursor_origin,
        saved_cursor_protected,
        saved_cursor_active_charset,
        saved_raw_charsets,
        saved_cursor_style_vt,
        link_offset,
    ) = if saved_present && payload.len() >= 76 {
        let sc = &payload[53..76];
        let sx = u16::from_le_bytes([sc[0], sc[1]]);
        let sy = u16::from_le_bytes([sc[2], sc[3]]);
        let (spen, sstyle) = parse_style_entry(&sc[4..20]);
        let sflags = sc[20];
        let sprotected = (sflags & (1 << 0)) != 0;
        let spending_wrap = (sflags & (1 << 1)) != 0;
        let sorigin = (sflags & (1 << 2)) != 0;
        let scharset_raw = u16::from_le_bytes([sc[21], sc[22]]);
        let (scharsets, sraw_charsets, sgl) = decode_charset_state(scharset_raw);
        (
            CursorSnapshot {
                point: GridPoint::new(RunnerLine(i32::from(sy)), Column(usize::from(sx))),
                charsets: scharsets,
                input_needs_wrap: spending_wrap,
                template: spen,
            },
            true,
            sorigin,
            sprotected,
            sgl,
            sraw_charsets,
            sstyle,
            76usize,
        )
    } else {
        (
            CursorSnapshot::default(),
            false,
            false,
            false,
            SnapshotCharsetIndex::G0,
            [0; 4],
            vt::Style::default(),
            53usize,
        )
    };
    let cursor_hyperlink = payload.get(link_offset..).and_then(parse_hyperlink_entry);
    let mut template = pen;
    template.hyperlink = cursor_hyperlink;
    let cursor = CursorSnapshot {
        point: GridPoint::new(
            RunnerLine(i32::from(cursor_y)),
            Column(usize::from(cursor_x)),
        ),
        charsets,
        input_needs_wrap: pending_wrap,
        template,
    };
    Some(ParsedScreenMeta {
        screen_key,
        cursor,
        cursor_protected,
        cursor_style_vt,
        raw_charsets,
        saved_cursor,
        saved_cursor_present,
        saved_cursor_origin,
        saved_cursor_protected,
        saved_cursor_active_charset,
        saved_raw_charsets,
        saved_cursor_style_vt,
        visual_style,
        active_charset,
        kitty_stack,
    })
}

/// Extract Ghostty's 8-slot Kitty keyboard `FlagStack` ring in oldest-to-newest
/// order, capped to Phase 1 bit 0 (`DISAMBIGUATE_ESC_CODES`), stripping leading
/// unused zero slots.
fn parse_kitty_ring(kitty_index: usize, kitty_raw: &[u8]) -> Vec<u8> {
    let mut ordered = [0u8; 8];
    for (k, slot) in ordered.iter_mut().enumerate() {
        *slot = kitty_raw[(kitty_index + 1 + k) % 8] & 1;
    }
    match ordered.iter().position(|&v| v != 0) {
        Some(first) => ordered[first..].to_vec(),
        None => Vec::new(),
    }
}

fn parse_style_entry(buf: &[u8]) -> (CellSnapshot, vt::Style) {
    let fg = parse_style_color(&buf[0..4]);
    let bg = parse_style_color(&buf[4..8]);
    let ul = parse_style_color(&buf[8..12]);
    let flags = u16::from_le_bytes([buf[12], buf[13]]);
    let style = vt::Style {
        fg,
        bg,
        underline: ul,
        flags,
    };
    let cell = CellSnapshot {
        c: ' ',
        fg: style_color_to_runner(fg, NamedColor::Foreground),
        bg: cell_bg_to_runner(
            &vt::CellInfo {
                codepoint: 0,
                content_tag: vt::CellContentTag::Codepoint,
                wide: CellWide::Narrow,
                has_text: false,
                has_styling: true,
                has_hyperlink: false,
                protected: false,
                bg_palette: None,
                bg_rgb: None,
            },
            &style,
        ),
        flags: CellFlags::from_vt_style(flags, CellWide::Narrow, false),
        zerowidth: Vec::new(),
        underline_color: style_underline_color_to_runner(ul),
        hyperlink: None,
    };
    (cell, style)
}

fn parse_style_color(buf: &[u8]) -> StyleColor {
    match buf[0] {
        1 => StyleColor::Palette(buf[1]),
        2 => StyleColor::Rgb(Rgb::new(buf[1], buf[2], buf[3])),
        _ => StyleColor::None,
    }
}

fn decode_charset_state(raw: u16) -> ([SnapshotStandardCharset; 4], [u8; 4], SnapshotCharsetIndex) {
    // Ghostty `charsets.Charset`: 0 = utf8, 1 = ascii, 2 = british, 3 = dec_special.
    let raw_slots = [
        (raw & 0x3) as u8,
        ((raw >> 2) & 0x3) as u8,
        ((raw >> 4) & 0x3) as u8,
        ((raw >> 6) & 0x3) as u8,
    ];
    let charsets = raw_slots.map(|code| match code {
        3 => SnapshotStandardCharset::SpecialCharacterAndLineDrawing,
        _ => SnapshotStandardCharset::Ascii,
    });
    let gl = match (raw >> 8) & 0x3 {
        1 => SnapshotCharsetIndex::G1,
        2 => SnapshotCharsetIndex::G2,
        3 => SnapshotCharsetIndex::G3,
        _ => SnapshotCharsetIndex::G0,
    };
    (charsets, raw_slots, gl)
}

fn parse_hyperlink_entry(buf: &[u8]) -> Option<RunnerHyperlink> {
    let kind = *buf.first()?;
    match kind {
        0 => None,
        1 => {
            if buf.len() < 9 {
                return None;
            }
            let uri_len = u32::from_le_bytes([buf[5], buf[6], buf[7], buf[8]]) as usize;
            let uri_bytes = buf.get(9..9 + uri_len)?;
            let uri = String::from_utf8_lossy(uri_bytes);
            Some(RunnerHyperlink::new(
                Arc::<str>::from(""),
                Arc::<str>::from(uri.as_ref()),
            ))
        }
        2 => {
            if buf.len() < 5 {
                return None;
            }
            let id_len = u32::from_le_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize;
            let id_bytes = buf.get(5..5 + id_len)?;
            let uri_len_off = 5 + id_len;
            if buf.len() < uri_len_off + 4 {
                return None;
            }
            let uri_len = u32::from_le_bytes([
                buf[uri_len_off],
                buf[uri_len_off + 1],
                buf[uri_len_off + 2],
                buf[uri_len_off + 3],
            ]) as usize;
            let uri_bytes = buf.get(uri_len_off + 4..uri_len_off + 4 + uri_len)?;
            let id = String::from_utf8_lossy(id_bytes);
            let uri = String::from_utf8_lossy(uri_bytes);
            Some(RunnerHyperlink::new(
                Arc::<str>::from(id.as_ref()),
                Arc::<str>::from(uri.as_ref()),
            ))
        }
        _ => None,
    }
}

#[allow(dead_code)]
fn _grid_tag_used(_: GridPointTag) {}
