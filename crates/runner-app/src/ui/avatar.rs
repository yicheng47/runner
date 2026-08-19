use gpui::prelude::*;
use gpui::{
    div, rems, rgb, AnyElement, App, FontWeight, Hsla, IntoElement, RenderOnce, SharedString,
    Window,
};

use crate::theme;

const HUMAN_SOURCE_CELLS: [bool; 15] = [
    true, false, true, false, true, true, true, true, false, false, true, true, true, false, true,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunnerPresence {
    Busy,
    Idle,
    Stopped,
    Crashed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AvatarHue {
    Fixed(u32),
    Accent,
    Warning,
}

impl AvatarHue {
    pub fn color(self) -> Hsla {
        match self {
            Self::Fixed(value) => rgb(value).into(),
            Self::Accent => theme::accent(),
            Self::Warning => theme::warning(),
        }
    }
}

pub fn hue_for_seed(seed: &str) -> AvatarHue {
    if seed == "human" {
        return AvatarHue::Warning;
    }
    const HUES: [AvatarHue; 9] = [
        AvatarHue::Fixed(0xff8a4c),
        AvatarHue::Fixed(0xc792ea),
        AvatarHue::Fixed(0xf07178),
        AvatarHue::Accent,
        AvatarHue::Fixed(0x62d9f5),
        AvatarHue::Fixed(0xa3e635),
        AvatarHue::Fixed(0x7aa2f7),
        AvatarHue::Fixed(0xff6b9d),
        AvatarHue::Fixed(0xc3e88d),
    ];
    HUES[((fnv1a(seed) >> 16) as usize) % HUES.len()]
}

pub fn cells_for_seed(seed: &str) -> [bool; 25] {
    let hash = fnv1a(seed);
    let mut source = if seed == "human" {
        HUMAN_SOURCE_CELLS
    } else {
        std::array::from_fn(|bit| hash & (1 << bit) != 0)
    };
    if !source.iter().any(|painted| *painted) {
        source[8] = true;
    }
    std::array::from_fn(|index| {
        let row = index / 5;
        let column = index % 5;
        let source_column = if column < 3 { column } else { 4 - column };
        source[row * 3 + source_column]
    })
}

fn fnv1a(seed: &str) -> u32 {
    seed.as_bytes().iter().fold(0x811c9dc5, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x01000193)
    })
}

#[derive(IntoElement)]
pub struct RunnerAvatar {
    seed: SharedString,
    size: f32,
    presence: Option<RunnerPresence>,
}

impl RunnerAvatar {
    pub fn new(seed: impl Into<SharedString>, size: f32) -> Self {
        Self {
            seed: seed.into(),
            size,
            presence: None,
        }
    }

    pub fn presence(mut self, presence: RunnerPresence) -> Self {
        self.presence = Some(presence);
        self
    }
}

impl RenderOnce for RunnerAvatar {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let cells = cells_for_seed(&self.seed);
        let hue = hue_for_seed(&self.seed).color();
        let cell = self.size / 5.;
        let presence = self.presence;
        div()
            .relative()
            .flex_none()
            .size(rems(self.size / 16.))
            .child(
                div()
                    .relative()
                    .size_full()
                    .rounded(rems(4_f32.max((self.size * 0.23).round()) / 16.))
                    .overflow_hidden()
                    .children((0..5).map(|row| {
                        div()
                            .absolute()
                            .top(rems(row as f32 * cell / 16.))
                            .left_0()
                            .flex()
                            .h(rems(cell / 16.))
                            .children((0..5).map(move |column| {
                                div().size(rems(cell / 16.)).bg(if cells[row * 5 + column] {
                                    hue
                                } else {
                                    theme::raised()
                                })
                            }))
                    })),
            )
            .children(presence.map(|presence| {
                div()
                    .absolute()
                    .right(rems(-1. / 16.))
                    .bottom(rems(-1. / 16.))
                    .size(rems(9. / 16.))
                    .rounded_full()
                    .border_2()
                    .border_color(theme::bg())
                    .bg(match presence {
                        RunnerPresence::Busy => theme::accent(),
                        RunnerPresence::Idle => theme::with_alpha(theme::accent(), 0.4),
                        RunnerPresence::Stopped => theme::faint(),
                        RunnerPresence::Crashed => theme::danger(),
                    })
            }))
    }
}

pub fn lead_badge() -> AnyElement {
    div()
        .rounded(rems(4. / 16.))
        .bg(theme::with_alpha(theme::warning(), 0.2))
        .px(rems(6. / 16.))
        .py(rems(1. / 16.))
        .font_weight(FontWeight::BOLD)
        .text_size(rems(9. / 16.))
        .text_color(theme::warning())
        .child("LEAD")
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hue_derivation_matches_the_shipped_widget() {
        assert_eq!(hue_for_seed("coder"), AvatarHue::Accent);
        assert_eq!(hue_for_seed("reviewer"), AvatarHue::Fixed(0xc792ea));
        assert_eq!(hue_for_seed("human"), AvatarHue::Warning);
    }

    #[test]
    fn pixel_avatar_is_mirrored() {
        let cells = cells_for_seed("coder");
        for row in 0..5 {
            assert_eq!(cells[row * 5], cells[row * 5 + 4]);
            assert_eq!(cells[row * 5 + 1], cells[row * 5 + 3]);
        }
    }
}
