//! UI-agnostic terminal palettes and color resolution.

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalPalette {
    pub background: Rgb,
    pub foreground: Rgb,
    pub cursor: Rgb,
    pub cursor_accent: Rgb,
    pub selection: Rgb,
    pub ansi: [Rgb; 16],
}

const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
    Rgb { r, g, b }
}

pub const RUNNER: TerminalPalette = TerminalPalette {
    background: rgb(0x15, 0x16, 0x1b),
    foreground: rgb(0xdc, 0xdc, 0xe0),
    cursor: rgb(0x00, 0xff, 0x9c),
    cursor_accent: rgb(0x15, 0x16, 0x1b),
    selection: rgb(0x27, 0x29, 0x30),
    ansi: [
        rgb(0x15, 0x16, 0x1b),
        rgb(0xff, 0x4d, 0x6d),
        rgb(0x00, 0xff, 0x9c),
        rgb(0xff, 0xb0, 0x20),
        rgb(0x39, 0xe5, 0xff),
        rgb(0xc7, 0x92, 0xea),
        rgb(0x39, 0xe5, 0xff),
        rgb(0xdc, 0xdc, 0xe0),
        rgb(0x5a, 0x5c, 0x66),
        rgb(0xff, 0x7b, 0x8e),
        rgb(0x5f, 0xff, 0xb8),
        rgb(0xff, 0xcb, 0x6b),
        rgb(0x82, 0xaa, 0xff),
        rgb(0xc7, 0x92, 0xea),
        rgb(0x89, 0xdd, 0xff),
        rgb(0xff, 0xff, 0xff),
    ],
};

pub const BACKGROUND: Rgb = RUNNER.background;
pub const FOREGROUND: Rgb = RUNNER.foreground;
pub const CURSOR: Rgb = RUNNER.cursor;

pub const CATPPUCCIN_MOCHA: TerminalPalette = TerminalPalette {
    background: rgb(0x1e, 0x1e, 0x2e),
    foreground: rgb(0xcd, 0xd6, 0xf4),
    cursor: rgb(0xf5, 0xe0, 0xdc),
    cursor_accent: rgb(0x1e, 0x1e, 0x2e),
    selection: rgb(0x58, 0x5b, 0x70),
    ansi: [
        rgb(0x45, 0x47, 0x5a),
        rgb(0xf3, 0x8b, 0xa8),
        rgb(0xa6, 0xe3, 0xa1),
        rgb(0xf9, 0xe2, 0xaf),
        rgb(0x89, 0xb4, 0xfa),
        rgb(0xf5, 0xc2, 0xe7),
        rgb(0x94, 0xe2, 0xd5),
        rgb(0xba, 0xc2, 0xde),
        rgb(0x58, 0x5b, 0x70),
        rgb(0xf3, 0x8b, 0xa8),
        rgb(0xa6, 0xe3, 0xa1),
        rgb(0xf9, 0xe2, 0xaf),
        rgb(0x89, 0xb4, 0xfa),
        rgb(0xf5, 0xc2, 0xe7),
        rgb(0x94, 0xe2, 0xd5),
        rgb(0xa6, 0xad, 0xc8),
    ],
};

pub const SOLARIZED_DARK: TerminalPalette = TerminalPalette {
    background: rgb(0x00, 0x2b, 0x36),
    foreground: rgb(0x83, 0x94, 0x96),
    cursor: rgb(0x93, 0xa1, 0xa1),
    cursor_accent: rgb(0x00, 0x2b, 0x36),
    selection: rgb(0x07, 0x36, 0x42),
    ansi: [
        rgb(0x07, 0x36, 0x42),
        rgb(0xdc, 0x32, 0x2f),
        rgb(0x85, 0x99, 0x00),
        rgb(0xb5, 0x89, 0x00),
        rgb(0x26, 0x8b, 0xd2),
        rgb(0xd3, 0x36, 0x82),
        rgb(0x2a, 0xa1, 0x98),
        rgb(0xee, 0xe8, 0xd5),
        rgb(0x00, 0x2b, 0x36),
        rgb(0xcb, 0x4b, 0x16),
        rgb(0x58, 0x6e, 0x75),
        rgb(0x65, 0x7b, 0x83),
        rgb(0x83, 0x94, 0x96),
        rgb(0x6c, 0x71, 0xc4),
        rgb(0x93, 0xa1, 0xa1),
        rgb(0xfd, 0xf6, 0xe3),
    ],
};

