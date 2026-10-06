use gpui::prelude::*;
use gpui::{div, rems, AnyElement, FontWeight, IntoElement, RenderOnce, SharedString, Window};

use crate::theme;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Tone {
    Accent,
    #[default]
    Neutral,
    Muted,
    Warning,
    Danger,
    Info,
}

fn tone_color(tone: Tone) -> gpui::Hsla {
    match tone {
        Tone::Accent => theme::accent(),
        Tone::Neutral => theme::text(),
        Tone::Muted => theme::faint(),
        Tone::Warning => theme::warning(),
        Tone::Danger => theme::danger(),
        Tone::Info => theme::info(),
    }
}

pub fn notice_banner(message: impl IntoElement, tone: Tone) -> gpui::Div {
    let color = tone_color(tone);
    let (background, border, text) = if tone == Tone::Danger {
        (
            gpui::rgb(0x22161a).into(),
            gpui::rgb(0x4a2226).into(),
            gpui::rgb(0xd8d8dc).into(),
        )
    } else {
        (
            theme::with_alpha(color, 0.1),
            theme::with_alpha(color, 0.4),
            theme::text(),
        )
    };
    div()
        .debug_selector(|| "NOTICE_BANNER".into())
        .w_full()
        .h(rems(41. / 16.))
        .flex_none()
        .flex()
        .items_center()
        .gap(rems(10. / 16.))
        .pl(rems(1.))
        .pr(rems(12. / 16.))
        .bg(background)
        .border_b_1()
        .border_color(border)
        .child(
            gpui::svg()
                .debug_selector(|| "NOTICE_BANNER_ICON".into())
                .path(if tone == Tone::Warning {
                    "triangle-alert.svg"
                } else {
                    "circle-x.svg"
                })
                .size(rems(14. / 16.))
                .flex_none()
                .text_color(color),
        )
        .child(
            div()
                .debug_selector(|| "NOTICE_BANNER_MESSAGE".into())
                .flex_1()
                .min_w(gpui::px(0.))
                .truncate()
                .text_size(theme::text_body())
                .text_color(text)
                .child(message),
        )
}

#[derive(IntoElement)]
pub struct Card {
    child: AnyElement,
    padded: bool,
}

impl Card {
    pub fn new(child: impl IntoElement) -> Self {
        Self {
            child: child.into_any_element(),
            padded: true,
        }
    }

    pub fn padded(mut self, padded: bool) -> Self {
        self.padded = padded;
        self
    }
}

impl RenderOnce for Card {
    fn render(self, _window: &mut Window, _cx: &mut gpui::App) -> impl IntoElement {
        div()
            .overflow_hidden()
            .rounded(rems(12. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .when(self.padded, |card| card.p_4())
            .child(self.child)
    }
}

#[derive(IntoElement)]
pub struct Badge {
    label: SharedString,
    tone: Tone,
    dot: bool,
}

impl Badge {
    pub fn new(label: impl Into<SharedString>, tone: Tone) -> Self {
        Self {
            label: label.into(),
            tone,
            dot: false,
        }
    }

    pub fn dot(mut self, dot: bool) -> Self {
        self.dot = dot;
        self
    }
}

impl RenderOnce for Badge {
    fn render(self, _window: &mut Window, _cx: &mut gpui::App) -> impl IntoElement {
        let color = tone_color(self.tone);
        div()
            .flex()
            .items_center()
            .gap(rems(6. / 16.))
            .h(rems(18. / 16.))
            .px(rems(8. / 16.))
            .rounded_full()
            .bg(theme::with_alpha(
                color,
                if self.tone == Tone::Neutral {
                    0.05
                } else {
                    0.1
                },
            ))
            .font_weight(FontWeight::MEDIUM)
            .text_size(theme::text_caption())
            .text_color(color)
            .when(self.dot, |badge| {
                badge.child(div().size(rems(6. / 16.)).rounded_full().bg(color))
            })
            .child(self.label)
    }
}

pub fn status_badge(label: impl Into<SharedString>, tone: Tone) -> Badge {
    Badge::new(label, tone).dot(true)
}

#[derive(IntoElement)]
pub struct RuntimeBadge {
    label: SharedString,
    overridden: bool,
    uppercase: bool,
}

impl RuntimeBadge {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            overridden: false,
            uppercase: false,
        }
    }

