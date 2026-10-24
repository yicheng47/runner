use super::*;

pub(crate) fn signed_in(status: &RemoteStatus) -> bool {
    matches!(
        status.state,
        RemoteState::Connecting { .. }
            | RemoteState::Connected { .. }
            | RemoteState::Reconnecting { .. }
            | RemoteState::Paused
    )
}

pub(crate) fn account_initial(status: &RemoteStatus) -> Option<String> {
    signed_in(status)
        .then_some(status.account.as_ref())
        .flatten()
        .and_then(|account| account.display_name.trim().chars().next())
        .map(|initial| initial.to_uppercase().collect())
}

pub(crate) fn avatar(status: &RemoteStatus, size: f32) -> AnyElement {
    div()
        .flex_none()
        .size(rems(size / 16.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(theme::raised())
        .text_color(if signed_in(status) {
            theme::text()
        } else {
            theme::muted()
        })
        .font_weight(FontWeight::SEMIBOLD)
        .map(|avatar| match account_initial(status) {
            Some(initial) => avatar.child(initial),
            None => avatar.child(avatar_icon(size)),
        })
        .into_any_element()
}

fn avatar_icon(size: f32) -> gpui::Svg {
    svg()
        .path("user-round.svg")
        .size(rems(size * 0.45 / 16.))
        .text_color(theme::muted())
}

pub(super) fn card() -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .overflow_hidden()
        .rounded(rems(12. / 16.))
        .border_1()
        .border_color(theme::border())
        .bg(theme::panel())
        .text_size(theme::text_body())
}

pub(super) fn divider() -> gpui::Div {
    div().border_t_1().border_color(theme::border())
}

pub(super) fn badge(key: &'static str, beta: bool) -> AnyElement {
    div()
        .flex_none()
        .rounded_full()
        .px_2()
        .py(rems(1. / 16.))
        .bg(if beta {
            theme::with_alpha(theme::warning(), 0.12)
        } else {
            theme::raised()
        })
        .text_color(if beta {
            theme::warning()
        } else {
            theme::muted()
        })
        .text_size(theme::text_micro())
        .child(crate::i18n::text(key))
        .into_any_element()
}

pub(super) fn footer(key: &'static str) -> AnyElement {
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
                .mt(rems(2. / 16.))
                .text_color(theme::faint()),
        )
        .child(crate::i18n::text(key))
        .into_any_element()
}

pub(super) fn approval_reminder() -> AnyElement {
    div()
        .debug_selector(|| "ACCOUNT_APPROVAL_REMINDER".into())
        .flex()
        .items_center()
        .gap_2()
        .text_size(theme::text_meta())
        .text_color(theme::faint())
        .child(
            svg()
                .path("info.svg")
                .size(rems(12. / 16.))
                .text_color(theme::faint()),
        )
        .child(crate::i18n::text("remote.approve_detail"))
        .into_any_element()
}

