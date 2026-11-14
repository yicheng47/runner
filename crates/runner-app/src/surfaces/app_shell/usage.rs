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

/// The Agents settings link carries a dot while an enabled agent has an update.
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
        Some(UnavailableReason::SignIn) => match runtime {
            Runtime::Antigravity => crate::i18n::text("ui.sign_in_to_antigravity_cli_to_see_usage"),
            Runtime::Muse => crate::i18n::text("ui.sign_in_to_muse_to_see_usage"),
            _ => crate::i18n::text("ui.sign_in_to_claude_code_to_see_usage"),
        },
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
        Some(UnavailableReason::AntigravityNoAnswer) => {
            crate::i18n::text("ui.antigravity_cli_didn_t_answer")
        }
        Some(UnavailableReason::InvalidResponse) => {
            crate::runtime_ui::runtime_ui(runtime).usage_invalid
        }
        None => crate::i18n::text("ui.checking"),
    }
}

pub(super) fn weekly_usage_percent(runtime: Runtime, usage: Option<&AgentUsage>) -> Option<f64> {
    if crate::runtime_ui::runtime_ui(runtime).usage_groups {
        return usage?
            .windows
            .iter()
            .filter(|window| is_weekly_pool_window(&window.name))
            .map(|window| window.used_percent)
            .reduce(f64::max);
    }
    let name = crate::runtime_ui::runtime_ui(runtime).usage_week;
    usage?
        .windows
        .iter()
        .find(|window| window.name == name)
        .map(|window| window.used_percent)
}

fn is_weekly_pool_window(name: &str) -> bool {
    let label = name.split_once(" · ").map_or(name, |(_, label)| label);
    let label = label.strip_suffix(" used").unwrap_or(label);
    label == "Week"
}

/// The runtimes the usage footer and popover show: usage-capable, installed and
/// enabled in Settings, whether or not they have run a session (#752), in
/// `Runtime::ALL` order.
pub(crate) fn visible_usage_runtimes(installed: &[Runtime], enabled: &[Runtime]) -> Vec<Runtime> {
    Runtime::ALL
        .into_iter()
        .filter(|runtime| {
            crate::runtime_ui::catalog_capabilities(&[], runtime.key()).usage
                && installed.contains(runtime)
                && enabled.contains(runtime)
        })
        .collect()
}

pub(crate) fn usage_popover_runtimes(installed: &[Runtime], enabled: &[Runtime]) -> Vec<Runtime> {
    let mut runtimes = visible_usage_runtimes(installed, enabled);
    runtimes.sort_by_key(|runtime| crate::runtime_ui::runtime_ui(*runtime).usage_order);
    runtimes
}

fn estimated_badge_text_width(text: &str) -> f32 {
    text.chars()
        .map(|ch| match ch {
            '0'..='9' => 7.0,
            '%' | '—' => 10.0,
            '+' => 7.2,
            _ => 7.0,
        })
        .sum()
}

fn estimated_entry_width(text: &str) -> f32 {
    // 12 px runtime mark + 2 px icon-to-% gap + value text
    14.0 + estimated_badge_text_width(text)
}

