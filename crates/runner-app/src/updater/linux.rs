use std::cell::{Cell, RefCell};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::{bail, Context as _, Result};
use base64::Engine as _;
use gpui::{AsyncApp, Context, Task, WeakEntity};
use serde::Deserialize;

use super::{release_urls, UpdateInfo, UpdateState, UpdateStep, Updater};

#[path = "linux/install.rs"]
mod install;

use install::{attempt_path, available_update, sweep};
pub use install::{relaunch, run_installer};

const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
pub(super) const PUBLIC_KEY: &str = include_str!("../../../../packaging/windows-update-public-key");

#[derive(Clone, Debug, Eq, PartialEq)]
struct Candidate {
    info: UpdateInfo,
    name: String,
    installer_url: String,
    sig_url: Option<String>,
    size: u64,
}

impl Candidate {
    fn available(&self) -> UpdateState {
        UpdateState::Available {
            info: self.info.clone(),
            size: self.size,
            manual_only: self.sig_url.is_none(),
        }
    }
}

#[derive(Clone, Default)]
struct Transfer {
    received: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
    verifying: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
}

#[derive(Debug)]
struct Failure {
    step: UpdateStep,
    message: String,
}

impl Failure {
    fn new(step: UpdateStep, error: impl std::fmt::Display) -> Self {
        Self {
            step,
            message: error.to_string(),
        }
    }
}

pub(super) struct NativeUpdater {
    automatically_downloads: Cell<bool>,
    checking: Cell<bool>,
    last_check_at: Cell<Option<SystemTime>>,
    updater: WeakEntity<Updater>,
    cx: AsyncApp,
    poll_task: RefCell<Option<Task<()>>>,
    updates_dir: PathBuf,
    candidate: Option<Candidate>,
    transfer: Option<Transfer>,
    progress_task: Option<Task<()>>,
    verified_path: Option<PathBuf>,
    install_log: Option<PathBuf>,
    /// What the person chose during a check: to download, which starts when the check succeeds,
    /// or to cancel, which also holds off an automatic download for that check.
    download_after_check: Option<bool>,
    installing: bool,
}

impl Drop for NativeUpdater {
    fn drop(&mut self) {
        if let Some(transfer) = &self.transfer {
            transfer.cancelled.store(true, Ordering::Relaxed);
        }
    }
}

impl NativeUpdater {
    pub(super) fn is_available(&self) -> bool {
        true
    }

