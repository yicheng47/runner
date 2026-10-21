use std::ffi::CString;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Component;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use anyhow::{ensure, Context as _};
use flate2::read::GzDecoder;

use super::*;

const INSTALLER_FLAG: &str = "--runner-install-update";
const MAX_PACKAGE_BYTES: u64 = 1024 * 1024 * 1024;
const ARCH: &str = std::env::consts::ARCH;
static RELAUNCH: Mutex<Option<PathBuf>> = Mutex::new(None);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InstallKind {
    Tar,
    Deb,
    Rpm,
}

impl InstallKind {
    fn extension(self) -> &'static str {
        match self {
            Self::Tar => "tar.gz",
            Self::Deb => "deb",
            Self::Rpm => "rpm",
        }
    }

    fn detect(executable: &Path) -> Result<Self> {
        let package = executable
            .parent()
            .context("Runner has no install directory")?;
        ensure!(
            fs::read_to_string(package.join("install-layout-v1"))?.trim()
                == "runner-linux-package-v1",
            "Not a managed Runner installation"
        );
        if executable == Path::new("/usr/lib/runner/Runner") {
            trusted_system_executable(executable)?;
            if Command::new("/usr/bin/dpkg-query")
                .args(["-S", "/usr/lib/runner/Runner"])
                .output()
                .is_ok_and(|out| {
                    out.status.success()
                        && String::from_utf8_lossy(&out.stdout)
                            .lines()
                            .any(|line| line == "runner: /usr/lib/runner/Runner")
                })
            {
                return Ok(Self::Deb);
            }
            if Command::new("/usr/bin/rpm")
                .args(["-qf", "--queryformat", "%{NAME}", "/usr/lib/runner/Runner"])
                .output()
                .is_ok_and(|out| out.status.success() && out.stdout == b"runner")
            {
                return Ok(Self::Rpm);
            }
            bail!("Runner is not owned by the runner system package");
        }
        ensure!(
            package.file_name().is_some_and(|name| name == "runner")
                && package
                    .parent()
                    .is_some_and(|dir| dir.file_name().is_some_and(|name| name == "lib")),
            "Unsupported Runner install layout"
        );
        ensure!(
            package.metadata()?.uid() == unsafe { libc::geteuid() }
                && unsafe { libc::geteuid() } != 0,
            "Use the system package manager for system installations"
        );
        Ok(Self::Tar)
    }
}

fn trusted_system_executable(path: &Path) -> Result<()> {
    ensure!(
        path == Path::new("/usr/lib/runner/Runner"),
        "Unexpected system installer path"
    );
    for path in path.ancestors() {
        let meta = fs::symlink_metadata(path)?;
        ensure!(
            !meta.file_type().is_symlink() && meta.uid() == 0 && meta.mode() & 0o022 == 0,
            "System installer must be owned by root and not writable by other users"
        );
    }
    Ok(())
}

pub(super) fn installer_version(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix("Runner-")?;
    let (label, platform) = rest.rsplit_once("-linux-")?;
    let extension = ["tar.gz", "deb", "rpm"]
        .into_iter()
        .find_map(|ext| platform.strip_suffix(&format!(".{ext}")))?;
    if !matches!(extension, "aarch64" | "x86_64") {
        return None;
    }
    if semver::Version::parse(label)
        .is_ok_and(|version| version.pre.is_empty() && version.build.is_empty())
    {
        return Some((label, ""));
    }
    let nightly = label
        .strip_prefix("nightly.")
        .or_else(|| label.strip_prefix("remote-nightly."))?;
    let (sha, stamp) = nightly.split_once('.')?;
    let (date, time) = stamp.split_once('.')?;
    (sha.len() == 7
        && sha.bytes().all(|c| c.is_ascii_hexdigit())
        && date.len() == 8
        && time.len() == 4
        && date.bytes().chain(time.bytes()).all(|c| c.is_ascii_digit()))
    .then_some((label, stamp))
}

fn newer(
    label: &str,
    stamp: &str,
    channel: &str,
    installed_version: &str,
    installed_stamp: Option<&str>,
) -> bool {
    if channel == "production" {
        let version = semver::Version::parse(label);
        let installed = semver::Version::parse(installed_version);
        return matches!((version, installed), (Ok(version), Ok(installed)) if version > installed);
    }
    label.starts_with(&format!("{channel}."))
        && installed_stamp.is_some_and(|installed| stamp > installed)
}

