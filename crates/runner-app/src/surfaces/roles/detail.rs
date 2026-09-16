use super::logic::big_stat;
use super::logic::detail_card;
use super::logic::detail_metadata_row;
use super::logic::detail_row;
use super::logic::error_banner;
use super::logic::format_timestamp;

use gpui::prelude::*;
use gpui::{div, px, relative, rems, AnyElement, Context, FontWeight, KeyDownEvent, SharedString};
use runner_app::ui::{Button, ButtonVariant, RuntimeBadge};
use runner_backend::model::Role;
use runner_backend::ops::role::RoleActivity;
use runner_backend::ops::slot::CrewMembership;

use crate::*;

impl NativeRoot {
    pub(super) fn render_role_detail(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let root = cx.entity();
        let back_root = root.clone();
        let back_key_root = root.clone();
        let edit_root = root.clone();
        let chat_root = root.clone();
        let detail = &self.role_surfaces.detail;
        let handle = detail.handle.clone();
        let role = detail.role.clone();
        let pending = role.as_ref().is_some_and(|role| {
            self.role_surfaces.chat_pending.as_deref() == Some(role.id.as_str())
        });
        let body = if detail.loading {
            div()
                .text_size(theme::text_title())
                .text_color(theme::muted())
                .child("Loading…")
                .into_any_element()
        } else if let Some(role) = role.clone() {
            self.render_role_detail_body(role, detail.activity.clone(), detail.crews.clone(), cx)
        } else {
            div()
                .rounded_sm()
                .border_1()
                .border_color(theme::with_alpha(theme::danger(), 0.4))
                .bg(theme::with_alpha(theme::danger(), 0.1))
                .px_3()
                .py_2()
                .text_size(theme::text_title())
                .text_color(theme::danger())
                .child(format!("Role @{handle} not found."))
                .into_any_element()
        };
        let header_role = role.clone();
        let chat_role = role.clone();
        div()
            .id("role-detail-scroll")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .child(
                div()
                    .mx_auto()
                    .w_full()
                    .max_w(rems(1024. / 16.))
                    .flex()
                    .flex_col()
                    .gap_6()
                    .px_8()
                    .py_8()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_4()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .text_size(theme::text_title())
                                    .text_color(theme::muted())
                                    .child(
                                        div()
                                            .id("role-detail-back")
                                            .tab_index(0)
                                            .cursor_pointer()
                                            .hover(|text| text.text_color(theme::text()))
                                            .focus_visible(|text| {
                                                text.text_color(theme::text()).underline()
                                            })
                                            .on_click(move |_, window, cx| {
                                                back_root.update(cx, |this, cx| {
                                                    this.open_roles(window, cx)
                                                });
                                            })
                                            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                                                if matches!(
                                                    event.keystroke.key.as_str(),
                                                    "enter" | "space"
                                                ) {
                                                    cx.stop_propagation();
                                                    back_key_root.update(cx, |this, cx| {
                                                        this.open_roles(window, cx)
                                                    });
                                                }
                                            })
                                            .child("Roles"),
                                    )
                                    .child(div().text_color(theme::border_strong()).child("›"))
                                    .child(
                                        div()
                                            .font_family(theme::UI_MONOSPACE_FONT)
                                            .text_size(theme::text_heading())
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme::text())
                                            .child(format!("@{handle}")),
                                    )
                                    .children(role.as_ref().map(|role| {
                                        RuntimeBadge::new(role.runtime.clone()).uppercase(true)
                                    })),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        Button::new("edit-role", "Edit")
                                            .tooltip("Edit role")
                                            .disabled(header_role.is_none())
                                            .on_press(move |window, cx| {
                                                if let Some(role) = header_role.clone() {
                                                    edit_root.update(cx, |this, cx| {
                                                        this.open_role_edit(role, None, window, cx)
                                                    });
                                                }
                                            }),
                                    )
                                    .child(
                                        Button::new(
                                            "role-detail-chat",
                                            if pending { "Starting…" } else { "Chat now" },
                                        )
                                        .variant(ButtonVariant::Primary)
                                        .tooltip("Start a one-on-one PTY with this role")
                                        .disabled(chat_role.is_none() || pending)
                                        .on_press(
                                            move |window, cx| {
                                                if let Some(role) = chat_role.clone() {
                                                    chat_root.update(cx, |this, cx| {
                                                        this.start_role_chat(role, window, cx)
                                                    });
                                                }
                                            },
                                        ),
                                    ),
                            ),
                    )
                    .children(role.as_ref().map(|role| {
                        div()
                            .text_size(theme::text_title())
                            .text_color(theme::muted())
                            .child(role.display_name.clone())
                    }))
                    .children(detail.error.clone().map(error_banner))
                    .child(body),
            )
            .into_any_element()
    }

    fn render_role_detail_body(
        &self,
        role: Role,
        activity: Option<RoleActivity>,
        crews: Vec<CrewMembership>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = cx.entity();
        let crew_rows = if crews.is_empty() {
            div()
                .text_size(theme::text_title())
                .text_color(theme::faint())
                .italic()
                .child("Not in any crew yet. Add it to one from Crew Detail.")
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .children(crews.into_iter().map(|membership| {
                    let crew_root = root.clone();
                    let crew_key_root = root.clone();
                    let crew_id = membership.crew_id.clone();
                    let crew_key_id = crew_id.clone();
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .border_b_1()
                        .border_color(theme::border())
                        .py_2()
                        .text_size(theme::text_title())
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(theme::text())
                                        .child(membership.crew_name),
                                )
                                .children(membership.lead.then(|| {
                                    div()
                                        .rounded_sm()
                                        .bg(theme::with_alpha(theme::accent(), 0.1))
                                        .px(rems(6. / 16.))
                                        .py(rems(2. / 16.))
                                        .text_size(theme::text_caption())
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(theme::accent())
                                        .child("LEAD")
                                })),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!("open-role-crew-{}", crew_id)))
                                .tab_index(0)
                                .cursor_pointer()
                                .text_size(theme::text_ui())
                                .text_color(theme::accent())
                                .hover(|text| text.underline())
                                .focus_visible(|text| text.underline())
                                .on_click(move |_, window, cx| {
                                    crew_root.update(cx, |this, cx| {
                                        this.open_crew_editor(crew_id.clone(), window, cx)
                                    });
                                })
                                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                        cx.stop_propagation();
                                        crew_key_root.update(cx, |this, cx| {
                                            this.open_crew_editor(crew_key_id.clone(), window, cx)
                                        });
                                    }
                                })
                                .child("Open →"),
                        )
                }))
                .into_any_element()
        };
        let sessions = activity.as_ref().map_or(0, |value| value.active_sessions);
        let missions = activity.as_ref().map_or(0, |value| value.active_missions);
        let crew_count = activity.as_ref().map_or(0, |value| value.crew_count);
        let last_seen = activity
            .and_then(|value| value.last_started_at)
            .map(format_timestamp)
            .unwrap_or_else(|| "—".into());
        let args = role.args.join(" ");
        div()
            .w_full()
            .flex()
            .items_start()
            .gap_4()
            .child(
                div()
                    .w(relative(2. / 3.))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(detail_card(
                        "Default system prompt",
                        Some("Used whenever this role spawns. Override per crew/mission slot later (v0.x)."),
                        if let Some(prompt) = role.system_prompt.clone() {
                            div()
                                .font_family(theme::UI_MONOSPACE_FONT)
                                .text_size(theme::text_ui())
                                .line_height(rems(19.5 / 16.))
                                .text_color(theme::text())
                                .child(prompt)
                                .into_any_element()
                        } else {
                            div()
                                .text_size(theme::text_title())
                                .italic()
                                .text_color(theme::faint())
                                .child("No system prompt set.")
                                .into_any_element()
                        },
                    ))
                    .child(detail_card("Crews using this role", None, crew_rows))
                    .child(detail_card(
                        "Chat now",
                        Some("Spawn a one-on-one PTY. Chats don't join any mission's coordination bus."),
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .text_size(theme::text_ui())
                            .text_color(theme::muted())
                            .child("Working directory")
                            .child(
                                div()
                                    .rounded_sm()
                                    .border_1()
                                    .border_color(theme::border_strong())
                                    .bg(theme::bg())
                                    .p(rems(6. / 16.))
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(theme::text_ui())
                                    .text_color(theme::faint())
                                    .child(role.working_dir.clone().unwrap_or_else(|| "—".into())),
                            )
                            .child(
                                div()
                                    .text_size(theme::text_meta())
                                    .text_color(theme::faint())
                                    .child("Inherits the role's working directory. Click Edit to change it, or override per-chat from the chat itself."),
                            ),
                    )),
            )
            .child(
                div()
                    .w(relative(1. / 3.))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(detail_card(
                        "Activity",
                        None,
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(big_stat("sessions", sessions, sessions > 0))
                                    .child(big_stat("missions", missions, missions > 0)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .border_t_1()
                                    .border_color(theme::border())
                                    .pt_3()
                                    .text_size(theme::text_ui())
                                    .child(detail_row("In crews", crew_count.to_string()))
                                    .child(detail_row("Last seen", last_seen)),
                            ),
                    ))
                    .child(detail_card(
                        "Details",
                        None,
                        div()
                            .flex()
                            .flex_col()
                            .gap(rems(6. / 16.))
                            .child(detail_metadata_row(
                                "Handle",
                                format!("@{}", role.handle),
                                true,
                                false,
                            ))
                            .child(detail_metadata_row(
                                "Runtime",
                                role.runtime,
                                false,
                                false,
                            ))
                            .child(detail_metadata_row(
                                "Command",
                                role.command,
                                true,
                                false,
                            ))
                            .children((!args.is_empty()).then(|| {
                                detail_metadata_row("Args", args, true, false)
                            }))
                            .child(detail_metadata_row(
                                "Created",
                                format_timestamp(role.created_at),
                                false,
                                false,
                            ))
                            .child(detail_metadata_row("ID", role.id, true, true)),
                    )),
            )
            .into_any_element()
    }
}
