//! Per-session launch/path helpers.
//!
//! The current PTY runtime uses the PATH/env composition helpers
//! directly before spawning the agent with `portable-pty`.

use std::path::Path;

/// Tool dirs we always include on the spawned process's PATH, even
/// when the shell-PATH resolver failed/timed out. Covers the most
/// common locations users install agent CLIs into. `~/`-prefixed
/// entries are expanded against the caller-provided HOME at compose
/// time.
const FALLBACK_CLI_DIRS: &[&str] = &[
    "~/.local/bin",
    "~/.cargo/bin",
    "~/.npm-global/bin",
    "~/.local/share/mise/shims",
    "~/.asdf/shims",
    "~/.volta/bin",
    "~/.bun/bin",
    "~/.deno/bin",
    "~/Library/pnpm",
    "~/.local/share/fnm/aliases/default/bin",
];

const FALLBACK_SYSTEM_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin"];

/// Compose the launched agent's PATH. Order:
///
/// 1. `shim_dir` (mission only — per-(mission, slot) `runner`
///    shim that injects mission-bus env vars).
/// 2. `bundled_bin_dir` (mission only — the bundled `runner` CLI
///    that the shim execs into; direct chats omit both to enforce
///    the off-bus invariant from PR #51).
/// 3. `shell_path` (best-effort login-shell PATH from
///    `shell_path::resolve_login_shell_env`, possibly None).
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
        for entry in std::env::split_paths(sp) {
            push(entry.to_string_lossy().into_owned());
        }
    }
    for fallback in fallback_cli_dirs(home) {
        push(fallback);
    }
    if let Some(pp) = process_path {
        for entry in std::env::split_paths(pp) {
            push(entry.to_string_lossy().into_owned());
        }
    }

    // The old Unix join treated literal colons in supplied directories as PATH separators.
    #[cfg(unix)]
    let parts = parts.iter().flat_map(std::env::split_paths);
    std::env::join_paths(parts)
        .expect("PATH entries must not contain the platform separator")
        .to_string_lossy()
        .into_owned()
}

fn fallback_cli_dirs(home: Option<&Path>) -> Vec<String> {
    let mut dirs = FALLBACK_CLI_DIRS
        .iter()
        .map(|p| expand_home(p, home))
        .collect::<Vec<_>>();
    dirs.extend(nvm_node_bin_dirs(home));
    dirs.extend(FALLBACK_SYSTEM_DIRS.iter().map(|p| expand_home(p, home)));
    dirs
}

fn nvm_node_bin_dirs(home: Option<&Path>) -> Vec<String> {
    let Some(versions_dir) = home.map(|home| home.join(".nvm/versions/node")) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(versions_dir) else {
        return Vec::new();
    };
    let mut versions = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let bin = entry.path().join("bin");
            bin.is_dir().then(|| {
                (
                    node_version_key(&entry.file_name().to_string_lossy()),
                    bin.display().to_string(),
                )
            })
        })
        .collect::<Vec<_>>();
    versions.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    versions.into_iter().map(|(_, path)| path).collect()
}

