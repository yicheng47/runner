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
        } else {
            Self::Failed
        }
    }
    fn accepts_pin(self) -> bool {
        matches!(self, Self::Entry | Self::WrongPin | Self::Failed)
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

impl NativeRoot {
    pub(super) fn open_computer_pair(
        &mut self,
        computer: Computer,
        purpose: Purpose,
        window: &mut Window,
        cx: &mut Context<Self>,
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
        field.read(cx).focus.clone().focus(window, cx);
        self.client_ui.pair_generation += 1;
        self.client_ui.picker_open = false;
        self.client_ui.pair = Some(PairDialog {
            computer,
            purpose,
            state: PairState::Entry,
            again,
            changed,
            _subscription: cx.observe(&field, |_, _, cx| cx.notify()),
            field,
            focus: cx.focus_handle(),
            secondary: cx.focus_handle(),
            primary: cx.focus_handle(),
            generation: self.client_ui.pair_generation,
        });
        cx.notify();
    }
    pub(super) fn sync_computer_pair(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(dialog) = self.client_ui.pair.as_ref() else {
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
            self.client_ui.pair = None;
            self.client_ui.pair_generation += 1;
            return true;
        };
        if computer.fingerprint == dialog.computer.fingerprint {
            return false;
        }
        self.client_ui.pair_generation += 1;
        let generation = self.client_ui.pair_generation;
        let field = cx.new(PinField::new);
        let dialog = self.client_ui.pair.as_mut().unwrap();
        dialog.computer = computer;
        dialog.generation = generation;
        dialog.again = true;
        dialog.changed = true;
        dialog.state = PairState::Entry;
        dialog.field = field.clone();
        dialog._subscription = cx.observe(&field, |_, _, cx| cx.notify());
        cx.spawn(async move |weak, cx| {
            let _ = weak.update_in(cx, |root, window, cx| {
                if let Some(dialog) = root
                    .client_ui
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
    pub(super) fn submit_computer_pin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sync_computer_pair(cx) {
            return;
        }
        let Some(dialog) = self.client_ui.pair.as_mut() else {
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
                            let epoch = clients.pair_epochs.entry(computer.id.clone()).or_default();
                            *epoch = epoch.wrapping_add(1);
                            clients.send(Command::Paired(computer.id.clone()));
                        }
                        clients.refresh(cx);
                    });
                }
                root.sync_computer_pair(cx);
                if !root
                    .client_ui
                    .pair
                    .as_ref()
                    .is_some_and(|dialog| dialog.generation == generation)
                {
                    return;
                }
                match result {
                    Ok(()) => {
                        root.client_ui.pair = None;
                        if let Purpose::Open { new_window } = purpose {
                            let mut computer = computer;
                            computer.paired = true;
                            root.open_computer(computer, new_window, cx);
                        }
                    }
                    Err(error) => {
                        let field = cx.new(PinField::new);
                        let dialog = root.client_ui.pair.as_mut().unwrap();
                        let identity_changed =
                            error.message == "Remote computer identity changed before pairing.";
                        dialog.state = PairState::refusal(&error.message);
                        field.update(cx, |field, _| {
                            field.invalid = dialog.state == PairState::WrongPin
                        });
                        dialog.field = field.clone();
                        dialog._subscription = cx.observe(&field, |_, _, cx| cx.notify());
                        if dialog.state.accepts_pin() {
                            field.read(cx).focus.clone().focus(window, cx);
                        } else {
                            dialog.primary.focus(window, cx);
                        }
                        if identity_changed {
                            root.refresh_computers(cx);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(crate) fn render_computer_pair(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.app_store.read(cx).remote_access_enabled() {
            return None;
        }
        let dialog = self.client_ui.pair.as_ref()?;
        let name = &dialog.computer.name;
        let blocked = matches!(dialog.state, PairState::Locked | PairState::NoPin);
        let checking = dialog.state == PairState::Checking;
        let title = label(
            match dialog.state {
                PairState::Locked => "client.pin_locked_title",
                PairState::NoPin => "client.no_pin_title",
                _ if dialog.again => "client.pair_again_title",
                _ => "client.pair_title",
            },
            name,
        );
        let detail = label(
            match dialog.state {
                PairState::Locked => "client.pin_locked_detail",
                PairState::NoPin => "client.no_pin_detail",
                _ if dialog.again => "client.pair_again_detail",
                _ => "client.pair_detail",
            },
            name,
        );
        let hint = match dialog.state {
            PairState::Checking => label("client.checking_pin", name),
            PairState::WrongPin => label("client.wrong_pin", name),
            PairState::Failed => crate::i18n::text("client.pair_failed").into(),
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
        let mut stops = if blocked || checking {
            Vec::new()
        } else {
            vec![field.read(cx).focus.clone()]
        };
        if !blocked {
            stops.push(dialog.secondary.clone());
        }
        if blocked || (complete && !checking) {
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
                        root.client_ui.pair = None;
                        root.root_focus.focus(window, cx);
                        cx.notify();
                    });
                })
                .on_key_down(cx.listener(move |root, event: &KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            root.client_ui.pair = None;
                            root.root_focus.focus(window, cx);
                            cx.stop_propagation();
                            cx.notify();
                        }
                        "enter" if !event.is_held => {
                            if blocked {
                                root.client_ui.pair = None;
                                cx.notify();
                            } else {
                                root.submit_computer_pin(window, cx);
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
                        .when(!blocked, |body| {
                            body.child(
                                div()
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
                            .child(
                                div()
                                    .text_size(theme::text_meta())
                                    .text_color(if dialog.state == PairState::WrongPin {
                                        theme::danger()
                                    } else if dialog.changed {
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
                                                    root.client_ui.pair = None;
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
                                        } else {
                                            "client.pair"
                                        }),
                                    )
                                    .focus_handle(dialog.primary.clone())
                                    .disabled(!blocked && (!complete || checking))
                                    .on_press(
                                        move |window, cx| {
                                            let _ = submit.update(cx, |root, cx| {
                                                if blocked {
                                                    root.client_ui.pair = None;
                                                    root.root_focus.focus(window, cx);
                                                    cx.notify();
                                                } else {
                                                    root.submit_computer_pin(window, cx);
                                                }
                                            });
                                        },
                                    ),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}
