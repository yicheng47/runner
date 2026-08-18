//! UI-agnostic terminal model for Runner's native app — the `terminal`
//! half of the Zed-style `terminal` / `terminal_view` split (impl 0046
//! Workstream C). Owns the `alacritty_terminal` state, parsing, input
//! encoding, and the fixture corpus; rendering lives in `runner-app`.
//!
//! Deliberate deviation from Zed: this crate does not own the PTY.
//! `runner_backend`'s `SessionManager` spawns and manages sessions; this
//! crate consumes its output events and produces renderable grid state
//! plus encoded input bytes.

pub mod fixtures;
pub mod mappings;
pub mod palette;
pub mod replay;
pub mod terminal;
