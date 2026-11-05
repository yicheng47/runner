use super::*;

impl RemoteClient {
    pub(crate) fn mission_surface(&self, cx: &mut Context<NativeRoot>) -> Entity<MissionWorkspace> {
        let remote = self.workspace.as_ref().unwrap();
        let mut sizes = remote.sizes.clone();
        sizes.extend(
            remote
                .terminals
                .iter()
                .map(|(id, pane)| (id.clone(), pane.attachment.terminal.size())),
        );
        remote.mission_workspace.update(cx, |workspace, _| {
            workspace.set_remote_terminal_sizes(sizes)
        });
        remote.mission_workspace.clone()
    }

    pub(crate) fn render_remote_workspace(
        &self,
        body: AnyElement,
        cx: &mut Context<NativeRoot>,
    ) -> AnyElement {
        let remote = self.workspace.as_ref().unwrap();
        let retry_shell = remote
            .shell_placement
            .lock()
            .unwrap()
            .as_ref()
            .map(|(_, layout)| layout.clone());
        let retry_root = cx.entity();
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(body)
            .children(remote.action_error.clone().map(|error| {
                runner_app::ui::notice_banner(error, runner_app::ui::Tone::Danger)
                    .debug_selector(|| "REMOTE_ACTION_ERROR".into())
            }))
            .children(
                retry_shell
                    .filter(|_| {
                        remote.action_error.is_some() && remote.front_page.supports(&["layout"])
                    })
                    .map(|draft| {
                        Button::new("retry-shell-placement", crate::i18n::text("ui.retry"))
                            .on_press(move |_, cx| {
                                retry_root.update(cx, |root, cx| {
                                    root.remote_start_shell(
                                        draft.clone(),
                                        None,
                                        (INITIAL_COLS, INITIAL_ROWS),
                                        cx,
                                    )
                                })
                            })
                    }),
            )
            .children((remote.connection == Connection::Reconnecting).then(|| {
                div()
                    .absolute()
                    .top(rems(74. / 16.))
                    .left_0()
                    .right_0()
                    .px_4()
                    .py_2()
                    .bg(theme::with_alpha(theme::warning(), 0.12))
                    .text_color(theme::warning())
                    .text_size(theme::text_body())
                    .child(label("client.reconnecting_banner", &remote.computer.name))
            }))
            .children(
                matches!(
                    remote.connection,
                    Connection::Unreachable | Connection::Revoked | Connection::Forgotten
                )
                .then(|| self.remote_connection_card(cx)),
            )
            .into_any_element()
    }

