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
        let status = self
            .app_store
            .read(cx)
            .remote_status
            .clone()
            .unwrap_or_default();
        let local_name = if crate::surfaces::settings::remote::account::signed_in(&status) {
            clients
                .read(cx)
                .computers
                .iter()
                .find(|computer| computer.is_self)
                .map(|computer| computer.name.clone())
                .or(status.host_name)
        } else {
            None
        };
        let name = current
            .map(|remote| remote.computer.name.clone())
            .or(local_name)
            .unwrap_or_else(|| crate::i18n::text("client.this_computer").into());
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
        let status = self
            .app_store
            .read(cx)
            .remote_status
            .clone()
            .unwrap_or_default();
        let signed_in = crate::surfaces::settings::remote::account::signed_in(&status);
        let mut computers = if signed_in {
            clients.read(cx).computers.clone()
        } else {
            Vec::new()
        };
        if !computers.iter().any(|c| c.is_self) {
            computers.insert(0, model::this_computer(&status, signed_in));
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
            let detail = if computer.is_self {
                crate::i18n::text("client.this_computer").into()
            } else if !computer.paired {
                crate::i18n::text("client.unpaired_picker").into()
            } else {
                status_label(computer, false)
            };
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
                    .when(!computer.is_self, |row| {
                        row.debug_selector(|| "COMPUTER_PICKER_REMOTE_ROW".into())
                    })
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
                                    .debug_selector(|| "COMPUTER_PICKER_NAME".into())
                                    .truncate()
                                    .font_weight(if selected {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .child(computer.name.clone()),
                            )
                            .when(signed_in, |column| {
                                column.child(
                                    div()
                                        .debug_selector(|| "COMPUTER_PICKER_DETAIL".into())
                                        .text_size(theme::text_meta())
                                        .text_color(theme::faint())
                                        .truncate()
                                        .child(detail),
                                )
                            }),
                    )
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
            .rounded(rems(8. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .shadow_2xl()
            .track_focus(&self.client_ui.picker_focus)
            .on_key_down(cx.listener(|root, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => {
                        root.client_ui.picker_open = false;
                        root.root_focus.focus(window, cx);
                    }
                    "tab" => {
                        if event.keystroke.modifiers.shift {
                            window.focus_prev(cx);
                        } else {
                            window.focus_next(cx);
                        }
                        if !root.client_ui.picker_focus.contains_focused(window, cx) {
                            root.client_ui.picker_open = false;
                            root.root_focus.focus(window, cx);
                        }
                    }
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .flex()
            .flex_col()
            .gap_1()
            .p_1()
            .text_size(theme::text_body())
            .when(cfg!(test), |menu| {
                #[cfg(test)]
                return crate::theme_snapshot::record_fill("COMPUTER_PICKER_PANEL", menu);
                #[cfg(not(test))]
                menu
            })
            .children(rows)
            .when(signed_in && clients.read(cx).action_failed, |menu| {
                menu.child(
                    div()
                        .debug_selector(|| "COMPUTER_PICKER_ACTION_ERROR".into())
                        .px_2()
                        .py_2()
                        .text_size(theme::text_meta())
                        .text_color(theme::warning())
                        .child(crate::i18n::text("client.action_failed")),
                )
            })
            .when(signed_in && clients.read(cx).directory_error, |menu| {
                menu.child(
                    div()
                        .debug_selector(|| "COMPUTER_PICKER_ERROR".into())
                        .px_2()
                        .py_2()
                        .text_size(theme::text_meta())
                        .text_color(theme::warning())
                        .child(crate::i18n::text("client.directory_failed")),
                )
            })
            .when(signed_in, |menu| {
                menu.child(
                    div()
                        .debug_selector(|| "COMPUTER_PICKER_HINT".into())
                        .border_t_1()
                        .border_color(theme::border())
                        .p_2()
                        .text_size(theme::text_meta())
                        .text_color(theme::faint())
                        .child(crate::i18n::text("client.picker_hint")),
                )
            })
            .when(!signed_in, |menu| {
                menu.child(div().h(px(1.)).bg(theme::border()).my_1())
                    .child(
                        div()
                            .id("computer-picker-sign-in")
                            .debug_selector(|| "COMPUTER_PICKER_SIGN_IN".into())
                            .tab_index(0)
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py_2()
                            .rounded(rems(6. / 16.))
                            .cursor_pointer()
                            .hover(|row| row.bg(theme::chrome_hover()))
                            .child(icon("log-in.svg", theme::muted()))
                            .child(crate::i18n::text("client.picker_sign_in"))
                            .on_click(cx.listener(|root, _, window, cx| {
                                root.client_ui.picker_open = false;
                                root.enter_settings_route(Some("account"), window, cx);
                            }))
                            .on_key_down(cx.listener(|root, event: &KeyDownEvent, window, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    root.client_ui.picker_open = false;
                                    root.enter_settings_route(Some("account"), window, cx);
                                    cx.stop_propagation();
                                }
                            })),
                    )
            });
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
