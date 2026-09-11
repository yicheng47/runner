use std::sync::atomic::{AtomicU8, Ordering};

use gpui::{rems, rgb, Hsla, Rems};
use serde::{Deserialize, Serialize};

pub use crate::platform_fonts::{SYSTEM_MONOSPACE_FONT, UI_MONOSPACE_FONT};

/// Type scale. Every text size in the app goes through one of these so the
/// steps stay in one place and scale with the window's rem size (app zoom).
/// 9px: avatar initials and the smallest badges.
pub fn text_micro() -> Rems {
    rems(9. / 16.)
}

/// 10px: captions, counters, and timestamps.
pub fn text_caption() -> Rems {
    rems(10. / 16.)
}

/// 11px: secondary metadata next to a row's main text.
pub fn text_meta() -> Rems {
    rems(11. / 16.)
}

/// 12px: dense UI chrome such as rows, chips, and field hints.
pub fn text_ui() -> Rems {
    rems(12. / 16.)
}

/// 13px: body copy such as feed messages, ask prompts, and sidebar rows.
pub fn text_body() -> Rems {
    rems(13. / 16.)
}

/// 14px: row titles, form fields, and default buttons.
pub fn text_title() -> Rems {
    rems(14. / 16.)
}

/// 15px: overlay and dialog titles.
pub fn text_lead() -> Rems {
    rems(15. / 16.)
}

/// 16px: section headings and empty-state titles.
pub fn text_heading() -> Rems {
    rems(1.)
}

/// 20px: page titles.
pub fn text_display() -> Rems {
    rems(20. / 16.)
}

/// 24px: hero copy on empty states.
pub fn text_display_lg() -> Rems {
    rems(24. / 16.)
}

/// 30px: single large figures.
pub fn text_display_xl() -> Rems {
    rems(30. / 16.)
}

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
    CatppuccinLatte,
    #[default]
    #[serde(other)]
    RunnerLight,
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
    RunnerLight,
    CatppuccinLatte,
}

impl ThemeVariant {
    pub fn is_light(self) -> bool {
        matches!(self, Self::RunnerLight | Self::CatppuccinLatte)
    }
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
            LightTheme::RunnerLight => ThemeVariant::RunnerLight,
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

pub const RUNNER_LIGHT: ThemeColors = ThemeColors {
    bg: 0xf6f6f8,
    panel: 0xffffff,
    raised: 0xecedf1,
    line: 0xe4e5ea,
    line_strong: 0xd6d8df,
    sidebar: 0xeeeff3,
    sidebar_selected: 0xe1e3e9,
    sidebar_selected_border: 0xd3d6de,
    fg: 0x1c1d22,
    fg_2: 0x5f616b,
    fg_3: 0x9a9ca6,
    accent: 0x00a66a,
    accent_ink: 0xffffff,
    warn: 0xc27c0e,
    danger: 0xd63b57,
    info: 0x0a8fb3,
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
        value if value == ThemeVariant::RunnerLight as u8 => ThemeVariant::RunnerLight,
        value if value == ThemeVariant::CatppuccinLatte as u8 => ThemeVariant::CatppuccinLatte,
        _ => ThemeVariant::Carbon,
    }
}

pub fn colors_for(variant: ThemeVariant) -> ThemeColors {
    match variant {
        ThemeVariant::Carbon => CARBON,
        ThemeVariant::CatppuccinMocha => CATPPUCCIN_MOCHA,
        ThemeVariant::RunnerLight => RUNNER_LIGHT,
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

pub fn info() -> Hsla {
    color(colors().info)
}

pub fn scrim() -> Hsla {
    gpui::hsla(
        0.,
        0.,
        0.,
        if active_variant().is_light() {
            0.2
        } else {
            0.35
        },
    )
}

pub fn window_close_hover() -> Hsla {
    color(0xc42b1c)
}

pub fn window_close_hover_ink() -> Hsla {
    color(0xffffff)
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
                LightTheme::RunnerLight,
                DarkTheme::CatppuccinMocha,
            ),
            ThemeVariant::CatppuccinMocha
        );
        assert_eq!(
            resolve_variant(
                ThemeIntent::Light,
                false,
                LightTheme::RunnerLight,
                DarkTheme::CatppuccinMocha,
            ),
            ThemeVariant::RunnerLight
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
    fn runner_light_is_the_default_and_migrates_codex() {
        assert_eq!(LightTheme::default(), LightTheme::RunnerLight);
        assert_eq!(
            serde_json::from_str::<LightTheme>(r#""codex""#).unwrap(),
            LightTheme::RunnerLight
        );
        for intent in [ThemeIntent::Auto, ThemeIntent::Light] {
            assert_eq!(
                resolve_variant(intent, true, LightTheme::default(), DarkTheme::default()),
                ThemeVariant::RunnerLight
            );
        }
        assert!(ThemeVariant::RunnerLight.is_light());
        assert!(ThemeVariant::CatppuccinLatte.is_light());
        assert!(!ThemeVariant::Carbon.is_light());
        assert!(!ThemeVariant::CatppuccinMocha.is_light());
    }

    #[test]
    fn runner_light_matches_the_signed_off_tokens() {
        assert_eq!(
            RUNNER_LIGHT,
            ThemeColors {
                bg: 0xf6f6f8,
                panel: 0xffffff,
                raised: 0xecedf1,
                line: 0xe4e5ea,
                line_strong: 0xd6d8df,
                sidebar: 0xeeeff3,
                sidebar_selected: 0xe1e3e9,
                sidebar_selected_border: 0xd3d6de,
                fg: 0x1c1d22,
                fg_2: 0x5f616b,
                fg_3: 0x9a9ca6,
                accent: 0x00a66a,
                accent_ink: 0xffffff,
                warn: 0xc27c0e,
                danger: 0xd63b57,
                info: 0x0a8fb3,
            }
        );
        let terminal_bg = runner_terminal::palette::RUNNER_LIGHT.background;
        assert_eq!(
            RUNNER_LIGHT.bg,
            u32::from(terminal_bg.r) << 16
                | u32::from(terminal_bg.g) << 8
                | u32::from(terminal_bg.b)
        );
    }

    #[test]
    fn shipped_roles_match_react_tokens() {
        assert_eq!(CARBON.accent, 0x00ff9c);
        assert_eq!(CATPPUCCIN_MOCHA.sidebar_selected, 0x3b3d52);
        assert_eq!(RUNNER_LIGHT.panel, 0xffffff);
        assert_eq!(CATPPUCCIN_LATTE.sidebar_selected_border, 0xc8ccda);
    }
}
