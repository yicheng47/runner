use crate::surfaces::settings_page::{settings_section_heading, SettingsPane};
use crate::{theme, NativeRoot};
use futures::FutureExt;
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

use super::about::{DISCORD_URL, WECHAT_README_URL};

pub(crate) mod account;
pub(crate) mod pin;
use pin::PinField;

pub(crate) struct RemotePaneState {
    busy: bool,
    opening: bool,
    request_generation: u64,
    enrollment_start: Option<EnrollmentStart>,
    enrollment_cleanup: Option<futures::future::Shared<gpui::Task<()>>>,
    error: Option<String>,
    pub(crate) confirm: bool,
    reset_identity: bool,
    enable_local_only: bool,
    local_only_error: Option<String>,
    pin: Option<PinDialog>,
    pub(crate) focus: gpui::FocusHandle,
    generation: u64,
}
type EnrollmentTask =
    futures::future::Shared<gpui::Task<Result<RemoteStatus, runner_core::protocol::ClientError>>>;
struct EnrollmentStart {
    abandoned: std::sync::Arc<std::sync::atomic::AtomicBool>,
    dispatched: std::sync::Arc<std::sync::atomic::AtomicBool>,
    task: EnrollmentTask,
}
struct PinDialog {
    fields: [gpui::Entity<PinField>; 2],
    step: usize,
    warning: bool,
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
            enrollment_start: None,
            enrollment_cleanup: None,
            error: None,
            confirm: false,
            reset_identity: false,
            enable_local_only: false,
            local_only_error: None,
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
    runner_core::remote::validate_pin_format(&first.0).map_err(|_| "remote.pin_invalid")?;
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
    pub(crate) fn sync_remote_access_visibility(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        if !self
            .app_store
            .read(cx)
            .remote_status
            .as_ref()
            .is_some_and(account::signed_in)
        {
            self.client_ui.picker_open = false;
        }
        if self.app_store.read(cx).remote_access_enabled() {
            return;
        }
        let changed = self.client_ui.picker_open
            || self.client_ui.pair.is_some()
            || self.account_menu.state.open
            || self.remote_overlay_open()
            || matches!(
                self.settings_page.pane,
                SettingsPane::Account | SettingsPane::Remote
            );
        self.client_ui.picker_open = false;
        if self.client_ui.pair.take().is_some() {
            self.client_ui.pair_generation += 1;
        }
        self.account_menu.state.close();
        self.dismiss_remote_dialog();
        if matches!(
            self.settings_page.pane,
            SettingsPane::Account | SettingsPane::Remote
        ) {
            self.settings_page.pane = SettingsPane::General;
        }
        if self.remote_window.is_some() {
            self.return_to_this_computer(window, cx);
        } else if changed {
            self.root_focus.focus(window, cx);
            cx.notify();
        }
    }

    fn set_local_only(&self, enabled: bool, cx: &mut Context<Self>) {
        self.update_app_settings(cx, true, |settings| {
            if settings.local_only == enabled {
                return false;
            }
            settings.local_only = enabled;
            true
        });
    }

    fn change_local_only(
        &mut self,
        enabled: bool,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_page.remote.busy {
            return;
        }
        self.settings_page.remote.local_only_error = None;
        let status = self
            .app_store
            .read(cx)
            .remote_status
            .clone()
            .unwrap_or_default();
        if enabled && account::signed_in(&status) {
            self.confirm_sign_out(cx);
            self.settings_page.remote.enable_local_only = true;
            self.settings_page.remote.focus.focus(window, cx);
        } else {
            self.set_local_only(enabled, cx);
            if enabled
                && matches!(
                    status.state,
                    RemoteState::Enrolling { .. } | RemoteState::Confirming { .. }
                )
            {
                self.settings_page.remote.enable_local_only = true;
                self.remote_request(false, |c| c.remote_enroll_cancel(), cx);
            }
        }
        cx.notify();
    }

    pub(crate) fn render_local_only_setting(&self, cx: &Context<Self>) -> AnyElement {
        let root = cx.entity();
        let toggle = Toggle::new(
            "settings-local-only",
            !self.app_store.read(cx).remote_access_enabled(),
        )
        .disabled(self.settings_page.remote.busy)
        .on_change(move |enabled, window, cx| {
            root.update(cx, |root, cx| root.change_local_only(enabled, window, cx));
        });
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(settings_section_heading(crate::i18n::text(
                "ui.remote_access",
            )))
            .child(SettingsCard::new(vec![SettingsRow::new(
                crate::i18n::text("remote.local_only"),
                div()
                    .debug_selector(|| "SETTINGS_LOCAL_ONLY_TOGGLE".into())
                    .child(toggle),
            )
            .subtitle(crate::i18n::text("remote.local_only_detail"))
            .into_any_element()]))
            .children(
                self.settings_page
                    .remote
                    .local_only_error
                    .as_ref()
                    .map(|error| {
                        div()
                            .debug_selector(|| "SETTINGS_LOCAL_ONLY_ERROR".into())
                            .text_size(theme::text_meta())
                            .text_color(theme::danger())
                            .child(error.clone())
                    }),
            )
            .into_any_element()
    }

