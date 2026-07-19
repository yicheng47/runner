//! Color resolution. `alacritty_terminal` ships no default palette —
//! the embedder supplies 256 colors plus fg/bg; `term.colors()` only
//! overrides slots the running program changed via OSC.
//!
//! Base 16 are Tokyo Night terminal colors (Runner's default theme).

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

pub const BACKGROUND: Rgb = Rgb {
    r: 0x1a,
    g: 0x1b,
    b: 0x26,
};
pub const FOREGROUND: Rgb = Rgb {
    r: 0xc0,
    g: 0xca,
    b: 0xf5,
};
pub const CURSOR: Rgb = Rgb {
    r: 0xc0,
    g: 0xca,
    b: 0xf5,
};

const ANSI16: [Rgb; 16] = [
    Rgb {
        r: 0x15,
        g: 0x16,
        b: 0x1e,
    }, // black
    Rgb {
        r: 0xf7,
        g: 0x76,
        b: 0x8e,
    }, // red
    Rgb {
        r: 0x9e,
        g: 0xce,
        b: 0x6a,
    }, // green
    Rgb {
        r: 0xe0,
        g: 0xaf,
        b: 0x68,
    }, // yellow
    Rgb {
        r: 0x7a,
        g: 0xa2,
        b: 0xf7,
    }, // blue
    Rgb {
        r: 0xbb,
        g: 0x9a,
        b: 0xf7,
    }, // magenta
    Rgb {
        r: 0x7d,
        g: 0xcf,
        b: 0xff,
    }, // cyan
    Rgb {
        r: 0xa9,
        g: 0xb1,
        b: 0xd6,
    }, // white
    Rgb {
        r: 0x41,
        g: 0x48,
        b: 0x68,
    }, // bright black
    Rgb {
        r: 0xf7,
        g: 0x76,
        b: 0x8e,
    },
    Rgb {
        r: 0x9e,
        g: 0xce,
        b: 0x6a,
    },
    Rgb {
        r: 0xe0,
        g: 0xaf,
        b: 0x68,
    },
    Rgb {
        r: 0x7a,
        g: 0xa2,
        b: 0xf7,
    },
    Rgb {
        r: 0xbb,
        g: 0x9a,
        b: 0xf7,
    },
    Rgb {
        r: 0x7d,
        g: 0xcf,
        b: 0xff,
    },
    Rgb {
        r: 0xc0,
        g: 0xca,
        b: 0xf5,
    }, // bright white
];

pub fn base_palette() -> [Rgb; 256] {
    let mut p = [Rgb::default(); 256];
    p[..16].copy_from_slice(&ANSI16);
    // 6x6x6 color cube.
    for i in 0..216 {
        let (r, g, b) = (i / 36, (i / 6) % 6, i % 6);
        let ch = |n: usize| if n == 0 { 0 } else { (55 + 40 * n) as u8 };
        p[16 + i] = Rgb {
            r: ch(r),
            g: ch(g),
            b: ch(b),
        };
    }
    // Grayscale ramp.
    for i in 0..24 {
        let v = (8 + 10 * i) as u8;
        p[232 + i] = Rgb { r: v, g: v, b: v };
    }
    p
}

/// Resolve a raw color-table index as delivered by
/// `Event::ColorRequest` (OSC 4/10/11/12 queries): 0-255 are palette
/// slots, 256+ are alacritty's named slots (`NamedColor`
/// discriminants — 256 Foreground, 257 Background, 258 Cursor,
/// 259-266 dim ANSI, 267 BrightForeground, 268 DimForeground).
pub fn resolve_index(index: usize, palette: &[Rgb; 256]) -> Rgb {
    match index {
        0..=255 => palette[index],
        256 => FOREGROUND,
        257 => BACKGROUND,
        258 => CURSOR,
        259..=266 => palette[index - 259] * 0.66,
        267 => FOREGROUND,
        _ => FOREGROUND * 0.66,
    }
}

/// Resolve a cell color against runtime OSC overrides, then the base
/// palette.
pub fn resolve(color: Color, overrides: &Colors, palette: &[Rgb; 256]) -> Rgb {
    match color {
        Color::Spec(rgb) => rgb,
        Color::Indexed(i) => overrides[i as usize].unwrap_or(palette[i as usize]),
        Color::Named(named) => overrides[named].unwrap_or_else(|| match named {
            NamedColor::Foreground | NamedColor::BrightForeground => FOREGROUND,
            NamedColor::DimForeground => FOREGROUND * 0.66,
            NamedColor::Background => BACKGROUND,
            NamedColor::Cursor => CURSOR,
            _ => {
                let idx = named as usize;
                if idx < 16 {
                    palette[idx]
                } else {
                    // Dim variants (259..=266) map to base * 0.66.
                    palette[idx.saturating_sub(259).min(7)] * 0.66
                }
            }
        }),
    }
}
