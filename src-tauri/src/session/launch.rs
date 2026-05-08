#![allow(dead_code)] // Wired into TmuxRuntime in Step 5; foundation now.

//! Per-session launcher script generator (Step 4 of
//! docs/impls/0004-tmux-session-runtime.md).
//!
//! The tmux runtime invokes
//! `tmux new-session … -- '<path-to-this-script>'` to spawn the
//! agent. The script:
//!
//!   1. Exports the composed PATH so the agent CLI can resolve
//!      regardless of launchd's stripped GUI PATH.
//!   2. Exports mission / direct-chat env vars (event-log path,
//!      slot handle, etc. for missions; nothing extra for direct
//!      chats).
//!   3. cds to the working directory.
//!   4. execs the agent command + argv.
//!
//! Why a script instead of letting tmux invoke argv directly: tmux's
//! trailing positional argument to `new-session` is a
//! `shell-command` string, not argv — tmux passes it through the
//! user's `default-shell -c`. Once we're crossing that boundary,
//! owning the script (with controlled quoting in Rust, written to
//! disk under our per-session runtime dir) is safer than building
//! a single shell string out of user-supplied env values and argv.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::runtime::{RuntimeError, RuntimeResult};

/// Tool dirs we always include on the spawned process's PATH, even
/// when the shell-PATH resolver failed/timed out. Covers the most
/// common locations users install agent CLIs into. `~/`-prefixed
/// entries are expanded against the caller-provided HOME at compose
/// time.
const FALLBACK_CLI_DIRS: &[&str] = &[
    "~/.local/bin",
    "~/.cargo/bin",
    "~/.npm-global/bin",
    "/opt/homebrew/bin",
    "/usr/local/bin",
];

/// Inputs `render_launch_script` needs that aren't already on
/// `SpawnSpec`. Kept separate from `SpawnSpec` because the runtime
/// computes some of these (composed PATH) on its own.
#[derive(Debug, Clone)]
pub struct LaunchScript {
    /// Agent CLI command name (`claude`, `codex`, …).
    pub command: String,
    /// Argv tail. Each element is single-quoted independently when
    /// rendered.
    pub args: Vec<String>,
    /// Working directory. None ⇒ omit the `cd` line entirely; the
    /// agent inherits whatever cwd the tmux server runs in.
    pub cwd: Option<PathBuf>,
    /// Per-session env vars. PATH must NOT be in here — pass it via
    /// `path` so we can be explicit that PATH is not user-supplied.
    pub env: BTreeMap<String, String>,
    /// The composed PATH value. See `compose_path`.
    pub path: String,
}

/// Compose the launched agent's PATH. Order:
///
/// 1. `shim_dir` (mission only — per-(mission, slot) `runner`
///    shim that injects mission-bus env vars).
/// 2. `bundled_bin_dir` (mission only — the bundled `runner` CLI
///    that the shim execs into; direct chats omit both to enforce
///    the off-bus invariant from PR #51).
/// 3. `shell_path` (best-effort login-shell PATH from
///    `shell_path::resolve_login_shell_path`, possibly None).
/// 4. Fallback CLI dirs (`~/.local/bin` etc.). Always included so
///    spawn correctness doesn't depend on the shell resolver
///    succeeding before a fixed timer.
/// 5. Process PATH (the launchd-stripped default on a Finder
///    launch; contains `/usr/bin`, `/bin` etc.).
///
/// Duplicate entries (e.g. shell PATH already includes
/// `/opt/homebrew/bin`) are collapsed to first-occurrence so the
/// resulting PATH stays compact.
pub fn compose_path(
    shim_dir: Option<&Path>,
    bundled_bin_dir: Option<&Path>,
    shell_path: Option<&str>,
    home: Option<&Path>,
    process_path: Option<&str>,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut push = |part: String| {
        if !part.is_empty() && !parts.iter().any(|p| p == &part) {
            parts.push(part);
        }
    };

    if let Some(shim) = shim_dir {
        push(shim.display().to_string());
    }
    if let Some(bin) = bundled_bin_dir {
        push(bin.display().to_string());
    }
    if let Some(sp) = shell_path {
        for entry in sp.split(':') {
            push(entry.to_string());
        }
    }
    for fallback in FALLBACK_CLI_DIRS {
        let expanded = expand_home(fallback, home);
        push(expanded);
    }
    if let Some(pp) = process_path {
        for entry in pp.split(':') {
            push(entry.to_string());
        }
    }

    parts.join(":")
}

