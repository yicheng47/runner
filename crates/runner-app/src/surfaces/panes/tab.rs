use super::*;

impl NativeRoot {
    pub(crate) fn render_archived_chat(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.sync_chat_panel(cx);
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
        let session_id = detail.session_id.clone();
        let is_chat = detail.agent_runtime != "shell";
        let back_label = if detail.handle.is_some() {
            crate::i18n::text("ui.back_to_role")
        } else {
            crate::i18n::text("ui.back_to_roles")
        };
        let handle = detail.handle.clone();
        let archived_menu = is_chat.then(|| {
            let menu = match &self.archived_title_menu {
                Some((id, menu)) if id == &session_id => menu.clone(),
                _ => {
                    let menu = cx.new(|menu_cx| {
                        PopoverMenu::new(
                            SharedString::from(format!("archived-title-{session_id}")),
                            menu_cx.focus_handle(),
                            Vec::new(),
                            Rc::new(|_, _, _| {}),
                            menu_cx,
                        )
                        .min_width(px(320.))
                        .single_line_labels()
                    });
                    self.archived_title_menu = Some((session_id.clone(), menu.clone()));
                    menu
                }
            };
            let root_weak = cx.weak_entity();
            let menu_weak = menu.downgrade();
            let sid = session_id.clone();
            let header_renderer: runner_app::ui::menu::HeaderRenderer =
                Rc::new(move |_: &mut Window, cx: &mut App| {
                    let menu_weak = menu_weak.clone();
                    let sid = sid.clone();
                    root_weak
                        .update(cx, |this, cx| {
                            this.render_pane_info_card_for_session(&sid, menu_weak, cx)
                        })
                        .unwrap_or_else(|_| div().into_any_element())
                });
            let trigger_label = label.clone();
            menu.update(cx, |menu, menu_cx| {
                menu.set_header_renderer(Some(header_renderer), menu_cx);
                menu.set_trigger_label(
                    runner_app::ui::MenuTriggerLabel {
                        text: trigger_label.into(),
                        color: theme::text(),
                    },
                    menu_cx,
                );
            });
            menu
        });
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
            .when(
                !(self.chat_panel_is_open(cx)
                    && side_panel_available(self.settings(cx), PanelSurface::Chat)),
                |header| header.rounded_tr(rems(11. / 16.)),
            )
            .children(sidebar_toggle)
            .children(sidebar_divider)
            .child(
                svg()
                    .path("terminal.svg")
                    .size(rems(15. / 16.))
                    .flex_none()
                    .text_color(theme::accent()),
            )
            .child(if let Some(menu) = archived_menu {
                div()
                    .min_w(px(0.))
                    .h(rems(28. / 16.))
                    .flex()
                    .items_center()
                    .debug_selector({
                        let label = label.clone();
                        move || format!("PANE_TITLE:{label}")
                    })
                    .child(menu)
            } else {
                div()
                    .min_w(px(0.))
                    .truncate()
                    .text_size(theme::text_body())
                    .font_weight(FontWeight::MEDIUM)
                    .child(label)
            })
            .child(
                Button::new("archived-chat-back", back_label)
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .on_press(move |window, cx| {
                        let handle = handle.clone();
                        root.update(cx, |this, root_cx| {
                            this.archived_chat_detail = None;
                            this.archived_title_menu = None;
                            if let Some(handle) = handle {
                                this.open_role_detail(handle, window, root_cx);
                            } else {
                                this.open_roles(window, root_cx);
                            }
                        });
                    }),
            )
            .child(div().min_w(px(0.)).flex_1())
            .when(
                !self.chat_panel_is_open(cx)
                    && side_panel_available(self.settings(cx), PanelSurface::Chat),
                |header| {
                    header.child(
                        IconButton::new("open-archived-chat-panel", "panel-right-hidden.svg")
                            .tooltip(crate::i18n::text("ui.open_side_panel"))
                            .on_press(move |window, cx| {
                                panel_root.update(cx, |this, cx| {
                                    this.set_chat_panel_open(true, cx);
                                    this.focus_chat_panel_shown(window, cx);
                                    cx.notify();
                                });
                            }),
                    )
                },
            );
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
        let panel_open = self.chat_panel_is_open(cx)
            && side_panel_available(self.settings(cx), PanelSurface::Chat);
        if !panel_open && self.chat_panel_maximized {
            self.chat_panel_maximized = false;
        }
        let panel_maximized = panel_open && self.chat_panel_maximized;
        let panel_layout = self.chat_panel_layout(self.archived_chat_detail.as_ref(), 1, cx);
        let panel_sheet = panel_open && !panel_maximized && panel_layout.is_sheet();
        let now = Instant::now();
        let (panel_visibility, panel_animating) = if panel_maximized || panel_sheet {
            (1., false)
        } else if panel_layout.is_sheet() {
            // Collapsed narrow sheet uses zero docked width rather than animating a docked rail.
            self.chat_panel_visibility = SidebarVisibilityTransition::new(false);
            (0., false)
        } else {
            self.chat_panel_visibility.animate_to(
                if panel_open { 1. } else { 0. },
                now,
                Duration::from_millis(CHAT_PANEL_TRANSITION_MS),
            )
        };
        let (sheet_progress, sheet_animating) = if panel_sheet {
            self.chat_sheet_anim.animate_to(
                1.,
                now,
                Duration::from_millis(CHAT_PANEL_TRANSITION_MS),
            )
        } else {
            self.chat_sheet_anim = SidebarVisibilityTransition::new(false);
            (0., false)
        };
        let target_width = panel_layout.width.round();
        let width_mode = (
            panel_layout.widened.is_some(),
            panel_layout.widened.is_some_and(|w| w.tree),
        );
        let prev_mode = self.chat_panel_width_mode;
        self.chat_panel_width_mode = Some(width_mode);
        let mode_changed = prev_mode.is_some_and(|prev| prev != width_mode);
        let (animated_width, width_animating) = if panel_sheet {
            // Keep the docked width tracker at the reserved docked width (e.g.
            // 300 px for a split auto-sheet, 0 px for a narrow sheet) so closing
            // a sheet back to docked never starts a docked animation from the
            // full sheet width and squeezes the terminal panes.
            let docked = panel_layout.reserved_docked_width.round();
            self.chat_panel_width_anim = SidebarVisibilityTransition::at(docked);
            (target_width, false)
        } else if std::mem::take(&mut self.chat_panel_width_snap)
            || panel_maximized
            || self.chat_panel_resizing
            || !panel_open
            || (!mode_changed && self.chat_panel_width_anim.started_at.is_none())
        {
            self.chat_panel_width_anim = SidebarVisibilityTransition::at(target_width);
            (target_width, false)
        } else {
            self.chat_panel_width_anim.animate_to(
                target_width,
                now,
                Duration::from_millis(CHAT_PANEL_TRANSITION_MS),
            )
        };
        if panel_animating || sheet_animating || width_animating {
            let weak = cx.weak_entity();
            window.on_next_frame(move |_, cx| {
                let _ = weak.update(cx, |_, cx| cx.notify());
            });
            window.request_animation_frame();
        }
        if !panel_open && !panel_animating {
            self.chat_files.hide(cx);
        }
        let side_panel = self.render_chat_side_panel(
            self.archived_chat_detail.as_ref(),
            PanelSurface::Chat,
            panel_layout,
            animated_width,
            if panel_sheet { 1. } else { panel_visibility },
            panel_sheet || panel_open || panel_animating,
            panel_sheet || (panel_open && !panel_animating),
            window,
            cx,
        );
        let dismiss_root = cx.entity();
        let (docked_panel, sheet_overlay) = if panel_sheet {
            (
                None,
                Some(side_panel::overlay_sheet(
                    "archived-chat-panel-sheet",
                    side_panel,
                    panel_layout.width,
                    sheet_progress,
                    Rc::new(move |_, cx| {
                        dismiss_root.update(cx, |this, cx| {
                            this.set_chat_panel_open(false, cx);
                            cx.notify();
                        });
                    }),
                )),
            )
        } else {
            (Some(side_panel), None)
        };
        div()
            .relative()
            .track_focus(&self.chat_focus)
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _window, cx| {
                if panel_sheet
                    && event.keystroke.key == "escape"
                    && !event.keystroke.modifiers.modified()
                {
                    cx.stop_propagation();
                    this.set_chat_panel_open(false, cx);
                    cx.notify();
                }
            }))
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .child(self.chat_files.measure(self.settings(cx).app_zoom))
            .when(!panel_maximized, |el| el.child(chat_column))
            .children(docked_panel)
            .children(sheet_overlay)
            .into_any_element()
    }

    pub(crate) fn render_active_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.sync_chat_panel(cx);
        self.prune_launchers(cx);
        self.ensure_active_chat_detail(cx);
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

        let layout_opt = self.front_page_layout().cloned();
        let restore_fallback_banner = self
            .chat_restore_fallback
            .as_ref()
            .filter(|_| self.remote_client.is_local())
            .filter(|_| !self.app_store.read(cx).daemon_disconnected)
            .filter(|fallback| {
                layout_opt.as_ref().is_some_and(|layout| {
                    layout.root.leaves().iter().any(|leaf| {
                        leaf.session_id.as_deref() == Some(fallback.session_id.as_str())
                    })
                })
            })
            .map(|fallback| {
                let confirm_root = cx.entity();
                let dismiss_root = cx.entity();
                let busy = self.chat_transitions.contains_key(&fallback.session_id);
                crate::surfaces::worktree_restore::render_restore_fallback_banner(
                    "chat",
                    &fallback.prompt,
                    busy,
                    move |window, cx| {
                        confirm_root.update(cx, |this, cx| {
                            this.confirm_chat_restore_in_main(window, cx);
                        });
                    },
                    move |_, cx| {
                        dismiss_root.update(cx, |this, cx| {
                            this.dismiss_chat_restore_fallback(cx);
                        });
                    },
                )
            });

        let Some(layout) = layout_opt else {
            let setup_visible = self.setup_visible(cx);
            let content = if setup_visible {
                self.render_setup(window, cx)
            } else {
                self.render_launcher(None, true, window, cx)
                    .into_any_element()
            };
            let close = (!setup_visible && self.launcher_page_closable(cx)).then(|| {
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
                        .child(content)
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
            .children(restore_fallback_banner)
            .children(warning_banner)
            .child(tab_body);
        let (surface, panel_available) = self.active_chat_panel_surface(cx);
        let local = self.remote_client.is_local();
        let split_columns = side_panel::split_columns_for_layout(
            Some(&layout),
            maximized_leaf(&self.maximized_panes, &layout).is_some(),
        );
        let panel_open = !show_start && panel_available && self.chat_panel_is_open(cx);
        if local && !panel_open && self.chat_panel_maximized {
            self.chat_panel_maximized = false;
        }
        let panel_maximized = local && panel_open && self.chat_panel_maximized;
        let panel_layout = self.chat_panel_layout(
            local.then_some(self.active_chat_detail.as_ref()).flatten(),
            split_columns,
            cx,
        );
        let panel_sheet = panel_open && !panel_maximized && panel_layout.is_sheet();
        let now = Instant::now();
        let (panel_visibility, panel_animating) = if panel_maximized || panel_sheet {
            (1., false)
        } else if panel_layout.is_sheet() {
            self.chat_panel_visibility = SidebarVisibilityTransition::new(false);
            (0., false)
        } else {
            self.chat_panel_visibility.animate_to(
                if panel_open { 1. } else { 0. },
                now,
                Duration::from_millis(CHAT_PANEL_TRANSITION_MS),
            )
        };
        let (sheet_progress, sheet_animating) = if panel_sheet {
            self.chat_sheet_anim.animate_to(
                1.,
                now,
                Duration::from_millis(CHAT_PANEL_TRANSITION_MS),
            )
        } else {
            self.chat_sheet_anim = SidebarVisibilityTransition::new(false);
            (0., false)
        };
        let target_width = panel_layout.width.round();
        let width_mode = (
            panel_layout.widened.is_some(),
            panel_layout.widened.is_some_and(|w| w.tree),
        );
        let prev_mode = self.chat_panel_width_mode;
        self.chat_panel_width_mode = Some(width_mode);
        let mode_changed = prev_mode.is_some_and(|prev| prev != width_mode);
        let (animated_width, width_animating) = if panel_sheet {
            // Keep the docked width tracker at the reserved docked width (e.g.
            // 300 px for a split auto-sheet, 0 px for a narrow sheet) so closing
            // a sheet back to docked never starts a docked animation from the
            // full sheet width and squeezes the terminal panes.
            let docked = panel_layout.reserved_docked_width.round();
            self.chat_panel_width_anim = SidebarVisibilityTransition::at(docked);
            (target_width, false)
        } else if std::mem::take(&mut self.chat_panel_width_snap)
            || panel_maximized
            || self.chat_panel_resizing
            || !panel_open
            || (!mode_changed && self.chat_panel_width_anim.started_at.is_none())
        {
            self.chat_panel_width_anim = SidebarVisibilityTransition::at(target_width);
            (target_width, false)
        } else {
            self.chat_panel_width_anim.animate_to(
                target_width,
                now,
                Duration::from_millis(CHAT_PANEL_TRANSITION_MS),
            )
        };
        if panel_animating || sheet_animating || width_animating {
            let weak = cx.weak_entity();
            window.on_next_frame(move |_, cx| {
                let _ = weak.update(cx, |_, cx| cx.notify());
            });
            window.request_animation_frame();
        }
        if !panel_open && !panel_animating {
            self.chat_files.hide(cx);
        }
        let panel_region_style = if panel_maximized {
            gpui::StyleRefinement::default()
                .flex_1()
                .min_w(px(0.))
                .h_full()
        } else if panel_sheet {
            gpui::StyleRefinement::default()
                .w(rems(panel_layout.width / 16.))
                .h_full()
                .flex_none()
        } else {
            gpui::StyleRefinement::default()
                .w(rems(animated_width * panel_visibility / 16.))
                .h_full()
                .flex_none()
        };
        let side_panel = self.cached_region(
            "side-panel",
            panel_region_style,
            move |root, window, cx| {
                root.render_chat_side_panel(
                    root.active_chat_detail.as_ref(),
                    surface,
                    panel_layout,
                    animated_width,
                    panel_visibility,
                    panel_open || panel_animating,
                    panel_open && !panel_animating,
                    window,
                    cx,
                )
            },
            cx,
        );
        let widened = panel_layout.widened;
        let docked_open = !panel_maximized && !panel_sheet && (panel_open || panel_animating);
        let panel_resize = docked_open.then(|| {
            let zoom = self.settings(cx).app_zoom;
            let panel_width = animated_width * panel_visibility * zoom;
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
        let sheet_dismiss_root = cx.entity();
        let (docked_panel, sheet_overlay) = if panel_sheet {
            (
                None,
                Some(side_panel::overlay_sheet(
                    "chat-panel-sheet",
                    side_panel,
                    panel_layout.width,
                    sheet_progress,
                    Rc::new(move |window, cx| {
                        sheet_dismiss_root.update(cx, |this, cx| {
                            this.set_chat_panel_open(false, cx);
                            this.focus_active_terminal(window, cx);
                            cx.notify();
                        });
                    }),
                )),
            )
        } else {
            (Some(side_panel), None)
        };
        let reserved_spacer = (panel_sheet && panel_layout.reserved_docked_width > 0.).then(|| {
            div()
                .w(rems((panel_layout.reserved_docked_width + 6.) / 16.))
                .h_full()
                .flex_none()
        });

        div()
            .relative()
            .when(grouped, |surface| surface.key_context("ChatSplit"))
            .track_focus(&self.chat_focus)
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if panel_sheet
                    && event.keystroke.key == "escape"
                    && !event.keystroke.modifiers.modified()
                {
                    cx.stop_propagation();
                    this.set_chat_panel_open(false, cx);
                    this.focus_active_terminal(window, cx);
                    cx.notify();
                }
            }))
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .child(self.chat_files.measure(self.settings(cx).app_zoom))
            .when(!panel_maximized, |el| el.child(chat_column))
            .children(docked_open.then(|| div().w(rems(6. / 16.)).h_full().flex_none()))
            .children(reserved_spacer)
            .children(docked_panel)
            .children(panel_resize)
            .children(sheet_overlay)
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
                move |this, event: &DragMoveEvent<ChatPanelResizeDrag>, _, cx| {
                    let width = f32::from(event.bounds.right() - event.event.position.x)
                        / this.settings(cx).app_zoom;
                    if !this.chat_panel_resizing {
                        this.chat_panel_resizing = true;
                        cx.notify();
                    }
                    // With a file or diff open the drag sizes the viewer beside the tree/list.
                    if let Some(widened) = widened {
                        let tree = if widened.tree { widened.tree_width } else { 0. };
                        let width = app_settings::clamp_file_viewer_width(width - tree);
                        let is_changes = side_panel::shown(
                            surface,
                            this.settings(cx),
                            &this.chat_surfaces.selection,
                        ) == Some(side_panel::PanelViewId::Changes);
                        this.update_app_settings(cx, false, |settings| {
                            let target = if is_changes {
                                &mut settings.changes_viewer_width
                            } else {
                                &mut settings.file_viewer_width
                            };
                            if *target == width {
                                return false;
                            }
                            *target = width;
                            true
                        });
                        return;
                    }
                    let width = app_settings::clamp_chat_panel_width(width);
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
