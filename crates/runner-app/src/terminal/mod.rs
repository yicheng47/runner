//! Terminal rendering: the grid element and the procedural glyph painter.

pub(crate) mod control;
pub(crate) mod element;
pub(crate) mod glyphs;
mod viewport;

pub(crate) use element::{TerminalElement, TerminalInteraction};
