use super::*;

/// Agents whose npm `latest` is newer than the installed version.
pub(crate) fn agents_with_updates(core: &DaemonClient) -> Vec<Runtime> {
    core.runtime_status_list()
        .ok()
        .map(|status| {
            status
                .runtimes
                .into_iter()
                .filter(|runtime| runtime.available_version.is_some())
                .map(|runtime| runtime.name)
                .collect()
        })
        .unwrap_or_default()
}

/// The Agent settings gear carries a dot while any enabled agent has an
/// update available; a disabled agent's update does not count.
pub(crate) fn update_dot_visible(
    updates: &[Runtime],
    settings: &app_settings::AppSettings,
) -> bool {
    updates.iter().any(|runtime| {
        settings.is_agent_enabled(
            *runtime,
            runner_core::protocol::runtime_metadata::runtime_default_enabled(*runtime),
        )
    })
}

pub(crate) fn usage_installed(core: &DaemonClient) -> Vec<Runtime> {
    core.runtime_status_list()
        .ok()
        .map(|status| {
            status
                .runtimes
                .into_iter()
                .filter(|runtime| {
                    crate::runtime_ui::catalog_capabilities(&[], runtime.name.key()).usage
                        && matches!(
                            runtime.effective_source,
                            Some(
                                runner_core::protocol::RuntimeCommandSource::Detected
                                    | runner_core::protocol::RuntimeCommandSource::Override
                            )
                        )
                })
                .map(|runtime| runtime.name)
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum UsageTone {
    Normal,
    Warning,
    Danger,
}

pub(super) fn usage_tone(windows: impl IntoIterator<Item = f64>) -> UsageTone {
    let max = windows.into_iter().fold(0.0_f64, f64::max);
    if max >= 100. {
        UsageTone::Danger
    } else if max >= 80. {
        UsageTone::Warning
    } else {
        UsageTone::Normal
    }
}

pub(super) fn usage_color(tone: UsageTone) -> gpui::Hsla {
    match tone {
        UsageTone::Normal => theme::muted(),
        UsageTone::Warning => theme::warning(),
        UsageTone::Danger => theme::danger(),
    }
}

pub(super) fn reset_label(
    resets_at: Option<chrono::DateTime<chrono::Utc>>,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let Some(reset) = resets_at else {
        return crate::i18n::text("ui.reset_unknown").into();
    };
    let seconds = reset.signed_duration_since(now).num_seconds().max(0);
    if seconds < 60 {
        return crate::i18n::text("ui.resets_1m").into();
    }
    let minutes = (seconds + 59) / 60;
    let days = minutes / 1440;
    let hours = minutes % 1440 / 60;
    let mins = minutes % 60;
    if days > 0 {
        crate::i18n::tf(
            "format.resets_d_h",
            &[("days", (days).to_string()), ("hours", (hours).to_string())],
        )
    } else if hours > 0 {
        crate::i18n::tf(
            "format.resets_h_m",
            &[("hours", (hours).to_string()), ("mins", (mins).to_string())],
        )
    } else {
        crate::i18n::tf("format.resets_m", &[("mins", (mins).to_string())])
    }
}

pub(super) fn unavailable_line(
    reason: Option<UnavailableReason>,
    runtime: Runtime,
) -> &'static str {
    match reason {
        Some(UnavailableReason::SignIn) => {
            crate::i18n::text("ui.sign_in_to_claude_code_to_see_usage")
        }
        Some(UnavailableReason::KeychainDenied) => crate::i18n::text(
            "ui.runner_was_not_allowed_to_read_claude_code_s_sign_in_from_the_keychain",
        ),
        Some(UnavailableReason::KeychainUnavailable) => {
            crate::i18n::text("ui.couldn_t_read_claude_code_s_sign_in_from_the_keychain")
        }
        Some(UnavailableReason::ClaudeUnreachable) => {
            crate::i18n::text("ui.couldn_t_reach_anthropic")
        }
        Some(UnavailableReason::CodexNoAnswer) => crate::i18n::text("ui.codex_didn_t_answer"),
        Some(UnavailableReason::InvalidResponse) => {
            crate::runtime_ui::runtime_ui(runtime).usage_invalid
        }
        None => crate::i18n::text("ui.checking"),
    }
}

pub(super) fn weekly_usage_percent(runtime: Runtime, usage: Option<&AgentUsage>) -> Option<f64> {
    let name = crate::runtime_ui::runtime_ui(runtime).usage_week;
    usage?
        .windows
        .iter()
        .find(|window| window.name == name)
        .map(|window| window.used_percent)
}

/// The runtimes the usage footer and popover show: usage-capable, installed and
/// enabled in Settings, whether or not they have run a session (#752), in
/// `Runtime::ALL` order.
pub(super) fn visible_usage_runtimes(installed: &[Runtime], enabled: &[Runtime]) -> Vec<Runtime> {
    Runtime::ALL
        .into_iter()
        .filter(|runtime| {
            crate::runtime_ui::catalog_capabilities(&[], runtime.key()).usage
                && installed.contains(runtime)
                && enabled.contains(runtime)
        })
        .collect()
}

pub(super) fn usage_popover_runtimes(installed: &[Runtime], enabled: &[Runtime]) -> Vec<Runtime> {
    let mut runtimes = visible_usage_runtimes(installed, enabled);
    runtimes.sort_by_key(|runtime| crate::runtime_ui::runtime_ui(*runtime).usage_order);
    runtimes
}

pub(super) fn sidebar_usage_element(
    snapshot: &UsageSnapshot,
    visible: &[Runtime],
    zoom: f32,
) -> gpui::Stateful<Div> {
    let entries: Vec<_> = visible
        .iter()
        .map(|runtime| {
            let usage = snapshot.get(*runtime);
            let percent = weekly_usage_percent(*runtime, usage);
            let value = percent.map_or_else(|| "—".to_owned(), |value| format!("{value:.0}%"));
            let color = percent.map_or(theme::muted(), |value| usage_color(usage_tone([value])));
            div()
                .when(cfg!(test), |entry| {
                    let runtime = *runtime;
                    entry.debug_selector(move || format!("SIDEBAR_USAGE_ENTRY_{}", runtime.key()))
                })
                .flex_none()
                .flex()
                .items_center()
                .gap(px(2. * zoom))
                .child({
                    let icon = ChatIcon::for_runtime(runtime.key());
                    icon.render(px(12. * zoom), icon.color(theme::text(), true), true)
                })
                .child(
                    div()
                        .when(cfg!(test), |value| {
                            let runtime = *runtime;
                            value.debug_selector(move || {
                                format!("SIDEBAR_USAGE_VALUE_{}", runtime.key())
                            })
                        })
                        .text_size(px(11. * zoom))
                        .text_color(color)
                        .child(value),
                )
        })
        .collect();
    div()
        .id("sidebar-usage")
        .when(cfg!(test), |usage| {
            usage.debug_selector(|| "SIDEBAR_USAGE".into())
        })
        .w_full()
        .h(px(32. * zoom))
        .flex()
        .items_center()
        .justify_between()
        .px(px(10. * zoom))
        .rounded(px(6. * zoom))
        .children(entries)
}

pub(super) fn usage_window_label(label: &str) -> String {
    match label {
        "Week" => crate::i18n::text("ui.week").into(),
        "week" => crate::i18n::text("usage.week_scoped").into(),
        "5 hours" => crate::i18n::text("usage.five_hours").into(),
        _ => label
            .strip_suffix(" min")
            .filter(|minutes| minutes.parse::<u64>().is_ok())
            .map(|minutes| crate::i18n::tf("usage.minutes", &[("minutes", minutes.into())]))
            .unwrap_or_else(|| label.into()),
    }
}

pub(super) fn usage_window_name(name: &str) -> String {
    let (group, label) = name
        .split_once(" · ")
        .map_or((None, name), |(group, label)| (Some(group), label));
    let label = if let Some(label) = label.strip_suffix(" used") {
        crate::i18n::tf("usage.used", &[("window", usage_window_label(label))])
    } else {
        usage_window_label(label)
    };
    group.map_or_else(|| label.clone(), |group| format!("{group} · {label}"))
}

pub(super) fn usage_window_row(
    runtime: Runtime,
    window: &UsageWindow,
    label: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> AnyElement {
    let tone = usage_tone([window.used_percent]);
    let fill = usage_color(tone);
    div()
        .w_full()
        .flex()
        .items_center()
        .gap_2()
        .child(Tooltip::new(
            format!("usage-window-{}-{}", runtime.key(), window.name),
            usage_window_name(&window.name),
            div()
                .w(rems(76. / 16.))
                .flex_none()
                .truncate()
                .text_size(theme::text_meta())
                .text_color(theme::text())
                .child(label.to_owned()),
        ))
        .child(
            div()
                .flex_1()
                .h(rems(4. / 16.))
                .rounded_full()
                .bg(theme::border())
                .child(
                    div()
                        .w(relative((window.used_percent / 100.) as f32))
                        .h_full()
                        .rounded_full()
                        .bg(fill),
                ),
        )
        .child(
            div()
                .w(rems(30. / 16.))
                .flex_none()
                .text_align(TextAlign::Right)
                .text_size(theme::text_meta())
                .text_color(fill)
                .child(format!("{:.0}%", window.used_percent)),
        )
        .child(
            div()
                .w(rems(84. / 16.))
                .flex_none()
                .text_align(TextAlign::Right)
                .text_size(theme::text_meta())
                .text_color(theme::muted())
                .child(reset_label(window.resets_at, now)),
        )
        .into_any_element()
}

pub(super) fn usage_section(
    runtime: Runtime,
    usage: Option<&AgentUsage>,
    reason: Option<UnavailableReason>,
    refreshing: bool,
    now: chrono::DateTime<chrono::Utc>,
) -> AnyElement {
    usage_section_with_account(runtime, usage, reason, refreshing, now, None)
}

pub(super) fn usage_section_with_account(
    runtime: Runtime,
    usage: Option<&AgentUsage>,
    reason: Option<UnavailableReason>,
    refreshing: bool,
    now: chrono::DateTime<chrono::Utc>,
    account_picker: Option<AnyElement>,
) -> AnyElement {
    let ui = crate::runtime_ui::runtime_ui(runtime);
    let name = ui.usage_title;
    let refresh_spinner_id = ui.usage_refresh_spinner;
    #[cfg(test)]
    crate::catalog_golden::record(
        serde_json::json!({"runtime":runtime,"title":name,"refresh_spinner":refresh_spinner_id}),
    );
    let icon = ChatIcon::for_runtime(runtime.key());
    let mut rows = Vec::new();
    if let Some(usage) = usage {
        for window in &usage.windows {
            let name = usage_window_name(&window.name);
            #[cfg(test)]
            crate::catalog_golden::record(serde_json::json!({"window":window.name,"label":name}));
            rows.push(usage_window_row(runtime, window, &name, now));
        }
    }
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_2()
        .py_3()
        .when(cfg!(test), |section| {
            section.debug_selector(move || format!("USAGE_SECTION_{}", runtime.key()))
        })
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .gap_2()
                .child(icon.render(px(16.), icon.color(theme::text(), true), true))
                .child(
                    div()
                        .text_size(theme::text_title())
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(name),
                )
                .child(div().flex_1())
                .children(account_picker)
                .children(
                    (refreshing && usage.is_some())
                        .then(|| spinner(refresh_spinner_id, 12., theme::muted())),
                ),
        )
        .children(rows)
        .children(
            (usage.is_none() && reason.is_none() && refreshing).then(|| {
                let id = ui.usage_checking_spinner;
                #[cfg(test)]
                crate::catalog_golden::record(serde_json::json!({"checking_spinner":id}));
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(theme::text_body())
                    .text_color(theme::muted())
                    .child(spinner(id, 12., theme::muted()))
                    .child(crate::i18n::text("ui.checking"))
            }),
        )
        .children(
            (usage.is_none() && (reason.is_some() || !refreshing)).then(|| {
                div()
                    .text_size(theme::text_body())
                    .text_color(theme::muted())
                    .child(unavailable_line(reason, runtime))
            }),
        )
        .into_any_element()
}

pub(super) fn usage_popover_panel(
    max_height: Pixels,
    header: AnyElement,
    sections: Vec<AnyElement>,
) -> AnyElement {
    div()
        .w_full()
        .max_h(max_height)
        .px_4()
        .pt_3()
        .pb_2()
        .flex()
        .flex_col()
        .rounded(px(8.))
        .border_1()
        .border_color(theme::border_strong())
        .bg(theme::panel())
        .shadow(vec![BoxShadow {
            color: gpui::black().opacity(0.25),
            blur_radius: px(16.),
            spread_radius: px(0.),
            inset: false,
            offset: point(px(0.), px(4.)),
        }])
        .when(cfg!(test), |panel| {
            #[cfg(test)]
            return crate::theme_snapshot::record_fill("USAGE_POPOVER_PANEL", panel);
            #[cfg(not(test))]
            panel
        })
        .child(div().flex_none().child(header))
        .child(
            div()
                .id("usage-popover-body")
                .min_h(px(0.))
                .flex_shrink(1.)
                .overflow_y_scroll()
                .scrollbar_width(px(0.))
                .when(cfg!(test), |body| {
                    body.debug_selector(|| "USAGE_POPOVER_BODY".into())
                })
                .children(sections),
        )
        .into_any_element()
}

pub(crate) fn alpha(mut color: gpui::Hsla, value: f32) -> gpui::Hsla {
    color.a = value;
    color
}

impl NativeRoot {
    pub(super) fn render_claude_usage_picker(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        use runner_app::ui::select::{SelectLeading, SelectOption, StyledSelect};
        let store = self.app_store.read(cx);
        if !store.claude_accounts.multiple() {
            return None;
        }
        let accounts = store.claude_accounts.clone();
        let mut options = accounts
            .accounts
            .iter()
            .filter(|account| {
                accounts
                    .statuses
                    .get(&account.id)
                    .is_some_and(|status| status.logged_in)
            })
            .map(|account| {
                let weekly = store
                    .claude_account_usage
                    .get(&account.id)
                    .and_then(|usage| usage.value.as_ref())
                    .and_then(|usage| usage.windows.iter().find(|window| window.name == "Week"))
                    .map(|window| format!("{:.0}%", window.used_percent))
                    .unwrap_or_else(|| "—".into());
                SelectOption::new(account.id.clone(), account.name.clone()).description(weekly)
            })
            .collect::<Vec<_>>();
        options.push(
            SelectOption::new("manage", crate::i18n::message("accounts.manage"))
                .leading(SelectLeading::new("manage-accounts", |size| {
                    svg()
                        .path("settings.svg")
                        .size(rems(size / 16.))
                        .text_color(theme::muted())
                        .into_any_element()
                }))
                .separator_before(true),
        );
        let owner = cx.weak_entity();
        let picker = self.claude_account_picker.get_or_insert_with(|| {
            cx.new(|cx| {
                StyledSelect::new(
                    "claude-usage-account",
                    cx.focus_handle(),
                    accounts.active_id.clone(),
                    Vec::new(),
                    Rc::new(move |id, window, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            if id == "manage" {
                                this.usage_open = false;
                                this.open_this_computer_settings(Some("agents"), window, cx);
                                this.open_claude_accounts(cx);
                            } else {
                                let client = this.core(cx).clone();
                                let store = this.app_store.clone();
                                let task = cx.background_spawn(async move {
                                    client.claude_account_action(
                                        runner_core::protocol::ClaudeAccountAction::Use { id },
                                    )
                                });
                                cx.spawn(async move |_, cx| {
                                    let result = task.await;
                                    store.update(cx, |store, cx| {
                                        if let Err(error) = result {
                                            store.error = Some(crate::surfaces::settings::claude_accounts::account_error(&error.to_string()).into());
                                        }
                                        store.refresh(
                                            crate::app_store::StoreRefreshKind::Activity,
                                            cx,
                                        );
                                    });
                                })
                                .detach();
                            }
                            cx.notify();
                        });
                    }),
                    cx,
                )
                .compact(true)
                .width(px(120.))
                .min_menu_width(px(208.))
            })
        });
        picker.update(cx, |picker, cx| {
            if picker.value() != accounts.active_id {
                picker.set_value(accounts.active_id.clone(), cx);
            }
            if let Some(active) = accounts.active() {
                if picker.placeholder_text() != active.name {
                    picker.set_placeholder(active.name.clone(), cx);
                }
            }
            picker.set_options(options, cx);
        });
        Some(picker.clone().into_any_element())
    }

    pub(super) fn render_usage_popover(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let snapshot = self.app_store.read(cx).usage.clone();
        let now = chrono::Utc::now();
        let updated = snapshot
            .runtimes
            .values()
            .filter_map(|entry| entry.value.as_ref())
            .map(|usage| usage.updated_at)
            .max();
        let age = if snapshot.refreshing {
            crate::i18n::text("ui.updating").to_owned()
        } else if let Some(updated) = updated {
            let minutes = now.signed_duration_since(updated).num_minutes().max(0);
            if minutes == 0 {
                crate::i18n::text("ui.updated_just_now").into()
            } else {
                crate::i18n::tf(
                    "format.updated_m_ago",
                    &[("minutes", (minutes).to_string())],
                )
            }
        } else {
            crate::i18n::text("ui.not_updated_yet").into()
        };
        let refresh = div()
            .id("refresh-usage")
            .size(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_sm()
            .when(!snapshot.refreshing, |button| {
                button
                    .cursor_pointer()
                    .hover(|button| button.bg(theme::sidebar_selected()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        let core = this.core(cx).clone();
                        let _ = core.usage_refresh(RefreshReason::Button);
                        cx.notify();
                    }))
            })
            .child(if snapshot.refreshing {
                spinner("usage-refresh-spinner", 14., theme::muted())
            } else {
                svg()
                    .path("refresh-cw.svg")
                    .size(px(14.))
                    .text_color(theme::muted())
                    .into_any_element()
            });
        let update_dot = update_dot_visible(&self.agent_updates, self.settings(cx));
        let owner = cx.weak_entity();
        let agent_settings = Tooltip::new(
            "usage-agent-settings-tooltip",
            if update_dot {
                crate::i18n::text("ui.agent_settings_agent_cli_update_available")
            } else {
                crate::i18n::text("ui.agent_settings")
            },
            div()
                .id("usage-agent-settings")
                .debug_selector(|| "USAGE_AGENT_SETTINGS".into())
                .relative()
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .cursor_pointer()
                .hover(|button| button.bg(theme::sidebar_selected()))
                .on_click(move |_, window, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        this.usage_open = false;
                        this.open_this_computer_settings(Some("agents"), window, cx);
                    });
                })
                .child(
                    svg()
                        .path("settings.svg")
                        .size(px(14.))
                        .text_color(theme::muted()),
                )
                .children(update_dot.then(|| {
                    div()
                        .debug_selector(|| "USAGE_AGENT_SETTINGS_DOT".into())
                        .absolute()
                        .top(px(3.))
                        .right(px(3.))
                        .size(px(6.))
                        .rounded_full()
                        .bg(theme::accent())
                })),
        );
        let mut account_picker = self.render_claude_usage_picker(cx);
        let sections: Vec<AnyElement> =
            usage_popover_runtimes(&self.usage_installed, &self.settings(cx).model_runtimes())
                .into_iter()
                .map(|runtime| {
                    if runtime == Runtime::ClaudeCode && account_picker.is_some() {
                        usage_section_with_account(
                            runtime,
                            snapshot.get(runtime),
                            snapshot.error(runtime),
                            snapshot.refreshing,
                            now,
                            account_picker.take(),
                        )
                    } else {
                        usage_section(
                            runtime,
                            snapshot.get(runtime),
                            snapshot.error(runtime),
                            snapshot.refreshing,
                            now,
                        )
                    }
                })
                .collect();
        let header = div()
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_size(theme::text_lead())
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(crate::i18n::text("ui.usage")),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_size(theme::text_meta())
                    .text_color(theme::muted())
                    .child(age),
            )
            .child(refresh)
            .child(agent_settings)
            .into_any_element();
        usage_popover_panel(window.bounds().size.height - px(16.), header, sections)
    }
}
