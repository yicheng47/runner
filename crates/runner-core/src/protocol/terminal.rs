use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::ClientError;

/// Reference-counted output payload shared across `TerminalFrame::Output` clones
/// so `push_frame` fans large graphics frames out to multiple local subscribers
/// without copying their bytes.
#[derive(Clone)]
pub struct SharedBytes(SharedBytesRepr);

impl Eq for SharedBytes {}

#[derive(Clone)]
enum SharedBytesRepr {
    Slice(Arc<[u8]>),
    Owned(Arc<Vec<u8>>),
}

impl SharedBytes {
    pub fn as_slice(&self) -> &[u8] {
        match &self.0 {
            SharedBytesRepr::Slice(bytes) => bytes,
            SharedBytesRepr::Owned(bytes) => bytes,
        }
    }

    pub fn into_vec(self) -> Vec<u8> {
        match self.0 {
            SharedBytesRepr::Slice(bytes) => bytes.to_vec(),
            SharedBytesRepr::Owned(bytes) => {
                Arc::try_unwrap(bytes).unwrap_or_else(|shared| (*shared).clone())
            }
        }
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (SharedBytesRepr::Slice(a), SharedBytesRepr::Slice(b)) => Arc::ptr_eq(a, b),
            (SharedBytesRepr::Owned(a), SharedBytesRepr::Owned(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Default for SharedBytes {
    fn default() -> Self {
        Self(SharedBytesRepr::Slice(Arc::from([] as [u8; 0])))
    }
}

impl Deref for SharedBytes {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl AsRef<[u8]> for SharedBytes {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl From<Vec<u8>> for SharedBytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self(SharedBytesRepr::Owned(Arc::new(bytes)))
    }
}

impl From<&[u8]> for SharedBytes {
    fn from(bytes: &[u8]) -> Self {
        Self(SharedBytesRepr::Slice(Arc::from(bytes)))
    }
}

impl<const N: usize> From<&[u8; N]> for SharedBytes {
    fn from(bytes: &[u8; N]) -> Self {
        Self::from(bytes.as_slice())
    }
}

impl From<Arc<[u8]>> for SharedBytes {
    fn from(bytes: Arc<[u8]>) -> Self {
        Self(SharedBytesRepr::Slice(bytes))
    }
}

impl std::fmt::Debug for SharedBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_slice().fmt(f)
    }
}

impl PartialEq for SharedBytes {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl PartialEq<[u8]> for SharedBytes {
    fn eq(&self, other: &[u8]) -> bool {
        self.as_slice() == other
    }
}

impl PartialEq<&[u8]> for SharedBytes {
    fn eq(&self, other: &&[u8]) -> bool {
        self.as_slice() == *other
    }
}

impl PartialEq<Vec<u8>> for SharedBytes {
    fn eq(&self, other: &Vec<u8>) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<const N: usize> PartialEq<[u8; N]> for SharedBytes {
    fn eq(&self, other: &[u8; N]) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<const N: usize> PartialEq<&[u8; N]> for SharedBytes {
    fn eq(&self, other: &&[u8; N]) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Serialize for SharedBytes {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_slice().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SharedBytes {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Vec::<u8>::deserialize(deserializer).map(Self::from)
    }
}

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
        bytes: SharedBytes,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_bytes_shares_allocation_and_matches_vec_serde() {
        let raw = vec![1u8, 2, 3, 4, 5, 6];
        let orig_ptr = raw.as_ptr();
        let shared = SharedBytes::from(raw.clone());
        assert_eq!(shared, raw);
        assert_eq!(shared, b"\x01\x02\x03\x04\x05\x06");

        // Wrapping a Vec<u8> must not copy its heap buffer, and cloning
        // TerminalFrame::Output must share the same underlying allocation.
        let owned = SharedBytes::from(raw);
        assert_eq!(owned.as_ptr(), orig_ptr);
        let frame = TerminalFrame::Output {
            seq: 7,
            bytes: owned,
        };
        let cloned = frame.clone();
        match (frame, cloned) {
            (
                TerminalFrame::Output { seq: s1, bytes: b1 },
                TerminalFrame::Output { seq: s2, bytes: b2 },
            ) => {
                assert_eq!((s1, s2), (7, 7));
                assert!(b1.ptr_eq(&b2));
                assert_eq!(b1.as_ptr(), orig_ptr);
                drop(b2);
                let recovered = b1.into_vec();
                assert_eq!(recovered.as_ptr(), orig_ptr);
            }
            _ => unreachable!(),
        }

        // Slice-backed SharedBytes also shares its allocation across clones.
        let from_slice = SharedBytes::from(&b"hello"[..]);
        let from_slice_clone = from_slice.clone();
        assert!(from_slice.ptr_eq(&from_slice_clone));

        // JSON serialization matches Vec<u8> byte-for-byte.
        let vec_json = serde_json::to_string(&vec![10u8, 20, 30]).unwrap();
        let shared_json = serde_json::to_string(&SharedBytes::from(vec![10u8, 20, 30])).unwrap();
        assert_eq!(shared_json, vec_json);
        let decoded: SharedBytes = serde_json::from_str(&shared_json).unwrap();
        assert_eq!(decoded, b"\x0a\x14\x1e");
    }
}
