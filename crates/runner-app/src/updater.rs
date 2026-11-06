use std::path::Path;
use std::time::{Duration, SystemTime};

use gpui::{App, Context, Entity, Global, Task};

#[cfg(windows)]
#[path = "updater/windows.rs"]
mod native;

#[cfg(target_os = "linux")]
#[path = "updater/linux.rs"]
pub mod linux;
#[cfg(target_os = "linux")]
use linux as native;

#[cfg(all(target_os = "macos", feature = "updater"))]
#[path = "updater/sparkle.rs"]
mod native;

#[cfg(any(test, all(target_os = "macos", feature = "updater")))]
#[path = "updater/sparkle_flow.rs"]
mod sparkle_flow;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateInfo {
    version: String,
}

impl UpdateInfo {
    pub fn new(version: impl Into<String>) -> Self {
        Self::on_channel(version.into(), option_env!("RUNNER_RELEASE_CHANNEL"))
    }

    fn on_channel(version: String, channel: Option<&str>) -> Self {
        let commit = |prefix: &str| {
            version
                .strip_prefix(prefix)
                .and_then(|rest| rest.split_once('.'))
                .map(|(sha, _)| sha)
        };
        // Sparkle reports only the commit, so it takes the label of the feed this build reads.
        let feed_label = match channel {
            Some("preview-nightly") => "Preview Nightly",
            _ => "Nightly",
        };
        let label = commit("nightly.")
            .map(|sha| format!("Nightly ({sha})"))
            .or_else(|| commit("preview-nightly.").map(|sha| format!("Preview Nightly ({sha})")))
            .or_else(|| {
                (version.len() == 7 && version.bytes().all(|byte| byte.is_ascii_hexdigit()))
                    .then(|| format!("{feed_label} ({version})"))
            });
        Self {
            version: label.unwrap_or(version),
        }
    }

    pub fn version(&self) -> &str {
        &self.version
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdateStep {
    Check,
    Download,
    Verify,
    Install,
}

/// What every platform's update engine reports to the UI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateState {
    UpToDate {
        checking: bool,
    },
    Available {
        info: UpdateInfo,
        size: u64,
        manual_only: bool,
    },
    Downloading {
        info: UpdateInfo,
        received: u64,
        total: u64,
    },
    Verifying {
        info: UpdateInfo,
    },
    Ready {
        info: UpdateInfo,
        size: u64,
    },
    Installing {
        info: UpdateInfo,
    },
    Failed {
        step: UpdateStep,
        message: String,
        info: Option<UpdateInfo>,
    },
}

impl UpdateState {
    pub fn info(&self) -> Option<&UpdateInfo> {
        match self {
            Self::UpToDate { .. } => None,
            Self::Available { info, .. }
            | Self::Downloading { info, .. }
            | Self::Verifying { info }
            | Self::Ready { info, .. }
            | Self::Installing { info } => Some(info),
            Self::Failed { info, .. } => info.as_ref(),
        }
    }

    fn in_progress(&self) -> bool {
        matches!(
            self,
            Self::Downloading { .. }
                | Self::Verifying { .. }
                | Self::Ready { .. }
                | Self::Installing { .. }
        )
    }
}

fn release_urls(channel: Option<&str>) -> (&'static str, &'static str) {
    match channel {
        Some("production") => (
            "https://api.github.com/repos/yicheng47/runner/releases/latest",
            "https://github.com/yicheng47/runner/releases/latest",
        ),
        Some("preview-nightly") => (
            "https://api.github.com/repos/yicheng47/runner/releases/tags/preview-nightly",
            "https://github.com/yicheng47/runner/releases/tag/preview-nightly",
        ),
        _ => (
            "https://api.github.com/repos/yicheng47/runner/releases/tags/nightly",
            "https://github.com/yicheng47/runner/releases/tag/nightly",
        ),
    }
}

/// Where What's new and View downloads go, on every platform.
pub fn release_page_url() -> &'static str {
    release_urls(option_env!("RUNNER_RELEASE_CHANNEL")).1
}

