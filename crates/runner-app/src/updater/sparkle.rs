//! Runner's `SPUUserDriver` on `SPUUpdater`: Sparkle stays the engine and shows no windows.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use block2::{DynBlock, RcBlock};
use gpui::{AsyncApp, Context, WeakEntity};
use objc2::ffi::NSInteger;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyObject, Bool, NSObject};
use objc2::{
    define_class, extern_class, extern_methods, msg_send, AnyThread, DefinedClass,
    MainThreadMarker, MainThreadOnly,
};
use objc2_foundation::{
    NSBundle, NSDate, NSDictionary, NSError, NSObjectProtocol, NSString, NSUnderlyingErrorKey,
};

use super::sparkle_flow::{
    abort_is_failure, apply_event, error_log_entry, event_for_abort, should_proceed, FoundUpdate,
    SparkleEvent, SparkleFlow, RUNNER_UPDATE_ERROR_DOMAIN, SPU_USER_UPDATE_CHOICE_DISMISS,
    SPU_USER_UPDATE_CHOICE_INSTALL,
};
use super::{UpdateInfo, UpdateState, Updater};

const RUNNER_UPDATE_ERROR_CODE: NSInteger = 1;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);
const TERMINATE_RETRY_INTERVAL: Duration = Duration::from_secs(5);
const TERMINATE_RETRIES: usize = 6;

#[link(name = "Sparkle", kind = "framework")]
unsafe extern "C" {}

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

extern_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct SPUUserUpdateState;
);

extern_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct SUUpdatePermissionResponse;
);

impl SUAppcastItem {
    extern_methods!(
        #[unsafe(method(displayVersionString))]
        #[unsafe(method_family = none)]
        fn display_version_string(&self) -> Retained<NSString>;

        #[unsafe(method(versionString))]
        #[unsafe(method_family = none)]
        fn version_string(&self) -> Retained<NSString>;

        #[unsafe(method(contentLength))]
        #[unsafe(method_family = none)]
        fn content_length(&self) -> u64;

        #[unsafe(method(isInformationOnlyUpdate))]
        #[unsafe(method_family = none)]
        fn is_information_only_update(&self) -> bool;
    );
}

impl SPUUserUpdateState {
    extern_methods!(
        #[unsafe(method(stage))]
        #[unsafe(method_family = none)]
        fn stage(&self) -> NSInteger;
    );
}

impl SUUpdatePermissionResponse {
    extern_methods!(
        #[unsafe(method(initWithAutomaticUpdateChecks:sendSystemProfile:))]
        #[unsafe(method_family = init)]
        fn init_with_checks(
            this: Allocated<Self>,
            automatic_update_checks: bool,
            send_system_profile: bool,
        ) -> Retained<Self>;
    );
}

