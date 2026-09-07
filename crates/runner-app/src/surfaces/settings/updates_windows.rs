use chrono::{DateTime, Local};
use gpui::prelude::*;
use gpui::{div, img, rems, Context, Entity, FontWeight, Render, Subscription, Window};
use runner_app::ui::{
    Button, ButtonSize, ButtonVariant, PaneHeader, SettingsCard, SettingsRow, Toggle,
};
use runner_app::updater::{UpdateState, Updater};

use crate::app_store::AppStore;
use crate::assets::app_icon_source;
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
        let up_to_date = matches!(updater.state(), UpdateState::UpToDate { .. });
        let checking = updater.is_checking();
        let automatically_downloads = updater.automatically_downloads_updates();
        let status = match updater.state() {
            UpdateState::UpToDate { .. } if checking => "Checking for updates…".into(),
            UpdateState::UpToDate { .. } if option_env!("RUNNER_BUILD_STAMP").is_none() => {
                "Local development build; version comparison is unavailable.".into()
            }
            UpdateState::UpToDate { .. } if updater.last_check_at().is_none() => {
                "No update check yet.".into()
            }
            UpdateState::UpToDate { .. } => "You're up to date.".into(),
            UpdateState::Available { info, .. } => {
                format!("Runner {} is available.", info.version())
            }
            UpdateState::Downloading { .. } => format!(
                "Downloading Runner {}…",
                updater.update_info().map_or("", |info| info.version())
            ),
            UpdateState::Ready { info, .. } => {
                format!("Runner {} is ready to install.", info.version())
            }
            UpdateState::Failed { .. } => "Update failed.".into(),
        };
        let last_checked = updater
            .last_check_at()
            .map(DateTime::<Local>::from)
            .map(|time| time.format("%b %-d, %Y at %-I:%M %p").to_string())
            .unwrap_or_else(|| "Never".into());
        let action_updater = self.updater.clone();
        let toggle_updater = self.updater.clone();
        let toggle_store = self.app_store.clone();
        div().flex().flex_col().gap_5()
            .child(PaneHeader::new("Updates", "Runner checks for new Windows builds at startup and every six hours, downloads them in the background, and installs when you choose."))
            .child(div().overflow_hidden().rounded(rems(12. / 16.)).border_1().border_color(theme::border()).bg(theme::panel())
                .child(div().flex().items_center().gap_4().p_5()
                    .child(img(app_icon_source()).size(rems(56. / 16.)).flex_none().rounded(rems(1.)))
                    .child(div().min_w_0().flex_1().flex().flex_col().gap_1()
                        .child(div().flex().items_center().gap_2()
                            .child(div().text_size(rems(1.)).font_weight(FontWeight::BOLD).text_color(theme::text()).child("Runner"))
                            .child(div().rounded_sm().bg(theme::raised()).px(rems(6. / 16.)).py(rems(2. / 16.))
                                .font_family(theme::SYSTEM_MONOSPACE_FONT).text_size(rems(11. / 16.)).text_color(theme::muted())
                                .child(format!("v{}", runner_app::version::display_version()))))
                        .child(div().text_size(rems(12. / 16.)).text_color(theme::muted()).child(status)))
                    .child(Button::new("updates-check", if up_to_date { "Check for updates" } else { "Update" })
                        .icon(if up_to_date { "refresh-cw.svg" } else { "circle-arrow-down.svg" })
                        .size(ButtonSize::Sm)
                        .variant(if up_to_date { ButtonVariant::Secondary } else { ButtonVariant::Primary })
                        .loading(up_to_date && checking)
                        .on_press(move |window, cx| {
                            if matches!(action_updater.read(cx).state(), UpdateState::UpToDate { .. }) {
                                action_updater.read(cx).check_for_updates();
                            } else {
                                crate::platform_ui::activate_update_hint(&action_updater, window, cx);
                            }
                        }))))
            .child(SettingsCard::new([
                SettingsRow::new("Automatically download updates",
                    Toggle::new("updates-auto-download", automatically_downloads).on_change(move |enabled, _, cx| {
                        toggle_store.update(cx, |store, store_cx| {
                            store.update_settings(|settings| {
                                if settings.automatically_download_updates == enabled { return false; }
                                settings.automatically_download_updates = enabled;
                                true
                            }, true, store_cx);
                        });
                        toggle_updater.update(cx, |updater, updater_cx| updater.set_automatically_downloads_updates(enabled, updater_cx));
                    }))
                    .subtitle("Downloads are verified before Runner offers to install them. Turn off to be notified only.")
                    .into_any_element(),
                SettingsRow::new("Last checked", div().text_size(rems(12. / 16.)).text_color(theme::muted()).child(last_checked)).into_any_element(),
            ]))
    }
}