fn select_update(
    release: &Release,
    kind: InstallKind,
    arch: &str,
    channel: &str,
    version: &str,
    stamp: Option<&str>,
) -> Result<Option<Candidate>> {
    let suffix = format!("-linux-{arch}.{}", kind.extension());
    let mut candidates = release
        .assets
        .iter()
        .filter(|asset| asset.state == "uploaded" && asset.name.ends_with(&suffix))
        .filter_map(|asset| {
            let (label, candidate_stamp) = installer_version(&asset.name)?;
            newer(label, candidate_stamp, channel, version, stamp).then_some((
                asset,
                label,
                candidate_stamp,
            ))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| {
        if channel == "production" {
            semver::Version::parse(a.1)
                .ok()
                .cmp(&semver::Version::parse(b.1).ok())
        } else {
            a.2.cmp(b.2)
        }
    });
    Ok(candidates.pop().map(|(asset, label, _)| Candidate {
        info: UpdateInfo::new(label),
        name: asset.name.clone(),
        installer_url: asset.browser_download_url.clone(),
        sig_url: release
            .assets
            .iter()
            .find(|sig| sig.state == "uploaded" && sig.name == format!("{}.sig", asset.name))
            .map(|sig| sig.browser_download_url.clone()),
        size: asset.size,
    }))
}

pub(super) fn available_update(
    release: &Release,
    stamp: Option<&str>,
) -> Result<Option<Candidate>> {
    let executable = std::env::current_exe()?;
    // Source builds can check releases but have no managed installation to replace.
    let kind = InstallKind::detect(&executable).unwrap_or(InstallKind::Tar);
    select_update(
        release,
        kind,
        ARCH,
        option_env!("RUNNER_RELEASE_CHANNEL").unwrap_or("nightly"),
        env!("CARGO_PKG_VERSION"),
        stamp,
    )
}

pub(super) fn attempt_path(path: &Path) -> PathBuf {
    path.with_file_name(format!(
        "{}.attempt",
        path.file_name().unwrap().to_string_lossy()
    ))
}

pub(super) fn sweep(
    dir: &Path,
    _: Option<&str>,
    candidate: Option<&Candidate>,
    known: bool,
) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let package = name
            .strip_suffix(".partial")
            .or_else(|| name.strip_suffix(".attempt"))
            .or_else(|| name.strip_suffix(".sig"))
            .unwrap_or(&name);
        if installer_version(package).is_some()
            && (name.ends_with(".partial")
                || (known && candidate.is_none_or(|candidate| candidate.name != package)))
        {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn copy_bounded(source: &Path, destination: &Path, limit: u64) -> Result<()> {
    let mut input = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(source)?;
    ensure!(
        input.metadata()?.is_file() && input.metadata()?.len() <= limit,
        "Invalid update file size or type"
    );
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(destination)?;
    let copied = std::io::copy(
        &mut std::io::Read::by_ref(&mut input).take(limit + 1),
        &mut output,
    )?;
    ensure!(copied <= limit, "Update file is too large");
    output.sync_all()?;
    Ok(())
}

fn pinned_package(path: &Path, signature: &Path, dir: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("Invalid package name")?;
    let (label, stamp) = installer_version(name).context("Invalid Linux update name")?;
    ensure!(
        name.contains(&format!("-linux-{ARCH}.")),
        "Update architecture does not match"
    );
    ensure!(
        newer(
            label,
            stamp,
            option_env!("RUNNER_RELEASE_CHANNEL").unwrap_or("nightly"),
            env!("CARGO_PKG_VERSION"),
            option_env!("RUNNER_BUILD_STAMP")
        ),
        "Update channel or version does not match this installation"
    );
    let target = dir.join(name);
    copy_bounded(path, &target, MAX_PACKAGE_BYTES)?;
    let sig = dir.join("signature");
    copy_bounded(signature, &sig, 64 * 1024)?;
    verify_file(&target, &fs::read_to_string(sig)?, PUBLIC_KEY)?;
    Ok(target)
}

fn unpack_package(path: &Path, destination: &Path) -> Result<()> {
    let stem = path
        .file_name()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_suffix(".tar.gz"))
        .context("Expected tar update")?;
    let mut archive = tar::Archive::new(GzDecoder::new(File::open(path)?));
    let mut total = 0;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        ensure!(
            path.components()
                .all(|component| matches!(component, Component::Normal(_))),
            "Invalid archive path"
        );
        let relative = path.strip_prefix(stem).context("Unexpected archive root")?;
        ensure!(
            entry.header().entry_type().is_dir() || entry.header().entry_type().is_file(),
            "Links and special archive entries are unsupported"
        );
        total += entry.size();
        ensure!(
            total <= 2 * MAX_PACKAGE_BYTES,
            "Expanded update is too large"
        );
        if let Ok(file) = relative.strip_prefix("lib/runner") {
            if entry.header().entry_type().is_dir() {
                continue;
            }
            ensure!(
                file.components().count() == 1
                    && matches!(
                        file.to_str(),
                        Some(
                            "Runner"
                                | "runner-agent-cli"
                                | "runnerd"
                                | "install-layout-v1"
                                | "install.sh"
                                | "icon.png"
                                | "build-platform.txt"
                                | "release-info.json"
                        )
                    ),
                "Unexpected application file"
            );
            let output = destination.join(file);
            let mut target = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&output)?;
            std::io::copy(&mut entry, &mut target)?;
            target.sync_all()?;
            fs::set_permissions(
                output,
                fs::Permissions::from_mode(
                    if matches!(
                        file.to_str(),
                        Some("Runner" | "runner-agent-cli" | "runnerd" | "install.sh")
                    ) {
                        0o755
                    } else {
                        0o644
                    },
                ),
            )?;
        }
    }
    ensure!(
        fs::read_to_string(destination.join("install-layout-v1"))?.trim()
            == "runner-linux-package-v1",
        "Unknown update layout"
    );
    let info: serde_json::Value =
        serde_json::from_slice(&fs::read(destination.join("release-info.json"))?)?;
    ensure!(
        info["label"].as_str()
            == installer_version(path.file_name().unwrap().to_str().unwrap()).map(|pair| pair.0)
            && info["arch"] == ARCH,
        "Archive identity does not match its release name"
    );
    for name in ["Runner", "runner-agent-cli", "runnerd"] {
        let mut header = [0; 20];
        File::open(destination.join(name))?.read_exact(&mut header)?;
        ensure!(
            &header[..6] == b"\x7fELF\x02\x01"
                && u16::from_le_bytes([header[18], header[19]])
                    == if ARCH == "aarch64" { 183 } else { 62 },
            "Invalid executable architecture"
        );
    }
    ensure!(
        fs::read(destination.join("runner-agent-cli"))? == fs::read(destination.join("runnerd"))?,
        "Daemon and CLI payloads differ"
    );
    Ok(())
}

