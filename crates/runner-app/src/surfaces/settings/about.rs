use gpui::prelude::*;
use gpui::{div, img, rems, svg, Context, CursorStyle, FontWeight, KeyDownEvent, Render, Window};
use runner_app::ui::{PaneHeader, SettingsCard};

use crate::{assets::app_icon_source, theme};

pub(crate) struct AboutPane;

impl AboutPane {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Render for AboutPane {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(PaneHeader::new("About", "Version, credits, and links."))
            .child(
                div()
                    .overflow_hidden()
                    .rounded(rems(12. / 16.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_4()
                            .p_5()
                            .child(
                                img(app_icon_source())
                                    .size(rems(56. / 16.))
                                    .flex_none()
                                    .rounded(rems(1.)),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .text_size(rems(1.))
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(theme::text())
                                                    .child("Runner"),
                                            )
                                            .child(
                                                div()
                                                    .rounded_sm()
                                                    .bg(theme::raised())
                                                    .px(rems(6. / 16.))
                                                    .py(rems(2. / 16.))
                                                    .font_family("Menlo")
                                                    .text_size(rems(11. / 16.))
                                                    .text_color(theme::muted())
                                                    .child(format!(
                                                        "v{}",
                                                        runner_app::version::display_version()
                                                    )),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(rems(12. / 16.))
                                            .text_color(theme::muted())
                                            .child("Local cockpit for coding agents."),
                                    ),
                            ),
                    ),
            )
            .child(SettingsCard::new([
                link_row(
                    "about-github",
                    "github.svg",
                    "GitHub",
                    Some("https://github.com/yicheng47/runner"),
                    None,
                ),
                link_row(
                    "about-documentation",
                    "book-text.svg",
                    "Documentation",
                    Some("https://github.com/yicheng47/runner#readme"),
                    None,
                ),
                link_row("about-license", "scale.svg", "License", None, Some("MIT")),
            ]))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(rems(11. / 16.))
                    .text_color(theme::faint())
                    .child("© 2026 wyc studios"),
            )
    }
}

fn link_row(
    id: &'static str,
    icon: &'static str,
    label: &'static str,
    url: Option<&'static str>,
    trailing: Option<&'static str>,
) -> gpui::AnyElement {
    let interactive = url.is_some();
    let mut row = div()
        .id(id)
        .tab_index(if interactive { 0 } else { -1 })
        .w_full()
        .flex()
        .items_center()
        .justify_between()
        .px_4()
        .py_3()
        .cursor(if interactive {
            CursorStyle::PointingHand
        } else {
            CursorStyle::Arrow
        })
        .when(interactive, |row| {
            row.hover(|row| row.bg(theme::with_alpha(theme::raised(), 0.4)))
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(10. / 16.))
                .child(
                    svg()
                        .path(icon)
                        .size(rems(14. / 16.))
                        .text_color(theme::muted()),
                )
                .child(
                    div()
                        .text_size(rems(13. / 16.))
                        .text_color(theme::text())
                        .child(label),
                ),
        )
        .child(if let Some(trailing) = trailing {
            div()
                .text_size(rems(12. / 16.))
                .text_color(theme::faint())
                .child(trailing)
        } else {
            div().child(
                svg()
                    .path("external-link.svg")
                    .size(rems(12. / 16.))
                    .text_color(theme::faint()),
            )
        });
    if let Some(url) = url {
        row = row.on_click(move |_, _, cx| cx.open_url(url)).on_key_down(
            move |event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    cx.open_url(url);
                }
            },
        );
    }
    row.into_any_element()
}
