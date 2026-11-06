use std::path::PathBuf;
use std::time::SystemTime;

use chrono::{DateTime, Local, Utc};
use gpui::prelude::*;
use gpui::{
    div, relative, rems, AnyElement, Context, Entity, Render, Subscription, WeakEntity, Window,
};
use runner_app::ui::{
    Button, ButtonSize, ButtonVariant, PaneHeader, SettingsCard, SettingsRow, Toggle,
};
use runner_app::updater::{UpdateState, Updater};

use crate::app_store::AppStore;
use crate::surfaces::update_ui::{run_update_action, working_note, UpdateAction};
use crate::{theme, NativeRoot};

pub(crate) struct UpdatesPane {
    app_store: Entity<AppStore>,
    updater: Entity<Updater>,
    log_dir: PathBuf,
    root: WeakEntity<NativeRoot>,
    _subscriptions: Vec<Subscription>,
}

impl UpdatesPane {
    pub(crate) fn new(
        app_store: Entity<AppStore>,
        updater: Entity<Updater>,
        log_dir: PathBuf,
        root: WeakEntity<NativeRoot>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&updater, |_, _, cx| cx.notify()),
            cx.observe(&app_store, |_, _, cx| cx.notify()),
        ];
        Self {
            app_store,
            updater,
            log_dir,
            root,
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }
}

#[derive(Debug, Default)]
struct HeroFacts {
    /// False when this build can't check at all, such as an unbundled macOS build.
    controls_available: bool,
    /// False for a local build, which has no release identity to compare against.
    compares_versions: bool,
    skipped: bool,
    last_check_at: Option<SystemTime>,
    working: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HeroButton {
    label: &'static str,
    action: UpdateAction,
    primary: bool,
    icon: Option<&'static str>,
    busy: bool,
    disabled: bool,
}

impl HeroButton {
    fn new(label: &'static str, action: UpdateAction) -> Self {
        Self {
            label,
            action,
            primary: false,
            icon: None,
            busy: false,
            disabled: false,
        }
    }

    fn primary(mut self) -> Self {
        self.primary = true;
        self
    }

    fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    fn check() -> Self {
        Self::new(
            crate::i18n::text("ui.check_for_updates_action"),
            UpdateAction::Check,
        )
        .icon("refresh-cw.svg")
    }

    fn view_downloads() -> Self {
        Self::new(
            crate::i18n::text("ui.view_downloads"),
            UpdateAction::OpenRelease,
        )
    }
}

#[derive(Debug, Default, Eq, PartialEq)]
struct HeroContent {
    status: String,
    danger: bool,
    buttons: Vec<HeroButton>,
    progress: Option<(u64, u64)>,
    working: Option<String>,
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1_000_000.)
}

fn progress_caption(received: u64, total: u64) -> String {
    if total == 0 {
        crate::i18n::tf(
            "format.mb_downloaded",
            &[("megabytes", megabytes(received))],
        )
    } else {
        crate::i18n::tf(
            "format.mb_of_mb",
            &[
                ("received_mb", megabytes(received)),
                ("total_mb", megabytes(total)),
                (
                    "percent",
                    format!("{:.0}", (received as f64 / total as f64 * 100.).min(100.)),
                ),
            ],
        )
    }
}

fn up_to_date_status(facts: &HeroFacts) -> String {
    if !facts.controls_available {
        return crate::i18n::text("ui.update_controls_are_available_in_bundled_builds").into();
    }
    if !facts.compares_versions {
        return crate::i18n::text("ui.local_development_build_version_comparison_is_unavailable")
            .into();
    }
    match facts.last_check_at {
        Some(checked_at) => crate::i18n::tf(
            "format.up_to_date_checked",
            &[(
                "time",
                runner_app::text_util::format_relative_time(
                    DateTime::<Utc>::from(checked_at),
                    Utc::now(),
                ),
            )],
        ),
        None => crate::i18n::text("ui.no_update_check_yet").into(),
    }
}

