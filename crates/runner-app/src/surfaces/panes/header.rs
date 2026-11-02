use super::*;

impl NativeRoot {
    pub(super) fn render_island_panel_toggle(
        &self,
        layout: &PaneLayout,
        pane_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let terminal_tab = super::super::chat::tab_is_terminal(layout, |id| {
            self.front_page_session(id, cx)
                .and_then(|entry| Runtime::parse(&entry.agent_runtime))
        });
        if !self.remote_client.is_local()
            || start_surface_visible(Some(layout))
            || self.settings(cx).chat_panel_open
            || terminal_tab
            || top_right_pane(&layout.root).id != pane_id
        {
            return None;
        }
        let root = cx.entity();
        Some(
            div()
                .when(cfg!(test), |toggle| {
                    toggle.debug_selector(|| "CHAT_PANEL_TOGGLE".into())
                })
                .flex_none()
                .flex()
                .items_center()
                .ml_1()
                .gap_1()
                .child(div().w(rems(1. / 16.)).h(rems(1.)).bg(theme::border()))
                .child(
                    IconButton::new("open-chat-panel", "panel-right-hidden.svg")
                        .size(IconButtonSize::Sm)
                        .tooltip(crate::i18n::text("ui.open_side_panel"))
                        .on_press(move |_, cx| {
                            root.update(cx, |this, cx| {
                                this.update_app_settings(cx, true, |settings| {
                                    settings.chat_panel_open = true;
                                    true
                                });
                                cx.notify();
                            })
                        }),
                )
                .into_any_element(),
        )
    }

    pub(super) fn claude_account_details(
        &self,
        detail: &DirectSessionEntry,
        cx: &App,
    ) -> Option<(String, Option<String>, String)> {
        let store = self.app_store.read(cx);
        if detail.agent_runtime != "claude-code" || !store.claude_accounts.multiple() {
            return None;
        }
        let session = detail.claude_account.as_ref();
        let account = store.claude_accounts.session_account(session);
        let name = account
            .map(|account| account.name.clone())
            .or_else(|| session.map(|session| session.folder.clone()))?;
        let email = account
            .and_then(|account| store.claude_accounts.statuses.get(&account.id))
            .and_then(|status| status.email.clone());
        let folder = session
            .map(|session| session.folder.clone())
            .or_else(|| account.map(|account| account.folder.clone()))?;
        Some((name, email, folder))
    }

    pub(super) fn claude_paused_text(
        &self,
        detail: &DirectSessionEntry,
        cx: &App,
    ) -> Option<String> {
        if detail
            .claude_account
            .as_ref()
            .is_some_and(|account| account.pinned)
        {
            return None;
        }
        let (previous, _, folder) = self.claude_account_details(detail, cx)?;
        let store = self.app_store.read(cx);
        let active = store.claude_accounts.active()?;
        let session = detail.claude_account.as_ref();
        if folder == active.folder && session.is_none_or(|session| session.native == active.native)
        {
            return None;
        }
        Some(crate::i18n::tf(
            "accounts.paused",
            &[("previous", previous), ("active", active.name.clone())],
        ))
    }

    pub(super) fn render_chat_side_panel(
        &self,
        detail: Option<&DirectSessionEntry>,
        session_key_copy: Entity<CopyValueButton>,
        visibility: f32,
        show_panel: bool,
        border_on: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        #[cfg(test)]
        crate::render_counts::count_render("side-panel", cx);
        let width = self.settings(cx).chat_panel_width;
        let visible_width = width * visibility;
        if !show_panel {
            return div()
                .id("chat-side-panel")
                .relative()
                .w(rems(visible_width / 16.))
                .h_full()
                .flex_none()
                .overflow_hidden()
                .into_any_element();
        }
        let role = detail
            .and_then(|detail| detail.role_id.as_deref())
            .and_then(|role_id| {
                self.app_store
                    .read(cx)
                    .roles
                    .iter()
                    .find(|role| role.id == role_id)
                    .cloned()
            });
        let collapse_root = cx.entity();
        let header = self.render_titlebar_drag_area(
            "chat-panel-titlebar-drag",
            div()
                .flex_none()
                .h(rems(
                    if self.route == AppRoute::Chat {
                        40.
                    } else {
                        WORKSPACE_HEADER_HEIGHT
                    } / 16.,
                ))
                .pl_4()
                .pr_2()
                .flex()
                .items_center()
                .justify_end()
                .border_b_1()
                .border_color(theme::border())
                .rounded_tr(rems(11. / 16.))
                .map(|element| {
                    #[cfg(test)]
                    let element = crate::theme_snapshot::record_fill("CHAT_PANEL_HEADER", element);
                    element
                })
                .child(
                    IconButton::new("collapse-chat-panel", "panel-right-open.svg")
                        .tooltip(crate::i18n::text("ui.collapse_side_panel"))
                        .on_press(move |_, cx| {
                            collapse_root.update(cx, |this, cx| {
                                this.update_app_settings(cx, true, |settings| {
                                    settings.chat_panel_open = false;
                                    true
                                });
                                cx.notify();
                            });
                        }),
                ),
            cx,
        );
        let content = if let Some(detail) = detail {
            let open_root = cx.entity();
            let toggle_root = cx.entity();
            let actions = ChatPanelActions {
                open_role: role.as_ref().map(|role| {
                    let handle = role.handle.clone();
                    Rc::new(move |window: &mut Window, cx: &mut App| {
                        open_root.update(cx, |this, cx| {
                            this.open_role_detail(handle.clone(), window, cx)
                        });
                    }) as ClickHandler
                }),
                toggle_prompt: Rc::new(move |_, cx| {
                    toggle_root.update(cx, |this, cx| {
                        this.chat_panel_prompt_expanded = !this.chat_panel_prompt_expanded;
                        cx.notify();
                    });
                }),
                expanded: self.chat_panel_prompt_expanded,
                view: cx.entity_id(),
            };
            chat_panel_content(
                detail,
                role.as_ref(),
                width,
                session_key_copy,
                actions,
                self.claude_account_details(detail, cx),
                cx,
            )
        } else {
            div()
                .text_size(theme::text_ui())
                .text_color(theme::faint())
                .child(crate::i18n::text("ui.loading_chat"))
                .into_any_element()
        };
        let panel = div()
            .relative()
            .w(rems(width / 16.))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .flex_col()
                    .pb(rems(8. / 16.))
                    .child(
                        div()
                            .id("chat-panel-scroll")
                            .flex_1()
                            .min_h(px(0.))
                            .overflow_y_scroll()
                            .p_5()
                            .child(content),
                    ),
            );
        div()
            .id("chat-side-panel")
            .debug_selector(|| "CHAT_SIDE_PANEL".into())
            .relative()
            .w(rems(visible_width / 16.))
            .h_full()
            .flex_none()
            .overflow_hidden()
            .when(self.route == AppRoute::Chat, |panel| {
                panel
                    .rounded(rems(12. / 16.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::bg())
            })
            .when(self.route != AppRoute::Chat && border_on, |panel| {
                panel.border_l_1().border_color(theme::border())
            })
            .child(panel)
            .into_any_element()
    }
}