fn node_version_key(version: &str) -> Vec<u64> {
    version
        .trim_start_matches('v')
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
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

/// Env var names users must not set on a runner row. These are
/// owned by the launcher itself; letting a runner env entry shadow
/// them defeats the deterministic-spawn guarantees the launcher is
/// designed to provide. Currently a one-element list — `PATH` —
/// because that's the only one wiring the GUI-launch fix from
/// issue #65 hangs off; widen the list (e.g. `LD_LIBRARY_PATH`,
/// `DYLD_*`) only when a concrete need arises.
pub const RESERVED_ENV_NAMES: &[&str] = &["PATH"];

/// True if `s` is a name we ban runners from setting via their
/// env map. See `RESERVED_ENV_NAMES`.
pub fn is_reserved_env_name(s: &str) -> bool {
    RESERVED_ENV_NAMES.contains(&s)
}

/// True if `s` is a POSIX shell identifier suitable for `export
/// <name>=…`. Rules: first char is `[A-Za-z_]`, every subsequent
/// char is `[A-Za-z0-9_]`, length ≥ 1. Bash and zsh agree on this
/// shape. Platform process APIs can reject malformed names, while
/// names outside this shape cannot be referenced as ordinary variables
/// by shells the agent launches. Validate at every layer that touches
/// user-supplied env: the runner-edit form on persist and the runtime
/// spawn path both reject a bad name, so legacy or directly constructed
/// rows fail clearly before the child process is spawned.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
            Some(
                &std::env::join_paths(["/opt/homebrew/bin", "/usr/local/bin"])
                    .unwrap()
                    .to_string_lossy(),
            ),
            Some(Path::new("/Users/test")),
            Some(
                &std::env::join_paths(["/usr/bin", "/bin"])
                    .unwrap()
                    .to_string_lossy(),
            ),
        );
        // Doesn't contain the per-mission shim or "/runner/bin"
        // bundled-bin path shapes — neither was passed in. Version
        // manager fallback paths may legitimately contain "shims".
        assert!(!path.contains("/data/shims/build/bin"), "path = {path}");
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
            Some(
                &std::env::join_paths(["/usr/bin", "/bin"])
                    .unwrap()
                    .to_string_lossy(),
            ),
        );
        let parts: Vec<_> = std::env::split_paths(&path).collect();
        let shim_idx = parts
            .iter()
            .position(|p| p == Path::new("/data/shims/build/bin"))
            .unwrap();
        let bundled_idx = parts
            .iter()
            .position(|p| p == Path::new("/data/runner/bin"))
            .unwrap();
        let homebrew_idx = parts
            .iter()
            .position(|p| p == Path::new("/opt/homebrew/bin"))
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
            "/h/.local/share/mise/shims",
            "/h/.asdf/shims",
            "/h/.volta/bin",
            "/h/.bun/bin",
            "/h/.deno/bin",
            "/h/Library/pnpm",
            "/h/.local/share/fnm/aliases/default/bin",
            "/opt/homebrew/bin",
            "/usr/local/bin",
        ] {
            #[cfg(unix)]
            assert!(path.contains(d), "fallback {d} missing from {path}");
            #[cfg(windows)]
            assert!(
                std::env::split_paths(&path).any(|entry| entry == Path::new(d)),
                "fallback {d} missing from {path}"
            );
        }
    }

    #[test]
    fn expand_home_handles_tilde_and_passthrough() {
        let h = Path::new("/Users/jason");
        #[cfg(unix)]
        assert_eq!(
            expand_home("~/.cargo/bin", Some(h)),
            "/Users/jason/.cargo/bin"
        );
        #[cfg(windows)]
        assert_eq!(
            expand_home("~/.cargo/bin", Some(h)),
            h.join(".cargo/bin").to_string_lossy()
        );
        assert_eq!(expand_home("~", Some(h)), "/Users/jason");
        assert_eq!(expand_home("/abs/path", Some(h)), "/abs/path");
        // No HOME → tilde stays literal (compose_path will treat
        // it as just another absolute-ish entry; harmless).
        assert_eq!(expand_home("~/.cargo/bin", None), "~/.cargo/bin");
    }

    #[test]
    fn compose_path_keeps_shell_entries_before_seed_and_orders_nvm_newest_first() {
        let home = tempfile::tempdir().unwrap();
        for version in ["v9.9.9", "v20.12.1", "v18.20.0"] {
            std::fs::create_dir_all(
                home.path()
                    .join(".nvm/versions/node")
                    .join(version)
                    .join("bin"),
            )
            .unwrap();
        }
        let path = compose_path(
            None,
            None,
            Some(
                &std::env::join_paths(["/shell/bin", "/opt/homebrew/bin"])
                    .unwrap()
                    .to_string_lossy(),
            ),
            Some(home.path()),
            Some("/usr/bin"),
        );
        let parts = std::env::split_paths(&path).collect::<Vec<_>>();
        assert_eq!(parts[0], Path::new("/shell/bin"));
        assert_eq!(
            parts
                .iter()
                .filter(|entry| entry.as_path() == Path::new("/opt/homebrew/bin"))
                .count(),
            1
        );
        let nvm = parts
            .iter()
            .filter(|entry| {
                entry
                    .to_string_lossy()
                    .replace('\\', "/")
                    .contains(".nvm/versions/node")
            })
            .map(|entry| entry.to_string_lossy())
            .collect::<Vec<_>>();
        assert!(nvm[0].contains("v20.12.1"), "{nvm:?}");
        assert!(nvm[1].contains("v18.20.0"), "{nvm:?}");
        assert!(nvm[2].contains("v9.9.9"), "{nvm:?}");
    }

    #[test]
    fn compose_path_dedupes_repeats() {
        // shell_path already includes /opt/homebrew/bin; fallbacks
        // include it again. Compose should keep first occurrence
        // only.
        let path = compose_path(
            None,
            None,
            Some(
                &std::env::join_paths(["/opt/homebrew/bin", "/usr/local/bin"])
                    .unwrap()
                    .to_string_lossy(),
            ),
            Some(Path::new("/h")),
            Some("/usr/bin"),
        );
        let parts: Vec<_> = std::env::split_paths(&path).collect();
        let homebrew_count = parts
            .iter()
            .filter(|p| p.as_path() == Path::new("/opt/homebrew/bin"))
            .count();
        let local_count = parts
            .iter()
            .filter(|p| p.as_path() == Path::new("/usr/local/bin"))
            .count();
        assert_eq!(homebrew_count, 1, "homebrew bin should appear once: {path}");
        assert_eq!(local_count, 1, "local bin should appear once: {path}");
    }

    #[cfg(unix)]
    #[test]
    fn compose_path_keeps_literal_colons_as_unix_path_separators() {
        let path = compose_path(Some(Path::new("/shim:directory")), None, None, None, None);
        assert!(path.starts_with("/shim:directory:"));
    }

    #[cfg(windows)]
    #[test]
    fn compose_path_keeps_drive_letters_and_quoted_directories() {
        let shell_path = std::env::join_paths([r"C:\Tools", r"D:\Tools;extra"]).unwrap();
        let path = compose_path(
            Some(Path::new(r"C:\Runner\shim")),
            None,
            Some(shell_path.to_str().unwrap()),
            None,
            None,
        );
        let parts = std::env::split_paths(&path).take(3).collect::<Vec<_>>();
        assert_eq!(
            parts,
            [
                Path::new(r"C:\Runner\shim"),
                Path::new(r"C:\Tools"),
                Path::new(r"D:\Tools;extra")
            ]
        );
    }

    #[test]
    fn compose_path_omits_empty_segments() {
        // Empty shell_path / process_path values shouldn't produce
        // a `::` segment.
        let path = compose_path(None, None, Some(""), Some(Path::new("/h")), Some(""));
        assert!(
            std::env::split_paths(&path).all(|part| !part.as_os_str().is_empty()),
            "path = {path}"
        );
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
    fn is_reserved_env_name_covers_path() {
        assert!(is_reserved_env_name("PATH"));
        // Case-sensitive: a lowercase `path` would just be a
        // weird user var, not the launcher's PATH.
        assert!(!is_reserved_env_name("path"));
        assert!(!is_reserved_env_name("FOO"));
        assert!(!is_reserved_env_name(""));
    }
}
