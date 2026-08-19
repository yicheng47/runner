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

#[derive(IntoElement)]
pub struct SessionOverlay {
    id: SharedString,
    kind: SessionOverlayKind,
    subtitle: Option<SharedString>,
    on_resume: Option<PressHandler>,
    on_archive: Option<PressHandler>,
}

impl SessionOverlay {
    pub fn transition(id: impl Into<SharedString>, kind: SessionOverlayKind) -> Self {
        debug_assert!(kind != SessionOverlayKind::Ended);
        Self {
            id: id.into(),
            kind,
            subtitle: None,
            on_resume: None,
            on_archive: None,
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
            subtitle: Some(subtitle.into()),
            on_resume: Some(Rc::new(on_resume)),
            on_archive: Some(Rc::new(on_archive)),
        }
    }
}

impl RenderOnce for SessionOverlay {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let id = self.id.clone();
        match self.kind {
            SessionOverlayKind::Starting | SessionOverlayKind::Resuming => {
                let label = if self.kind == SessionOverlayKind::Starting {
                    "Starting chat…"
                } else {
                    "Resuming…"
                };
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
                let resume_click = Rc::clone(&resume);
                let archive_click = Rc::clone(&archive);
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_end()
                    .justify_center()
                    .pb(rems(56. / 16.))
                    .px_4()
                    .bg(theme::with_alpha(theme::bg(), 0.7))
                    .child(
                        div()
                            .w_full()
                            .max_w(rems(672. / 16.))
                            .flex()
                            .flex_col()
                            .gap(rems(14. / 16.))
                            .rounded_xl()
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::panel())
                            .p_5()
                            .shadow_lg()
                            .child(
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
                                            .child("Chat paused"),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(rems(13. / 16.))
                                    .line_height(rems(18. / 16.))
                                    .text_color(theme::muted())
                                    .child(self.subtitle.expect("ended overlay subtitle")),
                            )
                            .child(
                                div()
                                    .h(rems(ButtonSize::Md.height(true) / 16.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        Button::new(
                                            SharedString::from(format!("{id}-resume")),
                                            "Resume",
                                        )
                                        .icon("play.svg")
                                        .variant(ButtonVariant::Primary)
                                        .on_press(move |window, cx| resume_click(window, cx)),
                                    )
                                    .child(
                                        Button::new(
                                            SharedString::from(format!("{id}-archive")),
                                            "Archive",
                                        )
                                        .icon("archive.svg")
                                        .on_press(move |window, cx| archive_click(window, cx)),
                                    ),
                            ),
                    )
                    .into_any_element()
            }
        }
    }
}
