//! Resolves env vars from the user's login shell so child PTYs spawned by a
//! GUI-launched app see the same values they would inside Terminal.app.
//!
//! On macOS, launchd hands GUI apps a stripped env: a default PATH of
//! `/usr/bin:/bin:/usr/sbin:/sbin`, no `HTTPS_PROXY` / `HTTP_PROXY` / etc.
//! The user's shell rc files set those, but they're sourced by the shell —
//! not by launchd. So a GUI-launched Runner inherits a stripped env and
//! tools that need either the user's PATH (Homebrew, mise/asdf/fnm,
//! npm-global) OR the user's HTTP proxy (claude / codex login behind a
//! corporate VPN or ClashX-style local proxy) fail in ways that don't
//! reproduce under `pnpm tauri dev` from a terminal.
//!
//! Workaround: at startup, spawn `$SHELL -ilc '<probe>'` once and parse
//! marker-delimited values out of its stdout. We capture a fixed set of
//! var names (`PATH` + the standard proxy quartet, both upper- and
//! lower-case) — names that are dot-files-set rather than launchd-set
//! and that downstream tools universally respect. Values stay whatever
//! the user's rc files happen to export.
//!
//! Capture choices:
//!   - `printenv NAME` per var, wrapped in per-var `__RUNNER_KV_NAME_…__`
//!     markers — rc files often print banners or tool noise (`nvm`
//!     warnings, `direnv` notices, etc.) on interactive startup; the
//!     markers let us pluck each value out of an arbitrarily noisy
//!     stdout, and a missing var (`printenv` exits non-zero) just leaves
//!     an empty marker pair.
//!   - try_wait poll with deadline + `kill + wait` on timeout — a hung
//!     rc would otherwise leave a dangling login shell behind every app
//!     launch.
//!
//! On Windows the problem doesn't arise (no launchd) and this returns an
//! empty `LoginShellEnv`.

use std::collections::BTreeMap;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const RESOLVE_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const STDOUT_DRAIN_GRACE: Duration = Duration::from_millis(500);

/// Var names captured from the login shell. PATH covers the original
/// GUI-launch shim-discovery problem (issue #65); the proxy quartet
/// (both cases) covers the claude / codex login-behind-VPN problem
/// (issue #152). Lowercase variants are standard across curl /
/// reqwest / Python requests / Node's undici / Go's `http.ProxyFromEnvironment`
/// and many users only export the lowercase form.
///
/// This list is intentionally narrow: capturing arbitrary login-shell
/// env would propagate every prompt customization and tool warning a
/// user has in their rc files. Adding a name here is an explicit
/// decision that the var (a) is typically rc-file-set, (b) affects
/// behavior of CLIs the user expects to "just work."
const CAPTURED_VARS: &[&str] = &[
    "PATH",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "no_proxy",
];

/// Snapshot of the login shell's env captured at app start.
///
/// `path` is split out because PATH composition (shim_dir →
/// bundled_bin_dir → login_shell_path → launchd inherited) has its
/// own precedence rules in `launch::compose_path` and isn't a
/// straight env-var passthrough. `vars` holds everything else we
/// capture (today: the proxy quartet, both cases) and is layered
/// into every child PTY's env under `runner.env`, so a runner row's
/// explicit override still wins.
#[derive(Debug, Clone, Default)]
pub struct LoginShellEnv {
    pub path: Option<String>,
    pub vars: BTreeMap<String, String>,
}

#[cfg(unix)]
pub fn resolve_login_shell_env() -> LoginShellEnv {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());

    // -i (interactive) sources `.zshrc` / `.bashrc`; -l (login) sources
    // `.zprofile` / `.bash_profile`. Both are needed: Homebrew
    // `shellenv` typically lives in `.zprofile` on Apple Silicon while
    // mise / fnm / asdf inject from `.zshrc`. `printenv` is in
    // coreutils on every Unix we target and reads the exported env var
    // straight from the process env (no shell-specific `$var`
    // expansion).
    //
    // For each captured var: print BEGIN marker → printenv (stderr to
    // /dev/null so a missing var doesn't pollute stdout) → END marker
    // on its own line. The trailing newline keeps banner text on
    // subsequent lines from running into the END marker. Missing vars
    // produce `BEGIN__END__\n` (empty value) which the parser skips.
    let mut inner = String::new();
    for v in CAPTURED_VARS {
        // The shell sees the script as a single -c arg — using
        // double-quoted markers inside single-quoted printf args
        // means we don't have to escape anything. var names are
        // ASCII identifiers (no shell metachars) so direct
        // interpolation is safe here.
        inner.push_str(&format!(
            "printf '%s' '__RUNNER_KV_{v}_BEGIN__'; printenv {v} 2>/dev/null; printf '%s\\n' '__RUNNER_KV_{v}_END__'; "
        ));
    }

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
                "login-shell env resolution via `{shell}` failed to spawn ({e}); \
                 falling back to launchd env"
            );
            return LoginShellEnv::default();
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
            log::warn!("login-shell env resolution lost stdout pipe; falling back");
            return LoginShellEnv::default();
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
                        "login-shell env resolution via `{shell}` timed out after {}s; \
                         killed shell; falling back to launchd env",
                        RESOLVE_TIMEOUT.as_secs()
                    );
                    return LoginShellEnv::default();
                }
                thread::sleep(POLL_INTERVAL);
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                log::warn!(
                    "login-shell env resolution via `{shell}` failed waiting ({e}); \
                     falling back to launchd env"
                );
                return LoginShellEnv::default();
            }
        }
    };

    if !status.success() {
        log::warn!(
            "login-shell env resolution via `{shell}` exited non-zero (status={:?}); \
             falling back to launchd env",
            status.code()
        );
        return LoginShellEnv::default();
    }

    let stdout_bytes = match rx.recv_timeout(STDOUT_DRAIN_GRACE) {
        Ok(b) => b,
        Err(_) => {
            log::warn!(
                "login-shell env resolution via `{shell}` produced no stdout in time; \
                 falling back"
            );
            return LoginShellEnv::default();
        }
    };
    let stdout_str = String::from_utf8_lossy(&stdout_bytes);
    let parsed = parse_login_shell_env(&stdout_str);
    if parsed.path.is_none() && parsed.vars.is_empty() {
        log::warn!(
            "login-shell env resolution via `{shell}` returned no captured vars; \
             falling back to launchd env"
        );
    }
    parsed
}