    pub(crate) fn enter_remote_settings(&mut self, cx: &mut Context<Self>) {
        self.refresh_computers(cx);
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
        let enable_local_only = std::mem::take(&mut self.settings_page.remote.enable_local_only);
        self.settings_page.remote.busy = true;
        self.settings_page.remote.opening = open_browser;
        self.settings_page.remote.request_generation += 1;
        let generation = self.settings_page.remote.request_generation;
        self.settings_page.remote.error = None;
        let client = self.core(cx).clone();
        let task = if open_browser {
            let cleanup = self.settings_page.remote.enrollment_cleanup.clone();
            let abandoned = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let dispatched = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let cancelled = abandoned.clone();
            let started = dispatched.clone();
            let task = cx
                .background_spawn(async move {
                    // A cancelled Start may still mutate the daemon until its cleanup finishes.
                    if let Some(cleanup) = cleanup {
                        cleanup.await;
                    }
                    if cancelled.load(std::sync::atomic::Ordering::SeqCst) {
                        return Err(runner_core::protocol::ClientError::msg(
                            "Enrollment cancelled.",
                        ));
                    }
                    started.store(true, std::sync::atomic::Ordering::SeqCst);
                    request(client)
                })
                .shared();
            self.settings_page.remote.enrollment_start = Some(EnrollmentStart {
                abandoned,
                dispatched,
                task: task.clone(),
            });
            task
        } else {
            cx.background_spawn(async move { request(client) }).shared()
        };
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |root, cx| {
                if root.settings_page.remote.request_generation != generation {
                    return;
                }
                if open_browser {
                    root.settings_page.remote.enrollment_start = None;
                }
                root.settings_page.remote.busy = false;
                match result {
                    Ok(status) => {
                        if enable_local_only && !account::signed_in(&status) {
                            root.set_local_only(true, cx);
                        }
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
                    Err(error) => {
                        if enable_local_only {
                            root.settings_page.remote.local_only_error =
                                Some(error.message.clone());
                        }
                        root.settings_page.remote.error = Some(error.message);
                    }
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
        let mut body = div()
            .flex()
            .flex_col()
            .gap_5()
            .child(PaneHeader::new(crate::i18n::text("ui.remote_access"), ""));
        if account::signed_in(&status) {
            body = body
                .child(self.relay_card(&status, &trust, cx))
                .child(self.render_computers(&trust, &status, cx))
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
            body = body.child(self.remote_signed_out(&status, cx));
        }
        body.child(account::footer("remote.footer"))
            .into_any_element()
    }

    fn remote_signed_out(&self, status: &RemoteStatus, cx: &mut Context<Self>) -> AnyElement {
        let mut content = div()
            .debug_selector(|| "REMOTE_RELAY_SIGNED_OUT".into())
            .flex()
            .flex_col()
            .gap_5()
            .child(
                account::card()
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
                    .child(
                        div()
                            .text_color(theme::muted())
                            .child(crate::i18n::text("remote.relay_choice_detail")),
                    )
                    .child(
                        div()
                            .self_start()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_3()
                            .child(
                                self.remote_button(
                                    "remote-connect",
                                    "remote.sign_in",
                                    Self::remote_connect,
                                    cx,
                                )
                                .debug_selector("REMOTE_SIGN_IN")
                                .variant(ButtonVariant::Primary),
                            )
                            .child(
                                Button::new("remote-invite-discord", "Discord")
                                    .debug_selector("REMOTE_INVITE_DISCORD")
                                    .icon("discord.svg")
                                    .size(ButtonSize::Sm)
                                    .variant(ButtonVariant::Ghost)
                                    .on_press(|_, cx| cx.open_url(DISCORD_URL)),
                            )
                            .child(
                                Button::new(
                                    "remote-invite-wechat-readme",
                                    crate::i18n::text("community.wechat_readme"),
                                )
                                .debug_selector("REMOTE_INVITE_WECHAT_README")
                                .icon("wechat.svg")
                                .size(ButtonSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .on_press(|_, cx| cx.open_url(WECHAT_README_URL)),
                            ),
                    ),
            );
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
        self.settings_page.remote.enable_local_only = false;
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
            warning: false,
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
        if dialog.warning || dialog.fields[dialog.step].read(cx).value().0.len() != 6 {
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
                if runner_core::remote::is_weak_pin(&pin.0) {
                    dialog.warning = true;
                    dialog.secondary_focus.focus(window, cx);
                } else {
                    self.remote_submit_pin(pin, cx);
                }
            }
            Err(error) => {
                self.remote_open_pin(cx);
                self.settings_page.remote.pin.as_ref().unwrap().fields[0]
                    .read(cx)
                    .focus
                    .clone()
                    .focus(window, cx);
                self.show_toast(crate::i18n::text(error), crate::toast::ToastTone::Error, cx);
            }
        }
        cx.notify();
    }

    fn remote_accept_pin_warning(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self
            .settings_page
            .remote
            .pin
            .as_ref()
            .filter(|dialog| dialog.warning)
        else {
            return;
        };
        let pin = dialog.fields[0].read(cx).value().clone();
        self.remote_submit_pin(pin, cx);
    }

    fn remote_submit_pin(&mut self, pin: Secret, cx: &mut Context<Self>) {
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
    fn remote_pin_overlay(&self, cx: &mut Context<Self>) -> AnyElement {
        let dialog = self.settings_page.remote.pin.as_ref().unwrap();
        let field = dialog.fields[dialog.step].clone();
        let confirming = dialog.step == 1;
        let warning = dialog.warning;
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
            .capture_key_down(move |event: &gpui::KeyDownEvent, _, cx| {
                if warning
                    && event.is_held
                    && matches!(event.keystroke.key.as_str(), "enter" | "space")
                {
                    cx.stop_propagation();
                }
            })
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
                            let stops = if dialog.warning {
                                vec![dialog.secondary_focus.clone(), dialog.primary_focus.clone()]
                            } else {
                                let field = dialog.fields[dialog.step].read(cx);
                                let mut stops =
                                    vec![field.focus.clone(), dialog.secondary_focus.clone()];
                                if field.value().0.len() == 6 {
                                    stops.push(dialog.primary_focus.clone());
                                }
                                stops
                            };
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
                                    .path(if warning {
                                        "triangle-alert.svg"
                                    } else {
                                        "key-round.svg"
                                    })
                                    .size(rems(1.))
                                    .text_color(if warning {
                                        theme::warning()
                                    } else {
                                        theme::muted()
                                    }),
                            )
                            .child(crate::i18n::text(if warning {
                                "remote.pin_weak_title"
                            } else if changing {
                                "remote.change_pin"
                            } else {
                                "remote.set_pin_title"
                            })),
                    )
                    .child(
                        div()
                            .debug_selector(move || {
                                if warning {
                                    "REMOTE_PIN_WARNING"
                                } else {
                                    "REMOTE_PIN_EXPLANATION"
                                }
                                .into()
                            })
                            .text_size(theme::text_body())
                            .text_color(theme::muted())
                            .child(crate::i18n::text(if warning {
                                "remote.pin_weak"
                            } else {
                                "remote.pin_explanation"
                            })),
                    )
                    .when(!warning, |panel| {
                        panel.child(
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
                    })
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
                                            crate::i18n::text(if warning {
                                                "remote.pin_choose_another"
                                            } else if confirming {
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
                                                    if warning || confirming {
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
                                        crate::i18n::text(if warning {
                                            "remote.pin_use"
                                        } else if confirming {
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
                                                if warning {
                                                    r.remote_accept_pin_warning(cx)
                                                } else {
                                                    r.remote_advance_pin(window, cx)
                                                }
                                            })
                                        },
                                    ),
                                ),
                            ),
                    ),
            )
            .into_any_element()
    }
    fn confirm_remote_action(&mut self, cx: &mut Context<Self>) {
        if self.settings_page.remote.busy || !self.settings_page.remote.confirm {
            return;
        }
        let reset = self.settings_page.remote.reset_identity;
        self.settings_page.remote.confirm = false;
        self.settings_page.remote.reset_identity = false;
        self.remote_request(
            false,
            move |client| {
                if reset {
                    client.remote_reset_identity()?;
                } else {
                    client.remote_logout()?;
                }
                client.remote_status()
            },
            cx,
        );
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
                root.update(cx, |root, cx| root.confirm_remote_action(cx));
            }),
            Rc::new(move |_, cx| {
                cancel.update(cx, |r, cx| {
                    r.settings_page.remote.confirm = false;
                    r.settings_page.remote.reset_identity = false;
                    r.settings_page.remote.enable_local_only = false;
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
                store.update(&mut cx, |store, _| store.settings.local_only = false);
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
                            visual.debug_bounds("REMOTE_RELAY_SIGNED_OUT").is_some(),
                            !signed_in && !waiting
                        );
                        for selector in ["REMOTE_INVITE_DISCORD", "REMOTE_INVITE_WECHAT_README"] {
                            assert_eq!(
                                visual.debug_bounds(selector).is_some(),
                                !signed_in && !waiting
                            );
                        }
                        assert!(visual.debug_bounds("REMOTE_TAILSCALE_DISABLED").is_none());
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
                        if signed_in {
                            let sign_out = visual.debug_bounds("ACCOUNT_SIGN_OUT_SECTION").unwrap();
                            let pane = visual.debug_bounds("ACCOUNT_SETTINGS").unwrap();
                            assert_eq!(sign_out.top(), card.bottom() + px(20.));
                            assert_eq!(pane.bottom(), sign_out.bottom());
                        }
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
                        assert!(visual.debug_bounds("REMOTE_RELAY_SIGNED_OUT").is_none());
                        window
                            .update(&mut cx, |root, _, cx| {
                                root.settings_page.pane = SettingsPane::Remote;
                                cx.notify();
                            })
                            .unwrap();
                    }
                    cx.simulate_window_resize(window.into(), size(px(640.), px(700.)));
                    cx.run_until_parked();
                    let mut visual = VisualTestContext::from_window(window.into(), &cx);
                    let card = visual.debug_bounds("REMOTE_RELAY_SIGNED_OUT").unwrap();
                    for selector in [
                        "REMOTE_SIGN_IN",
                        "REMOTE_INVITE_DISCORD",
                        "REMOTE_INVITE_WECHAT_README",
                    ] {
                        let link = visual.debug_bounds(selector).unwrap();
                        assert!(
                            link.left() >= card.left() && link.right() <= card.right(),
                            "{selector}"
                        );
                        assert!(
                            link.top() >= card.top() && link.bottom() <= card.bottom(),
                            "{selector}"
                        );
                    }
                    window
                        .update(&mut cx, |root, window, cx| {
                            root.enter_settings_pane(SettingsPane::About, window, cx);
                        })
                        .unwrap();
                    cx.run_until_parked();
                    let mut visual = VisualTestContext::from_window(window.into(), &cx);
                    for selector in [
                        "about-discord",
                        "about-wechat-readme",
                        "ABOUT_INVITE_GUIDANCE",
                    ] {
                        assert!(visual.debug_bounds(selector).is_some(), "{selector}");
                    }
                    window
                        .update(&mut cx, |root, _, cx| {
                            root.settings_page.pane = SettingsPane::Remote;
                            cx.notify();
                        })
                        .unwrap();
                    cx.simulate_window_resize(window.into(), size(px(1200.), px(900.)));
                }
            });
        }
    }
    #[derive(Default)]
    struct RemoteRequests {
        calls: std::sync::Mutex<Vec<&'static str>>,
        status: std::sync::Mutex<RemoteStatus>,
        fail_logout: std::sync::atomic::AtomicBool,
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
                Request::remote_set_pin { .. } => {
                    calls.push("set pin");
                    Response::remote_set_pin(Ok(RemoteTrust {
                        pin_set: true,
                        ..Default::default()
                    }))
                }
                Request::remote_logout {} => {
                    calls.push("sign out");
                    if self.fail_logout.load(std::sync::atomic::Ordering::SeqCst) {
                        return Err(runner_core::protocol::ClientError::msg(
                            "test sign-out failed",
                        ));
                    }
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

    fn switch_window(
        cx: &mut TestAppContext,
        store: &gpui::Entity<crate::app_store::AppStore>,
        path: &std::path::Path,
        name: &str,
    ) -> gpui::WindowHandle<NativeRoot> {
        let window = cx.add_window(|window, cx| {
            let mut root = NativeRoot::new(
                name.into(),
                path.join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.route = crate::AppRoute::Chat;
            root
        });
        cx.simulate_window_resize(window.into(), size(px(1440.), px(1100.)));
        window
    }

    #[test]
    fn local_only_defaults_off_and_persists_without_enrollment_across_windows() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for locale in [Locale::English, Locale::SimplifiedChinese] {
            with_locale(locale, || {
                let temp = tempfile::tempdir().unwrap();
                let mut cx = TestAppContext::single();
                let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
                let requests = std::sync::Arc::new(RemoteRequests::default());
                let window = switch_window(&mut cx, &store, temp.path(), "local-only");
                let peer = switch_window(&mut cx, &store, temp.path(), "local-only-peer");
                store.update(&mut cx, |store, _| {
                    store.client = DaemonClient::new(requests.clone())
                });
                window
                    .update(&mut cx, |root, window, cx| {
                        root.enter_settings_pane(SettingsPane::General, window, cx)
                    })
                    .unwrap();
                cx.simulate_window_resize(window.into(), size(px(1440.), px(1100.)));
                cx.run_until_parked();
                let mut visual = VisualTestContext::from_window(window.into(), &cx);
                let mut peer_visual = VisualTestContext::from_window(peer.into(), &cx);
                visual.update(|window, _| window.activate_window());
                assert!(!store.read_with(&visual, |store, _| store.settings.local_only));
                assert!(peer_visual.debug_bounds("SIDEBAR_ACCOUNT").is_some());
                assert!(peer_visual
                    .debug_bounds("COMPUTER_PICKER_TRIGGER")
                    .is_none());
                assert!(visual.debug_bounds("SETTINGS_NAV_account").is_some());
                assert!(visual.debug_bounds("SETTINGS_NAV_remote").is_some());
                for enabled in [true, false] {
                    visual.update(|window, _| window.activate_window());
                    visual.run_until_parked();
                    let toggle = visual.debug_bounds("SETTINGS_LOCAL_ONLY_TOGGLE").unwrap();
                    assert!(
                        toggle.right() <= px(1440.) && toggle.bottom() <= px(1100.),
                        "toggle outside viewport: {toggle:?}"
                    );
                    window
                        .read_with(&visual, |root, _| assert!(!root.settings_page.remote.busy))
                        .unwrap();
                    visual.simulate_click(toggle.center(), Default::default());
                    visual.run_until_parked();
                    assert_eq!(
                        store.read_with(&visual, |store, _| store.settings.local_only),
                        enabled
                    );
                    assert_eq!(
                        crate::app_settings::AppSettings::load(&temp.path().join("settings.json"))
                            .unwrap()
                            .local_only,
                        enabled
                    );
                    assert_eq!(
                        visual.debug_bounds("SETTINGS_NAV_account").is_none(),
                        enabled
                    );
                    assert_eq!(
                        visual.debug_bounds("SETTINGS_NAV_remote").is_none(),
                        enabled
                    );
                    assert_eq!(
                        peer_visual.debug_bounds("SIDEBAR_SETTINGS").is_some(),
                        enabled
                    );
                    assert_eq!(
                        peer_visual.debug_bounds("SIDEBAR_ACCOUNT").is_none(),
                        enabled
                    );
                    assert!(peer_visual
                        .debug_bounds("COMPUTER_PICKER_TRIGGER")
                        .is_none());
                    assert!(requests.calls.lock().unwrap().is_empty());
                    for pane in [SettingsPane::Account, SettingsPane::Remote] {
                        window
                            .update(&mut visual, |root, window, cx| {
                                root.enter_settings_pane(pane, window, cx);
                                assert_eq!(
                                    root.settings_page.pane,
                                    if enabled { SettingsPane::General } else { pane }
                                );
                            })
                            .unwrap();
                        visual.run_until_parked();
                        assert!(visual.debug_bounds("REMOTE_ACCESS_SWITCH").is_none());
                        assert!(visual.debug_bounds("REMOTE_ACCESS_OFF_DETAIL").is_none());
                    }
                    window
                        .update(&mut visual, |root, window, cx| {
                            root.enter_settings_pane(SettingsPane::General, window, cx)
                        })
                        .unwrap();
                    visual.run_until_parked();
                }
            });
        }
    }

    #[test]
    fn local_only_waits_for_successful_sign_out_and_keeps_failure_off() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for fail in [false, true] {
            for status_before_result in [false, true] {
                let temp = tempfile::tempdir().unwrap();
                let mut cx = TestAppContext::single();
                let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
                cx.run_until_parked();
                let requests = std::sync::Arc::new(RemoteRequests::default());
                *requests.status.lock().unwrap() = RemoteStatus {
                    state: RemoteState::Paused,
                    ..Default::default()
                };
                requests
                    .fail_logout
                    .store(fail, std::sync::atomic::Ordering::SeqCst);
                let trust = RemoteTrust {
                    fingerprint: Some("fixture-identity".into()),
                    pin_set: true,
                    ..Default::default()
                };
                store.update(&mut cx, |store, _| {
                    store.settings.local_only = false;
                    store.remote_status = Some(requests.status.lock().unwrap().clone());
                    store.remote_trust = Some(trust.clone());
                });
                let window = switch_window(&mut cx, &store, temp.path(), "local-only-signout");
                let peer = switch_window(&mut cx, &store, temp.path(), "local-only-signout-peer");
                store.update(&mut cx, |store, _| {
                    store.client = DaemonClient::new(requests.clone())
                });
                window
                    .update(&mut cx, |root, window, cx| {
                        root.enter_settings_pane(SettingsPane::General, window, cx)
                    })
                    .unwrap();
                cx.run_until_parked();
                window
                    .update(&mut cx, |root, window, cx| {
                        root.change_local_only(true, window, cx)
                    })
                    .unwrap();
                cx.run_until_parked();
                let mut visual = VisualTestContext::from_window(window.into(), &cx);
                let mut peer_visual = VisualTestContext::from_window(peer.into(), &cx);
                visual.update(|window, _| window.activate_window());
                assert!(visual.debug_bounds("CONFIRM_DIALOG_PANEL").is_some());
                assert!(!store.read_with(&visual, |store, _| store.settings.local_only));
                visual.simulate_keystrokes("escape");
                visual.run_until_parked();
                assert!(requests.calls.lock().unwrap().is_empty());
                window
                    .update(&mut visual, |root, window, cx| {
                        root.change_local_only(true, window, cx)
                    })
                    .unwrap();
                visual.run_until_parked();
                window
                    .update(&mut visual, |root, _, cx| {
                        root.confirm_remote_action(cx);
                        assert!(!root.app_store.read(cx).settings.local_only);
                        assert!(root.settings_page.remote.busy);
                        if status_before_result {
                            root.app_store.update(cx, |store, cx| {
                                store.remote_status = Some(RemoteStatus::default());
                                cx.notify();
                            });
                        }
                    })
                    .unwrap();
                visual.run_until_parked();
                assert_eq!(*requests.calls.lock().unwrap(), ["sign out"]);
                assert_eq!(
                    store.read_with(&visual, |store, _| store.settings.local_only),
                    !fail
                );
                if fail {
                    assert!(visual.debug_bounds("SETTINGS_LOCAL_ONLY_ERROR").is_some());
                    assert!(peer_visual.debug_bounds("SIDEBAR_ACCOUNT").is_some());
                } else {
                    store.update(&mut visual, |store, cx| {
                        store.remote_status = Some(RemoteStatus::default());
                        store.revisions.shell_settings += 1;
                        cx.notify();
                    });
                    visual.run_until_parked();
                    assert!(peer_visual.debug_bounds("SIDEBAR_ACCOUNT").is_none());
                    assert!(peer_visual.debug_bounds("SIDEBAR_SETTINGS").is_some());
                }
                assert_eq!(
                    store.read_with(&visual, |store, _| store.remote_trust.clone()),
                    Some(trust)
                );
            }
        }
    }

    #[test]
    fn cancel_start_ignores_late_daemon_results_and_unlocks_sign_in() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for fail in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            cx.run_until_parked();
            store.update(&mut cx, |store, _| store.settings.local_only = false);
            let window = switch_window(&mut cx, &store, temp.path(), "cancel-start");
            let requests = std::sync::Arc::new(RemoteRequests::default());
            requests
                .fail_logout
                .store(true, std::sync::atomic::Ordering::SeqCst);
            store.update(&mut cx, |store, _| {
                store.client = DaemonClient::new(requests.clone());
            });
            let completed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let pending = completed.clone();
            window
                .update(&mut cx, |root, _, cx| {
                    root.route = crate::AppRoute::Settings;
                    root.settings_page.pane = SettingsPane::Remote;
                    root.remote_request(
                        true,
                        move |client| {
                            pending.store(true, std::sync::atomic::Ordering::SeqCst);
                            if fail {
                                Err(runner_core::protocol::ClientError::msg(
                                    "late startup error",
                                ))
                            } else {
                                client.remote_enroll_start(None, false)
                            }
                        },
                        cx,
                    );
                    assert!(root.settings_page.remote.busy);
                    assert!(root.settings_page.remote.opening);
                })
                .unwrap();
            for _ in 0..100 {
                if completed.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                assert!(cx.dispatcher.tick(true));
            }
            assert!(completed.load(std::sync::atomic::Ordering::SeqCst));
            window
                .update(&mut cx, |root, _, cx| {
                    assert!(root.settings_page.remote.busy);
                    let generation = root.settings_page.remote.request_generation;
                    root.remote_cancel_start(cx);
                    assert!(!root.settings_page.remote.busy);
                    assert!(!root.settings_page.remote.opening);
                    assert_eq!(root.settings_page.remote.request_generation, generation + 1);
                })
                .unwrap();
            cx.run_until_parked();
            assert!(completed.load(std::sync::atomic::Ordering::SeqCst));
            assert!(
                cx.opened_url().is_none(),
                "abandoned start must not open a browser"
            );
            window
                .read_with(&cx, |root, _| {
                    assert!(!root.settings_page.remote.busy);
                    assert!(root.settings_page.remote.error.is_none());
                })
                .unwrap();
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            assert!(visual.debug_bounds("REMOTE_CANCEL_START").is_none());
            let sign_in = visual.debug_bounds("REMOTE_SIGN_IN").unwrap();
            visual.simulate_click(sign_in.center(), Default::default());
            visual.run_until_parked();
            assert!(
                cx.opened_url().is_some(),
                "Sign in must be usable after Cancel"
            );
        }
    }

    #[test]
    fn immediate_sign_in_waits_for_abandoned_start_cleanup_and_skips_undispatched_start() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for dispatch_first in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            cx.run_until_parked();
            store.update(&mut cx, |store, _| store.settings.local_only = false);
            let window = switch_window(&mut cx, &store, temp.path(), "cancel-restart");
            let requests = std::sync::Arc::new(RemoteRequests::default());
            store.update(&mut cx, |store, _| {
                store.client = DaemonClient::new(requests.clone())
            });
            window
                .update(&mut cx, |root, _, cx| {
                    root.route = crate::AppRoute::Settings;
                    root.settings_page.pane = SettingsPane::Remote;
                    root.remote_connect(cx);
                })
                .unwrap();
            if dispatch_first {
                for _ in 0..100 {
                    if !requests.calls.lock().unwrap().is_empty() {
                        break;
                    }
                    assert!(cx.dispatcher.tick(true));
                }
                assert_eq!(*requests.calls.lock().unwrap(), ["sign in"]);
            }
            window
                .update(&mut cx, |root, _, cx| {
                    assert!(root.settings_page.remote.busy);
                    root.remote_cancel_start(cx);
                    assert!(!root.settings_page.remote.busy);
                    assert!(!root.settings_page.remote.opening);
                    root.remote_connect(cx);
                    assert!(root.settings_page.remote.busy);
                })
                .unwrap();
            // Both cleanup and the second Start are queued before background work resumes.
            cx.run_until_parked();
            assert_eq!(
                *requests.calls.lock().unwrap(),
                if dispatch_first {
                    vec!["sign in", "sign out", "sign in"]
                } else {
                    vec!["sign in"]
                }
            );
            assert!(matches!(
                requests.status.lock().unwrap().state,
                RemoteState::Enrolling { .. }
            ));
            assert!(cx.opened_url().is_some());
            window
                .read_with(&cx, |root, _| assert!(!root.settings_page.remote.busy))
                .unwrap();
        }
    }

    #[test]
    fn local_only_cancels_pending_enrollment() {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        cx.run_until_parked();
        let requests = std::sync::Arc::new(RemoteRequests::default());
        *requests.status.lock().unwrap() = RemoteStatus {
            state: RemoteState::Enrolling {
                user_code: "fixture-code".into(),
                verification_uri: "https://runner.example/add".into(),
                verification_uri_complete: "https://runner.example/add".into(),
                expires_at: chrono::Utc::now() + chrono::Duration::minutes(10),
            },
            ..Default::default()
        };
        store.update(&mut cx, |store, _| {
            store.remote_status = Some(requests.status.lock().unwrap().clone());
        });
        let window = switch_window(&mut cx, &store, temp.path(), "local-only-cancel");
        store.update(&mut cx, |store, _| {
            store.client = DaemonClient::new(requests.clone())
        });
        window
            .update(&mut cx, |root, window, cx| {
                root.change_local_only(true, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(*requests.calls.lock().unwrap(), ["cancel"]);
        assert!(store.read_with(&cx, |store, _| store.settings.local_only));
    }

    #[test]
    fn sign_out_and_reset_preserve_local_only_including_signed_in_override() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for local_only in [false, true] {
            for reset in [false, true] {
                let temp = tempfile::tempdir().unwrap();
                let mut cx = TestAppContext::single();
                let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
                cx.run_until_parked();
                let requests = std::sync::Arc::new(RemoteRequests::default());
                *requests.status.lock().unwrap() = RemoteStatus {
                    state: RemoteState::Paused,
                    ..Default::default()
                };
                store.update(&mut cx, |store, _| {
                    store.settings.local_only = local_only;
                    store.remote_status = Some(requests.status.lock().unwrap().clone());
                });
                let window = switch_window(&mut cx, &store, temp.path(), "local-only-preserved");
                let peer = switch_window(&mut cx, &store, temp.path(), "local-only-preserved-peer");
                store.update(&mut cx, |store, _| {
                    store.client = DaemonClient::new(requests.clone())
                });
                window
                    .update(&mut cx, |root, window, cx| {
                        root.enter_settings_pane(
                            if reset {
                                SettingsPane::Remote
                            } else {
                                SettingsPane::Account
                            },
                            window,
                            cx,
                        );
                    })
                    .unwrap();
                cx.run_until_parked();
                let mut visual = VisualTestContext::from_window(window.into(), &cx);
                let mut peer_visual = VisualTestContext::from_window(peer.into(), &cx);
                visual.update(|window, _| window.activate_window());
                assert!(peer_visual.debug_bounds("SIDEBAR_ACCOUNT").is_some());
                assert!(peer_visual
                    .debug_bounds("COMPUTER_PICKER_TRIGGER")
                    .is_some());
                window
                    .update(&mut visual, |root, window, cx| {
                        root.confirm_sign_out(cx);
                        root.settings_page.remote.reset_identity = reset;
                        root.settings_page.remote.focus.focus(window, cx);
                    })
                    .unwrap();
                visual.run_until_parked();
                visual.simulate_keystrokes("enter");
                visual.run_until_parked();
                store.update(&mut visual, |store, cx| {
                    store.remote_status = Some(requests.status.lock().unwrap().clone());
                    store.revisions.shell_settings += 1;
                    cx.notify();
                });
                visual.run_until_parked();
                assert_eq!(
                    *requests.calls.lock().unwrap(),
                    [if reset { "reset" } else { "sign out" }]
                );
                assert_eq!(
                    store.read_with(&visual, |store, _| store.settings.local_only),
                    local_only
                );
                assert!(peer_visual
                    .debug_bounds("COMPUTER_PICKER_TRIGGER")
                    .is_none());
                assert_eq!(
                    peer_visual.debug_bounds("SIDEBAR_SETTINGS").is_some(),
                    local_only
                );
                assert_eq!(
                    peer_visual.debug_bounds("SIDEBAR_ACCOUNT").is_none(),
                    local_only
                );
                window
                    .read_with(&visual, |root, _| {
                        assert_eq!(
                            root.settings_page.pane,
                            if local_only {
                                SettingsPane::General
                            } else if reset {
                                SettingsPane::Remote
                            } else {
                                SettingsPane::Account
                            }
                        )
                    })
                    .unwrap();
            }
        }
    }

    #[test]
    fn local_only_dismisses_other_windows_pin_and_reset_dialogs() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for reset in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            cx.run_until_parked();
            let requests = std::sync::Arc::new(RemoteRequests::default());
            *requests.status.lock().unwrap() = RemoteStatus {
                state: RemoteState::Paused,
                ..Default::default()
            };
            store.update(&mut cx, |store, _| {
                store.remote_status = Some(requests.status.lock().unwrap().clone())
            });
            let first = switch_window(&mut cx, &store, temp.path(), "local-only-first");
            let second = switch_window(&mut cx, &store, temp.path(), "local-only-second");
            store.update(&mut cx, |store, _| {
                store.client = DaemonClient::new(requests.clone())
            });
            second
                .update(&mut cx, |root, window, cx| {
                    root.enter_settings_pane(SettingsPane::Remote, window, cx)
                })
                .unwrap();
            cx.run_until_parked();
            second
                .update(&mut cx, |root, window, cx| {
                    if reset {
                        root.settings_page.remote.confirm = true;
                        root.settings_page.remote.reset_identity = true;
                        root.settings_page.remote.focus.focus(window, cx);
                    } else {
                        root.remote_open_pin(cx);
                    }
                    cx.notify();
                })
                .unwrap();
            cx.run_until_parked();
            let mut peer = VisualTestContext::from_window(second.into(), &cx);
            assert!(peer
                .debug_bounds(if reset {
                    "CONFIRM_DIALOG_PANEL"
                } else {
                    "REMOTE_PIN_DIALOG"
                })
                .is_some());
            first
                .update(&mut cx, |root, window, cx| {
                    root.enter_settings_pane(SettingsPane::General, window, cx);
                    root.change_local_only(true, window, cx)
                })
                .unwrap();
            cx.run_until_parked();
            first
                .update(&mut cx, |root, _, cx| root.confirm_remote_action(cx))
                .unwrap();
            cx.run_until_parked();
            store.update(&mut cx, |store, cx| {
                store.remote_status = Some(requests.status.lock().unwrap().clone());
                store.revisions.shell_settings += 1;
                cx.notify();
            });
            cx.run_until_parked();
            assert!(peer.debug_bounds("REMOTE_PIN_DIALOG").is_none());
            assert!(peer.debug_bounds("CONFIRM_DIALOG_PANEL").is_none());
            second
                .read_with(&cx, |root, _| {
                    assert!(!root.remote_overlay_open());
                    assert_eq!(root.settings_page.pane, SettingsPane::General);
                })
                .unwrap();
            cx.simulate_keystrokes(second.into(), "enter");
            cx.run_until_parked();
            assert_eq!(*requests.calls.lock().unwrap(), ["sign out"]);
        }
    }

    #[test]
    fn account_actions_reuse_requests_and_sign_out_and_reset_require_confirmation() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        store.update(&mut cx, |store, _| store.settings.local_only = false);
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
            fail_logout: Default::default(),
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
        assert!(visual.debug_bounds("REMOTE_RELAY_SIGNED_OUT").is_some());
        assert!(visual.debug_bounds("REMOTE_TAILSCALE_DISABLED").is_none());
        let sign_in = visual.debug_bounds("REMOTE_SIGN_IN").unwrap();
        visual.simulate_click(sign_in.center(), Default::default());
        cx.run_until_parked();
        assert_eq!(requests.calls.lock().unwrap().last(), Some(&"sign in"));
    }

    #[test]
    fn pin_confirmation_and_lock_warning_keep_secrets_out_of_errors() {
        let pin = Secret("001234".into());
        assert!(confirmed_pin(&pin, &pin).unwrap().0 == pin.0);
        for weak in ["000000", "123456", "111111", "121212", "654321"] {
            let weak = Secret(weak.into());
            assert!(confirmed_pin(&weak, &weak).unwrap().0 == weak.0);
        }
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
    fn weak_pin_warning_requires_an_explicit_choice_and_keeps_the_pin_private() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for locale in [Locale::English, Locale::SimplifiedChinese] {
            with_locale(locale, || {
                for action in ["choose", "enter", "escape", "mouse", "keyboard"] {
                    let temp = tempfile::tempdir().unwrap();
                    let mut cx = TestAppContext::single();
                    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
                    store.update(&mut cx, |store, _| store.settings.local_only = false);
                    let window = cx.add_window(|window, cx| {
                        let mut root = NativeRoot::new(
                            "weak-pin-warning".into(),
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
                    cx.simulate_keystrokes(window.into(), "0 0 0 0 0 0 enter 0 0 0 0 0 0 enter");
                    cx.run_until_parked();
                    let mut visual = VisualTestContext::from_window(window.into(), &cx);
                    assert!(visual.debug_bounds("REMOTE_PIN_WARNING").is_some());
                    assert!(visual.debug_bounds("REMOTE_PIN_BOX_0").is_none());
                    assert!(!crate::i18n::text("remote.pin_weak").contains("000000"));
                    assert!(crate::i18n::text("remote.pin_weak").contains(match locale {
                        Locale::English => "five",
                        Locale::SimplifiedChinese => "五",
                    }));
                    for focus_next in [false, true] {
                        if focus_next {
                            cx.simulate_keystrokes(window.into(), "tab");
                        }
                        for key in ["enter", "space"] {
                            visual.simulate_event(gpui::KeyDownEvent {
                                keystroke: gpui::Keystroke::parse(key).unwrap(),
                                is_held: true,
                                prefer_character_input: false,
                            });
                        }
                        assert!(visual.debug_bounds("REMOTE_PIN_WARNING").is_some());
                        window
                            .read_with(&cx, |root, _| {
                                assert_eq!(root.settings_page.remote.request_generation, 0);
                            })
                            .unwrap();
                    }
                    cx.simulate_keystrokes(window.into(), "shift-tab");
                    window
                        .update(&mut cx, |root, window, _| {
                            assert_eq!(root.settings_page.remote.request_generation, 0);
                            assert!(root
                                .settings_page
                                .remote
                                .pin
                                .as_ref()
                                .unwrap()
                                .secondary_focus
                                .is_focused(window));
                            assert!(root.toasts.active().is_none());
                        })
                        .unwrap();
                    match action {
                        "choose" => {
                            let button = visual.debug_bounds("REMOTE_PIN_SECONDARY").unwrap();
                            visual.simulate_click(button.center(), Default::default());
                        }
                        "enter" => cx.simulate_keystrokes(window.into(), "enter"),
                        "escape" => cx.simulate_keystrokes(window.into(), "escape"),
                        "mouse" => {
                            let button = visual.debug_bounds("REMOTE_PIN_PRIMARY").unwrap();
                            visual.simulate_click(button.center(), Default::default());
                        }
                        _ => {
                            cx.simulate_keystrokes(window.into(), "tab shift-tab tab space");
                        }
                    }
                    window
                        .read_with(&cx, |root, cx| {
                            let submitted = matches!(action, "mouse" | "keyboard");
                            assert_eq!(root.settings_page.remote.request_generation > 0, submitted);
                            if matches!(action, "choose" | "enter") {
                                let dialog = root.settings_page.remote.pin.as_ref().unwrap();
                                assert_eq!(dialog.step, 0);
                                assert!(dialog
                                    .fields
                                    .iter()
                                    .all(|field| { field.read(cx).value().0.is_empty() }));
                            } else {
                                assert!(root.settings_page.remote.pin.is_none());
                            }
                        })
                        .unwrap();
                }
            });
        }
    }

    #[test]
    fn two_step_pin_requires_six_digits_clears_on_back_and_mismatch_and_submits_on_enter() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        with_locale(Locale::English, || {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            store.update(&mut cx, |store, _| store.settings.local_only = false);
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
                store.update(&mut cx, |store, _| store.settings.local_only = false);
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
