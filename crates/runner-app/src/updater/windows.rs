use std::cell::{Cell, RefCell};
use std::time::{Duration, SystemTime};

use anyhow::{bail, Context as _, Result};
use gpui::{AsyncApp, Context, Task, WeakEntity};
use serde::Deserialize;

use super::{UpdateInfo, Updater};

pub const WINDOWS_DOWNLOAD_URL: &str =
    "https://github.com/yicheng47/runner/releases/tag/nightly-win";
const RELEASE_API: &str = "https://api.github.com/repos/yicheng47/runner/releases/tags/nightly-win";
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

pub(super) struct NativeUpdater {
    automatically_checks: Cell<bool>,
    checking: Cell<bool>,
    last_check_at: Cell<Option<SystemTime>>,
    error: RefCell<Option<String>>,
    updater: WeakEntity<Updater>,
    cx: AsyncApp,
    poll_task: RefCell<Option<Task<()>>>,
}

impl NativeUpdater {
    pub(super) fn new(
        automatically_checks: bool,
        updater: WeakEntity<Updater>,
        cx: AsyncApp,
    ) -> Self {
        Self {
            automatically_checks: Cell::new(automatically_checks),
            checking: Cell::new(false),
            last_check_at: Cell::new(None),
            error: RefCell::new(None),
            updater,
            cx,
            poll_task: RefCell::new(None),
        }
    }

    pub(super) fn is_available(&self) -> bool {
        true
    }

    pub(super) fn start(&self) {
        // Unstamped local builds have no release identity to compare against.
        if option_env!("RUNNER_BUILD_STAMP").is_none() || super::dev_available().is_some() {
            return;
        }
        if self.automatically_checks.get() {
            self.check_for_updates();
        }
        let updater = self.updater.clone();
        *self.poll_task.borrow_mut() = Some(self.cx.spawn(async move |cx| loop {
            cx.background_executor().timer(CHECK_INTERVAL).await;
            if updater
                .update(cx, |updater, _| {
                    if updater.automatically_checks_for_updates() {
                        updater.check_for_updates();
                    }
                })
                .is_err()
            {
                break;
            }
        }));
    }

    pub(super) fn check_for_updates(&self) {
        if self.checking.replace(true) {
            return;
        }
        self.error.borrow_mut().take();
        let updater = self.updater.clone();
        self.cx
            .spawn(async move |cx| {
                if updater.update(cx, |_, cx| cx.notify()).is_err() {
                    return;
                }
                let result = cx
                    .background_executor()
                    .spawn(async {
                        let release = fetch_release(RELEASE_API)?;
                        available_update(&release, option_env!("RUNNER_BUILD_STAMP"))
                    })
                    .await;
                let _ = updater.update(cx, |updater, cx| {
                    updater.finish_windows_check(result, cx);
                });
            })
            .detach();
    }

    pub(super) fn automatically_checks_for_updates(&self) -> bool {
        self.automatically_checks.get()
    }

    pub(super) fn set_automatically_checks_for_updates(&self, enabled: bool) {
        if !self.automatically_checks.replace(enabled)
            && enabled
            && option_env!("RUNNER_BUILD_STAMP").is_some()
        {
            self.check_for_updates();
        }
    }

    pub(super) fn last_check_at(&self) -> Option<SystemTime> {
        self.last_check_at.get()
    }

    pub(super) fn is_checking(&self) -> bool {
        self.checking.get()
    }

    pub(super) fn check_error(&self) -> Option<String> {
        self.error.borrow().clone()
    }
}

impl Updater {
    fn finish_windows_check(&mut self, result: Result<Option<UpdateInfo>>, cx: &mut Context<Self>) {
        self.native.checking.set(false);
        match result {
            Ok(available) => {
                self.native.last_check_at.set(Some(SystemTime::now()));
                self.native.error.borrow_mut().take();
                self.available = super::dev_available().or(available);
            }
            Err(error) => {
                tracing::warn!("Windows update check failed: {error:#}");
                *self.native.error.borrow_mut() =
                    Some("Could not check for updates. Try again or open downloads.".into());
            }
        }
        cx.notify();
    }
}