pub(super) fn select_sidebar_usage_entries(
    entries: &[(Runtime, Option<f64>, String)],
    max_width: f32,
    zoom: f32,
) -> (Vec<usize>, usize) {
    const PILL_PAD_X: f32 = 12.0; // 6 px left + 6 px right
    const BADGE_GAP: f32 = 8.0;

    if entries.is_empty() {
        return (Vec::new(), 0);
    }

    let budget = if zoom > 0.0 {
        (max_width / zoom).max(0.0)
    } else {
        0.0
    };

    let all_width = PILL_PAD_X
        + entries
            .iter()
            .map(|(_, _, value)| estimated_entry_width(value))
            .sum::<f32>()
        + BADGE_GAP * entries.len().saturating_sub(1) as f32;
    if all_width <= budget {
        return ((0..entries.len()).collect(), 0);
    }

    let mut ranked: Vec<usize> = (0..entries.len()).collect();
    ranked.sort_by(|&a, &b| {
        let pa = entries[a].1;
        let pb = entries[b].1;
        match (pa, pb) {
            (Some(a_pct), Some(b_pct)) => b_pct.total_cmp(&a_pct).then_with(|| a.cmp(&b)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.cmp(&b),
        }
    });

    let mut kept_count = 1.min(ranked.len());
    for count in (1..entries.len()).rev() {
        let hidden = entries.len() - count;
        let overflow_label = format!("+{hidden}");
        let width = PILL_PAD_X
            + ranked[..count]
                .iter()
                .map(|&idx| estimated_entry_width(&entries[idx].2))
                .sum::<f32>()
            + estimated_badge_text_width(&overflow_label)
            + BADGE_GAP * count as f32;
        if width <= budget {
            kept_count = count;
            break;
        }
    }

    let hidden = entries.len().saturating_sub(kept_count);
    (ranked[..kept_count].to_vec(), hidden)
}

pub(crate) fn sidebar_usage_element(
    snapshot: &UsageSnapshot,
    visible: &[Runtime],
    max_width: f32,
    zoom: f32,
) -> gpui::Stateful<Div> {
    let raw_entries: Vec<_> = visible
        .iter()
        .map(|runtime| {
            let usage = snapshot.get(*runtime);
            let percent = weekly_usage_percent(*runtime, usage);
            let value = percent.map_or_else(|| "—".to_owned(), |value| format!("{value:.0}%"));
            (*runtime, percent, value)
        })
        .collect();
    let (kept_indices, hidden) = select_sidebar_usage_entries(&raw_entries, max_width, zoom);

    let mut children: Vec<AnyElement> = kept_indices
        .into_iter()
        .map(|index| {
            let (runtime, percent, value) = &raw_entries[index];
            let color = percent.map_or(theme::muted(), |value| usage_color(usage_tone([value])));
            let runtime = *runtime;
            div()
                .when(cfg!(test), |entry| {
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
                            value.debug_selector(move || {
                                format!("SIDEBAR_USAGE_VALUE_{}", runtime.key())
                            })
                        })
                        .text_size(px(11. * zoom))
                        .text_color(color)
                        .child(value.clone()),
                )
                .into_any_element()
        })
        .collect();

    if hidden > 0 {
        children.push(
            div()
                .when(cfg!(test), |overflow| {
                    overflow.debug_selector(|| "SIDEBAR_USAGE_OVERFLOW".into())
                })
                .flex_none()
                .text_size(px(11. * zoom))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::muted())
                .child(format!("+{hidden}"))
                .into_any_element(),
        );
    }

    div()
        .id("sidebar-usage")
        .when(cfg!(test), |usage| {
            usage.debug_selector(|| "SIDEBAR_USAGE".into())
        })
        .flex_none()
        .h(px(32. * zoom))
        .flex()
        .items_center()
        .gap(px(8. * zoom))
        .px(px(6. * zoom))
        .rounded(px(6. * zoom))
        .border_1()
        .border_color(gpui::transparent_black())
        .hover(|usage| usage.bg(theme::chrome_hover()))
        .focus_visible(|usage| usage.bg(theme::chrome_hover()))
        .map(|usage| {
            #[cfg(test)]
            let usage = crate::theme_snapshot::record_fill("SIDEBAR_USAGE", usage);
            #[cfg(test)]
            let usage = crate::theme_snapshot::record_border("SIDEBAR_USAGE", usage);
            usage
        })
        .children(children)
}

pub(super) fn sidebar_update_hint_element(max_width: f32, zoom: f32) -> gpui::Stateful<Div> {
    div()
        .id("sidebar-usage-update")
        .when(cfg!(test), |hint| {
            hint.debug_selector(|| "SIDEBAR_USAGE_UPDATE".into())
        })
        .max_w(px(max_width))
        .min_w_0()
        .overflow_hidden()
        .h(px(32. * zoom))
        .flex()
        .items_center()
        .gap(px(5. * zoom))
        .px(px(10. * zoom))
        .rounded(px(6. * zoom))
        .text_color(theme::muted())
        .child(
            svg()
                .flex_none()
                .path("circle-arrow-up.svg")
                .size(px(12. * zoom))
                .text_color(theme::muted()),
        )
        .child(
            div()
                .when(cfg!(test), |text| {
                    text.debug_selector(|| "SIDEBAR_USAGE_UPDATE_TEXT".into())
                })
                .min_w_0()
                .truncate()
                .text_size(px(11. * zoom))
                .child(crate::i18n::text("client.update_for_usage")),
        )
}

