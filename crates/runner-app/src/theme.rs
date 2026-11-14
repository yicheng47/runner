use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use gpui::{rems, rgb, Hsla, Rems};
use runner_terminal::palette::{self, TerminalPalette};
use serde::{Deserialize, Deserializer, Serialize};

/// Bundled third-party theme licence notices, registered in [`crate::assets::Assets`] and platform packages.
pub(crate) const SHIPPED_NOTICES: &str = include_str!("../LICENSE.themes");

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeIntent {
    #[default]
    Auto,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LightTheme {
    #[default]
    RunnerLight,
    CatppuccinLatte,
    RosePineDawn,
    TokyoNightDay,
    GruvboxLight,
    SolarizedLight,
}

impl LightTheme {
    pub const ALL: [Self; 6] = [
        Self::RunnerLight,
        Self::CatppuccinLatte,
        Self::RosePineDawn,
        Self::TokyoNightDay,
        Self::GruvboxLight,
        Self::SolarizedLight,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::RunnerLight => "Runner Light",
            Self::CatppuccinLatte => "Catppuccin Latte",
            Self::RosePineDawn => "Rosé Pine Dawn",
            Self::TokyoNightDay => "Tokyo Night Day",
            Self::GruvboxLight => "Gruvbox Light",
            Self::SolarizedLight => "Solarized Light",
        }
    }

    pub const fn key(self) -> &'static str {
        match self {
            Self::RunnerLight => "runner-light",
            Self::CatppuccinLatte => "catppuccin-latte",
            Self::RosePineDawn => "rose-pine-dawn",
            Self::TokyoNightDay => "tokyo-night-day",
            Self::GruvboxLight => "gruvbox-light",
            Self::SolarizedLight => "solarized-light",
        }
    }

    pub const fn variant(self) -> ThemeVariant {
        match self {
            Self::RunnerLight => ThemeVariant::RunnerLight,
            Self::CatppuccinLatte => ThemeVariant::CatppuccinLatte,
            Self::RosePineDawn => ThemeVariant::RosePineDawn,
            Self::TokyoNightDay => ThemeVariant::TokyoNightDay,
            Self::GruvboxLight => ThemeVariant::GruvboxLight,
            Self::SolarizedLight => ThemeVariant::SolarizedLight,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|theme| theme.key() == value)
    }
}

impl<'de> Deserialize<'de> for LightTheme {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_or_default(deserializer, Self::parse)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DarkTheme {
    #[default]
    #[serde(rename = "carbon")]
    Runner,
    CatppuccinMocha,
    RosePineMoon,
    TokyoNight,
    GruvboxDark,
    SolarizedDark,
}

impl DarkTheme {
    pub const ALL: [Self; 6] = [
        Self::Runner,
        Self::CatppuccinMocha,
        Self::RosePineMoon,
        Self::TokyoNight,
        Self::GruvboxDark,
        Self::SolarizedDark,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Runner => "Runner",
            Self::CatppuccinMocha => "Catppuccin Mocha",
            Self::RosePineMoon => "Rosé Pine Moon",
            Self::TokyoNight => "Tokyo Night",
            Self::GruvboxDark => "Gruvbox Dark",
            Self::SolarizedDark => "Solarized Dark",
        }
    }

    pub const fn key(self) -> &'static str {
        match self {
            Self::Runner => "carbon",
            Self::CatppuccinMocha => "catppuccin-mocha",
            Self::RosePineMoon => "rose-pine-moon",
            Self::TokyoNight => "tokyo-night",
            Self::GruvboxDark => "gruvbox-dark",
            Self::SolarizedDark => "solarized-dark",
        }
    }

    pub const fn variant(self) -> ThemeVariant {
        match self {
            Self::Runner => ThemeVariant::Carbon,
            Self::CatppuccinMocha => ThemeVariant::CatppuccinMocha,
            Self::RosePineMoon => ThemeVariant::RosePineMoon,
            Self::TokyoNight => ThemeVariant::TokyoNight,
            Self::GruvboxDark => ThemeVariant::GruvboxDark,
            Self::SolarizedDark => ThemeVariant::SolarizedDark,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|theme| theme.key() == value)
    }
}

