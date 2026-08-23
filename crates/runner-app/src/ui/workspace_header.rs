use gpui::prelude::*;
use gpui::{div, px, rems, svg, AnyElement, FontWeight, Pixels, SharedString};

use crate::theme;

pub const WORKSPACE_HEADER_HEIGHT: f32 = 44.;

pub struct WorkspaceHeader {
    left_padding: Pixels,
    icon: SharedString,
    title: SharedString,
    sidebar_toggle: Option<AnyElement>,
    title_actions: Vec<AnyElement>,
    trailing_actions: Vec<AnyElement>,
}

impl WorkspaceHeader {
    pub fn new(
        left_padding: Pixels,
        icon: impl Into<SharedString>,
        title: impl Into<SharedString>,
    ) -> Self {
        Self {
            left_padding,
            icon: icon.into(),
            title: title.into(),
            sidebar_toggle: None,
            title_actions: Vec::new(),
            trailing_actions: Vec::new(),
        }
    }

    pub fn sidebar_toggle(mut self, toggle: Option<AnyElement>) -> Self {
        self.sidebar_toggle = toggle;
        self
    }

    pub fn title_actions(mut self, actions: impl IntoIterator<Item = AnyElement>) -> Self {
        self.title_actions = actions.into_iter().collect();
        self
    }

    pub fn trailing_actions(mut self, actions: impl IntoIterator<Item = AnyElement>) -> Self {
        self.trailing_actions = actions.into_iter().collect();
        self
    }

    pub fn into_div(self) -> gpui::Div {
        let sidebar_divider = self.sidebar_toggle.is_some().then(|| {
            div()
                .mx_1()
                .h(rems(20. / 16.))
                .w(rems(1. / 16.))
                .flex_none()
                .bg(theme::border())
        });
        let title_group = div()
            .flex_1()
            .min_w(px(0.))
            .flex()
            .items_center()
            .gap_3()
            .child(
                svg()
                    .path(self.icon)
                    .size(rems(15. / 16.))
                    .flex_none()
                    .text_color(theme::accent()),
            )
            .child(
                div()
                    .min_w(px(0.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .relative()
                            .top(rems(1. / 16.))
                            .min_w(px(0.))
                            .truncate()
                            .text_size(rems(13. / 16.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::text())
                            .child(self.title),
                    )
                    .children(self.title_actions),
            );

        div()
            .flex_none()
            .h(rems(WORKSPACE_HEADER_HEIGHT / 16.))
            .pl(self.left_padding)
            .pr_2()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .children(self.sidebar_toggle)
                    .children(sidebar_divider)
                    .child(title_group)
                    .children((!self.trailing_actions.is_empty()).then(|| {
                        div()
                            .ml_auto()
                            .flex()
                            .items_center()
                            .gap_2()
                            .children(self.trailing_actions)
                    })),
            )
    }
}
