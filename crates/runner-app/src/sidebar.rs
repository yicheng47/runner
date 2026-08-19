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

    pub(crate) fn render_sidebar_contents(&self, cx: &mut Context<Self>) -> AnyElement {
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

        let list = div()
            .id("tab-list")
            .size_full()
            .min_h(px(0.))
            .overflow_y_scroll()
            .scrollbar_width(px(0.))
            .track_scroll(&self.sidebar_scroll)
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
                        .border_1()
                        .border_color(if selected {
                            theme::sidebar_selected_border()
                        } else {
                            gpui::transparent_black()
                        })
                        .when(selected, |row| row.bg(theme::sidebar_selected()))
                        .hover(|row| row.bg(theme::sidebar_selected()))
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
            ));

        div()
            .min_h(px(0.))
            .flex_1()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_5()
                    .pb_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(rems(10. / 16.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme::faint())
                            .child("CHATS & MISSIONS"),
                    )
                    .child({
                        let root = cx.entity();
                        Button::new("new-tab-sidebar", "+ New")
                            .size(ButtonSize::Sm)
                            .variant(runner_app::ui::ButtonVariant::Ghost)
                            .on_press(move |window, cx| {
                                root.update(cx, |this, cx| {
                                    this.open_new_tab_modal(&NewTab, window, cx);
                                });
                            })
                    }),
            )
            .child(
                div()
                    .relative()
                    .min_h(px(0.))
                    .flex_1()
                    .child(list)
                    .child(self.sidebar_scrollbar.clone()),
            )
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
