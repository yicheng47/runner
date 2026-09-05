// Install Runner's bundled CLI sidecars under `$APPDATA/runner/bin/`.
// Child PTYs get `runner` on PATH for mission coordination, while MCP
// clients launch `runner-mcp` directly from their config files.
//
// Naming. The source-side agent binary remains `runner-agent-cli`; this
// installer renames it to `runner` in app data so spawned PTYs get the
// intended user-facing command without colliding with another `runner`
// artifact in a shared target directory. The GPUI binary is `Runner`. The
// MCP proxy is a separate `runner-mcp` binary and is installed as-is.
//
// Source resolution. Development builds and release packaging leave
// `runner-agent-cli` and `runner-mcp` next to the `Runner` executable;
// `locate_source` resolves them from that directory by name.
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

/// Name of the MCP proxy binary registered with Claude Code, Codex, and TRAE.
pub const MCP_DEST_BIN_NAME: &str = if cfg!(windows) {
    "runner-mcp.exe"
} else {
    "runner-mcp"
};

// Called from the app's `boot_core` on every launch, before any session can
// spawn or MCP config is written. Mission shims, spawned PATHs, and MCP
// configs all consume the destinations.
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
    #[cfg(windows)]
    let real_runner = PathBuf::from(real_runner.to_string_lossy().replace('\\', "/"));
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
    #[cfg(windows)]
    {
        let cmd_script = windows_cmd_shim(
            &app_data_dir.join("bin").join(AGENT_DEST_BIN_NAME),
            crew_id,
            mission_id,
            handle,
            &event_log_str,
            mission_cwd,
        )?;
        let tmp = tempfile::NamedTempFile::new_in(&shim_dir)?;
        std::fs::write(tmp.path(), cmd_script.as_bytes())?;
        tmp.persist(shim_dir.join("runner.cmd"))
            .map_err(|e| Error::Io(e.error))?;
    }
    Ok(shim_dir)
}

