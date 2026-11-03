use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Sits beside a mission slot's `runner.exe` shim. That shim is the CLI itself:
/// finding this file beside its own executable makes it forward to `runner`.
pub const SESSION_SHIM_CONFIG_NAME: &str = "runner-shim.json";

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionShimConfig {
    pub runner: PathBuf,
    /// `None` clears a variable the caller inherited.
    pub env: BTreeMap<String, Option<String>>,
}

/// Source-side agent CLI artifact. Installed into app data as `runner`.
pub const AGENT_SOURCE_BIN_NAME: &str = if cfg!(windows) {
    "runner-agent-cli.exe"
} else {
    "runner-agent-cli"
};

/// Name of the agent CLI we drop into `$APPDATA/runner/bin/`. Must match what
/// `SessionManager::spawn` puts on PATH — arch §5.3 Layer 2 has the
/// CLI being invoked as bare `runner` from inside spawned PTYs.
pub const AGENT_DEST_BIN_NAME: &str = if cfg!(windows) {
    "runner.exe"
} else {
    "runner"
};

/// Legacy MCP bridge name, retained only for upgrade cleanup and registration matching.
pub const MCP_DEST_BIN_NAME: &str = if cfg!(windows) {
    "runner-mcp.exe"
} else {
    "runner-mcp"
};

// Installed before a daemon starts. Mission shims and spawned PATHs consume the destination.
pub fn install_runner_cli(app_data_dir: &Path) -> Result<()> {
    let Some(source) = locate_source(AGENT_SOURCE_BIN_NAME)? else {
        return Ok(());
    };
    install_from_source(app_data_dir, &source)
}

pub fn remove_stale_mcp_cli(app_data_dir: &Path) -> Result<()> {
    let path = app_data_dir.join("bin").join(MCP_DEST_BIN_NAME);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::msg(format!("remove {}: {error}", path.display()))),
    }
}

pub fn install_from_source(app_data_dir: &Path, source: &Path) -> Result<()> {
    install_from_source_hashed(app_data_dir, source, None)
}

/// Like `install_from_source`, with the caller's freshly computed SHA-256
/// of `source`, so the installer does not read the binary again.
pub fn install_from_source_hashed(
    app_data_dir: &Path,
    source: &Path,
    source_sha256: Option<&str>,
) -> Result<()> {
    std::fs::create_dir_all(app_data_dir)?;
    let lock = crate::daemon_process::lock_file(app_data_dir)?;
    install_binary(app_data_dir, source, AGENT_DEST_BIN_NAME, source_sha256)?;
    match lock.try_lock_exclusive() {
        Ok(()) => {
            install_binary(app_data_dir, source, DAEMON_DEST_BIN_NAME, source_sha256)?;
            #[cfg(windows)]
            for name in ["conpty.dll", "OpenConsole.exe"] {
                let companion = source.parent().unwrap().join(name);
                if companion.is_file() {
                    install_binary(app_data_dir, &companion, name, None)?;
                }
            }
        }
        Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => (),
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

/// SHA-256 of `source` as last installed, when the install record beside
/// `$APPDATA/bin/<dest_name>` still matches this launch's app version and
/// the source's size and mtime. Lets a connecting client skip hashing the
/// sidecar on every cold start.
pub fn cached_install_sha256(
    app_data_dir: &Path,
    dest_name: &str,
    source: &Path,
) -> Option<String> {
    let dest = app_data_dir.join("bin").join(dest_name);
    let meta: InstalledMeta = read_json(&meta_path(&dest))?;
    let source_meta = std::fs::metadata(source).ok()?;
    source_matches_meta(&meta, &source_meta).then_some(meta.source_sha256)
}

pub const DAEMON_DEST_BIN_NAME: &str = if cfg!(windows) {
    "runnerd.exe"
} else {
    "runnerd"
};

fn install_binary(
    app_data_dir: &Path,
    source: &Path,
    dest_name: &str,
    known_source_sha256: Option<&str>,
) -> Result<()> {
    let dest_dir = app_data_dir.join("bin");
    std::fs::create_dir_all(&dest_dir)?;
    let dest = dest_dir.join(dest_name);

    let (matched, source_sha256) = dest_matches(source, &dest, known_source_sha256)?;
    if matched {
        return Ok(());
    }
    let source_sha256 = match source_sha256 {
        Some(sha) => sha,
        // The dest did not exist, so there was nothing to compare against.
        None => crate::daemon_process::executable_hash(source)?,
    };

    // Copy via tempfile + rename to keep the swap atomic — a half-written
    // file would crash the next process that runs this sidecar.
    let tmp = tempfile::NamedTempFile::new_in(&dest_dir)?;
    std::fs::copy(source, tmp.path())?;
    #[cfg(windows)]
    let tmp = {
        for index in 0..ASIDE_SLOTS {
            let _ = std::fs::remove_file(aside_path(&dest, index));
        }
        match tmp.persist(&dest) {
            Ok(_) => return Ok(()),
            // A running executable can be renamed but not replaced, and a
            // `runner` started outside Runner can hold this file at any time.
            Err(error) if in_use(&error.error) => {
                move_aside(&dest)?;
                error.file
            }
            Err(error) => return Err(Error::Io(error.error)),
        }
    };
    tmp.persist(&dest).map_err(|e| Error::Io(e.error))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dest, perms)?;
    }
    // Record what was installed so the next launch can trust it instead of
    // hashing both binaries again.
    let source_meta = std::fs::metadata(source)?;
    let dest_len = std::fs::metadata(&dest)?.len();
    let _ = write_meta(&dest, &source_meta, &source_sha256, dest_len);
    Ok(())
}