    pub(crate) fn render_remote_terminal_content(
        &self,
        id: &str,
        cx: &mut Context<NativeRoot>,
    ) -> AnyElement {
        let remote = self.workspace.as_ref().unwrap();
        let clients = state(&self.app_store, cx);
        let running = clients
            .read(cx)
            .overviews
            .get(&remote.computer.id)
            .and_then(|overview| overview.sessions.get(id))
            .is_some_and(|session| {
                !matches!(
                    session.status.status.effective(),
                    Status::Stopped | Status::Error
                )
            });
        let mut surface = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0();
        let Some(pane) = remote.terminals.get(id) else {
            let failed = remote.errors.contains(id);
            let owner = cx.weak_entity();
            let session = id.to_owned();
            let epoch = remote.epoch;
            let style = self.terminal_style(cx);
            let measure = gpui::canvas(
                move |bounds, window, cx| {
                    let font = window.text_system().resolve_font(&style.font);
                    let cell = window
                        .text_system()
                        .em_advance(font, px(style.font_size))
                        .unwrap_or(px(style.font_size * 0.6));
                    if let Some(size) = runner_app::terminal_resize::terminal_grid_size(
                        f32::from(bounds.size.width),
                        f32::from(bounds.size.height),
                        f32::from(cell),
                        (style.font_size * crate::terminal::element::LINE_HEIGHT_FACTOR).round(),
                    ) {
                        let owner = owner.clone();
                        let session = session.clone();
                        cx.defer(move |cx| {
                            let _ = owner.update(cx, |root, cx| {
                                if let Some(remote) = root
                                    .remote_client
                                    .workspace
                                    .as_mut()
                                    .filter(|remote| remote.epoch == epoch)
                                {
                                    let measured = (size.cols.max(20), size.rows.max(8));
                                    if remote.sizes.insert(session, measured) != Some(measured) {
                                        root.remote_client.attach_remote_panes(cx);
                                        cx.notify();
                                    }
                                }
                            });
                        });
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0();
            if !running {
                return surface.child(measure).into_any_element();
            }
            return surface
                .child(
                    div()
                        .when(failed, |pane| {
                            pane.debug_selector(|| "REMOTE_TERMINAL_FAILED".into())
                        })
                        .relative()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_col()
                        .gap_3()
                        .children(
                            (!failed).then(|| {
                                spinner("remote-terminal-connecting", 24., theme::muted())
                            }),
                        )
                        .child(measure)
                        .child(label(
                            if failed {
                                "client.terminal_failed"
                            } else {
                                "client.terminal_connecting"
                            },
                            &remote.computer.name,
                        )),
                )
                .into_any_element();
        };
        let connected = remote.connection == Connection::Connected;
        let controlled = pane.attachment.terminal.control().desktop();
        let style = self.terminal_style(cx);
        if pane.attachment.terminal.palette() != style.palette {
            pane.attachment.terminal.set_palette(style.palette);
        }
        let terminal = pane.attachment.terminal.clone();
        let key_terminal = terminal.clone();
        let copy_terminal = terminal.clone();
        let paste_terminal = terminal.clone();
        let input = pane.attachment.terminal_input.clone();
        let take = cx.weak_entity();
        let take_id = id.to_owned();
        if !controlled {
            let owner_name =
                match &terminal.control().controller {
                    runner_core::protocol::terminal::TerminalController::Browser {
                        name, ..
                    } if name != "Host desktop" => name.clone(),
                    _ => remote.computer.name.clone(),
                };
            surface = surface.child(
                div()
                    .debug_selector(|| "REMOTE_TAKEN_BACK".into())
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .bg(theme::panel())
                    .text_size(theme::text_meta())
                    .child(div().flex_1().min_w_0().child(
                        if terminal.control().controller
                            == runner_core::protocol::terminal::TerminalController::None
                        {
                            crate::i18n::text("client.no_controller").to_string()
                        } else {
                            label("client.taken_back", &owner_name)
                        },
                    ))
                    .child(
                        Button::new(
                            SharedString::from(format!("take-remote-{id}")),
                            crate::i18n::text("client.take_control"),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Secondary)
                        .disabled(!connected)
                        .on_press(move |_, cx| {
                            let _ = take.update(cx, |root, cx| {
                                root.remote_client.take_remote_control(&take_id, cx)
                            });
                        }),
                    ),
            );
        }
        if pane.control_failed {
            surface = surface.child(
                div()
                    .debug_selector(|| "REMOTE_CONTROL_FAILED".into())
                    .px_3()
                    .py_2()
                    .text_size(theme::text_meta())
                    .text_color(theme::danger())
                    .child(crate::i18n::text("client.take_control_failed")),
            );
        }
        let interactive = connected && controlled && running;
        let option_as_meta = self.settings(cx).option_as_meta;
        surface
            .child(
                div().relative().flex_1().min_h_0().min_w_0().child(
                    crate::surfaces::panes::TerminalSurface {
                        terminal,
                        interaction: pane.attachment.terminal_interaction.clone(),
                        input: pane.attachment.terminal_input.clone(),
                        focus: pane.attachment.terminal_focus.clone(),
                        scrollbar: pane.attachment.terminal_scrollbar.clone(),
                        style,
                        interactive,
                        resize_owner: interactive,
                        scrollable: true,
                        find: None,
                    }
                    .render()
                    .id(SharedString::from(format!("remote-terminal-{id}")))
                    .opacity(if interactive { 1. } else { 0.45 })
                    .on_key_down(move |event: &KeyDownEvent, _, cx| {
                        if !interactive {
                            cx.stop_propagation();
                            return;
                        }
                        let option_is_meta = option_as_meta.is_meta(cx);
                        let _ = crate::terminal::input::handle_key(
                            &key_terminal,
                            input.read(cx).is_composing(),
                            &event.keystroke,
                            option_is_meta,
                            cx,
                        );
                    })
                    .on_action(move |_: &Copy, _, cx| {
                        if !crate::terminal::input::copy_selection(&copy_terminal, cx) {
                            #[cfg(any(windows, target_os = "linux"))]
                            cx.propagate();
                        }
                    })
                    .on_action(move |_: &Paste, _, cx| {
                        if interactive {
                            let _ = crate::terminal::input::paste_text_from_clipboard(
                                &paste_terminal,
                                cx,
                            );
                        }
                        cx.stop_propagation();
                    }),
                ),
            )
            .into_any_element()
    }

    pub(in crate::surfaces::remote_client) fn take_remote_control(
        &mut self,
        id: &str,
        cx: &mut Context<NativeRoot>,
    ) {
        let Some(remote) = self.workspace.as_mut() else {
            return;
        };
        let Some(pane) = remote.terminals.get_mut(id) else {
            return;
        };
        let terminal = pane.attachment.terminal.clone();
        let (cols, rows) = pane
            .attachment
            .terminal_interaction
            .read(cx)
            .pane_size
            .map(|size| (size.cols, size.rows))
            .unwrap_or_else(|| terminal.size());
        let epoch = remote.epoch;
        pane.control_failed = false;
        let taking = terminal.clone();
        let task = cx.background_spawn(async move { taking.take_back(cols, rows) });
        let id = id.to_owned();
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |root, cx| {
                let Some(remote) = root
                    .remote_client
                    .workspace
                    .as_mut()
                    .filter(|remote| remote.epoch == epoch)
                else {
                    return;
                };
                let Some(pane) = remote
                    .terminals
                    .get_mut(&id)
                    .filter(|pane| Arc::ptr_eq(&pane.attachment.terminal, &terminal))
                else {
                    return;
                };
                pane.control_failed = result.is_err();
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn remote_connection_card(&self, cx: &mut Context<NativeRoot>) -> AnyElement {
        let remote = self.workspace.as_ref().unwrap();
        let revoked = remote.connection == Connection::Revoked;
        let forgotten = remote.connection == Connection::Forgotten;
        let pair = revoked || forgotten;
        let retry = cx.weak_entity();
        let close = cx.weak_entity();
        let computer = remote.computer.clone();
        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::with_alpha(theme::bg(), 0.7))
            .p_6()
            .child(
                div()
                    .debug_selector(|| "REMOTE_CONNECTION_CARD".into())
                    .w_full()
                    .max_w(rems(440. / 16.))
                    .rounded_xl()
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .p_6()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_4()
                    .text_center()
                    .child(icon(
                        if revoked {
                            "shield-x.svg"
                        } else {
                            "cloud-off.svg"
                        },
                        if revoked {
                            theme::danger()
                        } else {
                            theme::muted()
                        },
                    ))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(theme::text_lead())
                            .child(label(
                                if forgotten {
                                    "client.forgotten_title"
                                } else if revoked {
                                    "client.revoked_title"
                                } else {
                                    "client.unreachable_title"
                                },
                                &computer.name,
                            )),
                    )
                    .child(
                        div()
                            .text_size(theme::text_body())
                            .text_color(theme::muted())
                            .child(label(
                                if forgotten {
                                    "client.forgotten_detail"
                                } else if revoked {
                                    "client.revoked_detail"
                                } else {
                                    "client.unreachable_detail"
                                },
                                &computer.name,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                Button::new(
                                    "remote-reconnect",
                                    crate::i18n::text(if forgotten {
                                        "client.pair"
                                    } else if revoked {
                                        "client.pair_again"
                                    } else {
                                        "client.reconnect"
                                    }),
                                )
                                .debug_selector("REMOTE_RECONNECT_ACTION")
                                .on_press(move |window, cx| {
                                    let _ = retry.update(cx, |root, cx| {
                                        if pair {
                                            root.remote_client.open_computer_pair(
                                                computer.clone(),
                                                pairing::Purpose::Open { new_window: false },
                                                window,
                                                cx,
                                            );
                                        } else {
                                            let clients = state(&root.app_store, cx);
                                            clients.update(cx, |clients, cx| {
                                                clients.connections.insert(
                                                    computer.id.clone(),
                                                    Connection::Opening,
                                                );
                                                clients.send(Command::Retry(computer.id.clone()));
                                                cx.notify();
                                            });
                                        }
                                    });
                                }),
                            )
                            .child(
                                Button::new("remote-close-view", crate::i18n::text("ui.close_tab"))
                                    .variant(ButtonVariant::Ghost)
                                    .on_press(move |window, cx| {
                                        let _ = close.update(cx, |root, cx| {
                                            root.return_to_this_computer(window, cx)
                                        });
                                    }),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn refresh_remote_palette(
        &self,
        palette: &Entity<crate::surfaces::command_palette::CommandPaletteState>,
        cx: &mut Context<NativeRoot>,
    ) {
        let data = self.workspace.as_ref().map(|remote| {
            let overview = self
                .app_store
                .read(cx)
                .remote_clients
                .read(cx)
                .overviews
                .get(&remote.computer.id)
                .cloned();
            (overview, remote.front_page.clone())
        });
        palette.update(cx, |palette, cx| palette.set_remote_data(data, cx));
    }
}
