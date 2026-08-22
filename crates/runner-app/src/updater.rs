use std::time::SystemTime;

use gpui::{App, Context, Entity, Global};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateInfo {
    version: String,
}

impl UpdateInfo {
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            version: version.into(),
        }
    }

    pub fn version(&self) -> &str {
        &self.version
    }
}

#[cfg(any(test, all(target_os = "macos", feature = "updater")))]
#[derive(Debug, Eq, PartialEq)]
enum UpdateTransition {
    Found(String),
    NotFound,
    Aborted,
    UserSkipped,
}

#[cfg(any(test, all(target_os = "macos", feature = "updater")))]
const RUNNER_UPDATE_ERROR_DOMAIN: &str = "com.wycstudios.runner.updater";

#[cfg(any(test, all(target_os = "macos", feature = "updater")))]
const SPU_UPDATE_CHECK_UPDATES_IN_BACKGROUND: isize = 1;

#[cfg(any(test, all(target_os = "macos", feature = "updater")))]
fn should_proceed(update_check: isize) -> bool {
    update_check != SPU_UPDATE_CHECK_UPDATES_IN_BACKGROUND
}

#[cfg(any(test, all(target_os = "macos", feature = "updater")))]
fn transition_for_abort(error_domain: &str) -> Option<UpdateTransition> {
    (error_domain != RUNNER_UPDATE_ERROR_DOMAIN).then_some(UpdateTransition::Aborted)
}

#[cfg(any(test, all(target_os = "macos", feature = "updater")))]
fn apply_available_transition(
    available: &mut Option<UpdateInfo>,
    transition: UpdateTransition,
) -> bool {
    let next = match transition {
        UpdateTransition::Found(version) => Some(UpdateInfo::new(version)),
        UpdateTransition::NotFound | UpdateTransition::Aborted | UpdateTransition::UserSkipped => {
            None
        }
    };
    if *available == next {
        false
    } else {
        *available = next;
        true
    }
}

#[cfg(debug_assertions)]
fn dev_available() -> Option<UpdateInfo> {
    std::env::var("RUNNER_DEV_UPDATE_AVAILABLE")
        .ok()
        .filter(|version| !version.trim().is_empty())
        .map(UpdateInfo::new)
}

#[cfg(not(debug_assertions))]
fn dev_available() -> Option<UpdateInfo> {
    None
}

#[derive(Clone)]
pub struct GlobalUpdater(pub Entity<Updater>);

impl Global for GlobalUpdater {}

pub fn global_updater(cx: &App) -> Entity<Updater> {
    cx.global::<GlobalUpdater>().0.clone()
}

pub struct Updater {
    native: native::NativeUpdater,
    available: Option<UpdateInfo>,
}

impl Updater {
    pub fn new(automatically_checks: bool, cx: &mut Context<Self>) -> Self {
        Self {
            native: native::NativeUpdater::new(
                automatically_checks,
                cx.weak_entity(),
                cx.to_async(),
            ),
            available: dev_available(),
        }
    }

    pub fn is_available(&self) -> bool {
        self.native.is_available()
    }

    pub fn start(&self) {
        self.native.start();
    }

    pub fn check_for_updates(&self) {
        self.native.check_for_updates();
    }

    pub fn available(&self) -> Option<&UpdateInfo> {
        self.available.as_ref()
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

    #[cfg(all(target_os = "macos", feature = "updater"))]
    fn apply_transition(&mut self, transition: UpdateTransition, cx: &mut Context<Self>) {
        if apply_available_transition(&mut self.available, transition) {
            cx.notify();
        }
    }
}

#[cfg(all(target_os = "macos", feature = "updater"))]
mod native {
    use std::time::{Duration, SystemTime};

    use gpui::{AsyncApp, WeakEntity};
    use objc2::ffi::NSInteger;
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyObject, Bool, NSObject};
    use objc2::{
        define_class, extern_class, extern_methods, msg_send, AnyThread, DefinedClass,
        MainThreadMarker, MainThreadOnly,
    };
    use objc2_foundation::{NSDate, NSDictionary, NSError, NSObjectProtocol, NSString};

    use super::{
        should_proceed, transition_for_abort, UpdateTransition, Updater, RUNNER_UPDATE_ERROR_DOMAIN,
    };

    const RUNNER_UPDATE_ERROR_CODE: NSInteger = 1;
    const SPU_USER_UPDATE_CHOICE_SKIP: isize = 0;

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

