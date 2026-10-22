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
        let store = self.app_store.read(cx);
        let status = store.remote_status.clone().unwrap_or_default();
        let trust = store.remote_trust.clone().unwrap_or_default();
        let mut body = div().flex().flex_col().gap_5().child(PaneHeader::new(
            crate::i18n::text("ui.remote_access"),
            crate::i18n::text("remote.subtitle"),
        ));
        if account::signed_in(&status) {
            body = body
                .child(self.relay_card(&status, &trust, cx))
                .child(self.remote_devices(&trust, &status, cx))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(settings_section_heading(crate::i18n::text("remote.route")))
                        .child(SettingsCard::new(vec![
                            SettingsRow::new(crate::i18n::text("remote.relay"), div())
                                .label_icon("cloud.svg", theme::muted())
                                .subtitle(crate::i18n::text("remote.route_detail"))
                                .into_any_element(),
                            SettingsRow::new(
                                tr(
                                    "account.signed_in",
                                    &[("account", account::username(&status))],
                                ),
                                self.settings_link(
                                    SettingsPane::Account,
                                    "remote-account",
                                    "account.title",
                                    cx,
                                ),
                            )
                            .label_icon("user-round.svg", theme::muted())
                            .subtitle(crate::i18n::text("remote.route_account_detail"))
                            .into_any_element(),
                        ])),
                );
        } else if matches!(
            status.state,
            RemoteState::Enrolling { .. } | RemoteState::Confirming { .. }
        ) {
            body = body.child(self.account_status_card(&status, &trust, cx));
            if matches!(status.state, RemoteState::Enrolling { .. }) {
                body = body.child(account::approval_reminder());
            }
        } else {
            body = body.child(self.remote_route_choice(&status, cx));
        }
        body.child(account::footer("remote.footer"))
            .into_any_element()
    }

    fn remote_route_choice(&self, status: &RemoteStatus, cx: &mut Context<Self>) -> AnyElement {
        let mut content = div()
            .debug_selector(|| "REMOTE_ROUTE_CHOICE".into())
            .flex()
            .flex_col()
            .gap_5()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(theme::text_lead())
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(crate::i18n::text("remote.choose_route")),
                    )
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .text_color(theme::muted())
                            .child(crate::i18n::text("remote.choose_route_detail")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_3()
                    .child(
                        account::card()
                            .flex_1()
                            .min_w_0()
                            .p_5()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        svg()
                                            .path("cloud.svg")
                                            .size(rems(18. / 16.))
                                            .text_color(theme::muted()),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(crate::i18n::text("remote.through_relay")),
                                    )
                                    .child(account::badge("remote.private_beta", true)),
                            )
                            .child(div().text_color(theme::muted()).child(tr(
                                "remote.relay_choice_detail",
                                &[("service", &status.origin)],
                            )))
                            .child(
                                div()
                                    .debug_selector(|| "REMOTE_SIGN_IN".into())
                                    .self_start()
                                    .flex()
                                    .child(
                                        self.remote_button(
                                            "remote-connect",
                                            "remote.sign_in",
                                            Self::remote_connect,
                                            cx,
                                        )
                                        .variant(ButtonVariant::Primary),
                                    ),
                            ),
                    )
                    .child(
                        account::card()
                            .flex_1()
                            .min_w_0()
                            .p_5()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        svg()
                                            .path("network.svg")
                                            .size(rems(18. / 16.))
                                            .text_color(theme::muted()),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(crate::i18n::text("remote.over_tailscale")),
                                    )
                                    .child(account::badge("account.later", false)),
                            )
                            .child(
                                div()
                                    .text_color(theme::muted())
                                    .child(crate::i18n::text("remote.tailscale_choice_detail")),
                            )
                            .child(
                                div()
                                    .debug_selector(|| "REMOTE_TAILSCALE_DISABLED".into())
                                    .flex()
                                    .child(
                                        Button::new(
                                            "remote-tailscale",
                                            crate::i18n::text("remote.use_tailscale"),
                                        )
                                        .size(ButtonSize::Sm)
                                        .variant(ButtonVariant::Secondary)
                                        .disabled(true),
                                    ),
                            ),
                    ),
            );
        if matches!(status.state, RemoteState::Stopped { .. }) {
            content = content.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .text_color(theme::muted())
                            .child(stopped_text(status)),
                    )
                    .child(self.settings_link(
                        SettingsPane::Account,
                        "remote-stop-account",
                        "account.title",
                        cx,
                    )),
            );
        }
        if let Some(error) = self
            .settings_page
            .remote
            .error
            .as_ref()
            .or(status.last_error.as_ref())
        {
            content = content.child(div().text_color(theme::danger()).child(error.clone()));
        }
        if self.settings_page.remote.busy && self.settings_page.remote.opening {
            content = content.child(self.cancel_enrollment_start(cx));
        }
        content.into_any_element()
    }

    fn relay_card(
        &self,
        status: &RemoteStatus,
        trust: &RemoteTrust,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut header = div().p_5().flex().items_center().gap_4();
        let (title, subtitle, color) = match status.state {
            RemoteState::Connected { since } => (
                "remote.relay_on",
                tr(
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
                ),
                theme::accent(),
            ),
            RemoteState::Reconnecting { next_retry_at, .. } => (
                "remote.reconnecting",
                tr(
                    "remote.retry_in",
                    &[(
                        "seconds",
                        &next_retry_at
                            .signed_duration_since(chrono::Utc::now())
                            .num_seconds()
                            .max(0)
                            .to_string(),
                    )],
                ),
                theme::warning(),
            ),
            RemoteState::Paused => (
                "remote.paused",
                crate::i18n::text("remote.paused_detail").into(),
                theme::muted(),
            ),
            _ => ("remote.connecting", String::new(), theme::muted()),
        };
        header = header
            .child(if matches!(status.state, RemoteState::Connecting { .. }) {
                runner_app::ui::button::spinner("remote-connecting-spinner", 16., theme::muted())
                    .into_any_element()
            } else {
                div()
                    .size(rems(8. / 16.))
                    .flex_none()
                    .rounded_full()
                    .bg(color)
                    .into_any_element()
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
                            .child(crate::i18n::text(title)),
                    )
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .text_color(theme::muted())
                            .child(subtitle),
                    ),
            );
        if matches!(status.state, RemoteState::Reconnecting { .. }) {
            header = header.child(
                self.remote_button(
                    "remote-retry-now",
                    "remote.retry_now",
                    |r, cx| r.remote_request(false, |c| c.remote_retry(), cx),
                    cx,
                )
                .icon("refresh-cw.svg"),
            );
        }
        let root = cx.entity();
        let toggle = Toggle::new("remote-allow", !matches!(status.state, RemoteState::Paused))
            .on_change(move |enabled, _, cx| {
                root.update(cx, |r, cx| {
                    r.remote_request(false, move |c| c.remote_set_paused(!enabled), cx)
                })
            });
        let mut card = account::card()
            .debug_selector(|| "REMOTE_STATUS_CARD".into())
            .child(header)
            .child(
                account::divider()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .gap_5()
                            .child(detail(
                                "remote.account",
                                account::account_name(status),
                                false,
                            ))
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
                                account::fingerprint(status, trust),
                                true,
                            ))
                            .child(detail("remote.service", status.origin.clone(), false)),
                    ),
            )
            .child(
                account::divider().child(
                    SettingsRow::new(crate::i18n::text("remote.allow"), toggle)
                        .subtitle(crate::i18n::text("remote.allow_detail")),
                ),
            )
            .child(account::divider().child(self.remote_pairing(trust, cx)));
        if let Some(error) = self
            .settings_page
            .remote
            .error
            .as_ref()
            .or(status.last_error.as_ref())
        {
            card = card.child(
                div()
                    .px_5()
                    .pb_4()
                    .text_color(theme::danger())
                    .child(error.clone()),
            );
        }
        card.into_any_element()
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
                div()
                    .bg(theme::with_alpha(theme::warning(), 0.06))
                    .child(
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
                        .label_icon("lock.svg", theme::warning()),
                    )
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
                    "remote.reset",
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
            .children(rows.into_iter().enumerate().map(|(index, row)| {
                account::divider()
                    .when(index == 0, |row| row.border_t_0())
                    .child(row)
            }))
            .into_any_element()
    }
    fn remote_devices(
        &self,
        trust: &RemoteTrust,
        status: &RemoteStatus,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
                    .child(settings_section_heading(tr(
                        "remote.computers_heading",
                        &[("account", account::username(status))],
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
                "remote.reset"
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
        .icon(if reset { "key-round.svg" } else { "unplug.svg" })
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
                        let stop_reason = if let RemoteState::Stopped { reason } = state {
                            Some(reason)
                        } else {
                            None
                        };
                        let enrolling = matches!(state, RemoteState::Enrolling { .. });
                        let off = matches!(state, RemoteState::Off);
                        let signed_in = matches!(
                            state,
                            RemoteState::Connecting { .. }
                                | RemoteState::Connected { .. }
                                | RemoteState::Reconnecting { .. }
                                | RemoteState::Paused
                        );
                        let waiting = matches!(
                            state,
                            RemoteState::Enrolling { .. } | RemoteState::Confirming { .. }
                        );
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
                        assert_eq!(
                            visual.debug_bounds("REMOTE_STATUS_CARD").is_some(),
                            signed_in
                        );
                        assert_eq!(
                            visual.debug_bounds("REMOTE_ROUTE_CHOICE").is_some(),
                            !signed_in && !waiting
                        );
                        assert_eq!(
                            visual.debug_bounds("REMOTE_TAILSCALE_DISABLED").is_some(),
                            !signed_in && !waiting
                        );
                        assert_eq!(visual.debug_bounds("REMOTE_DEVICES").is_some(), signed_in);
                        assert_eq!(
                            visual.debug_bounds("ACCOUNT_APPROVAL_REMINDER").is_some(),
                            enrolling
                        );
                        window
                            .update(&mut cx, |root, _, cx| {
                                root.settings_page.pane = SettingsPane::Account;
                                cx.notify();
                            })
                            .unwrap();
                        cx.run_until_parked();
                        let card = visual.debug_bounds("ACCOUNT_STATUS_CARD").unwrap();
                        assert!(card.bottom() < px(900.));
                        assert_eq!(
                            visual.debug_bounds("ACCOUNT_APPROVAL_REMINDER").is_some(),
                            enrolling
                        );
                        assert_eq!(visual.debug_bounds("ACCOUNT_SIGN_IN").is_some(), off);
                        assert_eq!(visual.debug_bounds("ACCOUNT_DETAILS").is_some(), signed_in);
                        assert_eq!(
                            visual.debug_bounds("ACCOUNT_RETRY").is_some(),
                            matches!(
                                stop_reason,
                                Some(StopReason::Removed | StopReason::InUseElsewhere)
                            )
                        );
                        assert_eq!(
                            visual.debug_bounds("ACCOUNT_SIGN_IN_AGAIN").is_some(),
                            matches!(
                                stop_reason,
                                Some(
                                    StopReason::Removed | StopReason::Denied | StopReason::Expired
                                )
                            )
                        );
                        assert_eq!(
                            visual.debug_bounds("ACCOUNT_STOP_SIGN_OUT").is_some(),
                            stop_reason == Some(StopReason::InUseElsewhere)
                        );
                        assert_eq!(
                            visual.debug_bounds("ACCOUNT_CHECK_UPDATES").is_some(),
                            stop_reason == Some(StopReason::UpdateRequired)
                        );
                        assert!(visual.debug_bounds("REMOTE_ROUTE_CHOICE").is_none());
                        window
                            .update(&mut cx, |root, _, cx| {
                                root.settings_page.pane = SettingsPane::Remote;
                                cx.notify();
                            })
                            .unwrap();
                    }
                }
            });
        }
    }
    struct RemoteRequests {
        calls: std::sync::Mutex<Vec<&'static str>>,
        status: std::sync::Mutex<RemoteStatus>,
    }
    impl runner_core::protocol::Transport for RemoteRequests {
        fn call(
            &self,
            request: runner_core::protocol::Request,
        ) -> Result<runner_core::protocol::Response, runner_core::protocol::ClientError> {
            use runner_core::protocol::{Request, Response};
            let mut status = self.status.lock().unwrap();
            let mut calls = self.calls.lock().unwrap();
            Ok(match request {
                Request::remote_status {} => Response::remote_status(Ok(status.clone())),
                Request::remote_enroll_start {
                    origin,
                    confirm_account,
                } => {
                    assert!(origin.is_none());
                    assert!(!confirm_account);
                    calls.push("sign in");
                    status.state = RemoteState::Enrolling {
                        user_code: "BCDF-GHJK".into(),
                        verification_uri: "https://runner.example/add".into(),
                        verification_uri_complete: "https://runner.example/add?code=BCDF-GHJK"
                            .into(),
                        expires_at: chrono::Utc::now() + chrono::Duration::minutes(10),
                    };
                    Response::remote_enroll_start(Ok(status.clone()))
                }
                Request::remote_enroll_cancel {} => {
                    calls.push("cancel");
                    *status = RemoteStatus::default();
                    Response::remote_enroll_cancel(Ok(status.clone()))
                }
                Request::remote_retry {} => {
                    calls.push("retry");
                    status.state = RemoteState::Connecting { attempt: 0 };
                    Response::remote_retry(Ok(status.clone()))
                }
                Request::remote_set_paused { paused } => {
                    calls.push(if paused { "pause" } else { "resume" });
                    status.state = if paused {
                        RemoteState::Paused
                    } else {
                        RemoteState::Connecting { attempt: 0 }
                    };
                    Response::remote_set_paused(Ok(status.clone()))
                }
                Request::remote_logout {} => {
                    calls.push("sign out");
                    *status = RemoteStatus::default();
                    Response::remote_logout(Ok(runner_core::remote::RemoteLogout::Removed))
                }
                Request::remote_reset_identity {} => {
                    calls.push("reset");
                    *status = RemoteStatus::default();
                    Response::remote_reset_identity(Ok(RemoteTrust::default()))
                }
                _ => {
                    return Err(runner_core::protocol::ClientError::msg(
                        "unused fixture request",
                    ))
                }
            })
        }
        fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
            unreachable!("the request fixture does not subscribe")
        }
    }

    #[test]
    fn account_actions_reuse_requests_and_sign_out_and_reset_require_confirmation() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let window = cx.add_window(|window, cx| {
            NativeRoot::new(
                "account-actions".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            )
        });
        cx.run_until_parked();
        let requests = std::sync::Arc::new(RemoteRequests {
            calls: Default::default(),
            status: Default::default(),
        });
        store.update(&mut cx, |store, _| {
            store.client = DaemonClient::new(requests.clone())
        });
        window
            .update(&mut cx, |root, _, cx| {
                root.route = crate::AppRoute::Settings;
                root.settings_page.pane = SettingsPane::Account;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        let sign_in = visual.debug_bounds("ACCOUNT_SIGN_IN").unwrap();
        visual.simulate_click(sign_in.center(), Default::default());
        cx.run_until_parked();
        assert_eq!(*requests.calls.lock().unwrap(), ["sign in"]);
        window
            .update(&mut cx, |root, _, cx| {
                root.remote_request(false, |c| c.remote_enroll_cancel(), cx)
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |root, _, cx| {
                root.remote_request(false, |c| c.remote_retry(), cx)
            })
            .unwrap();
        cx.run_until_parked();
        for paused in [true, false] {
            window
                .update(&mut cx, |root, _, cx| {
                    root.remote_request(false, move |c| c.remote_set_paused(paused), cx)
                })
                .unwrap();
            cx.run_until_parked();
        }
        assert_eq!(
            *requests.calls.lock().unwrap(),
            ["sign in", "cancel", "retry", "pause", "resume"]
        );
        window
            .update(&mut cx, |root, window, cx| {
                root.confirm_sign_out(cx);
                root.settings_page.remote.focus.focus(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        assert!(visual.debug_bounds("CONFIRM_DIALOG_PANEL").is_some());
        assert!(!requests.calls.lock().unwrap().contains(&"sign out"));
        cx.simulate_keystrokes(window.into(), "escape");
        cx.run_until_parked();
        assert!(visual.debug_bounds("CONFIRM_DIALOG_PANEL").is_none());
        assert!(!requests.calls.lock().unwrap().contains(&"sign out"));
        window
            .update(&mut cx, |root, window, cx| {
                root.confirm_sign_out(cx);
                root.settings_page.remote.focus.focus(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        cx.simulate_keystrokes(window.into(), "enter");
        cx.run_until_parked();
        assert_eq!(requests.calls.lock().unwrap().last(), Some(&"sign out"));
        for confirm in [false, true] {
            window
                .update(&mut cx, |root, window, cx| {
                    root.settings_page.pane = SettingsPane::Remote;
                    root.settings_page.remote.confirm = true;
                    root.settings_page.remote.reset_identity = true;
                    root.settings_page.remote.focus.focus(window, cx);
                    cx.notify();
                })
                .unwrap();
            cx.run_until_parked();
            cx.simulate_keystrokes(window.into(), if confirm { "enter" } else { "escape" });
            cx.run_until_parked();
            assert_eq!(requests.calls.lock().unwrap().contains(&"reset"), confirm);
        }
        store.update(&mut cx, |store, cx| {
            store.remote_status = Some(requests.status.lock().unwrap().clone());
            cx.notify();
        });
        window.update(&mut cx, |_, _, cx| cx.notify()).unwrap();
        cx.run_until_parked();
        assert!(visual.debug_bounds("REMOTE_ROUTE_CHOICE").is_some());
        let tailscale = visual.debug_bounds("REMOTE_TAILSCALE_DISABLED").unwrap();
        let before = requests.calls.lock().unwrap().len();
        visual.simulate_click(tailscale.center(), Default::default());
        cx.run_until_parked();
        assert_eq!(requests.calls.lock().unwrap().len(), before);
        let sign_in = visual.debug_bounds("REMOTE_SIGN_IN").unwrap();
        visual.simulate_click(sign_in.center(), Default::default());
        cx.run_until_parked();
        assert_eq!(requests.calls.lock().unwrap().last(), Some(&"sign in"));
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
                cx.run_until_parked();
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
