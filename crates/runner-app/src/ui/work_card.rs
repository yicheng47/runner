use gpui::prelude::*;
use gpui::{div, px, rems, Div};

use crate::theme;

/// The gap between an expanded sidebar and the island area, before zoom.
pub const SIDEBAR_GAP: f32 = 6.;

pub fn work_card(collapsed_sidebar: bool) -> Div {
    div()
        .debug_selector(|| "WORK_CARD".into())
        .relative()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .my(rems(8. / 16.))
        .when(cfg!(any(windows, target_os = "linux")), |card| card.mt_0())
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

pub fn island_area(collapsed_sidebar: bool) -> Div {
    div()
        .debug_selector(|| "ISLAND_AREA".into())
        .relative()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .my(rems(8. / 16.))
        .when(cfg!(any(windows, target_os = "linux")), |area| area.mt_0())
        .mr(rems(8. / 16.))
        .ml(rems(if collapsed_sidebar {
            8. / 16.
        } else {
            SIDEBAR_GAP / 16.
        }))
}
