//! Terminal rendering: the grid element and the procedural glyph painter.

pub(crate) mod control;
pub(crate) mod element;
pub(crate) mod glyphs;
pub(crate) mod input;
mod viewport;

pub(crate) use element::{TerminalElement, TerminalInteraction};
