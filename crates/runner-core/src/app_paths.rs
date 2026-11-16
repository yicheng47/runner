use std::ffi::OsString;
use std::fmt;
#[cfg(not(windows))]
use std::path::Component;
use std::path::{Path, PathBuf};

const APP_IDENTIFIER: &str = "com.wycstudios.runner";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IpcEndpoint(pub PathBuf);

impl fmt::Display for IpcEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.display().fmt(f)
    }
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::home_dir()
}

pub fn app_identifier(debug: bool) -> String {
    if debug {
        format!("{APP_IDENTIFIER}-dev")
    } else {
        APP_IDENTIFIER.to_string()
    }
}

pub fn app_data_dir(debug: bool) -> Option<PathBuf> {
    Some(app_data_dir_for_home(&home_dir()?, debug))
}

/// On Windows and Linux, `home` is the fallback when APPDATA or XDG_DATA_HOME is unset.
pub fn app_data_dir_for_home(home: &Path, debug: bool) -> PathBuf {
    #[cfg(target_os = "macos")]
    let base = home.join("Library").join("Application Support");
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local").join("share"));
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("AppData").join("Roaming"));
    base.join(app_identifier(debug))
}

pub fn log_dir(debug: bool) -> Option<PathBuf> {
    Some(log_dir_for_home(&home_dir()?, debug))
}

pub fn log_dir_for_home(home: &Path, debug: bool) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home.join("Library")
            .join("Logs")
            .join(app_identifier(debug))
    }
    #[cfg(not(target_os = "macos"))]
    {
        app_data_dir_for_home(home, debug).join("logs")
    }
}

pub fn mcp_endpoint(app_data_dir: &Path, debug: bool) -> IpcEndpoint {
    #[cfg(unix)]
    {
        let _ = debug;
        IpcEndpoint(app_data_dir.join("mcp.sock"))
    }
    #[cfg(windows)]
    {
        let _ = app_data_dir;
        #[cfg(debug_assertions)]
        if let Some(endpoint) = std::env::var_os("RUNNER_TEST_MCP_ENDPOINT") {
            return IpcEndpoint(endpoint.into());
        }
        IpcEndpoint(PathBuf::from(format!(
            r"\\.\pipe\{}",
            app_identifier(debug)
        )))
    }
}

pub fn daemon_endpoint(app_data_dir: &Path, debug: bool) -> IpcEndpoint {
    #[cfg(unix)]
    {
        let _ = debug;
        IpcEndpoint(app_data_dir.join("runnerd.sock"))
    }
    #[cfg(windows)]
    {
        let _ = app_data_dir;
        #[cfg(debug_assertions)]
        if let Some(endpoint) = std::env::var_os("RUNNER_TEST_DAEMON_ENDPOINT") {
            return IpcEndpoint(endpoint.into());
        }
        IpcEndpoint(PathBuf::from(if debug {
            r"\\.\pipe\com.wycstudios.runnerd-dev"
        } else {
            r"\\.\pipe\com.wycstudios.runnerd"
        }))
    }
}

/// Canonicalizes an existing checkout (or its nearest existing ancestor when a
/// checkout folder has been removed) and strips Windows's `\\?\` verbatim
/// prefix, resolving 8.3 short names and unifying separators so paths from
/// `git`, `std::fs::canonicalize`, and session/settings strings share one
/// identity on every platform.
pub fn normalize_checkout_path(path: &Path) -> PathBuf {
    if let Ok(canon) = path.canonicalize() {
        return clean_canonical_path(canon);
    }
    let mut missing_tail: Vec<OsString> = Vec::new();
    let mut cursor = path;
    while let Some(parent) = cursor.parent() {
        if parent.as_os_str().is_empty() || parent.parent().is_none() {
            break;
        }
        let Some(name) = cursor.file_name() else {
            break;
        };
        missing_tail.push(name.to_os_string());
        if let Ok(canon_parent) = parent.canonicalize() {
            let mut out = clean_canonical_path(canon_parent);
            for part in missing_tail.into_iter().rev() {
                out.push(part);
            }
            return out;
        }
        cursor = parent;
    }
    clean_lexical_path(path)
}

pub fn same_checkout_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    same_normalized_checkout_path(&normalize_checkout_path(a), &normalize_checkout_path(b))
}

