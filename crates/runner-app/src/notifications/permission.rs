use futures::channel::oneshot;

/// Whether the system lets Runner post notifications. `Unknown` covers a
/// build that cannot ask, such as an unbundled macOS build, and the moment
/// before the first answer arrives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Permission {
    #[default]
    Unknown,
    #[cfg(any(target_os = "macos", windows))]
    Allowed,
    #[cfg(any(target_os = "macos", windows))]
    Denied,
}

pub(super) fn query() -> oneshot::Receiver<Permission> {
    platform::query()
}

/// macOS lists an app under System Settings → Notifications only once it
/// has requested authorization, so Runner asks at launch rather than on its
/// first notification. The system prompts once; later requests return the
/// saved answer.
pub(super) fn request() -> oneshot::Receiver<()> {
    platform::request()
}

/// System Settings opens on Runner's own notification page when the link
/// names the running app's bundle id.
#[cfg(target_os = "macos")]
pub(crate) fn settings_url() -> String {
    macos_settings_url(platform::bundle_id())
}

#[cfg(windows)]
pub(crate) fn settings_url() -> String {
    "ms-settings:notifications".into()
}

#[cfg(target_os = "macos")]
fn macos_settings_url(bundle_id: Option<String>) -> String {
    let page = "x-apple.systempreferences:com.apple.Notifications-Settings.extension";
    bundle_id.map_or_else(|| page.into(), |id| format!("{page}?id={id}"))
}

#[cfg(all(target_os = "macos", not(test)))]
mod platform {
    use std::ptr::NonNull;
    use std::sync::Mutex;

    use block2::RcBlock;
    use futures::channel::oneshot;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSBundle, NSError};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNAuthorizationStatus, UNNotificationSettings,
        UNUserNotificationCenter,
    };

    use super::Permission;

    // `UNUserNotificationCenter` aborts the process outside an app bundle,
    // as GPUI's own notification code notes.
    fn bundled() -> bool {
        bundle_id().is_some()
    }

    pub(super) fn bundle_id() -> Option<String> {
        NSBundle::mainBundle()
            .bundleIdentifier()
            .map(|id| id.to_string())
    }

    pub(super) fn query() -> oneshot::Receiver<Permission> {
        let (tx, rx) = oneshot::channel();
        if !bundled() {
            let _ = tx.send(Permission::Unknown);
            return rx;
        }
        let tx = Mutex::new(Some(tx));
        let completion = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
            // SAFETY: the system passes a valid settings object for the
            // duration of the callback.
            let status = unsafe { settings.as_ref() }.authorizationStatus();
            let permission = if status == UNAuthorizationStatus::NotDetermined
                || status == UNAuthorizationStatus::Denied
            {
                Permission::Denied
            } else {
                Permission::Allowed
            };
            if let Some(tx) = tx.lock().unwrap().take() {
                let _ = tx.send(permission);
            }
        });
        UNUserNotificationCenter::currentNotificationCenter()
            .getNotificationSettingsWithCompletionHandler(&completion);
        rx
    }

    pub(super) fn request() -> oneshot::Receiver<()> {
        let (tx, rx) = oneshot::channel();
        if !bundled() {
            let _ = tx.send(());
            return rx;
        }
        let tx = Mutex::new(Some(tx));
        let completion = RcBlock::new(move |_: Bool, _: *mut NSError| {
            if let Some(tx) = tx.lock().unwrap().take() {
                let _ = tx.send(());
            }
        });
        UNUserNotificationCenter::currentNotificationCenter()
            .requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                &completion,
            );
        rx
    }
}

#[cfg(all(windows, not(test)))]
mod platform {
    use futures::channel::oneshot;
    use windows::core::HSTRING;
    use windows::UI::Notifications::{NotificationSetting, ToastNotificationManager};

    use super::Permission;

    pub(super) fn query() -> oneshot::Receiver<Permission> {
        let (tx, rx) = oneshot::channel();
        let id = HSTRING::from(runner_core::app_paths::app_identifier(cfg!(
            debug_assertions
        )));
        let permission = ToastNotificationManager::CreateToastNotifierWithId(&id)
            .and_then(|notifier| notifier.Setting())
            .map_or(Permission::Unknown, |setting| {
                if setting == NotificationSetting::Enabled {
                    Permission::Allowed
                } else {
                    Permission::Denied
                }
            });
        let _ = tx.send(permission);
        rx
    }

    // Windows has no permission prompt.
    pub(super) fn request() -> oneshot::Receiver<()> {
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(());
        rx
    }
}

#[cfg(any(test, not(any(target_os = "macos", windows))))]
mod platform {
    use futures::channel::oneshot;

    use super::Permission;

    pub(super) fn query() -> oneshot::Receiver<Permission> {
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(Permission::Unknown);
        rx
    }

    pub(super) fn request() -> oneshot::Receiver<()> {
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(());
        rx
    }

    #[cfg(target_os = "macos")]
    pub(super) fn bundle_id() -> Option<String> {
        None
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn settings_link_names_the_running_app() {
        assert_eq!(
            macos_settings_url(Some("com.wycstudios.runner".into())),
            "x-apple.systempreferences:com.apple.Notifications-Settings.extension?id=com.wycstudios.runner"
        );
        assert_eq!(
            macos_settings_url(None),
            "x-apple.systempreferences:com.apple.Notifications-Settings.extension"
        );
    }
}
