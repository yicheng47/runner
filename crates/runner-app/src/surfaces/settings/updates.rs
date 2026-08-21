use std::time::{Duration, SystemTime};

use chrono::{DateTime, Local};
use gpui::prelude::*;
use gpui::{div, img, rems, Context, Entity, FontWeight, Render, Subscription, Window};
use runner_app::ui::{Button, ButtonSize, PaneHeader, SettingsCard, SettingsRow, Toggle};
use runner_app::updater::Updater;

use crate::app_store::AppStore;
use crate::assets::app_icon_source;
use crate::theme;

pub(crate) struct UpdatesPane {
    app_store: Entity<AppStore>,
    updater: Entity<Updater>,
    last_seen_check_at: Option<SystemTime>,
    check_refresh_generation: u64,
    poll_generation: u64,
    poll_active: bool,
    _subscriptions: Vec<Subscription>,
}

impl UpdatesPane {
    pub(crate) fn new(
        app_store: Entity<AppStore>,
        updater: Entity<Updater>,
        cx: &mut Context<Self>,
    ) -> Self {
        let updater_subscription = cx.observe(&updater, |_, _, cx| cx.notify());
        let settings_subscription = cx.observe(&app_store, |_, _, cx| cx.notify());
        let last_seen_check_at = updater.read(cx).last_check_at();
        Self {
            app_store,
            updater,
            last_seen_check_at,
            check_refresh_generation: 0,
            poll_generation: 0,
            poll_active: false,
            _subscriptions: vec![updater_subscription, settings_subscription],
        }
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if !self.poll_active {
            self.poll_active = true;
            self.poll_generation = self.poll_generation.wrapping_add(1);
            let generation = self.poll_generation;
            cx.spawn(async move |weak, cx| loop {
                cx.background_executor()
                    .timer(Duration::from_secs(30))
                    .await;
                let keep_polling = weak
                    .update(cx, |this, cx| {
                        if !this.poll_active || this.poll_generation != generation {
                            return false;
                        }
                        let checked_at = this.updater.read(cx).last_check_at();
                        if checked_at != this.last_seen_check_at {
                            this.last_seen_check_at = checked_at;
                            cx.notify();
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep_polling {
                    break;
                }
            })
            .detach();
        }
        cx.notify();
    }

    pub(crate) fn pause(&mut self) {
        self.poll_active = false;
        self.poll_generation = self.poll_generation.wrapping_add(1);
    }

    fn check_for_updates(&mut self, cx: &mut Context<Self>) {
        let previous = self.updater.read(cx).last_check_at();
        self.updater
            .update(cx, |updater, _| updater.check_for_updates());
        self.check_refresh_generation = self.check_refresh_generation.wrapping_add(1);
        let generation = self.check_refresh_generation;
        cx.spawn(async move |weak, cx| {
            for _ in 0..240 {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                let done = weak
                    .update(cx, |this, cx| {
                        if this.check_refresh_generation != generation {
                            return true;
                        }
                        let checked_at = this.updater.read(cx).last_check_at();
                        let changed = checked_at != previous;
                        this.last_seen_check_at = checked_at;
                        cx.notify();
                        changed
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
        })
        .detach();
    }
}

impl Render for UpdatesPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (updater_available, automatically_checks, last_checked) = {
            let updater = self.updater.read(cx);
            let updater_available = updater.is_available();
            let automatically_checks = if updater_available {
                updater.automatically_checks_for_updates()
            } else {
                self.app_store
                    .read(cx)
                    .settings
                    .automatically_check_for_updates
            };
            (
                updater_available,
                automatically_checks,
                format_last_checked(updater.last_check_at()),
            )
        };

        let pane = cx.entity();
        let toggle_updater = self.updater.clone();
        let toggle_store = self.app_store.clone();

        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(PaneHeader::new(
                "Updates",
                "Check for new versions and choose how Runner checks.",
            ))
            .child(
                div()
                    .overflow_hidden()
                    .rounded(rems(12. / 16.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_4()
                            .p_5()
                            .child(
                                img(app_icon_source())
                                    .size(rems(56. / 16.))
                                    .flex_none()
                                    .rounded(rems(1.)),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .text_size(rems(1.))
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(theme::text())
                                                    .child("Runner"),
                                            )
                                            .child(
                                                div()
                                                    .rounded_sm()
                                                    .bg(theme::raised())
                                                    .px(rems(6. / 16.))
                                                    .py(rems(2. / 16.))
                                                    .font_family("Menlo")
                                                    .text_size(rems(11. / 16.))
                                                    .text_color(theme::muted())
                                                    .child(format!(
                                                        "v{}",
                                                        runner_app::version::display_version()
                                                    )),
                                            ),
                                    )
                                    .children((!updater_available).then(|| {
                                        div()
                                            .text_size(rems(12. / 16.))
                                            .text_color(theme::muted())
                                            .child(
                                                "Update controls are available in bundled builds.",
                                            )
                                    })),
                            )
                            .child(
                                Button::new("updates-check", "Check for updates")
                                    .icon("refresh-cw.svg")
                                    .size(ButtonSize::Sm)
                                    .disabled(!updater_available)
                                    .on_press(move |_, cx| {
                                        pane.update(cx, |pane, cx| pane.check_for_updates(cx));
                                    }),
                            ),
                    ),
            )
            .child(SettingsCard::new([
                SettingsRow::new(
                    "Automatically check for updates",
                    Toggle::new("updates-auto-check", automatically_checks)
                        .disabled(!updater_available)
                        .on_change(move |enabled, _, cx| {
                            toggle_store.update(cx, |store, store_cx| {
                                store.update_settings(
                                    |settings| {
                                        if settings.automatically_check_for_updates == enabled {
                                            return false;
                                        }
                                        settings.automatically_check_for_updates = enabled;
                                        true
                                    },
                                    true,
                                    store_cx,
                                );
                            });
                            toggle_updater.update(cx, |updater, updater_cx| {
                                updater.set_automatically_checks_for_updates(enabled, updater_cx)
                            });
                        }),
                )
                .subtitle("Let Sparkle check GitHub for a newer signed release when due.")
                .into_any_element(),
                SettingsRow::new(
                    "Last checked",
                    div()
                        .text_size(rems(12. / 16.))
                        .text_color(theme::muted())
                        .child(last_checked),
                )
                .subtitle("Sparkle’s most recent update check.")
                .into_any_element(),
            ]))
    }
}

fn format_last_checked(checked_at: Option<SystemTime>) -> String {
    checked_at
        .map(DateTime::<Local>::from)
        .map(|checked_at| checked_at.format("%b %-d, %Y at %-I:%M %p").to_string())
        .unwrap_or_else(|| "Never".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_sparkle_check_date_is_never() {
        assert_eq!(format_last_checked(None), "Never");
    }
}