#[cfg(windows)]
const ASIDE_SLOTS: usize = 8;

#[cfg(windows)]
fn aside_path(dest: &Path, index: usize) -> PathBuf {
    let mut path = dest.as_os_str().to_owned();
    path.push(if index == 0 {
        ".old".to_owned()
    } else {
        format!(".{index}.old")
    });
    path.into()
}

#[cfg(windows)]
fn in_use(error: &std::io::Error) -> bool {
    use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION};
    error.raw_os_error().is_some_and(|code| {
        code == ERROR_ACCESS_DENIED as i32 || code == ERROR_SHARING_VIOLATION as i32
    })
}

#[cfg(windows)]
fn move_aside(dest: &Path) -> Result<()> {
    let aside = (0..ASIDE_SLOTS)
        .map(|index| aside_path(dest, index))
        .find(|aside| !aside.exists())
        .ok_or_else(|| Error::msg(format!("{} is in use", dest.display())))?;
    std::fs::rename(dest, aside)
        .map_err(|error| Error::msg(format!("move {} aside: {error}", dest.display())))
}

pub fn locate_source(source_name: &str) -> Result<Option<PathBuf>> {
    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| Error::msg("current_exe has no parent"))?;
    let candidate = dir.join(source_name);
    // The app executable is `Runner`, so the equality guard only protects
    // future renames from copying the running executable over itself; the
    // candidate must also exist.
    if candidate.exists() && candidate != exe {
        return Ok(Some(candidate));
    }
    Ok(None)
}

/// Install record beside `$APPDATA/bin/<dest_name>`, written whenever a
/// copy or a hash comparison confirms what is installed. Identity fields
/// (app version, source size and mtime) let a later launch skip reading
/// both binaries; a mismatch of any of them reverts to hashing, and dev
/// rebuilds always change the source mtime.
#[derive(Debug, Serialize, Deserialize)]
struct InstalledMeta {
    app_version: String,
    source_len: u64,
    source_modified_nanos: Option<u64>,
    source_sha256: String,
    dest_len: u64,
}

fn meta_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_owned();
    name.push(".meta.json");
    dest.with_file_name(name)
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn meta_mtime_nanos(meta: &std::fs::Metadata) -> Option<u64> {
    meta.modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_nanos()).ok())
}

fn source_matches_meta(meta: &InstalledMeta, source_meta: &std::fs::Metadata) -> bool {
    meta.app_version == crate::version::display_version()
        && meta.source_len == source_meta.len()
        && meta.source_modified_nanos == meta_mtime_nanos(source_meta)
}

fn write_meta(
    dest: &Path,
    source_meta: &std::fs::Metadata,
    source_sha256: &str,
    dest_len: u64,
) -> Result<()> {
    let meta = InstalledMeta {
        app_version: crate::version::display_version(),
        source_len: source_meta.len(),
        source_modified_nanos: meta_mtime_nanos(source_meta),
        source_sha256: source_sha256.to_owned(),
        dest_len,
    };
    std::fs::write(meta_path(dest), serde_json::to_vec(&meta)?).map_err(Error::Io)
}

