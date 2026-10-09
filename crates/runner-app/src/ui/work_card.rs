use gpui::prelude::*;
use gpui::{div, px, rems, Div};

use crate::theme;

pub fn work_card(collapsed_sidebar: bool) -> Div {
    div()
        .debug_selector(|| "WORK_CARD".into())
        .relative()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .my(rems(8. / 16.))
        .mr(rems(8. / 16.))
        .when(collapsed_sidebar, |card| card.ml(rems(8. / 16.)))
        .flex()
        .flex_col()
        .rounded(rems(12. / 16.))
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg())
        .overflow_hidden()
}
