use super::*;

impl NativeRoot {
    pub(crate) fn render_archived_chat(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(detail) = self.archived_chat_detail.as_ref() else {
            return div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_col()
                .children(
                    (self.route != AppRoute::Settings)
                        .then(|| self.render_daemon_banner(cx))
                        .flatten(),
                )
                .into_any_element();
        };
        let label = session_label(detail);
        let back_label = if detail.handle.is_some() {
            crate::i18n::text("ui.back_to_role")
        } else {
            crate::i18n::text("ui.back_to_roles")
        };
        let handle = detail.handle.clone();
        let root = cx.entity();
        let panel_root = root.clone();
        let sidebar_toggle = self.render_collapsed_titlebar_spacer();
        let sidebar_divider = sidebar_toggle.is_some().then(|| {
            div()
                .mx_1()
                .h(rems(20. / 16.))
                .w(rems(1. / 16.))
                .flex_none()
                .bg(theme::border())
        });
        let header = div()
            .flex_none()
            .h(rems(WORKSPACE_HEADER_HEIGHT / 16.))
            .pl(px(self.workspace_titlebar_padding(window, cx)))
            .pr_2()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(theme::border())
            .rounded_tl(rems(11. / 16.))
            .when(!self.settings(cx).chat_panel_open, |header| {
                header.rounded_tr(rems(11. / 16.))
            })
            .children(sidebar_toggle)
            .children(sidebar_divider)
            .child(
                svg()
                    .path("terminal.svg")
                    .size(rems(15. / 16.))
                    .flex_none()
                    .text_color(theme::accent()),
            )
            .child(
                div()
                    .min_w(px(0.))
                    .truncate()
                    .text_size(theme::text_body())
                    .font_weight(FontWeight::MEDIUM)
                    .child(label),
            )
            .child(
                Button::new("archived-chat-back", back_label)
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .on_press(move |window, cx| {
                        let handle = handle.clone();
                        root.update(cx, |this, root_cx| {
                            this.archived_chat_detail = None;
                            if let Some(handle) = handle {
                                this.open_role_detail(handle, window, root_cx);
                            } else {
                                this.open_roles(window, root_cx);
                            }
                        });
                    }),
            )
            .child(div().min_w(px(0.)).flex_1())
            .when(!self.settings(cx).chat_panel_open, |header| {
                header.child(
                    IconButton::new("open-archived-chat-panel", "panel-right-hidden.svg")
                        .tooltip(crate::i18n::text("ui.open_side_panel"))
                        .on_press(move |_, cx| {
                            panel_root.update(cx, |this, cx| {
                                this.update_app_settings(cx, true, |settings| {
                                    settings.chat_panel_open = true;
                                    true
                                });
                                cx.notify();
                            });
                        }),
                )
            });
        let chat_column = div()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .child(self.render_titlebar_drag_area("archived-chat-titlebar-drag", header, cx))
            .children((self.route != AppRoute::Settings).then(|| self.render_daemon_banner(cx)).flatten())
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .max_w(rems(448. / 16.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_2()
                            .rounded(rems(4. / 16.))
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::raised())
                            .px_6()
                            .py_5()
                            .text_center()
                            .child(
                                div()
                                    .text_size(theme::text_body())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(crate::i18n::text("ui.session_ended_terminal_closed")),
                            )
                            .child(
                                div()
                                    .text_size(theme::text_ui())
                                    .text_color(theme::muted())
                                    .child(crate::i18n::text("ui.this_chat_was_archived_the_pty_is_gone_and_the_workspace_is_read_only")),
                            ),
                    ),
            );
        let panel_open = self.settings(cx).chat_panel_open;
        let (panel_visibility, panel_animating) = self.chat_panel_visibility.animate_to(
            if panel_open { 1. } else { 0. },
            Instant::now(),
            Duration::from_millis(CHAT_PANEL_TRANSITION_MS),
        );
        if panel_animating {
            window.request_animation_frame();
        }
        self.reset_chat_panel_prompt(
            self.archived_chat_detail
                .as_ref()
                .map(|chat| chat.session_id.clone()),
        );
        let side_panel = self.render_chat_side_panel(
            self.archived_chat_detail.as_ref(),
            self.archived_session_key_copy.clone(),
            panel_visibility,
            panel_open || panel_animating,
            panel_open && !panel_animating,
            cx,
        );
        div()
            .track_focus(&self.chat_focus)
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .child(chat_column)
            .child(side_panel)
            .into_any_element()
    }

    pub(crate) fn render_active_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.prune_launchers(cx);
        if self.remote_client.is_local() {
            self.ensure_active_chat_detail(cx);
        }
        let error_banner = self
            .chat_error
            .clone()
            .filter(|_| self.remote_client.is_local())
            .filter(|_| !self.app_store.read(cx).daemon_disconnected)
            .map(|error| {
                runner_app::ui::notice_banner(error.resolve(), runner_app::ui::Tone::Danger)
                    .when(cfg!(test), |banner| {
                        banner.debug_selector(|| "CHAT_ERROR_BANNER".into())
                    })
            });
        let warning_root = cx.entity();
        let warning_banner = self
            .chat_warning
            .clone()
            .filter(|_| self.remote_client.is_local())
            .filter(|_| !self.app_store.read(cx).daemon_disconnected)
            .map(|warning| {
                runner_app::ui::notice_banner(warning, runner_app::ui::Tone::Warning).child(
                    Button::new("dismiss-chat-warning", crate::i18n::text("ui.dismiss"))
                        .size(ButtonSize::Sm)
                        .variant(runner_app::ui::ButtonVariant::Ghost)
                        .on_press(move |_, cx| {
                            warning_root.update(cx, |this, cx| {
                                this.chat_warning = None;
                                cx.notify();
                            });
                        }),
                )
            });

        let Some(layout) = self.front_page_layout().cloned() else {
            let launcher = self.render_launcher(None, true, window, cx);
            let close = self.launcher_page_closable(cx).then(|| {
                let root = cx.entity();
                div()
                    .debug_selector(|| "LAUNCHER_PAGE_CLOSE".into())
                    .absolute()
                    .top(rems(8. / 16.))
                    .right(rems(8. / 16.))
                    .child(
                        IconButton::new("launcher-page-close", "close.svg")
                            .size(IconButtonSize::Sm)
                            .tooltip(crate::i18n::text("keymap.close-pane.title"))
                            .on_press(move |window, cx| {
                                root.update(cx, |root, cx| root.close_launcher_page(window, cx));
                            }),
                    )
            });
            return div()
                .debug_selector(|| "LAUNCHER_PAGE".into())
                .track_focus(&self.chat_focus)
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_col()
                .rounded(rems(12. / 16.))
                .border_1()
                .border_color(theme::border())
                .bg(theme::bg())
                .overflow_hidden()
                .children(
                    (self.route != AppRoute::Settings)
                        .then(|| self.render_daemon_banner(cx))
                        .flatten(),
                )
                .children(error_banner)
                .children(warning_banner)
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h(px(0.))
                        .flex()
                        .flex_col()
                        .child(launcher)
                        .children(close),
                )
                .into_any_element();
        };
        if self.remote_client.is_local()
            && layout.all_session_ids().iter().any(|id| {
                self.app_store
                    .read(cx)
                    .bridge
                    .session(id)
                    .is_some_and(|terminal| !terminal.control().desktop())
            })
        {
            if let Err(error) = self.ensure_owned_active_tab_attached(window, cx) {
                self.chat_error = Some(error.to_string().into());
            }
        }
        let group_control = self.remote_client.is_local() && self.tab_control_covered(&layout, cx);
        let show_start = start_surface_visible(Some(&layout));
        let grouped = layout.root.leaves().len() > 1;
        let terminal_tab = self.active_tab_is_terminal(cx);
        if self.remote_client.is_local() {
            self.prune_pane_state(&layout);
        }
        prune_maximized(&mut self.maximized_panes, &layout);
        let pane_tree = match maximized_leaf(&self.maximized_panes, &layout).cloned() {
            Some(leaf) => self.render_pane_node(&PaneNode::Leaf(leaf), &layout, cx),
            None => self.render_pane_node(&layout.root, &layout, cx),
        };
        let tab_archiving =
            split_tab_archiving(&layout, |id| self.sidebar_archiving_session(id, cx));
        let tab_body = div()
            .id("chat-tab-body")
            .relative()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .flex_col()
                    .child(pane_tree),
            )
            .on_drop(cx.listener(|this, drag: &PaneDrag, window, cx| {
                this.apply_pane_drop(drag, window, cx);
            }))
            .when(tab_archiving, |body| {
                body.child(SessionOverlay::transition(
                    format!("archiving-tab-{}", layout.id),
                    SessionOverlayKind::Archiving,
                ))
            });
        let tab_body = if group_control {
            crate::terminal::control::cover(
                tab_body,
                "take-back-tab",
                &self
                    .control_tab_session_ids(&layout, cx)
                    .iter()
                    .filter_map(|id| self.attached.get(id))
                    .collect::<Vec<_>>(),
                crate::terminal::control::GroupKind::Tab,
            )
        } else {
            tab_body
        };
        let chat_column = div()
            .relative()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .children(
                (self.route != AppRoute::Settings)
                    .then(|| self.render_daemon_banner(cx))
                    .flatten(),
            )
            .children(error_banner)
            .children(warning_banner)
            .child(tab_body);
        let panel_open = self.remote_client.is_local()
            && !show_start
            && side_panel_open(self.settings(cx).chat_panel_open, terminal_tab);
        let (panel_visibility, panel_animating) = self.chat_panel_visibility.animate_to(
            if panel_open { 1. } else { 0. },
            Instant::now(),
            Duration::from_millis(CHAT_PANEL_TRANSITION_MS),
        );
        if panel_animating {
            window.request_animation_frame();
        }
        self.reset_chat_panel_prompt(
            self.active_chat_detail
                .as_ref()
                .map(|chat| chat.session_id.clone()),
        );
        let side_panel = self.cached_region(
            "side-panel",
            gpui::StyleRefinement::default()
                .w(rems(
                    self.settings(cx).chat_panel_width * panel_visibility / 16.,
                ))
                .h_full()
                .flex_none(),
            move |root, _, cx| {
                root.render_chat_side_panel(
                    root.active_chat_detail.as_ref(),
                    root.session_key_copy.clone(),
                    panel_visibility,
                    panel_open || panel_animating,
                    panel_open && !panel_animating,
                    cx,
                )
            },
            cx,
        );
        let panel_resize = (panel_open || panel_animating).then(|| {
            let zoom = self.settings(cx).app_zoom;
            let panel_width = self.settings(cx).chat_panel_width * panel_visibility * zoom;
            resize_strip(
                "chat-panel-resize",
                ResizeAxis::Columns,
                self.chat_panel_resizing,
                zoom,
                None,
                false,
            )
            .id("chat-panel-resize")
            .when(cfg!(test), |handle| {
                handle.debug_selector(|| "CHAT_PANEL_GUTTER".into())
            })
            .map(|handle| {
                #[cfg(any(windows, target_os = "linux"))]
                let handle = handle.occlude();
                handle
            })
            .absolute()
            .top_0()
            .right(px(panel_width - resize_strip_inset(zoom)))
            .on_drag(
                ChatPanelResizeDrag,
                |drag: &ChatPanelResizeDrag, _, _, cx: &mut App| cx.new(|_| drag.clone()),
            )
        });

        div()
            .relative()
            .when(grouped, |surface| surface.key_context("ChatSplit"))
            .track_focus(&self.chat_focus)
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .child(chat_column)
            .children(
                (panel_open || panel_animating)
                    .then(|| div().w(rems(6. / 16.)).h_full().flex_none()),
            )
            .child(side_panel)
            .children(panel_resize)
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.finish_split_resize(cx);
                    this.finish_terminal_drawer_resize(cx);
                    this.finish_chat_panel_resize(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.finish_split_resize(cx);
                    this.finish_terminal_drawer_resize(cx);
                    this.finish_chat_panel_resize(cx);
                }),
            )
            .on_drag_move::<ChatPanelResizeDrag>(cx.listener(
                |this, event: &DragMoveEvent<ChatPanelResizeDrag>, _, cx| {
                    let width = f32::from(event.bounds.right() - event.event.position.x)
                        / this.settings(cx).app_zoom;
                    let width = app_settings::clamp_chat_panel_width(width);
                    if !this.chat_panel_resizing {
                        this.chat_panel_resizing = true;
                        cx.notify();
                    }
                    this.update_app_settings(cx, false, |settings| {
                        if settings.chat_panel_width == width {
                            return false;
                        }
                        settings.chat_panel_width = width;
                        true
                    });
                },
            ))
            .on_drop(cx.listener(|this, _: &ChatPanelResizeDrag, _, cx| {
                this.finish_chat_panel_resize(cx);
                this.save_settings(cx);
            }))
            .into_any_element()
    }
}