/// `RUNNER_DEV_UPDATE_AVAILABLE=<version>[:<state>]`, for looking at each state in a
/// development build. Nothing is downloaded, staged or installed.
fn parse_dev_preview(value: &str) -> Option<UpdateState> {
    let (version, state) = value.split_once(':').unwrap_or((value, "available"));
    let version = version.trim();
    if version.is_empty() {
        return None;
    }
    let info = UpdateInfo::new(version);
    Some(match state.trim() {
        "available" => UpdateState::Available {
            info,
            size: PREVIEW_SIZE,
            manual_only: false,
        },
        "manual" => UpdateState::Available {
            info,
            size: PREVIEW_SIZE,
            manual_only: true,
        },
        "downloading" => UpdateState::Downloading {
            info,
            received: PREVIEW_SIZE * 9 / 20,
            total: PREVIEW_SIZE,
        },
        "downloading-unknown" => UpdateState::Downloading {
            info,
            received: PREVIEW_SIZE * 9 / 20,
            total: 0,
        },
        "verifying" => UpdateState::Verifying { info },
        "ready" => UpdateState::Ready {
            info,
            size: PREVIEW_SIZE,
        },
        "installing" => UpdateState::Installing { info },
        "failed" => UpdateState::Failed {
            step: UpdateStep::Download,
            message: "The network connection was lost.".into(),
            info: Some(info),
        },
        "uptodate" => UpdateState::UpToDate { checking: false },
        _ => return None,
    })
}

const PREVIEW_SIZE: u64 = 40_400_000;

#[cfg(debug_assertions)]
fn dev_preview() -> Option<UpdateState> {
    parse_dev_preview(&std::env::var("RUNNER_DEV_UPDATE_AVAILABLE").ok()?)
}

#[cfg(not(debug_assertions))]
fn dev_preview() -> Option<UpdateState> {
    None
}

struct Preview {
    initial: UpdateState,
    last_check_at: Option<SystemTime>,
    task: Option<Task<()>>,
}

impl Preview {
    fn new(initial: UpdateState) -> Self {
        Self {
            initial,
            last_check_at: None,
            task: None,
        }
    }
}

#[derive(Clone)]
pub struct GlobalUpdater(pub Entity<Updater>);

impl Global for GlobalUpdater {}

pub fn global_updater(cx: &App) -> Entity<Updater> {
    cx.global::<GlobalUpdater>().0.clone()
}

pub struct Updater {
    native: native::NativeUpdater,
    state: UpdateState,
    skipped_version: Option<String>,
    preview: Option<Preview>,
}

impl Updater {
    fn initial_state() -> (UpdateState, Option<Preview>) {
        match dev_preview() {
            Some(state) => (state.clone(), Some(Preview::new(state))),
            None => (UpdateState::UpToDate { checking: false }, None),
        }
    }

    pub fn is_available(&self) -> bool {
        self.preview.is_some() || self.native.is_available()
    }

    pub fn start(&self) {
        if self.preview.is_none() {
            self.native.start();
        }
    }

    pub fn check_for_updates(&self) {
        if self.preview.is_none() {
            self.native.check_for_updates();
        }
    }

    pub fn is_preview(&self) -> bool {
        self.preview.is_some()
    }

    pub fn state(&self) -> &UpdateState {
        &self.state
    }

    pub fn is_checking(&self) -> bool {
        matches!(self.state, UpdateState::UpToDate { checking: true }) || self.native.is_checking()
    }

    pub fn automatically_checks_for_updates(&self) -> bool {
        self.native.automatically_checks_for_updates()
    }

    pub fn set_automatically_checks_for_updates(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.native.set_automatically_checks_for_updates(enabled);
        cx.notify();
    }

    pub fn last_check_at(&self) -> Option<SystemTime> {
        match &self.preview {
            Some(preview) => preview.last_check_at,
            None => self.native.last_check_at(),
        }
    }

    pub fn skipped_version(&self) -> Option<&str> {
        self.skipped_version.as_deref()
    }

    pub fn set_skipped_version(&mut self, version: Option<String>) {
        self.skipped_version = version;
    }