impl<'de> Deserialize<'de> for DarkTheme {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_or_default(deserializer, Self::parse)
    }
}

fn deserialize_or_default<'de, D: Deserializer<'de>, T: Default>(
    deserializer: D,
    parse: fn(&str) -> Option<T>,
) -> Result<T, D::Error> {
    let value = String::deserialize(deserializer)?;
    Ok(parse(value.as_str()).unwrap_or_default())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeVariant {
    #[default]
    Carbon,
    CatppuccinMocha,
    RosePineMoon,
    TokyoNight,
    GruvboxDark,
    SolarizedDark,
    RunnerLight,
    CatppuccinLatte,
    RosePineDawn,
    TokyoNightDay,
    GruvboxLight,
    SolarizedLight,
}

impl ThemeVariant {
    pub const ALL: [Self; 12] = [
        Self::Carbon,
        Self::CatppuccinMocha,
        Self::RosePineMoon,
        Self::TokyoNight,
        Self::GruvboxDark,
        Self::SolarizedDark,
        Self::RunnerLight,
        Self::CatppuccinLatte,
        Self::RosePineDawn,
        Self::TokyoNightDay,
        Self::GruvboxLight,
        Self::SolarizedLight,
    ];

    pub const fn is_light(self) -> bool {
        matches!(
            self,
            Self::RunnerLight
                | Self::CatppuccinLatte
                | Self::RosePineDawn
                | Self::TokyoNightDay
                | Self::GruvboxLight
                | Self::SolarizedLight
        )
    }

    pub const fn terminal_palette(self) -> TerminalPalette {
        match self {
            Self::Carbon => palette::RUNNER,
            Self::CatppuccinMocha => palette::CATPPUCCIN_MOCHA,
            Self::RosePineMoon => palette::ROSE_PINE_MOON,
            Self::TokyoNight => palette::TOKYO_NIGHT,
            Self::GruvboxDark => palette::GRUVBOX_DARK,
            Self::SolarizedDark => palette::SOLARIZED_DARK,
            Self::RunnerLight | Self::RosePineDawn => palette::ROSE_PINE_DAWN,
            Self::CatppuccinLatte => palette::CATPPUCCIN_LATTE,
            Self::TokyoNightDay => palette::TOKYO_NIGHT_DAY,
            Self::GruvboxLight => palette::GRUVBOX_LIGHT,
            Self::SolarizedLight => palette::SOLARIZED_LIGHT,
        }
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
        light.variant()
    } else {
        dark.variant()
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
    bg: 0xf2f3f5,
    panel: 0xf8f8fa,
    raised: 0xecedf1,
    line: 0xd9dbe1,
    line_strong: 0xcdd0d7,
    sidebar: 0xe2e4e9,
    sidebar_selected: 0xd6d9e0,
    sidebar_selected_border: 0xcaced7,
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

pub const ROSE_PINE_MOON: ThemeColors = ThemeColors {
    bg: 0x232136,
    panel: 0x2a273f,
    raised: 0x393552,
    line: 0x393552,
    line_strong: 0x56526e,
    sidebar: 0x2a273f,
    sidebar_selected: 0x393552,
    sidebar_selected_border: 0x44415a,
    fg: 0xe0def4,
    fg_2: 0x908caa,
    fg_3: 0x6e6a86,
    accent: 0xea9a97,
    accent_ink: 0x232136,
    warn: 0xf6c177,
    danger: 0xeb6f92,
    info: 0x9ccfd8,
};

pub const TOKYO_NIGHT: ThemeColors = ThemeColors {
    bg: 0x1a1b26,
    panel: 0x24283b,
    raised: 0x2f3549,
    line: 0x292e42,
    line_strong: 0x3b4261,
    sidebar: 0x24283b,
    sidebar_selected: 0x2f3549,
    sidebar_selected_border: 0x414868,
    fg: 0xc0caf5,
    fg_2: 0xa9b1d6,
    fg_3: 0x565f89,
    accent: 0x7aa2f7,
    accent_ink: 0x1a1b26,
    warn: 0xe0af68,
    danger: 0xf7768e,
    info: 0x7dcfff,
};

pub const GRUVBOX_DARK: ThemeColors = ThemeColors {
    bg: 0x282828,
    panel: 0x32302f,
    raised: 0x3c3836,
    line: 0x3c3836,
    line_strong: 0x504945,
    sidebar: 0x3c3836,
    sidebar_selected: 0x504945,
    sidebar_selected_border: 0x665c54,
    fg: 0xebdbb2,
    fg_2: 0xbdae93,
    fg_3: 0x928374,
    accent: 0xfe8019,
    accent_ink: 0x282828,
    warn: 0xfabd2f,
    danger: 0xfb4934,
    info: 0x83a598,
};

pub const SOLARIZED_DARK: ThemeColors = ThemeColors {
    bg: 0x002b36,
    panel: 0x073642,
    raised: 0x0e4552,
    line: 0x0e4552,
    line_strong: 0x165361,
    sidebar: 0x073642,
    sidebar_selected: 0x0e4552,
    sidebar_selected_border: 0x1b5a69,
    fg: 0x93a1a1,
    fg_2: 0x839496,
    fg_3: 0x586e75,
    accent: 0x268bd2,
    accent_ink: 0xffffff,
    warn: 0xb58900,
    danger: 0xdc322f,
    info: 0x2aa198,
};

pub const ROSE_PINE_DAWN: ThemeColors = ThemeColors {
    bg: 0xfaf4ed,
    panel: 0xfffaf3,
    raised: 0xf4ede8,
    line: 0xe8ded7,
    line_strong: 0xcecacd,
    sidebar: 0xf2e9e1,
    sidebar_selected: 0xdfdad9,
    sidebar_selected_border: 0xcecacd,
    fg: 0x575279,
    fg_2: 0x797593,
    fg_3: 0x9893a5,
    accent: 0x286983,
    accent_ink: 0xffffff,
    warn: 0xc47c18,
    danger: 0xb4637a,
    info: 0x3a7a85,
};

pub const TOKYO_NIGHT_DAY: ThemeColors = ThemeColors {
    bg: 0xe1e2e7,
    panel: 0xeaebf0,
    raised: 0xf4f5f8,
    line: 0xc4c8da,
    line_strong: 0xb4b8cc,
    sidebar: 0xd0d5e3,
    sidebar_selected: 0xc4c8da,
    sidebar_selected_border: 0xb2b7cc,
    fg: 0x3760bf,
    fg_2: 0x52629e,
    fg_3: 0x848cb5,
    accent: 0x2e7de9,
    accent_ink: 0xffffff,
    warn: 0x8c6c3e,
    danger: 0xf52a65,
    info: 0x007197,
};

pub const GRUVBOX_LIGHT: ThemeColors = ThemeColors {
    bg: 0xfbf1c7,
    panel: 0xf2e5bc,
    raised: 0xf9f5d7,
    line: 0xd5c4a1,
    line_strong: 0xbdae93,
    sidebar: 0xebdbb2,
    sidebar_selected: 0xd5c4a1,
    sidebar_selected_border: 0xbdae93,
    fg: 0x3c3836,
    fg_2: 0x665c54,
    fg_3: 0x928374,
    accent: 0xaf3a03,
    accent_ink: 0xffffff,
    warn: 0xb57614,
    danger: 0xcc241d,
    info: 0x076678,
};

pub const SOLARIZED_LIGHT: ThemeColors = ThemeColors {
    bg: 0xfdf6e3,
    panel: 0xeee8d5,
    raised: 0xfdf9ee,
    line: 0xe0d8c0,
    line_strong: 0xd3cab1,
    sidebar: 0xeee8d5,
    sidebar_selected: 0xe4dcc4,
    sidebar_selected_border: 0xd3cab1,
    fg: 0x586e75,
    fg_2: 0x657b83,
    fg_3: 0x93a1a1,
    accent: 0x268bd2,
    accent_ink: 0xffffff,
    warn: 0xb58900,
    danger: 0xdc322f,
    info: 0x2aa198,
};

static ACTIVE_VARIANT: AtomicU8 = AtomicU8::new(ThemeVariant::Carbon as u8);

pub fn set_active_variant(variant: ThemeVariant) {
    #[cfg(any(test, debug_assertions))]
    if testing::set_variant_if_scoped(variant) {
        return;
    }
    ACTIVE_VARIANT.store(variant as u8, Ordering::Relaxed);
}

pub fn active_variant() -> ThemeVariant {
    #[cfg(any(test, debug_assertions))]
    if let Some(variant) = testing::variant() {
        return variant;
    }
    let value = ACTIVE_VARIANT.load(Ordering::Relaxed);
    ThemeVariant::ALL
        .into_iter()
        .find(|variant| *variant as u8 == value)
        .unwrap_or_default()
}

pub const fn colors_for(variant: ThemeVariant) -> ThemeColors {
    match variant {
        ThemeVariant::Carbon => CARBON,
        ThemeVariant::CatppuccinMocha => CATPPUCCIN_MOCHA,
        ThemeVariant::RosePineMoon => ROSE_PINE_MOON,
        ThemeVariant::TokyoNight => TOKYO_NIGHT,
        ThemeVariant::GruvboxDark => GRUVBOX_DARK,
        ThemeVariant::SolarizedDark => SOLARIZED_DARK,
        ThemeVariant::RunnerLight => RUNNER_LIGHT,
        ThemeVariant::CatppuccinLatte => CATPPUCCIN_LATTE,
        ThemeVariant::RosePineDawn => ROSE_PINE_DAWN,
        ThemeVariant::TokyoNightDay => TOKYO_NIGHT_DAY,
        ThemeVariant::GruvboxLight => GRUVBOX_LIGHT,
        ThemeVariant::SolarizedLight => SOLARIZED_LIGHT,
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

static GLASS: AtomicBool = AtomicBool::new(false);

pub fn set_glass(glass: bool) {
    #[cfg(any(test, debug_assertions))]
    if testing::set_glass_if_scoped(glass) {
        return;
    }
    GLASS.store(glass, Ordering::Relaxed);
}

pub fn is_glass() -> bool {
    #[cfg(any(test, debug_assertions))]
    if let Some(glass) = testing::glass() {
        return glass;
    }
    GLASS.load(Ordering::Relaxed)
}

pub fn chrome() -> Hsla {
    if is_glass() {
        with_alpha(
            sidebar(),
            if active_variant().is_light() {
                0.90
            } else {
                0.88
            },
        )
    } else {
        sidebar()
    }
}

pub fn chrome_border() -> Hsla {
    if is_glass() {
        with_alpha(
            if active_variant().is_light() {
                text()
            } else {
                gpui::white()
            },
            if active_variant().is_light() {
                0.094
            } else {
                0.071
            },
        )
    } else {
        border()
    }
}

pub fn chrome_selected() -> Hsla {
    if is_glass() {
        with_alpha(
            if active_variant().is_light() {
                text()
            } else {
                gpui::white()
            },
            if active_variant().is_light() {
                0.047
            } else {
                0.071
            },
        )
    } else {
        sidebar_selected()
    }
}

pub fn chrome_selected_border() -> Hsla {
    if is_glass() {
        chrome_border()
    } else {
        sidebar_selected_border()
    }
}

pub fn chrome_hover() -> Hsla {
    let selected = chrome_selected();
    with_alpha(selected, selected.a * 0.4)
}

pub fn danger() -> Hsla {
    color(colors().danger)
}

pub fn warning() -> Hsla {
    color(colors().warn)
}

/// Find-in-terminal highlights are app colours, not terminal palette ones, so
/// they read over any ANSI colour: a tint for every match and a solid fill,
/// outline and text for the current one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FindHighlight {
    pub tint: Hsla,
    pub current: Hsla,
    pub outline: Hsla,
    pub text: Hsla,
}

pub fn find_highlight() -> FindHighlight {
    if active_variant().is_light() {
        FindHighlight {
            tint: with_alpha(color(0xf2b544), 0.35),
            current: color(0xf2b544),
            outline: color(0xc27c0e),
            text: color(0x1c1d22),
        }
    } else {
        FindHighlight {
            tint: with_alpha(color(0xffb020), 0.25),
            current: color(0xffb020),
            outline: color(0xffd27a),
            text: color(0x15161b),
        }
    }
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

#[cfg(any(test, debug_assertions))]
pub mod testing {
    use super::ThemeVariant;
    use std::{cell::Cell, marker::PhantomData, rc::Rc};

    #[derive(Clone, Copy)]
    struct State {
        variant: ThemeVariant,
        glass: bool,
    }

    thread_local! { static OVERRIDE: Cell<Option<State>> = const { Cell::new(None) }; }

    pub(super) fn variant() -> Option<ThemeVariant> {
        OVERRIDE.get().map(|state| state.variant)
    }

    pub(super) fn glass() -> Option<bool> {
        OVERRIDE.get().map(|state| state.glass)
    }

    pub(super) fn set_variant_if_scoped(variant: ThemeVariant) -> bool {
        if let Some(mut state) = OVERRIDE.get() {
            state.variant = variant;
            OVERRIDE.set(Some(state));
            true
        } else {
            false
        }
    }

    pub(super) fn set_glass_if_scoped(glass: bool) -> bool {
        if let Some(mut state) = OVERRIDE.get() {
            state.glass = glass;
            OVERRIDE.set(Some(state));
            true
        } else {
            false
        }
    }

    pub struct ThemeScope {
        previous: Option<State>,
        _thread: PhantomData<Rc<()>>,
    }

    impl Drop for ThemeScope {
        fn drop(&mut self) {
            OVERRIDE.set(self.previous);
        }
    }

    // Explicit opt-in also works for binary tests linking the non-test library.
    pub fn scope() -> ThemeScope {
        let state = State {
            variant: super::active_variant(),
            glass: super::is_glass(),
        };
        ThemeScope {
            previous: OVERRIDE.replace(Some(state)),
            _thread: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_theme_restores_outer_state_after_nested_scope_and_unwind() {
        let outer = testing::scope();
        set_active_variant(ThemeVariant::Carbon);
        set_glass(true);
        {
            let _inner = testing::scope();
            set_active_variant(ThemeVariant::RunnerLight);
            set_glass(false);
            assert_eq!(active_variant(), ThemeVariant::RunnerLight);
            assert!(!is_glass());
        }
        assert_eq!(active_variant(), ThemeVariant::Carbon);
        assert!(is_glass());
        let result = std::panic::catch_unwind(|| {
            let _inner = testing::scope();
            set_active_variant(ThemeVariant::CatppuccinMocha);
            set_glass(false);
            panic!("unwind a theme scope");
        });
        assert!(result.is_err());
        assert_eq!(active_variant(), ThemeVariant::Carbon);
        assert!(is_glass());
        drop(outer);
        assert!(testing::variant().is_none());
        assert!(testing::glass().is_none());
    }

    #[test]
    fn glass_changes_only_chrome() {
        let _guard = crate::theme_snapshot::ThemeGuard::new();
        for variant in ThemeVariant::ALL {
            set_active_variant(variant);
            set_glass(false);
            assert_eq!(chrome(), sidebar());
            assert_eq!(chrome_selected(), sidebar_selected());
            let opaque_bg = bg();
            let opaque_panel = panel();
            let opaque_raised = raised();
            let opaque_selection = sidebar_selected();
            set_glass(true);
            assert_eq!(
                chrome(),
                with_alpha(sidebar(), if variant.is_light() { 0.90 } else { 0.88 })
            );
            assert_eq!(bg(), opaque_bg);
            assert_eq!(panel(), opaque_panel);
            assert_eq!(raised(), opaque_raised);
            assert_eq!(sidebar_selected(), opaque_selection);
            assert_eq!(raised().a, 1.);
            assert_eq!(bg().a, 1.);
            assert_eq!(panel().a, 1.);
        }
    }

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
        assert_eq!(DarkTheme::default(), DarkTheme::Runner);
        assert_eq!(
            serde_json::from_str::<LightTheme>(r#""codex""#).unwrap(),
            LightTheme::RunnerLight
        );
        assert_eq!(
            serde_json::from_str::<DarkTheme>(r#""unknown-theme""#).unwrap(),
            DarkTheme::Runner
        );
        for intent in [ThemeIntent::Auto, ThemeIntent::Light] {
            assert_eq!(
                resolve_variant(intent, true, LightTheme::default(), DarkTheme::default()),
                ThemeVariant::RunnerLight
            );
        }
        for light in LightTheme::ALL {
            assert!(light.variant().is_light(), "{light:?}");
        }
        for dark in DarkTheme::ALL {
            assert!(!dark.variant().is_light(), "{dark:?}");
        }
    }

    #[test]
    fn every_variant_maps_to_matching_lightness_terminal_palette() {
        for variant in ThemeVariant::ALL {
            assert_eq!(
                variant.terminal_palette().is_light(),
                variant.is_light(),
                "{variant:?}"
            );
        }
    }

    #[test]
    fn shipped_theme_notices_credit_non_runner_families() {
        for credit in [
            "Catppuccin",
            "Rosé Pine",
            "Tokyo Night",
            "Apache License",
            "Gruvbox",
            "Solarized",
        ] {
            assert!(SHIPPED_NOTICES.contains(credit), "{credit}");
        }
    }

    #[test]
    fn runner_light_matches_the_signed_off_tokens() {
        assert_eq!(
            RUNNER_LIGHT,
            ThemeColors {
                bg: 0xf2f3f5,
                panel: 0xf8f8fa,
                raised: 0xecedf1,
                line: 0xd9dbe1,
                line_strong: 0xcdd0d7,
                sidebar: 0xe2e4e9,
                sidebar_selected: 0xd6d9e0,
                sidebar_selected_border: 0xcaced7,
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
    }

    #[test]
    fn find_highlights_match_the_signed_off_tokens() {
        let _scope = testing::scope();
        for (variant, tint, current, outline, text) in [
            (
                ThemeVariant::Carbon,
                (0xffb020, 0.25),
                0xffb020,
                0xffd27a,
                0x15161b,
            ),
            (
                ThemeVariant::CatppuccinMocha,
                (0xffb020, 0.25),
                0xffb020,
                0xffd27a,
                0x15161b,
            ),
            (
                ThemeVariant::RunnerLight,
                (0xf2b544, 0.35),
                0xf2b544,
                0xc27c0e,
                0x1c1d22,
            ),
            (
                ThemeVariant::CatppuccinLatte,
                (0xf2b544, 0.35),
                0xf2b544,
                0xc27c0e,
                0x1c1d22,
            ),
        ] {
            set_active_variant(variant);
            assert_eq!(
                find_highlight(),
                FindHighlight {
                    tint: with_alpha(color(tint.0), tint.1),
                    current: color(current),
                    outline: color(outline),
                    text: color(text),
                },
                "{variant:?}"
            );
        }
    }

    #[test]
    fn shipped_roles_match_react_tokens() {
        assert_eq!(CARBON.accent, 0x00ff9c);
        assert_eq!(CATPPUCCIN_MOCHA.sidebar_selected, 0x3b3d52);
        assert_eq!(RUNNER_LIGHT.panel, 0xf8f8fa);
        assert_eq!(CATPPUCCIN_LATTE.sidebar_selected_border, 0xc8ccda);
    }
}