/// The hero runs each action directly, and shows the same state as the sidebar icon.
fn hero_content(state: &UpdateState, facts: &HeroFacts) -> HeroContent {
    let version = state.info().map_or("", |info| info.version()).to_string();
    let text = |key: &str| crate::i18n::tf(key, &[("version", version.clone())]);
    let check = || HeroButton {
        disabled: !facts.controls_available,
        ..HeroButton::check()
    };
    if facts.skipped {
        return HeroContent {
            status: text("format.update_skipped"),
            buttons: vec![check()],
            ..Default::default()
        };
    }
    match state {
        UpdateState::UpToDate { checking: true } => HeroContent {
            status: crate::i18n::text("ui.checking_for_updates").into(),
            buttons: vec![HeroButton {
                busy: true,
                ..HeroButton::new(crate::i18n::text("ui.checking"), UpdateAction::Check)
            }],
            ..Default::default()
        },
        UpdateState::UpToDate { checking: false } => HeroContent {
            status: up_to_date_status(facts),
            buttons: vec![check()],
            ..Default::default()
        },
        UpdateState::Available {
            manual_only: true, ..
        } => HeroContent {
            status: format!(
                "{} {}",
                text("format.runner_update_available_sentence"),
                crate::i18n::text("ui.update_manual_only")
            ),
            buttons: vec![HeroButton::view_downloads()],
            ..Default::default()
        },
        UpdateState::Available { size, .. } => HeroContent {
            status: if *size > 0 {
                crate::i18n::tf(
                    "format.update_available_with_size",
                    &[
                        ("version", version.clone()),
                        ("megabytes", megabytes(*size)),
                    ],
                )
            } else {
                text("format.runner_is_available")
            },
            buttons: vec![
                HeroButton::new(
                    crate::i18n::text("ui.skip_this_version"),
                    UpdateAction::Skip,
                ),
                HeroButton::new(crate::i18n::text("ui.download"), UpdateAction::Download)
                    .primary()
                    .icon("circle-arrow-down.svg"),
            ],
            ..Default::default()
        },
        UpdateState::Downloading {
            received, total, ..
        } => HeroContent {
            status: text("format.downloading_runner"),
            buttons: vec![HeroButton::new(
                crate::i18n::text("ui.cancel"),
                UpdateAction::Cancel,
            )],
            progress: Some((*received, *total)),
            ..Default::default()
        },
        UpdateState::Verifying { .. } => HeroContent {
            status: format!("{}…", text("format.update_verifying")),
            ..Default::default()
        },
        UpdateState::Ready { size, .. } => HeroContent {
            status: if *size > 0 {
                crate::i18n::tf(
                    "format.update_ready_downloaded",
                    &[
                        ("version", version.clone()),
                        ("megabytes", megabytes(*size)),
                    ],
                )
            } else {
                text("format.update_is_ready")
            },
            buttons: vec![HeroButton::new(
                crate::i18n::text("ui.restart_to_update"),
                UpdateAction::Restart,
            )
            .primary()
            .icon("rotate-cw.svg")],
            working: runner_app::lifecycle::working_caption(facts.working),
            ..Default::default()
        },
        UpdateState::Installing { .. } => HeroContent {
            status: format!(
                "{} · {}",
                text("format.update_installing"),
                crate::i18n::text("ui.update_reopens_when_done")
            ),
            ..Default::default()
        },
        UpdateState::Failed { message, .. } => HeroContent {
            status: message.clone(),
            danger: true,
            buttons: vec![
                HeroButton::view_downloads(),
                HeroButton::new(crate::i18n::text("ui.retry"), UpdateAction::Retry).primary(),
            ],
            ..Default::default()
        },
    }
}

impl UpdatesPane {
    fn run_action(&self, action: UpdateAction, cx: &mut gpui::App) {
        run_update_action(action, &self.updater, &self.app_store, &self.log_dir, cx);
        if matches!(action, UpdateAction::Download | UpdateAction::Retry) {
            let _ = self
                .root
                .update(cx, |root, cx| root.mark_update_download_started(cx));
        }
    }

    fn render_hero(&self, content: HeroContent, cx: &mut Context<Self>) -> AnyElement {
        let pane = cx.entity();
        let status = div()
            .debug_selector(|| "UPDATE_HERO_STATUS".into())
            .whitespace_normal()
            .text_size(theme::text_body())
            .text_color(if content.danger {
                theme::danger()
            } else {
                theme::muted()
            })
            .child(content.status)
            .into_any_element();
        let buttons = content.buttons.into_iter().map(|button| {
            let pane = pane.clone();
            Button::new(("update-hero", button.action as usize), button.label)
                .size(ButtonSize::Compact)
                .variant(if button.primary {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Secondary
                })
                .when_some(button.icon, |this, icon| this.icon(icon))
                .loading(button.busy)
                .disabled(button.disabled)
                .debug_selector(format!("UPDATE_HERO_{:?}", button.action))
                .on_press(move |_, cx| {
                    pane.update(cx, |pane, cx| pane.run_action(button.action, cx));
                })
        });
        let actions = div()
            .flex_none()
            .flex()
            .items_center()
            .gap_4()
            .children(content.working.map(working_note))
            .child(div().flex().items_center().gap_2().children(buttons))
            .into_any_element();
        let below = content.progress.map(|(received, total)| {
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap(rems(6. / 16.))
                .child(
                    div()
                        .w_full()
                        .h(rems(4. / 16.))
                        .rounded(rems(2. / 16.))
                        .overflow_hidden()
                        .bg(theme::sidebar_selected())
                        .child(
                            div()
                                .debug_selector(|| "UPDATE_HERO_PROGRESS".into())
                                .h_full()
                                .w(relative(
                                    crate::surfaces::update_ui::ring_fraction(received, total)
                                        .unwrap_or(0.),
                                ))
                                .bg(theme::accent()),
                        ),
                )
                .child(
                    div()
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_meta())
                        .text_color(theme::muted())
                        .child(progress_caption(received, total)),
                )
                .into_any_element()
        });
        super::app_identity_card([status], Some(actions), below).into_any_element()
    }
}