#[derive(Deserialize)]
struct Release {
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    state: String,
}

fn fetch_release(url: &str) -> Result<Release> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("Runner/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(20))
        .build()?
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()?
        .error_for_status()?
        .json()
        .context("read Windows nightly release")
}

fn nightly_version(name: &str) -> Option<(&str, &str)> {
    let version = name
        .strip_prefix("Runner-Nightly-")?
        .strip_suffix("-x64.zip")?;
    let mut parts = version.rsplitn(3, '.');
    let time = parts.next()?;
    let date = parts.next()?;
    let base = parts.next()?;
    if base.is_empty()
        || date.len() != 8
        || time.len() != 4
        || !date
            .bytes()
            .chain(time.bytes())
            .all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    Some((version, &version[base.len() + 1..]))
}

fn available_update(
    release: &Release,
    installed_stamp: Option<&str>,
) -> Result<Option<UpdateInfo>> {
    let Some((version, stamp)) = release
        .assets
        .iter()
        .filter(|asset| asset.state == "uploaded")
        .filter_map(|asset| nightly_version(&asset.name))
        .max_by_key(|(_, stamp)| *stamp)
    else {
        bail!("Windows release has no completed x64 nightly ZIP");
    };
    Ok(installed_stamp
        .filter(|installed| stamp > *installed)
        .map(|_| UpdateInfo::new(version)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext as _;

    #[test]
    fn rolling_release_uses_newest_completed_windows_asset() {
        let release: Release = serde_json::from_value(serde_json::json!({"assets": [
            {"name": "Runner-Nightly-0.7.5.20260906.0100-x64.zip", "state": "uploaded"},
            {"name": "Runner-Nightly-0.7.5.20260909.0100-x64.zip", "state": "starter"},
            {"name": "Runner-Nightly-0.7.5.20260908.0100-arm64.zip", "state": "uploaded"},
            {"name": "Runner-Nightly-0.7.5.20260907.0100-x64.zip", "state": "uploaded"},
            {"name": "Runner-Nightly-0.7.5.20260905.0100-x64.zip", "state": "uploaded"}
        ]}))
        .unwrap();
        assert_eq!(
            available_update(&release, Some("20260906.0100")).unwrap(),
            Some(UpdateInfo::new("0.7.5.20260907.0100"))
        );
        for stamp in [None, Some("20260907.0100"), Some("20260908.0100")] {
            assert_eq!(available_update(&release, stamp).unwrap(), None);
        }
    }

    #[test]
    fn incomplete_or_malformed_release_is_not_reported_as_up_to_date() {
        for name in [
            "Runner.zip",
            "Runner-Nightly-0.7.5.20260907.bad-x64.zip",
            "Runner-Nightly-0.7.5-x64.zip",
        ] {
            let release = Release {
                assets: vec![Asset {
                    name: name.into(),
                    state: "uploaded".into(),
                }],
            };
            assert!(available_update(&release, Some("20260906.0100")).is_err());
        }
        assert!(available_update(&Release { assets: vec![] }, None).is_err());
    }

    #[test]
    fn failed_check_keeps_known_update_and_success_timestamp() {
        let cx = gpui::TestAppContext::single();
        cx.update(|cx| {
            let updater = cx.new(|cx| Updater::new(false, cx));
            updater.update(cx, |updater, cx| {
                updater.available = Some(UpdateInfo::new("0.7.5.20260907.0100"));
                updater
                    .native
                    .last_check_at
                    .set(Some(SystemTime::UNIX_EPOCH));
                updater.native.checking.set(true);
                updater.finish_windows_check(Err(anyhow::anyhow!("offline")), cx);
                assert_eq!(
                    updater.available().unwrap().version(),
                    "0.7.5.20260907.0100"
                );
                assert_eq!(updater.last_check_at(), Some(SystemTime::UNIX_EPOCH));
                assert!(!updater.is_checking());
                assert!(updater.check_error().is_some());
                updater.finish_windows_check(Ok(None), cx);
                assert!(updater.check_error().is_none());
                assert!(updater.last_check_at().unwrap() > SystemTime::UNIX_EPOCH);
            });
        });
    }
}
