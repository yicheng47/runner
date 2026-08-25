use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, rems, svg, Animation, AnimationExt as _, App, FontWeight, RenderOnce, SharedString, Window,
};

use crate::theme;
use crate::ui::button::{spinner, Button, ButtonSize, ButtonVariant, PressHandler};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionOverlayKind {
    Starting,
    Resuming,
    Archiving,
    Ended,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EndedStyle {
    Chat,
    Shell,
}

#[derive(IntoElement)]
pub struct SessionOverlay {
    id: SharedString,
    kind: SessionOverlayKind,
    label: Option<SharedString>,
    title: Option<SharedString>,
    subtitle: Option<SharedString>,
    on_resume: Option<PressHandler>,
    on_archive: Option<PressHandler>,
    ended_style: EndedStyle,
}

impl SessionOverlay {
    pub fn transition(id: impl Into<SharedString>, kind: SessionOverlayKind) -> Self {
        debug_assert!(kind != SessionOverlayKind::Ended);
        Self {
            id: id.into(),
            kind,
            label: None,
            title: None,
            subtitle: None,
            on_resume: None,
            on_archive: None,
            ended_style: EndedStyle::Chat,
        }
    }

    pub fn ended(
        id: impl Into<SharedString>,
        subtitle: impl Into<SharedString>,
        on_resume: impl Fn(&mut Window, &mut App) + 'static,
        on_archive: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            kind: SessionOverlayKind::Ended,
            label: None,
            title: None,
            subtitle: Some(subtitle.into()),
            on_resume: Some(Rc::new(on_resume)),
            on_archive: Some(Rc::new(on_archive)),
            ended_style: EndedStyle::Chat,
        }
    }

    pub fn shell_exited(
        id: impl Into<SharedString>,
        subtitle: impl Into<SharedString>,
        on_restart: impl Fn(&mut Window, &mut App) + 'static,
        on_close: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            kind: SessionOverlayKind::Ended,
            label: None,
            title: Some("Shell exited".into()),
            subtitle: Some(subtitle.into()),
            on_resume: Some(Rc::new(on_restart)),
            on_archive: Some(Rc::new(on_close)),
            ended_style: EndedStyle::Shell,
        }
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }
}