    /// True when the current update is the version the person chose to skip.
    pub fn is_skipped(&self) -> bool {
        self.state
            .info()
            .is_some_and(|info| self.skipped_version.as_deref() == Some(info.version()))
    }

    /// Hides the current update until the next manual check, and returns the version to persist.
    pub fn skip_update(&mut self, cx: &mut Context<Self>) -> Option<String> {
        let version = self.state.info()?.version().to_owned();
        self.skipped_version = Some(version.clone());
        cx.notify();
        Some(version)
    }

    /// A check the person asked for, which also brings back a skipped version.
    pub fn manual_check(&mut self, cx: &mut Context<Self>) {
        self.skipped_version = None;
        if self.preview.is_some() {
            self.preview_check(cx);
        } else if !self.state.in_progress() {
            self.check_for_updates();
        }
        cx.notify();
    }

    pub fn request_download(&mut self, cx: &mut Context<Self>) {
        if self.preview.is_some() {
            self.preview_download(cx);
        } else {
            self.download_update(cx);
        }
    }

    pub fn request_cancel(&mut self, cx: &mut Context<Self>) {
        if self.preview.is_some() {
            self.preview_cancel(cx);
        } else {
            self.cancel_download(cx);
        }
    }

    /// Closes Runner and reopens it on the new version, the same way from every entry point.
    pub fn request_install(&mut self, log_dir: &Path, cx: &mut Context<Self>) {
        if self.preview.is_some() {
            self.preview_install(cx);
        } else {
            self.install_update(log_dir, cx);
        }
    }

    pub fn request_retry(&mut self, log_dir: &Path, cx: &mut Context<Self>) {
        match self.state {
            UpdateState::Failed {
                step: UpdateStep::Check,
                ..
            } => self.manual_check(cx),
            UpdateState::Failed {
                step: UpdateStep::Install,
                ..
            } if self.preview.is_none() => self.install_update(log_dir, cx),
            _ => self.request_download(cx),
        }
    }

    #[cfg(test)]
    pub(crate) fn set_state_for_test(&mut self, state: UpdateState, cx: &mut Context<Self>) {
        self.state = state;
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn set_preview_for_test(&mut self, state: UpdateState, cx: &mut Context<Self>) {
        self.preview = Some(Preview::new(state.clone()));
        self.state = state;
        cx.notify();
    }

    fn preview_check(&mut self, cx: &mut Context<Self>) {
        let Some(initial) = self.preview.as_ref().map(|preview| preview.initial.clone()) else {
            return;
        };
        if matches!(initial, UpdateState::UpToDate { .. }) {
            self.state = UpdateState::UpToDate { checking: true };
        }
        let task = cx.spawn(async move |updater, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(800))
                .await;
            let _ = updater.update(cx, |updater, cx| {
                updater.state = initial;
                if let Some(preview) = &mut updater.preview {
                    preview.last_check_at = Some(SystemTime::now());
                }
                cx.notify();
            });
        });
        if let Some(preview) = &mut self.preview {
            preview.task = Some(task);
        }
    }

    fn preview_download(&mut self, cx: &mut Context<Self>) {
        let Some(info) = self.state.info().cloned() else {
            return;
        };
        self.state = UpdateState::Downloading {
            info: info.clone(),
            received: 0,
            total: PREVIEW_SIZE,
        };
        let task = cx.spawn(async move |updater, cx| {
            for step in 1..=20 {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                let next = UpdateState::Downloading {
                    info: info.clone(),
                    received: PREVIEW_SIZE * step / 20,
                    total: PREVIEW_SIZE,
                };
                let _ = updater.update(cx, |updater, cx| {
                    updater.state = next;
                    cx.notify();
                });
            }
            let verifying = UpdateState::Verifying { info: info.clone() };
            let _ = updater.update(cx, |updater, cx| {
                updater.state = verifying;
                cx.notify();
            });
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            let _ = updater.update(cx, |updater, cx| {
                updater.state = UpdateState::Ready {
                    info,
                    size: PREVIEW_SIZE,
                };
                cx.notify();
            });
        });
        if let Some(preview) = &mut self.preview {
            preview.task = Some(task);
        }
    }

    fn preview_cancel(&mut self, cx: &mut Context<Self>) {
        let Some(info) = self.state.info().cloned() else {
            return;
        };
        if let Some(preview) = &mut self.preview {
            preview.task = None;
        }
        self.state = UpdateState::Available {
            info,
            size: PREVIEW_SIZE,
            manual_only: false,
        };
        cx.notify();
    }

    fn preview_install(&mut self, cx: &mut Context<Self>) {
        let Some(info) = self.state.info().cloned() else {
            return;
        };
        self.state = UpdateState::Installing { info: info.clone() };
        let task = cx.spawn(async move |updater, cx| {
            cx.background_executor().timer(Duration::from_secs(3)).await;
            let _ = updater.update(cx, |updater, cx| {
                updater.state = UpdateState::Ready {
                    info,
                    size: PREVIEW_SIZE,
                };
                cx.notify();
            });
        });
        if let Some(preview) = &mut self.preview {
            preview.task = Some(task);
        }
        cx.notify();
    }
}

