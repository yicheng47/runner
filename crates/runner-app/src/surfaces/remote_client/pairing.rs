use super::*;
use crate::surfaces::settings::remote::pin::PinField;
use runner_core::remote::Secret;

#[derive(Clone, Copy)]
pub(super) enum Purpose {
    PairOnly,
    Open { new_window: bool },
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PairState {
    Entry,
    Checking,
    WrongPin,
    Locked,
    NoPin,
    Offline,
    TimedOut,
    Unreachable,
    Failed,
}
impl PairState {
    pub(super) fn refusal(error: &str) -> Self {
        if error.contains("wrongPin") {
            Self::WrongPin
        } else if error.contains("pinLocked") {
            Self::Locked
        } else if error.contains("noPin") {
            Self::NoPin
        } else if error.contains("offline (4409)") {
            Self::Offline
        } else if error.contains("Pairing timed out") {
            Self::TimedOut
        } else if [
            "Remote service is unavailable",
            "Remote route refused",
            "Remote route closed",
            "Could not list computers",
        ]
        .iter()
        .any(|message| error.contains(message))
        {
            Self::Unreachable
        } else {
            Self::Failed
        }
    }
    fn accepts_pin(self) -> bool {
        matches!(
            self,
            Self::Entry
                | Self::WrongPin
                | Self::Offline
                | Self::TimedOut
                | Self::Unreachable
                | Self::Failed
        )
    }
    fn retry(self) -> bool {
        matches!(
            self,
            Self::Offline | Self::TimedOut | Self::Unreachable | Self::Failed
        )
    }
}
pub(crate) struct PairDialog {
    pub(super) computer: Computer,
    purpose: Purpose,
    pub(super) field: Entity<PinField>,
    pub(super) state: PairState,
    pub(super) again: bool,
    changed: bool,
    focus: FocusHandle,
    secondary: FocusHandle,
    primary: FocusHandle,
    generation: u64,
    _subscription: Subscription,
}

impl RemoteClient {
    pub(super) fn open_computer_pair(
        &mut self,
        computer: Computer,
        purpose: Purpose,
        window: &mut Window,
        cx: &mut Context<NativeRoot>,
    ) {
        let clients = state(&self.app_store, cx);
        let Some(computer) = clients
            .read(cx)
            .computers
            .iter()
            .find(|row| row.id == computer.id)
            .cloned()
        else {
            self.refresh_computers(cx);
            return;
        };
        let changed = clients.read(cx).changed_identities.contains(&computer.id);
        let again = changed || clients.read(cx).connection(&computer.id) == Connection::Revoked;
        let field = cx.new(PinField::new);
        let primary = cx.focus_handle();
        let state = if computer.online {
            PairState::Entry
        } else {
            PairState::Offline
        };
        if state == PairState::Offline {
            primary.focus(window, cx);
        } else {
            field.read(cx).focus.clone().focus(window, cx);
        }
        self.ui.pair_generation += 1;
        self.ui.picker_open = false;
        self.ui.pair = Some(PairDialog {
            computer,
            purpose,
            state,
            again,
            changed,
            _subscription: cx.observe(&field, |_, _, cx| cx.notify()),
            field,
            focus: cx.focus_handle(),
            secondary: cx.focus_handle(),
            primary,
            generation: self.ui.pair_generation,
        });
        cx.notify();
    }
    pub(super) fn sync_computer_pair(&mut self, cx: &mut Context<NativeRoot>) -> bool {
        let Some(dialog) = self.ui.pair.as_ref() else {
            return false;
        };
        let clients = state(&self.app_store, cx);
        let Some(computer) = clients
            .read(cx)
            .computers
            .iter()
            .find(|row| row.id == dialog.computer.id)
            .cloned()
        else {
            self.ui.pair = None;
            self.ui.pair_generation += 1;
            return true;
        };
        let identity_changed = computer.fingerprint != dialog.computer.fingerprint;
        if !identity_changed {
            let dialog = self.ui.pair.as_mut().unwrap();
            dialog.computer = computer.clone();
            if dialog.state != PairState::Offline
                || !dialog.computer.online
                || dialog.field.read(cx).value().0.len() == 6
            {
                return false;
            }
        }
        self.ui.pair_generation += 1;
        let generation = self.ui.pair_generation;
        let field = if identity_changed {
            cx.new(PinField::new)
        } else {
            self.ui.pair.as_ref().unwrap().field.clone()
        };
        let dialog = self.ui.pair.as_mut().unwrap();
        if identity_changed {
            dialog.computer = computer;
            dialog.again = true;
            dialog.changed = true;
        }
        dialog.generation = generation;
        dialog.state = PairState::Entry;
        dialog.field = field.clone();
        dialog._subscription = cx.observe(&field, |_, _, cx| cx.notify());
        cx.spawn(async move |weak, cx| {
            let _ = weak.update_in(cx, |root, window, cx| {
                if let Some(dialog) = root
                    .remote_client
                    .ui
                    .pair
                    .as_ref()
                    .filter(|dialog| dialog.generation == generation)
                {
                    dialog.field.read(cx).focus.clone().focus(window, cx);
                }
            });
        })
        .detach();
        cx.notify();
        true
    }
    fn retry_computer_pair(&mut self, window: &mut Window, cx: &mut Context<NativeRoot>) {
        if self.ui.pair.as_ref().is_some_and(|dialog| {
            dialog.state == PairState::Offline && dialog.field.read(cx).value().0.len() != 6
        }) {
            self.refresh_computers(cx);
        } else {
            self.submit_computer_pin(window, cx);
        }
    }
    pub(super) fn submit_computer_pin(
        &mut self,
        window: &mut Window,
        cx: &mut Context<NativeRoot>,
    ) {
        if self.sync_computer_pair(cx) {
            return;
        }
        let Some(dialog) = self.ui.pair.as_mut() else {
            return;
        };
        if !dialog.state.accepts_pin() || dialog.field.read(cx).value().0.len() != 6 {
            return;
        }
        let pin = Secret(dialog.field.read(cx).value().0.clone());
        let computer = dialog.computer.clone();
        let generation = dialog.generation;
        let purpose = dialog.purpose;
        let again = dialog.again;
        dialog.state = PairState::Checking;
        dialog.secondary.focus(window, cx);
        dialog.field.update(cx, |field, cx| {
            field.disabled = true;
            field.invalid = false;
            cx.notify();
        });
        let client = self.core(cx).clone();
        let target = computer.id.clone();
        let expected_fingerprint = computer.fingerprint.clone();
        let task = cx.background_spawn(async move {
            let current = client
                .remote_computers()?
                .into_iter()
                .find(|row| row.id == target);
            if !current.is_some_and(|row| row.fingerprint == expected_fingerprint) {
                return Err(runner_core::protocol::ClientError::msg(
                    "Remote computer identity changed before pairing.",
                ));
            }
            if again {
                client.remote_forget(target.clone())?;
            }
            client.remote_pair(target, pin)
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |root, window, cx| {
                if result.is_ok() {
                    state(&root.app_store, cx).update(cx, |clients, cx| {
                        if let Some(row) = clients.computers.iter_mut().find(|row| {
                            row.id == computer.id && row.fingerprint == computer.fingerprint
                        }) {
                            row.paired = true;
                        }
                        if clients.computers.iter().any(|row| {
                            row.id == computer.id && row.fingerprint == computer.fingerprint
                        }) {
                            clients.changed_identities.remove(&computer.id);
                            clients.connections.remove(&computer.id);
                            clients.invalidate_capabilities(&computer.id);
                            let epoch = clients.pair_epochs.entry(computer.id.clone()).or_default();
                            *epoch = epoch.wrapping_add(1);
                            clients.send(Command::Paired(computer.id.clone()));
                        }
                        clients.refresh(cx);
                    });
                }
                root.remote_client.sync_computer_pair(cx);
                if !root
                    .remote_client
                    .ui
                    .pair
                    .as_ref()
                    .is_some_and(|dialog| dialog.generation == generation)
                {
                    return;
                }
                match result {
                    Ok(()) => {
                        root.remote_client.ui.pair = None;
                        if let Purpose::Open { new_window } = purpose {
                            let mut computer = computer;
                            computer.paired = true;
                            root.remote_client.open_computer(computer, new_window, cx);
                        }
                    }
                    Err(error) => {
                        let dialog = root.remote_client.ui.pair.as_mut().unwrap();
                        let identity_changed =
                            error.message == "Remote computer identity changed before pairing.";
                        dialog.state = PairState::refusal(&error.message);
                        let field = if dialog.state == PairState::WrongPin {
                            let field = cx.new(PinField::new);
                            dialog.field = field.clone();
                            dialog._subscription = cx.observe(&field, |_, _, cx| cx.notify());
                            field
                        } else {
                            dialog.field.clone()
                        };
                        field.update(cx, |field, cx| {
                            field.disabled = false;
                            field.invalid = dialog.state == PairState::WrongPin;
                            cx.notify();
                        });
                        if dialog.state == PairState::WrongPin {
                            field.read(cx).focus.clone().focus(window, cx);
                        } else {
                            dialog.primary.focus(window, cx);
                        }
                        if identity_changed {
                            root.remote_client.refresh_computers(cx);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(crate) fn render_computer_pair(&self, cx: &mut Context<NativeRoot>) -> Option<AnyElement> {
        let dialog = self.ui.pair.as_ref()?;
        let name = &dialog.computer.name;
        let blocked = matches!(dialog.state, PairState::Locked | PairState::NoPin);
        let offline = dialog.state == PairState::Offline;
        let checking = dialog.state == PairState::Checking;
        let title = label(
            match dialog.state {
                PairState::Locked => "client.pin_locked_title",
                PairState::NoPin => "client.no_pin_title",
                PairState::Offline => "client.pair_offline_title",
                _ if dialog.again => "client.pair_again_title",
                _ => "client.pair_title",
            },
            name,
        );
        let detail = label(
            match dialog.state {
                PairState::Locked => "client.pin_locked_detail",
                PairState::NoPin => "client.no_pin_detail",
                PairState::Offline => "client.pair_offline_detail",
                _ if dialog.again => "client.pair_again_detail",
                _ => "client.pair_detail",
            },
            name,
        );
        let hint = match dialog.state {
            PairState::Checking => label("client.checking_pin", name),
            PairState::WrongPin => label("client.wrong_pin", name),
            PairState::Offline => status_label(&dialog.computer, false),
            PairState::TimedOut => label("client.pair_timed_out", name),
            PairState::Unreachable => label("client.pair_unreachable", name),
            PairState::Failed => label("client.pair_failed", name),
            _ => crate::i18n::tf(
                if dialog.changed {
                    "client.fingerprint_changed"
                } else {
                    "client.fingerprint"
                },
                &[
                    ("computer", name.clone()),
                    ("fingerprint", dialog.computer.fingerprint.clone()),
                ],
            ),
        };
        let cancel = cx.weak_entity();
        let close = cx.weak_entity();
        let submit = cx.weak_entity();
        let complete = dialog.field.read(cx).value().0.len() == 6;
        let field = dialog.field.clone();
        let mut stops = if blocked || checking || offline {
            Vec::new()
        } else {
            vec![field.read(cx).focus.clone()]
        };
        if !blocked {
            stops.push(dialog.secondary.clone());
        }
        if blocked || offline || (complete && !checking) {
            stops.push(dialog.primary.clone());
        }
        let focus = dialog.focus.clone();
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                .bg(theme::scrim())
                .track_focus(&focus)
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    let _ = cancel.update(cx, |root, cx| {
                        root.remote_client.ui.pair = None;
                        root.root_focus.focus(window, cx);
                        cx.notify();
                    });
                })
                .on_key_down(cx.listener(move |root, event: &KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            root.remote_client.ui.pair = None;
                            root.root_focus.focus(window, cx);
                            cx.stop_propagation();
                            cx.notify();
                        }
                        "enter" if !event.is_held => {
                            if blocked {
                                root.remote_client.ui.pair = None;
                                cx.notify();
                            } else {
                                root.remote_client.retry_computer_pair(window, cx);
                            }
                            cx.stop_propagation();
                        }
                        "tab" if !stops.is_empty() => {
                            let current = stops
                                .iter()
                                .position(|stop| stop.is_focused(window))
                                .unwrap_or(0);
                            let next = if event.keystroke.modifiers.shift {
                                (current + stops.len() - 1) % stops.len()
                            } else {
                                (current + 1) % stops.len()
                            };
                            stops[next].focus(window, cx);
                            cx.stop_propagation();
                        }
                        _ => {}
                    }
                }))
                .child(
                    div()
                        .debug_selector(|| "COMPUTER_PAIR_DIALOG".into())
                        .w_full()
                        .max_w(rems(420. / 16.))
                        .flex()
                        .flex_col()
                        .gap_4()
                        .p(rems(22. / 16.))
                        .rounded(rems(12. / 16.))
                        .border_1()
                        .border_color(theme::border())
                        .bg(theme::panel())
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .text_size(theme::text_lead())
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(icon(
                                    if blocked { "lock.svg" } else { "monitor.svg" },
                                    theme::muted(),
                                ))
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(theme::text_body())
                                .line_height(rems(20. / 16.))
                                .text_color(theme::muted())
                                .child(detail),
                        )
                        .when(!blocked && !offline, |body| {
                            body.child(
                                div()
                                    .debug_selector(|| "COMPUTER_PAIR_PIN".into())
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_size(theme::text_body())
                                            .text_color(theme::muted())
                                            .child(crate::i18n::text("client.pin")),
                                    )
                                    .child(field),
                            )
                        })
                        .when(!blocked, |body| {
                            body.child(
                                div()
                                    .text_size(theme::text_meta())
                                    .text_color(if dialog.state == PairState::WrongPin {
                                        theme::danger()
                                    } else if dialog.changed || (dialog.state.retry() && !offline) {
                                        theme::warning()
                                    } else {
                                        theme::faint()
                                    })
                                    .child(hint),
                            )
                        })
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .when(!blocked, |actions| {
                                    actions.child(
                                        Button::new(
                                            "cancel-computer-pair",
                                            crate::i18n::text("ui.cancel"),
                                        )
                                        .variant(ButtonVariant::Secondary)
                                        .focus_handle(dialog.secondary.clone())
                                        .on_press(
                                            move |window, cx| {
                                                let _ = close.update(cx, |root, cx| {
                                                    root.remote_client.ui.pair = None;
                                                    root.root_focus.focus(window, cx);
                                                    cx.notify();
                                                });
                                            },
                                        ),
                                    )
                                })
                                .child(
                                    Button::new(
                                        "submit-computer-pair",
                                        crate::i18n::text(if blocked {
                                            "ui.close"
                                        } else if checking {
                                            "client.pairing"
                                        } else if dialog.state.retry() {
                                            "remote.try_again"
                                        } else {
                                            "client.pair"
                                        }),
                                    )
                                    .focus_handle(dialog.primary.clone())
                                    .debug_selector("COMPUTER_PAIR_PRIMARY")
                                    .disabled(!blocked && !offline && (!complete || checking))
                                    .on_press(
                                        move |window, cx| {
                                            let _ = submit.update(cx, |root, cx| {
                                                if blocked {
                                                    root.remote_client.ui.pair = None;
                                                    root.root_focus.focus(window, cx);
                                                    cx.notify();
                                                } else {
                                                    root.remote_client
                                                        .retry_computer_pair(window, cx);
                                                }
                                            });
                                        },
                                    ),
                                ),
                        ),
                )
                .map(crate::platform_ui::window_scrim)
                .into_any_element(),
        )
    }
}