    pub(super) fn start(&self) {
        // Unstamped local builds have no release identity to compare against.
        if option_env!("RUNNER_BUILD_STAMP").is_none() {
            return;
        }
        self.check_for_updates();
        let updater = self.updater.clone();
        *self.poll_task.borrow_mut() = Some(self.cx.spawn(async move |cx| loop {
            cx.background_executor().timer(CHECK_INTERVAL).await;
            if updater
                .update(cx, |updater, _| updater.check_for_updates())
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
        let updater = self.updater.clone();
        let updates_dir = self.updates_dir.clone();
        self.cx
            .spawn(async move |cx| {
                let proceed = updater
                    .update(cx, |updater, cx| {
                        if updater.native.installing {
                            updater.native.checking.set(false);
                            return false;
                        }
                        if updater.native.transfer.is_some() {
                            updater.native.checking.set(false);
                            return false;
                        }
                        if matches!(updater.state, UpdateState::UpToDate { .. }) {
                            updater.state = UpdateState::UpToDate { checking: true };
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !proceed {
                    return;
                }
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        sweep(&updates_dir, option_env!("RUNNER_BUILD_STAMP"), None, false)?;
                        let client = http_client()?;
                        let release = fetch_release(
                            &client,
                            release_urls(option_env!("RUNNER_RELEASE_CHANNEL")).0,
                        )?;
                        let candidate =
                            available_update(&release, option_env!("RUNNER_BUILD_STAMP"))?;
                        sweep(
                            &updates_dir,
                            option_env!("RUNNER_BUILD_STAMP"),
                            candidate.as_ref(),
                            true,
                        )?;
                        let staged = candidate.as_ref().and_then(|candidate| {
                            let path = updates_dir.join(&candidate.name);
                            (candidate.sig_url.is_some() && path.is_file()).then(|| {
                                verify_staged(&client, &path, candidate, PUBLIC_KEY).map(|()| path)
                            })
                        });
                        Ok((candidate, staged))
                    })
                    .await;
                let _ = updater.update(cx, |updater, cx| updater.finish_check(result, cx));
            })
            .detach();
    }

    pub(super) fn automatically_checks_for_updates(&self) -> bool {
        true
    }
    pub(super) fn set_automatically_checks_for_updates(&self, _: bool) {}
    pub(super) fn last_check_at(&self) -> Option<SystemTime> {
        self.last_check_at.get()
    }
    pub(super) fn is_checking(&self) -> bool {
        self.checking.get()
    }
}

type CheckResult = Result<(Option<Candidate>, Option<Result<PathBuf, Failure>>)>;

impl Updater {
    pub fn new(
        automatically_downloads: bool,
        updates_dir: PathBuf,
        cx: &mut Context<Self>,
    ) -> Self {
        let (state, preview) = Self::initial_state();
        Self {
            native: NativeUpdater {
                automatically_downloads: Cell::new(automatically_downloads),
                checking: Cell::new(false),
                last_check_at: Cell::new(None),
                updater: cx.weak_entity(),
                cx: cx.to_async(),
                poll_task: RefCell::new(None),
                updates_dir,
                candidate: None,
                transfer: None,
                progress_task: None,
                verified_path: None,
                install_log: None,
                download_after_check: None,
                installing: false,
            },
            state,
            skipped_version: None,
            preview,
        }
    }

    pub fn automatically_downloads_updates(&self) -> bool {
        self.native.automatically_downloads.get()
    }
    pub fn set_automatically_downloads_updates(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.native.automatically_downloads.set(enabled);
        if enabled && matches!(self.state, UpdateState::Available { .. }) && !self.is_skipped() {
            self.request_download(cx);
        }
        cx.notify();
    }

    fn finish_check(&mut self, result: CheckResult, cx: &mut Context<Self>) {
        self.native.checking.set(false);
        let download_after_check = self.native.download_after_check.take();
        match result {
            Ok((candidate, staged)) => {
                self.native.last_check_at.set(Some(SystemTime::now()));
                self.native.verified_path = None;
                self.native.install_log = None;
                self.state = candidate.as_ref().map_or(
                    UpdateState::UpToDate { checking: false },
                    Candidate::available,
                );
                self.native.candidate = candidate;
                if let Some(staged) = staged {
                    self.finish_download(staged.map(Some), cx);
                } else if download_after_check
                    .unwrap_or_else(|| self.automatically_downloads_updates() && !self.is_skipped())
                {
                    self.download_update(cx);
                }
            }
            Err(error) => {
                tracing::warn!("Update check failed: {error:#}");
                // A failed check must not discard an already verified installer.
                if !matches!(
                    self.state,
                    UpdateState::Ready { .. }
                        | UpdateState::Failed {
                            step: UpdateStep::Install,
                            ..
                        }
                ) {
                    self.state = UpdateState::Failed {
                        step: UpdateStep::Check,
                        message: format!("Could not check for updates: {error}"),
                        info: self
                            .state
                            .info()
                            .or(self.native.candidate.as_ref().map(|c| &c.info))
                            .cloned(),
                    };
                }
            }
        }
        cx.notify();
    }

    pub fn download_update(&mut self, cx: &mut Context<Self>) {
        if self.native.transfer.is_some() {
            return;
        }
        if !matches!(
            self.state,
            UpdateState::Available { .. }
                | UpdateState::Failed {
                    step: UpdateStep::Download | UpdateStep::Verify,
                    ..
                }
        ) {
            return;
        }
        let Some(candidate) = self
            .native
            .candidate
            .clone()
            .filter(|candidate| candidate.sig_url.is_some())
        else {
            return;
        };
        // A check replaces the candidate when it finishes, so the download waits for it.
        if self.native.checking.get() {
            self.native.download_after_check = Some(true);
            self.state = UpdateState::Downloading {
                info: candidate.info,
                received: 0,
                total: 0,
            };
            cx.notify();
            return;
        }
        let transfer = Transfer::default();
        self.native.transfer = Some(transfer.clone());
        self.state = UpdateState::Downloading {
            info: candidate.info.clone(),
            received: 0,
            total: 0,
        };
        let progress = transfer.clone();
        let progress_info = candidate.info.clone();
        self.native.progress_task = Some(cx.spawn(async move |weak, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            if weak
                .update(cx, |updater, cx| {
                    if matches!(updater.state, UpdateState::Downloading { .. }) {
                        updater.state = if progress.verifying.load(Ordering::Relaxed) {
                            UpdateState::Verifying {
                                info: progress_info.clone(),
                            }
                        } else {
                            UpdateState::Downloading {
                                info: progress_info.clone(),
                                received: progress.received.load(Ordering::Relaxed),
                                total: progress.total.load(Ordering::Relaxed),
                            }
                        };
                        cx.notify();
                    }
                })
                .is_err()
            {
                break;
            }
        }));
        let updates_dir = self.native.updates_dir.clone();
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { download(&candidate, &updates_dir, &transfer, PUBLIC_KEY) })
                .await;
            let _ = weak.update(cx, |updater, cx| updater.finish_download(result, cx));
        })
        .detach();
        cx.notify();
    }

