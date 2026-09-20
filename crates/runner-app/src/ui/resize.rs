use gpui::prelude::*;
use gpui::{div, px, CursorStyle, Div, Hsla};

use crate::theme;

/// How wide a splitter is to the pointer, and how wide the bar it lights.
pub const RESIZE_GRAB: f32 = 6.;
pub const RESIZE_BAR: f32 = 3.;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ResizeAxis {
    /// A vertical divider between columns, dragged left and right.
    Columns,
    /// A horizontal divider between rows, dragged up and down.
    Rows,
}

impl ResizeAxis {
    fn cursor(self) -> CursorStyle {
        match self {
            Self::Columns => CursorStyle::ResizeLeftRight,
            Self::Rows => CursorStyle::ResizeUpDown,
        }
    }
}

/// Every splitter in the app looks and behaves the same: a grab strip centred
/// on the divider, carrying a bar that turns accent while the splitter is
/// hovered or dragged. A drag suppresses hover, so `active` is what keeps the
/// bar lit once the pointer is moving.
///
/// `rest` is the divider's own colour for splitters that draw their own line.
/// That line never changes; the accent bar lights over it, exactly as it does
/// over the border a neighbouring panel paints for the splitters that pass
/// `None`.
pub fn resize_strip(
    group: &'static str,
    axis: ResizeAxis,
    active: bool,
    zoom: f32,
    rest: Option<Hsla>,
) -> Div {
    let lit = theme::with_alpha(theme::accent(), 0.4);
    let grab = RESIZE_GRAB * zoom;
    let bar_size = px(RESIZE_BAR * zoom);
    let line_size = px(zoom);
    let line_inset = px((grab - zoom) / 2.);
    let bar = match axis {
        ResizeAxis::Columns => div().w(bar_size).h_full(),
        ResizeAxis::Rows => div().h(bar_size).w_full(),
    };
    let bar = bar
        .when(active, |bar| bar.bg(lit))
        .group_hover(group, move |bar| bar.bg(lit))
        .debug_selector(move || format!("{group}-bar"));
    let line = rest.map(|color| {
        let line = match axis {
            ResizeAxis::Columns => div().w(line_size).h_full().left(line_inset).top_0(),
            ResizeAxis::Rows => div().h(line_size).w_full().top(line_inset).left_0(),
        };
        line.absolute()
            .bg(color)
            .debug_selector(move || format!("{group}-line"))
    });
    let strip = match axis {
        ResizeAxis::Columns => div().w(px(grab)).h_full(),
        ResizeAxis::Rows => div().h(px(grab)).w_full(),
    };
    strip
        .group(group)
        .relative()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .cursor(axis.cursor())
        .children(line)
        .child(bar)
}

/// The offset from a divider's centre to the leading edge of its grab strip.
pub fn resize_strip_inset(zoom: f32) -> f32 {
    RESIZE_GRAB * zoom / 2.
}