impl SPUUpdater {
    extern_methods!(
        #[unsafe(method(initWithHostBundle:applicationBundle:userDriver:delegate:))]
        #[unsafe(method_family = init)]
        fn init_with_bundles(
            this: Allocated<Self>,
            host_bundle: &NSBundle,
            application_bundle: &NSBundle,
            user_driver: &AnyObject,
            delegate: Option<&AnyObject>,
        ) -> Retained<Self>;

        #[unsafe(method(startUpdater:_))]
        #[unsafe(method_family = none)]
        fn start_updater(&self) -> Result<(), Retained<NSError>>;

        #[unsafe(method(checkForUpdates))]
        #[unsafe(method_family = none)]
        fn check_for_updates(&self);

        #[unsafe(method(canCheckForUpdates))]
        #[unsafe(method_family = none)]
        fn can_check_for_updates(&self) -> bool;

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

fn found_update(item: &SUAppcastItem) -> FoundUpdate {
    let display_version = item.display_version_string().to_string();
    let version = if display_version.trim().is_empty() {
        item.version_string().to_string()
    } else {
        display_version
    };
    FoundUpdate {
        info: UpdateInfo::new(version),
        size: item.content_length(),
        manual_only: item.is_information_only_update(),
    }
}

fn background_check_declined_error() -> Retained<NSError> {
    let domain = NSString::from_str(RUNNER_UPDATE_ERROR_DOMAIN);
    let description = NSString::from_str(crate::i18n::text(
        "ui.runner_recorded_an_update_during_a_background_check",
    ));
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

/// The error and each one under it. `userInfo` is not logged: it holds the failing URLs.
fn describe_error(error: &NSError) -> String {
    let entry = error_log_entry(
        &error.domain().to_string(),
        error.code(),
        &error.localizedDescription().to_string(),
    );
    let underlying = error
        .userInfo()
        .objectForKey(unsafe { NSUnderlyingErrorKey })
        .and_then(|underlying| underlying.downcast::<NSError>().ok());
    match underlying {
        Some(underlying) => format!("{entry}; caused by {}", describe_error(&underlying)),
        None => entry,
    }
}

struct Shared {
    updater: WeakEntity<Updater>,
    cx: AsyncApp,
    flow: RefCell<SparkleFlow>,
    received: Cell<u64>,
    last_progress: Cell<Option<Instant>>,
}

impl Shared {
    /// Sparkle can call back while the `Updater` entity is being updated, so events wait a turn.
    fn send(&self, event: SparkleEvent) {
        let updater = self.updater.clone();
        self.cx
            .spawn(async move |cx| {
                let _ = cx.update(|cx| {
                    updater.update(cx, |updater, cx| {
                        updater.apply_sparkle_event(event, cx);
                    })
                });
            })
            .detach();
    }

    fn notify(&self) {
        let updater = self.updater.clone();
        self.cx
            .spawn(async move |cx| {
                let _ = cx.update(|cx| updater.update(cx, |_, cx| cx.notify()));
            })
            .detach();
    }
}

struct UserDriverIvars {
    shared: Rc<Shared>,
    automatically_checks: bool,
}

// Sparkle documents every SPUUserDriver method as called on the main thread.
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = UserDriverIvars]
    struct UserDriver;

    unsafe impl NSObjectProtocol for UserDriver {}

    impl UserDriver {
        #[unsafe(method(showUpdatePermissionRequest:reply:))]
        fn show_update_permission_request(
            &self,
            _request: &AnyObject,
            reply: &DynBlock<dyn Fn(*mut AnyObject)>,
        ) {
            let response = SUUpdatePermissionResponse::init_with_checks(
                SUUpdatePermissionResponse::alloc(self.mtm()),
                self.ivars().automatically_checks,
                false,
            );
            reply.call((Retained::as_ptr(&response) as *mut AnyObject,));
        }

        #[unsafe(method(showUserInitiatedUpdateCheckWithCancellation:))]
        fn show_user_initiated_update_check(&self, _cancellation: &DynBlock<dyn Fn()>) {
            self.shared().send(SparkleEvent::CheckStarted);
        }

        #[unsafe(method(showUpdateFoundWithAppcastItem:state:reply:))]
        fn show_update_found(
            &self,
            item: &SUAppcastItem,
            state: &SPUUserUpdateState,
            reply: &DynBlock<dyn Fn(NSInteger)>,
        ) {
            let shared = self.shared();
            let found = found_update(item);
            let manual_only = found.manual_only;
            shared.send(SparkleEvent::Found(found));
            let stage = state.stage();
            let reply = reply.copy();
            let answer = shared.flow.borrow_mut().update_found(
                stage,
                manual_only,
                Box::new(move |choice| reply.call((choice,))),
            );
            if let Some((reply, choice)) = answer {
                reply(choice);
            }
            if shared.flow.borrow().holds_ready_reply() {
                shared.send(SparkleEvent::Ready);
            }
        }

        #[unsafe(method(showUpdateReleaseNotesWithDownloadData:))]
        fn show_update_release_notes(&self, _download_data: &AnyObject) {}

        #[unsafe(method(showUpdateReleaseNotesFailedToDownloadWithError:))]
        fn show_update_release_notes_failed(&self, _error: &NSError) {}

        #[unsafe(method(showUpdateNotFoundWithError:acknowledgement:))]
        fn show_update_not_found(&self, _error: &NSError, acknowledgement: &DynBlock<dyn Fn()>) {
            self.shared().send(SparkleEvent::NotFound);
            acknowledgement.call(());
        }

        #[unsafe(method(showUpdaterError:acknowledgement:))]
        fn show_updater_error(&self, error: &NSError, acknowledgement: &DynBlock<dyn Fn()>) {
            if error.domain().to_string() != RUNNER_UPDATE_ERROR_DOMAIN {
                self.shared()
                    .send(SparkleEvent::Error(error.localizedDescription().to_string()));
            }
            acknowledgement.call(());
        }

        #[unsafe(method(showDownloadInitiatedWithCancellation:))]
        fn show_download_initiated(&self, cancellation: &DynBlock<dyn Fn()>) {
            let shared = self.shared();
            shared.received.set(0);
            shared.last_progress.set(None);
            let cancellation = cancellation.copy();
            shared
                .flow
                .borrow_mut()
                .download_started(Box::new(move || cancellation.call(())));
        }

        #[unsafe(method(showDownloadDidReceiveExpectedContentLength:))]
        fn show_download_expected_length(&self, length: u64) {
            self.shared().send(SparkleEvent::ContentLength(length));
        }

        #[unsafe(method(showDownloadDidReceiveDataOfLength:))]
        fn show_download_received(&self, length: u64) {
            let shared = self.shared();
            let received = shared.received.get().saturating_add(length);
            shared.received.set(received);
            let due = shared
                .last_progress
                .get()
                .is_none_or(|last| last.elapsed() >= PROGRESS_INTERVAL);
            if due {
                shared.last_progress.set(Some(Instant::now()));
                shared.send(SparkleEvent::Received(received));
            }
        }

        #[unsafe(method(showDownloadDidStartExtractingUpdate))]
        fn show_extraction_started(&self) {
            let shared = self.shared();
            shared.flow.borrow_mut().extraction_started();
            shared.send(SparkleEvent::Extracting);
        }

        #[unsafe(method(showExtractionReceivedProgress:))]
        fn show_extraction_progress(&self, _progress: f64) {}

        #[unsafe(method(showReadyToInstallAndRelaunch:))]
        fn show_ready_to_install(&self, reply: &DynBlock<dyn Fn(NSInteger)>) {
            let shared = self.shared();
            let reply = reply.copy();
            let replaced = shared
                .flow
                .borrow_mut()
                .hold_ready_reply(Box::new(move |choice| reply.call((choice,))));
            if let Some((reply, choice)) = replaced {
                reply(choice);
            }
            shared.send(SparkleEvent::Ready);
        }

        #[unsafe(method(showInstallingUpdateWithApplicationTerminated:retryTerminatingApplication:))]
        fn show_installing(
            &self,
            application_terminated: Bool,
            retry_terminating_application: &DynBlock<dyn Fn()>,
        ) {
            let shared = self.shared();
            shared.send(SparkleEvent::Installing);
            if application_terminated.as_bool() {
                return;
            }
            // Something can delay the quit, and Sparkle only asks once.
            let retry: RcBlock<dyn Fn()> = retry_terminating_application.copy();
            shared
                .cx
                .spawn(async move |cx| {
                    for _ in 0..TERMINATE_RETRIES {
                        cx.background_executor().timer(TERMINATE_RETRY_INTERVAL).await;
                        retry.call(());
                    }
                })
                .detach();
        }

        #[unsafe(method(showUpdateInstalledAndRelaunched:acknowledgement:))]
        fn show_update_installed(&self, _relaunched: Bool, acknowledgement: &DynBlock<dyn Fn()>) {
            acknowledgement.call(());
        }

        #[unsafe(method(dismissUpdateInstallation))]
        fn dismiss_update_installation(&self) {
            let shared = self.shared();
            let held = shared.flow.borrow_mut().dismissed();
            if let Some(reply) = held {
                reply(SPU_USER_UPDATE_CHOICE_DISMISS);
            }
            shared.send(SparkleEvent::Dismissed);
        }
    }
);

impl UserDriver {
    fn new(
        marker: MainThreadMarker,
        shared: Rc<Shared>,
        automatically_checks: bool,
    ) -> Retained<Self> {
        let this = Self::alloc(marker).set_ivars(UserDriverIvars {
            shared,
            automatically_checks,
        });
        unsafe { msg_send![super(this), init] }
    }

