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

use super::{UpdateInfo, Updater};

const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const PUBLIC_KEY: &str = include_str!("../../../../packaging/windows-update-public-key");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdateStep {
    Check,
    Download,
    Verify,
    Install,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateState {
    UpToDate {
        checking: bool,
    },
    Available {
        info: UpdateInfo,
        installer_url: String,
        sig_url: Option<String>,
    },
    Downloading {
        received: u64,
        total: u64,
    },
    Ready {
        path: PathBuf,
        info: UpdateInfo,
    },
    Failed {
        step: UpdateStep,
        message: String,
        info: Option<UpdateInfo>,
    },
}

impl UpdateState {
    pub fn available(&self) -> Option<&UpdateInfo> {
        match self {
            Self::Available { info, .. } | Self::Ready { info, .. } => Some(info),
            Self::Failed { info, .. } => info.as_ref(),
            _ => None,
        }
    }
}

fn release_urls(channel: Option<&str>) -> (&'static str, &'static str) {
    match channel {
        Some("production") => (
            "https://api.github.com/repos/yicheng47/runner/releases/latest",
            "https://github.com/yicheng47/runner/releases/latest",
        ),
        _ => (
            "https://api.github.com/repos/yicheng47/runner/releases/tags/nightly",
            "https://github.com/yicheng47/runner/releases/tag/nightly",
        ),
    }
}

pub fn windows_download_url() -> &'static str {
    release_urls(option_env!("RUNNER_RELEASE_CHANNEL")).1
}

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
            installer_url: self.installer_url.clone(),
            sig_url: self.sig_url.clone(),
        }
    }
}

