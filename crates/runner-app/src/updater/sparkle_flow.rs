//! Sparkle's flow without Sparkle: which state each callback leads to, and who answers each
//! reply block, so both can be tested without a signed bundle.

use super::{UpdateInfo, UpdateState, UpdateStep};

pub(super) const RUNNER_UPDATE_ERROR_DOMAIN: &str = "com.wycstudios.runner.updater";
const SPARKLE_ERROR_DOMAIN: &str = "SUSparkleErrorDomain";
const SU_INSTALLATION_CANCELED_ERROR: isize = 4007;

const SPU_UPDATE_CHECK_UPDATES_IN_BACKGROUND: isize = 1;

pub(super) const SPU_USER_UPDATE_CHOICE_INSTALL: isize = 1;
pub(super) const SPU_USER_UPDATE_CHOICE_DISMISS: isize = 2;

const SPU_USER_UPDATE_STAGE_INSTALLING: isize = 2;

/// Background checks only record the update; nothing downloads until the person asks.
pub(super) fn should_proceed(update_check: isize) -> bool {
    update_check != SPU_UPDATE_CHECK_UPDATES_IN_BACKGROUND
}

pub(super) fn event_for_abort(error_domain: &str, error_code: isize) -> Option<SparkleEvent> {
    let canceled =
        error_domain == SPARKLE_ERROR_DOMAIN && error_code == SU_INSTALLATION_CANCELED_ERROR;
    (error_domain != RUNNER_UPDATE_ERROR_DOMAIN && !canceled).then_some(SparkleEvent::Aborted)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FoundUpdate {
    pub(super) info: UpdateInfo,
    pub(super) size: u64,
    pub(super) manual_only: bool,
}

impl FoundUpdate {
    fn available(&self) -> UpdateState {
        UpdateState::Available {
            info: self.info.clone(),
            size: self.size,
            manual_only: self.manual_only,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum SparkleEvent {
    Found(FoundUpdate),
    NotFound,
    Aborted,
    CheckStarted,
    DownloadRequested,
    ContentLength(u64),
    Received(u64),
    Extracting,
    Ready,
    Installing,
    Error(String),
    Dismissed,
}

/// Returns whether the state changed.
pub(super) fn apply_event(
    state: &mut UpdateState,
    found: &mut Option<FoundUpdate>,
    event: SparkleEvent,
) -> bool {
    let before = state.clone();
    let settled = |found: &Option<FoundUpdate>| {
        found.as_ref().map_or(
            UpdateState::UpToDate { checking: false },
            FoundUpdate::available,
        )
    };
    match event {
        SparkleEvent::Found(update) => {
            match state {
                UpdateState::UpToDate { .. }
                | UpdateState::Available { .. }
                | UpdateState::Failed { .. } => *state = update.available(),
                UpdateState::Downloading { info, .. }
                | UpdateState::Verifying { info }
                | UpdateState::Ready { info, .. }
                | UpdateState::Installing { info } => info.clone_from(&update.info),
            }
            *found = Some(update);
        }
        SparkleEvent::NotFound => {
            *found = None;
            *state = UpdateState::UpToDate { checking: false };
        }
        SparkleEvent::Aborted => {
            // An error Sparkle reported arrives here too, and stays on screen until the retry.
            if !matches!(state, UpdateState::Failed { .. }) {
                *found = None;
                *state = UpdateState::UpToDate { checking: false };
            }
        }
        SparkleEvent::CheckStarted => {
            if matches!(state, UpdateState::UpToDate { .. }) {
                *state = UpdateState::UpToDate { checking: true };
            }
        }
        SparkleEvent::DownloadRequested => {
            if let Some(update) = found.as_ref().filter(|update| !update.manual_only) {
                *state = UpdateState::Downloading {
                    info: update.info.clone(),
                    received: 0,
                    total: 0,
                };
            }
        }
        SparkleEvent::ContentLength(length) => {
            if let UpdateState::Downloading { total, .. } = state {
                *total = length;
            }
        }
        SparkleEvent::Received(bytes) => {
            if let UpdateState::Downloading { received, .. } = state {
                *received = bytes;
            }
        }
        SparkleEvent::Extracting => {
            if let Some(update) = found.as_ref() {
                *state = UpdateState::Verifying {
                    info: update.info.clone(),
                };
            }
        }
        SparkleEvent::Ready => {
            if let Some(update) = found.as_ref() {
                *state = UpdateState::Ready {
                    info: update.info.clone(),
                    size: update.size,
                };
            }
        }
        SparkleEvent::Installing => {
            if let Some(update) = found.as_ref() {
                *state = UpdateState::Installing {
                    info: update.info.clone(),
                };
            }
        }
        SparkleEvent::Error(message) => {
            let step = match state {
                UpdateState::Downloading { .. } => UpdateStep::Download,
                UpdateState::Verifying { .. } => UpdateStep::Verify,
                UpdateState::Ready { .. } | UpdateState::Installing { .. } => UpdateStep::Install,
                _ => UpdateStep::Check,
            };
            *state = UpdateState::Failed {
                step,
                message,
                info: found.as_ref().map(|update| update.info.clone()),
            };
        }
        SparkleEvent::Dismissed => {
            if state.in_progress() || matches!(state, UpdateState::UpToDate { checking: true }) {
                *state = settled(found);
            }
        }
    }
    *state != before
}

pub(super) type ChoiceReply = Box<dyn FnOnce(isize)>;
pub(super) type Signal = Box<dyn FnOnce()>;

/// Holds Sparkle's blocks between its callbacks and the person's choice. Boxed `FnOnce`s can be
/// answered at most once, and each slot hands back what it replaces so nothing goes unanswered.
#[derive(Default)]
pub(super) struct SparkleFlow {
    download_requested: bool,
    queued_check: bool,
    ready_reply: Option<ChoiceReply>,
    cancel_download: Option<Signal>,
}

impl SparkleFlow {
    /// Returns true when the check can start now. Otherwise it starts when the session ends.
    pub(super) fn request_check(&mut self, can_check: bool) -> bool {
        self.queued_check = !can_check;
        can_check
    }

    pub(super) fn request_download(&mut self, can_check: bool) -> bool {
        self.download_requested = true;
        self.request_check(can_check)
    }

    /// Answers `showUpdateFoundWithAppcastItem:state:reply:`, returning the reply to send.
    pub(super) fn update_found(
        &mut self,
        stage: isize,
        information_only: bool,
        reply: ChoiceReply,
    ) -> Option<(ChoiceReply, isize)> {
        if information_only {
            self.download_requested = false;
            return Some((reply, SPU_USER_UPDATE_CHOICE_DISMISS));
        }
        // Replying Install at this stage quits and installs at once, so it waits for Restart.
        if stage == SPU_USER_UPDATE_STAGE_INSTALLING {
            self.download_requested = false;
            return self.hold_ready_reply(reply);
        }
        let choice = if std::mem::take(&mut self.download_requested) {
            SPU_USER_UPDATE_CHOICE_INSTALL
        } else {
            SPU_USER_UPDATE_CHOICE_DISMISS
        };
        Some((reply, choice))
    }

    pub(super) fn download_started(&mut self, cancel: Signal) {
        self.cancel_download = Some(cancel);
    }

    pub(super) fn extraction_started(&mut self) {
        self.cancel_download = None;
    }

    /// Holds `showReadyToInstallAndRelaunch:`'s reply until Restart to update.
    pub(super) fn hold_ready_reply(&mut self, reply: ChoiceReply) -> Option<(ChoiceReply, isize)> {
        self.ready_reply
            .replace(reply)
            .map(|previous| (previous, SPU_USER_UPDATE_CHOICE_DISMISS))
    }

    pub(super) fn holds_ready_reply(&self) -> bool {
        self.ready_reply.is_some()
    }

    pub(super) fn take_ready_reply(&mut self) -> Option<ChoiceReply> {
        self.ready_reply.take()
    }

    /// The block to run for Cancel, or nothing while the check runs: its reply then says Dismiss.
    pub(super) fn cancel(&mut self) -> Option<Signal> {
        self.download_requested = false;
        self.queued_check = false;
        self.cancel_download.take()
    }

    /// `dismissUpdateInstallation`: the session is over, so anything still held is answered.
    pub(super) fn dismissed(&mut self) -> Option<ChoiceReply> {
        self.cancel_download = None;
        self.ready_reply.take()
    }

    /// Returns true when a queued check should start now that the session has ended.
    pub(super) fn cycle_finished(&mut self) -> bool {
        if std::mem::take(&mut self.queued_check) {
            return true;
        }
        self.download_requested = false;
        false
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    const NOT_DOWNLOADED: isize = 0;
    const DOWNLOADED: isize = 1;

    fn update(version: &str) -> FoundUpdate {
        FoundUpdate {
            info: UpdateInfo::new(version),
            size: 40_400_000,
            manual_only: false,
        }
    }

    fn up_to_date() -> UpdateState {
        UpdateState::UpToDate { checking: false }
    }

    #[derive(Clone, Default)]
    struct Replies(Rc<RefCell<Vec<isize>>>);

    impl Replies {
        fn block(&self) -> ChoiceReply {
            let replies = self.0.clone();
            Box::new(move |choice| replies.borrow_mut().push(choice))
        }

        fn sent(&self) -> Vec<isize> {
            self.0.borrow().clone()
        }
    }

    fn send(reply: Option<(ChoiceReply, isize)>) {
        if let Some((reply, choice)) = reply {
            reply(choice);
        }
    }

    #[test]
    fn found_update_is_available_with_its_size() {
        let (mut state, mut found) = (up_to_date(), None);
        assert!(apply_event(
            &mut state,
            &mut found,
            SparkleEvent::Found(update("0.6.1"))
        ));
        assert_eq!(
            state,
            UpdateState::Available {
                info: UpdateInfo::new("0.6.1"),
                size: 40_400_000,
                manual_only: false,
            }
        );
    }

    #[test]
    fn repeated_found_event_changes_nothing() {
        let (mut state, mut found) = (up_to_date(), None);
        apply_event(&mut state, &mut found, SparkleEvent::Found(update("0.6.1")));
        assert!(!apply_event(
            &mut state,
            &mut found,
            SparkleEvent::Found(update("0.6.1"))
        ));
    }

    #[test]
    fn information_only_update_is_manual_download_only() {
        let (mut state, mut found) = (up_to_date(), None);
        let mut only = update("0.6.1");
        only.manual_only = true;
        apply_event(&mut state, &mut found, SparkleEvent::Found(only));
        assert!(matches!(
            state,
            UpdateState::Available {
                manual_only: true,
                ..
            }
        ));
        apply_event(&mut state, &mut found, SparkleEvent::DownloadRequested);
        assert!(matches!(state, UpdateState::Available { .. }));
    }

    #[test]
    fn not_found_and_abort_clear_an_available_update() {
        for event in [SparkleEvent::NotFound, SparkleEvent::Aborted] {
            let (mut state, mut found) = (up_to_date(), None);
            apply_event(&mut state, &mut found, SparkleEvent::Found(update("0.6.1")));
            apply_event(&mut state, &mut found, event);
            assert_eq!(state, up_to_date());
            assert_eq!(found, None);
        }
    }

    #[test]
    fn only_background_update_checks_are_declined() {
        assert!(should_proceed(0));
        assert!(!should_proceed(1));
        assert!(should_proceed(2));
    }

    #[test]
    fn runner_decline_and_installation_cancel_preserve_the_update() {
        assert_eq!(event_for_abort(RUNNER_UPDATE_ERROR_DOMAIN, 1), None);
        assert_eq!(
            event_for_abort(SPARKLE_ERROR_DOMAIN, SU_INSTALLATION_CANCELED_ERROR),
            None
        );
        assert_eq!(
            event_for_abort("com.example.other", SU_INSTALLATION_CANCELED_ERROR),
            Some(SparkleEvent::Aborted)
        );
        assert_eq!(
            event_for_abort(SPARKLE_ERROR_DOMAIN, 2001),
            Some(SparkleEvent::Aborted)
        );
    }

    #[test]
    fn a_download_runs_from_request_to_ready() {
        let info = UpdateInfo::new("0.6.1");
        let (mut state, mut found) = (up_to_date(), None);
        apply_event(&mut state, &mut found, SparkleEvent::Found(update("0.6.1")));
        apply_event(&mut state, &mut found, SparkleEvent::DownloadRequested);
        assert_eq!(
            state,
            UpdateState::Downloading {
                info: info.clone(),
                received: 0,
                total: 0,
            }
        );
        apply_event(&mut state, &mut found, SparkleEvent::ContentLength(100));
        apply_event(&mut state, &mut found, SparkleEvent::Received(45));
        assert_eq!(
            state,
            UpdateState::Downloading {
                info: info.clone(),
                received: 45,
                total: 100,
            }
        );
        // The check behind Download reports the same update again.
        apply_event(&mut state, &mut found, SparkleEvent::Found(update("0.6.1")));
        assert!(matches!(
            state,
            UpdateState::Downloading { received: 45, .. }
        ));
        apply_event(&mut state, &mut found, SparkleEvent::Extracting);
        assert_eq!(state, UpdateState::Verifying { info: info.clone() });
        apply_event(&mut state, &mut found, SparkleEvent::Ready);
        assert_eq!(
            state,
            UpdateState::Ready {
                info: info.clone(),
                size: 40_400_000,
            }
        );
        apply_event(&mut state, &mut found, SparkleEvent::Installing);
        assert_eq!(state, UpdateState::Installing { info });
    }

    #[test]
    fn cancelling_returns_to_available() {
        let (mut state, mut found) = (up_to_date(), None);
        apply_event(&mut state, &mut found, SparkleEvent::Found(update("0.6.1")));
        apply_event(&mut state, &mut found, SparkleEvent::DownloadRequested);
        apply_event(&mut state, &mut found, SparkleEvent::Dismissed);
        assert!(matches!(state, UpdateState::Available { .. }));
    }

    #[test]
    fn a_failure_stays_through_sparkles_dismiss_and_abort_until_retry() {
        let (mut state, mut found) = (up_to_date(), None);
        apply_event(&mut state, &mut found, SparkleEvent::Found(update("0.6.1")));
        apply_event(&mut state, &mut found, SparkleEvent::DownloadRequested);
        apply_event(
            &mut state,
            &mut found,
            SparkleEvent::Error("lost".to_owned()),
        );
        let failed = UpdateState::Failed {
            step: UpdateStep::Download,
            message: "lost".into(),
            info: Some(UpdateInfo::new("0.6.1")),
        };
        assert_eq!(state, failed);
        apply_event(&mut state, &mut found, SparkleEvent::Dismissed);
        apply_event(&mut state, &mut found, SparkleEvent::Aborted);
        assert_eq!(state, failed);
        apply_event(&mut state, &mut found, SparkleEvent::DownloadRequested);
        assert!(matches!(state, UpdateState::Downloading { .. }));
    }

    #[test]
    fn a_failed_manual_check_with_nothing_found_has_no_version() {
        let (mut state, mut found) = (up_to_date(), None);
        apply_event(&mut state, &mut found, SparkleEvent::CheckStarted);
        assert_eq!(state, UpdateState::UpToDate { checking: true });
        apply_event(
            &mut state,
            &mut found,
            SparkleEvent::Error("offline".into()),
        );
        assert_eq!(
            state,
            UpdateState::Failed {
                step: UpdateStep::Check,
                message: "offline".into(),
                info: None,
            }
        );
    }

    #[test]
    fn a_manual_check_settles_when_the_session_ends_without_a_result() {
        let (mut state, mut found) = (up_to_date(), None);
        apply_event(&mut state, &mut found, SparkleEvent::CheckStarted);
        apply_event(&mut state, &mut found, SparkleEvent::Dismissed);
        assert_eq!(state, up_to_date());
    }

    #[test]
    fn a_found_update_is_dismissed_unless_the_person_asked_to_download_it() {
        let mut flow = SparkleFlow::default();
        let replies = Replies::default();
        send(flow.update_found(NOT_DOWNLOADED, false, replies.block()));
        assert!(flow.request_download(true));
        send(flow.update_found(NOT_DOWNLOADED, false, replies.block()));
        send(flow.update_found(NOT_DOWNLOADED, false, replies.block()));
        assert_eq!(
            replies.sent(),
            [
                SPU_USER_UPDATE_CHOICE_DISMISS,
                SPU_USER_UPDATE_CHOICE_INSTALL,
                SPU_USER_UPDATE_CHOICE_DISMISS,
            ]
        );
    }

    #[test]
    fn a_downloaded_update_waits_for_the_person_like_any_other() {
        let mut flow = SparkleFlow::default();
        let replies = Replies::default();
        send(flow.update_found(DOWNLOADED, false, replies.block()));
        assert_eq!(replies.sent(), [SPU_USER_UPDATE_CHOICE_DISMISS]);
    }

    #[test]
    fn an_information_only_update_is_never_installed() {
        let mut flow = SparkleFlow::default();
        let replies = Replies::default();
        flow.request_download(true);
        send(flow.update_found(NOT_DOWNLOADED, true, replies.block()));
        assert_eq!(replies.sent(), [SPU_USER_UPDATE_CHOICE_DISMISS]);
        send(flow.update_found(NOT_DOWNLOADED, false, replies.block()));
        assert_eq!(
            replies.sent(),
            [
                SPU_USER_UPDATE_CHOICE_DISMISS,
                SPU_USER_UPDATE_CHOICE_DISMISS
            ]
        );
    }

    #[test]
    fn an_update_already_installing_waits_for_restart() {
        let mut flow = SparkleFlow::default();
        let replies = Replies::default();
        flow.request_download(true);
        send(flow.update_found(SPU_USER_UPDATE_STAGE_INSTALLING, false, replies.block()));
        assert!(replies.sent().is_empty());
        flow.take_ready_reply().unwrap()(SPU_USER_UPDATE_CHOICE_INSTALL);
        assert_eq!(replies.sent(), [SPU_USER_UPDATE_CHOICE_INSTALL]);
        assert!(flow.take_ready_reply().is_none());
    }

    #[test]
    fn the_ready_reply_waits_for_restart_and_is_answered_once() {
        let mut flow = SparkleFlow::default();
        let replies = Replies::default();
        send(flow.hold_ready_reply(replies.block()));
        assert!(replies.sent().is_empty());
        assert!(flow.holds_ready_reply());
        flow.take_ready_reply().unwrap()(SPU_USER_UPDATE_CHOICE_INSTALL);
        assert!(!flow.holds_ready_reply());
        assert!(flow.take_ready_reply().is_none());
        assert!(flow.dismissed().is_none());
        assert_eq!(replies.sent(), [SPU_USER_UPDATE_CHOICE_INSTALL]);
    }

    #[test]
    fn a_held_reply_is_answered_when_the_session_ends_or_it_is_replaced() {
        let mut flow = SparkleFlow::default();
        let replies = Replies::default();
        send(flow.hold_ready_reply(replies.block()));
        flow.dismissed().unwrap()(SPU_USER_UPDATE_CHOICE_DISMISS);
        assert_eq!(replies.sent(), [SPU_USER_UPDATE_CHOICE_DISMISS]);

        send(flow.hold_ready_reply(replies.block()));
        send(flow.hold_ready_reply(replies.block()));
        assert_eq!(
            replies.sent(),
            [
                SPU_USER_UPDATE_CHOICE_DISMISS,
                SPU_USER_UPDATE_CHOICE_DISMISS
            ]
        );
        assert!(flow.take_ready_reply().is_some());
        assert!(flow.take_ready_reply().is_none());
    }

    #[test]
    fn cancel_runs_the_download_block_once_and_only_before_extraction() {
        let cancels = Rc::new(RefCell::new(0));
        let signal = || -> Signal {
            let cancels = cancels.clone();
            Box::new(move || *cancels.borrow_mut() += 1)
        };
        let mut flow = SparkleFlow::default();
        flow.download_started(signal());
        flow.cancel().unwrap()();
        assert!(flow.cancel().is_none());
        assert_eq!(*cancels.borrow(), 1);

        flow.download_started(signal());
        flow.extraction_started();
        assert!(flow.cancel().is_none());
        assert_eq!(*cancels.borrow(), 1);
    }

    #[test]
    fn cancel_during_the_check_makes_the_found_reply_dismiss() {
        let mut flow = SparkleFlow::default();
        let replies = Replies::default();
        flow.request_download(true);
        assert!(flow.cancel().is_none());
        send(flow.update_found(NOT_DOWNLOADED, false, replies.block()));
        assert_eq!(replies.sent(), [SPU_USER_UPDATE_CHOICE_DISMISS]);
    }

    #[test]
    fn a_check_during_a_session_starts_when_the_session_ends() {
        let mut flow = SparkleFlow::default();
        let replies = Replies::default();
        assert!(!flow.request_download(false));
        assert!(flow.cycle_finished());
        send(flow.update_found(NOT_DOWNLOADED, false, replies.block()));
        assert_eq!(replies.sent(), [SPU_USER_UPDATE_CHOICE_INSTALL]);
        assert!(!flow.cycle_finished());
    }

    #[test]
    fn a_session_that_ends_without_finding_anything_drops_the_download_request() {
        let mut flow = SparkleFlow::default();
        let replies = Replies::default();
        assert!(flow.request_download(true));
        assert!(!flow.cycle_finished());
        send(flow.update_found(NOT_DOWNLOADED, false, replies.block()));
        assert_eq!(replies.sent(), [SPU_USER_UPDATE_CHOICE_DISMISS]);
    }
}
