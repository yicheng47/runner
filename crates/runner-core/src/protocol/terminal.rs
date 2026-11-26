use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::ClientError;

pub const TERMINAL_QUEUE_CAPACITY: usize = 64;
pub const TERMINAL_QUEUE_MAX_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_CELL_PIXEL_SIZE: u16 = 1024;

/// Normalizes a client-reported cell size in device pixels.
/// Zero on either axis means unknown (keep the session's current cell size);
/// non-zero values clamp to `1..=1024` pixels per side.
pub fn normalize_cell_size(width: u16, height: u16) -> Option<(u16, u16)> {
    if width == 0 || height == 0 {
        None
    } else {
        Some((
            width.clamp(1, MAX_CELL_PIXEL_SIZE),
            height.clamp(1, MAX_CELL_PIXEL_SIZE),
        ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputKind {
    Content { text: String },
    Edit,
    Submit,
    Cancel,
    Navigate,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputEvent {
    Key { kind: InputKind },
    Paste { text: String },
    Composing { composing: bool },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TerminalController {
    #[default]
    Desktop,
    Browser {
        channel: u64,
        name: String,
    },
    None,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TerminalMetadata {
    pub title: String,
    pub live_cwd: Option<PathBuf>,
    pub link_cwd: Option<PathBuf>,
    pub cols: u16,
    pub rows: u16,
    pub controller: TerminalController,
}

#[derive(Clone, Serialize, Deserialize)]
pub enum TerminalFrame {
    Output {
        seq: u64,
        bytes: Vec<u8>,
    },
    Resized {
        seq: u64,
        cols: u16,
        rows: u16,
        #[serde(default)]
        cell_width: u16,
        #[serde(default)]
        cell_height: u16,
        controller: TerminalController,
    },
    ClearGraphics {
        seq: u64,
    },
    Resync,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalSnapshotActivity {
    pub output_seq: u64,
    pub tui_ready_seq: u64,
    pub first_paint_seq: u64,
    pub paint_seq: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TerminalSnapshot {
    pub seq: u64,
    #[serde(default)]
    pub activity: Option<TerminalSnapshotActivity>,
    pub cols: u16,
    pub rows: u16,
    #[serde(default)]
    pub cell_width: u16,
    #[serde(default)]
    pub cell_height: u16,
    pub controller: TerminalController,
    pub bytes: Vec<u8>,
    pub unfinished_len: usize,
    pub preceding_char: Option<char>,
}

pub trait TerminalSubscription: Send {
    fn recv(&mut self) -> Result<TerminalFrame, ClientError>;
    fn cancellation(&self) -> std::sync::Arc<dyn Fn() + Send + Sync>;
}

pub struct TerminalAttachment {
    pub subscriber_id: u64,
    pub snapshot: TerminalSnapshot,
    pub frames: Box<dyn TerminalSubscription>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TerminalPalette {
    pub background: [u8; 3],
    pub foreground: [u8; 3],
    pub cursor: [u8; 3],
    pub cursor_accent: [u8; 3],
    pub selection: [u8; 3],
    pub ansi: [[u8; 3]; 16],
}

pub trait TerminalLifecycle: Send + Sync {
    fn event(&self, event: super::ClientEvent);
}

impl std::fmt::Debug for TerminalFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TerminalFrame([redacted])")
    }
}
impl std::fmt::Debug for TerminalSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TerminalSnapshot([redacted])")
    }
}
