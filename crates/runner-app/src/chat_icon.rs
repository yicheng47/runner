use gpui::Hsla;
use runner_backend::model::Runtime;

use crate::theme;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ChatIcon {
    pub path: &'static str,
    tint: Option<Hsla>,
}

impl ChatIcon {
    pub fn generic(path: &'static str) -> Self {
        Self { path, tint: None }
    }

    pub fn mission() -> Self {
        Self {
            path: "flag.svg",
            tint: Some(theme::accent()),
        }
    }

    pub fn for_runtime(runtime: &str) -> Self {
        let (path, tint) = match Runtime::parse(runtime) {
            Some(Runtime::ClaudeCode) => ("claude.svg", gpui::rgb(0xd97757).into()),
            Some(Runtime::Codex) => ("openai.svg", theme::text()),
            Some(Runtime::Trae) => ("trae.svg", gpui::rgb(0x32f08c).into()),
            Some(Runtime::Copilot) => ("copilot.svg", gpui::rgb(0x8534f3).into()),
            Some(Runtime::Pi) => ("pi.svg", theme::text()),
            Some(Runtime::Shell) => return Self::generic("square-terminal.svg"),
            None => return Self::generic("message-square.svg"),
        };
        Self {
            path,
            tint: Some(tint),
        }
    }

    pub fn color(self, fallback: Hsla, live: bool) -> Hsla {
        match self.tint {
            None => fallback,
            Some(tint) if live => tint,
            Some(_) => theme::with_alpha(theme::text(), 0.45),
        }
    }
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

            let icon = ChatIcon::mission();
            assert_eq!(icon.path, "flag.svg");
            for fallback in [theme::text(), theme::muted(), theme::faint()] {
                assert_eq!(icon.color(fallback, true), theme::accent());
                assert_eq!(
                    icon.color(fallback, false),
                    theme::with_alpha(theme::text(), 0.45)
                );
            }
        }
    }

    #[test]
    fn shell_and_unknown_runtimes_preserve_the_surface_color() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for (runtime, path) in [
            ("shell", "square-terminal.svg"),
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