pub fn same_normalized_checkout_path(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        match (a.to_str(), b.to_str()) {
            (Some(sa), Some(sb)) => sa.eq_ignore_ascii_case(sb),
            _ => a == b,
        }
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

pub fn normalized_checkout_starts_with(path: &Path, base: &Path) -> bool {
    #[cfg(windows)]
    {
        let mut path_components = path.components();
        for base_component in base.components() {
            let Some(path_component) = path_components.next() else {
                return false;
            };
            let p = path_component.as_os_str().to_string_lossy();
            let b = base_component.as_os_str().to_string_lossy();
            if !p.eq_ignore_ascii_case(&b) {
                return false;
            }
        }
        true
    }
    #[cfg(not(windows))]
    {
        path.starts_with(base)
    }
}

#[cfg(windows)]
fn clean_canonical_path(path: PathBuf) -> PathBuf {
    match path.to_str() {
        Some(raw) => PathBuf::from(normalize_windows_lexical(raw)),
        None => path,
    }
}

#[cfg(not(windows))]
fn clean_canonical_path(path: PathBuf) -> PathBuf {
    path
}

#[cfg(windows)]
fn clean_lexical_path(path: &Path) -> PathBuf {
    match path.to_str() {
        Some(raw) => PathBuf::from(normalize_windows_lexical(raw)),
        None => path.components().collect(),
    }
}

#[cfg(not(windows))]
fn clean_lexical_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else if !out.has_root() {
                    out.push(Component::ParentDir);
                }
            }
            other => out.push(other),
        }
    }
    if out.as_os_str().is_empty() && !path.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

