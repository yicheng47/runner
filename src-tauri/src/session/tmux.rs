#![allow(dead_code)] // Wired into SessionManager in Step 5+; foundation now.

// Tmux runtime — Step 2 of docs/impls/0004-tmux-session-runtime.md.
// This file currently owns:
//
//   - Binary discovery (`resolve_tmux_binary`) that does not depend
//     on the GUI-launchd PATH.
//   - The Runner-managed tmux config writer
//     (`write_runner_config`). The same config is referenced by
//     every `tmux` invocation via the `-f <runner.conf>` flag —
//     standalone `set-option`s are unreliable because the server
//     can exit between the option set and the next `new-session`.
//   - The `tmux_cmd()` helper that binds `-L runner -f
//     <runner.conf>` so individual call sites can never forget the
//     global flags.
//
// Everything else (spawn, pipe-pane, paste-buffer, reconciliation)
// arrives in later steps and bolts onto this foundation.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::runtime::{RuntimeError, RuntimeResult};

/// `-L` label for the private tmux server. A stable string is fine
/// — distinct sockets for prod / dev / tests come from the
/// `RUNNER_TMUX_LABEL` override.
pub const DEFAULT_LABEL: &str = "runner";

/// Stamp written into the generated config under
/// `@runner_config_version`. Bump this string whenever
/// `RUNNER_CONFIG_TEMPLATE` changes shape so the app-start
/// reconciler (Step 8) knows to `source-file` against an
/// already-running server.
pub const CONFIG_VERSION: &str = "v1";

/// Body of the generated config. Server-wide options live here
/// rather than being applied via runtime `set-option` calls — see
/// the plan's Step 5 rationale for why standalone `set-option` is
/// fragile under default `exit-empty=on`.
const RUNNER_CONFIG_TEMPLATE: &str = "\
# Runner-managed tmux config — do not hand-edit; rewritten on app start.
# See docs/impls/0004-tmux-session-runtime.md.

# Server-scoped: keep the server alive between sessions so reattach
# across app restart works without a churn cycle.
set-option -s exit-empty off

# Per-session option defaults applied at server start. New sessions
# inherit these; existing panes keep whatever was active when they
# were created (history-limit in particular is sticky per pane).
set-option -g history-limit 50000
set-option -g window-size manual
set-option -g default-size 120x32
set-option -g remain-on-exit on
set-option -g status off

# Stamp so the app-start reconciler can detect a stale server (one
# left over from a previous Runner version with a different config)
# and `source-file` the new options into it.
set-option -g @runner_config_version \"v1\"
";

/// Fallback absolute paths searched after `RUNNER_TMUX` and the
/// process PATH. Covers Apple Silicon Homebrew, Intel Homebrew, and
/// the system path Linux distros ship tmux at.
const FALLBACK_TMUX_PATHS: &[&str] = &[
    "/opt/homebrew/bin/tmux",
    "/usr/local/bin/tmux",
    "/usr/bin/tmux",
];

/// Find a usable tmux binary. Resolution order:
///
/// 1. `RUNNER_TMUX` env var — explicit override for tests / weird
///    installs.
/// 2. Process PATH — covers `pnpm tauri dev` runs from a terminal.
/// 3. Hard-coded absolute fallbacks — covers Finder/Dock launches
///    where launchd hands us a stripped PATH (the original bug from
///    issue #65).
///
/// Returns a typed error on Windows so callers can refuse to
/// instantiate `TmuxRuntime` cleanly; the native-pty runtime is the
/// future Windows path.
pub fn resolve_tmux_binary() -> RuntimeResult<PathBuf> {
    if cfg!(target_os = "windows") {
        return Err(RuntimeError::TmuxRequiresUnix);
    }

    let mut searched: Vec<PathBuf> = Vec::new();

    if let Ok(explicit) = env::var("RUNNER_TMUX") {
        let p = PathBuf::from(&explicit);
        if is_executable(&p) {
            return Ok(p);
        }
        searched.push(p);
    }

    if let Ok(path) = env::var("PATH") {
        for dir in env::split_paths(&path) {
            let candidate = dir.join("tmux");
            if is_executable(&candidate) {
                return Ok(candidate);
            }
            searched.push(candidate);
        }
    }

    for fallback in FALLBACK_TMUX_PATHS {
        let p = PathBuf::from(fallback);
        if is_executable(&p) {
            return Ok(p);
        }
        searched.push(p);
    }

    Err(RuntimeError::TmuxNotFound { searched })
}

/// Cheap `executable + non-directory` check. We don't need POSIX
/// access(X_OK) precision — if metadata says it's a file and the
/// permission bits include any execute bit, that's enough. tmux's
/// own startup will give a useful error if the file we picked
/// doesn't actually run.
fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Path to the Runner-managed tmux config file under the app's data
/// directory (`~/Library/Application Support/com.wycstudios.runner/`
/// on macOS, `$XDG_DATA_HOME/com.wycstudios.runner/` on Linux). The
/// file is generated / overwritten by `write_runner_config`.
pub fn config_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("tmux.conf")
}

/// Generate `<app_data_dir>/tmux.conf` if missing or out-of-date.
/// Idempotent: safe to call on every app start. Returns the path so
/// the caller can pass it to `tmux_cmd()`.
pub fn write_runner_config(app_data_dir: &Path) -> RuntimeResult<PathBuf> {
    let path = config_path(app_data_dir);
    let needs_write = match std::fs::read_to_string(&path) {
        Ok(existing) => existing != RUNNER_CONFIG_TEMPLATE,
        Err(_) => true,
    };
    if needs_write {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, RUNNER_CONFIG_TEMPLATE)?;
    }
    Ok(path)
}