fn exchange(first: &Path, second: &Path) -> Result<()> {
    use std::os::unix::ffi::OsStrExt as _;
    let first = CString::new(first.as_os_str().as_bytes())?;
    let second = CString::new(second.as_os_str().as_bytes())?;
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            first.as_ptr(),
            libc::AT_FDCWD,
            second.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };
    ensure!(
        result == 0,
        "Could not atomically replace Runner: {}",
        std::io::Error::last_os_error()
    );
    Ok(())
}

fn install_tar(path: &Path, executable: &Path) -> Result<()> {
    ensure!(
        InstallKind::detect(executable)? == InstallKind::Tar,
        "Not a user tar installation"
    );
    let package = executable.parent().unwrap();
    let stage = tempfile::Builder::new()
        .prefix(".runner-update-")
        .tempdir_in(package.parent().unwrap())?;
    let next = stage.path().join("runner");
    fs::create_dir(&next)?;
    unpack_package(path, &next)?;
    // Both trees are on the same filesystem. A crash cannot expose mixed sidecars.
    exchange(&next, package)?;
    Ok(())
}

fn package_fields(program: &str, args: &[&str], package: &Path) -> Result<String> {
    let out = Command::new(program)
        .args(args)
        .arg(package)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LC_ALL", "C")
        .output()?;
    ensure!(
        out.status.success(),
        "Package metadata check failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(String::from_utf8(out.stdout)?.trim().into())
}

fn system_command(kind: InstallKind, package: &Path) -> Result<Command> {
    let (label, _) = installer_version(package.file_name().unwrap().to_str().unwrap())
        .context("Invalid package")?;
    match kind {
        InstallKind::Deb => {
            ensure!(
                package.extension().is_some_and(|ext| ext == "deb"),
                "Expected a deb update"
            );
            // dpkg-deb's field follows the package path.
            let field = |field: &str| -> Result<String> {
                let out = Command::new("/usr/bin/dpkg-deb")
                    .arg("-f")
                    .arg(package)
                    .arg(field)
                    .output()?;
                ensure!(out.status.success(), "Could not read deb metadata");
                Ok(String::from_utf8(out.stdout)?.trim().into())
            };
            ensure!(
                field("Package")? == "runner"
                    && field("Architecture")? == if ARCH == "aarch64" { "arm64" } else { "amd64" },
                "Unexpected deb package identity"
            );
            let version = field("Version")?;
            ensure!(
                package_version_matches(label, &version, None),
                "Deb version does not match update identity"
            );
            let mut command = Command::new("/usr/bin/apt-get");
            command
                .args([
                    "-y",
                    "--no-remove",
                    "-o",
                    "DPkg::Lock::Timeout=60",
                    "install",
                ])
                .arg(package);
            Ok(command)
        }
        InstallKind::Rpm => {
            ensure!(
                package.extension().is_some_and(|ext| ext == "rpm"),
                "Expected an rpm update"
            );
            let fields = package_fields(
                "/usr/bin/rpm",
                &[
                    "-qp",
                    "--queryformat",
                    "%{NAME}\n%{ARCH}\n%{VERSION}\n%{RELEASE}",
                ],
                package,
            )?;
            let fields = fields.lines().collect::<Vec<_>>();
            ensure!(
                fields.len() == 4
                    && fields[0] == "runner"
                    && fields[1] == ARCH
                    && package_version_matches(label, fields[2], Some(fields[3])),
                "Unexpected rpm package identity"
            );
            let program = ["/usr/bin/dnf", "/usr/bin/dnf5"]
                .into_iter()
                .find(|path| Path::new(path).is_file())
                .context("Installing RPM updates requires DNF")?;
            let mut command = Command::new(program);
            // Detached Runner signatures were verified before this point; repository
            // dependency signatures remain checked by DNF.
            command
                .args(["-y", "--setopt=localpkg_gpgcheck=0", "install"])
                .arg(package);
            Ok(command)
        }
        InstallKind::Tar => bail!("System installer cannot replace a user installation"),
    }
}

fn package_version_matches(label: &str, version: &str, release: Option<&str>) -> bool {
    if semver::Version::parse(label).is_ok() {
        return version == label && release.is_none_or(|release| release == "1");
    }
    let (channel, remainder) = if let Some(rest) = label.strip_prefix("remote-nightly.") {
        ("remote.nightly", rest)
    } else if let Some(rest) = label.strip_prefix("nightly.") {
        ("nightly", rest)
    } else {
        return false;
    };
    let Some((sha, stamp)) = remainder.split_once('.') else {
        return false;
    };
    if let Some(release) = release {
        release == format!("0.{channel}.{stamp}.{sha}")
    } else {
        version.split_once('~').is_some_and(|(base, suffix)| {
            semver::Version::parse(base).is_ok() && suffix == format!("{channel}.{stamp}.{sha}")
        })
    }
}

/// Handles only the narrow installation mode, before GPUI, SQLite or agents start.
pub fn run_installer() -> Option<Result<()>> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(INSTALLER_FLAG)) {
        return None;
    }
    Some((|| {
        ensure!(
            unsafe { libc::geteuid() } == 0,
            "System update installation requires authorization"
        );
        let executable = std::env::current_exe()?;
        let kind = InstallKind::detect(&executable)?;
        ensure!(kind != InstallKind::Tar, "Expected a system installation");
        let path = PathBuf::from(args.next().context("Missing update package")?);
        let signature = PathBuf::from(args.next().context("Missing signature")?);
        ensure!(args.next().is_none(), "Unexpected installer arguments");
        let stage = tempfile::Builder::new()
            .prefix("runner-update-")
            .tempdir_in("/var/tmp")?;
        let package = pinned_package(&path, &signature, stage.path())?;
        let status = system_command(kind, &package)?
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .env("LC_ALL", "C.UTF-8")
            .env("DEBIAN_FRONTEND", "noninteractive")
            .stdin(Stdio::null())
            .status()?;
        ensure!(
            status.success(),
            "Package manager failed ({status}); the application has not been restarted"
        );
        Ok(())
    })())
}

