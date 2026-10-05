use crate::error::{Error, Result};
use std::path::{Path, PathBuf};

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
    std::fs::create_dir_all(app_data_dir)?;
    let lock = crate::daemon_process::lock_file(app_data_dir)?;
    install_binary(app_data_dir, source, AGENT_DEST_BIN_NAME)?;
    match fs2::FileExt::try_lock_exclusive(&lock) {
        Ok(()) => {
            install_binary(app_data_dir, source, DAEMON_DEST_BIN_NAME)?;
            #[cfg(windows)]
            for name in ["conpty.dll", "OpenConsole.exe"] {
                let companion = source.parent().unwrap().join(name);
                if companion.is_file() {
                    install_binary(app_data_dir, &companion, name)?;
                }
            }
        }
        Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => (),
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

pub const DAEMON_DEST_BIN_NAME: &str = if cfg!(windows) {
    "runnerd.exe"
} else {
    "runnerd"
};

fn install_binary(app_data_dir: &Path, source: &Path, dest_name: &str) -> Result<()> {
    let dest_dir = app_data_dir.join("bin");
    std::fs::create_dir_all(&dest_dir)?;
    let dest = dest_dir.join(dest_name);

    if up_to_date(source, &dest)? {
        return Ok(());
    }

    // Copy via tempfile + rename to keep the swap atomic — a half-written
    // file would crash the next process that runs this sidecar.
    let tmp = tempfile::NamedTempFile::new_in(&dest_dir)?;
    std::fs::copy(source, tmp.path())?;
    tmp.persist(&dest).map_err(|e| Error::Io(e.error))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dest, perms)?;
    }
    Ok(())
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

fn up_to_date(source: &Path, dest: &Path) -> Result<bool> {
    let Ok(dst_meta) = std::fs::metadata(dest) else {
        return Ok(false);
    };
    let src_meta = std::fs::metadata(source)?;
    if src_meta.len() != dst_meta.len() {
        return Ok(false);
    }
    Ok(crate::daemon_process::executable_hash(source)?
        == crate::daemon_process::executable_hash(dest)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
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
        fs2::FileExt::try_lock_exclusive(&daemon_lock).unwrap();
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
        assert!(!up_to_date(&source, &dest).unwrap());
        let tmp = tempfile::NamedTempFile::new_in(&bin_dir).unwrap();
        std::fs::copy(&source, tmp.path()).unwrap();
        tmp.persist(&dest).unwrap();
        assert!(dest.exists());
        assert_eq!(
            fs::metadata(&source).unwrap().len(),
            fs::metadata(&dest).unwrap().len()
        );

        // Second copy: identical contents should skip.
        assert!(up_to_date(&source, &dest).unwrap());
    }
}