    pub fn overridden(mut self, overridden: bool) -> Self {
        self.overridden = overridden;
        self
    }

    pub fn uppercase(mut self, uppercase: bool) -> Self {
        self.uppercase = uppercase;
        self
    }
}

impl RenderOnce for RuntimeBadge {
    fn render(self, _window: &mut Window, _cx: &mut gpui::App) -> impl IntoElement {
        let label: SharedString = if self.uppercase {
            self.label.to_uppercase().into()
        } else {
            self.label
        };
        div()
            .flex()
            .items_center()
            .h(rems(19. / 16.))
            .px(rems(6. / 16.))
            .rounded(rems(4. / 16.))
            .bg(if self.overridden {
                theme::with_alpha(theme::accent(), 0.1)
            } else {
                theme::raised()
            })
            .text_size(theme::text_caption())
            .line_height(rems(15. / 16.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(if self.overridden {
                theme::accent()
            } else {
                theme::muted()
            })
            .child(label)
    }
}

pub fn pill(label: impl Into<SharedString>, tone: Tone) -> AnyElement {
    let color = tone_color(tone);
    div()
        .flex()
        .items_center()
        .h(rems(24. / 16.))
        .px(rems(10. / 16.))
        .rounded(rems(6. / 16.))
        .border_1()
        .border_color(theme::with_alpha(color, 0.4))
        .bg(theme::with_alpha(color, 0.1))
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(theme::text_meta())
        .text_color(color)
        .child(label.into())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme_snapshot::ThemeGuard;
    use crate::ui::{Button, ButtonSize, ButtonVariant};
    use gpui::{px, size, Context, Render, TestAppContext, VisualTestContext};

    struct BannerHost(Tone);
    impl Render for BannerHost {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                notice_banner("An intentionally long notice must stay on one line while its action remains visible in a narrow window.", self.0)
                    .child(
                        div().debug_selector(|| "NOTICE_ACTION".into()).flex_none().child(
                            Button::new("notice-action", "Dismiss")
                                .size(ButtonSize::Sm)
                                .variant(ButtonVariant::Ghost),
                        ),
                    ),
            )
        }
    }

    #[test]
    fn warning_and_error_banners_keep_one_line_and_visible_actions_at_every_zoom() {
        let _theme = ThemeGuard::new();
        for variant in [
            theme::ThemeVariant::Carbon,
            theme::ThemeVariant::RunnerLight,
            theme::ThemeVariant::CatppuccinMocha,
            theme::ThemeVariant::CatppuccinLatte,
        ] {
            theme::set_active_variant(variant);
            for tone in [Tone::Warning, Tone::Danger] {
                for zoom in [0.8, 1., 1.5] {
                    let mut cx = TestAppContext::single();
                    let window = cx.add_window(|window, _| {
                        window.set_rem_size(px(16. * zoom));
                        BannerHost(tone)
                    });
                    let mut visual = VisualTestContext::from_window(window.into(), &cx);
                    visual.simulate_resize(size(px(320.), px(150.)));
                    cx.run_until_parked();
                    let banner = visual.debug_bounds("NOTICE_BANNER").unwrap();
                    let message = visual.debug_bounds("NOTICE_BANNER_MESSAGE").unwrap();
                    let icon = visual.debug_bounds("NOTICE_BANNER_ICON").unwrap();
                    let action = visual.debug_bounds("NOTICE_ACTION").unwrap();
                    assert_eq!(banner.size.width, px(320.));
                    assert!((f32::from(banner.size.height) - 41. * zoom).abs() <= 1.);
                    assert!((f32::from(icon.size.height) - 14. * zoom).abs() <= 1.);
                    assert!(message.size.height <= px(22. * zoom));
                    assert!(icon.right() <= message.left());
                    assert!(message.right() <= action.left());
                    assert!(action.right() <= banner.right());
                }
            }
        }
    }
}
