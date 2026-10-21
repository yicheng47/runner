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
            UpdateState::UpToDate { .. } if checking => {
                crate::i18n::text("ui.checking_for_updates").into()
            }
            UpdateState::UpToDate { .. } if option_env!("RUNNER_BUILD_STAMP").is_none() => {
                crate::i18n::text("ui.local_development_build_version_comparison_is_unavailable")
                    .into()
            }
            UpdateState::UpToDate { .. } if updater.last_check_at().is_none() => {
                crate::i18n::text("ui.no_update_check_yet").into()
            }
            UpdateState::UpToDate { .. } => crate::i18n::text("ui.you_re_up_to_date").into(),
            UpdateState::Available { info, .. } => crate::i18n::tf(
                "format.runner_update_available_sentence",
                &[("version", (info.version()).to_string())],
            ),
            UpdateState::Downloading { .. } => crate::i18n::tf(
                "format.downloading_runner",
                &[(
                    "version",
                    (updater.update_info().map_or("", |info| info.version())).to_string(),
                )],
            ),
            UpdateState::Ready { info, .. } => crate::i18n::tf(
                "format.runner_update_ready_sentence",
                &[("version", (info.version()).to_string())],
            ),
            UpdateState::Failed { .. } => crate::i18n::text("ui.update_failed").into(),
        };
        let last_checked = updater
            .last_check_at()
            .map(DateTime::<Local>::from)
            .map(|time| time.format(crate::i18n::text("date.checked")).to_string())
            .unwrap_or_else(|| crate::i18n::text("ui.never").into());
        let action_updater = self.updater.clone();
        let toggle_updater = self.updater.clone();
        let toggle_store = self.app_store.clone();
        div().flex().flex_col().gap_5()
            .child(PaneHeader::new(crate::i18n::text("ui.updates"), crate::i18n::text("ui.linux_updates_description")))
            .child(div().overflow_hidden().rounded(rems(12. / 16.)).border_1().border_color(theme::border()).bg(theme::panel())
                .child(div().flex().items_center().gap_4().p_5()
                    .child(img(app_icon_source()).size(rems(56. / 16.)).flex_none().rounded(rems(1.)))
                    .child(div().min_w_0().flex_1().flex().flex_col().gap_1()
                        .child(div().flex().items_center().gap_2()
                            .child(div().text_size(theme::text_heading()).font_weight(FontWeight::BOLD).text_color(theme::text()).child("Runner"))
                            .child(div().rounded_sm().bg(theme::raised()).px(rems(6. / 16.)).py(rems(2. / 16.))
                                .font_family(theme::UI_MONOSPACE_FONT).text_size(theme::text_meta()).text_color(theme::muted())
                                .child(runner_app::version::display_version_label())))
                        .child(div().text_size(theme::text_ui()).text_color(theme::muted()).child(status)))
                    .child(Button::new("updates-check", if up_to_date { crate::i18n::text("ui.check_for_updates_action") } else { crate::i18n::text("ui.update") })
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
                SettingsRow::new(crate::i18n::text("ui.automatically_download_updates"),
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
                    .subtitle(crate::i18n::text("ui.downloads_are_verified_before_runner_offers_to_install_them_turn_off_to_be_notified_only"))
                    .into_any_element(),
                SettingsRow::new(crate::i18n::text("ui.last_checked"), div().text_size(theme::text_ui()).text_color(theme::muted()).child(last_checked)).into_any_element(),
            ]))
    }
}