/// Pre-bound `tmux` command with `-L <label> -f <config>` already
/// applied. Every tmux invocation in the runtime layer should go
/// through this so the global flags can never get forgotten at a
/// call site (the failure mode for that is silently picking up the
/// user's `~/.tmux.conf` and inheriting their `default-shell`,
/// `history-limit`, etc.).
pub fn tmux_cmd(tmux_bin: &Path, label: &str, config: &Path) -> Command {
    let mut cmd = Command::new(tmux_bin);
    cmd.args([
        "-L".as_ref(),
        label.as_ref(),
        "-f".as_ref(),
        config.as_os_str(),
    ]);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;

    /// Tests that mutate `RUNNER_TMUX` / `PATH` must hold this
    /// mutex for the duration of the mutation. Rust's default
    /// test runner is multi-threaded, so without serialization
    /// two env-mutating tests can race and observe each other's
    /// half-restored state. Using a single `Mutex<()>` is enough
    /// — the lock scope just needs to span set + read + restore.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn resolve_honors_runner_tmux_when_executable() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("tmux");
        fs::write(&bin, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&bin).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&bin, perms).unwrap();
        }
        let prev = env::var_os("RUNNER_TMUX");
        env::set_var("RUNNER_TMUX", &bin);
        let resolved = resolve_tmux_binary();
        match prev {
            Some(v) => env::set_var("RUNNER_TMUX", v),
            None => env::remove_var("RUNNER_TMUX"),
        }
        assert_eq!(resolved.unwrap(), bin);
    }

    #[test]
    fn resolve_skips_runner_tmux_when_missing() {
        let _guard = ENV_LOCK.lock().unwrap();
        // Set RUNNER_TMUX to a path that doesn't exist; resolution
        // should fall through to PATH / fallbacks rather than
        // succeeding. We can't assert what it falls through to
        // because the dev's PATH may or may not have tmux — only
        // that the missing-RUNNER_TMUX path is included in the
        // searched list when everything else also fails.
        let prev = env::var_os("RUNNER_TMUX");
        let prev_path = env::var_os("PATH");
        let bogus = PathBuf::from("/nonexistent/tmux/please-fail");
        env::set_var("RUNNER_TMUX", &bogus);
        env::set_var("PATH", "/this/dir/does/not/exist");
        let result = resolve_tmux_binary();
        match prev {
            Some(v) => env::set_var("RUNNER_TMUX", v),
            None => env::remove_var("RUNNER_TMUX"),
        }
        match prev_path {
            Some(v) => env::set_var("PATH", v),
            None => env::remove_var("PATH"),
        }
        // Either we found a fallback Homebrew tmux on the dev's
        // machine, or we got TmuxNotFound including the bogus path.
        match result {
            Ok(_found) => {}
            Err(RuntimeError::TmuxNotFound { searched }) => {
                assert!(searched.contains(&bogus), "searched = {searched:?}");
            }
            Err(other) => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn resolve_returns_requires_unix_on_windows() {
        match resolve_tmux_binary() {
            Err(RuntimeError::TmuxRequiresUnix) => {}
            other => panic!("expected TmuxRequiresUnix, got {other:?}"),
        }
    }

    #[test]
    fn write_runner_config_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let p1 = write_runner_config(dir.path()).unwrap();
        let m1 = fs::metadata(&p1).unwrap().modified().unwrap();
        // Sleep enough that mtime resolution can distinguish.
        std::thread::sleep(std::time::Duration::from_millis(10));
        let p2 = write_runner_config(dir.path()).unwrap();
        let m2 = fs::metadata(&p2).unwrap().modified().unwrap();
        assert_eq!(p1, p2);
        assert_eq!(
            m1, m2,
            "config should not be rewritten when content matches"
        );
    }

    #[test]
    fn write_runner_config_rewrites_when_stale() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());
        fs::create_dir_all(dir.path()).unwrap();
        fs::write(&path, "old content").unwrap();
        let written = write_runner_config(dir.path()).unwrap();
        let body = fs::read_to_string(&written).unwrap();
        assert_eq!(body, RUNNER_CONFIG_TEMPLATE);
    }

    #[test]
    fn config_template_contains_required_options() {
        // Sanity: the options the rest of the plan depends on must
        // be present. If a future edit drops one of these, tests
        // catch it before we ship.
        assert!(RUNNER_CONFIG_TEMPLATE.contains("set-option -s exit-empty off"));
        assert!(RUNNER_CONFIG_TEMPLATE.contains("history-limit 50000"));
        assert!(RUNNER_CONFIG_TEMPLATE.contains("window-size manual"));
        assert!(RUNNER_CONFIG_TEMPLATE.contains("remain-on-exit on"));
        assert!(RUNNER_CONFIG_TEMPLATE.contains("@runner_config_version"));
        assert!(RUNNER_CONFIG_TEMPLATE.contains(CONFIG_VERSION));
    }

    #[test]
    fn tmux_cmd_binds_label_and_config() {
        let dir = tempfile::tempdir().unwrap();
        let conf = write_runner_config(dir.path()).unwrap();
        let cmd = tmux_cmd(Path::new("/usr/bin/tmux"), DEFAULT_LABEL, &conf);
        let args: Vec<&std::ffi::OsStr> = cmd.get_args().collect();
        assert_eq!(
            args,
            &[
                "-L",
                DEFAULT_LABEL,
                "-f",
                conf.as_os_str().to_str().unwrap()
            ]
        );
    }
}
