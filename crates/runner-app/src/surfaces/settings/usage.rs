use crate::app_settings::UsageSettings;
use crate::surfaces::app_shell::{
    is_five_hour, sidebar_usage_element, usage_section_with_account, visible_usage_runtimes,
    SidebarFooterControl,
};
use crate::{theme, NativeRoot};
use gpui::prelude::*;
use gpui::{div, px, AnyElement, Context, FontWeight};
use runner_app::ui::{Button, ButtonSize, PaneHeader, SettingsCard, SettingsRow, Toggle};
use runner_core::Runtime;

pub(super) const USAGE_PREVIEW_WIDTH: f32 = 240.;
pub(super) const USAGE_PREVIEW_PADDING: f32 = 20.;

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
        let visible: Vec<_> =
            visible_usage_runtimes(&self.usage_installed, &self.settings(cx).model_runtimes())
                .into_iter()
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
                Runtime::Antigravity => settings.antigravity,
                _ => false,
            };
            let row = SettingsRow::new(
                runtime.display_name(),
                Toggle::new(format!("usage-badge-{}", runtime.key()), enabled)
                    .disabled(!settings.sidebar)
                    .on_change(move |value, _, cx| {
                        owner.update(cx, |root, cx| {
                            root.update_app_settings(cx, true, |s| {
                                match runtime {
                                    Runtime::Codex => s.usage.codex = value,
                                    Runtime::ClaudeCode => s.usage.claude_code = value,
                                    Runtime::Antigravity => s.usage.antigravity = value,
                                    _ => {}
                                }
                                true
                            });
                        })
                    }),
            )
            .when_some(
                match runtime {
                    Runtime::ClaudeCode => Some(crate::i18n::text("usage.active_account")),
                    Runtime::Antigravity => {
                        Some(crate::i18n::text("usage.antigravity_gemini_pool"))
                    }
                    _ => None,
                },
                SettingsRow::subtitle,
            );
            #[cfg(test)]
            let row = row.debug_prefix(format!("USAGE_BADGE_{}", runtime.key()));
            sidebar.push(
                div()
                    .when(cfg!(test), |row| {
                        row.debug_selector(move || format!("USAGE_BADGE_ROW_{}", runtime.key()))
                    })
                    .opacity(if settings.sidebar { 1. } else { 0.45 })
                    .child(row)
                    .into_any_element(),
            );
        }
        let zoom = self.settings(cx).app_zoom;
        let preview_available = (USAGE_PREVIEW_WIDTH - 96.) * zoom;
        let preview_usage = (!visible.is_empty()).then(|| {
            sidebar_usage_element(&snapshot, &visible, preview_available, zoom)
                .max_w(px(preview_available))
                .into_any_element()
        });
        let preview = div()
            .debug_selector(|| "USAGE_SIDEBAR_PREVIEW".into())
            .w_full()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(24. * zoom))
            .px(px(USAGE_PREVIEW_PADDING * zoom))
            .py(px(16. * zoom))
            .child(
                div()
                    .debug_selector(|| "USAGE_PREVIEW_LABEL".into())
                    .flex_1()
                    .flex_basis(gpui::auto())
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .gap(px(3. * zoom))
                    .child(
                        div()
                            .text_size(px(13. * zoom))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::text())
                            .child(crate::i18n::text("usage.sidebar_preview")),
                    )
                    .child(
                        div()
                            .text_size(px(12. * zoom))
                            .text_color(theme::muted())
                            .child(crate::i18n::text("usage.weekly_usage")),
                    ),
            )
            .child(
                div()
                    .debug_selector(|| "USAGE_PREVIEW_SIDEBAR".into())
                    .w(px(USAGE_PREVIEW_WIDTH * zoom))
                    .flex_none()
                    .p(px(12. * zoom))
                    .rounded(px(8. * zoom))
                    .bg(theme::sidebar())
                    .child(crate::surfaces::app_shell::sidebar_footer_contents(
                        preview_usage,
                        SidebarFooterControl::Host
                            .render(zoom, theme::muted())
                            .debug_selector(|| "USAGE_PREVIEW_HOST".into())
                            .into_any_element(),
                        SidebarFooterControl::Settings
                            .render(zoom, theme::muted())
                            .debug_selector(|| "USAGE_PREVIEW_SETTINGS".into())
                            .into_any_element(),
                        zoom,
                    )),
            );
        sidebar.insert(0, preview.into_any_element());
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
