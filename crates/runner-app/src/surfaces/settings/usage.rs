use crate::app_settings::UsageSettings;
use crate::surfaces::app_shell::{is_five_hour, sidebar_usage_element, usage_section_with_account};
use crate::{theme, NativeRoot};
use gpui::prelude::*;
use gpui::{div, px, svg, AnyElement, Context};
use runner_app::ui::{Button, ButtonSize, PaneHeader, SettingsCard, SettingsRow, Toggle};
use runner_core::Runtime;

impl NativeRoot {
    pub(crate) fn render_usage_settings(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.settings(cx).usage.clone();
        let mut snapshot = self.app_store.read(cx).usage.clone();
        if !settings.five_hour {
            for entry in snapshot.runtimes.values_mut() {
                if let Some(value) = &mut entry.value {
                    value.windows.retain(|window| !is_five_hour(&window.name));
                }
            }
        }
        let runtimes = crate::surfaces::app_shell::usage_popover_runtimes(
            &self.usage_installed,
            &self.settings(cx).model_runtimes(),
        );
        let visible: Vec<_> = runtimes
            .iter()
            .copied()
            .filter(|r| settings.badge(*r))
            .collect();
        let now = chrono::Utc::now();
        let updated = snapshot
            .runtimes
            .values()
            .filter_map(|entry| entry.value.as_ref())
            .map(|usage| usage.updated_at)
            .max();
        let age = updated.map_or_else(
            || crate::i18n::text("ui.not_updated_yet").to_owned(),
            |updated| {
                crate::i18n::tf(
                    "format.updated_m_ago",
                    &[(
                        "minutes",
                        now.signed_duration_since(updated)
                            .num_minutes()
                            .max(0)
                            .to_string(),
                    )],
                )
            },
        );
        let owner = cx.entity();
        let refresh = Button::new("usage-settings-refresh", crate::i18n::text("ui.refresh"))
            .icon("refresh-cw.svg")
            .size(ButtonSize::Sm)
            .disabled(snapshot.refreshing)
            .on_press(move |_, cx| {
                owner.update(cx, |root, cx| {
                    let _ = root
                        .core(cx)
                        .usage_refresh(runner_core::protocol::RefreshReason::Button);
                })
            });
        let mut sidebar = vec![self.usage_toggle(
            "usage-sidebar",
            "usage.show_sidebar",
            "usage.sidebar_hint",
            |s| &mut s.sidebar,
            cx,
        )];
        for runtime in &runtimes {
            let runtime = *runtime;
            let owner = cx.entity();
            let enabled = match runtime {
                Runtime::Codex => settings.codex,
                Runtime::ClaudeCode => settings.claude_code,
                _ => false,
            };
            sidebar.push(
                div()
                    .opacity(if settings.sidebar { 1. } else { 0.45 })
                    .child(
                        SettingsRow::new(
                            runtime.display_name(),
                            Toggle::new(format!("usage-badge-{}", runtime.key()), enabled)
                                .disabled(!settings.sidebar)
                                .on_change(move |value, _, cx| {
                                    owner.update(cx, |root, cx| {
                                        root.update_app_settings(cx, true, |s| {
                                            match runtime {
                                                Runtime::Codex => s.usage.codex = value,
                                                Runtime::ClaudeCode => s.usage.claude_code = value,
                                                _ => {}
                                            }
                                            true
                                        });
                                    })
                                }),
                        )
                        .subtitle(if runtime == Runtime::ClaudeCode {
                            crate::i18n::text("usage.active_account")
                        } else {
                            ""
                        }),
                    )
                    .into_any_element(),
            );
        }
        let zoom = self.settings(cx).app_zoom;
        let preview_usage = (!visible.is_empty()).then(|| {
            sidebar_usage_element(&snapshot, &visible, zoom)
                .w(gpui::auto())
                .flex_none()
                .justify_start()
                .gap(px(12. * zoom))
                .border_1()
                .border_color(theme::chrome_selected_border())
                .bg(theme::chrome_selected())
                .into_any_element()
        });
        let (recent, recent_icon) = self
            .app_store
            .read(cx)
            .sessions
            .first()
            .map(|session| {
                (
                    session
                        .preferred_title(None)
                        .unwrap_or_else(|| session.display_name.clone()),
                    crate::chat_icon::ChatIcon::for_runtime(&session.agent_runtime),
                )
            })
            .unwrap_or_else(|| {
                (
                    crate::i18n::text("ui.new_chat").to_owned(),
                    crate::chat_icon::ChatIcon::for_runtime(&self.settings(cx).default_runtime),
                )
            });
        let preview = div()
            .debug_selector(|| "USAGE_SIDEBAR_PREVIEW".into())
            .w_full()
            .pt(px(28. * zoom))
            .rounded_md()
            .bg(theme::sidebar())
            .flex()
            .flex_col()
            .items_center()
            .child(
                div()
                    .debug_selector(|| "USAGE_PREVIEW_SIDEBAR".into())
                    .w(px(240. * zoom))
                    .px(px(12. * zoom))
                    .pb(px(12. * zoom))
                    .flex()
                    .flex_col()
                    .gap(px(8. * zoom))
                    .child(
                        div()
                            .debug_selector(|| "USAGE_PREVIEW_RECENT".into())
                            .w_full()
                            .h(px(30. * zoom))
                            .px(px(8. * zoom))
                            .opacity(0.4)
                            .flex()
                            .items_center()
                            .gap(px(8. * zoom))
                            .child(recent_icon.render(
                                px(12. * zoom),
                                recent_icon.color(theme::text(), true),
                                true,
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(13. * zoom))
                                    .text_color(theme::muted())
                                    .child(recent),
                            ),
                    )
                    .child(
                        crate::surfaces::app_shell::sidebar_footer_divider()
                            .debug_selector(|| "USAGE_PREVIEW_DIVIDER".into()),
                    )
                    .child(
                        crate::surfaces::app_shell::sidebar_footer_contents(
                            preview_usage,
                            preview_button("monitor.svg", 14., zoom),
                            preview_button("settings.svg", 16., zoom),
                            zoom,
                            8.,
                        )
                        .h(px(32. * zoom)),
                    ),
            );
        let mut account_picker = self.render_claude_usage_picker(cx);
        let cards: Vec<_> = runtimes
            .into_iter()
            .map(|runtime| {
                let picker = if runtime == Runtime::ClaudeCode {
                    account_picker.take()
                } else {
                    None
                };
                SettingsCard::new(vec![div()
                    .px_4()
                    .child(usage_section_with_account(
                        runtime,
                        snapshot.get(runtime),
                        snapshot.error(runtime),
                        snapshot.refreshing,
                        now,
                        picker,
                    ))
                    .into_any_element()])
                .into_any_element()
            })
            .collect();
        let interval = self
            .settings_page
            .usage_interval
            .read(cx)
            .value()
            .parse::<u64>()
            .unwrap_or(900)
            / 60;
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                PaneHeader::new(
                    crate::i18n::text("ui.usage"),
                    crate::i18n::text("usage.subtitle"),
                )
                .action(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .text_size(theme::text_meta())
                                .text_color(theme::muted())
                                .child(age),
                        )
                        .child(refresh),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(super::page::settings_section_heading(crate::i18n::text(
                        "usage.sidebar",
                    )))
                    .child(preview)
                    .child(SettingsCard::new(sidebar)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(super::page::settings_section_heading(crate::i18n::text(
                                "usage.limits",
                            )))
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_size(theme::text_meta())
                                    .text_color(theme::muted())
                                    .child(crate::i18n::text("usage.check_every")),
                            )
                            .child(self.settings_page.usage_interval.clone()),
                    )
                    .child(SettingsCard::new(vec![self.usage_toggle(
                        "usage-five-hour",
                        "usage.five_hour_limit",
                        "usage.five_hour_hint",
                        |s| &mut s.five_hour,
                        cx,
                    )]))
                    .children(cards)
                    .child(
                        div()
                            .text_size(theme::text_meta())
                            .text_color(theme::faint())
                            .child(crate::i18n::tf(
                                "usage.interval_caption",
                                &[("minutes", interval.to_string())],
                            )),
                    ),
            )
            .into_any_element()
    }

    fn usage_toggle(
        &self,
        id: &'static str,
        title: &'static str,
        subtitle: &'static str,
        field: fn(&mut UsageSettings) -> &mut bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut usage = self.settings(cx).usage.clone();
        let value = *field(&mut usage);
        let owner = cx.entity();
        div()
            .child(
                SettingsRow::new(
                    crate::i18n::text(title),
                    Toggle::new(id, value).on_change(move |value, _, cx| {
                        owner.update(cx, |root, cx| {
                            root.update_app_settings(cx, true, |s| {
                                *field(&mut s.usage) = value;
                                true
                            });
                        })
                    }),
                )
                .subtitle(crate::i18n::text(subtitle)),
            )
            .into_any_element()
    }
}

fn preview_button(icon: &'static str, icon_size: f32, zoom: f32) -> AnyElement {
    div()
        .when(cfg!(test), |button| {
            button.debug_selector(move || format!("USAGE_PREVIEW_{icon}"))
        })
        .flex_none()
        .size(px(32. * zoom))
        .flex()
        .items_center()
        .justify_center()
        .child(
            svg()
                .path(icon)
                .size(px(icon_size * zoom))
                .text_color(theme::muted()),
        )
        .into_any_element()
}
