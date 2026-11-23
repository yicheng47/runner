use gpui::prelude::*;
use gpui::{div, px, rems, Div};

use crate::theme;

/// The gap between islands, before zoom: an expanded sidebar and the island area, split
/// panes, and a chat or mission and its docked side panel.
pub const ISLAND_GAP: f32 = 4.;

/// The margin between the islands and the window's left, right and bottom edges, before zoom.
pub const ISLAND_MARGIN: f32 = 6.;

/// The margin above the islands on macOS, before zoom, which centres their headers on the
/// traffic lights. Windows and Linux have none: the caption bar sits above the islands.
pub const ISLAND_TOP_MARGIN: f32 = 8.;

pub fn work_card(collapsed_sidebar: bool) -> Div {
    div()
        .debug_selector(|| "WORK_CARD".into())
        .relative()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .mt(rems(ISLAND_TOP_MARGIN / 16.))
        .mb(rems(ISLAND_MARGIN / 16.))
        .when(cfg!(any(windows, target_os = "linux")), |card| card.mt_0())
        .mr(rems(ISLAND_MARGIN / 16.))
        .when(collapsed_sidebar, |card| card.ml(rems(ISLAND_MARGIN / 16.)))
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
        .mt(rems(ISLAND_TOP_MARGIN / 16.))
        .mb(rems(ISLAND_MARGIN / 16.))
        .when(cfg!(any(windows, target_os = "linux")), |area| area.mt_0())
        .mr(rems(ISLAND_MARGIN / 16.))
        .ml(rems(if collapsed_sidebar {
            ISLAND_MARGIN / 16.
        } else {
            ISLAND_GAP / 16.
        }))
}