impl Render for UpdatesPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (state, facts, last_checked, checks, available) = {
            let updater = self.updater.read(cx);
            let available = updater.is_available();
            let settings = &self.app_store.read(cx).settings;
            let checks = if available && !updater.is_preview() {
                updater.automatically_checks_for_updates()
            } else {
                settings.automatically_check_for_updates
            };
            (
                updater.state().clone(),
                HeroFacts {
                    controls_available: available,
                    compares_versions: updater.is_preview()
                        || cfg!(target_os = "macos")
                        || option_env!("RUNNER_BUILD_STAMP").is_some(),
                    skipped: updater.is_skipped(),
                    last_check_at: updater.last_check_at(),
                    working: self.app_store.read(cx).working_agent_count(),
                },
                format_last_checked(updater.last_check_at()),
                checks,
                available,
            )
        };
        let content = hero_content(&state, &facts);
        let hero = self.render_hero(content, cx);
        let last_checked_row = SettingsRow::new(
            crate::i18n::text("ui.last_checked"),
            div()
                .text_size(theme::text_ui())
                .text_color(theme::muted())
                .child(last_checked),
        )
        .into_any_element();
        #[cfg(target_os = "macos")]
        let description = crate::i18n::text("ui.runner_checks_for_new_versions_in_the_background");
        #[cfg(windows)]
        let description = crate::i18n::text(
            "ui.runner_checks_for_new_windows_builds_at_startup_and_every_six_hours_downloads_them_in_the",
        );
        #[cfg(target_os = "linux")]
        let description = crate::i18n::text("ui.linux_updates_description");
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(PaneHeader::new(
                crate::i18n::text("ui.updates"),
                description,
            ))
            .child(hero)
            .child(SettingsCard::new([
                self.render_automatic_row(checks, available, cx),
                last_checked_row,
            ]))
    }
}

impl UpdatesPane {
    #[cfg(target_os = "macos")]
    fn render_automatic_row(
        &self,
        checks: bool,
        available: bool,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let toggle_updater = self.updater.clone();
        let toggle_store = self.app_store.clone();
        SettingsRow::new(
            crate::i18n::text("ui.automatically_check_for_updates"),
            Toggle::new("updates-auto-check", checks)
                .disabled(!available)
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
        .subtitle(crate::i18n::text(
            "ui.look_for_a_newer_signed_release_when_due",
        ))
        .into_any_element()
    }

    #[cfg(any(windows, target_os = "linux"))]
    fn render_automatic_row(
        &self,
        _checks: bool,
        _available: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let automatically_downloads = self.updater.read(cx).automatically_downloads_updates();
        let toggle_updater = self.updater.clone();
        let toggle_store = self.app_store.clone();
        SettingsRow::new(
            crate::i18n::text("ui.automatically_download_updates"),
            Toggle::new("updates-auto-download", automatically_downloads).on_change(
                move |enabled, _, cx| {
                    toggle_store.update(cx, |store, store_cx| {
                        store.update_settings(
                            |settings| {
                                if settings.automatically_download_updates == enabled {
                                    return false;
                                }
                                settings.automatically_download_updates = enabled;
                                true
                            },
                            true,
                            store_cx,
                        );
                    });
                    toggle_updater.update(cx, |updater, updater_cx| {
                        updater.set_automatically_downloads_updates(enabled, updater_cx)
                    });
                },
            ),
        )
        .subtitle(crate::i18n::text(
            "ui.downloads_are_verified_before_runner_offers_to_install_them_turn_off_to_be_notified_only",
        ))
        .into_any_element()
    }
}

fn format_last_checked(checked_at: Option<SystemTime>) -> String {
    checked_at
        .map(DateTime::<Local>::from)
        .map(|checked_at| {
            checked_at
                .format(crate::i18n::text("date.checked"))
                .to_string()
        })
        .unwrap_or_else(|| crate::i18n::text("ui.never").into())
}

#[cfg(test)]
mod tests {
    use runner_app::updater::{UpdateInfo, UpdateStep};

    use super::*;

    fn facts() -> HeroFacts {
        HeroFacts {
            controls_available: true,
            compares_versions: true,
            ..Default::default()
        }
    }

