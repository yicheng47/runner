//! Shared terminal parsing, session-owned authoritative models, and app mirrors.
//! The backend owns PTYs and implements `TerminalHost`; this crate owns VT state,
//! input encoding, snapshots, and the fixture corpus. Rendering lives in the app.

pub mod delivery_probe;
pub mod fixtures;
pub mod input_state;
pub mod mappings;
pub mod palette;
pub mod program_status;
pub mod replay;
pub mod screen;
pub mod snapshot;
pub mod terminal;
pub mod title_parser;
pub mod types;
#[allow(dead_code, unused_imports)]
pub(crate) mod vt;

pub use palette::{Color, NamedColor, Rgb, TerminalPalette};
pub use screen::{CellView, ScreenGuard, ScreenView, ViewportCell, VisibleCells};
pub use types::{
    point_to_viewport, viewport_to_point, CellFlags, Column, CursorShape, CursorStyle, FindMatch,
    GridPoint, Hyperlink, Line, RenderCursor, SelectionKind, SelectionRange, Side,
    TerminalModeFlags, ViewportPoint,
};