#[cfg(not(unix))]
pub fn resolve_login_shell_env() -> LoginShellEnv {
    LoginShellEnv::default()
}

/// Pull each `__RUNNER_KV_<NAME>_BEGIN__…__RUNNER_KV_<NAME>_END__`
/// block out of arbitrary shell stdout. Tolerates rc-file banners
/// before / between / after blocks. Uses `rfind` for each begin
/// marker so a marker mention in a banner can't shadow the real one.
/// An empty value (missing var) is dropped — distinct from "var set
/// to empty string", which we'd represent the same way and which has
/// no useful effect on a child anyway.
fn parse_login_shell_env(stdout: &str) -> LoginShellEnv {
    let mut env = LoginShellEnv::default();
    for v in CAPTURED_VARS {
        let begin = format!("__RUNNER_KV_{v}_BEGIN__");
        let end = format!("__RUNNER_KV_{v}_END__");
        let Some(begin_idx) = stdout.rfind(&begin) else {
            continue;
        };
        let after_begin = &stdout[begin_idx + begin.len()..];
        let Some(end_idx) = after_begin.find(&end) else {
            continue;
        };
        let value = after_begin[..end_idx].trim();
        if value.is_empty() {
            continue;
        }
        if *v == "PATH" {
            env.path = Some(value.to_string());
        } else {
            env.vars.insert(v.to_string(), value.to_string());
        }
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(name: &str, value: &str) -> String {
        format!("__RUNNER_KV_{name}_BEGIN__{value}__RUNNER_KV_{name}_END__\n")
    }

    #[test]
    fn parses_path_and_proxy_quartet_ignoring_rc_banner() {
        let mut stdout = String::from("Welcome to zsh!\nnvm: using node v20\n");
        stdout.push_str(&block("PATH", "/opt/homebrew/bin:/usr/bin:/bin"));
        stdout.push_str(&block("HTTPS_PROXY", "http://127.0.0.1:7890"));
        stdout.push_str(&block("https_proxy", "http://127.0.0.1:7890"));
        stdout.push_str(&block("NO_PROXY", "localhost,127.0.0.1,*.byted.org"));
        stdout.push_str(&block("HTTP_PROXY", ""));
        let parsed = parse_login_shell_env(&stdout);
        assert_eq!(
            parsed.path.as_deref(),
            Some("/opt/homebrew/bin:/usr/bin:/bin")
        );
        assert_eq!(
            parsed.vars.get("HTTPS_PROXY").map(String::as_str),
            Some("http://127.0.0.1:7890"),
        );
        assert_eq!(
            parsed.vars.get("https_proxy").map(String::as_str),
            Some("http://127.0.0.1:7890"),
        );
        assert_eq!(
            parsed.vars.get("NO_PROXY").map(String::as_str),
            Some("localhost,127.0.0.1,*.byted.org"),
        );
        // Empty value (var was unset) is dropped — distinct from a
        // set-but-empty value, which we'd skip anyway since exporting
        // an empty proxy var has no useful effect on a child.
        assert!(!parsed.vars.contains_key("HTTP_PROXY"));
    }

    #[test]
    fn banner_mentioning_marker_substring_doesnt_shadow_real_block() {
        let mut stdout = String::from("echo: __RUNNER_KV_PATH_BEGIN__ (banner)\n");
        stdout.push_str(&block("PATH", "/usr/bin:/bin"));
        let parsed = parse_login_shell_env(&stdout);
        assert_eq!(parsed.path.as_deref(), Some("/usr/bin:/bin"));
    }

    #[test]
    fn missing_blocks_returns_default() {
        let parsed = parse_login_shell_env("just a banner");
        assert!(parsed.path.is_none());
        assert!(parsed.vars.is_empty());

        let parsed = parse_login_shell_env("__RUNNER_KV_PATH_BEGIN__only");
        assert!(parsed.path.is_none());
        assert!(parsed.vars.is_empty());

        let parsed = parse_login_shell_env("");
        assert!(parsed.path.is_none());
        assert!(parsed.vars.is_empty());
    }

    #[test]
    fn empty_value_between_markers_is_dropped() {
        let stdout = block("PATH", "") + &block("HTTPS_PROXY", "  \n  ");
        let parsed = parse_login_shell_env(&stdout);
        assert!(parsed.path.is_none());
        assert!(parsed.vars.is_empty());
    }
}