pub(super) fn older_host_popover(
    max_height: Pixels,
    computer: String,
    version: String,
) -> AnyElement {
    let lead = if version.is_empty() {
        crate::i18n::tf(
            "client.older_host_unknown",
            &[("computer", computer.clone())],
        )
    } else {
        crate::i18n::tf(
            "client.older_host_version",
            &[("computer", computer.clone()), ("version", version)],
        )
    };
    let header = div()
        .min_w_0()
        .truncate()
        .text_size(theme::text_lead())
        .font_weight(FontWeight::SEMIBOLD)
        .child(crate::i18n::tf(
            "client.usage_on",
            &[("computer", computer.clone())],
        ))
        .into_any_element();
    let body = div()
        .w_full()
        .flex()
        .flex_col()
        .gap_1()
        .py_3()
        .when(cfg!(test), |body| {
            body.debug_selector(|| "USAGE_OLDER_HOST".into())
        })
        .child(
            div()
                .text_size(theme::text_title())
                .font_weight(FontWeight::SEMIBOLD)
                .child(lead),
        )
        .child(
            div()
                .w_full()
                .min_w_0()
                .whitespace_normal()
                .text_size(theme::text_body())
                .text_color(theme::muted())
                .child(crate::i18n::tf(
                    "client.older_host_body",
                    &[("computer", computer)],
                )),
        )
        .into_any_element();
    usage_popover_panel(max_height, header, vec![body])
}

pub(crate) fn is_five_hour(name: &str) -> bool {
    let label = name.split_once(" · ").map_or(name, |(_, label)| label);
    let label = label.strip_suffix(" used").unwrap_or(label);
    label == "5 hours" || label == "5h" || label == "300 min"
}