pub fn base_palette() -> [Rgb; 256] {
    base_palette_for(RUNNER)
}

pub fn base_palette_for(theme: TerminalPalette) -> [Rgb; 256] {
    let mut palette = [Rgb::default(); 256];
    palette[..16].copy_from_slice(&theme.ansi);
    for index in 0..216 {
        let (r, g, b) = (index / 36, (index / 6) % 6, index % 6);
        let channel = |value: usize| {
            if value == 0 {
                0
            } else {
                (55 + 40 * value) as u8
            }
        };
        palette[16 + index] = rgb(channel(r), channel(g), channel(b));
    }
    for index in 0..24 {
        let value = (8 + 10 * index) as u8;
        palette[232 + index] = rgb(value, value, value);
    }
    palette
}

pub fn resolve_index(index: usize, palette: &[Rgb; 256]) -> Rgb {
    resolve_index_for(index, palette, RUNNER)
}

pub fn resolve_index_for(index: usize, palette: &[Rgb; 256], theme: TerminalPalette) -> Rgb {
    match index {
        0..=255 => palette[index],
        256 => theme.foreground,
        257 => theme.background,
        258 => theme.cursor,
        259..=266 => palette[index - 259] * 0.66,
        267 => theme.foreground,
        _ => theme.foreground * 0.66,
    }
}

pub fn resolve(color: Color, overrides: &Colors, palette: &[Rgb; 256]) -> Rgb {
    resolve_for(color, overrides, palette, RUNNER)
}

pub fn resolve_for(
    color: Color,
    overrides: &Colors,
    palette: &[Rgb; 256],
    theme: TerminalPalette,
) -> Rgb {
    match color {
        Color::Spec(rgb) => rgb,
        Color::Indexed(index) => overrides[index as usize].unwrap_or(palette[index as usize]),
        Color::Named(named) => overrides[named].unwrap_or_else(|| match named {
            NamedColor::Foreground | NamedColor::BrightForeground => theme.foreground,
            NamedColor::DimForeground => theme.foreground * 0.66,
            NamedColor::Background => theme.background,
            NamedColor::Cursor => theme.cursor,
            _ => {
                let index = named as usize;
                if index < 16 {
                    palette[index]
                } else {
                    palette[index.saturating_sub(259).min(7)] * 0.66
                }
            }
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_palettes_have_expected_roles() {
        assert_eq!(RUNNER.background, rgb(0x15, 0x16, 0x1b));
        assert_eq!(RUNNER.cursor, rgb(0x00, 0xff, 0x9c));
        assert_eq!(CATPPUCCIN_MOCHA.ansi[4], rgb(0x89, 0xb4, 0xfa));
        assert_eq!(SOLARIZED_DARK.selection, rgb(0x07, 0x36, 0x42));
    }

    #[test]
    fn named_slots_use_selected_palette() {
        let base = base_palette_for(CATPPUCCIN_MOCHA);
        assert_eq!(
            resolve_index_for(256, &base, CATPPUCCIN_MOCHA),
            CATPPUCCIN_MOCHA.foreground
        );
        assert_eq!(
            resolve_index_for(257, &base, CATPPUCCIN_MOCHA),
            CATPPUCCIN_MOCHA.background
        );
        assert_eq!(
            resolve_index_for(258, &base, CATPPUCCIN_MOCHA),
            CATPPUCCIN_MOCHA.cursor
        );
    }
}