    fn shared(&self) -> &Shared {
        &self.ivars().shared
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Rc<Shared>]
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
            self.ivars().send(SparkleEvent::Found(found_update(item)));
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
            self.ivars().send(SparkleEvent::Found(found_update(item)));
        }

        // What's new opens the release page, so Sparkle has no notes to fetch.
        #[unsafe(method(updater:shouldDownloadReleaseNotesForUpdate:))]
        fn should_download_release_notes(
            &self,
            _updater: &SPUUpdater,
            _item: &SUAppcastItem,
        ) -> Bool {
            false.into()
        }

        #[unsafe(method(updaterDidNotFindUpdate:))]
        fn did_not_find_update(&self, _updater: &SPUUpdater) {
            self.ivars().send(SparkleEvent::NotFound);
        }

        #[unsafe(method(updaterWillRelaunchApplication:))]
        fn will_relaunch_application(&self, _updater: &SPUUpdater) {
            crate::lifecycle::mark_update_quit();
        }

        #[unsafe(method(updater:didAbortWithError:))]
        fn did_abort_with_error(&self, _updater: &SPUUpdater, error: &NSError) {
            let domain = error.domain().to_string();
            if abort_is_failure(&domain, error.code()) {
                tracing::warn!("Sparkle update failed: {}", describe_error(error));
            }
            if let Some(event) = event_for_abort(&domain, error.code()) {
                self.ivars().send(event);
            }
        }

        // Sparkle marks the session finished before this, so starting a check here is safe.
        #[unsafe(method(updater:didFinishUpdateCycleForUpdateCheck:error:))]
        fn did_finish_update_cycle(
            &self,
            updater: &SPUUpdater,
            _update_check: NSInteger,
            _error: Option<&NSError>,
        ) {
            let start_queued = self.ivars().flow.borrow_mut().cycle_finished();
            self.ivars().notify();
            if start_queued {
                updater.check_for_updates();
            }
        }
    }
);

