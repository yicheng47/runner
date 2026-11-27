//! UI-agnostic terminal palettes and color resolution.

use std::ops::Mul;

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi as alacritty_ansi;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    #[inline]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    #[inline]
    pub(crate) fn from_alacritty(rgb: alacritty_ansi::Rgb) -> Self {
        Self {
            r: rgb.r,
            g: rgb.g,
            b: rgb.b,
        }
    }

    #[inline]
    pub(crate) fn to_alacritty(self) -> alacritty_ansi::Rgb {
        alacritty_ansi::Rgb {
            r: self.r,
            g: self.g,
            b: self.b,
        }
    }
}

impl Mul<f32> for Rgb {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: f32) -> Self::Output {
        Self {
            r: (f32::from(self.r) * rhs).clamp(0.0, 255.0) as u8,
            g: (f32::from(self.g) * rhs).clamp(0.0, 255.0) as u8,
            b: (f32::from(self.b) * rhs).clamp(0.0, 255.0) as u8,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NamedColor {
    Black = 0,
    Red = 1,
    Green = 2,
    Yellow = 3,
    Blue = 4,
    Magenta = 5,
    Cyan = 6,
    White = 7,
    BrightBlack = 8,
    BrightRed = 9,
    BrightGreen = 10,
    BrightYellow = 11,
    BrightBlue = 12,
    BrightMagenta = 13,
    BrightCyan = 14,
    BrightWhite = 15,
    Foreground = 256,
    Background = 257,
    Cursor = 258,
    DimBlack = 259,
    DimRed = 260,
    DimGreen = 261,
    DimYellow = 262,
    DimBlue = 263,
    DimMagenta = 264,
    DimCyan = 265,
    DimWhite = 266,
    BrightForeground = 267,
    DimForeground = 268,
}

impl NamedColor {
    #[inline]
    pub(crate) fn from_alacritty(named: alacritty_ansi::NamedColor) -> Self {
        match named {
            alacritty_ansi::NamedColor::Black => Self::Black,
            alacritty_ansi::NamedColor::Red => Self::Red,
            alacritty_ansi::NamedColor::Green => Self::Green,
            alacritty_ansi::NamedColor::Yellow => Self::Yellow,
            alacritty_ansi::NamedColor::Blue => Self::Blue,
            alacritty_ansi::NamedColor::Magenta => Self::Magenta,
            alacritty_ansi::NamedColor::Cyan => Self::Cyan,
            alacritty_ansi::NamedColor::White => Self::White,
            alacritty_ansi::NamedColor::BrightBlack => Self::BrightBlack,
            alacritty_ansi::NamedColor::BrightRed => Self::BrightRed,
            alacritty_ansi::NamedColor::BrightGreen => Self::BrightGreen,
            alacritty_ansi::NamedColor::BrightYellow => Self::BrightYellow,
            alacritty_ansi::NamedColor::BrightBlue => Self::BrightBlue,
            alacritty_ansi::NamedColor::BrightMagenta => Self::BrightMagenta,
            alacritty_ansi::NamedColor::BrightCyan => Self::BrightCyan,
            alacritty_ansi::NamedColor::BrightWhite => Self::BrightWhite,
            alacritty_ansi::NamedColor::Foreground => Self::Foreground,
            alacritty_ansi::NamedColor::Background => Self::Background,
            alacritty_ansi::NamedColor::Cursor => Self::Cursor,
            alacritty_ansi::NamedColor::DimBlack => Self::DimBlack,
            alacritty_ansi::NamedColor::DimRed => Self::DimRed,
            alacritty_ansi::NamedColor::DimGreen => Self::DimGreen,
            alacritty_ansi::NamedColor::DimYellow => Self::DimYellow,
            alacritty_ansi::NamedColor::DimBlue => Self::DimBlue,
            alacritty_ansi::NamedColor::DimMagenta => Self::DimMagenta,
            alacritty_ansi::NamedColor::DimCyan => Self::DimCyan,
            alacritty_ansi::NamedColor::DimWhite => Self::DimWhite,
            alacritty_ansi::NamedColor::BrightForeground => Self::BrightForeground,
            alacritty_ansi::NamedColor::DimForeground => Self::DimForeground,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Color {
    Rgb(Rgb),
    Indexed(u8),
    Named(NamedColor),
}

impl Color {
    #[inline]
    pub(crate) fn from_alacritty(color: alacritty_ansi::Color) -> Self {
        match color {
            alacritty_ansi::Color::Spec(rgb) => Self::Rgb(Rgb::from_alacritty(rgb)),
            alacritty_ansi::Color::Indexed(index) => Self::Indexed(index),
            alacritty_ansi::Color::Named(named) => Self::Named(NamedColor::from_alacritty(named)),
        }
    }
}

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

impl TerminalPalette {
    /// Whether the ground reads as light, the DEC 2031 sense of "colour
    /// scheme" that a TUI on auto keys its own paint set off.
    pub fn is_light(&self) -> bool {
        let Rgb { r, g, b } = self.background;
        0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b) > 127.5
    }
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
        rgb(0x61, 0xaf, 0xef),
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

/// Rosé Pine Dawn with Runner’s muted ground: black is overlay, white is
/// text and cyan is rose, as the theme itself defines them.
pub const ROSE_PINE_DAWN: TerminalPalette = TerminalPalette {
    background: rgb(0xf5, 0xf1, 0xeb),
    foreground: rgb(0x57, 0x52, 0x79),
    cursor: rgb(0x57, 0x52, 0x79),
    cursor_accent: rgb(0xf5, 0xf1, 0xeb),
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

pub const CATPPUCCIN_LATTE: TerminalPalette = TerminalPalette {
    background: rgb(0xef, 0xf1, 0xf5),
    foreground: rgb(0x4c, 0x4f, 0x69),
    cursor: rgb(0xdc, 0x8a, 0x78),
    cursor_accent: rgb(0xef, 0xf1, 0xf5),
    selection: rgb(0xbc, 0xc0, 0xcc),
    ansi: [
        rgb(0xbc, 0xc0, 0xcc),
        rgb(0xd2, 0x0f, 0x39),
        rgb(0x40, 0xa0, 0x2b),
        rgb(0xdf, 0x8e, 0x1d),
        rgb(0x1e, 0x66, 0xf5),
        rgb(0xea, 0x76, 0xcb),
        rgb(0x17, 0x92, 0x99),
        rgb(0x5c, 0x5f, 0x77),
        rgb(0xac, 0xb0, 0xbe),
        rgb(0xd2, 0x0f, 0x39),
        rgb(0x40, 0xa0, 0x2b),
        rgb(0xdf, 0x8e, 0x1d),
        rgb(0x1e, 0x66, 0xf5),
        rgb(0xea, 0x76, 0xcb),
        rgb(0x17, 0x92, 0x99),
        rgb(0x6c, 0x6f, 0x85),
    ],
};

pub const ROSE_PINE_MOON: TerminalPalette = TerminalPalette {
    background: rgb(0x23, 0x21, 0x36),
    foreground: rgb(0xe0, 0xde, 0xf4),
    cursor: rgb(0xe0, 0xde, 0xf4),
    cursor_accent: rgb(0x23, 0x21, 0x36),
    selection: rgb(0x44, 0x41, 0x5a),
    ansi: [
        rgb(0x39, 0x35, 0x52),
        rgb(0xeb, 0x6f, 0x92),
        rgb(0x3e, 0x8f, 0xb0),
        rgb(0xf6, 0xc1, 0x77),
        rgb(0x9c, 0xcf, 0xd8),
        rgb(0xc4, 0xa7, 0xe7),
        rgb(0xea, 0x9a, 0x97),
        rgb(0xe0, 0xde, 0xf4),
        rgb(0x6e, 0x6a, 0x86),
        rgb(0xeb, 0x6f, 0x92),
        rgb(0x3e, 0x8f, 0xb0),
        rgb(0xf6, 0xc1, 0x77),
        rgb(0x9c, 0xcf, 0xd8),
        rgb(0xc4, 0xa7, 0xe7),
        rgb(0xea, 0x9a, 0x97),
        rgb(0xe0, 0xde, 0xf4),
    ],
};

pub const TOKYO_NIGHT_DAY: TerminalPalette = TerminalPalette {
    background: rgb(0xe1, 0xe2, 0xe7),
    foreground: rgb(0x37, 0x60, 0xbf),
    cursor: rgb(0x37, 0x60, 0xbf),
    cursor_accent: rgb(0xe1, 0xe2, 0xe7),
    selection: rgb(0xb7, 0xc1, 0xe3),
    ansi: [
        rgb(0xb4, 0xb5, 0xb9),
        rgb(0xf5, 0x2a, 0x65),
        rgb(0x58, 0x75, 0x39),
        rgb(0x8c, 0x6c, 0x3e),
        rgb(0x2e, 0x7d, 0xe9),
        rgb(0x98, 0x54, 0xf1),
        rgb(0x00, 0x71, 0x97),
        rgb(0x61, 0x72, 0xb0),
        rgb(0xa1, 0xa6, 0xc5),
        rgb(0xff, 0x47, 0x74),
        rgb(0x5c, 0x85, 0x24),
        rgb(0xa2, 0x76, 0x29),
        rgb(0x35, 0x8a, 0xff),
        rgb(0xa4, 0x63, 0xff),
        rgb(0x00, 0x7e, 0xa8),
        rgb(0x37, 0x60, 0xbf),
    ],
};

pub const TOKYO_NIGHT: TerminalPalette = TerminalPalette {
    background: rgb(0x1a, 0x1b, 0x26),
    foreground: rgb(0xc0, 0xca, 0xf5),
    cursor: rgb(0xc0, 0xca, 0xf5),
    cursor_accent: rgb(0x1a, 0x1b, 0x26),
    selection: rgb(0x28, 0x34, 0x57),
    ansi: [
        rgb(0x15, 0x16, 0x1e),
        rgb(0xf7, 0x76, 0x8e),
        rgb(0x9e, 0xce, 0x6a),
        rgb(0xe0, 0xaf, 0x68),
        rgb(0x7a, 0xa2, 0xf7),
        rgb(0xbb, 0x9a, 0xf7),
        rgb(0x7d, 0xcf, 0xff),
        rgb(0xa9, 0xb1, 0xd6),
        rgb(0x41, 0x48, 0x68),
        rgb(0xff, 0x89, 0x9d),
        rgb(0x9f, 0xe0, 0x44),
        rgb(0xfa, 0xba, 0x4a),
        rgb(0x8d, 0xb0, 0xff),
        rgb(0xc7, 0xa9, 0xff),
        rgb(0xa4, 0xda, 0xff),
        rgb(0xc0, 0xca, 0xf5),
    ],
};

pub const GRUVBOX_LIGHT: TerminalPalette = TerminalPalette {
    background: rgb(0xfb, 0xf1, 0xc7),
    foreground: rgb(0x3c, 0x38, 0x36),
    cursor: rgb(0x3c, 0x38, 0x36),
    cursor_accent: rgb(0xfb, 0xf1, 0xc7),
    selection: rgb(0xd5, 0xc4, 0xa1),
    ansi: [
        rgb(0xfb, 0xf1, 0xc7),
        rgb(0xcc, 0x24, 0x1d),
        rgb(0x98, 0x97, 0x1a),
        rgb(0xd7, 0x99, 0x21),
        rgb(0x45, 0x85, 0x88),
        rgb(0xb1, 0x62, 0x86),
        rgb(0x68, 0x9d, 0x6a),
        rgb(0x7c, 0x6f, 0x64),
        rgb(0x92, 0x83, 0x74),
        rgb(0x9d, 0x00, 0x06),
        rgb(0x79, 0x74, 0x0e),
        rgb(0xb5, 0x76, 0x14),
        rgb(0x07, 0x66, 0x78),
        rgb(0x8f, 0x3f, 0x71),
        rgb(0x42, 0x7b, 0x58),
        rgb(0x3c, 0x38, 0x36),
    ],
};

pub const GRUVBOX_DARK: TerminalPalette = TerminalPalette {
    background: rgb(0x28, 0x28, 0x28),
    foreground: rgb(0xeb, 0xdb, 0xb2),
    cursor: rgb(0xeb, 0xdb, 0xb2),
    cursor_accent: rgb(0x28, 0x28, 0x28),
    selection: rgb(0x50, 0x49, 0x45),
    ansi: [
        rgb(0x28, 0x28, 0x28),
        rgb(0xcc, 0x24, 0x1d),
        rgb(0x98, 0x97, 0x1a),
        rgb(0xd7, 0x99, 0x21),
        rgb(0x45, 0x85, 0x88),
        rgb(0xb1, 0x62, 0x86),
        rgb(0x68, 0x9d, 0x6a),
        rgb(0xa8, 0x99, 0x84),
        rgb(0x92, 0x83, 0x74),
        rgb(0xfb, 0x49, 0x34),
        rgb(0xb8, 0xbb, 0x26),
        rgb(0xfa, 0xbd, 0x2f),
        rgb(0x83, 0xa5, 0x98),
        rgb(0xd3, 0x86, 0x9b),
        rgb(0x8e, 0xc0, 0x7c),
        rgb(0xeb, 0xdb, 0xb2),
    ],
};

pub const SOLARIZED_LIGHT: TerminalPalette = TerminalPalette {
    background: rgb(0xfd, 0xf6, 0xe3),
    foreground: rgb(0x65, 0x7b, 0x83),
    cursor: rgb(0x65, 0x7b, 0x83),
    cursor_accent: rgb(0xfd, 0xf6, 0xe3),
    selection: rgb(0xe4, 0xdc, 0xc4),
    ansi: [
        rgb(0x07, 0x36, 0x42),
        rgb(0xdc, 0x32, 0x2f),
        rgb(0x85, 0x99, 0x00),
        rgb(0xb5, 0x89, 0x00),
        rgb(0x26, 0x8b, 0xd2),
        rgb(0xd3, 0x36, 0x82),
        rgb(0x2a, 0xa1, 0x98),
        rgb(0xee, 0xe8, 0xd5),
        // base1 avoids matching dark ink on comment/dim ANSI 8 slots on the cream ground.
        rgb(0x93, 0xa1, 0xa1),
        rgb(0xcb, 0x4b, 0x16),
        rgb(0x58, 0x6e, 0x75),
        rgb(0x65, 0x7b, 0x83),
        rgb(0x83, 0x94, 0x96),
        rgb(0x6c, 0x71, 0xc4),
        rgb(0x93, 0xa1, 0xa1),
        rgb(0xfd, 0xf6, 0xe3),
    ],
};

pub const SOLARIZED_DARK: TerminalPalette = TerminalPalette {
    background: rgb(0x00, 0x2b, 0x36),
    foreground: rgb(0x83, 0x94, 0x96),
    cursor: rgb(0x83, 0x94, 0x96),
    cursor_accent: rgb(0x00, 0x2b, 0x36),
    selection: rgb(0x0e, 0x45, 0x52),
    ansi: [
        rgb(0x07, 0x36, 0x42),
        rgb(0xdc, 0x32, 0x2f),
        rgb(0x85, 0x99, 0x00),
        rgb(0xb5, 0x89, 0x00),
        rgb(0x26, 0x8b, 0xd2),
        rgb(0xd3, 0x36, 0x82),
        rgb(0x2a, 0xa1, 0x98),
        rgb(0xee, 0xe8, 0xd5),
        // base01 avoids vanishing against the base03 ground when ANSI 8 renders dim text or comments.
        rgb(0x58, 0x6e, 0x75),
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

pub(crate) fn resolve_with_alacritty_colors(
    color: Color,
    overrides: &Colors,
    palette: &[Rgb; 256],
    theme: TerminalPalette,
) -> Rgb {
    match color {
        Color::Rgb(rgb) => rgb,
        Color::Indexed(index) => overrides[index as usize]
            .map(Rgb::from_alacritty)
            .unwrap_or(palette[index as usize]),
        Color::Named(named) => overrides[named as usize]
            .map(Rgb::from_alacritty)
            .unwrap_or_else(|| match named {
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
        assert_eq!(RUNNER.ansi[4], rgb(0x61, 0xaf, 0xef));
        assert_eq!(RUNNER.ansi[6], rgb(0x39, 0xe5, 0xff));
        assert_eq!(RUNNER.ansi[12], rgb(0x82, 0xaa, 0xff));
        assert_eq!(CATPPUCCIN_MOCHA.ansi[4], rgb(0x89, 0xb4, 0xfa));
        assert_eq!(CATPPUCCIN_LATTE.background, rgb(0xef, 0xf1, 0xf5));
        assert_eq!(ROSE_PINE_MOON.background, rgb(0x23, 0x21, 0x36));
        assert_eq!(TOKYO_NIGHT_DAY.background, rgb(0xe1, 0xe2, 0xe7));
        assert_eq!(TOKYO_NIGHT.background, rgb(0x1a, 0x1b, 0x26));
        assert_eq!(GRUVBOX_LIGHT.background, rgb(0xfb, 0xf1, 0xc7));
        assert_eq!(GRUVBOX_DARK.background, rgb(0x28, 0x28, 0x28));
        assert_eq!(SOLARIZED_LIGHT.background, rgb(0xfd, 0xf6, 0xe3));
        assert_eq!(SOLARIZED_DARK.background, rgb(0x00, 0x2b, 0x36));
    }

    #[test]
    fn lightness_follows_the_ground() {
        for dark in [
            RUNNER,
            CATPPUCCIN_MOCHA,
            ROSE_PINE_MOON,
            TOKYO_NIGHT,
            GRUVBOX_DARK,
            SOLARIZED_DARK,
        ] {
            assert!(!dark.is_light(), "{dark:?}");
        }
        for light in [
            ROSE_PINE_DAWN,
            CATPPUCCIN_LATTE,
            TOKYO_NIGHT_DAY,
            GRUVBOX_LIGHT,
            SOLARIZED_LIGHT,
        ] {
            assert!(light.is_light(), "{light:?}");
        }
    }

    #[test]
    fn rose_pine_dawn_uses_runner_ground_and_upstream_ink() {
        assert_eq!(ROSE_PINE_DAWN.background, rgb(0xf5, 0xf1, 0xeb));
        assert_eq!(ROSE_PINE_DAWN.foreground, rgb(0x57, 0x52, 0x79));
        assert_eq!(ROSE_PINE_DAWN.cursor_accent, ROSE_PINE_DAWN.background);
        assert_eq!(ROSE_PINE_DAWN.ansi[0], rgb(0xf2, 0xe9, 0xe1));
        assert_eq!(ROSE_PINE_DAWN.ansi[6], rgb(0xd7, 0x82, 0x7e));
        assert_eq!(ROSE_PINE_DAWN.ansi[15], ROSE_PINE_DAWN.foreground);
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