    fn actions(content: &HeroContent) -> Vec<UpdateAction> {
        content.buttons.iter().map(|button| button.action).collect()
    }

    #[test]
    fn missing_check_date_is_never() {
        assert_eq!(format_last_checked(None), "Never");
    }

    #[test]
    fn progress_caption_handles_an_unknown_size() {
        assert_eq!(
            progress_caption(18_400_000, 40_400_000),
            "18.4 MB of 40.4 MB · 46%"
        );
        assert_eq!(progress_caption(18_400_000, 0), "18.4 MB downloaded");
    }

    #[test]
    fn the_hero_says_up_to_date_after_a_check_and_reports_checking() {
        let up_to_date = UpdateState::UpToDate { checking: false };
        let never = hero_content(&up_to_date, &facts());
        assert_eq!(never.status, "No update check yet.");
        assert_eq!(actions(&never), [UpdateAction::Check]);
        let checked = hero_content(
            &up_to_date,
            &HeroFacts {
                last_check_at: Some(SystemTime::now()),
                ..facts()
            },
        );
        assert_eq!(
            checked.status,
            "You're on the latest version · checked just now"
        );
        let checking = hero_content(&UpdateState::UpToDate { checking: true }, &facts());
        assert_eq!(checking.status, "Checking for updates…");
        assert!(checking.buttons[0].busy);
        assert_eq!(checking.buttons[0].label, "Checking…");
    }

    #[test]
    fn a_build_with_no_updater_says_so_and_disables_the_check() {
        let content = hero_content(
            &UpdateState::UpToDate { checking: false },
            &HeroFacts::default(),
        );
        assert_eq!(
            content.status,
            "Update controls are available in bundled builds."
        );
        assert!(content.buttons[0].disabled);
    }

    #[test]
    fn each_state_offers_its_actions_directly() {
        let info = UpdateInfo::new("0.19.5");
        let available = hero_content(
            &UpdateState::Available {
                info: info.clone(),
                size: 40_400_000,
                manual_only: false,
            },
            &facts(),
        );
        assert_eq!(available.status, "Runner 0.19.5 is available · 40.4 MB");
        assert_eq!(
            actions(&available),
            [UpdateAction::Skip, UpdateAction::Download]
        );
        let manual = hero_content(
            &UpdateState::Available {
                info: info.clone(),
                size: 40_400_000,
                manual_only: true,
            },
            &facts(),
        );
        assert_eq!(actions(&manual), [UpdateAction::OpenRelease]);
        let downloading = hero_content(
            &UpdateState::Downloading {
                info: info.clone(),
                received: 1,
                total: 2,
            },
            &facts(),
        );
        assert_eq!(downloading.status, "Downloading Runner 0.19.5…");
        assert_eq!(actions(&downloading), [UpdateAction::Cancel]);
        assert_eq!(downloading.progress, Some((1, 2)));
        assert!(
            hero_content(&UpdateState::Verifying { info: info.clone() }, &facts())
                .buttons
                .is_empty()
        );
        assert!(
            hero_content(&UpdateState::Installing { info: info.clone() }, &facts())
                .buttons
                .is_empty()
        );
        let failed = hero_content(
            &UpdateState::Failed {
                step: UpdateStep::Download,
                message: "Download failed.".into(),
                info: Some(info),
            },
            &facts(),
        );
        assert_eq!(failed.status, "Download failed.");
        assert!(failed.danger);
        assert_eq!(
            actions(&failed),
            [UpdateAction::OpenRelease, UpdateAction::Retry]
        );
    }

    #[test]
    fn ready_offers_restart_with_the_working_count() {
        let ready = UpdateState::Ready {
            info: UpdateInfo::new("0.19.5"),
            size: 40_400_000,
        };
        let idle = hero_content(&ready, &facts());
        assert_eq!(
            idle.status,
            "Runner 0.19.5 is ready · 40.4 MB downloaded and verified"
        );
        assert_eq!(actions(&idle), [UpdateAction::Restart]);
        assert_eq!(idle.working, None);
        let working = hero_content(
            &ready,
            &HeroFacts {
                working: 2,
                ..facts()
            },
        );
        assert_eq!(working.working.as_deref(), Some("2 agents working"));
    }

    #[test]
    fn a_skipped_version_is_named_and_offers_a_check() {
        let content = hero_content(
            &UpdateState::Available {
                info: UpdateInfo::new("0.19.5"),
                size: 1,
                manual_only: false,
            },
            &HeroFacts {
                skipped: true,
                ..facts()
            },
        );
        assert_eq!(content.status, "You skipped Runner 0.19.5.");
        assert_eq!(actions(&content), [UpdateAction::Check]);
    }
}
