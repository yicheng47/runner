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
    selection: rgb(0x3b, 0x3e, 0x49),
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

pub const RUNNER_LIGHT: TerminalPalette = TerminalPalette {
    background: rgb(0xf6, 0xf6, 0xf8),
    foreground: rgb(0x1c, 0x1d, 0x22),
    cursor: rgb(0x00, 0xa6, 0x6a),
    cursor_accent: rgb(0xf6, 0xf6, 0xf8),
    selection: rgb(0xd6, 0xe9, 0xdf),
    ansi: [
        rgb(0x5f, 0x61, 0x6b),
        rgb(0xd6, 0x3b, 0x57),
        rgb(0x00, 0xa6, 0x6a),
        rgb(0xc2, 0x7c, 0x0e),
        rgb(0x25, 0x63, 0xeb),
        rgb(0x7c, 0x3a, 0xed),
        rgb(0x0a, 0x8f, 0xb3),
        rgb(0xb4, 0xb7, 0xc1),
        rgb(0x6e, 0x71, 0x7c),
        rgb(0xe0, 0x52, 0x6e),
        rgb(0x0b, 0xbf, 0x80),
        rgb(0xd9, 0x93, 0x2a),
        rgb(0x3b, 0x82, 0xf6),
        rgb(0xa7, 0x8b, 0xfa),
        rgb(0x22, 0xa6, 0xc9),
        rgb(0xc9, 0xcb, 0xd3),
    ],
};

pub const BACKGROUND: Rgb = RUNNER.background;
pub const FOREGROUND: Rgb = RUNNER.foreground;
pub const CURSOR: Rgb = RUNNER.cursor;

/// Official Rosé Pine Dawn terminal values: black is overlay, white is
/// text, cyan is rose, as the theme itself defines them.
pub const ROSE_PINE_DAWN: TerminalPalette = TerminalPalette {
    background: rgb(0xfa, 0xf4, 0xed),
    foreground: rgb(0x57, 0x52, 0x79),
    cursor: rgb(0x57, 0x52, 0x79),
    cursor_accent: rgb(0xfa, 0xf4, 0xed),
    selection: rgb(0xdf, 0xda, 0xd9),
    ansi: [
        rgb(0xf2, 0xe9, 0xe1),
        rgb(0xb4, 0x63, 0x7a),
        rgb(0x28, 0x69, 0x83),
        rgb(0xea, 0x9d, 0x34),
        rgb(0x56, 0x94, 0x9f),
        rgb(0x90, 0x7a, 0xa9),
        rgb(0xd7, 0x82, 0x7e),
        rgb(0x57, 0x52, 0x79),
        rgb(0x98, 0x93, 0xa5),
        rgb(0xb4, 0x63, 0x7a),
        rgb(0x28, 0x69, 0x83),
        rgb(0xea, 0x9d, 0x34),
        rgb(0x56, 0x94, 0x9f),
        rgb(0x90, 0x7a, 0xa9),
        rgb(0xd7, 0x82, 0x7e),
        rgb(0x57, 0x52, 0x79),
    ],
};

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

pub const MONOKAI: TerminalPalette = TerminalPalette {
    background: rgb(0x2d, 0x2a, 0x2e),
    foreground: rgb(0xfc, 0xfc, 0xfa),
    cursor: rgb(0xc1, 0xc0, 0xc0),
    cursor_accent: rgb(0x8e, 0x8d, 0x8d),
    selection: rgb(0x5b, 0x59, 0x5c),
    ansi: [
        rgb(0x2d, 0x2a, 0x2e),
        rgb(0xff, 0x61, 0x88),
        rgb(0xa9, 0xdc, 0x76),
        rgb(0xff, 0xd8, 0x66),
        rgb(0xfc, 0x98, 0x67),
        rgb(0xab, 0x9d, 0xf2),
        rgb(0x78, 0xdc, 0xe8),
        rgb(0xfc, 0xfc, 0xfa),
        rgb(0x72, 0x70, 0x72),
        rgb(0xff, 0x61, 0x88),
        rgb(0xa9, 0xdc, 0x76),
        rgb(0xff, 0xd8, 0x66),
        rgb(0xfc, 0x98, 0x67),
        rgb(0xab, 0x9d, 0xf2),
        rgb(0x78, 0xdc, 0xe8),
        rgb(0xfc, 0xfc, 0xfa),
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
        assert_eq!(RUNNER.selection, rgb(0x3b, 0x3e, 0x49));
        assert_eq!(RUNNER.cursor, rgb(0x00, 0xff, 0x9c));
        assert_eq!(CATPPUCCIN_MOCHA.ansi[4], rgb(0x89, 0xb4, 0xfa));
        assert_eq!(MONOKAI.ansi[4], rgb(0xfc, 0x98, 0x67));
        assert_eq!(MONOKAI.cursor, rgb(0xc1, 0xc0, 0xc0));
    }

    #[test]
    fn rose_pine_dawn_matches_the_official_palette() {
        assert_eq!(ROSE_PINE_DAWN.background, rgb(0xfa, 0xf4, 0xed));
        assert_eq!(ROSE_PINE_DAWN.foreground, rgb(0x57, 0x52, 0x79));
        assert_eq!(ROSE_PINE_DAWN.cursor_accent, ROSE_PINE_DAWN.background);
        assert_eq!(ROSE_PINE_DAWN.ansi[0], rgb(0xf2, 0xe9, 0xe1));
        assert_eq!(ROSE_PINE_DAWN.ansi[6], rgb(0xd7, 0x82, 0x7e));
        assert_eq!(ROSE_PINE_DAWN.ansi[15], ROSE_PINE_DAWN.foreground);
    }

    #[test]
    fn runner_light_matches_the_signed_off_palette() {
        assert_eq!(RUNNER_LIGHT.background, rgb(0xf6, 0xf6, 0xf8));
        assert_eq!(RUNNER_LIGHT.foreground, rgb(0x1c, 0x1d, 0x22));
        assert_eq!(RUNNER_LIGHT.cursor, rgb(0x00, 0xa6, 0x6a));
        assert_eq!(RUNNER_LIGHT.cursor_accent, RUNNER_LIGHT.background);
        assert_eq!(RUNNER_LIGHT.selection, rgb(0xd6, 0xe9, 0xdf));
        let ansi = [
            0x5f616b, 0xd63b57, 0x00a66a, 0xc27c0e, 0x2563eb, 0x7c3aed, 0x0a8fb3, 0xb4b7c1,
            0x6e717c, 0xe0526e, 0x0bbf80, 0xd9932a, 0x3b82f6, 0xa78bfa, 0x22a6c9, 0xc9cbd3,
        ];
        assert_eq!(
            RUNNER_LIGHT.ansi,
            ansi.map(|value| rgb((value >> 16) as u8, (value >> 8) as u8, value as u8))
        );
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