    extern_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        struct SUAppcastItem;
    );

    impl SUAppcastItem {
        extern_methods!(
            #[unsafe(method(displayVersionString))]
            #[unsafe(method_family = none)]
            fn display_version_string(&self) -> Retained<NSString>;

            #[unsafe(method(versionString))]
            #[unsafe(method_family = none)]
            fn version_string(&self) -> Retained<NSString>;
        );
    }

    fn update_version(item: &SUAppcastItem) -> String {
        let display_version = item.display_version_string().to_string();
        if display_version.trim().is_empty() {
            item.version_string().to_string()
        } else {
            display_version
        }
    }

    fn background_check_declined_error() -> Retained<NSError> {
        let domain = NSString::from_str(RUNNER_UPDATE_ERROR_DOMAIN);
        let description =
            NSString::from_str("Runner recorded an update during a background check.");
        let user_info =
            NSDictionary::from_slices(&[NSError::NSLocalizedDescriptionKey()], &[&*description]);
        unsafe {
            msg_send![
                NSError::alloc(),
                initWithDomain: &*domain,
                code: RUNNER_UPDATE_ERROR_CODE,
                userInfo: &*user_info
            ]
        }
    }

    struct UpdaterDelegateIvars {
        updater: WeakEntity<Updater>,
        cx: AsyncApp,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[ivars = UpdaterDelegateIvars]
        struct UpdaterDelegate;

        unsafe impl NSObjectProtocol for UpdaterDelegate {}

        impl UpdaterDelegate {
            #[unsafe(method(updater:shouldProceedWithUpdate:updateCheck:error:))]
            fn should_proceed_with_update(
                &self,
                _updater: &SPUUpdater,
                item: &SUAppcastItem,
                update_check: NSInteger,
                error: Option<&mut *mut NSError>,
            ) -> Bool {
                self.apply_transition(UpdateTransition::Found(update_version(item)));
                if should_proceed(update_check) {
                    return true.into();
                }

                if let Some(error) = error {
                    *error = Retained::autorelease_ptr(background_check_declined_error());
                }
                false.into()
            }

            #[unsafe(method(updater:didFindValidUpdate:))]
            fn did_find_valid_update(&self, _updater: &SPUUpdater, item: &SUAppcastItem) {
                self.apply_transition(UpdateTransition::Found(update_version(item)));
            }

            #[unsafe(method(updaterDidNotFindUpdate:))]
            fn did_not_find_update(&self, _updater: &SPUUpdater) {
                self.apply_transition(UpdateTransition::NotFound);
            }

            #[unsafe(method(updater:userDidMakeChoice:forUpdate:state:))]
            fn user_did_make_choice(
                &self,
                _updater: &SPUUpdater,
                choice: isize,
                _item: &SUAppcastItem,
                _state: &AnyObject,
            ) {
                if choice == SPU_USER_UPDATE_CHOICE_SKIP {
                    self.apply_transition(UpdateTransition::UserSkipped);
                }
            }

            #[unsafe(method(updater:didAbortWithError:))]
            fn did_abort_with_error(&self, _updater: &SPUUpdater, error: &NSError) {
                if let Some(transition) = transition_for_abort(&error.domain().to_string()) {
                    self.apply_transition(transition);
                }
            }
        }
    );

    impl UpdaterDelegate {
        fn new(
            marker: MainThreadMarker,
            updater: WeakEntity<Updater>,
            cx: AsyncApp,
        ) -> Retained<Self> {
            let this = Self::alloc(marker).set_ivars(UpdaterDelegateIvars { updater, cx });
            unsafe { msg_send![super(this), init] }
        }

        fn apply_transition(&self, transition: UpdateTransition) {
            let _main_thread =
                MainThreadMarker::new().expect("Sparkle updater delegate must run on main thread");
            let updater = self.ivars().updater.clone();
            self.ivars()
                .cx
                .spawn(async move |cx| {
                    let _ = cx.update(|cx| {
                        updater.update(cx, |updater, cx| {
                            updater.apply_transition(transition, cx);
                        })
                    });
                })
                .detach();
        }
    }

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
            #[unsafe(method(checkForUpdatesInBackground))]
            #[unsafe(method_family = none)]
            fn check_for_updates_in_background(&self);

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
        _delegate: Retained<UpdaterDelegate>,
        automatically_checks: bool,
    }

    impl NativeUpdater {
        pub(super) fn new(
            automatically_checks: bool,
            updater: WeakEntity<Updater>,
            cx: AsyncApp,
        ) -> Self {
            let marker =
                MainThreadMarker::new().expect("Runner updater must start on the main thread");
            let delegate = UpdaterDelegate::new(marker, updater, cx);
            let controller = SPUStandardUpdaterController::init_with_starting_updater(
                SPUStandardUpdaterController::alloc(marker),
                false,
                Some(&delegate),
                None,
            );
            Self {
                controller,
                _delegate: delegate,
                automatically_checks,
            }
        }

        pub(super) fn start(&self) {
            self.controller.start_updater();
            let updater = self.controller.updater();
            updater.set_automatically_checks_for_updates(self.automatically_checks);
            if updater.automatically_checks_for_updates() {
                updater.check_for_updates_in_background();
            }
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

    use gpui::{AsyncApp, WeakEntity};

    use super::Updater;

    pub(super) struct NativeUpdater;

    impl NativeUpdater {
        pub(super) fn new(
            _automatically_checks: bool,
            _updater: WeakEntity<Updater>,
            _cx: AsyncApp,
        ) -> Self {
            Self
        }

        pub(super) fn start(&self) {}

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

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(all(target_os = "macos", feature = "updater")))]
    use gpui::AppContext as _;

    #[test]
    fn found_update_sets_available_version() {
        let mut available = None;
        apply_available_transition(&mut available, UpdateTransition::Found("0.6.1".into()));

        assert_eq!(available, Some(UpdateInfo::new("0.6.1")));
    }

    #[test]
    fn duplicate_found_update_does_not_change_available_state() {
        let mut available = Some(UpdateInfo::new("0.6.1"));

        assert!(!apply_available_transition(
            &mut available,
            UpdateTransition::Found("0.6.1".into()),
        ));
    }

    #[test]
    fn not_found_clears_available_update() {
        let mut available = Some(UpdateInfo::new("0.6.1"));
        apply_available_transition(&mut available, UpdateTransition::NotFound);

        assert_eq!(available, None);
    }

    #[test]
    fn abort_clears_available_update() {
        let mut available = Some(UpdateInfo::new("0.6.1"));
        apply_available_transition(&mut available, UpdateTransition::Aborted);

        assert_eq!(available, None);
    }

    #[test]
    fn only_background_update_checks_are_declined() {
        assert!(should_proceed(0));
        assert!(!should_proceed(1));
        assert!(should_proceed(2));
    }

    #[test]
    fn runner_decline_abort_preserves_available_update() {
        let mut available = Some(UpdateInfo::new("0.6.1"));

        if let Some(transition) = transition_for_abort(RUNNER_UPDATE_ERROR_DOMAIN) {
            apply_available_transition(&mut available, transition);
        }
        assert_eq!(available, Some(UpdateInfo::new("0.6.1")));

        if let Some(transition) = transition_for_abort("com.example.other") {
            apply_available_transition(&mut available, transition);
        }
        assert_eq!(available, None);
    }

    #[test]
    fn runner_decline_does_not_renotify_same_found_update() {
        let mut available = None;
        assert!(apply_available_transition(
            &mut available,
            UpdateTransition::Found("0.6.1".into()),
        ));

        assert_eq!(transition_for_abort(RUNNER_UPDATE_ERROR_DOMAIN), None);
        assert!(!apply_available_transition(
            &mut available,
            UpdateTransition::Found("0.6.1".into()),
        ));
        assert_eq!(available, Some(UpdateInfo::new("0.6.1")));
    }

    #[test]
    fn user_skip_clears_available_update() {
        let mut available = Some(UpdateInfo::new("0.6.1"));
        apply_available_transition(&mut available, UpdateTransition::UserSkipped);

        assert_eq!(available, None);
    }

    #[cfg(not(all(target_os = "macos", feature = "updater")))]
    #[test]
    fn development_build_has_no_updater_capability() {
        let cx = gpui::TestAppContext::single();
        cx.update(|cx| {
            let updater = cx.new(|cx| Updater::new(true, cx));
            let updater = updater.read(cx);
            assert!(!updater.is_available());
            assert!(!updater.automatically_checks_for_updates());
            assert_eq!(updater.last_check_at(), None);
        });
    }
}