#[derive(Clone, Default)]
struct Transfer {
    received: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
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
        if option_env!("RUNNER_BUILD_STAMP").is_none() || super::dev_available().is_some() {
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
                let _ = updater.update(cx, |updater, cx| updater.finish_windows_check(result, cx));
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
        let state =
            super::dev_available().map_or(UpdateState::UpToDate { checking: false }, |info| {
                UpdateState::Available {
                    info,
                    installer_url: windows_download_url().into(),
                    sig_url: None,
                }
            });
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
            },
            state,
        }
    }

    pub fn state(&self) -> &UpdateState {
        &self.state
    }
    pub fn available(&self) -> Option<&UpdateInfo> {
        self.state.available()
    }
    pub fn update_info(&self) -> Option<&UpdateInfo> {
        self.available().or_else(|| {
            self.native
                .candidate
                .as_ref()
                .map(|candidate| &candidate.info)
        })
    }
    pub fn download_size(&self) -> u64 {
        self.native
            .candidate
            .as_ref()
            .map_or(0, |candidate| candidate.size)
    }
    pub fn automatically_downloads_updates(&self) -> bool {
        self.native.automatically_downloads.get()
    }
    pub fn set_automatically_downloads_updates(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.native.automatically_downloads.set(enabled);
        if enabled && matches!(self.state, UpdateState::Available { .. }) {
            self.download_update(cx);
        }
        cx.notify();
    }

    fn finish_windows_check(&mut self, result: CheckResult, cx: &mut Context<Self>) {
        self.native.checking.set(false);
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
                } else if self.automatically_downloads_updates() {
                    self.download_update(cx);
                }
            }
            Err(error) => {
                tracing::warn!("Windows update check failed: {error:#}");
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
                        info: self.update_info().cloned(),
                    };
                }
            }
        }
        cx.notify();
    }

    pub fn download_update(&mut self, cx: &mut Context<Self>) {
        if self.native.transfer.is_some() || self.native.checking.get() {
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
        let transfer = Transfer::default();
        self.native.transfer = Some(transfer.clone());
        self.state = UpdateState::Downloading {
            received: 0,
            total: 0,
        };
        let progress = transfer.clone();
        self.native.progress_task = Some(cx.spawn(async move |weak, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            if weak
                .update(cx, |updater, cx| {
                    if matches!(updater.state, UpdateState::Downloading { .. }) {
                        updater.state = UpdateState::Downloading {
                            received: progress.received.load(Ordering::Relaxed),
                            total: progress.total.load(Ordering::Relaxed),
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

    pub fn cancel_download(&mut self) {
        if let Some(transfer) = &self.native.transfer {
            transfer.cancelled.store(true, Ordering::Relaxed);
        }
    }

    pub fn install_log_path(&self) -> Option<&Path> {
        self.native.install_log.as_deref()
    }

    pub fn prepare_install(&mut self, log_dir: &Path) -> Result<(PathBuf, PathBuf)> {
        if !matches!(
            self.state,
            UpdateState::Ready { .. }
                | UpdateState::Failed {
                    step: UpdateStep::Install,
                    ..
                }
        ) {
            bail!("The update is not ready to install");
        }
        let path = self
            .native
            .verified_path
            .clone()
            .context("No verified installer is staged")?;
        let candidate = self
            .native
            .candidate
            .as_ref()
            .context("No update is selected")?;
        let (_, stamp) = installer_version(&candidate.name).context("Invalid installer version")?;
        let log = log_dir.join(format!("update-{stamp}.log"));
        fs::create_dir_all(log_dir)?;
        fs::write(attempt_path(&path), log.to_string_lossy().as_bytes())?;
        self.native.install_log = Some(log.clone());
        Ok((path, log))
    }

    pub fn fail_install(&mut self, error: impl std::fmt::Display, cx: &mut Context<Self>) {
        self.state = UpdateState::Failed {
            step: UpdateStep::Install,
            message: error.to_string(),
            info: self.update_info().cloned(),
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
                        message: "The installer did not finish the update. Check the installer log in Settings → Diagnostics, then try again.".into(),
                        info: Some(candidate.info.clone()),
                    }
                } else {
                    UpdateState::Ready {
                        path,
                        info: candidate.info.clone(),
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
        .context("create Windows update client")
}

fn fetch_release(client: &reqwest::blocking::Client, url: &str) -> Result<Release> {
    client
        .get(url)
        .timeout(Duration::from_secs(20))
        .header("Accept", "application/vnd.github+json")
        .send()?
        .error_for_status()?
        .json()
        .context("read Windows release")
}

fn installer_version(name: &str) -> Option<(&str, &str)> {
    let version = name
        .strip_prefix("Runner-Setup-")
        .and_then(|name| name.strip_suffix("-x64.exe"))
        .or_else(|| {
            name.strip_prefix("Runner-Nightly-")?
                .strip_suffix("-x64.zip")
        })?;
    let mut parts = version.rsplitn(3, '.');
    let time = parts.next()?;
    let date = parts.next()?;
    let base = parts.next()?;
    if base.is_empty()
        || !base
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
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

fn available_update(release: &Release, installed_stamp: Option<&str>) -> Result<Option<Candidate>> {
    let Some((asset, version, stamp)) = release
        .assets
        .iter()
        .filter(|asset| asset.state == "uploaded")
        .filter_map(|asset| {
            installer_version(&asset.name).map(|(version, stamp)| (asset, version, stamp))
        })
        .max_by_key(|(asset, _, stamp)| (*stamp, asset.name.ends_with(".exe")))
    else {
        bail!("Windows release has no completed x64 installer or portable ZIP");
    };
    Ok(installed_stamp
        .filter(|installed| stamp > *installed)
        .map(|_| {
            let sig_name = format!("{}.sig", asset.name);
            let sig_url = asset
                .name
                .ends_with(".exe")
                .then(|| {
                    release
                        .assets
                        .iter()
                        .find(|sig| sig.name == sig_name && sig.state == "uploaded")
                        .map(|sig| sig.browser_download_url.clone())
                })
                .flatten();
            Candidate {
                info: UpdateInfo::new(version),
                name: asset.name.clone(),
                installer_url: asset.browser_download_url.clone(),
                sig_url,
                size: asset.size,
            }
        }))
}

fn sweep(
    dir: &Path,
    installed_stamp: Option<&str>,
    candidate: Option<&Candidate>,
    candidate_known: bool,
) -> Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let installer_name = name.strip_suffix(".attempt").unwrap_or(&name);
        let stale = installer_version(installer_name).is_some_and(|(_, stamp)| {
            installed_stamp.is_some_and(|installed| stamp <= installed)
                || (candidate_known
                    && candidate.is_none_or(|candidate| candidate.name != installer_name))
        });
        if name.ends_with(".partial") || stale {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn attempt_path(path: &Path) -> PathBuf {
    path.with_extension("exe.attempt")
}

fn verify_file(path: &Path, encoded_signature: &str, public_key: &str) -> Result<()> {
    let signature = base64::engine::general_purpose::STANDARD.decode(encoded_signature.trim())?;
    let signature = minisign_verify::Signature::decode(std::str::from_utf8(&signature)?)?;
    let key = minisign_verify::PublicKey::decode(public_key)?;
    key.verify(&fs::read(path)?, &signature, true)
        .context("verify Windows installer signature")
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
            .context("fetch Windows installer signature")
    })()
    .map_err(|error| Failure::new(UpdateStep::Verify, error))?;
    let result = verify_file(path, &signature, public_key);
    if result.is_err() {
        let _ = fs::remove_file(path);
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
    use super::*;
    use gpui::AppContext as _;
    use std::net::TcpListener;
    use std::thread::JoinHandle;

    const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/updates/installer.txt");
    const SIGNATURE: &str = include_str!("../../tests/fixtures/updates/installer.txt.sig");
    const TEST_KEY: &str = include_str!("../../tests/fixtures/updates/public-key");

    fn release(assets: &[(&str, &str)]) -> Release {
        serde_json::from_value(serde_json::json!({"assets": assets.iter().map(|(name, state)| {
            serde_json::json!({"name": name, "state": state, "browser_download_url": format!("https://example.com/{name}"), "size": 44})
        }).collect::<Vec<_>>()})).unwrap()
    }

    fn candidate() -> Candidate {
        available_update(
            &release(&[
                ("Runner-Setup-0.8.2.20260908.0100-x64.exe", "uploaded"),
                ("Runner-Setup-0.8.2.20260908.0100-x64.exe.sig", "uploaded"),
            ]),
            Some("20260907.0100"),
        )
        .unwrap()
        .unwrap()
    }

    fn server(responses: Vec<(&'static str, Vec<u8>)>) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = std::thread::spawn(move || {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        (url, task)
    }

    #[test]
    fn release_channels_keep_nightly_and_production_addresses_separate() {
        assert_eq!(
            release_urls(Some("nightly")),
            (
                "https://api.github.com/repos/yicheng47/runner/releases/tags/nightly",
                "https://github.com/yicheng47/runner/releases/tag/nightly",
            )
        );
        assert_eq!(release_urls(None), release_urls(Some("nightly")));
        assert_eq!(
            release_urls(Some("production")),
            (
                "https://api.github.com/repos/yicheng47/runner/releases/latest",
                "https://github.com/yicheng47/runner/releases/latest",
            )
        );
    }

    #[test]
    fn unified_nightly_selects_the_newest_windows_installer_and_its_signature() {
        let release = release(&[
            ("Runner-Nightly-def5678.20260909.0100-arm64.dmg", "uploaded"),
            ("appcast.xml", "uploaded"),
            (
                "Runner-Setup-nightly.abc1234.20260908.0100-x64.exe.sig",
                "uploaded",
            ),
            (
                "Runner-Setup-nightly.abc1234.20260908.0100-x64.exe",
                "uploaded",
            ),
            ("Runner-Setup-9.0.0.20260907.0100-x64.exe", "uploaded"),
            ("Runner-Setup-9.0.0.20260907.0100-x64.exe.sig", "uploaded"),
        ]);
        let update = available_update(&release, Some("20260907.0100"))
            .unwrap()
            .unwrap();
        assert_eq!(update.info.version(), "Nightly (abc1234)");
        assert_eq!(
            update.installer_url,
            "https://example.com/Runner-Setup-nightly.abc1234.20260908.0100-x64.exe"
        );
        assert_eq!(
            update.sig_url.as_deref(),
            Some("https://example.com/Runner-Setup-nightly.abc1234.20260908.0100-x64.exe.sig")
        );
    }

    #[test]
    fn commit_identified_nightly_updates_existing_versioned_installers() {
        let release = release(&[
            ("Runner-Setup-9.0.0.20260907.0100-x64.exe", "uploaded"),
            (
                "Runner-Setup-nightly.abc1234.20260908.0100-x64.exe",
                "uploaded",
            ),
            (
                "Runner-Setup-nightly.abc1234.20260908.0100-x64.exe.sig",
                "uploaded",
            ),
        ]);
        let update = available_update(&release, Some("20260907.0100"))
            .unwrap()
            .unwrap();
        assert_eq!(update.info.version(), "Nightly (abc1234)");
        assert_eq!(
            update.sig_url.as_deref(),
            Some("https://example.com/Runner-Setup-nightly.abc1234.20260908.0100-x64.exe.sig")
        );
        assert!(available_update(&release, Some("20260908.0100"))
            .unwrap()
            .is_none());
    }

    #[test]
    fn production_release_ignores_macos_assets_and_requires_a_windows_installer() {
        let mut release = release(&[
            ("Runner-0.8.2-arm64.dmg", "uploaded"),
            ("appcast.xml", "uploaded"),
            ("Runner-Setup-0.8.2.20260908.0100-x64.exe", "uploaded"),
        ]);
        let update = available_update(&release, Some("20260907.0100"))
            .unwrap()
            .unwrap();
        assert_eq!(update.info.version(), "0.8.2.20260908.0100");
        assert!(update.sig_url.is_none());
        assert_eq!(
            available_update(&release, Some("20260908.0100")).unwrap(),
            None
        );
        release.assets.pop();
        assert!(available_update(&release, Some("20260907.0100")).is_err());
    }

    #[test]
    fn signature_must_match_the_newest_completed_installer_in_the_same_release() {
        let mut release = release(&[
            ("Runner-Setup-0.8.2.20260909.0100-x64.exe", "starter"),
            ("Runner-Setup-0.8.2.20260908.0100-arm64.exe", "uploaded"),
            ("Runner-Setup-0.8.2.20260908.0100-x64.exe", "uploaded"),
            ("Runner-Setup-0.8.2.20260907.0100-x64.exe.sig", "uploaded"),
            ("Runner-Setup-0.8.2.20260908.0100-x64.exe.sig", "starter"),
        ]);
        assert!(available_update(&release, Some("20260907.0100"))
            .unwrap()
            .unwrap()
            .sig_url
            .is_none());
        release.assets.last_mut().unwrap().state = "uploaded".into();
        assert_eq!(
            available_update(&release, Some("20260907.0100"))
                .unwrap()
                .unwrap()
                .sig_url,
            Some("https://example.com/Runner-Setup-0.8.2.20260908.0100-x64.exe.sig".into())
        );
        for stamp in [None, Some("20260908.0100"), Some("20260909.0100")] {
            assert!(available_update(&release, stamp).unwrap().is_none());
        }
    }

    #[test]
    fn portable_zip_transition_preserves_comparison_but_never_installs_zip() {
        let mut release = release(&[
            ("Runner-Nightly-0.8.2.20260908.0100-x64.zip", "uploaded"),
            ("Runner-Nightly-0.8.2.20260908.0100-x64.zip.sig", "uploaded"),
            ("Runner-Nightly-0.8.2.20260909.0100-x64.zip", "starter"),
        ]);
        let update = available_update(&release, Some("20260907.0100"))
            .unwrap()
            .unwrap();
        assert_eq!(update.info.version(), "0.8.2.20260908.0100");
        assert!(update.sig_url.is_none());
        release.assets.extend(
            self::release(&[("Runner-Setup-0.8.2.20260908.0100-x64.exe", "uploaded")]).assets,
        );
        assert!(available_update(&release, Some("20260907.0100"))
            .unwrap()
            .unwrap()
            .name
            .ends_with(".exe"));
    }

    #[test]
    fn incomplete_or_malformed_release_is_not_reported_as_up_to_date() {
        for name in [
            "Runner.zip",
            "Runner-Nightly-0.8.1-x64.zip",
            "Runner-Setup-0.8.1.20260907.bad-x64.exe",
            "Runner-Setup-0.8.1.20260907.0100-x64.zip",
            "Runner-Nightly-0.8.1.20260907.0100-x64.exe",
            "Runner-Setup-../0.8.1.20260907.0100-x64.exe",
        ] {
            assert!(
                available_update(&release(&[(name, "uploaded")]), Some("20260906.0100")).is_err()
            );
        }
        assert!(available_update(&release(&[]), None).is_err());
    }

    #[test]
    fn sweep_preserves_current_candidate_and_removes_partials_and_other_stamps() {
        let dir = tempfile::tempdir().unwrap();
        let candidate = candidate();
        for name in [
            &candidate.name,
            "Runner-Setup-0.8.1.20260907.0100-x64.exe",
            "Runner-Setup-0.8.3.20260909.0100-x64.exe",
            "interrupted.partial",
            "keep.txt",
        ] {
            fs::write(dir.path().join(name), FIXTURE).unwrap();
        }
        sweep(dir.path(), Some("20260907.0100"), None, false).unwrap();
        assert!(dir.path().join(&candidate.name).is_file());
        assert!(dir
            .path()
            .join("Runner-Setup-0.8.3.20260909.0100-x64.exe")
            .is_file());
        assert!(!dir.path().join("interrupted.partial").exists());
        assert!(!dir
            .path()
            .join("Runner-Setup-0.8.1.20260907.0100-x64.exe")
            .exists());
        sweep(dir.path(), Some("20260907.0100"), Some(&candidate), true).unwrap();
        verify_file(&dir.path().join(&candidate.name), SIGNATURE, TEST_KEY).unwrap();
        assert!(!dir
            .path()
            .join("Runner-Setup-0.8.3.20260909.0100-x64.exe")
            .exists());
        sweep(dir.path(), Some("20260908.0100"), None, true).unwrap();
        assert!(!dir.path().join(&candidate.name).exists());
        assert!(dir.path().join("keep.txt").exists());
    }

    #[test]
    fn signature_fixture_accepts_only_the_signed_bytes_and_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture");
        fs::write(&path, FIXTURE).unwrap();
        verify_file(&path, SIGNATURE, TEST_KEY).unwrap();
        assert!(verify_file(&path, SIGNATURE, PUBLIC_KEY).is_err());
        assert!(verify_file(&path, "bad signature", TEST_KEY).is_err());
        fs::write(&path, b"tampered").unwrap();
        assert!(verify_file(&path, SIGNATURE, TEST_KEY).is_err());
    }

    #[test]
    fn streamed_download_verifies_before_rename_and_reports_progress() {
        let dir = tempfile::tempdir().unwrap();
        let (url, task) = server(vec![
            ("200 OK", FIXTURE.to_vec()),
            ("200 OK", SIGNATURE.as_bytes().to_vec()),
        ]);
        let mut candidate = candidate();
        candidate.installer_url = url.clone();
        candidate.sig_url = Some(url);
        let transfer = Transfer::default();
        fs::write(
            dir.path().join(format!("{}.partial", candidate.name)),
            vec![0; 1024],
        )
        .unwrap();
        let path = download(&candidate, dir.path(), &transfer, TEST_KEY)
            .unwrap()
            .unwrap();
        task.join().unwrap();
        assert_eq!(path, dir.path().join(&candidate.name));
        assert_eq!(fs::read(path).unwrap(), FIXTURE);
        assert_eq!(
            transfer.received.load(Ordering::Relaxed),
            FIXTURE.len() as u64
        );
        assert_eq!(transfer.total.load(Ordering::Relaxed), FIXTURE.len() as u64);
        assert!(!dir
            .path()
            .join(format!("{}.partial", candidate.name))
            .exists());
    }

    #[test]
    fn cancelling_a_live_stream_removes_received_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut candidate = candidate();
        candidate.installer_url = format!("http://{}", listener.local_addr().unwrap());
        let transfer = Transfer::default();
        let control = transfer.clone();
        let task = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 131072\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            stream.write_all(&vec![0; 65536]).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while control.received.load(Ordering::Relaxed) == 0 {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
            control.cancelled.store(true, Ordering::Relaxed);
            let _ = stream.write_all(&vec![0; 65536]);
        });
        assert!(download(&candidate, dir.path(), &transfer, TEST_KEY)
            .unwrap()
            .is_none());
        task.join().unwrap();
        assert!(transfer.received.load(Ordering::Relaxed) > 0);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn download_verify_failures_and_cancel_leave_no_staged_files() {
        for (status, signature, cancelled, expected_step) in [
            ("503 Unavailable", None, false, Some(UpdateStep::Download)),
            ("200 OK", Some("invalid"), false, Some(UpdateStep::Verify)),
            ("200 OK", None, true, None),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut responses = vec![(status, FIXTURE.to_vec())];
            if let Some(sig) = signature {
                responses.push(("200 OK", sig.as_bytes().to_vec()));
            }
            let (url, task) = server(responses);
            let mut candidate = candidate();
            candidate.installer_url = url.clone();
            candidate.sig_url = Some(url);
            let transfer = Transfer::default();
            transfer.cancelled.store(cancelled, Ordering::Relaxed);
            let result = download(&candidate, dir.path(), &transfer, TEST_KEY);
            task.join().unwrap();
            match expected_step {
                Some(step) => assert_eq!(result.unwrap_err().step, step),
                None => assert!(result.unwrap().is_none()),
            }
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        }
    }

    #[test]
    fn cached_installer_is_reverified_without_downloading_again() {
        let dir = tempfile::tempdir().unwrap();
        let mut candidate = candidate();
        let path = dir.path().join(&candidate.name);
        fs::write(&path, FIXTURE).unwrap();
        let (url, task) = server(vec![("200 OK", SIGNATURE.as_bytes().to_vec())]);
        candidate.sig_url = Some(url);
        verify_staged(&http_client().unwrap(), &path, &candidate, TEST_KEY).unwrap();
        task.join().unwrap();
        assert!(path.exists());
        let (url, task) = server(vec![("200 OK", SIGNATURE.as_bytes().to_vec())]);
        candidate.sig_url = Some(url);
        fs::write(&path, b"tampered").unwrap();
        assert_eq!(
            verify_staged(&http_client().unwrap(), &path, &candidate, TEST_KEY)
                .unwrap_err()
                .step,
            UpdateStep::Verify
        );
        task.join().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn signature_transport_failure_preserves_cached_installer_for_retry() {
        let dir = tempfile::tempdir().unwrap();
        let mut candidate = candidate();
        let path = dir.path().join(&candidate.name);
        fs::write(&path, FIXTURE).unwrap();
        let (url, task) = server(vec![
            ("503 Unavailable", Vec::new()),
            ("200 OK", SIGNATURE.as_bytes().to_vec()),
        ]);
        candidate.sig_url = Some(url);
        candidate.installer_url = "http://127.0.0.1:0/must-not-download".into();
        assert_eq!(
            verify_staged(&http_client().unwrap(), &path, &candidate, TEST_KEY)
                .unwrap_err()
                .step,
            UpdateStep::Verify
        );
        assert_eq!(fs::read(&path).unwrap(), FIXTURE);
        let transfer = Transfer::default();
        assert_eq!(
            download(&candidate, dir.path(), &transfer, TEST_KEY).unwrap(),
            Some(path)
        );
        task.join().unwrap();
        assert_eq!(transfer.received.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn install_handoff_requires_verified_state_and_recovers_an_unfinished_attempt() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        let candidate = candidate();
        let path = dir.path().join(&candidate.name);
        fs::write(&path, FIXTURE).unwrap();
        verify_file(&path, SIGNATURE, TEST_KEY).unwrap();
        let cx = gpui::TestAppContext::single();
        cx.update(|cx| {
            let updater = cx.new(|cx| Updater::new(false, dir.path().to_owned(), cx));
            updater.update(cx, |updater, cx| {
                updater.finish_windows_check(Ok((Some(candidate.clone()), None)), cx);
                for state in [
                    UpdateState::UpToDate { checking: false },
                    candidate.available(),
                    UpdateState::Downloading {
                        received: 1,
                        total: 2,
                    },
                    UpdateState::Ready {
                        path: path.clone(),
                        info: candidate.info.clone(),
                    },
                    UpdateState::Failed {
                        step: UpdateStep::Install,
                        message: "test".into(),
                        info: Some(candidate.info.clone()),
                    },
                ] {
                    updater.state = state;
                    assert!(updater.prepare_install(&logs).is_err());
                }
                assert!(!attempt_path(&path).exists());
                updater.finish_download(Ok(Some(path.clone())), cx);
                assert!(matches!(updater.state(), UpdateState::Ready { .. }));
                let (installer, log) = updater.prepare_install(&logs).unwrap();
                assert_eq!(installer, path);
                assert_eq!(log, logs.join("update-20260908.0100.log"));
                assert_eq!(updater.install_log_path(), Some(log.as_path()));
                assert_eq!(
                    fs::read_to_string(attempt_path(&path)).unwrap(),
                    log.to_string_lossy()
                );
                updater.fail_install("spawn failed", cx);
                assert_eq!(updater.prepare_install(&logs).unwrap(), (path.clone(), log));
            });
            let restarted = cx.new(|cx| Updater::new(false, dir.path().to_owned(), cx));
            restarted.update(cx, |updater, cx| {
                verify_file(&path, SIGNATURE, TEST_KEY).unwrap();
                updater.finish_windows_check(
                    Ok((Some(candidate.clone()), Some(Ok(path.clone())))),
                    cx,
                );
                assert!(matches!(
                    updater.state(),
                    UpdateState::Failed {
                        step: UpdateStep::Install,
                        ..
                    }
                ));
                assert!(updater.available().is_some());
                assert_eq!(
                    updater.install_log_path(),
                    Some(logs.join("update-20260908.0100.log").as_path())
                );
                updater.finish_windows_check(Err(anyhow::anyhow!("offline")), cx);
                assert!(matches!(
                    updater.state(),
                    UpdateState::Failed {
                        step: UpdateStep::Install,
                        ..
                    }
                ));
                assert!(updater.prepare_install(&logs).is_ok());
            });
            let unverified = cx.new(|cx| Updater::new(false, dir.path().to_owned(), cx));
            unverified.update(cx, |updater, cx| {
                updater.finish_windows_check(
                    Ok((
                        Some(candidate.clone()),
                        Some(Err(Failure::new(UpdateStep::Verify, "bad signature"))),
                    )),
                    cx,
                );
                assert!(matches!(
                    updater.state(),
                    UpdateState::Failed {
                        step: UpdateStep::Verify,
                        ..
                    }
                ));
                assert!(updater.prepare_install(&logs).is_err());
                assert!(updater.install_log_path().is_none());
            });
        });
        sweep(dir.path(), Some("20260907.0100"), Some(&candidate), true).unwrap();
        assert!(path.exists());
        assert!(attempt_path(&path).exists());
        sweep(dir.path(), Some("20260908.0100"), None, false).unwrap();
        assert!(!path.exists());
        assert!(!attempt_path(&path).exists());
    }

    #[test]
    fn check_and_transfer_transitions_keep_sidebar_and_retry_state_consistent() {
        let dir = tempfile::tempdir().unwrap();
        let cx = gpui::TestAppContext::single();
        cx.update(|cx| {
            let updater = cx.new(|cx| Updater::new(false, dir.path().to_owned(), cx));
            updater.update(cx, |updater, cx| {
                assert!(updater.automatically_checks_for_updates());
                updater.set_automatically_checks_for_updates(false, cx);
                assert!(updater.automatically_checks_for_updates());
                updater.finish_windows_check(Ok((Some(candidate()), None)), cx);
                assert!(matches!(updater.state(), UpdateState::Available { sig_url: Some(_), .. }));
                assert!(updater.available().is_some());
                let checked_at = updater.last_check_at();
                updater.finish_windows_check(Err(anyhow::anyhow!("offline")), cx);
                assert_eq!(updater.last_check_at(), checked_at);
                assert!(updater.available().is_some());
                for step in [UpdateStep::Download, UpdateStep::Verify] {
                    updater.state = UpdateState::Downloading { received: 8, total: 16 };
                    assert!(updater.available().is_none());
                    updater.finish_download(Err(Failure::new(step, "test error")), cx);
                    assert!(matches!(updater.state(), UpdateState::Failed { step: actual, .. } if *actual == step));
                    assert!(updater.available().is_some());
                }
                updater.finish_download(Ok(None), cx);
                assert!(matches!(updater.state(), UpdateState::Available { .. }));
                updater.finish_download(Ok(Some(dir.path().join(&candidate().name))), cx);
                assert!(matches!(updater.state(), UpdateState::Ready { .. }));
                updater.finish_windows_check(Err(anyhow::anyhow!("offline")), cx);
                assert!(matches!(updater.state(), UpdateState::Ready { .. }));
                updater.finish_windows_check(Ok((None, None)), cx);
                assert_eq!(*updater.state(), UpdateState::UpToDate { checking: false });
                assert!(updater.available().is_none());
            });
        });
    }
}