impl RenderOnce for SessionOverlay {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let id = self.id.clone();
        match self.kind {
            SessionOverlayKind::Starting | SessionOverlayKind::Resuming => {
                let label = self.label.unwrap_or_else(|| {
                    if self.kind == SessionOverlayKind::Starting {
                        "Starting chat…".into()
                    } else {
                        "Resuming…".into()
                    }
                });
                div()
                    .absolute()
                    .inset_4()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(10. / 16.))
                            .rounded_full()
                            .border_1()
                            .border_color(theme::with_alpha(theme::info(), 0.4))
                            .bg(theme::with_alpha(theme::info(), 0.1))
                            .px_4()
                            .py_2()
                            .text_size(rems(13. / 16.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::info())
                            .shadow_lg()
                            .child(spinner(
                                SharedString::from(format!("{id}-spinner")),
                                16.,
                                theme::info(),
                            ))
                            .child(label),
                    )
                    .into_any_element()
            }
            SessionOverlayKind::Archiving => div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::with_alpha(theme::bg(), 0.95))
                .child(
                    div()
                        .h(rems(30. / 16.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_full()
                        .border_1()
                        .border_color(theme::with_alpha(theme::warning(), 0.4))
                        .bg(theme::with_alpha(theme::warning(), 0.15))
                        .px_3()
                        .font_family("Menlo")
                        .text_size(rems(13. / 16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::warning())
                        .child(
                            div()
                                .size(rems(8. / 16.))
                                .rounded_full()
                                .bg(theme::warning())
                                .with_animation(
                                    SharedString::from(format!("{id}-pulse")),
                                    Animation::new(Duration::from_millis(800)).repeat(),
                                    |dot, delta| {
                                        let opacity = if delta <= 0.5 {
                                            0.4 + delta * 1.2
                                        } else {
                                            1.6 - delta * 1.2
                                        };
                                        dot.opacity(opacity)
                                    },
                                ),
                        )
                        .child("Archiving…"),
                )
                .into_any_element(),
            SessionOverlayKind::Ended => {
                let resume = self.on_resume.expect("ended overlay resume action");
                let archive = self.on_archive.expect("ended overlay archive action");
                let title = self.title.unwrap_or_else(|| "Chat paused".into());
                let subtitle = self.subtitle.expect("ended overlay subtitle");
                let shell = self.ended_style == EndedStyle::Shell;
                let resume_click = Rc::clone(&resume);
                let archive_click = Rc::clone(&archive);
                let header = if shell {
                    div()
                        .text_size(rems(13. / 16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text())
                        .child(title)
                        .into_any_element()
                } else {
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(10. / 16.))
                        .child(
                            svg()
                                .path("pause.svg")
                                .size(rems(1.))
                                .text_color(theme::faint()),
                        )
                        .child(
                            div()
                                .text_size(rems(15. / 16.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::text())
                                .child(title),
                        )
                        .into_any_element()
                };
                let actions = if shell {
                    div()
                        .debug_selector(|| "SESSION_ENDED_ACTIONS".into())
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new(SharedString::from(format!("{id}-restart")), "Restart")
                                .size(ButtonSize::Sm)
                                .variant(ButtonVariant::Primary)
                                .on_press(move |window, cx| resume_click(window, cx)),
                        )
                        .child(
                            Button::new(SharedString::from(format!("{id}-close")), "Close")
                                .size(ButtonSize::Sm)
                                .on_press(move |window, cx| archive_click(window, cx)),
                        )
                        .into_any_element()
                } else {
                    div()
                        .debug_selector(|| "SESSION_ENDED_ACTIONS".into())
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new(SharedString::from(format!("{id}-resume")), "Resume")
                                .icon("play.svg")
                                .variant(ButtonVariant::Primary)
                                .on_press(move |window, cx| resume_click(window, cx)),
                        )
                        .child(
                            Button::new(SharedString::from(format!("{id}-archive")), "Archive")
                                .icon("archive.svg")
                                .on_press(move |window, cx| archive_click(window, cx)),
                        )
                        .into_any_element()
                };
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .justify_center()
                    .px_4()
                    .bg(theme::with_alpha(theme::bg(), 0.7))
                    .when(shell, |overlay| overlay.items_center())
                    .when(!shell, |overlay| overlay.items_end().pb(rems(56. / 16.)))
                    .child(
                        div()
                            .debug_selector(|| "SESSION_ENDED_CARD".into())
                            .w_full()
                            .flex()
                            .flex_col()
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::panel())
                            .when(shell, |card| {
                                card.max_w(rems(380. / 16.))
                                    .items_center()
                                    .gap(rems(10. / 16.))
                                    .rounded(rems(4. / 16.))
                                    .py_5()
                                    .px(rems(24. / 16.))
                            })
                            .when(!shell, |card| {
                                card.max_w(rems(672. / 16.))
                                    .gap(rems(14. / 16.))
                                    .rounded_xl()
                                    .p_5()
                                    .shadow_lg()
                            })
                            .child(header)
                            .child(
                                div()
                                    .debug_selector(|| "SESSION_ENDED_SUBTITLE".into())
                                    .w_full()
                                    .min_w_0()
                                    .whitespace_normal()
                                    .text_size(rems(if shell { 12. / 16. } else { 13. / 16. }))
                                    .line_height(rems(if shell { 17.4 / 16. } else { 18. / 16. }))
                                    .text_color(theme::muted())
                                    .when(shell, |subtitle| subtitle.text_center())
                                    .child(subtitle),
                            )
                            .child(actions),
                    )
                    .into_any_element()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{px, Context, Render, TestAppContext, VisualTestContext};

    struct EndedOverlayTest;

    struct ShellExitedOverlayTest;

    impl Render for EndedOverlayTest {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(SessionOverlay::ended(
                "test-ended",
                "The PTY is closed. Resume to start a fresh agent process — there's no saved conversation to pick up from this row.",
                |_, _| {},
                |_, _| {},
            ))
        }
    }

    impl Render for ShellExitedOverlayTest {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(SessionOverlay::shell_exited(
                "test-shell-exited",
                "exit 0 · zsh · ~/workspace/oec",
                |_, _| {},
                |_, _| {},
            ))
        }
    }

    #[test]
    fn wrapped_ended_overlay_preserves_bottom_padding() {
        let mut cx = TestAppContext::single();
        let window = cx.add_window(|window, _| {
            window.set_rem_size(px(20.8));
            EndedOverlayTest
        });
        cx.run_until_parked();
        let mut window = VisualTestContext::from_window(window.into(), &cx);
        let card = window
            .debug_bounds("SESSION_ENDED_CARD")
            .expect("card bounds");
        let actions = window
            .debug_bounds("SESSION_ENDED_ACTIONS")
            .expect("actions bounds");
        assert_eq!(card.bottom() - actions.bottom(), px(27.));
    }

    #[test]
    fn ended_overlay_subtitle_wraps_inside_the_card() {
        let mut cx = TestAppContext::single();
        let window = cx.add_window(|window, _| {
            window.set_rem_size(px(20.8));
            EndedOverlayTest
        });
        cx.run_until_parked();
        let mut window = VisualTestContext::from_window(window.into(), &cx);
        let card = window
            .debug_bounds("SESSION_ENDED_CARD")
            .expect("card bounds");
        let subtitle = window
            .debug_bounds("SESSION_ENDED_SUBTITLE")
            .expect("subtitle bounds");
        let line_height = px(20.8 * 18. / 16.);
        assert!(
            subtitle.size.height >= line_height * 2.,
            "subtitle did not wrap: {subtitle:?}"
        );
        assert!(subtitle.right() <= card.right(), "{subtitle:?} vs {card:?}");
    }

    #[test]
    fn shell_exited_overlay_uses_the_compact_design_card() {
        let mut cx = TestAppContext::single();
        let window = cx.add_window(|window, _| {
            window.set_rem_size(px(16.));
            ShellExitedOverlayTest
        });
        cx.run_until_parked();
        let mut window = VisualTestContext::from_window(window.into(), &cx);
        let card = window
            .debug_bounds("SESSION_ENDED_CARD")
            .expect("card bounds");
        let actions = window
            .debug_bounds("SESSION_ENDED_ACTIONS")
            .expect("actions bounds");
        assert_eq!(card.size.width, px(380.));
        assert!(card.top() < actions.top());
        assert!(actions.bottom() < card.bottom());
    }
}