fn install(path: &Path, log: &Path) -> Result<PathBuf> {
    let result = install_inner(path, log);
    if let Err(error) = &result {
        if let Ok(mut output) = fs::OpenOptions::new().create(true).append(true).open(log) {
            let _ = writeln!(output, "Runner update failed: {error:#}");
        }
    }
    result
}

fn install_inner(path: &Path, log: &Path) -> Result<PathBuf> {
    fs::create_dir_all(log.parent().context("Missing update log directory")?)?;
    let executable = std::env::current_exe()?;
    let kind = InstallKind::detect(&executable)?;
    let signature = path.with_file_name(format!(
        "{}.sig",
        path.file_name().unwrap().to_string_lossy()
    ));
    let stage = tempfile::Builder::new()
        .prefix("runner-update-")
        .tempdir()?;
    let package = pinned_package(path, &signature, stage.path())?;
    if kind == InstallKind::Tar {
        fs::write(log, "Installing verified Runner update.\n")?;
        install_tar(&package, &executable)?;
        let _ = fs::write(
            log,
            "Runner application files were atomically replaced. Restarting.\n",
        );
    } else {
        let log_file = File::create(log)?;
        let mut child = Command::new("/usr/bin/pkexec")
            .arg(&executable)
            .arg(INSTALLER_FLAG)
            .arg(&package)
            .arg(&signature)
            .stdin(Stdio::null())
            .stdout(log_file.try_clone()?)
            .stderr(log_file)
            .spawn()
            .context("Could not request system authorization (pkexec is required)")?;
        let status = child.wait()?;
        ensure!(
            status.success(),
            "System installation failed or authorization was cancelled ({status}). See {}",
            log.display()
        );
    }
    Ok(executable)
}

