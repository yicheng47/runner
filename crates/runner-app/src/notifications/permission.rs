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

pub(super) fn query(executor: &gpui::BackgroundExecutor) -> oneshot::Receiver<Permission> {
    platform::query(executor)
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

    pub(super) fn query(_executor: &gpui::BackgroundExecutor) -> oneshot::Receiver<Permission> {
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

#[cfg(windows)]
mod windows_toast {
    use futures::channel::oneshot;
    use windows::core::HSTRING;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
    use windows::UI::Notifications::{NotificationSetting, ToastNotificationManager};

    use super::Permission;

    /// Initializes the current worker thread as an MTA for out-of-proc WinRT
    /// calls so `ToastNotificationManager` never runs on GPUI's OLE STA UI
    /// thread (where `combase!CCliModalLoop` re-entrantly pumps messages, eats
    /// queued `WM_SYSCOMMAND(SC_RESTORE)` messages, and rings `MessageBeep`).
    struct MtaGuard {
        should_uninitialize: bool,
    }

    impl MtaGuard {
        fn enter() -> Self {
            // `S_OK` / `S_FALSE` increment the thread's COM init count;
            // `RPC_E_CHANGED_MODE` means COM is already initialized on this
            // thread in another apartment and must not be paired with
            // `CoUninitialize`.
            let should_uninitialize = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
            Self {
                should_uninitialize,
            }
        }
    }

    impl Drop for MtaGuard {
        fn drop(&mut self) {
            if self.should_uninitialize {
                unsafe { CoUninitialize() };
            }
        }
    }

    fn query_toast_permission(app_id: &str) -> Permission {
        let _com = MtaGuard::enter();
        let id = HSTRING::from(app_id);
        ToastNotificationManager::CreateToastNotifierWithId(&id)
            .and_then(|notifier| notifier.Setting())
            .map_or(Permission::Unknown, |setting| {
                if setting == NotificationSetting::Enabled {
                    Permission::Allowed
                } else {
                    Permission::Denied
                }
            })
    }

    pub(super) fn query_on(
        executor: &gpui::BackgroundExecutor,
        app_id: String,
    ) -> oneshot::Receiver<Permission> {
        let (tx, rx) = oneshot::channel();
        executor
            .spawn(async move {
                let _ = tx.send(query_toast_permission(&app_id));
            })
            .detach();
        rx
    }
}

#[cfg(all(windows, not(test)))]
mod platform {
    use futures::channel::oneshot;

    use super::{windows_toast, Permission};

    pub(super) fn query(executor: &gpui::BackgroundExecutor) -> oneshot::Receiver<Permission> {
        windows_toast::query_on(
            executor,
            runner_core::app_paths::app_identifier(cfg!(debug_assertions)),
        )
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

    pub(super) fn query(_executor: &gpui::BackgroundExecutor) -> oneshot::Receiver<Permission> {
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

#[cfg(all(test, windows))]
mod windows_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use gpui::Platform as _;
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, IsIconic, PeekMessageW,
        PostMessageW, RegisterClassW, ShowWindow, TranslateMessage, UnregisterClassW,
        CW_USEDEFAULT, MSG, PM_REMOVE, SC_RESTORE, SW_MINIMIZE, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE,
        WM_SYSCOMMAND, WNDCLASSW, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
    };

    use super::windows_toast;

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    fn drain_messages() {
        let mut msg = MSG::default();
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            let _ = unsafe { TranslateMessage(&msg) };
            unsafe { DispatchMessageW(&msg) };
        }
    }

    #[test]
    fn windows_permission_query_does_not_eat_queued_sc_restore_on_sta_thread() {
        static NEXT_CLASS: AtomicUsize = AtomicUsize::new(1);
        let class_id = NEXT_CLASS.fetch_add(1, Ordering::Relaxed);

        std::thread::spawn(move || {
            // `WindowsPlatform::new(true)` initializes this thread as an OLE STA
            // (matching the real UI thread) and exposes the production threadpool
            // `BackgroundExecutor`.
            let platform =
                gpui_windows::WindowsPlatform::new(true).expect("headless WindowsPlatform");

            let class_name = HSTRING::from(format!(
                "RunnerPermQueryStaTest_{}_{}",
                std::process::id(),
                class_id
            ));
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                lpszClassName: PCWSTR(class_name.as_ptr()),
                ..Default::default()
            };
            assert_ne!(unsafe { RegisterClassW(&wc) }, 0, "RegisterClassW failed");

            let hwnd = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    PCWSTR(class_name.as_ptr()),
                    PCWSTR(class_name.as_ptr()),
                    WS_SYSMENU | WS_THICKFRAME | WS_MAXIMIZEBOX | WS_MINIMIZEBOX,
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    320,
                    240,
                    None,
                    None,
                    None,
                    None,
                )
            }
            .expect("CreateWindowExW failed");

            let _ = unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
            drain_messages();
            let _ = unsafe { ShowWindow(hwnd, SW_MINIMIZE) };
            drain_messages();
            assert!(
                unsafe { IsIconic(hwnd) }.as_bool(),
                "window should be minimized before posting SC_RESTORE"
            );

            // Queue `WM_SYSCOMMAND(SC_RESTORE)` just as `SwitchToThisWindow`
            // does behind `WM_GPUI_TASK_DISPATCHED_ON_MAIN_THREAD` when a
            // minimized window is activated from the taskbar or Alt+Tab.
            unsafe {
                PostMessageW(
                    Some(hwnd),
                    WM_SYSCOMMAND,
                    WPARAM(SC_RESTORE as usize),
                    LPARAM(0),
                )
            }
            .expect("PostMessageW(SC_RESTORE) failed");

            let rx = windows_toast::query_on(
                &platform.background_executor(),
                runner_core::app_paths::app_identifier(true),
            );

            // If `query_on` ran WinRT on the STA thread, `combase!CCliModalLoop`
            // would already have removed and discarded `SC_RESTORE` here.
            drain_messages();
            assert!(
                !unsafe { IsIconic(hwnd) }.as_bool(),
                "queued WM_SYSCOMMAND(SC_RESTORE) must restore the minimized window"
            );

            let permission = futures::executor::block_on(rx)
                .expect("background permission query should resolve");
            assert!(matches!(
                permission,
                super::Permission::Allowed | super::Permission::Denied | super::Permission::Unknown
            ));

            let _ = unsafe { DestroyWindow(hwnd) };
            drain_messages();
            let _ = unsafe { UnregisterClassW(PCWSTR(class_name.as_ptr()), None) };
            drop(platform);
        })
        .join()
        .expect("STA test thread panicked");
    }
}
