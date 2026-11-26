//! Shared terminal parsing, session-owned authoritative models, and app mirrors.
//! The backend owns PTYs and implements `TerminalHost`; this crate owns VT state,
//! input encoding, snapshots, and the fixture corpus. Rendering lives in the app.

pub mod delivery_probe;
pub mod fixtures;
pub mod input_state;
pub mod kitty;
pub mod mappings;
pub mod palette;
pub mod program_status;
pub mod replay;
pub mod snapshot;
pub mod terminal;