#[cfg(target_os = "macos")]
impl Updater {
    pub fn new(automatically_checks: bool, cx: &mut Context<Self>) -> Self {
        let (state, preview) = Self::initial_state();
        Self {
            native: native::NativeUpdater::new(
                automatically_checks,
                cx.weak_entity(),
                cx.to_async(),
            ),
            state,
            skipped_version: None,
            preview,
        }
    }
}

#[cfg(not(any(
    windows,
    target_os = "linux",
    all(target_os = "macos", feature = "updater")
)))]
mod native {
    use std::time::SystemTime;

    use std::path::Path;

    use gpui::{AsyncApp, Context, WeakEntity};

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

        pub(super) fn is_checking(&self) -> bool {
            false
        }
    }

    impl Updater {
        pub(super) fn download_update(&mut self, _cx: &mut Context<Self>) {}

        pub(super) fn cancel_download(&mut self, _cx: &mut Context<Self>) {}

        pub(super) fn install_update(&mut self, _log_dir: &Path, _cx: &mut Context<Self>) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(any(
        windows,
        target_os = "linux",
        all(target_os = "macos", feature = "updater")
    )))]
    use gpui::AppContext as _;

    #[test]
    fn nightly_update_displays_its_commit_and_preserves_existing_release_labels() {
        assert_eq!(
            UpdateInfo::new("nightly.abc1234.20260908.0100").version(),
            "Nightly (abc1234)"
        );
        assert_eq!(
            UpdateInfo::on_channel("abc1234".into(), None).version(),
            "Nightly (abc1234)"
        );
        assert_eq!(
            UpdateInfo::on_channel("abc1234".into(), Some("nightly")).version(),
            "Nightly (abc1234)"
        );
        assert_eq!(
            UpdateInfo::new("preview-nightly.abc1234.20261104.0100").version(),
            "Preview Nightly (abc1234)"
        );
        assert_eq!(
            UpdateInfo::on_channel("abc1234".into(), Some("preview-nightly")).version(),
            "Preview Nightly (abc1234)"
        );
        for version in [
            "0.8.2.20260907.0100",
            "0.8.3-nightly.20260907.0100",
            "0.8.2",
        ] {
            assert_eq!(UpdateInfo::new(version).version(), version);
        }
    }

    #[test]
    fn dev_preview_reaches_every_state_and_ignores_unknown_ones() {
        let info = UpdateInfo::new("0.19.5");
        assert_eq!(
            parse_dev_preview("0.19.5"),
            Some(UpdateState::Available {
                info: info.clone(),
                size: PREVIEW_SIZE,
                manual_only: false,
            })
        );
        assert!(matches!(
            parse_dev_preview("0.19.5:manual"),
            Some(UpdateState::Available {
                manual_only: true,
                ..
            })
        ));
        assert!(matches!(
            parse_dev_preview("0.19.5:downloading"),
            Some(UpdateState::Downloading { total, .. }) if total == PREVIEW_SIZE
        ));
        assert!(matches!(
            parse_dev_preview("0.19.5:downloading-unknown"),
            Some(UpdateState::Downloading { total: 0, .. })
        ));
        assert_eq!(
            parse_dev_preview("0.19.5:verifying"),
            Some(UpdateState::Verifying { info: info.clone() })
        );
        assert_eq!(
            parse_dev_preview("0.19.5:ready"),
            Some(UpdateState::Ready {
                info: info.clone(),
                size: PREVIEW_SIZE,
            })
        );
        assert_eq!(
            parse_dev_preview("0.19.5:installing"),
            Some(UpdateState::Installing { info })
        );
        assert!(matches!(
            parse_dev_preview("0.19.5:failed"),
            Some(UpdateState::Failed {
                step: UpdateStep::Download,
                ..
            })
        ));
        assert_eq!(
            parse_dev_preview("0.19.5:uptodate"),
            Some(UpdateState::UpToDate { checking: false })
        );
        assert_eq!(parse_dev_preview(" "), None);
        assert_eq!(parse_dev_preview("0.19.5:nonsense"), None);
    }

    #[cfg(not(any(
        windows,
        target_os = "linux",
        all(target_os = "macos", feature = "updater")
    )))]
    #[test]
    fn preview_walks_the_download_and_install_states_without_an_engine() {
        let mut cx = gpui::TestAppContext::single();
        let updater = cx.new(|cx| {
            let mut updater = Updater::new(false, cx);
            let state = parse_dev_preview("0.19.5").unwrap();
            updater.preview = Some(Preview::new(state.clone()));
            updater.state = state;
            updater
        });
        let log_dir = Path::new("");
        updater.update(&mut cx, |updater, cx| updater.request_download(cx));
        assert!(matches!(
            updater.read_with(&cx, |updater, _| updater.state().clone()),
            UpdateState::Downloading { received: 0, .. }
        ));
        updater.update(&mut cx, |updater, cx| updater.request_cancel(cx));
        assert!(matches!(
            updater.read_with(&cx, |updater, _| updater.state().clone()),
            UpdateState::Available { .. }
        ));
        updater.update(&mut cx, |updater, cx| updater.request_download(cx));
        cx.executor().advance_clock(Duration::from_secs(4));
        cx.run_until_parked();
        assert!(matches!(
            updater.read_with(&cx, |updater, _| updater.state().clone()),
            UpdateState::Verifying { .. }
        ));
        cx.executor().advance_clock(Duration::from_secs(2));
        cx.run_until_parked();
        assert!(matches!(
            updater.read_with(&cx, |updater, _| updater.state().clone()),
            UpdateState::Ready { .. }
        ));
        updater.update(&mut cx, |updater, cx| {
            updater.request_install(log_dir, cx);
        });
        assert!(matches!(
            updater.read_with(&cx, |updater, _| updater.state().clone()),
            UpdateState::Installing { .. }
        ));
        assert!(!crate::lifecycle::update_quit_pending());
    }

    #[cfg(not(any(
        windows,
        target_os = "linux",
        all(target_os = "macos", feature = "updater")
    )))]
    #[test]
    fn a_skipped_version_is_only_skipped_until_the_next_manual_check() {
        let cx = gpui::TestAppContext::single();
        cx.update(|cx| {
            let updater = cx.new(|cx| {
                let mut updater = Updater::new(false, cx);
                updater.state = parse_dev_preview("0.19.5").unwrap();
                updater
            });
            updater.update(cx, |updater, cx| {
                assert!(!updater.is_skipped());
                assert_eq!(updater.skip_update(cx).as_deref(), Some("0.19.5"));
                assert!(updater.is_skipped());
                updater.state = parse_dev_preview("0.19.6").unwrap();
                assert!(!updater.is_skipped());
                updater.state = parse_dev_preview("0.19.5").unwrap();
                assert!(updater.is_skipped());
                updater.manual_check(cx);
                assert!(!updater.is_skipped());
                assert_eq!(updater.skipped_version(), None);
            });
        });
    }

    #[cfg(not(any(
        windows,
        target_os = "linux",
        all(target_os = "macos", feature = "updater")
    )))]
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