/// Compare `dest` against the hashed `source`, trusting the install record
/// when its identity still matches. Returns whether the dest is current,
/// plus the source hash when this call computed one.
fn dest_matches(
    source: &Path,
    dest: &Path,
    known_source_sha256: Option<&str>,
) -> Result<(bool, Option<String>)> {
    let Ok(dest_meta) = std::fs::metadata(dest) else {
        return Ok((false, None));
    };
    let source_meta = std::fs::metadata(source)?;
    if let Some(meta) = read_json::<InstalledMeta>(&meta_path(dest)) {
        if source_matches_meta(&meta, &source_meta) && meta.dest_len == dest_meta.len() {
            return Ok((true, None));
        }
    }
    let source_sha256 = match known_source_sha256 {
        Some(sha) => sha.to_owned(),
        None => crate::daemon_process::executable_hash(source)?,
    };
    let matched = source_sha256 == crate::daemon_process::executable_hash(dest)?;
    if matched {
        // Move the match's evidence to disk so later launches skip both hashes.
        let _ = write_meta(dest, &source_meta, &source_sha256, dest_meta.len());
    }
    Ok((matched, Some(source_sha256)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    #[cfg(unix)]
    struct ForkedLockHolder(libc::pid_t);

    #[cfg(unix)]
    impl ForkedLockHolder {
        fn new() -> Self {
            let pid = unsafe { libc::fork() };
            assert!(pid >= 0, "fork: {}", std::io::Error::last_os_error());
            if pid == 0 {
                // Keep inherited descriptors open using only async-signal-safe calls.
                loop {
                    unsafe { libc::pause() };
                }
            }
            Self(pid)
        }
    }

    #[cfg(unix)]
    impl Drop for ForkedLockHolder {
        fn drop(&mut self) {
            unsafe { libc::kill(self.0, libc::SIGKILL) };
            while unsafe { libc::waitpid(self.0, std::ptr::null_mut(), 0) } < 0 {
                if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                    break;
                }
            }
        }
    }

    #[test]
    fn install_records_a_reusable_source_hash() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let source = root.path().join("source");
        std::fs::write(&source, b"BUILD").unwrap();
        install_from_source(&data, &source).unwrap();

        let expected = crate::daemon_process::executable_hash(&source).unwrap();
        assert_eq!(
            cached_install_sha256(&data, DAEMON_DEST_BIN_NAME, &source).as_deref(),
            Some(expected.as_str())
        );

        // A changed source invalidates the record, and the next install
        // replaces the dest and records the new hash.
        std::fs::write(&source, b"NEW BUILD").unwrap();
        assert_eq!(
            cached_install_sha256(&data, DAEMON_DEST_BIN_NAME, &source),
            None
        );
        install_from_source(&data, &source).unwrap();
        let dest = data.join("bin").join(DAEMON_DEST_BIN_NAME);
        assert_eq!(fs::read(&dest).unwrap(), b"NEW BUILD");
        let expected = crate::daemon_process::executable_hash(&source).unwrap();
        assert_eq!(
            cached_install_sha256(&data, DAEMON_DEST_BIN_NAME, &source).as_deref(),
            Some(expected.as_str())
        );
    }

    #[test]
    fn installing_while_daemon_is_locked_updates_only_the_cli() {
        let root = tempfile::tempdir().unwrap();
        let source_dir = root.path().join("source");
        fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join(AGENT_SOURCE_BIN_NAME);
        fs::write(&source, b"OLD-BUILD").unwrap();
        #[cfg(windows)]
        for name in ["conpty.dll", "OpenConsole.exe"] {
            fs::write(source_dir.join(name), b"OLD-COMPANION").unwrap();
        }
        let data = root.path().join("data");
        install_from_source(&data, &source).unwrap();
        let daemon_lock = crate::daemon_process::lock_file(&data).unwrap();
        daemon_lock.try_lock_exclusive().unwrap();
        #[cfg(unix)]
        let _fork = ForkedLockHolder::new();
        fs::write(&source, b"NEW-BUILD").unwrap();
        #[cfg(windows)]
        for name in ["conpty.dll", "OpenConsole.exe"] {
            fs::write(source_dir.join(name), b"NEW-COMPANION").unwrap();
        }
        install_from_source(&data, &source).unwrap();
        let bin = data.join("bin");
        assert_eq!(
            fs::read(bin.join(AGENT_DEST_BIN_NAME)).unwrap(),
            b"NEW-BUILD"
        );
        assert_eq!(
            fs::read(bin.join(DAEMON_DEST_BIN_NAME)).unwrap(),
            b"OLD-BUILD"
        );
        #[cfg(windows)]
        for name in ["conpty.dll", "OpenConsole.exe"] {
            assert_eq!(fs::read(bin.join(name)).unwrap(), b"OLD-COMPANION");
        }
        drop(daemon_lock);
        install_from_source(&data, &source).unwrap();
        assert_eq!(
            fs::read(bin.join(DAEMON_DEST_BIN_NAME)).unwrap(),
            b"NEW-BUILD"
        );
        #[cfg(windows)]
        for name in ["conpty.dll", "OpenConsole.exe"] {
            assert_eq!(fs::read(bin.join(name)).unwrap(), b"NEW-COMPANION");
        }
    }

    #[cfg(windows)]
    #[test]
    fn install_moves_a_running_binary_aside() {
        use std::time::{Duration, Instant};
        if let Some(ready) = std::env::var_os("RUNNER_TEST_HOLD_IMAGE") {
            fs::write(ready, b"").unwrap();
            std::thread::sleep(Duration::from_secs(60));
            return;
        }
        let data = tempfile::tempdir().unwrap();
        let bin = data.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let dest = bin.join(AGENT_DEST_BIN_NAME);
        fs::copy(std::env::current_exe().unwrap(), &dest).unwrap();
        // Like a long-running sidecar, the copy is past startup (and any
        // antivirus scan of the new file) before it is replaced.
        let ready = data.path().join("ready");
        let mut running = std::process::Command::new(&dest)
            .args([
                "--exact",
                "cli_install::tests::install_moves_a_running_binary_aside",
            ])
            .env("RUNNER_TEST_HOLD_IMAGE", &ready)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while !ready.exists() {
            assert!(Instant::now() < deadline, "running copy never started");
            std::thread::sleep(Duration::from_millis(50));
        }
        let source = data.path().join("source.exe");
        fs::write(&source, b"NEW-BUILD").unwrap();
        let installed = install_binary(data.path(), &source, AGENT_DEST_BIN_NAME);
        running.kill().unwrap();
        running.wait().unwrap();
        installed.unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"NEW-BUILD");
        assert!(bin.join("runner.exe.old").exists());

        fs::write(&source, b"NEWER-BUILD").unwrap();
        install_binary(data.path(), &source, AGENT_DEST_BIN_NAME).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"NEWER-BUILD");
        assert!(!bin.join("runner.exe.old").exists());
    }

    #[test]
    fn same_size_older_source_replaces_both_aliases() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("old");
        let source = root.path().join("source");
        fs::write(&old, b"OLD-BUILD").unwrap();
        fs::write(&source, b"NEW-BUILD").unwrap();
        fs::File::options()
            .write(true)
            .open(&source)
            .unwrap()
            .set_times(
                fs::FileTimes::new()
                    .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1)),
            )
            .unwrap();
        let data = root.path().join("data");
        install_from_source(&data, &old).unwrap();
        install_from_source(&data, &source).unwrap();
        for name in [AGENT_DEST_BIN_NAME, DAEMON_DEST_BIN_NAME] {
            assert_eq!(fs::read(data.join("bin").join(name)).unwrap(), b"NEW-BUILD");
        }
    }
    #[test]
    fn install_copies_source_to_dest_and_renames() {
        // Stage a fake source binary next to a fake current_exe and
        // assert install_runner_cli puts it at $APPDATA/bin/runner with
        // executable permissions on Unix.
        let workspace = tempfile::tempdir().unwrap();
        let exe_dir = workspace.path().join("target/debug");
        fs::create_dir_all(&exe_dir).unwrap();

        // Fake the CLI artifact next to the (would-be) current_exe.
        let source = exe_dir.join(AGENT_SOURCE_BIN_NAME);
        {
            let mut f = fs::File::create(&source).unwrap();
            writeln!(f, "#!/bin/sh\necho fake").unwrap();
        }
        // Note: this test exercises the copy logic indirectly. We call
        // through the public install fn against an `app_data_dir` that
        // is just a tempdir; locate_source uses `current_exe()`, which
        // for `cargo test` returns the test binary itself, not our
        // fake — so we'd skip with "not found". To make the test
        // meaningful, we exercise the up_to_date and copy helpers
        // directly instead. install_runner_cli's prod path is covered
        // manually until end-to-end packaging tests land.
        let app_data = tempfile::tempdir().unwrap();
        let bin_dir = app_data.path().join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let dest = bin_dir.join(AGENT_DEST_BIN_NAME);

        // First copy: dest doesn't exist, must be replaced.
        assert!(!dest_matches(&source, &dest, None).unwrap().0);
        let tmp = tempfile::NamedTempFile::new_in(&bin_dir).unwrap();
        std::fs::copy(&source, tmp.path()).unwrap();
        tmp.persist(&dest).unwrap();
        assert!(dest.exists());
        assert_eq!(
            fs::metadata(&source).unwrap().len(),
            fs::metadata(&dest).unwrap().len()
        );

        // Second copy: identical contents should skip.
        assert!(dest_matches(&source, &dest, None).unwrap().0);
    }
}
