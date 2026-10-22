use crate::surfaces::settings_page::{settings_section_heading, SettingsPane};
use crate::{theme, NativeRoot};
use gpui::prelude::*;
use gpui::{div, rems, svg, AnyElement, Context, FontWeight, SharedString};
use runner_app::ui::{
    Button, ButtonSize, ButtonVariant, ConfirmDialog, PaneHeader, SettingsCard, SettingsRow, Toggle,
};
use runner_core::{
    protocol::DaemonClient,
    remote::{RemoteState, RemoteStatus, RemoteTrust, Secret, StopReason},
};
use std::rc::Rc;

pub(crate) mod account;
mod pin;
use pin::PinField;

pub(crate) struct RemotePaneState {
    busy: bool,
    opening: bool,
    request_generation: u64,
    error: Option<String>,
    pub(crate) confirm: bool,
    reset_identity: bool,
    pin: Option<PinDialog>,
    pub(crate) focus: gpui::FocusHandle,
    generation: u64,
}
struct PinDialog {
    fields: [gpui::Entity<PinField>; 2],
    step: usize,
    secondary_focus: gpui::FocusHandle,
    primary_focus: gpui::FocusHandle,
    _subscriptions: Vec<gpui::Subscription>,
}
impl RemotePaneState {
    pub(crate) fn new(cx: &mut Context<NativeRoot>) -> Self {
        Self {
            busy: false,
            opening: false,
            request_generation: 0,
            error: None,
            confirm: false,
            reset_identity: false,
            pin: None,
            focus: cx.focus_handle(),
            generation: 0,
        }
    }
}
pub(crate) fn warn_pin_lock(previous: Option<&RemoteTrust>, next: &RemoteTrust) -> bool {
    // The daemon publishes an empty snapshot before restoring its secret store.
    previous.is_some_and(|p| p.pin_set && !p.pin_locked) && next.pin_locked
}
fn confirmed_pin(first: &Secret, second: &Secret) -> Result<Secret, &'static str> {
    if first.0.len() != 6 || !first.0.bytes().all(|b| b.is_ascii_digit()) {
        return Err("remote.pin_invalid");
    }
    if first.0 != second.0 {
        return Err("remote.pin_mismatch");
    }
    Ok(first.clone())
}
pub(crate) fn warn_transition(previous: Option<&RemoteStatus>, next: &RemoteStatus) -> bool {
    previous.is_some_and(|p| p.state != next.state)
        && matches!(
            next.state,
            RemoteState::Stopped {
                reason: StopReason::Removed
                    | StopReason::InUseElsewhere
                    | StopReason::UpdateRequired
            }
        )
}
pub(crate) fn stopped_text(status: &RemoteStatus) -> String {
    let key = match status.state {
        RemoteState::Stopped {
            reason: StopReason::Removed,
        } => "remote.removed",
        RemoteState::Stopped {
            reason: StopReason::InUseElsewhere,
        } => "remote.in_use",
        RemoteState::Stopped {
            reason: StopReason::UpdateRequired,
        } => "remote.update_required",
        RemoteState::Stopped {
            reason: StopReason::Denied,
        } => "remote.denied",
        _ => "remote.expired",
    };
    tr(
        key,
        &[(
            "account",
            status
                .account
                .as_ref()
                .map(|a| a.username.as_str())
                .unwrap_or("unknown"),
        )],
    )
}
fn title(status: &RemoteStatus) -> String {
    let key = match status.state {
        RemoteState::Off => "remote.off",
        RemoteState::Enrolling { .. } => "remote.waiting",
        RemoteState::Confirming { .. } => "remote.confirming",
        RemoteState::Connecting { .. } => "remote.connecting",
        RemoteState::Connected { .. } => "remote.connected",
        RemoteState::Reconnecting { .. } => "remote.reconnecting",
        RemoteState::Paused => "remote.paused",
        RemoteState::Stopped { .. } => "remote.stopped",
    };
    tr(
        key,
        &[(
            "account",
            status
                .account
                .as_ref()
                .map(|a| a.username.as_str())
                .unwrap_or("unknown"),
        )],
    )
}
fn detail(label: &'static str, value: impl Into<SharedString>, mono: bool) -> AnyElement {
    let mut content = div().flex().flex_col().gap_1().flex_1().child(
        div()
            .text_size(theme::text_meta())
            .text_color(theme::faint())
            .child(crate::i18n::text(label)),
    );
    let mut text = div().text_size(theme::text_body()).child(value.into());
    if mono {
        text = text.font_family(theme::SYSTEM_MONOSPACE_FONT);
    }
    content = content.child(text);
    content.into_any_element()
}
impl NativeRoot {
    pub(crate) fn enter_remote_settings(&mut self, cx: &mut Context<Self>) {
        self.remote_request(false, |c| c.remote_status(), cx);
        self.settings_page.remote.generation += 1;
        let generation = self.settings_page.remote.generation;
        cx.spawn(async move |weak, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(1))
                .await;
            let keep = weak
                .update(cx, |root, cx| {
                    if root.route != crate::AppRoute::Settings
                        || root.settings_page.remote.generation != generation
                        || !matches!(
                            root.settings_page.pane,
                            SettingsPane::Account | SettingsPane::Remote
                        )
                    {
                        return false;
                    }
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !keep {
                break;
            }
        })
        .detach();
    }
    fn remote_request(
        &mut self,
        open_browser: bool,
        request: impl FnOnce(DaemonClient) -> Result<RemoteStatus, runner_core::protocol::ClientError>
            + Send
            + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.settings_page.remote.busy {
            return;
        }
        self.settings_page.remote.busy = true;
        self.settings_page.remote.opening = open_browser;
        self.settings_page.remote.request_generation += 1;
        let generation = self.settings_page.remote.request_generation;
        self.settings_page.remote.error = None;
        let client = self.core(cx).clone();
        let task = cx.background_spawn(async move { request(client) });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |root, cx| {
                if root.settings_page.remote.request_generation != generation {
                    return;
                }
                root.settings_page.remote.busy = false;
                match result {
                    Ok(status) => {
                        if open_browser {
                            if let RemoteState::Enrolling {
                                verification_uri_complete,
                                ..
                            } = &status.state
                            {
                                cx.open_url(verification_uri_complete);
                            }
                        }
                    }
                    Err(error) => root.settings_page.remote.error = Some(error.message),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn remote_connect(&mut self, cx: &mut Context<Self>) {
        self.remote_request(true, |c| c.remote_enroll_start(None, false), cx);
    }
    fn remote_button(
        &self,
        id: &'static str,
        key: &'static str,
        action: fn(&mut Self, &mut Context<Self>),
        cx: &Context<Self>,
    ) -> Button {
        let root = cx.entity();
        Button::new(id, crate::i18n::text(key))
            .size(ButtonSize::Sm)
            .variant(ButtonVariant::Secondary)
            .disabled(self.settings_page.remote.busy)
            .on_press(move |window, cx| {
                root.update(cx, |r, cx| {
                    action(r, cx);
                    if r.settings_page.remote.confirm {
                        r.settings_page.remote.focus.focus(window, cx);
                    } else if let Some(dialog) = &r.settings_page.remote.pin {
                        dialog.fields[dialog.step]
                            .read(cx)
                            .focus
                            .clone()
                            .focus(window, cx);
                    }
                })
            })
    }
    pub(crate) fn render_remote_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let status = self
            .app_store
            .read(cx)
            .remote_status
            .clone()
            .unwrap_or_default();
        let trust = self
            .app_store
            .read(cx)
            .remote_trust
            .clone()
            .unwrap_or_default();
        let stopped = matches!(status.state, RemoteState::Stopped { .. });
        let connected = matches!(status.state, RemoteState::Connected { .. });
        let mut card = div()
            .debug_selector(|| "REMOTE_STATUS_CARD".into())
            .flex()
            .flex_col()
            .rounded_lg()
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .p_5()
            .gap_4()
            .text_size(theme::text_body());
        card = card.child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(if matches!(status.state, RemoteState::Off) {
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_none()
                        .size(rems(36. / 16.))
                        .rounded(rems(10. / 16.))
                        .bg(theme::raised())
                        .child(
                            svg()
                                .path("globe.svg")
                                .size(rems(18. / 16.))
                                .text_color(theme::muted()),
                        )
                } else {
                    div().size(rems(8. / 16.)).rounded_full().bg(if connected {
                        theme::accent()
                    } else if stopped {
                        theme::warning()
                    } else {
                        theme::muted()
                    })
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(theme::text_lead())
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title(&status)),
                        )
                        .when(matches!(status.state, RemoteState::Off), |header| {
                            header.child(
                                div()
                                    .text_size(theme::text_body())
                                    .text_color(theme::muted())
                                    .child(crate::i18n::text("remote.intro")),
                            )
                        }),
                ),
        );
        match &status.state {
            RemoteState::Off => {
                card = card
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(10. / 16.))
                            .rounded_lg()
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::bg())
                            .py(rems(10. / 16.))
                            .px_3()
                            .text_color(theme::muted())
                            .child(svg().path("ticket.svg").flex_none().size(rems(15. / 16.)))
                            .child(
                                div()
                                    .flex()
                                    .flex_none()
                                    .gap(rems(0.4 / 16.))
                                    .bg(theme::raised())
                                    .rounded(rems(4. / 16.))
                                    .py(rems(2. / 16.))
                                    .px(rems(6. / 16.))
                                    .text_size(theme::text_micro())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .children(
                                        crate::i18n::text("remote.private_beta")
                                            .chars()
                                            .map(|letter| div().child(letter.to_string())),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(theme::text_ui())
                                    .child(crate::i18n::text("remote.beta_detail")),
                            ),
                    )
                    .child(
                        div().flex().items_start().gap_3().children(
                            [
                                ("1", "remote.step_connect", "remote.step_connect_detail"),
                                ("2", "remote.step_approve", "remote.step_approve_detail"),
                                ("3", "remote.step_anywhere", "remote.step_anywhere_detail"),
                            ]
                            .into_iter()
                            .map(|(number, label, value)| {
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .items_start()
                                    .gap_2()
                                    .py(rems(10. / 16.))
                                    .px_3()
                                    .rounded_lg()
                                    .border_1()
                                    .border_color(theme::border())
                                    .bg(theme::bg())
                                    .child(
                                        div()
                                            .flex()
                                            .flex_none()
                                            .items_center()
                                            .justify_center()
                                            .size(rems(20. / 16.))
                                            .rounded_full()
                                            .bg(theme::raised())
                                            .text_size(theme::text_meta())
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme::muted())
                                            .child(number),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .text_size(theme::text_ui())
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .child(crate::i18n::text(label)),
                                            )
                                            .child(
                                                div()
                                                    .text_color(theme::faint())
                                                    .text_size(theme::text_meta())
                                                    .child(crate::i18n::text(value)),
                                            ),
                                    )
                            }),
                        ),
                    )
                    .child(
                        div().flex().items_start().child(
                            div()
                                .debug_selector(|| "REMOTE_CONNECT_BUTTON".into())
                                .child(
                                    self.remote_button(
                                        "remote-connect",
                                        "remote.connect",
                                        Self::remote_connect,
                                        cx,
                                    )
                                    .size(ButtonSize::Compact)
                                    .radius(6.)
                                    .icon("plug-zap.svg")
                                    .variant(ButtonVariant::Primary),
                                ),
                        ),
                    );
                if self.settings_page.remote.busy && self.settings_page.remote.opening {
                    let root = cx.entity();
                    card = card.child(
                        div().flex().child(
                            Button::new("remote-cancel-start", crate::i18n::text("ui.cancel"))
                                .size(ButtonSize::Sm)
                                .variant(ButtonVariant::Secondary)
                                .on_press(move |_, cx| {
                                    root.update(cx, |r, cx| {
                                        r.settings_page.remote.busy = false;
                                        r.remote_request(
                                            false,
                                            |c| {
                                                c.remote_logout()?;
                                                c.remote_status()
                                            },
                                            cx,
                                        );
                                    })
                                }),
                        ),
                    );
                }
            }
            RemoteState::Enrolling {
                user_code,
                verification_uri,
                verification_uri_complete,
                expires_at,
            } => {
                let secs = expires_at
                    .signed_duration_since(chrono::Utc::now())
                    .num_seconds()
                    .max(0);
                card = card
                    .child(
                        div()
                            .text_color(theme::muted())
                            .child(crate::i18n::text("remote.approve_detail")),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_5()
                            .rounded_md()
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::bg())
                            .p_5()
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_color(theme::muted())
                                            .text_size(theme::text_caption())
                                            .child(crate::i18n::text("remote.code")),
                                    )
                                    .child(
                                        div()
                                            .font_family(theme::SYSTEM_MONOSPACE_FONT)
                                            .text_size(theme::text_display())
                                            .child(user_code.clone()),
                                    ),
                            )
                            .child(detail(
                                "remote.fingerprint",
                                trust
                                    .fingerprint
                                    .clone()
                                    .or_else(|| status.fingerprint.clone())
                                    .unwrap_or_default(),
                                true,
                            )),
                    )
                    .child(
                        div()
                            .text_color(theme::muted())
                            .child(verification_uri.clone()),
                    )
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .text_color(theme::muted())
                            .child(tr(
                                "remote.time_left",
                                &[("time", &format!("{}:{:02}", secs / 60, secs % 60))],
                            )),
                    );
                let open = verification_uri_complete.clone();
                let copy = open.clone();
                card = card.child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            Button::new("remote-browser", crate::i18n::text("remote.open_browser"))
                                .size(ButtonSize::Sm)
                                .on_press(move |_, cx| cx.open_url(&open)),
                        )
                        .child(
                            Button::new("remote-copy", crate::i18n::text("remote.copy_link"))
                                .size(ButtonSize::Sm)
                                .variant(ButtonVariant::Secondary)
                                .on_press(move |_, cx| {
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                        copy.clone(),
                                    ))
                                }),
                        )
                        .child(self.remote_button(
                            "remote-cancel",
                            "ui.cancel",
                            |root, cx| root.remote_request(false, |c| c.remote_enroll_cancel(), cx),
                            cx,
                        )),
                );
            }
            RemoteState::Confirming { account, .. } => {
                card = card.child(div().text_color(theme::muted()).child(tr(
                    "remote.cli_confirmation",
                    &[("account", &account.username)],
                )));
            }
            RemoteState::Stopped { reason } => {
                let mut actions = div().flex().gap_2();
                if matches!(reason, StopReason::Removed | StopReason::InUseElsewhere)
                    && status.host_id.is_some()
                {
                    actions = actions.child(self.remote_button(
                        "remote-retry",
                        "remote.try_again",
                        |r, cx| r.remote_request(false, |c| c.remote_retry(), cx),
                        cx,
                    ));
                }
                if matches!(
                    reason,
                    StopReason::Removed | StopReason::Denied | StopReason::Expired
                ) {
                    actions = actions.child(
                        self.remote_button(
                            "remote-again",
                            "remote.connect_again",
                            Self::remote_connect,
                            cx,
                        )
                        .variant(ButtonVariant::Primary),
                    );
                }
                if *reason == StopReason::InUseElsewhere {
                    actions = actions.child(self.remote_button(
                        "remote-stop-disconnect",
                        "account.sign_out",
                        |r, cx| {
                            r.settings_page.remote.confirm = true;
                            cx.notify();
                        },
                        cx,
                    ));
                }
                if *reason == StopReason::UpdateRequired {
                    let root = cx.entity();
                    actions = actions.child(
                        Button::new("remote-updates", crate::i18n::text("remote.check_updates"))
                            .size(ButtonSize::Sm)
                            .on_press(move |window, cx| {
                                root.update(cx, |r, cx| {
                                    r.enter_settings_pane(SettingsPane::Updates, window, cx)
                                })
                            }),
                    );
                }
                let warning = matches!(
                    reason,
                    StopReason::Removed | StopReason::InUseElsewhere | StopReason::UpdateRequired
                );
                let title_key = match reason {
                    StopReason::Removed => "remote.removed_title",
                    StopReason::InUseElsewhere => "remote.in_use_title",
                    StopReason::UpdateRequired => "remote.update_title",
                    StopReason::Denied => "remote.denied_title",
                    StopReason::Expired => "remote.expired_title",
                };
                card = div()
                    .debug_selector(|| "REMOTE_STATUS_CARD".into())
                    .flex()
                    .items_center()
                    .gap_4()
                    .rounded_lg()
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .p_5()
                    .text_size(theme::text_body())
                    .child(div().size(rems(8. / 16.)).rounded_full().bg(if warning {
                        theme::danger()
                    } else {
                        theme::muted()
                    }))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(theme::text_lead())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(crate::i18n::text(title_key)),
                            )
                            .child(
                                div()
                                    .text_size(theme::text_body())
                                    .text_color(theme::muted())
                                    .child(stopped_text(&status)),
                            ),
                    )
                    .child(actions);
            }
            _ => {
                if let RemoteState::Connected { since } = status.state {
                    card = card.child(
                        div().text_color(theme::muted()).child(tr(
                            "remote.online_since",
                            &[
                                (
                                    "time",
                                    &since
                                        .with_timezone(&chrono::Local)
                                        .format("%H:%M")
                                        .to_string(),
                                ),
                                (
                                    "minutes",
                                    &chrono::Utc::now()
                                        .signed_duration_since(since)
                                        .num_minutes()
                                        .to_string(),
                                ),
                            ],
                        )),
                    );
                }
                if let RemoteState::Reconnecting { next_retry_at, .. } = status.state {
                    let seconds = next_retry_at
                        .signed_duration_since(chrono::Utc::now())
                        .num_seconds()
                        .max(0);
                    card = card
                        .child(
                            div()
                                .text_color(theme::muted())
                                .child(tr("remote.retry_in", &[("seconds", &seconds.to_string())])),
                        )
                        .child(div().flex().child(self.remote_button(
                            "remote-retry-now",
                            "remote.retry_now",
                            |r, cx| r.remote_request(false, |c| c.remote_retry(), cx),
                            cx,
                        )));
                }
                if matches!(status.state, RemoteState::Connecting { .. }) {
                    card = card.child(runner_app::ui::button::spinner(
                        "remote-connecting-spinner",
                        16.,
                        theme::muted(),
                    ));
                }
                if matches!(status.state, RemoteState::Paused) {
                    card = card.child(
                        div()
                            .text_color(theme::muted())
                            .child(crate::i18n::text("remote.paused_detail")),
                    );
                }
                let account = status
                    .account
                    .as_ref()
                    .map(|a| format!("@{} · {}", a.username, a.display_name))
                    .unwrap_or_default();
                card = card
                    .child(
                        div()
                            .flex()
                            .gap_5()
                            .child(detail("remote.account", account, false))
                            .child(detail(
                                "remote.computer",
                                status.host_name.clone().unwrap_or_default(),
                                false,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_5()
                            .child(detail(
                                "remote.fingerprint",
                                trust
                                    .fingerprint
                                    .clone()
                                    .or_else(|| status.fingerprint.clone())
                                    .unwrap_or_default(),
                                true,
                            ))
                            .child(detail("remote.service", status.origin.clone(), false)),
                    );
            }
        }
        if matches!(status.state, RemoteState::Off) {
            if let Some(fingerprint) = &trust.fingerprint {
                card = card.child(detail("remote.fingerprint", fingerprint.clone(), true));
            }
        }
        if let Some(error) = self
            .settings_page
            .remote
            .error
            .as_ref()
            .or(status.last_error.as_ref())
        {
            card = card.child(div().text_color(theme::danger()).child(error.clone()));
        }
        let mut body = div()
            .flex()
            .flex_col()
            .gap_5()
            .child(PaneHeader::new(
                crate::i18n::text("ui.remote_access"),
                crate::i18n::text("remote.subtitle"),
            ))
            .child(card);
        if matches!(
            status.state,
            RemoteState::Connecting { .. }
                | RemoteState::Connected { .. }
                | RemoteState::Reconnecting { .. }
                | RemoteState::Paused
        ) {
            let root = cx.entity();
            let toggle = Toggle::new("remote-allow", !matches!(status.state, RemoteState::Paused))
                .on_change(move |enabled, _, cx| {
                    root.update(cx, |r, cx| {
                        r.remote_request(false, move |c| c.remote_set_paused(!enabled), cx)
                    })
                });
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(settings_section_heading(crate::i18n::text("remote.access")))
                    .child(SettingsCard::new(vec![
                        SettingsRow::new(crate::i18n::text("remote.allow"), toggle)
                            .subtitle(crate::i18n::text("remote.allow_detail"))
                            .into_any_element(),
                        SettingsRow::new(
                            crate::i18n::text("remote.disconnect_computer"),
                            self.remote_button(
                                "remote-disconnect",
                                "account.sign_out",
                                |r, cx| {
                                    r.settings_page.remote.confirm = true;
                                    cx.notify();
                                },
                                cx,
                            )
                            .variant(ButtonVariant::Danger),
                        )
                        .subtitle(crate::i18n::text("remote.disconnect_detail"))
                        .into_any_element(),
                    ])),
            );
        }
        body = body.child(self.remote_pairing(&trust, cx));
        body = body.child(self.remote_devices(&trust, cx));
        body.child(
            div()
                .flex()
                .items_start()
                .gap(rems(6. / 16.))
                .text_size(theme::text_meta())
                .text_color(theme::faint())
                .child(
                    svg()
                        .path("lock.svg")
                        .flex_none()
                        .size(rems(12. / 16.))
                        .mt(rems(2. / 16.)),
                )
                .child(crate::i18n::text("remote.footer")),
        )
        .into_any_element()
    }
    pub(crate) fn dismiss_remote_dialog(&mut self) {
        self.settings_page.remote.pin = None;
        self.settings_page.remote.confirm = false;
        self.settings_page.remote.reset_identity = false;
    }
    pub(crate) fn remote_overlay_open(&self) -> bool {
        self.settings_page.remote.confirm || self.settings_page.remote.pin.is_some()
    }
    pub(crate) fn remote_pin_dialog_open(&self) -> bool {
        self.settings_page.remote.pin.is_some()
    }
    fn remote_open_pin(&mut self, cx: &mut Context<Self>) {
        let fields = [cx.new(PinField::new), cx.new(PinField::new)];
        let subscriptions = fields
            .iter()
            .map(|field| cx.observe(field, |_, _, cx| cx.notify()))
            .collect();
        self.settings_page.remote.pin = Some(PinDialog {
            fields,
            step: 0,
            secondary_focus: cx.focus_handle(),
            primary_focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        });
        cx.notify();
    }
    fn remote_advance_pin(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let Some(dialog) = &mut self.settings_page.remote.pin else {
            return;
        };
        if dialog.fields[dialog.step].read(cx).value().0.len() != 6 {
            return;
        }
        if dialog.step == 0 {
            dialog.step = 1;
            dialog.fields[1].read(cx).focus.clone().focus(window, cx);
            cx.notify();
            return;
        }
        match confirmed_pin(
            dialog.fields[0].read(cx).value(),
            dialog.fields[1].read(cx).value(),
        ) {
            Ok(pin) => {
                self.settings_page.remote.pin = None;
                self.remote_request(
                    false,
                    move |c| {
                        c.remote_set_pin(pin)?;
                        c.remote_status()
                    },
                    cx,
                );
            }
            Err(_) => {
                self.remote_open_pin(cx);
                self.settings_page.remote.pin.as_ref().unwrap().fields[0]
                    .read(cx)
                    .focus
                    .clone()
                    .focus(window, cx);
                self.show_toast(
                    crate::i18n::text("remote.pin_mismatch"),
                    crate::toast::ToastTone::Error,
                    cx,
                );
            }
        }
        cx.notify();
    }
    fn remote_pairing(&self, trust: &RemoteTrust, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = Vec::new();
        if trust.pin_locked {
            rows.push(
                SettingsRow::new(
                    crate::i18n::text("remote.pin_locked"),
                    self.remote_button(
                        "remote-unlock",
                        "remote.unlock",
                        |r, cx| {
                            r.remote_request(
                                false,
                                |c| {
                                    c.remote_unlock_pin()?;
                                    c.remote_status()
                                },
                                cx,
                            )
                        },
                        cx,
                    ),
                )
                .subtitle(crate::i18n::text("remote.pin_locked_detail"))
                .label_icon("lock.svg", theme::warning())
                .into_any_element(),
            );
        }
        rows.push(
            SettingsRow::new(
                crate::i18n::text("remote.pin"),
                self.remote_button(
                    "remote-pin",
                    if trust.pin_set {
                        "remote.change_pin"
                    } else {
                        "remote.set_pin"
                    },
                    Self::remote_open_pin,
                    cx,
                )
                .variant(if trust.pin_set {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Primary
                }),
            )
            .subtitle(crate::i18n::text(if trust.pin_set {
                "remote.pin_set_detail"
            } else {
                "remote.pin_unset_detail"
            }))
            .into_any_element(),
        );
        rows.push(
            SettingsRow::new(
                crate::i18n::text("remote.reset_identity"),
                self.remote_button(
                    "remote-reset-identity",
                    "remote.reset_identity",
                    |r, cx| {
                        r.settings_page.remote.reset_identity = true;
                        r.settings_page.remote.confirm = true;
                        cx.notify();
                    },
                    cx,
                )
                .variant(ButtonVariant::Danger),
            )
            .subtitle(crate::i18n::text("remote.reset_identity_detail"))
            .into_any_element(),
        );
        div()
            .debug_selector(|| "REMOTE_PAIRING".into())
            .flex()
            .flex_col()
            .gap_2()
            .child(settings_section_heading(crate::i18n::text(
                "remote.pairing",
            )))
            .child(SettingsCard::new(rows))
            .into_any_element()
    }
    fn remote_devices(&self, trust: &RemoteTrust, cx: &mut Context<Self>) -> AnyElement {
        let mut table = div()
            .flex()
            .flex_col()
            .rounded_lg()
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .overflow_hidden();
        if !trust.devices.is_empty() {
            table = table.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_2()
                    .text_size(theme::text_meta())
                    .text_color(theme::faint())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(crate::i18n::text("remote.device")),
                    )
                    .child(
                        div()
                            .w(rems(210. / 16.))
                            .child(crate::i18n::text("remote.fingerprint")),
                    )
                    .child(
                        div()
                            .w(rems(75. / 16.))
                            .child(crate::i18n::text("remote.paired")),
                    )
                    .child(
                        div()
                            .w(rems(135. / 16.))
                            .child(crate::i18n::text("remote.last_seen")),
                    )
                    .child(div().w(rems(70. / 16.))),
            );
        }
        for device in &trust.devices {
            let root = cx.entity();
            let fingerprint = device.fingerprint.clone();
            table = table.child(
                div()
                    .debug_selector(|| "REMOTE_DEVICE_ROW".into())
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .border_t_1()
                    .border_color(theme::border())
                    .text_size(theme::text_body())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                svg()
                                    .path("monitor.svg")
                                    .size(rems(1.))
                                    .flex_none()
                                    .text_color(theme::muted()),
                            )
                            .child(div().truncate().child(device.name.clone())),
                    )
                    .child(
                        div()
                            .w(rems(210. / 16.))
                            .text_size(theme::text_meta())
                            .font_family(theme::SYSTEM_MONOSPACE_FONT)
                            .text_color(theme::muted())
                            .child(device.fingerprint.clone()),
                    )
                    .child(
                        div()
                            .w(rems(75. / 16.))
                            .text_size(theme::text_meta())
                            .text_color(theme::muted())
                            .child(
                                device
                                    .paired_at
                                    .with_timezone(&chrono::Local)
                                    .format("%Y-%m-%d")
                                    .to_string(),
                            ),
                    )
                    .child(
                        div()
                            .w(rems(135. / 16.))
                            .text_size(theme::text_meta())
                            .text_color(if device.connected {
                                theme::accent()
                            } else {
                                theme::muted()
                            })
                            .flex()
                            .items_center()
                            .gap_2()
                            .children(device.connected.then(|| {
                                div()
                                    .debug_selector(|| "REMOTE_CONNECTED_NOW".into())
                                    .size(rems(6. / 16.))
                                    .rounded_full()
                                    .bg(theme::accent())
                            }))
                            .child(if device.connected {
                                crate::i18n::text("remote.connected_now").to_string()
                            } else {
                                device
                                    .last_seen_at
                                    .with_timezone(&chrono::Local)
                                    .format("%Y-%m-%d %H:%M")
                                    .to_string()
                            }),
                    )
                    .child(
                        div().w(rems(70. / 16.)).child(
                            Button::new(
                                gpui::SharedString::from(format!(
                                    "remote-revoke-{}",
                                    device.fingerprint
                                )),
                                crate::i18n::text("remote.revoke"),
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Outline)
                            .disabled(self.settings_page.remote.busy)
                            .on_press(move |_, cx| {
                                root.update(cx, |r, cx| {
                                    let fingerprint = fingerprint.clone();
                                    r.remote_request(
                                        false,
                                        move |c| {
                                            c.remote_revoke_device(fingerprint)?;
                                            c.remote_status()
                                        },
                                        cx,
                                    );
                                });
                            }),
                        ),
                    ),
            );
        }
        table = table.child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .px_4()
                .py_3()
                .when(!trust.devices.is_empty(), |row| {
                    row.border_t_1().border_color(theme::border())
                })
                .text_size(theme::text_ui())
                .text_color(theme::faint())
                .child(svg().path("plus.svg").size(rems(15. / 16.)).flex_none())
                .child(crate::i18n::text(if trust.devices.is_empty() {
                    "remote.devices_empty"
                } else {
                    "remote.devices_hint"
                })),
        );
        div()
            .debug_selector(|| "REMOTE_DEVICES".into())
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(settings_section_heading(crate::i18n::text(
                        "remote.devices",
                    )))
                    .when(!trust.devices.is_empty(), |row| {
                        row.child(
                            div()
                                .rounded_full()
                                .bg(theme::raised())
                                .px_1()
                                .text_size(theme::text_micro())
                                .text_color(theme::muted())
                                .child(trust.devices.len().to_string()),
                        )
                    }),
            )
            .child(table)
            .into_any_element()
    }
    fn remote_pin_overlay(&self, cx: &mut Context<Self>) -> AnyElement {
        let dialog = self.settings_page.remote.pin.as_ref().unwrap();
        let field = dialog.fields[dialog.step].clone();
        let confirming = dialog.step == 1;
        let complete = field.read(cx).value().0.len() == 6;
        let changing = self
            .app_store
            .read(cx)
            .remote_trust
            .as_ref()
            .is_some_and(|trust| trust.pin_set);
        let root = cx.entity();
        let cancel = root.clone();
        let save = root.clone();
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .bg(theme::scrim())
            .occlude()
            .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                cancel.update(cx, |r, cx| {
                    r.settings_page.remote.pin = None;
                    cx.notify();
                });
            })
            .on_key_down(move |event: &gpui::KeyDownEvent, window, cx| {
                root.update(cx, |r, cx| match event.keystroke.key.as_str() {
                    "escape" => {
                        r.settings_page.remote.pin = None;
                        cx.stop_propagation();
                        cx.notify();
                    }
                    "enter" => {
                        r.remote_advance_pin(window, cx);
                        cx.stop_propagation();
                    }
                    "tab" => {
                        if let Some(dialog) = &r.settings_page.remote.pin {
                            let field = dialog.fields[dialog.step].read(cx);
                            let mut stops =
                                vec![field.focus.clone(), dialog.secondary_focus.clone()];
                            if field.value().0.len() == 6 {
                                stops.push(dialog.primary_focus.clone());
                            }
                            let current = stops
                                .iter()
                                .position(|focus| focus.is_focused(window))
                                .unwrap_or(0);
                            let next = if event.keystroke.modifiers.shift {
                                (current + stops.len() - 1) % stops.len()
                            } else {
                                (current + 1) % stops.len()
                            };
                            stops[next].focus(window, cx);
                        }
                        cx.stop_propagation();
                    }
                    _ => {}
                });
            })
            .child(
                div()
                    .debug_selector(|| "REMOTE_PIN_DIALOG".into())
                    .w_full()
                    .max_w(rems(420. / 16.))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .rounded(rems(12. / 16.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .p(rems(22. / 16.))
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .text_size(theme::text_lead())
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(
                                svg()
                                    .path("key-round.svg")
                                    .size(rems(1.))
                                    .text_color(theme::muted()),
                            )
                            .child(crate::i18n::text(if changing {
                                "remote.change_pin"
                            } else {
                                "remote.set_pin_title"
                            })),
                    )
                    .child(
                        div()
                            .text_size(theme::text_body())
                            .text_color(theme::muted())
                            .child(crate::i18n::text("remote.pin_explanation")),
                    )
                    .child(
                        div()
                            .debug_selector(move || {
                                format!("REMOTE_PIN_STEP_{}", usize::from(confirming))
                            })
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(theme::text_ui())
                                    .text_color(theme::muted())
                                    .child(crate::i18n::text(if confirming {
                                        "remote.confirm_pin"
                                    } else {
                                        "remote.choose_pin"
                                    })),
                            )
                            .child(field)
                            .child(
                                div()
                                    .text_size(theme::text_meta())
                                    .text_color(theme::faint())
                                    .child(crate::i18n::text(if confirming {
                                        "remote.pin_hint_confirm"
                                    } else {
                                        "remote.pin_hint"
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .child(
                                div()
                                    .debug_selector(|| "REMOTE_PIN_SECONDARY".into())
                                    .child(
                                        Button::new(
                                            "remote-pin-cancel",
                                            crate::i18n::text(if confirming {
                                                "remote.pin_back"
                                            } else {
                                                "ui.cancel"
                                            }),
                                        )
                                        .size(ButtonSize::Compact)
                                        .radius(8.)
                                        .variant(ButtonVariant::Secondary)
                                        .focus_handle(dialog.secondary_focus.clone())
                                        .on_press({
                                            let root = cx.entity();
                                            move |window, cx| {
                                                root.update(cx, |r, cx| {
                                                    if confirming {
                                                        r.remote_open_pin(cx);
                                                        r.settings_page
                                                            .remote
                                                            .pin
                                                            .as_ref()
                                                            .unwrap()
                                                            .fields[0]
                                                            .read(cx)
                                                            .focus
                                                            .clone()
                                                            .focus(window, cx);
                                                    } else {
                                                        r.settings_page.remote.pin = None;
                                                    }
                                                    cx.notify();
                                                })
                                            }
                                        }),
                                    ),
                            )
                            .child(
                                div().debug_selector(|| "REMOTE_PIN_PRIMARY".into()).child(
                                    Button::new(
                                        "remote-pin-save",
                                        crate::i18n::text(if confirming {
                                            "remote.set_pin"
                                        } else {
                                            "remote.pin_continue"
                                        }),
                                    )
                                    .size(ButtonSize::Compact)
                                    .radius(8.)
                                    .variant(ButtonVariant::Primary)
                                    .focus_handle(dialog.primary_focus.clone())
                                    .disabled(!complete)
                                    .on_press(
                                        move |window, cx| {
                                            save.update(cx, |r, cx| {
                                                r.remote_advance_pin(window, cx)
                                            })
                                        },
                                    ),
                                ),
                            ),
                    ),
            )
            .into_any_element()
    }
    pub(crate) fn remote_confirm_overlay(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.settings_page.remote.pin.is_some() {
            return self.remote_pin_overlay(cx);
        }
        let reset = self.settings_page.remote.reset_identity;
        let root = cx.entity();
        let cancel = root.clone();
        let status = self
            .app_store
            .read(cx)
            .remote_status
            .clone()
            .unwrap_or_default();
        ConfirmDialog::new(
            crate::i18n::text(if reset {
                "remote.reset_identity_title"
            } else {
                "account.sign_out_title"
            }),
            tr(
                if reset {
                    if status.account.is_some() {
                        "remote.reset_identity_confirm"
                    } else {
                        "remote.reset_identity_confirm_off"
                    }
                } else {
                    "account.sign_out_confirm"
                },
                &[
                    ("computer", status.host_name.as_deref().unwrap_or("")),
                    (
                        "account",
                        status
                            .account
                            .as_ref()
                            .map(|a| a.username.as_str())
                            .unwrap_or("unknown"),
                    ),
                ],
            ),
            crate::i18n::text(if reset {
                "remote.reset_identity"
            } else {
                "account.sign_out"
            }),
            crate::i18n::text(if reset {
                "remote.resetting"
            } else {
                "account.signing_out"
            }),
            self.settings_page.remote.busy,
            Rc::new(move |_, cx| {
                root.update(cx, |r, cx| {
                    r.settings_page.remote.confirm = false;
                    r.settings_page.remote.reset_identity = false;
                    r.remote_request(
                        false,
                        move |c| {
                            if reset {
                                c.remote_reset_identity()?;
                            } else {
                                c.remote_logout()?;
                            }
                            c.remote_status()
                        },
                        cx,
                    );
                })
            }),
            Rc::new(move |_, cx| {
                cancel.update(cx, |r, cx| {
                    r.settings_page.remote.confirm = false;
                    r.settings_page.remote.reset_identity = false;
                    cx.notify();
                })
            }),
        )
        .focus_handle(self.settings_page.remote.focus.clone())
        .icon(if reset { "key-round.svg" } else { "globe.svg" })
        .into_any_element()
    }
}

fn tr(key: &str, args: &[(&str, &str)]) -> String {
    crate::i18n::tf(
        key,
        &args
            .iter()
            .map(|(k, v)| (*k, (*v).to_owned()))
            .collect::<Vec<_>>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{px, size, EntityInputHandler, TestAppContext, VisualTestContext};
    use runner_app::i18n::{testing::with_locale, Locale};
    #[test]
    fn every_status_renders_in_both_languages_and_themes() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for locale in [Locale::English, Locale::SimplifiedChinese] {
            with_locale(locale, || {
                let temp = tempfile::tempdir().unwrap();
                let mut cx = TestAppContext::single();
                let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
                let window = cx.add_window(|window, cx| {
                    let mut root = NativeRoot::new(
                        "remote-snapshots".into(),
                        temp.path().join("logs"),
                        None,
                        None,
                        store.clone(),
                        window,
                        cx,
                    );
                    root.route = crate::AppRoute::Settings;
                    root.settings_page.pane = SettingsPane::Remote;
                    root
                });
                cx.simulate_window_resize(window.into(), size(px(1200.), px(900.)));
                cx.run_until_parked();
                for variant in [
                    theme::ThemeVariant::Carbon,
                    theme::ThemeVariant::RunnerLight,
                ] {
                    theme::set_active_variant(variant);
                    let now = chrono::Utc::now();
                    let mut states = vec![
                        RemoteState::Off,
                        RemoteState::Enrolling {
                            user_code: "BCDF-GHJK".into(),
                            verification_uri: "https://runner.example/hosts/add".into(),
                            verification_uri_complete:
                                "https://runner.example/hosts/add?code=BCDF-GHJK".into(),
                            expires_at: now + chrono::Duration::minutes(10),
                        },
                        RemoteState::Confirming {
                            account: Default::default(),
                            expires_at: now + chrono::Duration::minutes(5),
                        },
                        RemoteState::Connecting { attempt: 0 },
                        RemoteState::Connected { since: now },
                        RemoteState::Reconnecting {
                            attempt: 2,
                            next_retry_at: now + chrono::Duration::seconds(15),
                        },
                        RemoteState::Paused,
                    ];
                    states.extend(
                        [
                            StopReason::Removed,
                            StopReason::InUseElsewhere,
                            StopReason::UpdateRequired,
                            StopReason::Denied,
                            StopReason::Expired,
                        ]
                        .into_iter()
                        .map(|reason| RemoteState::Stopped { reason }),
                    );
                    for state in states {
                        let off = matches!(state, RemoteState::Off);
                        store.update(&mut cx, |store, cx| {
                            store.remote_status = Some(RemoteStatus {
                                state,
                                account: Some(runner_core::remote::RemoteAccount {
                                    username: "jason".into(),
                                    display_name: "Jason".into(),
                                }),
                                host_id: Some("7383920468475904".into()),
                                host_name: Some("Mac Studio".into()),
                                fingerprint: Some("630d cd29 66c4 3366 9112".into()),
                                ..Default::default()
                            });
                            cx.notify();
                        });
                        window.update(&mut cx, |_, _, cx| cx.notify()).unwrap();
                        cx.run_until_parked();
                        let mut visual = VisualTestContext::from_window(window.into(), &cx);
                        let card = visual.debug_bounds("REMOTE_STATUS_CARD").unwrap();
                        if off {
                            let connect = visual.debug_bounds("REMOTE_CONNECT_BUTTON").unwrap();
                            assert!(connect.size.width > px(0.));
                            assert!(connect.size.width < card.size.width / 2.);
                            assert!(connect.origin.x < card.origin.x + card.size.width / 4.);
                            assert!(card.origin.y + card.size.height < px(900.));
                        } else {
                            assert!(visual.debug_bounds("REMOTE_CONNECT_BUTTON").is_none());
                        }
                    }
                }
            });
        }
    }
    #[test]
    fn pin_confirmation_and_lock_warning_keep_secrets_out_of_errors() {
        let pin = Secret("001234".into());
        assert!(confirmed_pin(&pin, &pin).unwrap().0 == pin.0);
        for invalid in [
            "12345",
            "1234567",
            "0123456789",
            " 123456",
            "１２３４５６",
            "12345x",
        ] {
            let invalid = Secret(invalid.into());
            assert_eq!(
                confirmed_pin(&invalid, &invalid).unwrap_err(),
                "remote.pin_invalid"
            );
        }
        assert_eq!(
            confirmed_pin(&pin, &Secret("654321".into())).unwrap_err(),
            "remote.pin_mismatch"
        );
        let unlocked = RemoteTrust {
            pin_set: true,
            ..Default::default()
        };
        let locked = RemoteTrust {
            pin_locked: true,
            ..unlocked.clone()
        };
        assert!(!warn_pin_lock(None, &locked));
        assert!(warn_pin_lock(Some(&unlocked), &locked));
        assert!(!warn_pin_lock(Some(&locked), &locked));
        assert!(!warn_pin_lock(Some(&locked), &unlocked));
    }

    #[test]
    fn two_step_pin_requires_six_digits_clears_on_back_and_mismatch_and_submits_on_enter() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        with_locale(Locale::English, || {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            let window = cx.add_window(|window, cx| {
                let mut root = NativeRoot::new(
                    "pin-steps".into(),
                    temp.path().join("logs"),
                    None,
                    None,
                    store.clone(),
                    window,
                    cx,
                );
                root.route = crate::AppRoute::Settings;
                root.settings_page.pane = SettingsPane::Remote;
                root.remote_open_pin(cx);
                root.settings_page.remote.pin.as_ref().unwrap().fields[0]
                    .read(cx)
                    .focus
                    .clone()
                    .focus(window, cx);
                root
            });
            cx.simulate_window_resize(window.into(), size(px(1200.), px(900.)));
            cx.simulate_keystrokes(window.into(), "tab tab");
            window
                .update(&mut cx, |root, window, cx| {
                    assert!(root.settings_page.remote.pin.as_ref().unwrap().fields[0]
                        .read(cx)
                        .focus
                        .is_focused(window));
                })
                .unwrap();
            cx.simulate_keystrokes(window.into(), "0 0 1 2 3 enter");
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            assert!(visual.debug_bounds("REMOTE_PIN_STEP_0").is_some());
            let primary = visual.debug_bounds("REMOTE_PIN_PRIMARY").unwrap();
            visual.simulate_click(primary.center(), Default::default());
            window
                .read_with(&cx, |root, cx| {
                    let dialog = root.settings_page.remote.pin.as_ref().unwrap();
                    assert_eq!(dialog.step, 0);
                    assert_eq!(dialog.fields[0].read(cx).value().0.len(), 5);
                })
                .unwrap();
            let input = visual.debug_bounds("REMOTE_PIN_BOX_5").unwrap();
            visual.simulate_click(input.center(), Default::default());
            cx.simulate_keystrokes(window.into(), "4");
            cx.run_until_parked();
            window
                .read_with(&cx, |root, cx| {
                    assert_eq!(
                        root.settings_page.remote.pin.as_ref().unwrap().fields[0]
                            .read(cx)
                            .value()
                            .0
                            .len(),
                        6
                    );
                })
                .unwrap();
            cx.simulate_keystrokes(window.into(), "tab tab");
            window
                .update(&mut cx, |root, window, _| {
                    assert!(root
                        .settings_page
                        .remote
                        .pin
                        .as_ref()
                        .unwrap()
                        .primary_focus
                        .is_focused(window));
                })
                .unwrap();
            cx.simulate_keystrokes(window.into(), "tab");
            let primary = visual.debug_bounds("REMOTE_PIN_PRIMARY").unwrap();
            visual.simulate_click(primary.center(), Default::default());
            window
                .update(&mut cx, |root, window, cx| {
                    let dialog = root.settings_page.remote.pin.as_ref().unwrap();
                    assert_eq!(dialog.step, 1);
                    assert!(dialog.fields[1].read(cx).focus.is_focused(window));
                })
                .unwrap();
            cx.simulate_keystrokes(window.into(), "6 5 4 3 2 enter");
            window
                .read_with(&cx, |root, _| {
                    assert_eq!(root.settings_page.remote.pin.as_ref().unwrap().step, 1)
                })
                .unwrap();
            cx.simulate_keystrokes(window.into(), "1 enter");
            window
                .read_with(&cx, |root, cx| {
                    let dialog = root.settings_page.remote.pin.as_ref().unwrap();
                    assert_eq!(dialog.step, 0);
                    assert!(dialog
                        .fields
                        .iter()
                        .all(|field| field.read(cx).value().0.is_empty()));
                    let toast = root.toasts.active().unwrap();
                    assert_eq!(toast.message, "The PINs don't match.");
                    assert_eq!(toast.tone, crate::toast::ToastTone::Error);
                })
                .unwrap();
            cx.run_until_parked();
            let close = visual.debug_bounds("GLOBAL_TOAST_CLOSE").unwrap();
            visual.simulate_click(close.center(), Default::default());
            window
                .read_with(&cx, |root, cx| {
                    assert!(root.toasts.active().is_none());
                    let dialog = root.settings_page.remote.pin.as_ref().unwrap();
                    assert_eq!(dialog.step, 0);
                    assert!(dialog
                        .fields
                        .iter()
                        .all(|field| field.read(cx).value().0.is_empty()));
                })
                .unwrap();
            cx.run_until_parked();
            assert!(visual.debug_bounds("GLOBAL_TOAST").is_none());
            assert!(visual.debug_bounds("REMOTE_PIN_STEP_0").is_some());
            let input = visual.debug_bounds("REMOTE_PIN_BOX_0").unwrap();
            visual.simulate_click(input.center(), Default::default());
            cx.simulate_keystrokes(window.into(), "0 0 1 2 3 4 enter 0 0");
            cx.run_until_parked();
            let back = visual.debug_bounds("REMOTE_PIN_SECONDARY").unwrap();
            visual.simulate_click(back.center(), Default::default());
            window
                .read_with(&cx, |root, cx| {
                    let dialog = root.settings_page.remote.pin.as_ref().unwrap();
                    assert_eq!(dialog.step, 0);
                    assert!(dialog
                        .fields
                        .iter()
                        .all(|field| field.read(cx).value().0.is_empty()));
                })
                .unwrap();
            cx.simulate_keystrokes(window.into(), "0 0 1 2 3 4 enter 0 0 1 2 3 4 enter");
            window
                .read_with(&cx, |root, _| {
                    assert!(root.settings_page.remote.pin.is_none());
                    assert!(root.settings_page.remote.request_generation > 0);
                })
                .unwrap();
            window
                .update(&mut cx, |root, window, cx| {
                    root.remote_open_pin(cx);
                    root.settings_page.remote.pin.as_ref().unwrap().fields[0]
                        .read(cx)
                        .focus
                        .clone()
                        .focus(window, cx);
                })
                .unwrap();
            cx.simulate_keystrokes(window.into(), "0 0 escape");
            window
                .read_with(&cx, |root, _| {
                    assert!(root.settings_page.remote.pin.is_none())
                })
                .unwrap();
        });
    }

    #[test]
    fn trust_rows_dialogs_and_lock_toast_render_in_both_languages_and_themes() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for locale in [Locale::English, Locale::SimplifiedChinese] {
            with_locale(locale, || {
                let temp = tempfile::tempdir().unwrap();
                let mut cx = TestAppContext::single();
                let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
                let window = cx.add_window(|window, cx| {
                    let mut root = NativeRoot::new(
                        "trust-snapshots".into(),
                        temp.path().join("logs"),
                        None,
                        None,
                        store.clone(),
                        window,
                        cx,
                    );
                    root.route = crate::AppRoute::Settings;
                    root.settings_page.pane = SettingsPane::Remote;
                    root
                });
                cx.simulate_window_resize(window.into(), size(px(1440.), px(1160.)));
                for variant in [
                    theme::ThemeVariant::Carbon,
                    theme::ThemeVariant::RunnerLight,
                ] {
                    theme::set_active_variant(variant);
                    for (pin_set, pin_locked, devices) in
                        [(false, false, 0), (true, false, 3), (true, true, 3)]
                    {
                        let now = chrono::Utc::now();
                        store.update(&mut cx, |store, cx| {
                            store.remote_status = Some(RemoteStatus {
                                state: RemoteState::Connected { since: now },
                                account: Some(runner_core::remote::RemoteAccount {
                                    username: "jason".into(),
                                    display_name: "Jason".into(),
                                }),
                                host_name: Some("Mac Studio".into()),
                                ..Default::default()
                            });
                            store.remote_trust = Some(RemoteTrust {
                                fingerprint: Some("630d cd29 66c4 3366 9112".into()),
                                pin_set,
                                pin_locked,
                                devices: (0..devices)
                                    .map(|index| runner_core::remote::PairedDevice {
                                        fingerprint: format!("2b7f 91c0 a3e4 5d12 0f6{index}"),
                                        name: ["MacBook Air", "Windows PC", "Linux server"][index]
                                            .into(),
                                        public_key: String::new(),
                                        paired_at: now,
                                        last_seen_at: now,
                                        connected: index == 0,
                                    })
                                    .collect(),
                            });
                            cx.notify();
                        });
                        window
                            .update(&mut cx, |root, _, cx| {
                                root.dismiss_remote_dialog();
                                if pin_locked {
                                    root.toasts.show(
                                        crate::i18n::text("remote.pin_locked_toast").to_string(),
                                        crate::toast::ToastTone::Warning,
                                    );
                                    root.toasts.pin_lock_warning();
                                } else {
                                    root.toasts.dismiss();
                                }
                                cx.notify();
                            })
                            .unwrap();
                        cx.run_until_parked();
                        let mut visual = VisualTestContext::from_window(window.into(), &cx);
                        let pairing = visual.debug_bounds("REMOTE_PAIRING").unwrap();
                        let table = visual.debug_bounds("REMOTE_DEVICES").unwrap();
                        assert!(table.top() > pairing.top());
                        assert!(table.bottom() < px(1160.));
                        assert_eq!(
                            visual.debug_bounds("REMOTE_DEVICE_ROW").is_some(),
                            devices > 0
                        );
                        assert_eq!(
                            visual.debug_bounds("REMOTE_CONNECTED_NOW").is_some(),
                            devices > 0
                        );
                        if devices > 0 {
                            store.update(&mut cx, |store, cx| {
                                store.remote_trust.as_mut().unwrap().devices[0].connected = false;
                                cx.notify();
                            });
                            window.update(&mut cx, |_, _, cx| cx.notify()).unwrap();
                            cx.run_until_parked();
                            assert!(visual.debug_bounds("REMOTE_CONNECTED_NOW").is_none());
                        }
                        if pin_locked {
                            let toast = visual.debug_bounds("GLOBAL_TOAST").unwrap();
                            assert!(toast.top() > px(1000.));
                        }
                        window
                            .update(&mut cx, |root, window, cx| {
                                root.remote_open_pin(cx);
                                root.settings_page.remote.pin.as_ref().unwrap().fields[0]
                                    .read(cx)
                                    .focus
                                    .clone()
                                    .focus(window, cx);
                                cx.notify();
                            })
                            .unwrap();
                        cx.run_until_parked();
                        let pin_dialog = visual.debug_bounds("REMOTE_PIN_DIALOG").unwrap();
                        assert!(pin_dialog.left() >= px(0.) && pin_dialog.right() <= px(1440.));
                        assert!(pin_dialog.top() >= px(0.) && pin_dialog.bottom() <= px(1160.));
                        let boxes = [
                            "REMOTE_PIN_BOX_0",
                            "REMOTE_PIN_BOX_1",
                            "REMOTE_PIN_BOX_2",
                            "REMOTE_PIN_BOX_3",
                            "REMOTE_PIN_BOX_4",
                            "REMOTE_PIN_BOX_5",
                        ];
                        let first = visual.debug_bounds(boxes[0]).unwrap();
                        for selector in boxes {
                            let bounds = visual.debug_bounds(selector).unwrap();
                            assert_eq!(bounds.size.height, px(48.));
                            assert!((bounds.size.width - first.size.width).abs() < px(1.));
                            assert!(
                                bounds.left() > pin_dialog.left()
                                    && bounds.right() < pin_dialog.right()
                            );
                        }
                        window
                            .update(&mut cx, |root, window, cx| {
                                root.remote_advance_pin(window, cx);
                                assert_eq!(root.settings_page.remote.pin.as_ref().unwrap().step, 0);
                                root.settings_page.remote.pin.as_ref().unwrap().fields[0].update(
                                    cx,
                                    |field, cx| {
                                        field.replace_text_in_range(None, "001234", window, cx);
                                    },
                                );
                                root.remote_advance_pin(window, cx);
                                assert_eq!(root.settings_page.remote.pin.as_ref().unwrap().step, 1);
                            })
                            .unwrap();
                        cx.run_until_parked();
                        assert!(visual.debug_bounds("REMOTE_PIN_STEP_0").is_none());
                        assert!(visual.debug_bounds("REMOTE_PIN_STEP_1").is_some());
                        window
                            .update(&mut cx, |root, _, cx| {
                                root.dismiss_remote_dialog();
                                assert!(root.settings_page.remote.pin.is_none());
                                root.settings_page.remote.confirm = true;
                                root.settings_page.remote.reset_identity = true;
                                cx.notify();
                            })
                            .unwrap();
                        cx.run_until_parked();
                        let reset_dialog = visual.debug_bounds("CONFIRM_DIALOG_PANEL").unwrap();
                        assert!(reset_dialog.left() >= px(0.) && reset_dialog.right() <= px(1440.));
                        assert!(reset_dialog.top() >= px(0.) && reset_dialog.bottom() <= px(1160.));
                    }
                }
            });
        }
    }

    #[test]
    fn warning_requires_a_live_transition_to_a_relevant_stop() {
        let mut s = RemoteStatus::default();
        for reason in [
            StopReason::Removed,
            StopReason::InUseElsewhere,
            StopReason::UpdateRequired,
            StopReason::Denied,
            StopReason::Expired,
        ] {
            s.state = RemoteState::Stopped { reason };
            assert!(!warn_transition(None, &s));
            assert!(!warn_transition(Some(&s), &s));
            assert_eq!(
                warn_transition(Some(&RemoteStatus::default()), &s),
                matches!(
                    reason,
                    StopReason::Removed | StopReason::InUseElsewhere | StopReason::UpdateRequired
                )
            );
        }
    }
}