    pub(super) fn cancel_download(&mut self, cx: &mut Context<Self>) {
        if let Some(transfer) = &self.native.transfer {
            transfer.cancelled.store(true, Ordering::Relaxed);
        }
        if self.native.download_after_check == Some(true) {
            self.native.download_after_check = Some(false);
            if let Some(candidate) = &self.native.candidate {
                self.state = candidate.available();
            }
            cx.notify();
        }
    }

    pub fn install_log_path(&self) -> Option<&Path> {
        self.native.install_log.as_deref()
    }

    pub fn fail_install(&mut self, error: impl std::fmt::Display, cx: &mut Context<Self>) {
        self.state = UpdateState::Failed {
            step: UpdateStep::Install,
            message: error.to_string(),
            info: self
                .state
                .info()
                .or(self.native.candidate.as_ref().map(|c| &c.info))
                .cloned(),
        };
        cx.notify();
    }

    fn finish_download(
        &mut self,
        mut result: Result<Option<PathBuf>, Failure>,
        cx: &mut Context<Self>,
    ) {
        if self
            .native
            .transfer
            .as_ref()
            .is_some_and(|transfer| transfer.cancelled.load(Ordering::Relaxed))
        {
            if let Ok(Some(path)) = &result {
                result = fs::remove_file(path)
                    .map(|()| None)
                    .map_err(|error| Failure::new(UpdateStep::Download, error));
            }
        }
        self.native.transfer = None;
        self.native.progress_task = None;
        self.native.verified_path = None;
        self.native.install_log = None;
        let Some(candidate) = &self.native.candidate else {
            return;
        };
        self.state = match result {
            Ok(Some(path)) => {
                self.native.verified_path = Some(path.clone());
                self.native.install_log = fs::read_to_string(attempt_path(&path))
                    .ok()
                    .map(PathBuf::from);
                if self.native.install_log.is_some() {
                    UpdateState::Failed {
                        step: UpdateStep::Install,
                        message: crate::i18n::text("ui.installer_did_not_finish_the_update").into(),
                        info: Some(candidate.info.clone()),
                    }
                } else {
                    UpdateState::Ready {
                        info: candidate.info.clone(),
                        size: candidate.size,
                    }
                }
            }
            Ok(None) => candidate.available(),
            Err(failure) => UpdateState::Failed {
                step: failure.step,
                message: failure.message,
                info: Some(candidate.info.clone()),
            },
        };
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
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

fn http_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("Runner/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(10 * 60))
        .build()
        .context("create update client")
}

fn fetch_release(client: &reqwest::blocking::Client, url: &str) -> Result<Release> {
    client
        .get(url)
        .timeout(Duration::from_secs(20))
        .header("Accept", "application/vnd.github+json")
        .send()?
        .error_for_status()?
        .json()
        .context("read release")
}

fn verify_file(path: &Path, encoded_signature: &str, public_key: &str) -> Result<()> {
    let signature = base64::engine::general_purpose::STANDARD.decode(encoded_signature.trim())?;
    let signature = minisign_verify::Signature::decode(std::str::from_utf8(&signature)?)?;
    let key = minisign_verify::PublicKey::decode(public_key)?;
    key.verify(&fs::read(path)?, &signature, true)
        .context("verify update signature")
}

fn verify_staged(
    client: &reqwest::blocking::Client,
    path: &Path,
    candidate: &Candidate,
    public_key: &str,
) -> Result<(), Failure> {
    let signature = (|| -> Result<String> {
        client
            .get(
                candidate
                    .sig_url
                    .as_ref()
                    .context("missing installer signature")?,
            )
            .timeout(Duration::from_secs(20))
            .send()?
            .error_for_status()?
            .text()
            .context("fetch update signature")
    })()
    .map_err(|error| Failure::new(UpdateStep::Verify, error))?;
    let result = verify_file(path, &signature, public_key);
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    if result.is_ok() {
        fs::write(
            path.parent()
                .unwrap()
                .join(format!("{}.sig", candidate.name)),
            &signature,
        )
        .map_err(|error| Failure::new(UpdateStep::Verify, error))?;
    }
    result.map_err(|error| Failure::new(UpdateStep::Verify, error))
}

fn stream_installer(reader: &mut impl Read, path: &Path, transfer: &Transfer) -> Result<bool> {
    let mut file = File::create(path)?;
    let mut buffer = [0; 64 * 1024];
    loop {
        if transfer.cancelled.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let read = reader.read(&mut buffer)?;
        if transfer.cancelled.load(Ordering::Relaxed) {
            return Ok(false);
        }
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])?;
        transfer.received.fetch_add(read as u64, Ordering::Relaxed);
    }
    file.sync_all()?;
    Ok(true)
}

