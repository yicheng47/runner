use super::*;

/// The title trigger's padding, gap and chevron beside its text.
const TITLE_TRIGGER_CHROME: f32 = 28.;

impl NativeRoot {
    pub(crate) fn render_pane(
        &mut self,
        leaf: &PaneLeaf,
        layout: &PaneLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        #[cfg(test)]
        crate::render_counts::count_render(&format!("pane:{}", leaf.id), cx);
        let focused = layout.focused_pane_id == leaf.id;
        let grouped = layout.root.leaves().len() > 1;
        let maximized =
            maximized_leaf(&self.maximized_panes, layout).is_some_and(|shown| shown.id == leaf.id);
        let pane_id = leaf.id.clone();
        let pane_id_for_focus = pane_id.clone();
        let entry = leaf
            .session_id
            .as_deref()
            .and_then(|session_id| self.front_page_session(session_id, cx));
        let secondary = entry.as_ref().map(|entry| {
            if self.remote_client.is_active() {
                Default::default()
            } else {
                self.cached_chat_secondary_state(&entry.session_id)
            }
        });
        let pane_session_for_focus = entry.as_ref().map(|entry| entry.session_id.clone());
        let close_root = cx.entity();
        let header = {
            let split_menu = self
                .front_page_client(cx)
                .supports(if self.active_tab_is_terminal(cx) {
                    &["layout", "startShell"]
                } else {
                    &["layout"]
                })
                .then(|| self.split_menu(SplitMenuSurface::Pane, &layout.id, &pane_id, cx));
            let close_pane_id = pane_id.clone();
            let close_behavior =
                pane_close_behavior(entry.as_ref().map(|entry| entry.agent_runtime.as_str()));
            let close_session_id = entry.as_ref().map(|entry| entry.session_id.clone());
            let rename_input = entry.as_ref().and_then(|entry| {
                self.pane_rename
                    .as_ref()
                    .filter(|rename| rename.session_id == entry.session_id)
                    .map(|rename| rename.input.clone())
            });
            let mark_icon =
                pane_identity_icon(entry.as_ref().map(|entry| entry.agent_runtime.as_str()));
            let mark_live = entry
                .as_ref()
                .is_some_and(|entry| entry.status == SessionStatus::Running);
            let mark_label = entry
                .as_ref()
                .map(|entry| {
                    entry.label(
                        self.remote_client
                            .is_local()
                            .then(|| self.attached_title(&entry.session_id, cx))
                            .flatten()
                            .as_deref(),
                    )
                })
                .unwrap_or_else(|| crate::i18n::text("launcher.new").into());
            let drag_pane = pane_id.clone();
            let drag_session = entry.as_ref().map(|entry| entry.session_id.clone());
            let grip = grouped.then(|| {
                div()
                    .id(SharedString::from(format!("pane-grip-{pane_id}")))
                    .debug_selector({
                        let pane_id = pane_id.clone();
                        move || format!("PANE_DRAG_GRIP:{pane_id}")
                    })
                    .flex_none()
                    .mr(rems(4. / 16.))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            if let Some(session_id) = drag_session.as_deref() {
                                this.focus_terminal(&drag_pane, session_id, window, cx);
                            } else {
                                this.focus_pane(&drag_pane, cx);
                            }
                        }),
                    )
                    .child(
                        svg()
                            .path("grip-vertical.svg")
                            .size(rems(14. / 16.))
                            .text_color(theme::faint())
                            .group_hover("pane-header", |grip| grip.text_color(theme::muted()))
                            .hover(|grip| grip.text_color(theme::text())),
                    )
                    .when(self.front_page_client(cx).supports(&["layout"]), |grip| {
                        grip.cursor(CursorStyle::OpenHand).on_drag(
                            PaneDrag {
                                tab_id: layout.id.clone(),
                                pane_id: pane_id.clone(),
                                label: mark_label,
                                icon: mark_icon,
                                live: mark_live,
                            },
                            |drag: &PaneDrag, _, _, cx: &mut App| cx.new(|_| drag.clone()),
                        )
                    })
            });
            let mark = div()
                .id(SharedString::from(format!("pane-mark-{pane_id}")))
                .debug_selector({
                    let pane_id = pane_id.clone();
                    move || format!("PANE_RUNTIME_MARK:{pane_id}")
                })
                .flex_none()
                .child(mark_icon.render(
                    rems(15. / 16.),
                    mark_icon.color(
                        if entry.is_some() && focused {
                            theme::accent()
                        } else {
                            theme::faint()
                        },
                        mark_live,
                    ),
                    mark_live,
                ));
            let (identity, lifecycle_control) = if let Some(entry) = entry.as_ref() {
                let session_id = entry.session_id.clone();
                let live = self
                    .remote_client
                    .is_local()
                    .then(|| self.attached_title(&session_id, cx))
                    .flatten();
                let label = entry.label(live.as_deref());
                let shell = entry.agent_runtime == "shell";
                let status = self
                    .front_page_status(&entry.session_id, cx)
                    .filter(|_| !shell)
                    .map(|mut status| {
                        if let Some(transition) = self
                            .chat_transitions
                            .get(&entry.session_id)
                            .filter(|_| self.remote_client.is_local())
                        {
                            status.kind = if transition.kind == TransitionKind::Resuming {
                                runner_app::ui::agent_status::StatusKind::Resuming
                            } else {
                                runner_app::ui::agent_status::StatusKind::Starting
                            };
                            status.estimated = false;
                        }
                        status
                    });
                let renaming = rename_input.is_some();
                let name = if let Some(input) = rename_input {
                    div()
                        .w_full()
                        .min_w(px(0.))
                        .child(input)
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                            match pane_rename_key(&event.keystroke.key) {
                                Some(PaneRenameKey::Submit) => {
                                    cx.stop_propagation();
                                    this.submit_pane_rename(window, cx);
                                }
                                Some(PaneRenameKey::Cancel) => {
                                    cx.stop_propagation();
                                    this.cancel_pane_rename(window, cx);
                                }
                                None => {}
                            }
                        }))
                        .into_any_element()
                } else {
                    let title_color = if entry.status != SessionStatus::Running {
                        theme::faint()
                    } else if focused {
                        theme::text()
                    } else {
                        theme::muted()
                    };
                    let title_selector = {
                        let label = label.clone();
                        move || format!("PANE_TITLE:{label}")
                    };
                    let disabled = self.session_lifecycle_disabled(&entry.session_id, cx)
                        || secondary.as_ref().is_some_and(|state| state.secondary);
                    match self.pane_title_menu(
                        entry,
                        &layout.id,
                        label.clone().into(),
                        title_color,
                        disabled,
                        cx,
                    ) {
                        Some(menu) => div()
                            .id(SharedString::from(format!("pane-name-{session_id}")))
                            .debug_selector(title_selector)
                            .w_full()
                            .min_w(px(0.))
                            .flex()
                            .child(menu)
                            .into_any_element(),
                        None => div()
                            .id(SharedString::from(format!("pane-name-{session_id}")))
                            .debug_selector(title_selector)
                            .w_full()
                            .min_w(px(0.))
                            .truncate()
                            .text_size(theme::text_ui())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(title_color)
                            .child(label.clone())
                            .into_any_element(),
                    }
                };
                let account = entry
                    .local
                    .as_ref()
                    .and_then(|entry| self.claude_account_details(entry, cx))
                    .map(|(name, _, _)| name);
                let lifecycle = status
                    .as_ref()
                    .and_then(|status| {
                        pane_lifecycle_action(
                            &entry.agent_runtime,
                            entry.status == SessionStatus::Running,
                            status.kind,
                            self.session_lifecycle_disabled(&entry.session_id, cx),
                            secondary.as_ref().is_some_and(|state| state.secondary),
                        )
                    })
                    .filter(|action| {
                        self.front_page_client(cx).supports(&[match action {
                            SessionControlKind::Stop => "stop",
                            _ => "resume",
                        }])
                    })
                    .map(|action| {
                        (
                            action,
                            start_action_shortcut(
                                if action == SessionControlKind::Stop {
                                    "stop-session"
                                } else {
                                    "resume-session"
                                },
                                &self.settings(cx).keymap_overrides,
                            ),
                        )
                    });
                let lifecycle_control = lifecycle.map(|(action, shortcut)| {
                    let target = pane_id.clone();
                    let target_session = entry.session_id.clone();
                    let root = cx.entity();
                    runner_app::ui::agent_status::pane_lifecycle_control(
                        SharedString::from(format!("lifecycle-{pane_id}")),
                        action,
                        shortcut,
                        move |window, cx| {
                            root.update(cx, |this, cx| {
                                this.focus_pane(&target, cx);
                                this.activate_primary_pane_action(&target_session, window, cx);
                            });
                        },
                        window,
                        cx,
                    )
                });
                let lifecycle_control = status.is_some().then(|| {
                    div().flex_none().ml_2().mr_1().child(
                        runner_app::ui::agent_status::pane_lifecycle_slot(lifecycle_control),
                    )
                });
                let target = pane_id.clone();
                let identity = gpui::container_query(move |size, window, cx| {
                    let zoom = f32::from(window.rem_size()) / 16.;
                    let text_width = |text: &str, font_size: f32, weight| {
                        let mut style = window.text_style();
                        style.font_weight = weight;
                        f32::from(
                            window
                                .text_system()
                                .shape_line(
                                    text.to_owned().into(),
                                    px(font_size * zoom),
                                    &[style.to_run(text.len())],
                                    None,
                                )
                                .width
                                .ceil(),
                        )
                    };
                    let name_preferred =
                        text_width(&label, 12., FontWeight::MEDIUM) + TITLE_TRIGGER_CHROME * zoom;
                    let name_preferred = if renaming {
                        name_preferred.max(144. * zoom)
                    } else {
                        name_preferred
                    };
                    let account_preferred = account.as_ref().map_or(0., |account| {
                        (text_width(account, 11., FontWeight::NORMAL) + 16. * zoom).min(160. * zoom)
                    });
                    let fixed = (15.
                        + if grouped { 18. } else { 0. }
                        + 8.
                        + if account.is_some() { 8. } else { 0. }
                        + if status.is_some() { 8. } else { 0. })
                        * zoom;
                    let budget = (f32::from(size.width) - fixed - 1.).max(0.);
                    let label_width = status
                        .as_ref()
                        .map_or(0., |status| status.island_label_width(window));
                    let min_name = name_preferred.min((28. + TITLE_TRIGGER_CHROME) * zoom);
                    let label_visible = status.is_some() && budget >= min_name + label_width;
                    let detail_visible = label_visible
                        && status
                            .as_ref()
                            .is_some_and(|status| status.has_island_detail())
                        && budget >= name_preferred + account_preferred + 180. * zoom;
                    let status_width = if detail_visible {
                        180. * zoom
                    } else if label_visible {
                        label_width
                    } else if status.is_some() {
                        12. * zoom
                    } else {
                        0.
                    };
                    let account_visible = account.is_some()
                        && !renaming
                        && budget - status_width - min_name >= 24. * zoom;
                    let account_width = if account_visible {
                        account_preferred.min((budget - status_width - min_name).max(16. * zoom))
                    } else {
                        0.
                    };
                    let name_width =
                        name_preferred.min((budget - status_width - account_width).max(0.));
                    div()
                        .debug_selector(|| "ISLAND_HEADER_IDENTITY".into())
                        .size_full()
                        .min_w(px(0.))
                        .flex()
                        .items_center()
                        .children(grip)
                        .child(mark)
                        .child(
                            div()
                                .ml_2()
                                .flex_none()
                                .w(px(name_width))
                                .overflow_hidden()
                                .child(name),
                        )
                        .children(account.filter(|_| account_visible).map(|account| {
                            account_chip(account)
                                .ml_2()
                                .min_w(px(0.))
                                .w(px(account_width))
                        }))
                        .child(div().flex_1().min_w(px(0.)))
                        .children(status.map(|status| {
                            runner_app::ui::agent_status::pane_status(
                                status,
                                (label_visible, detail_visible),
                                SharedString::from(format!("status-{target}")),
                                window,
                                cx,
                            )
                        }))
                })
                .flex_1()
                .min_w(px(0.))
                .into_any_element();
                (identity, lifecycle_control)
            } else {
                (
                    div()
                        .min_w(px(0.))
                        .flex_1()
                        .flex()
                        .items_center()
                        .children(grip)
                        .child(mark)
                        .child(
                            div()
                                .ml_2()
                                .text_size(theme::text_ui())
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme::faint())
                                .child(crate::i18n::text("launcher.new")),
                        )
                        .into_any_element(),
                    None,
                )
            };
            div()
                .flex_none()
                .h(rems(40. / 16.))
                .pl(rems(if grouped { 8. } else { 14. } / 16.))
                .pr(rems(8. / 16.))
                .group("pane-header")
                .flex()
                .items_center()
                .map(|element| {
                    #[cfg(test)]
                    let element = crate::theme_snapshot::record_fill("PANE_IDENTITY_LINE", element);
                    element.debug_selector({
                        let id = leaf.session_id.clone().unwrap_or_else(|| pane_id.clone());
                        move || format!("PANE_HEADER:{id}")
                    })
                })
                .when(
                    (maximized || layout.root.leaves()[0].id == pane_id)
                        && self.front_page_sidebar_collapsed()
                        && cfg!(target_os = "macos"),
                    |header| {
                        header
                            .pl(px(self.workspace_titlebar_padding(window, cx)))
                            .children(self.render_collapsed_titlebar_spacer())
                    },
                )
                .child(identity)
                .children(lifecycle_control)
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .when(cfg!(test), |actions| {
                            actions.debug_selector(|| "PANE_HEADER_ACTIONS".into())
                        })
                        .children(split_menu)
                        .children(grouped.then(|| {
                            let root = cx.entity();
                            let target = pane_id.clone();
                            let label = crate::i18n::text(if maximized {
                                "ui.restore_split"
                            } else {
                                "ui.maximize_pane"
                            });
                            let tooltip = keymap::effective_binding(
                                "toggle-maximize-pane",
                                &self.settings(cx).keymap_overrides,
                            )
                            .map_or_else(
                                || label.to_owned(),
                                |combo| format!("{label} · {}", keymap::format_combo(&combo)),
                            );
                            div()
                                .flex_none()
                                .debug_selector({
                                    let pane_id = pane_id.clone();
                                    move || format!("PANE_MAXIMIZE:{pane_id}")
                                })
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .child(
                                    IconButton::new(
                                        SharedString::from(format!("maximize-pane-{pane_id}")),
                                        if maximized {
                                            "minimize-2.svg"
                                        } else {
                                            "maximize-2.svg"
                                        },
                                    )
                                    .size(IconButtonSize::Sm)
                                    .when(maximized, |button| {
                                        button.variant(ButtonVariant::Secondary)
                                    })
                                    .tooltip(tooltip)
                                    .on_press(
                                        move |window, cx| {
                                            root.update(cx, |this, cx| {
                                                this.toggle_maximize_pane(&target, window, cx)
                                            });
                                        },
                                    ),
                                )
                        }))
                        .children(
                            self.front_page_client(cx)
                                .supports(match close_behavior {
                                    PaneCloseBehavior::CloseTerminal => &["close", "layout"],
                                    PaneCloseBehavior::ArchiveChat => &["archive", "layout"],
                                    PaneCloseBehavior::LayoutOnly => &["layout"],
                                })
                                .then(|| {
                                    let selector_pane_id = close_pane_id.clone();
                                    div()
                                        .debug_selector(move || {
                                            format!("PANE_CLOSE:{selector_pane_id}")
                                        })
                                        .flex_none()
                                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                            cx.stop_propagation()
                                        })
                                        .child(
                                            IconButton::new(
                                                SharedString::from(format!(
                                                    "close-pane-{close_pane_id}"
                                                )),
                                                "close.svg",
                                            )
                                            .size(IconButtonSize::Sm)
                                            .tooltip(crate::i18n::text("ui.close_pane"))
                                            .on_press(
                                                move |window, cx| {
                                                    let pane_id = close_pane_id.clone();
                                                    let session_id = close_session_id.clone();
                                                    close_root.update(cx, |this, cx| {
                                                        if this.tab_close_guarded() {
                                                            return;
                                                        }
                                                        match (close_behavior, session_id) {
                                                            (
                                                                PaneCloseBehavior::CloseTerminal,
                                                                Some(session_id),
                                                            ) => {
                                                                this.request_close_terminal_pane(
                                                                    &pane_id,
                                                                    &session_id,
                                                                    window,
                                                                    cx,
                                                                );
                                                            }
                                                            (
                                                                PaneCloseBehavior::ArchiveChat,
                                                                Some(session_id),
                                                            ) => {
                                                                this.request_close_chat_pane(
                                                                    &pane_id,
                                                                    &session_id,
                                                                    window,
                                                                    cx,
                                                                );
                                                            }
                                                            (PaneCloseBehavior::LayoutOnly, _) => {
                                                                this.close_pane(
                                                                    &pane_id, window, cx,
                                                                );
                                                            }
                                                            (
                                                                PaneCloseBehavior::CloseTerminal
                                                                | PaneCloseBehavior::ArchiveChat,
                                                                None,
                                                            ) => {}
                                                        }
                                                    });
                                                },
                                            ),
                                        )
                                }),
                        ),
                )
                .children(self.render_island_panel_toggle(layout, &pane_id, cx))
        };

        let header = self.render_titlebar_drag_area("island-header-drag", header, cx);
        let body: AnyElement = if start_surface_visible(Some(layout)) {
            self.render_launcher(Some(PaneKey::new(&layout.id, &pane_id)), true, window, cx)
                .into_any_element()
        } else if let Some(entry) = entry.as_ref() {
            let session_id = entry.session_id.clone();
            let secondary_state = secondary.clone().unwrap_or_default();
            let fork_materializing = self.remote_client.is_local()
                && super::super::chat::fork_materializing(&self.forking_sessions, &session_id);
            let transition = if fork_materializing {
                Some(TransitionKind::Starting)
            } else {
                self.chat_transitions
                    .get(&session_id)
                    .filter(|_| self.remote_client.is_local())
                    .map(|item| item.kind)
            };
            let overlay = resolve_pane_overlay(
                self.sidebar_archiving_session(&session_id, cx),
                transition,
                entry.status,
                entry.resumable,
                self.session_exit_codes
                    .get(&session_id)
                    .filter(|_| self.remote_client.is_local())
                    .copied()
                    .flatten(),
            );
            let interactive = self.cached_session_is_interactive(&session_id, cx);
            let scrollable = self.route == AppRoute::Chat
                && transition.is_none()
                && !self.sidebar_archiving_session(&session_id, cx);
            let terminal_style = self.terminal_style(cx);
            let terminal_background =
                crate::terminal::element::to_hsla(terminal_style.palette.background, 1.);
            let terminal_surface = if self.remote_client.is_active() {
                self.remote_client
                    .render_remote_terminal_content(&session_id, cx)
            } else if secondary_state.secondary
                && self
                    .attached
                    .get(&session_id)
                    .is_none_or(|chat| chat.terminal.control().desktop())
            {
                div()
                    .absolute()
                    .inset_0()
                    .bg(terminal_background)
                    .into_any_element()
            } else if let Some(chat) = self.attached.get(&session_id) {
                let terminal = Arc::clone(&chat.terminal);
                let terminal_interaction = chat.terminal_interaction.clone();
                let terminal_scrollbar = chat.terminal_scrollbar.clone();
                let terminal_input = chat.terminal_input.clone();
                let terminal_focus = chat.terminal_focus.clone();
                let terminal_find = chat.terminal_find.clone();
                let resize_owner = scrollable
                    && !secondary_state.secondary
                    && layout.is_resize_owner(&pane_id, &session_id);
                let key_session_id = session_id.clone();
                let copy_session_id = session_id.clone();
                let paste_session_id = session_id.clone();
                let drop_session_id = session_id.clone();
                let drop_pane_id = pane_id.clone();
                let surface = TerminalSurface {
                    terminal,
                    interaction: terminal_interaction,
                    input: terminal_input,
                    focus: terminal_focus,
                    scrollbar: terminal_scrollbar,
                    style: terminal_style,
                    interactive,
                    resize_owner,
                    scrollable,
                    find: Some(terminal_find.clone()),
                    file_links: Some(self.file_link_route(&session_id, cx)),
                };
                let surface = surface
                    .render()
                    .id(SharedString::from(format!("terminal-{session_id}")))
                    .opacity(match overlay {
                        PaneOverlayState::Starting | PaneOverlayState::Resuming => 0.,
                        PaneOverlayState::Ended { .. } => 0.45,
                        PaneOverlayState::Archiving | PaneOverlayState::None => 1.,
                    })
                    .on_action(cx.listener(move |this, action: &Copy, window, cx| {
                        this.on_terminal_copy(&copy_session_id, action, window, cx);
                    }))
                    .when(
                        interactive
                            || self
                                .attached
                                .get(&session_id)
                                .is_some_and(|chat| !chat.terminal.control().desktop()),
                        |surface| {
                            surface
                                .on_key_down(cx.listener(move |this, event, window, cx| {
                                    this.on_key_down(&key_session_id, event, window, cx);
                                }))
                                .on_action(cx.listener(move |this, action, window, cx| {
                                    this.on_paste(&paste_session_id, action, window, cx);
                                }))
                                .child(runner_app::ui::file_drop_target(
                                    &session_id,
                                    cx.listener(move |this, paths, window, cx| {
                                        this.on_file_drop(
                                            Some(&drop_pane_id),
                                            &drop_session_id,
                                            paths,
                                            window,
                                            cx,
                                        );
                                    }),
                                ))
                        },
                    );
                crate::terminal::find::with_find_bar(surface, &terminal_find)
            } else {
                div()
                    .absolute()
                    .inset_0()
                    .bg(terminal_background)
                    .text_size(theme::text_ui())
                    .text_color(theme::faint())
                    .when(matches!(overlay, PaneOverlayState::None), |surface| {
                        surface
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(crate::i18n::text("ui.attaching_terminal"))
                    })
                    .into_any_element()
            };
            let group_control =
                self.remote_client.is_local() && self.tab_control_covered(layout, cx);
            let remote_card = self
                .attached
                .get(&session_id)
                .filter(|_| self.remote_client.is_local())
                .filter(|chat| {
                    entry.status == SessionStatus::Running && !chat.terminal.control().desktop()
                });
            let overlay_element = if let Some(remote_card) = remote_card.filter(|_| !group_control)
            {
                Some(
                    crate::terminal::control::cover(
                        div().absolute().inset_0(),
                        format!("take-back-{session_id}"),
                        &[remote_card],
                        crate::terminal::control::GroupKind::Tab,
                    )
                    .into_any_element(),
                )
            } else if secondary_state.secondary
                && remote_card.is_none()
                && !self.dismissed_duplicate_chats.contains(&session_id)
            {
                let focus_label = secondary_state.primary_label.clone();
                let stay_root = cx.entity();
                let stay_session_id = session_id.clone();
                Some(
                    DuplicateSubjectOverlay::new(
                        SharedString::from(format!("duplicate-chat-{session_id}")),
                        DuplicateSubjectKind::Chat,
                        focus_label.is_some(),
                        move |_, cx| {
                            if let Some(label) = focus_label.as_deref() {
                                focus_other_window(label, cx);
                            }
                        },
                        move |_, cx| {
                            stay_root.update(cx, |this, cx| {
                                this.dismiss_duplicate_chat(&stay_session_id, cx)
                            });
                        },
                    )
                    .into_any_element(),
                )
            } else {
                match overlay {
                    PaneOverlayState::Archiving
                        if split_tab_archiving(layout, |session_id| {
                            self.sidebar_archiving_session(session_id, cx)
                        }) =>
                    {
                        None
                    }
                    PaneOverlayState::Archiving => Some(
                        SessionOverlay::transition(
                            format!("archiving-{session_id}"),
                            SessionOverlayKind::Archiving,
                        )
                        .into_any_element(),
                    ),
                    PaneOverlayState::Resuming => Some(
                        if Runtime::parse(&entry.agent_runtime).is_some_and(Runtime::is_shell) {
                            SessionOverlay::transition(
                                format!("resuming-{session_id}"),
                                SessionOverlayKind::Resuming,
                            )
                            .label(crate::i18n::text("ui.starting_terminal"))
                            .into_any_element()
                        } else {
                            SessionOverlay::transition(
                                format!("resuming-{session_id}"),
                                SessionOverlayKind::Resuming,
                            )
                            .into_any_element()
                        },
                    ),
                    PaneOverlayState::Starting => {
                        let overlay = SessionOverlay::transition(
                            format!("starting-{session_id}"),
                            SessionOverlayKind::Starting,
                        );
                        Some(
                            match starting_overlay_label(&entry.agent_runtime, fork_materializing) {
                                Some(label) => overlay.label(label).into_any_element(),
                                None => overlay.into_any_element(),
                            },
                        )
                    }
                    PaneOverlayState::Ended {
                        status,
                        resumable,
                        exit_code,
                    } => {
                        if Runtime::parse(&entry.agent_runtime).is_some_and(Runtime::is_shell) {
                            None
                        } else {
                            match Runtime::parse(&entry.agent_runtime).and_then(|runtime| {
                                self.agent_recovery_overlay(
                                    &pane_id,
                                    &session_id,
                                    runtime,
                                    cx.weak_entity(),
                                    cx,
                                )
                            }) {
                                Some(overlay) => Some(overlay.into_any_element()),
                                _ => {
                                    let resume_root = cx.entity();
                                    let archive_root = resume_root.clone();
                                    let resume_id = session_id.clone();
                                    let archive_id = session_id.clone();
                                    let resume_pane = pane_id.clone();
                                    Some(
                                        SessionOverlay::ended(
                                            format!("ended-{session_id}"),
                                            entry
                                                .local
                                                .as_ref()
                                                .and_then(|entry| {
                                                    self.claude_paused_text(entry, cx)
                                                })
                                                .map_or_else(
                                                    || ended_subtitle(status, resumable, exit_code),
                                                    |account| {
                                                        if !resumable
                                                            || status == SessionStatus::Crashed
                                                        {
                                                            format!(
                                                                "{} {account}",
                                                                ended_subtitle(
                                                                    status, resumable, exit_code
                                                                )
                                                            )
                                                        } else {
                                                            account
                                                        }
                                                    },
                                                ),
                                            move |window, cx| {
                                                resume_root.update(cx, |this, cx| {
                                                    this.resume_chat(
                                                        &resume_pane,
                                                        &resume_id,
                                                        window,
                                                        cx,
                                                    )
                                                });
                                            },
                                            move |window, cx| {
                                                archive_root.update(cx, |this, cx| {
                                                    this.archive_chat_sessions(
                                                        vec![archive_id.clone()],
                                                        window,
                                                        cx,
                                                    )
                                                });
                                            },
                                        )
                                        .when_some(
                                            start_action_shortcut(
                                                "resume-session",
                                                &self.settings(cx).keymap_overrides,
                                            ),
                                            |overlay, shortcut| overlay.resume_shortcut(shortcut),
                                        )
                                        .actions_visible(
                                            self.front_page_client(cx).supports(&["resume"]),
                                            self.front_page_client(cx)
                                                .supports(&["archive", "layout"]),
                                        )
                                        .when(cfg!(test), |overlay| {
                                            overlay.primary_debug_selector(format!(
                                                "BUTTON:ended-{session_id}-resume"
                                            ))
                                        })
                                        .into_any_element(),
                                    )
                                }
                            }
                        }
                    }
                    PaneOverlayState::None => None,
                }
            };
            div()
                .relative()
                .flex_1()
                .min_h(px(0.))
                .overflow_hidden()
                .bg(terminal_background)
                .child(terminal_surface)
                .children(overlay_element)
                .into_any_element()
        } else if leaf.session_id.is_some() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme::muted())
                .child(crate::i18n::text("ui.loading"))
                .into_any_element()
        } else {
            self.render_launcher(Some(PaneKey::new(&layout.id, &pane_id)), false, window, cx)
                .into_any_element()
        };

        // The split menu gates its items on how big this pane was last laid
        // out, and those items are built during this entity's render — so a
        // size that actually changed has to ask for another one. Notifying
        // only on a change is what keeps that from looping every frame.
        let measure_root = cx.entity();
        let measured = PaneKey::new(&layout.id, &pane_id);
        let drop_key = measured.clone();
        let preview = self
            .pane_drop
            .as_ref()
            .and_then(|(key, side)| (key == &drop_key).then_some(*side));
        div()
            .id(SharedString::from(format!("pane-{pane_id}")))
            .debug_selector({
                let pane_id = pane_id.clone();
                move || format!("PANE_ISLAND:{pane_id}")
            })
            .relative()
            .size_full()
            .min_w(px(0.))
            .min_h(px(0.))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(rems(12. / 16.))
            .border_1()
            .border_color(if focused {
                theme::with_alpha(
                    theme::text(),
                    if theme::active_variant().is_light() {
                        0.22
                    } else {
                        0.2
                    },
                )
            } else {
                theme::border()
            })
            .bg(theme::bg())
            .pb(rems(8. / 16.))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, _, cx| {
                        measure_root.update(cx, |this, this_cx| {
                            if this.front_page_pane_bounds().get(&measured) == Some(&bounds.size) {
                                return;
                            }
                            this.front_page_pane_bounds_mut()
                                .insert(measured.clone(), bounds.size);
                            this_cx.notify();
                        });
                    },
                )
                .absolute()
                .inset_0(),
            )
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .flex()
                    .flex_col()
                    .opacity(pane_body_opacity(grouped, focused))
                    .child(body),
            )
            .children(preview.map(|side| {
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        window.paint_quad(quad(
                            drop_preview(bounds, side),
                            px(0.),
                            theme::with_alpha(theme::accent(), 0.12),
                            px(1.),
                            theme::accent(),
                            BorderStyle::Solid,
                        ));
                    },
                )
                .absolute()
                .inset_0()
            }))
            .on_drag_move::<PaneDrag>(cx.listener(
                move |this, event: &DragMoveEvent<PaneDrag>, _, cx| {
                    let drag = event.drag(cx);
                    let next = if event.bounds.contains(&event.event.position) {
                        let side = drop_side(event.bounds, event.event.position);
                        (drag.tab_id == drop_key.tab_id
                            && drag.pane_id != drop_key.pane_id
                            && drop_allowed(
                                Some(event.bounds.size),
                                side,
                                this.settings(cx).app_zoom,
                            ))
                        .then(|| (drop_key.clone(), side))
                    } else if this
                        .pane_drop
                        .as_ref()
                        .is_some_and(|(key, _)| key == &drop_key)
                    {
                        None
                    } else {
                        return;
                    };
                    if this.pane_drop != next {
                        this.pane_drop = next;
                        cx.notify();
                    }
                },
            ))
            .on_drop(cx.listener(|this, drag: &PaneDrag, window, cx| {
                this.apply_pane_drop(drag, window, cx);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    if let Some(session_id) = pane_session_for_focus.as_deref() {
                        this.focus_terminal(&pane_id_for_focus, session_id, window, cx);
                    } else {
                        this.focus_pane(&pane_id_for_focus, cx);
                    }
                }),
            )
            .into_any_element()
    }
}
