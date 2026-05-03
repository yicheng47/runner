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
//! Workaround: at startup, spawn `$SHELL -ilc 'printf "%s" "$PATH"'` once
//! to dump what the login shell would resolve to, capture stdout, and
//! prepend it onto every child PTY's PATH alongside our bundled-bin dir.
//!
//! On Windows the problem doesn't arise (no launchd) and this returns None.

use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const RESOLVE_TIMEOUT: Duration = Duration::from_secs(2);

#[cfg(unix)]
pub fn resolve_login_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let (tx, rx) = mpsc::channel();
    let shell_for_thread = shell.clone();
    thread::spawn(move || {
        // -i (interactive) sources `.zshrc` / `.bashrc`; -l (login) sources
        // `.zprofile` / `.bash_profile`. Both are needed: Homebrew
        // `shellenv` typically lives in `.zprofile` on Apple Silicon while
        // mise / fnm / asdf inject from `.zshrc`. Some users put PATH
        // edits in only one of those, so we ask for both.
        let result = Command::new(&shell_for_thread)
            .arg("-ilc")
            .arg("printf '%s' \"$PATH\"")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();
        let _ = tx.send(result);
    });
    match rx.recv_timeout(RESOLVE_TIMEOUT) {
        Ok(Ok(out)) if out.status.success() => {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        }
        Ok(Ok(out)) => {
            eprintln!(
                "runner: shell PATH resolution via `{shell}` exited non-zero (status={:?}); falling back to launchd PATH",
                out.status.code()
            );
            None
        }
        Ok(Err(e)) => {
            eprintln!(
                "runner: shell PATH resolution via `{shell}` failed to spawn ({e}); falling back to launchd PATH"
            );
            None
        }
        Err(_) => {
            eprintln!(
                "runner: shell PATH resolution via `{shell}` timed out after {}s; falling back to launchd PATH",
                RESOLVE_TIMEOUT.as_secs()
            );
            None
        }
    }
}

#[cfg(not(unix))]
pub fn resolve_login_shell_path() -> Option<String> {
    None
}
