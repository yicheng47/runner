use gpui::prelude::*;
use gpui::{div, img, rems, svg, AnyElement, DefiniteLength, Hsla};
use runner_core::protocol::model::Runtime;

use crate::assets::antigravity_icon_source;
use crate::theme;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ChatIcon {
    path: &'static str,
    tint: Option<Hsla>,
}

impl ChatIcon {
    #[cfg(test)]
    pub fn asset_path(self) -> &'static str {
        self.path
    }

    pub fn generic(path: &'static str) -> Self {
        Self { path, tint: None }
    }

    /// The app's own marks — mission, split, terminal — share the accent and
    /// so follow the same live-or-dim rule as a provider's mark.
    fn accented(path: &'static str) -> Self {
        Self {
            path,
            tint: Some(theme::accent()),
        }
    }

    pub fn mission() -> Self {
        Self::accented("flag.svg")
    }

    pub fn split() -> Self {
        Self::accented("columns-2.svg")
    }

    pub fn for_runtime(runtime: &str) -> Self {
        let Some(runtime) = Runtime::parse(runtime) else {
            return Self::generic("message-square.svg");
        };
        let ui = crate::runtime_ui::runtime_ui(runtime);
        Self {
            path: ui.icon,
            tint: Some(ui.tint),
        }
    }

    pub fn color(self, fallback: Hsla, live: bool) -> Hsla {
        match self.tint {
            None => fallback,
            Some(tint) if live => tint,
            Some(_) => theme::with_alpha(theme::text(), 0.45),
        }
    }

    pub fn render(self, size: impl Into<DefiniteLength>, color: Hsla, live: bool) -> AnyElement {
        let size = size.into();
        if self.path == "antigravity-icon.png" {
            img(antigravity_icon_source())
                .size(size)
                .flex_none()
                .opacity(if live { 1. } else { 0.45 })
                .into_any_element()
        } else {
            svg()
                .path(self.path)
                .size(size)
                .flex_none()
                .text_color(color)
                .into_any_element()
        }
    }
}

/// A provider's mark: bare at text size, on a raised tile from a card row up.
pub(crate) fn runtime_mark(runtime: &str, size: f32) -> AnyElement {
    let icon = ChatIcon::for_runtime(runtime);
    let color = icon.color(theme::muted(), true);
    if size < 24. {
        return icon.render(rems(size / 16.), color, true);
    }
    div()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .size(rems(size / 16.))
        .rounded(rems((size * 0.2).round() / 16.))
        .bg(theme::raised())
        .child(icon.render(rems(size / 2. / 16.), color, true))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_and_mission_marks_keep_their_tint_only_while_live() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for variant in [
            theme::ThemeVariant::Carbon,
            theme::ThemeVariant::RunnerLight,
        ] {
            theme::set_active_variant(variant);
            for (runtime, path, tint) in [
                ("claude-code", "claude.svg", gpui::rgb(0xd97757).into()),
                ("codex", "openai.svg", theme::text()),
                ("trae", "trae.svg", gpui::rgb(0x32f08c).into()),
                ("copilot", "copilot.svg", gpui::rgb(0x8534f3).into()),
                ("pi", "pi.svg", theme::text()),
                ("antigravity", "antigravity-icon.png", theme::text()),
            ] {
                let icon = ChatIcon::for_runtime(runtime);
                assert_eq!(icon.path, path);
                for fallback in [theme::accent(), theme::muted(), theme::faint()] {
                    assert_eq!(icon.color(fallback, true), tint);
                    assert_eq!(
                        icon.color(fallback, false),
                        theme::with_alpha(theme::text(), 0.45)
                    );
                }
            }

            for (icon, path) in [
                (ChatIcon::mission(), "flag.svg"),
                (ChatIcon::split(), "columns-2.svg"),
                (ChatIcon::for_runtime("shell"), "square-terminal.svg"),
            ] {
                assert_eq!(icon.path, path);
                for fallback in [theme::text(), theme::muted(), theme::faint()] {
                    assert_eq!(icon.color(fallback, true), theme::accent());
                    assert_eq!(
                        icon.color(fallback, false),
                        theme::with_alpha(theme::text(), 0.45)
                    );
                }
            }
        }
    }

    #[test]
    fn unknown_runtimes_preserve_the_surface_color() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for (runtime, path) in [
            ("unknown", "message-square.svg"),
            ("", "message-square.svg"),
        ] {
            let icon = ChatIcon::for_runtime(runtime);
            assert_eq!(icon.path, path);
            assert_eq!(icon.color(theme::accent(), true), theme::accent());
            assert_eq!(icon.color(theme::muted(), false), theme::muted());
        }
    }
}
