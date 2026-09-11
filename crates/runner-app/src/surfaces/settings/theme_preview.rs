//! The Appearance pane's preview: one pane per mode, each drawn from the
//! colours and terminal palette it is handed, never from the active theme,
//! so the light pane stays light while the app is dark and vice versa.

use gpui::prelude::*;
use gpui::{div, px, rems, AnyElement, Div, FontWeight, Hsla, SharedString};
use runner_terminal::palette::TerminalPalette;

use crate::terminal::element::to_hsla;
use crate::theme::{self, ThemeColors};

/// The caption of whichever pane's mode is currently resolved.
pub(crate) const ACTIVE_CAPTION: &str = "SETTINGS_THEME_PREVIEW_ACTIVE";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PreviewMode {
    Light,
    Dark,
}

impl PreviewMode {
    pub(crate) fn pane_selector(self) -> &'static str {
        match self {
            Self::Light => "SETTINGS_THEME_PREVIEW_LIGHT",
            Self::Dark => "SETTINGS_THEME_PREVIEW_DARK",
        }
    }

    pub(crate) fn terminal_selector(self) -> &'static str {
        match self {
            Self::Light => "SETTINGS_THEME_PREVIEW_LIGHT_TERMINAL",
            Self::Dark => "SETTINGS_THEME_PREVIEW_DARK_TERMINAL",
        }
    }
}

pub(crate) struct PreviewPane {
    pub mode: PreviewMode,
    pub colors: ThemeColors,
    pub palette: TerminalPalette,
    pub caption: SharedString,
    pub active: bool,
}

pub(crate) fn theme_preview(light: PreviewPane, dark: PreviewPane) -> AnyElement {
    div()
        .flex()
        .gap_4()
        .child(preview_pane(light))
        .child(preview_pane(dark))
        .into_any_element()
}

fn preview_pane(pane: PreviewPane) -> AnyElement {
    let PreviewPane {
        mode,
        colors,
        palette,
        caption,
        active,
    } = pane;
    let caption: SharedString = if active {
        format!("{caption} · active").into()
    } else {
        caption
    };
    div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap(rems(8. / 16.))
        .child(
            div()
                .h(rems(172. / 16.))
                .flex()
                .rounded(rems(12. / 16.))
                .border_1()
                .border_color(color(colors.line))
                .when(active, |pane| {
                    pane.border_2().border_color(color(colors.accent))
                })
                .overflow_hidden()
                .bg(color(colors.bg))
                .debug_selector(move || mode.pane_selector().into())
                .map(|pane| {
                    #[cfg(test)]
                    let pane = crate::theme_snapshot::record_fill(mode.pane_selector(), pane);
                    pane
                })
                .child(sidebar_strip(&colors))
                .child(main_area(mode, &colors, &palette)),
        )
        .child(
            div()
                .text_size(theme::text_ui())
                .text_color(theme::muted())
                .when(active, |caption| {
                    caption.debug_selector(|| ACTIVE_CAPTION.into())
                })
                .child(caption),
        )
        .into_any_element()
}

fn sidebar_strip(colors: &ThemeColors) -> Div {
    div()
        .w(rems(92. / 16.))
        .h_full()
        .flex_none()
        .flex()
        .flex_col()
        .gap(rems(8. / 16.))
        .p(rems(10. / 16.))
        .bg(color(colors.sidebar))
        .child(bar(48., 8., colors.fg_2))
        .child(
            div()
                .w_full()
                .h(rems(18. / 16.))
                .rounded(rems(4. / 16.))
                .bg(color(colors.sidebar_selected)),
        )
        .child(bar(56., 6., colors.fg_3))
        .child(bar(44., 6., colors.fg_3))
        .child(bar(50., 6., colors.fg_3))
}

fn main_area(mode: PreviewMode, colors: &ThemeColors, palette: &TerminalPalette) -> Div {
    let lines = [
        ("❯ cargo test -p runner-backend", palette.foreground),
        ("   Compiling runner-backend v0.8.6", palette.ansi[8]),
        ("test result: ok. 587 passed; 0 failed", palette.ansi[2]),
        ("warning: unused variable `slot`", palette.ansi[3]),
        ("error[E0308]: mismatched types", palette.ansi[1]),
        ("~/repos | Fable 5 · xhigh | US SJC", palette.ansi[4]),
    ];
    div()
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .flex()
        .flex_col()
        .gap(rems(8. / 16.))
        .p(rems(10. / 16.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(bar(72., 8., colors.fg))
                .child(
                    div()
                        .w(rems(28. / 16.))
                        .h(rems(10. / 16.))
                        .rounded_full()
                        .bg(color(colors.accent)),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .rounded(rems(8. / 16.))
                .overflow_hidden()
                .bg(to_hsla(palette.background, 1.))
                .debug_selector(move || mode.terminal_selector().into())
                .map(|terminal| {
                    #[cfg(test)]
                    let terminal =
                        crate::theme_snapshot::record_fill(mode.terminal_selector(), terminal);
                    terminal
                })
                .p(rems(8. / 16.))
                .flex()
                .flex_col()
                .font_family(theme::SYSTEM_MONOSPACE_FONT)
                .text_size(theme::text_meta())
                .font_weight(FontWeight::NORMAL)
                .line_height(rems(15. / 16.))
                .children(lines.into_iter().map(|(text, rgb)| {
                    div()
                        .whitespace_nowrap()
                        .text_color(to_hsla(rgb, 1.))
                        .child(text)
                })),
        )
}

fn bar(width: f32, height: f32, fill: u32) -> Div {
    div()
        .w(rems(width / 16.))
        .h(rems(height / 16.))
        .flex_none()
        .rounded_full()
        .bg(color(fill))
}

fn color(value: u32) -> Hsla {
    gpui::rgb(value).into()
}
