use super::*;

impl NativeRoot {
    pub(super) fn choose_computer(
        &mut self,
        computer: Computer,
        new_window: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.client_ui.picker_open = false;
        if computer.is_self {
            if new_window {
                let _ = open_new_runner_window(None, cx);
            } else {
                self.return_to_this_computer(window, cx);
            }
        } else if !computer.paired {
            self.open_computer_pair(computer, pairing::Purpose::Open { new_window }, window, cx);
        } else {
            self.open_computer(computer, new_window, cx);
        }
        cx.notify();
    }
    pub(super) fn open_computer(
        &mut self,
        computer: Computer,
        new_window: bool,
        cx: &mut Context<Self>,
    ) {
        if new_window {
            cx.defer(move |cx| match open_new_runner_window(None, cx) {
                Ok(label) => {
                    let window = cx
                        .windows()
                        .into_iter()
                        .filter_map(|window| window.downcast::<NativeRoot>())
                        .find(|window| {
                            window.read(cx).is_ok_and(|root| root.window_label == label)
                        });
                    if let Some(window) = window {
                        let _ = window.update(cx, |root, window, cx| {
                            root.visit_computer(computer, window, cx)
                        });
                    }
                }
                Err(error) => tracing::error!("open remote window: {error}"),
            });
        } else {
            cx.spawn(async move |weak, cx| {
                let _ = weak.update_in(cx, |root, window, cx| {
                    root.visit_computer(computer, window, cx)
                });
            })
            .detach();
        }
    }
    pub(crate) fn render_computer_trigger(&self, cx: &mut Context<Self>) -> AnyElement {
        let clients = state(&self.app_store, cx);
        let current = self.remote_window.as_ref();
        let name = current
            .map(|remote| remote.computer.name.clone())
            .or_else(|| {
                clients
                    .read(cx)
                    .computers
                    .iter()
                    .find(|computer| computer.is_self)
                    .map(|c| c.name.clone())
            })
            .or_else(|| {
                self.app_store
                    .read(cx)
                    .remote_status
                    .as_ref()
                    .and_then(|status| status.host_name.clone())
            })
            .unwrap_or_else(|| crate::i18n::text("client.this_computer").into());
        let badge = self.local_attention_count(cx);
        let owner = cx.weak_entity();
        div()
            .mx_3()
            .mb_3()
            .relative()
            .child(
                div()
                    .id("computer-picker-trigger")
                    .debug_selector(|| "COMPUTER_PICKER_TRIGGER".into())
                    .tab_index(0)
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded(rems(6. / 16.))
                    .border_1()
                    .border_color(if current.is_some() {
                        theme::with_alpha(theme::accent(), 0.4)
                    } else {
                        theme::border()
                    })
                    .bg(if current.is_some() {
                        theme::with_alpha(theme::accent(), 0.08)
                    } else {
                        theme::chrome_selected()
                    })
                    .cursor_pointer()
                    .on_click(
                        cx.listener(|root, _, window, cx| root.toggle_computer_picker(window, cx)),
                    )
                    .on_key_down(cx.listener(|root, event: &KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space" | "down") {
                            root.toggle_computer_picker(window, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .child(icon(
                        "monitor.svg",
                        if current.is_some() {
                            theme::accent()
                        } else {
                            theme::muted()
                        },
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(theme::text_body())
                            .child(name),
                    )
                    .children(current.map(|remote| {
                        match remote.connection {
                            Connection::Connected => div()
                                .size(rems(6. / 16.))
                                .rounded_full()
                                .bg(theme::accent())
                                .into_any_element(),
                            Connection::Revoked => {
                                icon("lock.svg", theme::danger()).into_any_element()
                            }
                            Connection::Unreachable | Connection::Forgotten => div()
                                .size(rems(6. / 16.))
                                .rounded_full()
                                .bg(theme::faint())
                                .into_any_element(),
                            _ => runner_app::ui::button::spinner(
                                "computer-connection-spinner",
                                12.,
                                if remote.connection == Connection::Reconnecting {
                                    theme::warning()
                                } else {
                                    theme::muted()
                                },
                            ),
                        }
                    }))
                    .children((badge > 0).then(|| {
                        div()
                            .rounded_full()
                            .px_2()
                            .bg(theme::with_alpha(theme::warning(), 0.15))
                            .text_color(theme::warning())
                            .text_size(theme::text_micro())
                            .child(badge.to_string())
                    }))
                    .child(icon("chevrons-up-down.svg", theme::faint())),
            )
            .child(
                gpui::canvas(
                    |_, _, _| {},
                    move |bounds, _, _, cx| {
                        let _ = owner.update(cx, |root, _| root.client_ui.anchor = Some(bounds));
                    },
                )
                .absolute()
                .inset_0(),
            )
            .into_any_element()
    }
    fn toggle_computer_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.client_ui.picker_open = !self.client_ui.picker_open;
        if self.client_ui.picker_open {
            self.refresh_computers(cx);
            self.client_ui.picker_focus.focus(window, cx);
        }
        cx.notify();
    }
    fn local_attention_count(&self, cx: &App) -> usize {
        let store = self.app_store.read(cx);
        store
            .session_statuses
            .values()
            .filter(|status| !status.observation.interactions.is_empty())
            .count()
            + store
                .missions
                .iter()
                .map(|mission| mission.pending_ask_count)
                .sum::<usize>()
    }
    pub(crate) fn render_computer_picker(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.client_ui.picker_open {
            return None;
        }
        let anchor = self.client_ui.anchor?;
        let clients = state(&self.app_store, cx);
        let mut computers = clients.read(cx).computers.clone();
        if !computers.iter().any(|c| c.is_self) {
            let status = self
                .app_store
                .read(cx)
                .remote_status
                .clone()
                .unwrap_or_default();
            computers.insert(
                0,
                Computer {
                    id: status.host_id.unwrap_or_default(),
                    name: status
                        .host_name
                        .unwrap_or_else(|| crate::i18n::text("client.this_computer").into()),
                    is_self: true,
                    ..Default::default()
                },
            );
        }
        let mut rows = Vec::new();
        for computer in &computers {
            let selected = if computer.is_self {
                self.remote_window.is_none()
            } else {
                self.remote_window
                    .as_ref()
                    .is_some_and(|remote| remote.computer.id == computer.id)
            };
            let badge = if computer.is_self {
                self.local_attention_count(cx)
            } else {
                clients
                    .read(cx)
                    .overviews
                    .get(&computer.id)
                    .map(model::needs_you)
                    .unwrap_or(0)
            };
            let mut detail = if computer.is_self {
                crate::i18n::text("client.this_computer").into()
            } else if !computer.paired {
                crate::i18n::text("client.unpaired_picker").into()
            } else {
                status_label(computer, false)
            };
            if badge > 0 && !computer.is_self {
                detail = format!(
                    "{detail} · {}",
                    crate::i18n::tf("client.needs_you", &[("count", badge.to_string())])
                );
            }
            let trust = self
                .app_store
                .read(cx)
                .remote_trust
                .clone()
                .unwrap_or_default();
            let devices = model::incoming_devices(computer, &computers, &trust);
            let menu = self.computer_menu(Some(computer), &devices, cx);
            let select = computer.clone();
            let keyboard = computer.clone();
            let another = computer.clone();
            rows.push(
                div()
                    .id(SharedString::from(format!("computer-{}", computer.id)))
                    .debug_selector(|| "COMPUTER_PICKER_ROW".into())
                    .tab_index(0)
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_2()
                    .rounded(rems(6. / 16.))
                    .bg(if selected {
                        theme::raised()
                    } else {
                        gpui::transparent_black()
                    })
                    .hover(|row| row.bg(theme::chrome_hover()))
                    .cursor_pointer()
                    .on_click(
                        cx.listener(move |root, event: &gpui::ClickEvent, window, cx| {
                            root.choose_computer(
                                select.clone(),
                                event.modifiers().shift,
                                window,
                                cx,
                            );
                            cx.stop_propagation();
                        }),
                    )
                    .on_key_down(cx.listener(move |root, event: &KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            root.choose_computer(
                                keyboard.clone(),
                                event.keystroke.modifiers.shift,
                                window,
                                cx,
                            );
                            cx.stop_propagation();
                        }
                    }))
                    .child(icon("monitor.svg", theme::muted()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .truncate()
                                    .font_weight(if selected {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .child(computer.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(theme::text_meta())
                                    .text_color(theme::faint())
                                    .truncate()
                                    .child(detail),
                            ),
                    )
                    .children((badge > 0).then(|| {
                        div()
                            .rounded_full()
                            .px_2()
                            .bg(theme::with_alpha(theme::warning(), 0.15))
                            .text_color(theme::warning())
                            .text_size(theme::text_micro())
                            .child(badge.to_string())
                    }))
                    .when(selected, |row| {
                        row.child(icon("check.svg", theme::accent()))
                    })
                    .when(!computer.is_self, |row| {
                        row.child(
                            IconButton::new(
                                SharedString::from(format!("open-computer-window-{}", computer.id)),
                                "app-window.svg",
                            )
                            .size(IconButtonSize::Sm)
                            .tooltip(crate::i18n::text("client.open_window"))
                            .on_press({
                                let owner = cx.weak_entity();
                                move |window, cx| {
                                    cx.stop_propagation();
                                    let _ = owner.update(cx, |root, cx| {
                                        root.choose_computer(another.clone(), true, window, cx)
                                    });
                                }
                            }),
                        )
                        .children(menu)
                    })
                    .into_any_element(),
            );
        }
        let owner = cx.weak_entity();
        let menu = div()
            .track_focus(&self.client_ui.picker_focus)
            .on_key_down(cx.listener(|root, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    root.client_ui.picker_open = false;
                    root.root_focus.focus(window, cx);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .flex()
            .flex_col()
            .gap_1()
            .p_1()
            .text_size(theme::text_body())
            .children(rows)
            .when(clients.read(cx).action_failed, |menu| {
                menu.child(
                    div()
                        .px_2()
                        .py_2()
                        .text_size(theme::text_meta())
                        .text_color(theme::warning())
                        .child(crate::i18n::text("client.action_failed")),
                )
            })
            .when(clients.read(cx).directory_error, |menu| {
                menu.child(
                    div()
                        .px_2()
                        .py_2()
                        .text_size(theme::text_meta())
                        .text_color(theme::warning())
                        .child(crate::i18n::text("client.directory_failed")),
                )
            })
            .child(
                div()
                    .border_t_1()
                    .border_color(theme::border())
                    .p_2()
                    .text_size(theme::text_meta())
                    .text_color(theme::faint())
                    .child(crate::i18n::text("client.picker_hint")),
            );
        Some(popup_layer(
            anchor,
            window,
            px(320. * self.settings(cx).app_zoom),
            menu.into_any_element(),
            Rc::new(move |window, cx| {
                let _ = owner.update(cx, |root, cx| {
                    root.client_ui.picker_open = false;
                    root.root_focus.focus(window, cx);
                    cx.notify();
                });
            }),
        ))
    }
}
