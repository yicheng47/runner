//! Kitty graphics protocol support for `runner-terminal`: scanning, command
//! parsing, image validation/decoding, local media reading, per-screen storage
//! with daemon-wide atomic budget, command execution, and snapshot replay.

pub mod command;
pub mod exec;
pub mod image;
pub mod media;
pub mod scanner;
pub mod snapshot;
pub mod storage;

pub use command::{Command, Control, Format, Medium, Quiet, Response};
pub use exec::{execute, ExecMode, ExecResult};
pub use image::{
    DecodedFormat, ImageError, StoredImage, MAX_DIMENSION, MAX_TRANSMITTED_BYTES, PER_SCREEN_LIMIT,
};
pub use media::{prepare_command, ForwardedItem, PreparedCommand, TempDirs};
pub use scanner::{strip_graphics_commands, ScanItem, ScanOutput, Scanner, MAX_COMMAND_BYTES};
pub use storage::{
    CellSize, GlobalImageBudget, ImageStorage, PinLocation, Placement, PlacementId, PlacementKey,
    PlacementLocation, RelativeLocation, SourceRect, TerminalGraphics, DEFAULT_GLOBAL_BUDGET,
};

/// Maximum bytes packed into one normalized output frame before closing it and
/// starting a new frame. A single graphics command larger than this travels
/// alone in its own frame (at most ~44 MiB for a 32 MiB transmission).
pub const MAX_NORMALIZED_FRAME_BYTES: usize = 1024 * 1024;

/// Pack normalized text and graphics command segments into frames capped at
/// `MAX_NORMALIZED_FRAME_BYTES`, closing a frame before any segment that would
/// push it over the limit so large transmissions travel alone.
pub fn split_normalized_frames(segments: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    let mut frames: Vec<Vec<u8>> = Vec::new();
    let mut current: Vec<u8> = Vec::new();
    for seg in segments {
        if seg.is_empty() {
            continue;
        }
        if !current.is_empty() && current.len() + seg.len() > MAX_NORMALIZED_FRAME_BYTES {
            frames.push(std::mem::take(&mut current));
        }
        if seg.len() > MAX_NORMALIZED_FRAME_BYTES {
            frames.push(seg);
        } else {
            current.extend_from_slice(&seg);
        }
    }
    if !current.is_empty() {
        frames.push(current);
    }
    frames
}