#[cfg_attr(not(any(windows, test)), allow(dead_code))]
pub(crate) fn normalize_windows_lexical(raw: &str) -> String {
    let stripped;
    let s = if let Some(after) = raw.strip_prefix(r"\\?\") {
        if after
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"UNC\"))
        {
            stripped = format!(r"\\{}", &after[4..]);
            stripped.as_str()
        } else if after
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
            && after.as_bytes().get(1) == Some(&b':')
            && matches!(after.as_bytes().get(2), None | Some(b'\\') | Some(b'/'))
        {
            after
        } else {
            return raw.to_owned();
        }
    } else {
        raw
    };

    if let Some(after_slashes) = s.strip_prefix(r"\\").or_else(|| s.strip_prefix("//")) {
        if after_slashes.starts_with(".\\")
            || after_slashes.starts_with("./")
            || after_slashes == "."
        {
            return raw.to_owned();
        }
        let mut parts = after_slashes.split(['/', '\\']).filter(|p| !p.is_empty());
        let Some(server) = parts.next() else {
            return r"\\".to_owned();
        };
        let Some(share) = parts.next() else {
            return format!(r"\\{server}");
        };
        let mut segs: Vec<&str> = Vec::new();
        for part in parts {
            match part {
                "." => {}
                ".." => {
                    segs.pop();
                }
                other => segs.push(other),
            }
        }
        return if segs.is_empty() {
            format!(r"\\{server}\{share}")
        } else {
            format!(r"\\{server}\{share}\{}", segs.join(r"\"))
        };
    }

    if s.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && s.as_bytes().get(1) == Some(&b':')
    {
        let drive = char::from(s.as_bytes()[0].to_ascii_uppercase());
        let rest = &s[2..];
        let rooted = rest.starts_with(['/', '\\']);
        let mut segs: Vec<&str> = Vec::new();
        for part in rest.split(['/', '\\']) {
            match part {
                "" | "." => {}
                ".." => {
                    if segs.last().is_some_and(|last| *last != "..") {
                        segs.pop();
                    } else if !rooted {
                        segs.push("..");
                    }
                }
                other => segs.push(other),
            }
        }
        return if rooted {
            if segs.is_empty() {
                format!(r"{drive}:\")
            } else {
                format!(r"{drive}:\{}", segs.join(r"\"))
            }
        } else if segs.is_empty() {
            format!("{drive}:")
        } else {
            format!(r"{drive}:{}", segs.join(r"\"))
        };
    }

    let rooted = s.starts_with(['/', '\\']);
    let mut segs: Vec<&str> = Vec::new();
    for part in s.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                if segs.last().is_some_and(|last| *last != "..") {
                    segs.pop();
                } else if !rooted {
                    segs.push("..");
                }
            }
            other => segs.push(other),
        }
    }
    if rooted {
        if segs.is_empty() {
            r"\".to_owned()
        } else {
            format!(r"\{}", segs.join(r"\"))
        }
    } else if segs.is_empty() {
        if raw.is_empty() {
            String::new()
        } else {
            ".".to_owned()
        }
    } else {
        segs.join(r"\")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_endpoint_separates_development_and_production() {
        let home = Path::new("/home/tester");
        let release = mcp_endpoint(&app_data_dir_for_home(home, false), false);
        let debug = mcp_endpoint(&app_data_dir_for_home(home, true), true);
        assert_ne!(release, debug);
        #[cfg(unix)]
        assert!(release.0.ends_with("com.wycstudios.runner/mcp.sock"));
        #[cfg(windows)]
        {
            assert_eq!(release.to_string(), r"\\.\pipe\com.wycstudios.runner");
            assert_eq!(debug.to_string(), r"\\.\pipe\com.wycstudios.runner-dev");
        }
    }

    #[test]
    fn windows_lexical_normalization_strips_verbatim_and_unifies_separators() {
        assert_eq!(
            normalize_windows_lexical(r"\\?\C:\Users\runneradmin\AppData\Local\Temp\.tmpzV6IBL"),
            r"C:\Users\runneradmin\AppData\Local\Temp\.tmpzV6IBL"
        );
        assert_eq!(
            normalize_windows_lexical(
                "C:/Users/runneradmin/AppData/Local/Temp/.tmpzV6IBL/.worktrees/feat-x/"
            ),
            r"C:\Users\runneradmin\AppData\Local\Temp\.tmpzV6IBL\.worktrees\feat-x"
        );
        assert_eq!(
            normalize_windows_lexical(r"c:\Users\runneradmin\repo\.\sub\..\.worktrees/feat-x"),
            r"C:\Users\runneradmin\repo\.worktrees\feat-x"
        );
        assert_eq!(
            normalize_windows_lexical(r"\\?\UNC\server\share\repo/.worktrees/wt/"),
            r"\\server\share\repo\.worktrees\wt"
        );
        assert_eq!(normalize_windows_lexical(r"\\?\c:\"), r"C:\");
        assert_eq!(normalize_windows_lexical("C:/"), r"C:\");
        assert_eq!(
            normalize_windows_lexical(r"\\.\pipe\com.wycstudios.runner"),
            r"\\.\pipe\com.wycstudios.runner"
        );
        // Non-ASCII first directory segments (CJK, accented Latin, emoji) must not slice inside UTF-8 characters.
        assert_eq!(
            normalize_windows_lexical(r"\\?\C:\项目\repo/.worktrees/feat-x/"),
            r"C:\项目\repo\.worktrees\feat-x"
        );
        assert_eq!(
            normalize_windows_lexical(r"\\?\d:\Équipe\🚀-app\sub\..\repo"),
            r"D:\Équipe\🚀-app\repo"
        );
        assert_eq!(
            normalize_windows_lexical(r"\\?\unc\服务器\共享\项目/repo/"),
            r"\\服务器\共享\项目\repo"
        );
    }

    #[test]
    fn normalize_checkout_path_unifies_existing_and_missing_leaf_paths() {
        let temp = tempfile::tempdir().unwrap();
        let wt_parent = temp.path().join("runner").join(".worktrees");
        let live_wt = wt_parent.join("feat-live");
        std::fs::create_dir_all(&live_wt).unwrap();

        let norm_live = normalize_checkout_path(&live_wt);
        assert!(
            !norm_live.to_string_lossy().starts_with(r"\\?\"),
            "verbatim prefix must be stripped: {norm_live:?}"
        );
        let dotted = wt_parent.join(".").join("feat-live");
        assert_eq!(normalize_checkout_path(&dotted), norm_live);
        assert!(same_checkout_path(&live_wt, &dotted));

        let gone_wt = wt_parent.join("feat-gone");
        let gone_alt = temp
            .path()
            .join("runner")
            .join(".")
            .join(".worktrees")
            .join("feat-gone");
        assert_eq!(
            normalize_checkout_path(&gone_wt),
            normalize_checkout_path(&gone_alt)
        );
        assert!(same_checkout_path(&gone_wt, &gone_alt));
        assert!(normalized_checkout_starts_with(
            &normalize_checkout_path(&gone_wt),
            &normalize_checkout_path(&wt_parent),
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_minimal_service_environment_respects_xdg_and_development_roots() {
        if let Some(expected) = std::env::var_os("RUNNER_TEST_XDG_ROOT") {
            let expected = PathBuf::from(expected);
            let release = app_data_dir(false).unwrap();
            let development = app_data_dir(true).unwrap();
            assert_eq!(release, expected.join(app_identifier(false)));
            assert_eq!(development, expected.join(app_identifier(true)));
            assert_ne!(release, development);
            assert_eq!(
                daemon_endpoint(&release, false).0,
                release.join("runnerd.sock")
            );
            return;
        }
        let root = tempfile::tempdir().unwrap();
        for custom_xdg in [false, true] {
            let home = root.path().join("home");
            let expected = if custom_xdg {
                root.path().join("xdg")
            } else {
                home.join(".local/share")
            };
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command.args(["--exact", "app_paths::tests::linux_minimal_service_environment_respects_xdg_and_development_roots"])
                    .env_clear().env("HOME", home).env("RUNNER_TEST_XDG_ROOT", &expected);
            if custom_xdg {
                command.env("XDG_DATA_HOME", &expected);
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "custom_xdg={custom_xdg}: {}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }
}