impl UpdaterDelegate {
    fn new(marker: MainThreadMarker, shared: Rc<Shared>) -> Retained<Self> {
        let this = Self::alloc(marker).set_ivars(shared);
        unsafe { msg_send![super(this), init] }
    }
}

pub(super) struct NativeUpdater {
    updater: Retained<SPUUpdater>,
    shared: Rc<Shared>,
    // SPUUpdater holds its delegate weakly.
    _delegate: Retained<UpdaterDelegate>,
    _driver: Retained<UserDriver>,
    automatically_checks: bool,
    start_failed: Cell<bool>,
    found: Option<FoundUpdate>,
}

impl NativeUpdater {
    pub(super) fn new(
        automatically_checks: bool,
        updater: WeakEntity<Updater>,
        cx: AsyncApp,
    ) -> Self {
        let marker = MainThreadMarker::new().expect("Runner updater must start on the main thread");
        let shared = Rc::new(Shared {
            updater,
            cx,
            flow: RefCell::default(),
            received: Cell::new(0),
            last_progress: Cell::new(None),
        });
        let driver = UserDriver::new(marker, shared.clone(), automatically_checks);
        let delegate = UpdaterDelegate::new(marker, shared.clone());
        let bundle = NSBundle::mainBundle();
        let updater = SPUUpdater::init_with_bundles(
            SPUUpdater::alloc(marker),
            &bundle,
            &bundle,
            &driver,
            Some(&delegate),
        );
        Self {
            updater,
            shared,
            _delegate: delegate,
            _driver: driver,
            automatically_checks,
            start_failed: Cell::new(false),
            found: None,
        }
    }

    pub(super) fn start(&self) {
        if let Err(error) = self.updater.start_updater() {
            tracing::warn!("Sparkle could not start: {}", error.localizedDescription());
            self.start_failed.set(true);
            return;
        }
        self.updater
            .set_automatically_checks_for_updates(self.automatically_checks);
        if self.updater.automatically_checks_for_updates() {
            self.updater.check_for_updates_in_background();
        }
    }

    pub(super) fn is_available(&self) -> bool {
        !self.start_failed.get()
    }

    pub(super) fn is_checking(&self) -> bool {
        false
    }

    pub(super) fn check_for_updates(&self) {
        if !self.is_available() {
            return;
        }
        let can_check = self.updater.can_check_for_updates();
        if self.shared.flow.borrow_mut().request_check(can_check) {
            self.updater.check_for_updates();
        }
    }

    pub(super) fn automatically_checks_for_updates(&self) -> bool {
        self.updater.automatically_checks_for_updates()
    }

    pub(super) fn set_automatically_checks_for_updates(&self, enabled: bool) {
        self.updater.set_automatically_checks_for_updates(enabled);
    }

    pub(super) fn last_check_at(&self) -> Option<SystemTime> {
        let seconds = self
            .updater
            .last_update_check_date()?
            .timeIntervalSince1970();
        (seconds >= 0.).then(|| SystemTime::UNIX_EPOCH + Duration::from_secs_f64(seconds))
    }
}

impl Updater {
    fn apply_sparkle_event(&mut self, event: SparkleEvent, cx: &mut Context<Self>) {
        if apply_event(&mut self.state, &mut self.native.found, event) {
            cx.notify();
        }
    }

    pub(super) fn download_update(&mut self, cx: &mut Context<Self>) {
        if !self.native.is_available() || self.state.in_progress() {
            return;
        }
        self.apply_sparkle_event(SparkleEvent::DownloadRequested, cx);
        if !matches!(self.state, UpdateState::Downloading { .. }) {
            return;
        }
        let can_check = self.native.updater.can_check_for_updates();
        let start = self
            .native
            .shared
            .flow
            .borrow_mut()
            .request_download(can_check);
        if start {
            self.native.updater.check_for_updates();
        }
    }

    pub(super) fn cancel_download(&mut self, cx: &mut Context<Self>) {
        let cancel = self.native.shared.flow.borrow_mut().cancel();
        match cancel {
            Some(cancel) => cancel(),
            None => self.apply_sparkle_event(SparkleEvent::Dismissed, cx),
        }
    }

    pub(super) fn install_update(&mut self, _log_dir: &Path, cx: &mut Context<Self>) {
        let reply = self.native.shared.flow.borrow_mut().take_ready_reply();
        match reply {
            Some(reply) => {
                self.apply_sparkle_event(SparkleEvent::Installing, cx);
                reply(SPU_USER_UPDATE_CHOICE_INSTALL);
            }
            // After a failed install Sparkle has ended the session, so it starts over.
            None => self.download_update(cx),
        }
    }
}
