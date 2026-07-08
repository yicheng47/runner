// Install Runner's bundled CLI sidecars under `$APPDATA/runner/bin/`.
// Child PTYs get `runner` on PATH for mission coordination, while MCP
// clients launch `runner-mcp` directly from their config files.
//
// Naming. The Tauri app crate also produces a binary called `runner`,
// which would collide if the CLI used the same name in the same
// `target/` dir. The source-side agent binary is therefore
// `runner-agent-cli`; this installer renames it to `runner` in app data
// so spawned PTYs get the intended user-facing command. The MCP proxy is
// a separate `runner-mcp` binary and is installed without renaming.
//
// Source resolution. In dev (`tauri dev`), the staging script builds
// both source binaries and stages them for Tauri's externalBin copy. In
// production, the same script cross-builds the release binaries for the
// active triple. Either path leaves `runner-agent-cli` and `runner-mcp`
// next to `current_exe`, which `locate_source` resolves by name.
//
// Skip-if-current optimization. Compare (size, mtime) — if the source
// file's mtime is `<=` the destination's AND sizes match, skip the
// copy. Hash-compare would be slower without buying anything for the
// "rebuilt-CLI mtime moves forward" case.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Source-side agent CLI artifact. Installed into app data as `runner`.
const AGENT_SOURCE_BIN_NAME: &str = if cfg!(windows) {
    "runner-agent-cli.exe"
} else {
    "runner-agent-cli"
};

/// Source-side MCP proxy artifact. Installed into app data as `runner-mcp`.
const MCP_SOURCE_BIN_NAME: &str = if cfg!(windows) {
    "runner-mcp.exe"
} else {
    "runner-mcp"
};

/// Name of the agent CLI we drop into `$APPDATA/runner/bin/`. Must match what
/// `SessionManager::spawn` puts on PATH — arch §5.3 Layer 2 has the
/// CLI being invoked as bare `runner` from inside spawned PTYs.
const AGENT_DEST_BIN_NAME: &str = if cfg!(windows) {
    "runner.exe"
} else {
    "runner"
};

/// Name of the MCP proxy binary registered with Claude Code / Codex.
pub const MCP_DEST_BIN_NAME: &str = if cfg!(windows) {
    "runner-mcp.exe"
} else {
    "runner-mcp"
};

pub fn install_runner_cli(app_data_dir: &Path) -> Result<()> {
    install_binary(app_data_dir, AGENT_SOURCE_BIN_NAME, AGENT_DEST_BIN_NAME)
}

pub fn install_mcp_cli(app_data_dir: &Path) -> Result<()> {
    install_binary(app_data_dir, MCP_SOURCE_BIN_NAME, MCP_DEST_BIN_NAME)
}

