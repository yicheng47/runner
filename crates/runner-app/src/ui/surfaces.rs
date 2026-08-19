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
            .text_size(rems(10. / 16.))
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
        .text_size(rems(11. / 16.))
        .text_color(color)
        .child(label.into())
        .into_any_element()
}