fn download(
    candidate: &Candidate,
    dir: &Path,
    transfer: &Transfer,
    public_key: &str,
) -> Result<Option<PathBuf>, Failure> {
    let partial = dir.join(format!("{}.partial", candidate.name));
    let result = (|| {
        let client = http_client().map_err(|error| Failure::new(UpdateStep::Download, error))?;
        let final_path = dir.join(&candidate.name);
        if final_path.is_file() {
            transfer.verifying.store(true, Ordering::Relaxed);
            verify_staged(&client, &final_path, candidate, public_key)?;
            return Ok(Some(final_path));
        }
        let streamed = (|| -> Result<bool> {
            fs::create_dir_all(dir)?;
            let mut response = client
                .get(&candidate.installer_url)
                .send()?
                .error_for_status()?;
            transfer
                .total
                .store(response.content_length().unwrap_or(0), Ordering::Relaxed);
            stream_installer(&mut response, &partial, transfer)
        })()
        .map_err(|error| Failure::new(UpdateStep::Download, error))?;
        if !streamed || transfer.cancelled.load(Ordering::Relaxed) {
            return Ok(None);
        }
        transfer.verifying.store(true, Ordering::Relaxed);
        verify_staged(&client, &partial, candidate, public_key)?;
        if transfer.cancelled.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let final_path = dir.join(&candidate.name);
        fs::rename(&partial, &final_path)
            .map_err(|error| Failure::new(UpdateStep::Verify, error))?;
        Ok(Some(final_path))
    })();
    if !matches!(result, Ok(Some(_))) {
        let _ = fs::remove_file(&partial);
    }
    if transfer.cancelled.load(Ordering::Relaxed) && !matches!(result, Ok(Some(_))) {
        return Ok(None);
    }
    result
}

#[cfg(test)]
mod tests {
    use gpui::AppContext as _;

    use super::*;

