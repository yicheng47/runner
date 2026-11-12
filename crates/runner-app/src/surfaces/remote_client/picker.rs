use super::*;
use gpui::Hsla;
use runner_app::ui::menu::{popup_layer_positioned, PopupWidth};

pub(super) fn computer_detail(computer: &Computer) -> String {
    if computer.is_self {
        return crate::i18n::text("client.this_computer").into();
    }
    let status = status_label(computer, false);
    if computer.paired {
        status
    } else {
        crate::i18n::tf("client.unpaired_picker", &[("status", status)])
    }
}

impl RemoteClient {
    pub(super) fn open_computer(
        &mut self,
        computer: Computer,
        new_window: bool,
        cx: &mut Context<NativeRoot>,
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
    pub(crate) fn render_host_button(&self, cx: &mut Context<NativeRoot>) -> AnyElement {
        let zoom = self.settings(cx).app_zoom;
        let clients = state(&self.app_store, cx);
        let current = self.workspace.as_ref();
        let signed_in = self.app_store.read(cx).remote_signed_in();
        let attention = signed_in
            && clients.read(cx).computers.iter().any(|computer| {
                !computer.is_self
                    && current.is_none_or(|remote| remote.computer.id != computer.id)
                    && clients.read(cx).needs_attention(&computer.id)
            });
        let tone = current.map(|remote| {
            host_tone(
                remote.connection,
                clients.read(cx).slow(&remote.computer.id),
            )
        });
        let open = self.ui.picker_open;
        let owner = cx.weak_entity();
        let button = crate::surfaces::app_shell::SidebarFooterControl::Host
            .render(zoom, tone.unwrap_or_else(theme::muted))
            .id("sidebar-host-button")
            .debug_selector(|| "SIDEBAR_HOST_BUTTON".into())
            .track_focus(&self.ui.host_focus.clone().tab_stop(true))
            .tab_index(0)
            .relative()
            .border_1()
            .border_color(tone.map_or(gpui::transparent_black(), |color| {
                theme::with_alpha(color, 0.4)
            }))
            .map(|button| match tone {
                Some(color) => button
                    .bg(theme::with_alpha(color, 0.08))
                    .hover(move |button| button.bg(theme::with_alpha(color, 0.16)))
                    .focus_visible(move |button| button.bg(theme::with_alpha(color, 0.16))),
                None => button
                    .when(open, |button| button.bg(theme::chrome_selected()))
                    .hover(|button| button.bg(theme::chrome_hover()))
                    .focus_visible(|button| button.bg(theme::chrome_hover())),
            })
            .map(|button| {
                #[cfg(test)]
                let button = crate::theme_snapshot::record_border("SIDEBAR_HOST_BUTTON", button);
                #[cfg(test)]
                let button = crate::theme_snapshot::record_fill("SIDEBAR_HOST_BUTTON", button);
                button
            })
            .cursor_pointer()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|root, _, window, cx| {
                root.remote_client.toggle_computer_picker(window, cx)
            }))
            .on_key_down(cx.listener(|root, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space" | "down") {
                    root.remote_client.toggle_computer_picker(window, cx);
                    cx.stop_propagation();
                }
            }))
            .children(attention.then(|| {
                div()
                    .absolute()
                    .top(px(4. * zoom))
                    .right(px(4. * zoom))
                    .child(attention_dot("HOST_BUTTON_ATTENTION"))
            }))
            .child(
                gpui::canvas(
                    |_, _, _| {},
                    move |bounds, _, _, cx| {
                        let _ =
                            owner.update(cx, |root, _| root.remote_client.ui.anchor = Some(bounds));
                    },
                )
                .absolute()
                .inset_0(),
            );
        Tooltip::new(
            "sidebar-host-tooltip",
            crate::i18n::text("client.host_button"),
            button,
        )
        .into_any_element()
    }
    fn toggle_computer_picker(&mut self, window: &mut Window, cx: &mut Context<NativeRoot>) {
        self.ui.picker_open = !self.ui.picker_open;
        self.ui.picker_signed_in = self.app_store.read(cx).remote_signed_in();
        if self.ui.picker_open {
            if self.ui.picker_signed_in {
                self.refresh_computers(cx);
            }
            self.ui.picker_focus.focus(window, cx);
        } else {
            self.ui.host_focus.focus(window, cx);
        }
        cx.notify();
    }
    pub(crate) fn render_computer_picker(
        &mut self,
        window: &Window,
        cx: &mut Context<NativeRoot>,
    ) -> Option<AnyElement> {
        if !self.ui.picker_open {
            return None;
        }
        let anchor = self.ui.anchor?;
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
            let mut this = model::this_computer(&status, signed_in);
            if !signed_in {
                let host = local_host();
                if host.hostname != "unknown" {
                    this.name = host.hostname.clone();
                }
                this.info = host.clone();
            }
            computers.insert(0, this);
        }
        if let Some(remote) = &self.workspace {
            if !computers.iter().any(|c| c.id == remote.computer.id) {
                computers.push(remote.computer.clone());
            }
        }
        computers.sort_by_key(|computer| !computer.is_self);
        let trust = self
            .app_store
            .read(cx)
            .remote_trust
            .clone()
            .unwrap_or_default();
        let mut rows = Vec::new();
        for computer in &computers {
            let current = if computer.is_self {
                self.workspace.is_none()
            } else {
                self.workspace
                    .as_ref()
                    .is_some_and(|remote| remote.computer.id == computer.id)
            };
            let status = self.workspace.as_ref().filter(|_| current).map(|remote| {
                connection_status(
                    remote.connection,
                    clients.read(cx).latency(&computer.id),
                    clients.read(cx).lacks_ping(&computer.id),
                    clients.read(cx).slow(&computer.id),
                    &computer.name,
                )
            });
            let (detail, detail_color) = status.as_ref().map_or_else(
                || (computer_detail(computer), theme::faint()),
                |status| (status.text.clone(), status.text_color),
            );
            let attention = !current && clients.read(cx).needs_attention(&computer.id);
            let devices = model::incoming_devices(computer, &computers, &trust);
            let menu = self.computer_menu(Some(computer), &devices, cx);
            let select = computer.clone();
            let keyboard = computer.clone();
            let another = computer.clone();
            let tile_color = if computer.is_self {
                theme::accent()
            } else {
                theme::muted()
            };
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
                    .py_1p5()
                    .rounded(rems(6. / 16.))
                    .hover(|row| row.bg(theme::chrome_hover()))
                    .focus_visible(|row| row.bg(theme::chrome_hover()))
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
                    .child(
                        div()
                            .flex_none()
                            .size(rems(28. / 16.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(rems(6. / 16.))
                            .bg(if computer.is_self {
                                theme::with_alpha(theme::accent(), 0.14)
                            } else {
                                theme::raised()
                            })
                            .map(|tile| {
                                #[cfg(test)]
                                let tile = if computer.is_self {
                                    crate::theme_snapshot::record_fill(
                                        "COMPUTER_PICKER_SELF_TILE",
                                        tile,
                                    )
                                } else {
                                    tile.debug_selector(|| "COMPUTER_PICKER_TILE".into())
                                };
                                tile
                            })
                            .child(icon(computer_glyph(computer), tile_color)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .debug_selector(|| "COMPUTER_PICKER_NAME".into())
                                    .truncate()
                                    .font_weight(if current {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .child(computer.name.clone()),
                            )
                            .child(
                                div()
                                    .debug_selector(|| "COMPUTER_PICKER_DETAIL".into())
                                    .when(current && !computer.is_self, |detail| {
                                        detail
                                            .debug_selector(|| "COMPUTER_PICKER_CONNECTION".into())
                                    })
                                    .flex()
                                    .items_center()
                                    .gap(rems(5. / 16.))
                                    .text_size(rems(11. / 16.))
                                    .text_color(detail_color)
                                    .children(status.as_ref().and_then(connection_mark))
                                    .child(
                                        div()
                                            .when(current && !computer.is_self, |text| {
                                                text.debug_selector(|| {
                                                    "COMPUTER_PICKER_STATUS_TEXT".into()
                                                })
                                            })
                                            .min_w_0()
                                            .truncate()
                                            .child(detail),
                                    ),
                            ),
                    )
                    .children(attention.then(|| attention_dot("COMPUTER_ROW_ATTENTION")))
                    .when(current, |row| {
                        row.child(
                            icon("check.svg", theme::accent())
                                .debug_selector(|| "COMPUTER_PICKER_CHECK".into()),
                        )
                    })
                    .when(!computer.is_self && !current, |row| {
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
            if computer.is_self && (computers.len() > 1 || !signed_in) {
                rows.push(picker_divider("COMPUTER_PICKER_DIVIDER"));
            }
        }
        let owner = cx.weak_entity();
        let menu = div()
            .rounded(rems(8. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .shadow_2xl()
            .track_focus(&self.ui.picker_focus)
            .on_key_down(cx.listener(|root, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => {
                        root.remote_client.ui.picker_open = false;
                        root.remote_client.ui.host_focus.focus(window, cx);
                    }
                    "tab" => {
                        if event.keystroke.modifiers.shift {
                            window.focus_prev(cx);
                        } else {
                            window.focus_next(cx);
                        }
                        if !root
                            .remote_client
                            .ui
                            .picker_focus
                            .contains_focused(window, cx)
                        {
                            root.remote_client.ui.picker_open = false;
                            root.remote_client.ui.host_focus.focus(window, cx);
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
            .map(|menu| {
                let (selector, id, pane, glyph, key) = if signed_in {
                    (
                        "COMPUTER_PICKER_SETTINGS",
                        "computer-picker-remote-settings",
                        "remote",
                        "sliders-horizontal.svg",
                        "client.remote_access_settings",
                    )
                } else {
                    (
                        "COMPUTER_PICKER_SIGN_IN",
                        "computer-picker-sign-in",
                        "account",
                        "log-in.svg",
                        "client.sign_in_other_computers",
                    )
                };
                menu.when(signed_in, |menu| {
                    menu.child(picker_divider("COMPUTER_PICKER_SETTINGS_DIVIDER"))
                })
                .child(
                    div()
                        .id(id)
                        .debug_selector(move || selector.into())
                        .tab_index(0)
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_2()
                        .rounded(rems(6. / 16.))
                        .hover(|row| row.bg(theme::chrome_hover()))
                        .focus_visible(|row| row.bg(theme::chrome_hover()))
                        .cursor_pointer()
                        .on_click(cx.listener(move |root, _, window, cx| {
                            root.open_this_computer_settings(Some(pane), window, cx);
                            cx.stop_propagation();
                        }))
                        .on_key_down(cx.listener(move |root, event: &KeyDownEvent, window, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                root.open_this_computer_settings(Some(pane), window, cx);
                                cx.stop_propagation();
                            }
                        }))
                        .child(
                            div()
                                .flex_none()
                                .w(rems(28. / 16.))
                                .flex()
                                .justify_center()
                                .child(icon(glyph, theme::muted())),
                        )
                        .child(crate::i18n::text(key)),
                )
            });
        let zoom = self.settings(cx).app_zoom;
        Some(popup_layer_positioned(
            anchor,
            window,
            PopupWidth::Fixed(px(320. * zoom)),
            self.ui.footer_left.unwrap_or(anchor.left()),
            menu.into_any_element(),
            Rc::new(move |window, cx| {
                let _ = owner.update(cx, |root, cx| {
                    root.remote_client.ui.picker_open = false;
                    root.remote_client.ui.host_focus.focus(window, cx);
                    cx.notify();
                });
            }),
        ))
    }
}

pub(super) fn host_tone(connection: Connection, slow: bool) -> Hsla {
    match connection {
        Connection::Connected if slow => theme::warning(),
        Connection::Connected | Connection::Opening => theme::accent(),
        Connection::Reconnecting => theme::warning(),
        Connection::Unreachable | Connection::Revoked => theme::danger(),
        Connection::Forgotten => theme::muted(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum LinkMark {
    Bars(u8),
    Icon(&'static str),
    None,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct LinkStatus {
    pub(super) mark: LinkMark,
    pub(super) text: String,
    pub(super) mark_color: Hsla,
    pub(super) text_color: Hsla,
}

pub(super) fn connection_status(
    connection: Connection,
    latency: Option<u64>,
    lacks_ping: bool,
    slow: bool,
    computer: &str,
) -> LinkStatus {
    let (text, text_color) = connection_subtitle(connection, latency, slow, computer);
    let mark = match (connection, latency) {
        (Connection::Connected, Some(ms)) => LinkMark::Bars(match ms {
            0..=99 => 4,
            100..=199 => 3,
            200..=499 => 2,
            _ => 1,
        }),
        (Connection::Connected, None) if lacks_ping => LinkMark::None,
        (Connection::Connected | Connection::Opening, _) => LinkMark::Icon("loader-circle.svg"),
        (Connection::Reconnecting, _) => LinkMark::Icon("refresh-cw.svg"),
        (Connection::Unreachable, _) => LinkMark::Icon("unplug.svg"),
        _ => LinkMark::None,
    };
    let mark_color = match connection {
        Connection::Connected if latency.is_some() => host_tone(connection, slow),
        Connection::Connected | Connection::Opening => theme::muted(),
        _ => text_color,
    };
    LinkStatus {
        mark,
        text,
        mark_color,
        text_color,
    }
}

fn connection_mark(status: &LinkStatus) -> Option<AnyElement> {
    match status.mark {
        LinkMark::Bars(lit) => Some(
            div()
                .debug_selector(|| "COMPUTER_PICKER_LATENCY_BARS".into())
                .flex_none()
                .flex()
                .items_end()
                .w(rems(18. / 16.))
                .h(rems(9. / 16.))
                .gap(rems(2. / 16.))
                .children([3., 5., 7., 9.].into_iter().enumerate().map(|(i, height)| {
                    div()
                        .w(rems(3. / 16.))
                        .h(rems(height / 16.))
                        .rounded(rems(0.5 / 16.))
                        .bg(if i < lit as usize {
                            status.mark_color
                        } else {
                            theme::border_strong()
                        })
                }))
                .into_any_element(),
        ),
        LinkMark::Icon(path) => Some(
            svg()
                .path(path)
                .size(rems(10. / 16.))
                .flex_none()
                .text_color(status.mark_color)
                .into_any_element(),
        ),
        LinkMark::None => None,
    }
}

pub(super) fn connection_subtitle(
    connection: Connection,
    latency: Option<u64>,
    slow: bool,
    computer: &str,
) -> (String, Hsla) {
    let (key, color) = match connection {
        Connection::Connected => {
            return (
                latency.map_or_else(
                    || crate::i18n::text("client.connection_live").into(),
                    |ms| crate::i18n::tf("client.connection_latency", &[("ms", ms.to_string())]),
                ),
                if slow {
                    theme::warning()
                } else {
                    theme::faint()
                },
            )
        }
        Connection::Opening => ("client.connection_opening", theme::faint()),
        Connection::Reconnecting => ("client.connection_reconnecting", theme::warning()),
        Connection::Unreachable => ("client.connection_unreachable", theme::danger()),
        Connection::Revoked => ("client.connection_revoked", theme::danger()),
        Connection::Forgotten => ("client.connection_forgotten", theme::muted()),
    };
    (label(key, computer), color)
}

fn local_host() -> &'static runner_core::remote::HostInfo {
    static HOST: std::sync::OnceLock<runner_core::remote::HostInfo> = std::sync::OnceLock::new();
    HOST.get_or_init(runner_core::remote::HostInfo::current)
}

pub(super) fn computer_glyph(computer: &Computer) -> &'static str {
    let host = computer.info.hostname.to_lowercase();
    if host.contains("macbook") || host.contains("laptop") {
        "laptop.svg"
    } else if host.contains("server") {
        "server.svg"
    } else {
        "monitor.svg"
    }
}

fn picker_divider(selector: &'static str) -> AnyElement {
    div()
        .debug_selector(move || selector.into())
        .mx_1()
        .h(px(1.))
        .bg(theme::border())
        .into_any_element()
}

fn attention_dot(selector: &'static str) -> AnyElement {
    div()
        .debug_selector(move || selector.into())
        .flex_none()
        .size(rems(7. / 16.))
        .rounded_full()
        .bg(theme::accent())
        .into_any_element()
}
