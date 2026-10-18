use crate::surfaces::settings_page::{settings_section_heading, SettingsPane};
use crate::{theme, NativeRoot};
use gpui::prelude::*;
use gpui::{div, rems, svg, AnyElement, Context, FontWeight, SharedString};
use runner_app::ui::{
    Button, ButtonSize, ButtonVariant, ConfirmDialog, PaneHeader, SettingsCard, SettingsRow, Toggle,
};
use runner_core::{
    protocol::DaemonClient,
    remote::{RemoteState, RemoteStatus, StopReason},
};
use std::rc::Rc;

pub(crate) struct RemotePaneState {
    busy: bool,
    opening: bool,
    request_generation: u64,
    error: Option<String>,
    pub(crate) confirm: bool,
    focus: gpui::FocusHandle,
    generation: u64,
}
impl RemotePaneState {
    pub(crate) fn new(cx: &mut Context<NativeRoot>) -> Self {
        Self {
            busy: false,
            opening: false,
            request_generation: 0,
            error: None,
            confirm: false,
            focus: cx.focus_handle(),
            generation: 0,
        }
    }
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
                    if root.settings_page.remote.generation != generation
                        || root.settings_page.pane != SettingsPane::Remote
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
                                status.fingerprint.clone().unwrap_or_default(),
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
                        "remote.disconnect",
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
                                status.fingerprint.clone().unwrap_or_default(),
                                true,
                            ))
                            .child(detail("remote.service", status.origin.clone(), false)),
                    );
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
                                "remote.disconnect",
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
    pub(crate) fn remote_confirm_overlay(&self, cx: &mut Context<Self>) -> AnyElement {
        let root = cx.entity();
        let cancel = root.clone();
        let status = self
            .app_store
            .read(cx)
            .remote_status
            .clone()
            .unwrap_or_default();
        ConfirmDialog::new(
            crate::i18n::text("remote.disconnect_title"),
            tr(
                "remote.disconnect_confirm",
                &[(
                    "account",
                    status
                        .account
                        .as_ref()
                        .map(|a| a.username.as_str())
                        .unwrap_or("unknown"),
                )],
            ),
            crate::i18n::text("remote.disconnect"),
            crate::i18n::text("remote.disconnecting"),
            self.settings_page.remote.busy,
            Rc::new(move |_, cx| {
                root.update(cx, |r, cx| {
                    r.settings_page.remote.confirm = false;
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
            Rc::new(move |_, cx| {
                cancel.update(cx, |r, cx| {
                    r.settings_page.remote.confirm = false;
                    cx.notify();
                })
            }),
        )
        .focus_handle(self.settings_page.remote.focus.clone())
        .icon("globe.svg")
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
    use gpui::{px, size, TestAppContext, VisualTestContext};
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
                    window.resize(size(px(1200.), px(900.)));
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
