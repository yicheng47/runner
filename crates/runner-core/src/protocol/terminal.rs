use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::ClientError;

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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TerminalMetadata {
    pub title: String,
    pub live_cwd: Option<PathBuf>,
    pub link_cwd: Option<PathBuf>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TerminalFrame {
    Output { seq: u64, bytes: Vec<u8> },
    Resized { seq: u64, cols: u16, rows: u16 },
    Resync,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TerminalSnapshot {
    pub seq: u64,
    pub cols: u16,
    pub rows: u16,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeUpdateCommand {
    pub session_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: std::collections::BTreeMap<String, String>,
    pub shell_path: Option<String>,
    pub size: (u16, u16),
}

pub trait TerminalLifecycle: Send + Sync {
    fn event(&self, event: super::ClientEvent);
}