impl NativeRoot {
    pub(crate) fn render_account_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let store = self.app_store.read(cx);
        let status = store.remote_status.clone().unwrap_or_default();
        let trust = store.remote_trust.clone().unwrap_or_default();
        let signed_in = signed_in(&status);
        let mut body = div()
            .debug_selector(|| "ACCOUNT_SETTINGS".into())
            .flex()
            .flex_col()
            .gap_5()
            .child(PaneHeader::new(
                crate::i18n::text("account.title"),
                crate::i18n::text("account.subtitle"),
            ))
            .child(self.account_status_card(&status, &trust, cx));
        if matches!(status.state, RemoteState::Enrolling { .. }) {
            body = body.child(approval_reminder());
        }
        if signed_in {
            body = body.child(
                div()
                    .debug_selector(|| "ACCOUNT_SIGN_OUT_SECTION".into())
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(settings_section_heading(crate::i18n::text(
                        "account.sign_out",
                    )))
                    .child(SettingsCard::new(vec![SettingsRow::new(
                        crate::i18n::text("account.sign_out_computer"),
                        self.remote_button(
                            "account-sign-out",
                            "account.sign_out",
                            Self::confirm_sign_out,
                            cx,
                        )
                        .variant(ButtonVariant::Danger),
                    )
                    .subtitle(tr(
                        "account.sign_out_detail",
                        &[
                            ("account", username(&status)),
                            ("computer", status.host_name.as_deref().unwrap_or_default()),
                        ],
                    ))
                    .into_any_element()])),
            );
        }
        body.into_any_element()
    }

    pub(crate) fn confirm_sign_out(&mut self, cx: &mut Context<Self>) {
        if self.remote_window.is_some() {
            return;
        }
        self.settings_page.remote.reset_identity = false;
        self.settings_page.remote.enable_local_only = false;
        self.settings_page.remote.confirm = true;
        cx.notify();
    }

    pub(super) fn settings_link(
        &self,
        pane: SettingsPane,
        id: &'static str,
        key: &'static str,
        cx: &Context<Self>,
    ) -> Button {
        let root = cx.entity();
        Button::new(id, crate::i18n::text(key))
            .size(ButtonSize::Sm)
            .variant(ButtonVariant::Ghost)
            .on_press(move |window, cx| {
                root.update(cx, |r, cx| r.enter_settings_pane(pane, window, cx))
            })
    }

    pub(super) fn account_status_card(
        &self,
        status: &RemoteStatus,
        trust: &RemoteTrust,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let signed_in = signed_in(status);
        let mut content = card().debug_selector(|| "ACCOUNT_STATUS_CARD".into());
        if matches!(
            status.state,
            RemoteState::Enrolling { .. } | RemoteState::Confirming { .. }
        ) {
            content = content.child(self.enrollment_card(status, trust, cx));
        } else if let RemoteState::Stopped { reason } = status.state {
            content = content.child(self.account_stop_card(status, reason, cx));
        } else {
            let name = status
                .account
                .as_ref()
                .map(|a| a.display_name.clone())
                .unwrap_or_default();
            content = content.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .p_5()
                    .child(avatar(status, 44.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(theme::text_lead())
                                    .child(if signed_in {
                                        name
                                    } else {
                                        crate::i18n::text("account.signed_out").into()
                                    }),
                            )
                            .child(
                                div()
                                    .text_size(theme::text_ui())
                                    .text_color(theme::muted())
                                    .child(if signed_in {
                                        tr("account.signed_in", &[("account", username(status))])
                                    } else {
                                        crate::i18n::text("account.beta_detail").into()
                                    }),
                            ),
                    )
                    .child(badge("remote.private_beta", true)),
            );
            if signed_in {
                content = content.child(
                    divider()
                        .debug_selector(|| "ACCOUNT_DETAILS".into())
                        .p_5()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .child(
                            div()
                                .flex()
                                .gap_5()
                                .child(detail("remote.account", account_name(status), false))
                                .child(detail("remote.service", status.origin.clone(), false)),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_5()
                                .child(detail(
                                    "account.this_computer",
                                    status.host_name.clone().unwrap_or_default(),
                                    false,
                                ))
                                .child(detail(
                                    "remote.fingerprint",
                                    fingerprint(status, trust),
                                    true,
                                )),
                        ),
                );
            } else {
                content = content.child(
                    divider()
                        .p_4()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div().debug_selector(|| "ACCOUNT_SIGN_IN".into()).child(
                                self.remote_button(
                                    "account-sign-in",
                                    "account.sign_in",
                                    Self::remote_connect,
                                    cx,
                                )
                                .variant(ButtonVariant::Primary),
                            ),
                        )
                        .child(
                            div()
                                .text_size(theme::text_meta())
                                .text_color(theme::faint())
                                .child(crate::i18n::text("account.sign_in_hint")),
                        ),
                );
                if self.settings_page.remote.busy && self.settings_page.remote.opening {
                    content = content.child(self.cancel_enrollment_start(cx));
                }
            }
        }
        if let Some(error) = self
            .settings_page
            .remote
            .error
            .as_ref()
            .or(status.last_error.as_ref())
        {
            content = content.child(
                div()
                    .px_5()
                    .pb_4()
                    .text_color(theme::danger())
                    .child(error.clone()),
            );
        }
        content.into_any_element()
    }

    pub(super) fn cancel_enrollment_start(&self, cx: &Context<Self>) -> AnyElement {
        let root = cx.entity();
        div()
            .px_4()
            .pb_4()
            .child(
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
            )
            .into_any_element()
    }

    fn enrollment_card(
        &self,
        status: &RemoteStatus,
        trust: &RemoteTrust,
        cx: &Context<Self>,
    ) -> AnyElement {
        let mut content = div().flex().flex_col();
        let mut header =
            div()
                .p_5()
                .flex()
                .items_center()
                .gap_3()
                .child(runner_app::ui::button::spinner(
                    "account-waiting-spinner",
                    16.,
                    theme::muted(),
                ));
        match &status.state {
            RemoteState::Enrolling {
                user_code,
                verification_uri,
                verification_uri_complete,
                expires_at,
            } => {
                let seconds = expires_at
                    .signed_duration_since(chrono::Utc::now())
                    .num_seconds()
                    .max(0);
                header = header.child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(crate::i18n::text("remote.waiting")),
                        )
                        .child(
                            div()
                                .text_size(theme::text_ui())
                                .text_color(theme::muted())
                                .child(tr(
                                    "account.waiting_detail",
                                    &[("time", &format!("{}:{:02}", seconds / 60, seconds % 60))],
                                )),
                        ),
                );
                let open = verification_uri_complete.clone();
                let copy = open.clone();
                content = content
                    .child(header)
                    .child(
                        divider()
                            .p_5()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .font_family(theme::SYSTEM_MONOSPACE_FONT)
                                    .text_size(rems(30. / 16.))
                                    .child(user_code.clone()),
                            )
                            .child(
                                div()
                                    .text_size(theme::text_meta())
                                    .text_color(theme::faint())
                                    .child(tr("account.code_hint", &[("link", verification_uri)])),
                            ),
                    )
                    .child(
                        divider()
                            .p_5()
                            .flex()
                            .gap_5()
                            .child(detail(
                                "remote.fingerprint",
                                fingerprint(status, trust),
                                true,
                            ))
                            .child(detail("remote.service", status.origin.clone(), false)),
                    )
                    .child(
                        divider()
                            .p_4()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new(
                                    "remote-browser",
                                    crate::i18n::text("remote.open_browser"),
                                )
                                .size(ButtonSize::Sm)
                                .icon("external-link.svg")
                                .on_press(move |_, cx| cx.open_url(&open)),
                            )
                            .child(
                                Button::new("remote-copy", crate::i18n::text("remote.copy_link"))
                                    .size(ButtonSize::Sm)
                                    .icon("copy.svg")
                                    .variant(ButtonVariant::Secondary)
                                    .on_press(move |_, cx| {
                                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                            copy.clone(),
                                        ))
                                    }),
                            )
                            .child(div().flex_1())
                            .child(self.remote_button(
                                "remote-cancel",
                                "ui.cancel",
                                |r, cx| r.remote_request(false, |c| c.remote_enroll_cancel(), cx),
                                cx,
                            )),
                    );
            }
            RemoteState::Confirming { account, .. } => {
                content = content.child(
                    header.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(crate::i18n::text("remote.confirming"))
                            .child(div().text_color(theme::muted()).child(tr(
                                "remote.cli_confirmation",
                                &[("account", &account.username)],
                            ))),
                    ),
                );
            }
            _ => unreachable!(),
        }
        content.into_any_element()
    }

    fn account_stop_card(
        &self,
        status: &RemoteStatus,
        reason: StopReason,
        cx: &Context<Self>,
    ) -> AnyElement {
        let mut actions = div().flex().gap_2();
        if matches!(reason, StopReason::Removed | StopReason::InUseElsewhere)
            && status.host_id.is_some()
        {
            actions = actions.child(div().debug_selector(|| "ACCOUNT_RETRY".into()).child(
                self.remote_button(
                    "remote-retry",
                    "remote.try_again",
                    |r, cx| r.remote_request(false, |c| c.remote_retry(), cx),
                    cx,
                ),
            ));
        }
        if matches!(
            reason,
            StopReason::Removed | StopReason::Denied | StopReason::Expired
        ) {
            actions = actions.child(
                div()
                    .debug_selector(|| "ACCOUNT_SIGN_IN_AGAIN".into())
                    .child(
                        self.remote_button(
                            "remote-again",
                            "account.sign_in_again",
                            Self::remote_connect,
                            cx,
                        )
                        .variant(ButtonVariant::Primary),
                    ),
            );
        }
        if reason == StopReason::InUseElsewhere {
            actions = actions.child(
                div()
                    .debug_selector(|| "ACCOUNT_STOP_SIGN_OUT".into())
                    .child(
                        self.remote_button(
                            "remote-stop-disconnect",
                            "account.sign_out",
                            Self::confirm_sign_out,
                            cx,
                        )
                        .variant(ButtonVariant::Danger),
                    ),
            );
        }
        if reason == StopReason::UpdateRequired {
            actions = actions.child(
                div()
                    .debug_selector(|| "ACCOUNT_CHECK_UPDATES".into())
                    .child(
                        self.settings_link(
                            SettingsPane::Updates,
                            "remote-updates",
                            "remote.check_updates",
                            cx,
                        )
                        .variant(ButtonVariant::Primary),
                    ),
            );
        }
        let title = match reason {
            StopReason::Removed => "remote.removed_title",
            StopReason::InUseElsewhere => "remote.in_use_title",
            StopReason::UpdateRequired => "remote.update_title",
            StopReason::Denied => "remote.denied_title",
            StopReason::Expired => "remote.expired_title",
        };
        div()
            .p_5()
            .flex()
            .items_center()
            .gap_4()
            .child(div().size(rems(8. / 16.)).rounded_full().flex_none().bg(
                if matches!(reason, StopReason::Denied | StopReason::Expired) {
                    theme::muted()
                } else {
                    theme::danger()
                },
            ))
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
                    .child(div().text_color(theme::muted()).child(stopped_text(status))),
            )
            .child(actions)
            .into_any_element()
    }
}

pub(crate) fn username(status: &RemoteStatus) -> &str {
    status
        .account
        .as_ref()
        .map(|a| a.username.as_str())
        .unwrap_or_default()
}
pub(super) fn account_name(status: &RemoteStatus) -> String {
    status
        .account
        .as_ref()
        .map(|a| format!("@{} · {}", a.username, a.display_name))
        .unwrap_or_default()
}
pub(super) fn fingerprint(status: &RemoteStatus, trust: &RemoteTrust) -> String {
    trust
        .fingerprint
        .clone()
        .or_else(|| status.fingerprint.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_out_avatar_svg_has_its_own_paint_color() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for variant in [
            theme::ThemeVariant::Carbon,
            theme::ThemeVariant::RunnerLight,
        ] {
            theme::set_active_variant(variant);
            for size in [26., 32., 44.] {
                assert_eq!(avatar_icon(size).style().text.color, Some(theme::muted()));
            }
        }
    }
}