/// Expand a leading `~/` against the caller's HOME. Non-tilde paths
/// pass through unchanged. We intentionally don't shell out for
/// expansion — keeping it pure makes the function trivially
/// testable.
fn expand_home(path: &str, home: Option<&Path>) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(h) = home {
            return h.join(rest).display().to_string();
        }
    }
    if path == "~" {
        if let Some(h) = home {
            return h.display().to_string();
        }
    }
    path.to_string()
}

/// True if `s` is a POSIX shell identifier suitable for `export
/// <name>=…`. Rules: first char is `[A-Za-z_]`, every subsequent
/// char is `[A-Za-z0-9_]`, length ≥ 1. Bash and zsh agree on this
/// shape; an invalid name (`FOO-BAR`, `FOO BAR`, or worse `FOO=x;
/// rm -rf /`) makes the launch script fail under `set -e` before
/// the agent starts, or — if rendered without escaping — runs
/// arbitrary shell. Validate at every layer that touches user-
/// supplied env: the runner-edit form on persist (rejects the row)
/// and `render_launch_script` on read (refuses to render a bad
/// legacy row), so a single missed validation can't turn into a
/// silent spawn-time crash.
pub fn is_valid_env_name(s: &str) -> bool {
    let mut chars = s.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Single-quote a string for safe inclusion in a bash command. Uses
/// the standard `'…'` form with internal `'` rendered as `'\''`
/// (close-quote, escaped quote, re-open). Works for any Unix shell
/// the launcher might run under.
pub fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Render the bash launcher to a string. Errors out (rather than
/// emitting a script that's guaranteed to fail under `set -e`) if
/// any env key isn't a valid POSIX shell identifier. This is the
/// last line of defence — the runner-edit form should also
/// validate at persist time so bad rows never reach the DB — but
/// rendering enforces the invariant for legacy rows that pre-date
/// the validation.
pub fn render_launch_script(script: &LaunchScript) -> RuntimeResult<String> {
    for k in script.env.keys() {
        if !is_valid_env_name(k) {
            return Err(RuntimeError::Msg(format!(
                "invalid env var name {k:?}: must match [A-Za-z_][A-Za-z0-9_]*"
            )));
        }
    }

    let mut out = String::new();
    out.push_str("#!/usr/bin/env bash\n");
    out.push_str("# Runner-generated session launcher — see\n");
    out.push_str("# docs/impls/0004-tmux-session-runtime.md (Step 4).\n");
    out.push_str("# Do not hand-edit; regenerated on every session spawn.\n");
    out.push_str("set -e\n");
    out.push_str(&format!("export PATH={}\n", shell_quote(&script.path)));
    // BTreeMap iter is alphabetical, so the rendered script is
    // stable across runs — useful for diffing one launcher against
    // another in a debugging context.
    for (k, v) in &script.env {
        out.push_str(&format!("export {}={}\n", k, shell_quote(v)));
    }
    if let Some(cwd) = &script.cwd {
        out.push_str(&format!("cd {}\n", shell_quote(&cwd.display().to_string())));
    }
    let cmd = shell_quote(&script.command);
    let args = script
        .args
        .iter()
        .map(|a| shell_quote(a))
        .collect::<Vec<_>>()
        .join(" ");
    if args.is_empty() {
        out.push_str(&format!("exec {cmd}\n"));
    } else {
        out.push_str(&format!("exec {cmd} {args}\n"));
    }
    Ok(out)
}

/// Write the rendered launcher to `dir/launch.sh` and chmod 700.
/// Returns the absolute path so the runtime can pass it to tmux.
/// Idempotent: rewrites every spawn so a stale script from a
/// crashed prior session doesn't get reused with the wrong env.
pub fn write_launch_script(dir: &Path, script: &LaunchScript) -> RuntimeResult<PathBuf> {
    let body = render_launch_script(script)?;
    std::fs::create_dir_all(dir)?;
    let path = dir.join("launch.sh");
    std::fs::write(&path, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)?.permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(&path, perms)?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_round_trips_through_bash() {
        for input in [
            "simple",
            "with spaces",
            "with 'single' quotes",
            "double \"quotes\" too",
            "$ENV_LIKE",
            "; rm -rf /",
            "tab\there",
            "newline\nhere",
            "",
        ] {
            // We can't actually exec bash in the unit test, but we
            // can sanity-check the quote shape: starts with `'`,
            // ends with `'`, internal `'` becomes `'\''`.
            let q = shell_quote(input);
            assert!(q.starts_with('\''), "quote = {q}");
            assert!(q.ends_with('\''), "quote = {q}");
            // Round-trip the escape: replace the `'\''` re-open
            // sequence back to a literal quote and strip the
            // outer quotes.
            let inner = &q[1..q.len() - 1];
            let unescaped = inner.replace("'\\''", "'");
            assert_eq!(unescaped, input, "quote = {q}");
        }
    }

    #[test]
    fn compose_path_direct_chat_omits_runner_cli_dirs() {
        // Off-bus invariant from PR #51: direct chats must not
        // see the bundled `runner` CLI on PATH.
        let path = compose_path(
            None,
            None,
            Some("/opt/homebrew/bin:/usr/local/bin"),
            Some(Path::new("/Users/test")),
            Some("/usr/bin:/bin"),
        );
        assert!(!path.contains("shims"), "path = {path}");
        // Doesn't contain "/runner/bin" (the bundled-bin path
        // shape) — it wasn't passed in.
        assert!(!path.contains("runner/bin"), "path = {path}");
        assert!(path.contains("/opt/homebrew/bin"), "path = {path}");
    }

    #[test]
    fn compose_path_mission_includes_shim_and_bundled_first() {
        let shim = PathBuf::from("/data/shims/build/bin");
        let bundled = PathBuf::from("/data/runner/bin");
        let path = compose_path(
            Some(&shim),
            Some(&bundled),
            Some("/opt/homebrew/bin"),
            Some(Path::new("/Users/test")),
            Some("/usr/bin:/bin"),
        );
        let parts: Vec<&str> = path.split(':').collect();
        let shim_idx = parts
            .iter()
            .position(|p| p == &"/data/shims/build/bin")
            .unwrap();
        let bundled_idx = parts.iter().position(|p| p == &"/data/runner/bin").unwrap();
        let homebrew_idx = parts
            .iter()
            .position(|p| p == &"/opt/homebrew/bin")
            .unwrap();
        assert!(
            shim_idx < bundled_idx,
            "shim must precede bundled bin: {path}"
        );
        assert!(
            bundled_idx < homebrew_idx,
            "bundled bin must precede shell PATH: {path}"
        );
    }

    #[test]
    fn compose_path_includes_fallback_cli_dirs() {
        // Even with shell_path = None (resolver failed), the
        // fallback dirs are present.
        let path = compose_path(None, None, None, Some(Path::new("/h")), Some("/usr/bin"));
        for d in [
            "/h/.local/bin",
            "/h/.cargo/bin",
            "/h/.npm-global/bin",
            "/opt/homebrew/bin",
            "/usr/local/bin",
        ] {
            assert!(path.contains(d), "fallback {d} missing from {path}");
        }
    }

    #[test]
    fn compose_path_dedupes_repeats() {
        // shell_path already includes /opt/homebrew/bin; fallbacks
        // include it again. Compose should keep first occurrence
        // only.
        let path = compose_path(
            None,
            None,
            Some("/opt/homebrew/bin:/usr/local/bin"),
            Some(Path::new("/h")),
            Some("/usr/bin"),
        );
        let parts: Vec<&str> = path.split(':').collect();
        let homebrew_count = parts.iter().filter(|p| **p == "/opt/homebrew/bin").count();
        let local_count = parts.iter().filter(|p| **p == "/usr/local/bin").count();
        assert_eq!(homebrew_count, 1, "homebrew bin should appear once: {path}");
        assert_eq!(local_count, 1, "local bin should appear once: {path}");
    }

    #[test]
    fn compose_path_omits_empty_segments() {
        // Empty shell_path / process_path values shouldn't produce
        // a `::` segment.
        let path = compose_path(None, None, Some(""), Some(Path::new("/h")), Some(""));
        assert!(!path.contains("::"), "path = {path}");
        assert!(!path.starts_with(':'), "path = {path}");
        assert!(!path.ends_with(':'), "path = {path}");
    }

    #[test]
    fn render_launch_script_has_set_e_and_exec() {
        let script = LaunchScript {
            command: "claude".into(),
            args: vec!["--permission-mode".into(), "acceptEdits".into()],
            cwd: Some(PathBuf::from("/work/proj")),
            env: BTreeMap::from([
                ("FOO".to_string(), "bar".to_string()),
                ("BAZ".to_string(), "with space".to_string()),
            ]),
            path: "/usr/bin:/bin".to_string(),
        };
        let body = render_launch_script(&script).unwrap();
        assert!(body.starts_with("#!/usr/bin/env bash\n"));
        assert!(body.contains("set -e\n"));
        assert!(body.contains("export PATH='/usr/bin:/bin'\n"));
        // BTreeMap order: BAZ before FOO.
        let baz_idx = body.find("export BAZ=").unwrap();
        let foo_idx = body.find("export FOO=").unwrap();
        assert!(baz_idx < foo_idx, "envs should be alphabetical: {body}");
        assert!(body.contains("export BAZ='with space'\n"));
        assert!(body.contains("cd '/work/proj'\n"));
        assert!(body.contains("exec 'claude' '--permission-mode' 'acceptEdits'\n"));
    }

    #[test]
    fn render_launch_script_handles_no_args_and_no_cwd() {
        let script = LaunchScript {
            command: "claude".into(),
            args: vec![],
            cwd: None,
            env: BTreeMap::new(),
            path: "/usr/bin".into(),
        };
        let body = render_launch_script(&script).unwrap();
        assert!(!body.contains("\ncd "), "no cd line expected: {body}");
        assert!(body.contains("exec 'claude'\n"));
    }

    #[test]
    fn render_launch_script_quotes_command_with_spaces() {
        // Defensive: tmux's shell-command boundary means an
        // unquoted command path with spaces would break. Step 4
        // tests this even though the runner row is unlikely to
        // ever have such a path.
        let script = LaunchScript {
            command: "/Applications/Weird App/bin/agent".into(),
            args: vec![],
            cwd: None,
            env: BTreeMap::new(),
            path: "/usr/bin".into(),
        };
        let body = render_launch_script(&script).unwrap();
        assert!(body.contains("exec '/Applications/Weird App/bin/agent'\n"));
    }

    #[test]
    fn write_launch_script_creates_executable_file() {
        let dir = tempfile::tempdir().unwrap();
        let script = LaunchScript {
            command: "echo".into(),
            args: vec!["hi".into()],
            cwd: None,
            env: BTreeMap::new(),
            path: "/usr/bin".into(),
        };
        let path = write_launch_script(dir.path(), &script).unwrap();
        assert!(path.is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "mode = {mode:o}");
        }
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("exec 'echo' 'hi'\n"));
    }

    #[test]
    fn expand_home_handles_tilde_and_passthrough() {
        let h = Path::new("/Users/jason");
        assert_eq!(
            expand_home("~/.cargo/bin", Some(h)),
            "/Users/jason/.cargo/bin"
        );
        assert_eq!(expand_home("~", Some(h)), "/Users/jason");
        assert_eq!(expand_home("/abs/path", Some(h)), "/abs/path");
        // No HOME → tilde stays literal (compose_path will treat
        // it as just another absolute-ish entry; harmless).
        assert_eq!(expand_home("~/.cargo/bin", None), "~/.cargo/bin");
    }

    #[test]
    fn is_valid_env_name_accepts_posix_identifiers() {
        for ok in ["FOO", "foo", "_under", "FOO_BAR", "X1", "_1", "F00"] {
            assert!(is_valid_env_name(ok), "{ok:?} should be valid");
        }
    }

    #[test]
    fn is_valid_env_name_rejects_bad_shapes() {
        for bad in [
            "",         // empty
            "1FOO",     // starts with digit
            "FOO-BAR",  // hyphen — bash export error
            "FOO BAR",  // space
            "FOO=x",    // assignment-shape
            "FOO;rm",   // shell metachar — script-injection vector
            "FOO\nBAR", // newline
            "FOO.BAR",  // period
            "FOO/BAR",  // slash
            "ünicode",  // non-ASCII
        ] {
            assert!(!is_valid_env_name(bad), "{bad:?} should be invalid");
        }
    }

    #[test]
    fn render_launch_script_rejects_invalid_env_name() {
        let script = LaunchScript {
            command: "claude".into(),
            args: vec![],
            cwd: None,
            env: BTreeMap::from([("FOO-BAR".to_string(), "value".to_string())]),
            path: "/usr/bin".into(),
        };
        let err = render_launch_script(&script).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("FOO-BAR"),
            "error should name the bad var: {msg}"
        );
        assert!(
            msg.contains("[A-Za-z_]"),
            "error should explain the rule: {msg}"
        );
    }

    #[test]
    fn write_launch_script_propagates_invalid_env_error() {
        let dir = tempfile::tempdir().unwrap();
        let script = LaunchScript {
            command: "claude".into(),
            args: vec![],
            cwd: None,
            env: BTreeMap::from([("FOO BAR".to_string(), "value".to_string())]),
            path: "/usr/bin".into(),
        };
        let err = write_launch_script(dir.path(), &script).unwrap_err();
        assert!(err.to_string().contains("FOO BAR"));
        // No file should be left behind on validation failure.
        assert!(!dir.path().join("launch.sh").exists());
    }
}
