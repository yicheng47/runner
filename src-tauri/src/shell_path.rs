//! Resolves the user's login-shell PATH so child PTYs spawned by a
//! GUI-launched app can find tools that live outside launchd's stripped
//! default PATH (Homebrew, npm-global, mise / asdf / fnm shims, etc.).
//!
//! On macOS, launchd hands GUI apps a default PATH of
//! `/usr/bin:/bin:/usr/sbin:/sbin`. The user's shell rc files extend PATH
//! with toolchains, but they're sourced by the shell — not by launchd. So a
//! GUI-launched Runner inherits a stripped PATH and `claude`, `codex`,
//! `mise` etc. don't resolve. `pnpm tauri dev` from a terminal hides this
//! because the terminal-spawned process already has the shell PATH.
//!
//! Workaround: at startup, spawn `$SHELL -ilc '<marker>; printenv PATH;
//! <marker>'` once, parse the marker-delimited PATH out of stdout, and
//! prepend it onto every child PTY's PATH alongside our bundled-bin dir.
//!
//! Capture choices:
//!   - `printenv PATH` instead of `printf '%s' "$PATH"` — `$PATH`
//!     expansion is shell-specific (fish renders it space-separated),
//!     while `printenv` reads the exported env var which is always
//!     colon-delimited on Unix regardless of the parent shell.
//!   - Sentinel markers around the value — rc files often print banners
//!     or tool noise (`nvm` warnings, `direnv` notices, etc.) on
//!     interactive startup; the markers let us extract the real PATH out
//!     of an arbitrarily noisy stdout.
//!   - try_wait poll with deadline + `kill + wait` on timeout — a hung
//!     rc would otherwise leave a dangling login shell behind every app
//!     launch.
//!
//! On Windows the problem doesn't arise (no launchd) and this returns None.

use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const RESOLVE_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const STDOUT_DRAIN_GRACE: Duration = Duration::from_millis(500);
const MARKER_BEGIN: &str = "__RUNNER_PATH_BEGIN__";
const MARKER_END: &str = "__RUNNER_PATH_END__";

#[cfg(unix)]
pub fn resolve_login_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());

    // -i (interactive) sources `.zshrc` / `.bashrc`; -l (login) sources
    // `.zprofile` / `.bash_profile`. Both are needed: Homebrew
    // `shellenv` typically lives in `.zprofile` on Apple Silicon while
    // mise / fnm / asdf inject from `.zshrc`. The single-quoted `printf`
    // forms work in zsh / bash / fish; `printenv` is in coreutils on
    // every Unix we target.
    let inner = format!("printf '%s' '{MARKER_BEGIN}'; printenv PATH; printf '%s' '{MARKER_END}'");

    let mut child = match Command::new(&shell)
        .arg("-ilc")
        .arg(&inner)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            log::warn!(
                "shell PATH resolution via `{shell}` failed to spawn ({e}); falling back to launchd PATH"
            );
            return None;
        }
    };

    // Drain stdout in a worker so a slow / chatty rc filling the pipe
    // buffer can't deadlock our timeout poll. We `take()` the handle so
    // `wait` below doesn't fight the reader for it.
    let mut stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            log::warn!("shell PATH resolution lost stdout pipe; falling back");
            return None;
        }
    };
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        use std::io::Read;
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });

    // Poll try_wait until the child exits or we hit the deadline. On
    // timeout, SIGTERM the child and reap it — without this a hung
    // shell init would leave one stranded login shell per app launch.
    let deadline = Instant::now() + RESOLVE_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    log::warn!(
                        "shell PATH resolution via `{shell}` timed out after {}s; killed shell; falling back to launchd PATH",
                        RESOLVE_TIMEOUT.as_secs()
                    );
                    return None;
                }
                thread::sleep(POLL_INTERVAL);
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                log::warn!(
                    "shell PATH resolution via `{shell}` failed waiting ({e}); falling back to launchd PATH"
                );
                return None;
            }
        }
    };

    if !status.success() {
        log::warn!(
            "shell PATH resolution via `{shell}` exited non-zero (status={:?}); falling back to launchd PATH",
            status.code()
        );
        return None;
    }

    let stdout_bytes = match rx.recv_timeout(STDOUT_DRAIN_GRACE) {
        Ok(b) => b,
        Err(_) => {
            log::warn!(
                "shell PATH resolution via `{shell}` produced no stdout in time; falling back"
            );
            return None;
        }
    };
    let stdout_str = String::from_utf8_lossy(&stdout_bytes);
    let parsed = extract_path_between_markers(&stdout_str);
    if parsed.is_none() {
        log::warn!(
            "shell PATH resolution via `{shell}` returned no PATH between markers; falling back to launchd PATH"
        );
    }
    parsed
}

#[cfg(not(unix))]
pub fn resolve_login_shell_path() -> Option<String> {
    None
}

/// Pull the marker-delimited PATH value out of arbitrary shell stdout.
/// Tolerates rc-file banners and tool warnings printed before / after
/// our markers; uses `rfind` for the begin marker so a marker mention
/// in a banner can't shadow the real one.
fn extract_path_between_markers(stdout: &str) -> Option<String> {
    let begin_idx = stdout.rfind(MARKER_BEGIN)?;
    let after_begin = &stdout[begin_idx + MARKER_BEGIN.len()..];
    let end_idx = after_begin.find(MARKER_END)?;
    let path = after_begin[..end_idx].trim();
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_path_between_markers_ignoring_rc_banner() {
        // Typical noisy interactive startup: rc banner before our begin
        // marker, then the real PATH (with `printenv`'s trailing
        // newline), then end marker.
        let stdout = "Welcome to zsh!\nnvm: using node v20\n__RUNNER_PATH_BEGIN__/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin\n__RUNNER_PATH_END__";
        assert_eq!(
            extract_path_between_markers(stdout).as_deref(),
            Some("/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin")
        );
    }

    #[test]
    fn extracts_path_when_banner_mentions_marker_substring() {
        // A rogue banner that prints something containing the begin
        // marker shouldn't shadow the real one — we use `rfind` so the
        // last begin-marker before our end-marker wins.
        let stdout = "echo: __RUNNER_PATH_BEGIN__ (this is a banner)\n__RUNNER_PATH_BEGIN__/usr/bin:/bin\n__RUNNER_PATH_END__";
        assert_eq!(
            extract_path_between_markers(stdout).as_deref(),
            Some("/usr/bin:/bin")
        );
    }

    #[test]
    fn missing_markers_returns_none() {
        assert_eq!(extract_path_between_markers("just a banner"), None);
        assert_eq!(
            extract_path_between_markers("__RUNNER_PATH_BEGIN__only"),
            None
        );
        assert_eq!(extract_path_between_markers(""), None);
    }

    #[test]
    fn empty_path_between_markers_returns_none() {
        assert_eq!(
            extract_path_between_markers("__RUNNER_PATH_BEGIN____RUNNER_PATH_END__"),
            None
        );
        assert_eq!(
            extract_path_between_markers("__RUNNER_PATH_BEGIN__\n  \n__RUNNER_PATH_END__"),
            None
        );
    }
}
