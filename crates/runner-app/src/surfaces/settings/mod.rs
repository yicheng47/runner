pub(crate) mod about;
pub(crate) mod agents;
pub(crate) mod archived;
pub(crate) mod claude_accounts;
pub(crate) mod mcp;
pub(crate) mod notifications;
pub(crate) mod page;
pub(crate) mod remote;
pub(crate) mod skills;
pub(crate) mod theme_preview;
pub(crate) mod updates;

/// Raised by a settings editor's Save, success or failure, so the app shell
/// can show it as a toast.
pub(crate) struct SaveNotice {
    pub message: runner_app::i18n::Text,
    pub tone: crate::toast::ToastTone,
}

fn app_identity_card(
    details: impl IntoIterator<Item = gpui::AnyElement>,
    action: Option<gpui::AnyElement>,
    below: Option<gpui::AnyElement>,
) -> gpui::Div {
    use crate::theme;
    use gpui::prelude::*;
    use gpui::{div, img, rems, FontWeight};

    div()
        .overflow_hidden()
        .rounded(rems(12. / 16.))
        .border_1()
        .border_color(theme::border())
        .bg(theme::panel())
        .flex()
        .flex_col()
        .gap(rems(14. / 16.))
        .p_5()
        .child(
            div()
                .flex()
                .items_center()
                .gap_4()
                .child(
                    img(crate::assets::app_icon_source())
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
                                        .text_size(theme::text_heading())
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
                                        .font_family(theme::UI_MONOSPACE_FONT)
                                        .text_size(theme::text_meta())
                                        .text_color(theme::muted())
                                        .child(runner_app::version::display_version_label()),
                                ),
                        )
                        .children(details),
                )
                .children(action),
        )
        .children(below)
}
