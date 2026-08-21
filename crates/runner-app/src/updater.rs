use std::time::SystemTime;

use gpui::{App, Context, Entity, Global};

#[derive(Clone)]
pub struct GlobalUpdater(pub Entity<Updater>);

impl Global for GlobalUpdater {}

pub fn global_updater(cx: &App) -> Entity<Updater> {
    cx.global::<GlobalUpdater>().0.clone()
}

pub struct Updater {
    native: native::NativeUpdater,
}

impl Updater {
    pub fn new(automatically_checks: bool) -> Self {
        Self {
            native: native::NativeUpdater::new(automatically_checks),
        }
    }

    pub fn is_available(&self) -> bool {
        self.native.is_available()
    }

    pub fn check_for_updates(&self) {
        self.native.check_for_updates();
    }

    pub fn automatically_checks_for_updates(&self) -> bool {
        self.native.automatically_checks_for_updates()
    }

    pub fn set_automatically_checks_for_updates(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.native.set_automatically_checks_for_updates(enabled);
        cx.notify();
    }

    pub fn last_check_at(&self) -> Option<SystemTime> {
        self.native.last_check_at()
    }
}

#[cfg(all(target_os = "macos", feature = "updater"))]
mod native {
    use std::time::{Duration, SystemTime};

    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyObject, NSObject};
    use objc2::{extern_class, extern_methods, MainThreadMarker, MainThreadOnly};
    use objc2_foundation::NSDate;

    #[link(name = "Sparkle", kind = "framework")]
    unsafe extern "C" {}

    extern_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        struct SPUStandardUpdaterController;
    );

    extern_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        struct SPUUpdater;
    );

    impl SPUStandardUpdaterController {
        extern_methods!(
            #[unsafe(method(initWithStartingUpdater:updaterDelegate:userDriverDelegate:))]
            #[unsafe(method_family = init)]
            fn init_with_starting_updater(
                this: Allocated<Self>,
                start_updater: bool,
                updater_delegate: Option<&AnyObject>,
                user_driver_delegate: Option<&AnyObject>,
            ) -> Retained<Self>;

            #[unsafe(method(updater))]
            #[unsafe(method_family = none)]
            fn updater(&self) -> Retained<SPUUpdater>;

            #[unsafe(method(startUpdater))]
            #[unsafe(method_family = none)]
            fn start_updater(&self);

            #[unsafe(method(checkForUpdates:))]
            #[unsafe(method_family = none)]
            fn check_for_updates(&self, sender: Option<&AnyObject>);
        );
    }

    impl SPUUpdater {
        extern_methods!(
            #[unsafe(method(automaticallyChecksForUpdates))]
            #[unsafe(method_family = none)]
            fn automatically_checks_for_updates(&self) -> bool;

            #[unsafe(method(setAutomaticallyChecksForUpdates:))]
            #[unsafe(method_family = none)]
            fn set_automatically_checks_for_updates(&self, enabled: bool);

            #[unsafe(method(lastUpdateCheckDate))]
            #[unsafe(method_family = none)]
            fn last_update_check_date(&self) -> Option<Retained<NSDate>>;
        );
    }

    pub(super) struct NativeUpdater {
        controller: Retained<SPUStandardUpdaterController>,
    }

    impl NativeUpdater {
        pub(super) fn new(automatically_checks: bool) -> Self {
            let marker =
                MainThreadMarker::new().expect("Runner updater must start on the main thread");
            let controller = SPUStandardUpdaterController::init_with_starting_updater(
                SPUStandardUpdaterController::alloc(marker),
                false,
                None,
                None,
            );
            controller.start_updater();
            controller
                .updater()
                .set_automatically_checks_for_updates(automatically_checks);
            Self { controller }
        }

        pub(super) fn is_available(&self) -> bool {
            // SPUStandardUpdaterController aborts if startup fails, so a live controller is usable.
            true
        }

        pub(super) fn check_for_updates(&self) {
            self.controller.check_for_updates(None);
        }

        pub(super) fn automatically_checks_for_updates(&self) -> bool {
            self.controller.updater().automatically_checks_for_updates()
        }

        pub(super) fn set_automatically_checks_for_updates(&self, enabled: bool) {
            self.controller
                .updater()
                .set_automatically_checks_for_updates(enabled);
        }

        pub(super) fn last_check_at(&self) -> Option<SystemTime> {
            let seconds = self
                .controller
                .updater()
                .last_update_check_date()?
                .timeIntervalSince1970();
            (seconds >= 0.).then(|| SystemTime::UNIX_EPOCH + Duration::from_secs_f64(seconds))
        }
    }
}

#[cfg(not(all(target_os = "macos", feature = "updater")))]
mod native {
    use std::time::SystemTime;

    pub(super) struct NativeUpdater;

    impl NativeUpdater {
        pub(super) fn new(_automatically_checks: bool) -> Self {
            Self
        }

        pub(super) fn is_available(&self) -> bool {
            false
        }

        pub(super) fn check_for_updates(&self) {}

        pub(super) fn automatically_checks_for_updates(&self) -> bool {
            false
        }

        pub(super) fn set_automatically_checks_for_updates(&self, _enabled: bool) {}

        pub(super) fn last_check_at(&self) -> Option<SystemTime> {
            None
        }
    }
}

#[cfg(all(test, not(all(target_os = "macos", feature = "updater"))))]
mod tests {
    use super::*;

    #[test]
    fn development_build_has_no_updater_capability() {
        let updater = Updater::new(true);
        assert!(!updater.is_available());
        assert!(!updater.automatically_checks_for_updates());
        assert_eq!(updater.last_check_at(), None);
    }
}
