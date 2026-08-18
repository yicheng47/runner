use std::sync::atomic::{AtomicU8, Ordering};

use gpui::{rgb, Hsla};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeIntent {
    #[default]
    Auto,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LightTheme {
    #[default]
    Codex,
    CatppuccinLatte,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DarkTheme {
    #[default]
    #[serde(rename = "carbon")]
    Runner,
    CatppuccinMocha,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeVariant {
    #[default]
    Carbon,
    CatppuccinMocha,
    Codex,
    CatppuccinLatte,
}

pub fn resolve_variant(
    intent: ThemeIntent,
    system_is_light: bool,
    light: LightTheme,
    dark: DarkTheme,
) -> ThemeVariant {
    let use_light = match intent {
        ThemeIntent::Auto => system_is_light,
        ThemeIntent::Light => true,
        ThemeIntent::Dark => false,
    };
    if use_light {
        match light {
            LightTheme::Codex => ThemeVariant::Codex,
            LightTheme::CatppuccinLatte => ThemeVariant::CatppuccinLatte,
        }
    } else {
        match dark {
            DarkTheme::Runner => ThemeVariant::Carbon,
            DarkTheme::CatppuccinMocha => ThemeVariant::CatppuccinMocha,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeColors {
    pub bg: u32,
    pub panel: u32,
    pub raised: u32,
    pub line: u32,
    pub line_strong: u32,
    pub sidebar: u32,
    pub sidebar_selected: u32,
    pub sidebar_selected_border: u32,
    pub fg: u32,
    pub fg_2: u32,
    pub fg_3: u32,
    pub accent: u32,
    pub accent_ink: u32,
    pub warn: u32,
    pub danger: u32,
    pub info: u32,
}

pub const CARBON: ThemeColors = ThemeColors {
    bg: 0x15161b,
    panel: 0x1d1e23,
    raised: 0x272930,
    line: 0x272930,
    line_strong: 0x2a2c33,
    sidebar: 0x272930,
    sidebar_selected: 0x333640,
    sidebar_selected_border: 0x3b3e49,
    fg: 0xdcdce0,
    fg_2: 0x9a9ba5,
    fg_3: 0x5a5c66,
    accent: 0x00ff9c,
    accent_ink: 0x001f10,
    warn: 0xffb020,
    danger: 0xff4d6d,
    info: 0x39e5ff,
};

pub const CATPPUCCIN_MOCHA: ThemeColors = ThemeColors {
    bg: 0x1e1e2e,
    panel: 0x313244,
    raised: 0x45475a,
    line: 0x45475a,
    line_strong: 0x585b70,
    sidebar: 0x313244,
    sidebar_selected: 0x3b3d52,
    sidebar_selected_border: 0x51546b,
    fg: 0xcdd6f4,
    fg_2: 0xa6adc8,
    fg_3: 0x6c7086,
    accent: 0xcba6f7,
    accent_ink: 0x1e1e2e,
    warn: 0xf9e2af,
    danger: 0xf38ba8,
    info: 0x74c7ec,
};

pub const CODEX: ThemeColors = ThemeColors {
    bg: 0xffffff,
    panel: 0xf7f7f8,
    raised: 0xffffff,
    line: 0xe5e5e7,
    line_strong: 0xd1d1d6,
    sidebar: 0xf7f7f8,
    sidebar_selected: 0xeaeaed,
    sidebar_selected_border: 0xd8d8de,
    fg: 0x1a1c1f,
    fg_2: 0x6e6e73,
    fg_3: 0xa0a0a8,
    accent: 0x339cff,
    accent_ink: 0xffffff,
    warn: 0xf59e0b,
    danger: 0xe5484d,
    info: 0x0ea5e9,
};

pub const CATPPUCCIN_LATTE: ThemeColors = ThemeColors {
    bg: 0xeff1f5,
    panel: 0xe6e9ef,
    raised: 0xffffff,
    line: 0xccd0da,
    line_strong: 0xbcc0cc,
    sidebar: 0xe6e9ef,
    sidebar_selected: 0xdce0ea,
    sidebar_selected_border: 0xc8ccda,
    fg: 0x4c4f69,
    fg_2: 0x6c6f85,
    fg_3: 0x8c8fa1,
    accent: 0x8839ef,
    accent_ink: 0xffffff,
    warn: 0xdf8e1d,
    danger: 0xd20f39,
    info: 0x209fb5,
};

static ACTIVE_VARIANT: AtomicU8 = AtomicU8::new(ThemeVariant::Carbon as u8);

pub fn set_active_variant(variant: ThemeVariant) {
    ACTIVE_VARIANT.store(variant as u8, Ordering::Relaxed);
}

pub fn active_variant() -> ThemeVariant {
    match ACTIVE_VARIANT.load(Ordering::Relaxed) {
        value if value == ThemeVariant::CatppuccinMocha as u8 => ThemeVariant::CatppuccinMocha,
        value if value == ThemeVariant::Codex as u8 => ThemeVariant::Codex,
        value if value == ThemeVariant::CatppuccinLatte as u8 => ThemeVariant::CatppuccinLatte,
        _ => ThemeVariant::Carbon,
    }
}

pub fn colors_for(variant: ThemeVariant) -> ThemeColors {
    match variant {
        ThemeVariant::Carbon => CARBON,
        ThemeVariant::CatppuccinMocha => CATPPUCCIN_MOCHA,
        ThemeVariant::Codex => CODEX,
        ThemeVariant::CatppuccinLatte => CATPPUCCIN_LATTE,
    }
}

pub fn colors() -> ThemeColors {
    colors_for(active_variant())
}

fn color(value: u32) -> Hsla {
    rgb(value).into()
}

pub fn bg() -> Hsla {
    color(colors().bg)
}

pub fn panel() -> Hsla {
    color(colors().panel)
}

pub fn raised() -> Hsla {
    color(colors().raised)
}

pub fn text() -> Hsla {
    color(colors().fg)
}

pub fn muted() -> Hsla {
    color(colors().fg_2)
}

pub fn faint() -> Hsla {
    color(colors().fg_3)
}

pub fn accent() -> Hsla {
    color(colors().accent)
}

pub fn accent_ink() -> Hsla {
    color(colors().accent_ink)
}

pub fn composer_bg() -> Hsla {
    panel()
}

pub fn border() -> Hsla {
    color(colors().line)
}

pub fn border_strong() -> Hsla {
    color(colors().line_strong)
}

pub fn sidebar() -> Hsla {
    color(colors().sidebar)
}

pub fn sidebar_selected() -> Hsla {
    color(colors().sidebar_selected)
}

pub fn sidebar_selected_border() -> Hsla {
    color(colors().sidebar_selected_border)
}

pub fn danger() -> Hsla {
    color(colors().danger)
}

pub fn warning() -> Hsla {
    color(colors().warn)
}

pub fn with_alpha(mut color: Hsla, alpha: f32) -> Hsla {
    color.a = alpha;
    color
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_auto_and_explicit_intents() {
        assert_eq!(
            resolve_variant(
                ThemeIntent::Auto,
                true,
                LightTheme::CatppuccinLatte,
                DarkTheme::Runner,
            ),
            ThemeVariant::CatppuccinLatte
        );
        assert_eq!(
            resolve_variant(
                ThemeIntent::Auto,
                false,
                LightTheme::Codex,
                DarkTheme::CatppuccinMocha,
            ),
            ThemeVariant::CatppuccinMocha
        );
        assert_eq!(
            resolve_variant(
                ThemeIntent::Light,
                false,
                LightTheme::Codex,
                DarkTheme::CatppuccinMocha,
            ),
            ThemeVariant::Codex
        );
        assert_eq!(
            resolve_variant(
                ThemeIntent::Dark,
                true,
                LightTheme::CatppuccinLatte,
                DarkTheme::Runner,
            ),
            ThemeVariant::Carbon
        );
    }

    #[test]
    fn shipped_roles_match_react_tokens() {
        assert_eq!(CARBON.accent, 0x00ff9c);
        assert_eq!(CATPPUCCIN_MOCHA.sidebar_selected, 0x3b3d52);
        assert_eq!(CODEX.panel, 0xf7f7f8);
        assert_eq!(CATPPUCCIN_LATTE.sidebar_selected_border, 0xc8ccda);
    }
}