fn install_binary(app_data_dir: &Path, source_name: &str, dest_name: &str) -> Result<()> {
    let Some(source) = locate_source(source_name)? else {
        log::warn!(
            "bundled CLI sidecar ({source_name}) not found next to current_exe; \
             skipping install of {dest_name}. Build the CLI sidecars and \
             relaunch."
        );
        return Ok(());
    };
    let dest_dir = app_data_dir.join("bin");
    std::fs::create_dir_all(&dest_dir)?;
    let dest = dest_dir.join(dest_name);

    if up_to_date(&source, &dest)? {
        return Ok(());
    }

    // Copy via tempfile + rename to keep the swap atomic — a half-written
    // file would crash the next process that runs this sidecar.
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

/// Drop a per-(mission,slot) `runner` shim into
/// `$APPDATA/missions/<mission_id>/shims/<handle>/bin/runner` that
/// hardcodes the slot's `RUNNER_*` env vars and `exec`s the real
/// bundled CLI. PATH inside the spawned PTY prepends this dir, so
/// `runner …` resolves to the shim regardless of what shell context
/// the agent CLI's tool-call subprocess runs under. Without this,
/// claude-code's Bash tool spawns a non-login shell that doesn't
/// inherit the PTY's env, and the bundled CLI exits with "missing
/// required env var".
///
/// Each call rewrites the shim atomically (tempfile + rename) so
/// resume can refresh the values without leaving a half-written
/// file an agent could crash on. The path is keyed by mission_id +
/// handle (not session_id) because session_id rotates on every
/// resume, while the env vars don't — the shim is reusable across
/// resumes of the same slot.
pub fn install_session_runner_shim(
    app_data_dir: &Path,
    crew_id: &str,
    mission_id: &str,
    handle: &str,
    event_log: &Path,
    mission_cwd: Option<&str>,
) -> Result<PathBuf> {
    let shim_dir = app_data_dir
        .join("missions")
        .join(mission_id)
        .join("shims")
        .join(handle)
        .join("bin");
    std::fs::create_dir_all(&shim_dir)?;
    let shim_path = shim_dir.join("runner");
    let real_runner = app_data_dir.join("bin").join(AGENT_DEST_BIN_NAME);

    let event_log_str = event_log.to_string_lossy();
    let mut script = String::new();
    script.push_str("#!/bin/sh\n");
    script
        .push_str("# Auto-generated session shim. See cli_install::install_session_runner_shim.\n");
    script.push_str(&format!("export RUNNER_CREW_ID='{}'\n", sh_escape(crew_id)));
    script.push_str(&format!(
        "export RUNNER_MISSION_ID='{}'\n",
        sh_escape(mission_id)
    ));
    script.push_str(&format!("export RUNNER_HANDLE='{}'\n", sh_escape(handle)));
    script.push_str(&format!(
        "export RUNNER_EVENT_LOG='{}'\n",
        sh_escape(&event_log_str)
    ));
    if let Some(cwd) = mission_cwd {
        script.push_str(&format!("export MISSION_CWD='{}'\n", sh_escape(cwd)));
    }
    script.push_str(&format!(
        "exec '{}' \"$@\"\n",
        sh_escape(&real_runner.to_string_lossy())
    ));

    let tmp = tempfile::NamedTempFile::new_in(&shim_dir)?;
    std::fs::write(tmp.path(), script.as_bytes())?;
    tmp.persist(&shim_path).map_err(|e| Error::Io(e.error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&shim_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&shim_path, perms)?;
    }
    Ok(shim_dir)
}

/// Escape a string for inside single-quoted POSIX shell. Single
/// quotes can't contain themselves; the canonical workaround is to
/// close the quote, emit `'\''`, and reopen.
fn sh_escape(s: &str) -> String {
    s.replace('\'', "'\\''")
}

fn locate_source(source_name: &str) -> Result<Option<PathBuf>> {
    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| Error::msg("current_exe has no parent"))?;
    let candidate = dir.join(source_name);
    // The Tauri exe itself is `runner` (different basename from the
    // source names), so the equality guard is belt-and-suspenders for
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

        // Second copy: dest now matches by size+mtime, should skip.
        assert!(up_to_date(&source, &dest).unwrap());
    }

    #[test]
    fn shim_dir_includes_mission_id_so_concurrent_missions_dont_collide() {
        // Regression guard for #55: when the per-crew "at most one live
        // mission" cap was lifted, two missions on the same crew can
        // run side by side. They share `crew_id` and (when the same
        // slot template is on both rosters) `slot_handle`, so the
        // shim's path key MUST also include `mission_id` to keep the
        // two RUNNER_* env exports separate. Two installs differing
        // only in `mission_id` must produce different dirs and
        // different baked env values.
        let app_data = tempfile::tempdir().unwrap();
        let event_log_a = app_data.path().join("missions/m-a/events.jsonl");
        let event_log_b = app_data.path().join("missions/m-b/events.jsonl");
        // The shim writer needs the source bin (for the `exec` line).
        // Stage a fake bundled CLI so the install has something to
        // point at — content is irrelevant; the shim just embeds the
        // path.
        let bin_dir = app_data.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::write(bin_dir.join(AGENT_DEST_BIN_NAME), "#!/bin/sh\nexit 0\n").unwrap();

        let dir_a = install_session_runner_shim(
            app_data.path(),
            "crew-1",
            "m-a",
            "architect",
            &event_log_a,
            None,
        )
        .unwrap();
        let dir_b = install_session_runner_shim(
            app_data.path(),
            "crew-1",
            "m-b",
            "architect",
            &event_log_b,
            None,
        )
        .unwrap();

        assert_ne!(
            dir_a, dir_b,
            "shim dirs for two missions on the same crew + slot must differ",
        );
        // Compare path components, not substrings — the separator is
        // platform-dependent.
        assert!(
            dir_a.components().any(|c| c.as_os_str() == "m-a"),
            "dir_a must include mission_id m-a: {dir_a:?}",
        );
        assert!(
            dir_b.components().any(|c| c.as_os_str() == "m-b"),
            "dir_b must include mission_id m-b: {dir_b:?}",
        );

        // The baked RUNNER_MISSION_ID export must match the dir's
        // mission_id, not leak across — without this guarantee a slot
        // running in mission m-a could attribute events to m-b.
        let script_a = std::fs::read_to_string(dir_a.join("runner")).unwrap();
        let script_b = std::fs::read_to_string(dir_b.join("runner")).unwrap();
        assert!(
            script_a.contains("export RUNNER_MISSION_ID='m-a'"),
            "shim_a must export the m-a mission id: {script_a}",
        );
        assert!(
            script_b.contains("export RUNNER_MISSION_ID='m-b'"),
            "shim_b must export the m-b mission id: {script_b}",
        );
    }
}