pub(super) fn usage_group_label(group: &str) -> &str {
    match group {
        "Gemini Models" => crate::i18n::text("usage.gemini_models"),
        "Claude and GPT models" => crate::i18n::text("usage.claude_gpt_models"),
        _ => group,
    }
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

pub(crate) fn usage_window_name(name: &str) -> String {
    let (group, label) = name
        .split_once(" · ")
        .map_or((None, name), |(group, label)| (Some(group), label));
    let label = if let Some(label) = label.strip_suffix(" used") {
        crate::i18n::tf("usage.used", &[("window", usage_window_label(label))])
    } else {
        usage_window_label(label)
    };
    group.map_or_else(
        || label.clone(),
        |group| format!("{} · {label}", usage_group_label(group)),
    )
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

fn grouped_credit_balance(balance: &str) -> Option<String> {
    let balance = balance.trim();
    let (mantissa, exponent) = balance
        .split_once(['e', 'E'])
        .map_or((balance, None), |(amount, exponent)| {
            (amount, Some(exponent))
        });
    let digits = |value: &str| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
    let exponent = exponent.map_or(Some(0), |value| value.parse::<isize>().ok())?;
    let unsigned = mantissa.strip_prefix(['+', '-']).unwrap_or(mantissa);
    let sign = &mantissa[..mantissa.len() - unsigned.len()];
    let (whole, fraction) = unsigned
        .split_once('.')
        .map_or((unsigned, None), |(whole, fraction)| {
            (whole, Some(fraction))
        });
    if !digits(whole) || fraction.is_some_and(|value| !digits(value)) {
        return None;
    }
    let coefficient = format!("{whole}{}", fraction.unwrap_or_default());
    let point = isize::try_from(whole.len()).ok()?.checked_add(exponent)?;
    let mut rounded = if point > 0 {
        let point = point as usize;
        let mut rounded = coefficient.as_bytes()[..point.min(coefficient.len())].to_vec();
        rounded.resize(point, b'0');
        rounded
    } else {
        vec![b'0']
    };
    if point >= 0
        && coefficient
            .as_bytes()
            .get(point as usize)
            .is_some_and(|digit| *digit >= b'5')
    {
        let mut carry = true;
        for digit in rounded.iter_mut().rev() {
            if *digit < b'9' {
                *digit += 1;
                carry = false;
                break;
            }
            *digit = b'0';
        }
        if carry {
            rounded.insert(0, b'1');
        }
    }
    let first = rounded.iter().position(|digit| *digit != b'0');
    let rounded = &rounded[first.unwrap_or(rounded.len() - 1)..];
    let mut grouped = if first.is_some() {
        sign.to_owned()
    } else {
        String::new()
    };
    for (index, digit) in rounded.iter().enumerate() {
        if index > 0 && (rounded.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(char::from(*digit));
    }
    Some(grouped)
}

pub(super) fn credit_value(
    credits: Option<&runner_core::protocol::UsageCredits>,
) -> (String, bool) {
    if let Some(credits) = credits {
        if credits.unlimited {
            return (crate::i18n::text("usage.unlimited").into(), true);
        }
        if let Some(balance) = credits.balance.as_deref() {
            if let Some(balance) = grouped_credit_balance(balance) {
                return (balance, true);
            }
        } else if !credits.has_credits {
            return ("0".into(), true);
        }
    }
    (crate::i18n::text("usage.unavailable").into(), false)
}

fn credit_row(usage: &AgentUsage) -> AnyElement {
    let (value, available) = credit_value(usage.credits.as_ref());
    div()
        .when(cfg!(test), |row| {
            row.debug_selector(|| "USAGE_CREDITS".into())
        })
        .w_full()
        .flex()
        .items_center()
        .gap_2()
        .border_t_1()
        .border_color(theme::border())
        .pt_2()
        .text_size(theme::text_meta())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(theme::muted())
                .child(crate::i18n::text("usage.credits_remaining")),
        )
        .child(
            div()
                .min_w_0()
                .text_align(TextAlign::Right)
                .text_color(if available {
                    theme::text()
                } else {
                    theme::muted()
                })
                .when(available, |value| value.font_weight(FontWeight::SEMIBOLD))
                .child(value),
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

pub(crate) fn usage_section_with_account(
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
    let mut previous_group = None;
    if let Some(usage) = usage {
        for window in &usage.windows {
            let (group, label) = if ui.usage_groups {
                window
                    .name
                    .split_once(" · ")
                    .map_or((None, window.name.as_str()), |(group, label)| {
                        (Some(group), label.strip_suffix(" used").unwrap_or(label))
                    })
            } else {
                (None, window.name.as_str())
            };
            let display_label = if ui.usage_groups {
                usage_window_label(label)
            } else {
                usage_window_name(label)
            };
            #[cfg(test)]
            crate::catalog_golden::record(
                serde_json::json!({"window":window.name,"group":group,"label":label}),
            );
            if let Some(group) = group.filter(|group| Some(*group) != previous_group) {
                rows.push(
                    div()
                        .when(previous_group.is_some(), |heading| heading.pt(px(2.)))
                        .text_size(theme::text_meta())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::muted())
                        .child(usage_group_label(group).to_owned())
                        .into_any_element(),
                );
            }
            previous_group = group;
            rows.push(usage_window_row(runtime, window, &display_label, now));
        }
        if runtime == Runtime::Codex {
            rows.push(credit_row(usage));
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
                if runtime == Runtime::Antigravity && reason == Some(UnavailableReason::SignIn) {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(theme::text_body())
                                .text_color(theme::muted())
                                .child(unavailable_line(reason, runtime)),
                        )
                        .child(
                            div()
                                .text_size(theme::text_meta())
                                .text_color(theme::faint())
                                .child(crate::i18n::text(
                                    "ui.run_agy_login_in_a_terminal_runner_won_t_open_login_automatically",
                                )),
                        )
                } else {
                    div()
                        .text_size(theme::text_body())
                        .text_color(theme::muted())
                        .child(unavailable_line(reason, runtime))
                }
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
    pub(crate) fn start_usage_refresh_timer(&mut self, cx: &mut Context<Self>) {
        self.usage_refresh_task = Some(cx.spawn(async move |weak, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(5 * 60))
                .await;
            let keep = weak
                .update(cx, |root, cx| {
                    if !root.usage_open || root.remote_client.is_active() {
                        return false;
                    }
                    let _ = root.core(cx).usage_refresh(RefreshReason::Open);
                    true
                })
                .unwrap_or(false);
            if !keep {
                break;
            }
        }));
    }

    pub(crate) fn sidebar_usage(&self, cx: &App) -> (UsageSnapshot, Vec<Runtime>, bool) {
        let remote = self.remote_client.is_active();
        let remote_usage = self.remote_client.remote_usage(cx);
        if remote {
            remote_usage.map_or_else(
                || (UsageSnapshot::default(), Vec::new(), false),
                |(usage, connection, _)| {
                    let snapshot = crate::surfaces::remote_client::usage_snapshot(&usage);
                    let agents: Vec<_> = snapshot.runtimes.keys().copied().collect();
                    let visible = visible_usage_runtimes(&agents, &agents);
                    (
                        snapshot,
                        visible,
                        connection != crate::surfaces::remote_client::model::Connection::Connected,
                    )
                },
            )
        } else {
            (
                self.app_store.read(cx).usage.clone(),
                visible_usage_runtimes(&self.usage_installed, &self.settings(cx).usage_runtimes())
                    .into_iter()
                    .filter(|runtime| self.settings(cx).usage.badge(*runtime))
                    .collect(),
                false,
            )
        }
    }

    pub(crate) fn render_claude_usage_picker(
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
        if let Some((computer, version)) = self.remote_client.older_host(cx) {
            return older_host_popover(window.bounds().size.height - px(16.), computer, version);
        }
        let remote = self.remote_client.remote_usage(cx);
        let readonly = self.remote_client.is_active();
        let mut snapshot = if readonly {
            remote
                .as_ref()
                .map(|(usage, _, _)| crate::surfaces::remote_client::usage_snapshot(usage))
                .unwrap_or_default()
        } else {
            self.app_store.read(cx).usage.clone()
        };
        if !self.settings(cx).usage.five_hour {
            for entry in snapshot.runtimes.values_mut() {
                if let Some(usage) = &mut entry.value {
                    usage.windows.retain(|window| !is_five_hour(&window.name));
                }
            }
        }
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
            .debug_selector(|| "USAGE_REFRESH".into())
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
        let owner = cx.weak_entity();
        let agent_settings = Tooltip::new(
            "usage-agent-settings-tooltip",
            crate::i18n::text("usage.settings"),
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
                        this.open_this_computer_settings(Some("usage"), window, cx);
                    });
                })
                .child(
                    svg()
                        .path("settings.svg")
                        .size(px(14.))
                        .text_color(theme::muted()),
                ),
        );
        let mut account_picker = if readonly {
            remote
                .as_ref()
                .and_then(|(usage, _, _)| usage.active_claude_account.as_ref())
                .map(|name| {
                    div()
                        .debug_selector(|| "USAGE_REMOTE_ACCOUNT".into())
                        .max_w(rems(120. / 16.))
                        .truncate()
                        .text_size(theme::text_meta())
                        .text_color(theme::muted())
                        .child(name.clone())
                        .into_any_element()
                })
        } else {
            self.render_claude_usage_picker(cx)
        };
        let runtimes = if readonly {
            let agents: Vec<_> = snapshot.runtimes.keys().copied().collect();
            usage_popover_runtimes(&agents, &agents)
        } else {
            usage_popover_runtimes(&self.usage_installed, &self.settings(cx).usage_runtimes())
        };
        let mut sections: Vec<AnyElement> = runtimes
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
        if !readonly {
            sections.extend(self.render_mission_usage_link(cx));
        }
        let header = div()
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(theme::text_lead())
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(remote.as_ref().map_or_else(
                        || crate::i18n::text("ui.usage").to_owned(),
                        |(_, _, name)| {
                            crate::i18n::tf("client.usage_on", &[("computer", name.clone())])
                        },
                    )),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex_none()
                    .text_size(theme::text_meta())
                    .text_color(theme::muted())
                    .child(age),
            )
            .when(!readonly, |header| {
                header.child(refresh).child(agent_settings)
            })
            .into_any_element();
        usage_popover_panel(window.bounds().size.height - px(16.), header, sections)
    }
}
