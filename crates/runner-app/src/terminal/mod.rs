//! Terminal rendering: the grid element and the procedural glyph painter.

pub(crate) mod clipboard;
pub(crate) mod control;
pub(crate) mod element;
pub(crate) mod find;
pub(crate) mod fonts;
pub(crate) mod glyphs;
pub mod ime;
pub(crate) mod input;
pub mod paste;
#[cfg(test)]
mod redraw_tests;
pub mod resize;
mod viewport;

pub(crate) use element::{TerminalElement, TerminalInteraction};
pub(crate) use find::TerminalFind;