#[cfg(windows)]
fn windows_cmd_shim(
    real_runner: &Path,
    crew_id: &str,
    mission_id: &str,
    handle: &str,
    event_log: &str,
    mission_cwd: Option<&str>,
) -> Result<String> {
    let real_runner = real_runner.to_string_lossy();
    let mut script = String::from("@echo off\r\nsetlocal\r\n");
    for (name, value) in [
        ("RUNNER_CREW_ID", Some(crew_id)),
        ("RUNNER_MISSION_ID", Some(mission_id)),
        ("RUNNER_HANDLE", Some(handle)),
        ("RUNNER_EVENT_LOG", Some(event_log)),
        ("MISSION_CWD", mission_cwd),
    ] {
        if let Some(value) = value {
            if value.contains('"') {
                return Err(Error::msg(format!("{name} contains a quote")));
            }
            let value = value.replace('%', "%%");
            script.push_str(&format!("set \"{name}={value}\"\r\n"));
        }
    }
    if real_runner.contains('"') {
        return Err(Error::msg("runner path contains a quote"));
    }
    let real_runner = real_runner.replace('%', "%%");
    script.push_str(&format!("\"{real_runner}\" %*\r\n"));
    Ok(script)
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
        #[cfg(unix)]
        assert!(
            dir_a.to_string_lossy().contains("/m-a/"),
            "dir_a must include mission_id m-a: {dir_a:?}",
        );
        #[cfg(windows)]
        assert!(
            dir_a.components().any(|part| part.as_os_str() == "m-a"),
            "dir_a must include mission_id m-a: {dir_a:?}",
        );
        #[cfg(unix)]
        assert!(
            dir_b.to_string_lossy().contains("/m-b/"),
            "dir_b must include mission_id m-b: {dir_b:?}",
        );
        #[cfg(windows)]
        assert!(
            dir_b.components().any(|part| part.as_os_str() == "m-b"),
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

    #[test]
    fn session_shim_has_exact_shell_contents() {
        let app_data = tempfile::tempdir().unwrap();
        let event_log = app_data.path().join("events.ndjson");
        let shim_dir = install_session_runner_shim(
            app_data.path(),
            "crew-1",
            "mission-1",
            "coder",
            &event_log,
            Some("a'b"),
        )
        .unwrap();
        let runner_path = app_data.path().join("bin").join(AGENT_DEST_BIN_NAME);
        let runner_path = runner_path.to_string_lossy();
        #[cfg(windows)]
        let runner_path = runner_path.replace('\\', "/");
        assert_eq!(
            fs::read_to_string(shim_dir.join("runner")).unwrap(),
            format!(
                "#!/bin/sh\n# Auto-generated session shim. See cli_install::install_session_runner_shim.\nexport RUNNER_CREW_ID='crew-1'\nexport RUNNER_MISSION_ID='mission-1'\nexport RUNNER_HANDLE='coder'\nexport RUNNER_EVENT_LOG='{}'\nexport MISSION_CWD='a'\\''b'\nexec '{}' \"$@\"\n",
                event_log.display(), runner_path,
            ),
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(shim_dir.join("runner"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o755
            );
            assert!(!shim_dir.join("runner.cmd").exists());
        }
    }

    #[cfg(windows)]
    #[test]
    fn session_shim_has_exact_cmd_contents_and_refreshes_both_files() {
        let app_data = tempfile::tempdir().unwrap();
        let event_log = app_data.path().join("events.ndjson");
        for cwd in [Some(r"C:\Agent Tools\repo"), None] {
            let shim_dir = install_session_runner_shim(
                app_data.path(),
                "crew-1",
                "mission-1",
                "coder",
                &event_log,
                cwd,
            )
            .unwrap();
            let cwd_line = cwd
                .map(|cwd| format!("set \"MISSION_CWD={cwd}\"\r\n"))
                .unwrap_or_default();
            assert_eq!(
                fs::read_to_string(shim_dir.join("runner.cmd")).unwrap(),
                format!(
                    "@echo off\r\nsetlocal\r\nset \"RUNNER_CREW_ID=crew-1\"\r\nset \"RUNNER_MISSION_ID=mission-1\"\r\nset \"RUNNER_HANDLE=coder\"\r\nset \"RUNNER_EVENT_LOG={}\"\r\n{cwd_line}\"{}\" %*\r\n",
                    event_log.display(), app_data.path().join("bin").join(AGENT_DEST_BIN_NAME).display(),
                ),
            );
            let shell = fs::read_to_string(shim_dir.join("runner")).unwrap();
            assert_eq!(shell.contains("export MISSION_CWD="), cwd.is_some());
        }
    }

    #[cfg(windows)]
    #[test]
    fn cmd_shim_rejects_quotes_in_every_value() {
        let bad = "bad\"value";
        for index in 0..6 {
            let mut values = ["runner.exe", "crew", "mission", "coder", "events", "cwd"];
            values[index] = bad;
            assert!(
                windows_cmd_shim(
                    Path::new(values[0]),
                    values[1],
                    values[2],
                    values[3],
                    values[4],
                    Some(values[5]),
                )
                .is_err(),
                "accepted {bad:?} at index {index}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn cmd_shim_escapes_percent_in_every_value_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let runner = dir.path().join("agent%helper.cmd");
        fs::write(&runner, "@echo off\r\necho %RUNNER_CREW_ID%\r\necho %RUNNER_MISSION_ID%\r\necho %RUNNER_HANDLE%\r\necho %RUNNER_EVENT_LOG%\r\necho %MISSION_CWD%\r\n").unwrap();
        let value = r"C:\a%b\repo";
        let script = windows_cmd_shim(&runner, value, value, value, value, Some(value)).unwrap();
        for name in [
            "RUNNER_CREW_ID",
            "RUNNER_MISSION_ID",
            "RUNNER_HANDLE",
            "RUNNER_EVENT_LOG",
            "MISSION_CWD",
        ] {
            assert!(script.contains(&format!("set \"{name}=C:\\a%%b\\repo\"\r\n")));
        }
        assert!(script.contains("agent%%helper.cmd\" %*"));
        let shim = dir.path().join("runner.cmd");
        fs::write(&shim, script).unwrap();
        let output = std::process::Command::new(&shim).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("{value}\r\n").repeat(5)
        );
    }

    #[cfg(windows)]
    #[test]
    fn cmd_shim_error_preserves_the_shell_shim() {
        let app_data = tempfile::tempdir().unwrap();
        let event_log = app_data.path().join("events.ndjson");
        let error = install_session_runner_shim(
            app_data.path(),
            "crew",
            "mission",
            "coder",
            &event_log,
            Some("bad\"cwd"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("quote"));
        let shim_dir = app_data.path().join("missions/mission/shims/coder/bin");
        let script = fs::read_to_string(shim_dir.join("runner")).unwrap();
        assert!(script.contains("export MISSION_CWD='bad\"cwd'\n"));
        assert!(!shim_dir.join("runner.cmd").exists());
    }
}