    #[test]
    fn a_download_asked_for_during_a_check_starts_once_when_the_check_finishes() {
        let dir = tempfile::tempdir().unwrap();
        let candidate = Candidate {
            info: UpdateInfo::new("0.19.5"),
            name: "Runner_0.19.5_linux-x64.tar.gz".into(),
            installer_url: "http://127.0.0.1:0/package".into(),
            sig_url: Some("http://127.0.0.1:0/signature".into()),
            size: 44,
        };
        let cx = gpui::TestAppContext::single();
        cx.update(|cx| {
            let pending_download = |cx: &mut gpui::App| {
                let updater = cx.new(|cx| Updater::new(false, dir.path().to_owned(), cx));
                updater.update(cx, |updater, cx| {
                    updater.finish_check(Ok((Some(candidate.clone()), None)), cx);
                    assert!(matches!(updater.state(), UpdateState::Available { .. }));
                    updater.native.checking.set(true);
                    updater.request_download(cx);
                    assert!(matches!(
                        updater.state(),
                        UpdateState::Downloading {
                            received: 0,
                            total: 0,
                            ..
                        }
                    ));
                    assert!(updater.native.transfer.is_none());
                    // A second click while it waits changes nothing.
                    updater.request_download(cx);
                    assert!(updater.native.transfer.is_none());
                });
                updater
            };

            let finished = pending_download(cx);
            finished.update(cx, |updater, cx| {
                updater.finish_check(Ok((Some(candidate.clone()), None)), cx);
                assert!(matches!(updater.state(), UpdateState::Downloading { .. }));
                assert!(updater.native.transfer.is_some());
                assert_eq!(updater.native.download_after_check, None);
                let started = updater.native.transfer.as_ref().unwrap().received.clone();
                updater.download_update(cx);
                let still = &updater.native.transfer.as_ref().unwrap().received;
                assert!(Arc::ptr_eq(&started, still));
            });

            // A failed check reports its error, which can be one only a restart fixes.
            let failed_check = pending_download(cx);
            failed_check.update(cx, |updater, cx| {
                updater.finish_check(Err(anyhow::anyhow!("restart Runner")), cx);
                let UpdateState::Failed {
                    step: UpdateStep::Check,
                    message,
                    info: Some(_),
                } = updater.state()
                else {
                    panic!("{:?}", updater.state());
                };
                assert!(message.contains("restart Runner"));
                assert!(updater.native.transfer.is_none());
                assert_eq!(updater.native.download_after_check, None);
            });

            let update_gone = pending_download(cx);
            update_gone.update(cx, |updater, cx| {
                updater.finish_check(Ok((None, None)), cx);
                assert_eq!(*updater.state(), UpdateState::UpToDate { checking: false });
                assert!(updater.native.transfer.is_none());
            });

            // Cancel holds for this check whichever way the setting is, and leaves the setting.
            for automatic in [false, true] {
                let cancelled = pending_download(cx);
                cancelled.update(cx, |updater, cx| {
                    updater.native.automatically_downloads.set(automatic);
                    updater.request_cancel(cx);
                    assert!(matches!(updater.state(), UpdateState::Available { .. }));
                    updater.finish_check(Ok((Some(candidate.clone()), None)), cx);
                    assert!(matches!(updater.state(), UpdateState::Available { .. }));
                    assert!(updater.native.transfer.is_none());
                    assert_eq!(updater.automatically_downloads_updates(), automatic);
                    // An explicit Download still works afterwards.
                    updater.request_download(cx);
                    assert!(updater.native.transfer.is_some());
                });

                // Changing their mind during the same check downloads after all.
                let changed_mind = pending_download(cx);
                changed_mind.update(cx, |updater, cx| {
                    updater.native.automatically_downloads.set(automatic);
                    updater.request_cancel(cx);
                    updater.request_download(cx);
                    assert!(matches!(updater.state(), UpdateState::Downloading { .. }));
                    assert!(updater.native.transfer.is_none());
                    updater.finish_check(Ok((Some(candidate.clone()), None)), cx);
                    assert!(updater.native.transfer.is_some());
                });
            }
        });
    }

    #[test]
    fn a_skipped_version_is_not_downloaded_automatically() {
        let dir = tempfile::tempdir().unwrap();
        let candidate = Candidate {
            info: UpdateInfo::new("0.19.5"),
            name: "Runner_0.19.5_linux-x64.tar.gz".into(),
            installer_url: "http://127.0.0.1:0/package".into(),
            sig_url: Some("http://127.0.0.1:0/signature".into()),
            size: 44,
        };
        let cx = gpui::TestAppContext::single();
        cx.update(|cx| {
            let updater = cx.new(|cx| Updater::new(true, dir.path().to_owned(), cx));
            updater.update(cx, |updater, cx| {
                updater.set_skipped_version(Some("0.19.5".to_owned()));
                updater.finish_check(Ok((Some(candidate), None)), cx);
                assert!(matches!(updater.state(), UpdateState::Available { .. }));
                assert!(updater.native.transfer.is_none());
                assert!(updater.is_skipped());
            });
        });
    }

    #[test]
    fn each_release_channel_reads_its_own_release() {
        assert_eq!(
            release_urls(Some("nightly")),
            (
                "https://api.github.com/repos/yicheng47/runner/releases/tags/nightly",
                "https://github.com/yicheng47/runner/releases/tag/nightly",
            )
        );
        assert_eq!(release_urls(None), release_urls(Some("nightly")));
        assert_eq!(
            release_urls(Some("preview-nightly")),
            (
                "https://api.github.com/repos/yicheng47/runner/releases/tags/preview-nightly",
                "https://github.com/yicheng47/runner/releases/tag/preview-nightly",
            )
        );
        assert_eq!(
            release_urls(Some("production")),
            (
                "https://api.github.com/repos/yicheng47/runner/releases/latest",
                "https://github.com/yicheng47/runner/releases/latest",
            )
        );
    }
}
