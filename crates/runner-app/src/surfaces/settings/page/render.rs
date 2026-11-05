use super::*;

impl NativeRoot {
    pub(crate) fn render_settings_takeover(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let zoom = self.settings(cx).app_zoom;
        let width = self.settings(cx).sidebar_width * zoom;
        let groups = filtered_nav_groups(&self.settings_page.search_query);
        let active = self.settings_page.pane;
        let nav = if groups.is_empty() {
            vec![div()
                .px(rems(10. / 16.))
                .py_1()
                .text_size(theme::text_ui())
                .text_color(theme::faint())
                .child(crate::i18n::text("ui.no_matching_settings"))
                .into_any_element()]
        } else {
            groups
                .into_iter()
                .map(|(label, panes)| self.render_settings_nav_group(label, panes, active, cx))
                .collect()
        };
        let content = self.render_settings_pane(cx);

        div()
            .absolute()
            .debug_selector(|| "SETTINGS_CHROME".into())
            .key_context("Settings")
            .track_focus(&self.settings_page.focus)
            .when(active == SettingsPane::Agents, |settings| {
                settings.on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "tab"
                        && !this.settings_page.search.read(cx).is_composing()
                        && !this
                            .settings_page
                            .agents
                            .as_ref()
                            .is_some_and(|pane| pane.read(cx).is_composing(cx))
                    {
                        cx.stop_propagation();
                        if event.keystroke.modifiers.shift {
                            window.focus_prev(cx);
                        } else {
                            window.focus_next(cx);
                        }
                    }
                }))
            })
            .inset_0()
            .flex()
            .overflow_hidden()
            .bg(gpui::transparent_black())
            .occlude()
            .child(
                div()
                    .relative()
                    .w(px(width))
                    .h_full()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .bg(if theme::is_glass() {
                        gpui::transparent_black()
                    } else {
                        theme::sidebar()
                    })
                    .children(self.render_sidebar_titlebar(window, cx))
                    .child(
                        div()
                            .flex_none()
                            .px_4()
                            .pb_3()
                            .pt_1()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(self.render_settings_back_button(cx))
                            .child(
                                div()
                                    .h(rems(32. / 16.))
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .rounded(rems(6. / 16.))
                                    .border_1()
                                    .border_color(theme::chrome_selected_border())
                                    .bg(theme::chrome_selected())
                                    .px(rems(10. / 16.))
                                    .child(
                                        svg()
                                            .path("search.svg")
                                            .size(rems(14. / 16.))
                                            .flex_none()
                                            .text_color(theme::faint()),
                                    )
                                    .child(
                                        div()
                                            .min_w(px(0.))
                                            .flex_1()
                                            .child(self.settings_page.search.clone()),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .relative()
                            .min_h(px(0.))
                            .flex_1()
                            .child(
                                div()
                                    .id("settings-nav-scroll")
                                    .size_full()
                                    .overflow_y_scroll()
                                    .scrollbar_width(px(0.))
                                    .track_scroll(&self.settings_page.nav_scroll)
                                    .px_3()
                                    .py_2()
                                    .flex()
                                    .flex_col()
                                    .gap_4()
                                    .children(nav),
                            )
                            .child(self.settings_page.nav_scrollbar.clone()),
                    ),
            )
            .child(
                runner_app::ui::work_card::work_card(false)
                    .children(self.render_daemon_banner(cx).map(|banner| {
                        div()
                            .flex_none()
                            .pt(rems(SETTINGS_DRAG_INSET / 16.))
                            .child(banner)
                    }))
                    .child(settings_content_column(
                        self.render_settings_titlebar_drag_area(
                            "settings-content-titlebar-drag",
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .right_0()
                                .h(rems(WORKSPACE_HEADER_HEIGHT / 16.))
                                .debug_selector(|| "SETTINGS_TITLEBAR_DRAG".into()),
                            cx,
                        ),
                        content,
                        &self.settings_page.content_scroll,
                        self.settings_page.content_scrollbar.clone(),
                        zoom,
                    )),
            )
            .child(self.render_sidebar_resize_handle(width, cx))
            .when(active == SettingsPane::Skills, |takeover| {
                takeover.children(
                    self.settings_page
                        .skills
                        .as_ref()
                        .map(|pane| pane.read(cx).detail.clone()),
                )
            })
            .when(active == SettingsPane::Mcp, |takeover| {
                takeover.children(
                    self.settings_page
                        .mcp
                        .as_ref()
                        .map(|pane| pane.read(cx).detail.clone()),
                )
            })
            .into_any_element()
    }

    pub(super) fn render_settings_back_button(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("settings-back")
            .debug_selector(|| "SETTINGS_BACK".into())
            .group("settings-back")
            .px_2()
            .py(rems(6. / 16.))
            .flex()
            .items_center()
            .gap_2()
            .rounded_sm()
            .border_1()
            .border_color(gpui::transparent_black())
            .cursor_pointer()
            .text_color(theme::muted())
            .hover(|button| {
                button
                    .border_color(theme::chrome_selected_border())
                    .bg(theme::chrome_hover())
                    .text_color(theme::text())
            })
            .child(
                svg()
                    .path("arrow-left.svg")
                    .size(rems(14. / 16.))
                    .flex_none()
                    .text_color(theme::muted())
                    .group_hover("settings-back", |icon| icon.text_color(theme::text())),
            )
            .child(
                div()
                    .text_size(theme::text_body())
                    .child(crate::i18n::text("ui.back_to_app")),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                this.leave_settings(window, cx);
            }))
            .into_any_element()
    }

    pub(super) fn render_settings_nav_group(
        &self,
        label: &'static str,
        panes: Vec<SettingsPane>,
        active: SettingsPane,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(rems(2. / 16.))
            .child(
                div()
                    .px(rems(10. / 16.))
                    .pb_1()
                    .text_size(theme::text_caption())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::faint())
                    .child(label.to_uppercase()),
            )
            .children(
                panes
                    .into_iter()
                    .map(|pane| self.render_settings_nav_item(pane, pane == active, cx)),
            )
            .into_any_element()
    }

    pub(super) fn render_settings_nav_item(
        &self,
        pane: SettingsPane,
        active: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = cx.entity();
        let key_root = root.clone();
        div()
            .id(SharedString::from(format!("settings-nav-{}", pane.key())))
            .debug_selector(move || format!("SETTINGS_NAV_{}", pane.key()))
            .tab_index(0)
            .px(rems(10. / 16.))
            .py_1()
            .flex()
            .items_center()
            .gap(rems(10. / 16.))
            .rounded_sm()
            .border_1()
            .border_color(if active {
                theme::chrome_selected_border()
            } else {
                gpui::transparent_black()
            })
            .bg(if active {
                theme::chrome_selected()
            } else {
                gpui::transparent_black()
            })
            .font_weight(if active {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::NORMAL
            })
            .text_size(theme::text_title())
            .text_color(if active {
                theme::text()
            } else {
                theme::muted()
            })
            .cursor_pointer()
            // A shadow paints its whole box, which shows through glass's translucent fill.
            .when(active && !theme::is_glass(), |row| row.shadow_sm())
            .when(!active, |row| {
                row.hover(|row| {
                    row.border_color(theme::chrome_selected_border())
                        .bg(theme::chrome_hover())
                        .text_color(theme::text())
                })
            })
            .focus_visible(|row| {
                row.shadow(vec![BoxShadow {
                    color: theme::border_strong(),
                    offset: gpui::point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(2.),
                    inset: false,
                }])
            })
            .child(
                svg()
                    .path(pane.icon())
                    .size(rems(14. / 16.))
                    .flex_none()
                    .text_color(if active {
                        theme::text()
                    } else {
                        theme::muted()
                    }),
            )
            .child(div().min_w(px(0.)).truncate().child(pane.label()))
            .on_click(move |_, window, cx| {
                root.update(cx, |this, root_cx| {
                    this.enter_settings_pane(pane, window, root_cx)
                });
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    key_root.update(cx, |this, root_cx| {
                        this.enter_settings_pane(pane, window, root_cx)
                    });
                }
            })
            .into_any_element()
    }

    pub(super) fn render_settings_pane(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        match self.settings_page.pane {
            SettingsPane::General => Some(self.render_general_settings(cx)),
            SettingsPane::Missions => Some(self.render_missions_settings(cx)),
            SettingsPane::Appearance => Some(self.render_appearance_settings(cx)),
            SettingsPane::Account => Some(self.render_account_settings(cx)),
            SettingsPane::Remote => Some(self.render_remote_settings(cx)),
            SettingsPane::Notifications => Some(self.render_notifications_settings(cx)),
            SettingsPane::Terminal => Some(self.render_terminal_settings(cx)),
            SettingsPane::Shortcuts => Some(self.render_shortcuts_settings(cx)),
            SettingsPane::SidePanel => Some(self.render_side_panel_settings(cx)),
            SettingsPane::Files => Some(self.render_files_settings(cx)),
            SettingsPane::Agents => self
                .settings_page
                .agents
                .clone()
                .map(IntoElement::into_any_element),
            SettingsPane::Skills => self
                .settings_page
                .skills
                .clone()
                .map(IntoElement::into_any_element),
            SettingsPane::Mcp => self
                .settings_page
                .mcp
                .clone()
                .map(IntoElement::into_any_element),
            SettingsPane::Updates => self
                .settings_page
                .updates
                .clone()
                .map(IntoElement::into_any_element),
            SettingsPane::Diagnostics => self
                .settings_page
                .diagnostics
                .clone()
                .map(IntoElement::into_any_element),
            SettingsPane::About => self
                .settings_page
                .about
                .clone()
                .map(IntoElement::into_any_element),
            SettingsPane::Archived => self
                .settings_page
                .archived
                .clone()
                .map(IntoElement::into_any_element),
        }
    }
}
