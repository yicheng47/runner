use gpui::prelude::*;
use gpui::{div, rems, AnyElement, Context};
use runner_app::ui::{PaneHeader, SettingsCard, SettingsRow, Toggle};

#[cfg(not(target_os = "linux"))]
use runner_app::ui::{Button, ButtonSize, ButtonVariant};

use crate::app_settings::NotificationSettings;
use crate::notifications::Permission;
use crate::surfaces::settings_page::settings_section_heading;
use crate::{theme, NativeRoot};

#[cfg(target_os = "macos")]
const SETTINGS_BUTTON: &str = "ui.open_system_settings";
#[cfg(windows)]
const SETTINGS_BUTTON: &str = "ui.open_settings";

#[derive(Debug, PartialEq, Eq)]
struct SystemRow {
    title: &'static str,
    subtitle: &'static str,
    warn: bool,
}

fn system_row(permission: Permission) -> SystemRow {
    let row = |title, subtitle| SystemRow {
        title,
        subtitle,
        warn: false,
    };
    #[cfg(target_os = "macos")]
    match permission {
        Permission::Unknown => row(
            crate::i18n::text("ui.allow_notifications_banner_style_and_sound"),
            crate::i18n::text("ui.set_by_macos_in_system_settings_notifications_runner"),
        ),
        Permission::Allowed => row(
            crate::i18n::text("ui.notifications_are_on"),
            crate::i18n::text(
                "ui.banner_style_and_sound_are_set_in_system_settings_notifications_runner",
            ),
        ),
        Permission::Denied => SystemRow {
            warn: true,
            ..row(
                crate::i18n::text("ui.notifications_are_off"),
                crate::i18n::text("ui.allow_notifications_in_system_settings_notifications_runner"),
            )
        },
    }
    #[cfg(target_os = "linux")]
    {
        let _ = permission;
        row(
            crate::i18n::text("ui.desktop_notifications"),
            crate::i18n::text("ui.desktop_notification_delivery_settings"),
        )
    }
    #[cfg(windows)]
    match permission {
        Permission::Unknown => row(
            crate::i18n::text("ui.allow_notifications_banner_style_and_sound"),
            crate::i18n::text("ui.set_by_windows_in_settings_system_notifications_runner"),
        ),
        Permission::Allowed => row(
            crate::i18n::text("ui.notifications_are_on"),
            crate::i18n::text(
                "ui.banners_and_sound_are_set_in_settings_system_notifications_runner",
            ),
        ),
        Permission::Denied => SystemRow {
            warn: true,
            ..row(
                crate::i18n::text("ui.notifications_are_off"),
                crate::i18n::text("ui.not_allowed_in_windows_settings_so_runner_can_t_notify_you"),
            )
        },
    }
}

impl NativeRoot {
    pub(crate) fn render_notifications_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let row = system_row(crate::notifications::permission(cx));
        #[cfg(not(target_os = "linux"))]
        let url = crate::notifications::settings_url();
        #[cfg(not(target_os = "linux"))]
        let control = Button::new(
            "notification-system-settings",
            crate::i18n::text(SETTINGS_BUTTON),
        )
        .icon("external-link.svg")
        .size(ButtonSize::Sm)
        .variant(ButtonVariant::Secondary)
        .on_press(move |_, cx| cx.open_url(&url));
        #[cfg(target_os = "linux")]
        let control = div();
        let mut system = SettingsRow::new(row.title, control).subtitle(row.subtitle);
        if row.warn {
            system = system.label_icon("triangle-alert.svg", theme::warning());
        }
        let system = system.into_any_element();
        let chats = vec![
            self.notification_toggle(
                "notification-chats-input",
                crate::i18n::text("ui.needs_your_input_notification"),
                crate::i18n::text("ui.approval_requests_questions_or_other_input"),
                |s| &mut s.chats.input,
                cx,
            ),
            self.notification_toggle(
                "notification-chats-finished",
                crate::i18n::text("ui.finishes_a_turn"),
                crate::i18n::text("ui.the_agent_is_done_and_waiting_for_your_next_message"),
                |s| &mut s.chats.finished,
                cx,
            ),
            self.notification_toggle(
                "notification-chats-failed",
                crate::i18n::text("ui.fails_or_exits"),
                crate::i18n::text("ui.response_failed_or_the_agent_process_stopped_with_an_error"),
                |s| &mut s.chats.failed,
                cx,
            ),
        ];
        let missions = vec![
            self.notification_toggle(
                "notification-missions-question",
                crate::i18n::text("ui.asks_you_a_question"),
                crate::i18n::text("ui.a_crew_member_asks_you_something_through_the_feed"),
                |s| &mut s.missions.question,
                cx,
            ),
            self.notification_toggle(
                "notification-missions-input",
                crate::i18n::text("ui.needs_your_input_notification"),
                crate::i18n::text("ui.a_crew_member_waits_for_approval_an_answer_or_input"),
                |s| &mut s.missions.input,
                cx,
            ),
            self.notification_toggle(
                "notification-missions-idle",
                crate::i18n::text("ui.goes_idle"),
                crate::i18n::text(
                    "ui.every_crew_member_is_idle_so_the_mission_has_stopped_making_progress",
                ),
                |s| &mut s.missions.idle,
                cx,
            ),
            self.notification_toggle(
                "notification-missions-failed",
                crate::i18n::text("ui.fails_or_exits"),
                crate::i18n::text(
                    "ui.a_crew_member_s_response_fails_or_its_process_stops_with_an_error",
                ),
                |s| &mut s.missions.failed,
                cx,
            ),
        ];
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(PaneHeader::new(
                crate::i18n::text("ui.notifications"),
                crate::i18n::text(
                    "ui.choose_when_runner_notifies_you_while_the_app_is_in_the_background",
                ),
            ))
            .children(
                [
                    (crate::i18n::text("ui.system"), vec![system]),
                    (crate::i18n::text("ui.chats"), chats),
                    (crate::i18n::text("ui.missions"), missions),
                ]
                .into_iter()
                .map(|(heading, rows)| {
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(8. / 16.))
                        .child(settings_section_heading(heading))
                        .child(SettingsCard::new(rows))
                }),
            )
            .into_any_element()
    }

    fn notification_toggle(
        &self,
        id: &'static str,
        label: &'static str,
        subtitle: &'static str,
        field: fn(&mut NotificationSettings) -> &mut bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        SettingsRow::new(label, self.notification_switch(id, field, cx))
            .subtitle(subtitle)
            .into_any_element()
    }

    fn notification_switch(
        &self,
        id: &'static str,
        field: fn(&mut NotificationSettings) -> &mut bool,
        cx: &mut Context<Self>,
    ) -> Toggle {
        let enabled = *field(&mut self.app_store.read(cx).settings.notifications.clone());
        let root = cx.entity();
        Toggle::new(id, enabled).on_change(move |enabled, _, cx| {
            root.update(cx, |this, cx| {
                this.update_app_settings(cx, true, |settings| {
                    *field(&mut settings.notifications) = enabled;
                    true
                });
            });
        })
    }
}

#[cfg(all(test, not(target_os = "linux")))]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn system_row_follows_the_system_permission() {
        let unknown = system_row(Permission::Unknown);
        assert!(!unknown.warn);
        assert_eq!(unknown.title, "Allow notifications, banner style and sound");

        let allowed = system_row(Permission::Allowed);
        assert!(!allowed.warn);
        assert_eq!(allowed.title, "Notifications are on");

        let denied = system_row(Permission::Denied);
        assert!(denied.warn);
        assert_eq!(denied.title, "Notifications are off");
    }
}
