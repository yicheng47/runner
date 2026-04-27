// Install the bundled CLI as `$APPDATA/runner/bin/runner` at app
// startup so child PTYs find it on PATH (arch §5.3 Layer 2).
//
// Naming. The Tauri app crate also produces a binary called `runner`,
// which would collide if the CLI used the same name in the same
// `target/` dir. The CLI source-side binary is therefore `runner-cli`
// (`cli/Cargo.toml`'s `[[bin]] name`). This installer copies the
// `runner-cli` artifact and renames it to `runner` at the destination
// — so the file on PATH (which is what spawned PTYs invoke) keeps the
// intended user-facing name without colliding at build time.
//
// Source resolution. In dev (`cargo run`), the CLI lives next to the
// Tauri exe under `target/{debug,release}/runner-cli`. In production,
// the Tauri bundler is expected to ship the same artifact alongside
// the app's main executable. Production-bundle wiring (tauri.conf.json
// `bundle.externalBin` declaration + a beforeBuildCommand step that
// stages the binary at the target-triple-suffixed path the bundler
// expects) is tracked as a follow-up; the dev path is what the v0
// demo runs.
//
// Skip-if-current optimization. Compare (size, mtime) — if the source
// file's mtime is `<=` the destination's AND sizes match, skip the
// copy. Hash-compare would be slower without buying anything for the
// "rebuilt-CLI mtime moves forward" case.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// On-disk name of the CLI source artifact (set in `cli/Cargo.toml`).
/// Different from the Tauri app crate's binary so they coexist in the
/// same `target/` dir without overwriting each other.
const SOURCE_BIN_NAME: &str = if cfg!(windows) {
    "runner-cli.exe"
} else {
    "runner-cli"
};

/// Name of the file we drop into `$APPDATA/runner/bin/`. Must match what
/// `SessionManager::spawn` puts on PATH — arch §5.3 Layer 2 has the
/// CLI being invoked as bare `runner` from inside spawned PTYs.
const DEST_BIN_NAME: &str = if cfg!(windows) {
    "runner.exe"
} else {
    "runner"
};

pub fn install_runner_cli(app_data_dir: &Path) -> Result<()> {
    let Some(source) = locate_source()? else {
        eprintln!(
            "runner: bundled CLI ({SOURCE_BIN_NAME}) not found next to current_exe; \
             skipping install. Sessions that invoke `runner` will error until the \
             binary is on PATH. Build the CLI with `cargo build -p runner-cli` and \
             relaunch."
        );
        return Ok(());
    };
    let dest_dir = app_data_dir.join("bin");
    std::fs::create_dir_all(&dest_dir)?;
    let dest = dest_dir.join(DEST_BIN_NAME);

    if up_to_date(&source, &dest)? {
        return Ok(());
    }

    // Copy via tempfile + rename to keep the swap atomic — a half-written
    // file would crash the next agent that runs `runner help`.
    let tmp = tempfile::NamedTempFile::new_in(&dest_dir)?;
    std::fs::copy(&source, tmp.path())?;
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

fn locate_source() -> Result<Option<PathBuf>> {
    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| Error::msg("current_exe has no parent"))?;
    let candidate = dir.join(SOURCE_BIN_NAME);
    // The Tauri exe itself is `runner` (different basename from
    // `runner-cli`), so the equality guard is belt-and-suspenders for
    // future renames; it still gates on existence.
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
    let src_mtime = src_meta.modified().ok();
    let dst_mtime = dst_meta.modified().ok();
    match (src_mtime, dst_mtime) {
        (Some(s), Some(d)) => Ok(s <= d),
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    #[test]
    fn install_copies_source_to_dest_and_renames() {
        // Stage a fake source binary next to a fake current_exe and
        // assert install_runner_cli puts it at $APPDATA/bin/runner with
        // executable permissions on Unix.
        let workspace = tempfile::tempdir().unwrap();
        let exe_dir = workspace.path().join("target/debug");
        fs::create_dir_all(&exe_dir).unwrap();

        // Fake the CLI artifact next to the (would-be) current_exe.
        let source = exe_dir.join(SOURCE_BIN_NAME);
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
        let dest = bin_dir.join(DEST_BIN_NAME);

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

        // Second copy: dest now matches by size+mtime, should skip.
        assert!(up_to_date(&source, &dest).unwrap());
    }
}
