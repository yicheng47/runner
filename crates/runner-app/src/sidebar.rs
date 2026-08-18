//! Sidebar rendering: the tab list and its labels.
use super::*;

impl NativeRoot {
    pub(crate) fn tab_label(&self, layout: &PaneLayout) -> String {
        if let Some(name) = &layout.name {
            return name.clone();
        }
        let labels = layout
            .session_ids()
            .into_iter()
            .filter_map(|session_id| self.session_entry(&session_id))
            .map(session_label)
            .collect::<Vec<_>>();
        if labels.is_empty() {
            "Empty tab".into()
        } else {
            labels.join(" + ")
        }
    }

    pub(crate) fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let active_tab_id = self.tabs.active_tab_id().map(str::to_owned);
        let tab_rows = self
            .tabs
            .tabs()
            .iter()
            .map(|layout| {
                (
                    layout.id.clone(),
                    self.tab_label(layout),
                    layout.root.leaves().len(),
                    layout.session_ids().iter().any(|session_id| {
                        self.session_entry(session_id)
                            .is_some_and(|entry| entry.status == SessionStatus::Running)
                    }),
                )
            })
            .collect::<Vec<_>>();

        let list: AnyElement = if let Some(target) = self.new_chat_target.as_ref() {
            let heading = match target {
                NewChatTarget::NewTab => "NEW TAB",
                NewChatTarget::Pane { .. } => "NEW CHAT IN PANE",
            };
            div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_col()
                .child(
                    div()
                        .px_4()
                        .pb_2()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(div().text_xs().text_color(theme::muted()).child(heading))
                        .child(
                            div()
                                .id("cancel-new-chat")
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .cursor_pointer()
                                .text_xs()
                                .text_color(theme::muted())
                                .hover(|button| button.bg(theme::border()))
                                .child("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.new_chat_target = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .id("new-chat-runner-list")
                        .flex_1()
                        .overflow_y_scroll()
                        .px_2()
                        .children(self.runners.iter().enumerate().map(|(index, runner)| {
                            let runner_id = runner.id.clone();
                            div()
                                .id(("new-chat-runner", index))
                                .w_full()
                                .mb_1()
                                .px_3()
                                .py_2()
                                .rounded_md()
                                .cursor_pointer()
                                .hover(|row| row.bg(theme::border()))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(theme::text())
                                        .child(format!("@{}", runner.handle)),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme::muted())
                                        .child(runner.display_name.clone()),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.start_chat(&runner_id, window, cx);
                                }))
                        })),
                )
                .into_any_element()
        } else {
            div()
                .id("tab-list")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .px_2()
                .children(tab_rows.into_iter().enumerate().map(
                    |(index, (tab_id, label, pane_count, running))| {
                        let selected = active_tab_id.as_deref() == Some(&tab_id);
                        let click_id = tab_id.clone();
                        div()
                            .id(("direct-tab", index))
                            .w_full()
                            .mb_1()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .cursor_pointer()
                            .when(selected, |row| row.bg(theme::border()))
                            .hover(|row| row.bg(theme::border()))
                            .child(div().text_sm().text_color(theme::text()).child(label))
                            .child(div().text_xs().text_color(theme::muted()).child(format!(
                                "{} · {pane_count} {}",
                                if running { "running" } else { "stopped" },
                                if pane_count == 1 { "pane" } else { "panes" }
                            )))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.activate_tab(&click_id, window, cx);
                            }))
                    },
                ))
                .into_any_element()
        };

        div()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(theme::composer_bg())
            .border_r_1()
            .border_color(theme::border())
            .child(
                div()
                    .px_4()
                    .pt_10()
                    .pb_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme::muted())
                            .child("DIRECT CHAT TABS"),
                    )
                    .child(
                        div()
                            .id("new-tab-sidebar")
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .cursor_pointer()
                            .text_xs()
                            .text_color(theme::accent())
                            .hover(|button| button.bg(theme::border()))
                            .child("+ New")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.begin_new_tab(&NewTab, window, cx);
                            })),
                    ),
            )
            .child(list)
            .into_any_element()
    }
}

pub(crate) fn session_label(entry: &DirectSessionEntry) -> String {
    entry.title.clone().unwrap_or_else(|| {
        entry
            .handle
            .as_ref()
            .map(|handle| format!("@{handle}"))
            .unwrap_or_else(|| entry.display_name.clone())
    })
}
