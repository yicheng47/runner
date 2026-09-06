use chrono::{DateTime, Local};
use gpui::prelude::*;
use gpui::{div, Context, Entity, Render, Subscription, Window};
use runner_app::ui::{Button, ButtonSize, PaneHeader, SettingsCard, SettingsRow, Toggle};
use runner_app::updater::{windows_download_url, Updater};

use crate::app_store::AppStore;
use crate::theme;

pub(crate) struct UpdatesPane {
    app_store: Entity<AppStore>,
    updater: Entity<Updater>,
    _subscriptions: Vec<Subscription>,
}

impl UpdatesPane {
    pub(crate) fn new(
        app_store: Entity<AppStore>,
        updater: Entity<Updater>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&updater, |_, _, cx| cx.notify()),
            cx.observe(&app_store, |_, _, cx| cx.notify()),
        ];
        Self {
            app_store,
            updater,
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }

    pub(crate) fn pause(&mut self) {}
}

impl Render for UpdatesPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let updater = self.updater.read(cx);
        let checking = updater.is_checking();
        let automatically_checks = updater.automatically_checks_for_updates();
        let status = if checking {
            "Checking for updates…".into()
        } else if let Some(error) = updater.check_error() {
            error
        } else if let Some(available) = updater.available() {
            format!("Update available: v{}", available.version())
        } else if option_env!("RUNNER_BUILD_STAMP").is_none() {
            "Local development build; version comparison is unavailable.".into()
        } else if updater.last_check_at().is_some() {
            "You're up to date.".into()
        } else {
            "No update check yet.".into()
        };
        let last_checked = updater
            .last_check_at()
            .map(DateTime::<Local>::from)
            .map(|time| time.format("%b %-d, %Y at %-I:%M %p").to_string())
            .unwrap_or_else(|| "Never".into());
        let check_updater = self.updater.clone();
        let toggle_updater = self.updater.clone();
        let toggle_store = self.app_store.clone();
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(PaneHeader::new(
                "Updates",
                "Get notified about new Windows builds. Download and install them manually.",
            ))
            .child(SettingsCard::new([
                SettingsRow::new(
                    "Installed version",
                    div()
                        .font_family(theme::SYSTEM_MONOSPACE_FONT)
                        .text_sm()
                        .text_color(theme::muted())
                        .child(format!("v{}", runner_app::version::display_version())),
                )
                .into_any_element(),
                SettingsRow::new(
                    "Update status",
                    Button::new("updates-check", "Check for updates")
                        .icon("refresh-cw.svg")
                        .size(ButtonSize::Sm)
                        .disabled(checking)
                        .on_press(move |_, cx| check_updater.read(cx).check_for_updates()),
                )
                .subtitle(status)
                .into_any_element(),
                SettingsRow::new(
                    "Windows downloads",
                    Button::new("updates-download", "View downloads")
                        .icon("external-link.svg")
                        .size(ButtonSize::Sm)
                        .on_press(|_, cx| {
                            cx.open_url(windows_download_url());
                        }),
                )
                .subtitle("Download Runner-Setup, close Runner, then run the installer. Your settings and missions stay in place.")
                .into_any_element(),
                SettingsRow::new(
                    "Automatically check for updates",
                    Toggle::new("updates-auto-check", automatically_checks).on_change(
                        move |enabled, _, cx| {
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
                                updater.set_automatically_checks_for_updates(enabled, updater_cx);
                            });
                        },
                    ),
                )
                .subtitle("Packaged builds check at startup and every six hours.")
                .into_any_element(),
                SettingsRow::new(
                    "Last checked",
                    div()
                        .text_sm()
                        .text_color(theme::muted())
                        .child(last_checked),
                )
                .into_any_element(),
            ]))
    }
}