impl Updater {
    pub fn is_installing(&self) -> bool {
        self.native.installing
    }

    pub fn install_update(&mut self, log_dir: &Path, cx: &mut Context<Self>) {
        if self.native.installing || self.native.checking.get() {
            return;
        }
        let Some(path) = self.native.verified_path.clone() else {
            return;
        };
        let log = log_dir.join("linux-install.log");
        if let Err(error) = fs::create_dir_all(log_dir)
            .and_then(|()| fs::write(attempt_path(&path), log.to_string_lossy().as_bytes()))
        {
            self.fail_install(error, cx);
            return;
        }
        self.native.install_log = Some(log.clone());
        self.native.installing = true;
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { install(&path, &log) })
                .await;
            let _ = weak.update(cx, |updater, cx| {
                updater.native.installing = false;
                match result {
                    Ok(path) => {
                        *RELAUNCH.lock().unwrap() = Some(path);
                        crate::lifecycle::mark_update_quit();
                        cx.quit();
                    }
                    Err(error) => updater.fail_install(format!("{error:#}"), cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

pub fn relaunch() -> Result<()> {
    if let Some(path) = RELAUNCH.lock().unwrap().take() {
        Command::new(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Could not reopen Runner")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn release(names: &[&str]) -> Release {
        serde_json::from_value(serde_json::json!({"assets": names.iter().map(|name| serde_json::json!({"name":name,"state":"uploaded","browser_download_url":format!("https://example.com/{name}"),"size":44})).collect::<Vec<_>>()})).unwrap()
    }

    #[test]
    fn selection_preserves_architecture_format_channel_and_semantic_order() {
        let source = release(&[
            "Runner-0.17.2-linux-aarch64.deb",
            "Runner-0.17.2-linux-aarch64.deb.sig",
            "Runner-0.17.10-linux-aarch64.deb",
            "Runner-0.17.10-linux-aarch64.deb.sig",
            "Runner-99.0.0-linux-x86_64.deb",
            "Runner-99.0.0-linux-aarch64.rpm",
            "Runner-nightly.aaaaaaa.20261020.1700-linux-aarch64.deb",
        ]);
        let candidate = select_update(
            &source,
            InstallKind::Deb,
            "aarch64",
            "production",
            "0.17.1",
            Some("20261020.1500"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(candidate.name, "Runner-0.17.10-linux-aarch64.deb");
        assert!(candidate.sig_url.is_some());
        assert!(select_update(
            &source,
            InstallKind::Deb,
            "aarch64",
            "production",
            "0.18.0",
            None
        )
        .unwrap()
        .is_none());
        assert!(select_update(
            &source,
            InstallKind::Deb,
            "aarch64",
            "remote-nightly",
            "0.17.1",
            Some("20261020.1500")
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn nightly_stamp_order_is_independent_of_sha_and_stable_version() {
        let source = release(&[
            "Runner-nightly.fffffff.20261020.1500-linux-x86_64.tar.gz",
            "Runner-nightly.aaaaaaa.20261020.1700-linux-x86_64.tar.gz",
            "Runner-remote-nightly.bbbbbbb.20261020.1800-linux-x86_64.tar.gz",
            "Runner-99.0.0-linux-x86_64.tar.gz",
        ]);
        let candidate = select_update(
            &source,
            InstallKind::Tar,
            "x86_64",
            "nightly",
            "0.17.1",
            Some("20261020.1600"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            candidate.name,
            "Runner-nightly.aaaaaaa.20261020.1700-linux-x86_64.tar.gz"
        );
        assert!(candidate.sig_url.is_none());
        assert!(select_update(
            &source,
            InstallKind::Tar,
            "x86_64",
            "nightly",
            "0.17.1",
            None
        )
        .unwrap()
        .is_none());
        for invalid in [
            "Runner-../../escape-linux-aarch64.deb",
            "Runner-nightly.sha.bad-linux-x86_64.rpm",
            "Runner-1.0.0-linux-ppc64le.deb",
            "Runner-1.0.0+bad-linux-aarch64.rpm",
        ] {
            assert!(installer_version(invalid).is_none(), "{invalid}");
        }
    }

    #[test]
    fn package_metadata_matches_release_channel_and_stamp() {
        assert!(package_version_matches("1.2.3", "1.2.3", None));
        assert!(!package_version_matches("1.2.3", "1.2.2", None));
        assert!(package_version_matches(
            "nightly.aaaaaaa.20261020.1700",
            "1.2.3~nightly.20261020.1700.aaaaaaa",
            None
        ));
        assert!(package_version_matches(
            "remote-nightly.aaaaaaa.20261020.1700",
            "1.2.3",
            Some("0.remote.nightly.20261020.1700.aaaaaaa")
        ));
        assert!(!package_version_matches(
            "nightly.aaaaaaa.20261020.1700",
            "1.2.3~remote.nightly.20261020.1700.aaaaaaa",
            None
        ));
        assert!(!package_version_matches(
            "1.2.3",
            "1.2.3",
            Some("0.nightly.20261020.1700.aaaaaaa")
        ));
    }

    fn append(
        builder: &mut tar::Builder<flate2::write::GzEncoder<File>>,
        path: &str,
        bytes: &[u8],
    ) {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append_data(&mut header, path, bytes).unwrap();
    }

    fn archive(root: &Path, label: &str, bad_sidecar: bool, link: bool) -> PathBuf {
        archive_with_identity(
            root,
            label,
            bad_sidecar,
            link,
            label,
            if ARCH == "aarch64" { 183 } else { 62 },
        )
    }

    fn archive_with_identity(
        root: &Path,
        label: &str,
        bad_sidecar: bool,
        link: bool,
        identity: &str,
        machine: u16,
    ) -> PathBuf {
        let name = format!("Runner-{label}-linux-{ARCH}");
        let path = root.join(format!("{name}.tar.gz"));
        let gz = flate2::write::GzEncoder::new(
            File::create(&path).unwrap(),
            flate2::Compression::fast(),
        );
        let mut builder = tar::Builder::new(gz);
        let mut elf = vec![0; 20];
        elf[..6].copy_from_slice(b"\x7fELF\x02\x01");
        elf[18..20].copy_from_slice(&machine.to_le_bytes());
        for binary in ["Runner", "runner-agent-cli", "runnerd"] {
            let mut bytes = elf.clone();
            if binary == "runnerd" && bad_sidecar {
                bytes.push(1);
            }
            append(&mut builder, &format!("{name}/lib/runner/{binary}"), &bytes);
        }
        append(
            &mut builder,
            &format!("{name}/lib/runner/install-layout-v1"),
            b"runner-linux-package-v1\n",
        );
        append(
            &mut builder,
            &format!("{name}/lib/runner/release-info.json"),
            serde_json::json!({"label":identity,"arch":ARCH})
                .to_string()
                .as_bytes(),
        );
        if link {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            header.set_mode(0o777);
            header.set_link_name("/tmp/runner-escape").unwrap();
            header.set_cksum();
            builder
                .append_data(&mut header, format!("{name}/link"), std::io::empty())
                .unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
        path
    }

    #[test]
    #[ignore = "requires a locally built Linux tar package"]
    fn actual_packaged_tar_passes_the_updater_extractor() {
        let package = PathBuf::from(
            std::env::var_os("RUNNER_TEST_LINUX_PACKAGE").expect("RUNNER_TEST_LINUX_PACKAGE"),
        );
        let destination = tempfile::tempdir().unwrap();
        unpack_package(&package, destination.path()).unwrap();
        assert!(destination.path().join("Runner").is_file());
        assert!(destination.path().join("release-info.json").is_file());
        assert_eq!(
            fs::read(destination.path().join("runnerd")).unwrap(),
            fs::read(destination.path().join("runner-agent-cli")).unwrap()
        );
    }

    #[test]
    fn atomic_tar_replacement_keeps_user_data_and_all_sidecars_together() {
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("lib/runner");
        fs::create_dir_all(&package).unwrap();
        fs::write(
            package.join("install-layout-v1"),
            "runner-linux-package-v1\n",
        )
        .unwrap();
        for binary in ["Runner", "runner-agent-cli", "runnerd"] {
            fs::write(package.join(binary), "old").unwrap();
        }
        fs::write(root.path().join("user-data"), "history").unwrap();
        let update = archive(root.path(), "99.0.0", false, false);
        install_tar(&update, &package.join("Runner")).unwrap();
        assert_eq!(
            fs::read(package.join("runnerd")).unwrap(),
            fs::read(package.join("runner-agent-cli")).unwrap()
        );
        assert_ne!(fs::read(package.join("Runner")).unwrap(), b"old");
        assert_eq!(
            fs::read_to_string(root.path().join("user-data")).unwrap(),
            "history"
        );
        assert_eq!(fs::read_dir(root.path().join("lib")).unwrap().count(), 1);
    }

    #[test]
    fn failed_archive_validation_preserves_the_old_installation() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("lib/runner");
        fs::create_dir_all(&package).unwrap();
        fs::write(
            package.join("install-layout-v1"),
            "runner-linux-package-v1\n",
        )
        .unwrap();
        fs::write(package.join("Runner"), "old").unwrap();
        for (bad_sidecar, link) in [(true, false), (false, true)] {
            let update = archive(root.path(), "99.0.0", bad_sidecar, link);
            assert!(install_tar(&update, &package.join("Runner")).is_err());
            assert_eq!(fs::read(package.join("Runner")).unwrap(), b"old");
        }
    }

    #[test]
    fn archive_identity_and_elf_must_match_before_replacement() {
        let root = tempfile::tempdir().unwrap();
        for (identity, machine) in [
            ("98.0.0", if ARCH == "aarch64" { 183 } else { 62 }),
            ("99.0.0", 0),
        ] {
            let path =
                archive_with_identity(root.path(), "99.0.0", false, false, identity, machine);
            let destination = tempfile::tempdir().unwrap();
            assert!(unpack_package(&path, destination.path()).is_err());
        }
    }

    #[test]
    fn traversal_and_oversized_archive_headers_are_rejected_before_writing() {
        let root = tempfile::tempdir().unwrap();
        let name = format!("Runner-99.0.0-linux-{ARCH}");
        let path = root.path().join(format!("{name}.tar.gz"));
        for (entry, size) in [
            (format!("{name}/../escape"), 0),
            (
                format!("{name}/lib/runner/Runner"),
                2 * MAX_PACKAGE_BYTES + 1,
            ),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(size);
            header.set_mode(0o755);
            header.as_mut_bytes()[..entry.len()].copy_from_slice(entry.as_bytes());
            header.set_cksum();
            let mut output = flate2::write::GzEncoder::new(
                File::create(&path).unwrap(),
                flate2::Compression::fast(),
            );
            output.write_all(header.as_bytes()).unwrap();
            output.write_all(&[0; 1024]).unwrap();
            output.finish().unwrap();
            let destination = tempfile::tempdir().unwrap();
            assert!(unpack_package(&path, destination.path()).is_err());
            assert_eq!(fs::read_dir(destination.path()).unwrap().count(), 0);
            assert!(!root.path().join("escape").exists());
        }
    }

    #[test]
    fn staged_copy_refuses_symlinks_and_size_overflow() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("source");
        fs::write(&original, "too long").unwrap();
        let alias = root.path().join("alias");
        symlink(&original, &alias).unwrap();
        assert!(copy_bounded(&alias, &root.path().join("out"), 50).is_err());
        assert!(copy_bounded(&original, &root.path().join("out"), 2).is_err());
        assert!(trusted_system_executable(&original).is_err());
    }

    #[test]
    fn deb_metadata_is_checked_before_building_the_system_command() {
        let root = tempfile::tempdir().unwrap();
        let tree = root.path().join("package/DEBIAN");
        fs::create_dir_all(&tree).unwrap();
        let arch = if ARCH == "aarch64" { "arm64" } else { "amd64" };
        let path = root.path().join(format!("Runner-99.0.0-linux-{ARCH}.deb"));
        for (name, version, valid) in [
            ("runner", "99.0.0", true),
            ("other", "99.0.0", false),
            ("runner", "98.0.0", false),
        ] {
            fs::write(tree.join("control"), format!("Package: {name}\nVersion: {version}\nArchitecture: {arch}\nMaintainer: Fixture <fixture@example.com>\nDescription: update test\n")).unwrap();
            let output = Command::new("/usr/bin/dpkg-deb")
                .arg("--build")
                .arg(tree.parent().unwrap())
                .arg(&path)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let command = system_command(InstallKind::Deb, &path);
            assert_eq!(command.is_ok(), valid);
            if let Ok(command) = command {
                assert_eq!(command.get_program(), "/usr/bin/apt-get");
                let args = command.get_args().collect::<Vec<_>>();
                assert_eq!(args.last().unwrap(), &path.as_os_str());
                assert!(args.iter().any(|arg| *arg == "--no-remove"));
            }
        }
    }

    fn http_fixture(responses: Vec<Vec<u8>>) -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = std::thread::spawn(move || {
            for body in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut header = Vec::new();
                while !header.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    header.push(byte[0]);
                }
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        (url, task)
    }

    #[test]
    fn linux_download_retains_signature_and_rejects_tampering_without_final_files() {
        let root = tempfile::tempdir().unwrap();
        let fixture = include_bytes!("../../../tests/fixtures/updates/installer.txt");
        let signature = include_bytes!("../../../tests/fixtures/updates/installer.txt.sig");
        let key = include_str!("../../../tests/fixtures/updates/public-key");
        for valid in [true, false] {
            let dir = root.path().join(if valid { "valid" } else { "invalid" });
            let (url, server) = http_fixture(vec![
                if valid {
                    fixture.to_vec()
                } else {
                    b"tampered".to_vec()
                },
                signature.to_vec(),
            ]);
            let candidate = Candidate {
                info: UpdateInfo::new("99.0.0"),
                name: format!("Runner-99.0.0-linux-{ARCH}.deb"),
                installer_url: url.clone(),
                sig_url: Some(format!("{url}/signature")),
                size: fixture.len() as u64,
            };
            let result = download(&candidate, &dir, &Transfer::default(), key);
            server.join().unwrap();
            assert_eq!(result.is_ok(), valid);
            assert!(!dir.join(format!("{}.partial", candidate.name)).exists());
            if valid {
                let path = result.unwrap().unwrap();
                assert_eq!(fs::read(&path).unwrap(), fixture);
                assert_eq!(
                    fs::read(dir.join(format!("{}.sig", candidate.name))).unwrap(),
                    signature
                );
                let (url, server) = http_fixture(vec![signature.to_vec()]);
                let cached = Candidate {
                    sig_url: Some(url),
                    ..candidate
                };
                assert_eq!(
                    download(&cached, &dir, &Transfer::default(), key).unwrap(),
                    Some(path)
                );
                server.join().unwrap();
            } else {
                assert!(!dir.join(&candidate.name).exists());
            }
        }
    }

    #[test]
    fn cancellation_removes_partial_and_signature_tampering_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let fixture = include_bytes!("../../../tests/fixtures/updates/installer.txt");
        let signature = include_str!("../../../tests/fixtures/updates/installer.txt.sig");
        let key = include_str!("../../../tests/fixtures/updates/public-key");
        let path = root.path().join("payload");
        fs::write(&path, fixture).unwrap();
        verify_file(&path, signature, key).unwrap();
        fs::write(&path, "tampered").unwrap();
        assert!(verify_file(&path, signature, key).is_err());
        let transfer = Transfer::default();
        transfer.cancelled.store(true, Ordering::Relaxed);
        assert!(!stream_installer(&mut &fixture[..], &path, &transfer).unwrap());
        let partial = root
            .path()
            .join(format!("Runner-99.0.0-linux-{ARCH}.deb.partial"));
        fs::write(&partial, "partial").unwrap();
        sweep(root.path(), None, None, false).unwrap();
        assert!(!partial.exists());
        assert!(path.exists());
    }
}
