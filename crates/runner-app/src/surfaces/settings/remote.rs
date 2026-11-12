use crate::surfaces::settings::page::{settings_section_heading, SettingsPane};
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
    prompt_pin_after_sign_in: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    error: Option<String>,
    pub(crate) confirm: bool,
    reset_identity: bool,
    pin: Option<PinDialog>,
    pub(crate) focus: gpui::FocusHandle,
    generation: u64,
}
type EnrollmentTask =
    futures::future::Shared<gpui::Task<Result<RemoteStatus, runner_core::protocol::ClientError>>>;
struct EnrollmentStart {
    abandoned: std::sync::Arc<std::sync::atomic::AtomicBool>,
    task: EnrollmentTask,
}
#[derive(Default)]
pub(crate) struct EnrollmentRequests {
    active: futures::lock::Mutex<Option<std::sync::Arc<std::sync::atomic::AtomicBool>>>,
}
impl EnrollmentRequests {
    async fn start(
        &self,
        abandoned: std::sync::Arc<std::sync::atomic::AtomicBool>,
        request: impl std::future::Future<
            Output = Result<RemoteStatus, runner_core::protocol::ClientError>,
        >,
    ) -> Result<RemoteStatus, runner_core::protocol::ClientError> {
        let mut active = self.active.lock().await;
        if abandoned.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(runner_core::protocol::ClientError::msg(
                "Enrollment cancelled.",
            ));
        }
        if let Some(previous) = active.replace(abandoned) {
            previous.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        request.await
    }
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
            prompt_pin_after_sign_in: None,
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
    pub(crate) fn leave_other_computers_when_signed_out(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        if self.app_store.read(cx).remote_signed_in() {
            return;
        }
        if self.remote_client.ui.picker_signed_in {
            self.remote_client.ui.picker_open = false;
            self.remote_client.ui.picker_signed_in = false;
        }
        if self.remote_client.ui.pair.take().is_some() {
            self.remote_client.ui.pair_generation += 1;
        }
        if self.remote_client.is_active() {
            self.return_to_this_computer(window, cx);
        }
    }

    pub(crate) fn enter_remote_settings(&mut self, cx: &mut Context<Self>) {
        self.remote_client.refresh_computers(cx);
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
        let task = if open_browser {
            let requests = self.app_store.read(cx).enrollment_requests.clone();
            let abandoned = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            self.settings_page.remote.prompt_pin_after_sign_in = Some(abandoned.clone());
            let cancelled = abandoned.clone();
            let task = cx
                .background_spawn(async move {
                    requests
                        .start(cancelled, async move { request(client) })
                        .await
                })
                .shared();
            self.settings_page.remote.enrollment_start = Some(EnrollmentStart {
                abandoned,
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
                        if open_browser {
                            root.settings_page.remote.prompt_pin_after_sign_in = None;
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
    pub(crate) fn remote_finish_sign_in(&mut self, cx: &mut Context<Self>) {
        let Some(attempt) = self.settings_page.remote.prompt_pin_after_sign_in.take() else {
            return;
        };
        if attempt.load(std::sync::atomic::Ordering::SeqCst)
            || self.route != crate::AppRoute::Settings
            || !self
                .app_store
                .read(cx)
                .remote_status
                .as_ref()
                .is_some_and(|status| {
                    matches!(
                        status.state,
                        RemoteState::Connecting { .. } | RemoteState::Connected { .. }
                    )
                })
        {
            return;
        }
        let generation = self.settings_page.remote.request_generation;
        let client = self.core(cx).clone();
        let requests = self.app_store.read(cx).enrollment_requests.clone();
        let owner = attempt.clone();
        // Read after enrollment completes; the earlier trust snapshot may predate secret restoration.
        let trust = cx.background_spawn(async move {
            let active = requests.active.lock().await;
            if !active
                .as_ref()
                .is_some_and(|active| std::sync::Arc::ptr_eq(active, &owner))
            {
                return None;
            }
            client.remote_trust().ok()
        });
        cx.spawn(async move |weak, cx| {
            let Some(trust) = trust.await else {
                return;
            };
            let _ = weak.update_in(cx, |root, window, cx| {
                if !trust.pin_set
                    && !attempt.load(std::sync::atomic::Ordering::SeqCst)
                    && root.route == crate::AppRoute::Settings
                    && !root.remote_overlay_open()
                    && root.settings_page.remote.request_generation == generation
                    && root
                        .app_store
                        .read(cx)
                        .remote_status
                        .as_ref()
                        .is_some_and(account::signed_in)
                {
                    root.remote_open_pin(cx);
                    root.settings_page.remote.pin.as_ref().unwrap().fields[0]
                        .read(cx)
                        .focus
                        .clone()
                        .focus(window, cx);
                }
            });
        })
        .detach();
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
                .child(self.remote_client.render_computers(&trust, &status, cx))
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
            .when(!trust.pin_set && !trust.pin_locked, |card| {
                card.child(
                    account::divider()
                        .debug_selector(|| "REMOTE_PIN_SETUP".into())
                        .bg(theme::with_alpha(theme::warning(), 0.06))
                        .child(
                            SettingsRow::new(
                                crate::i18n::text("remote.pin_setup"),
                                self.remote_button(
                                    "remote-setup-pin",
                                    "remote.set_pin",
                                    Self::remote_open_pin,
                                    cx,
                                )
                                .variant(ButtonVariant::Primary),
                            )
                            .subtitle(crate::i18n::text("remote.pin_setup_detail"))
                            .label_icon("key-round.svg", theme::warning()),
                        ),
                )
            })
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
        if trust.pin_set || trust.pin_locked {
            rows.push(
                div()
                    .debug_selector(|| "REMOTE_PIN_ROW".into())
                    .child(
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
                        })),
                    )
                    .into_any_element(),
            );
        }
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
            .map(crate::platform_ui::window_scrim)
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
mod tests;
