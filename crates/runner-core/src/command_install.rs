use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathStyle {
    Unix,
    Windows,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerCommandState {
    NotInstalled,
    Installed,
    Foreign,
    Shadowed,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RunnerCommandStatus {
    pub state: RunnerCommandState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadowed_by: Option<PathBuf>,
}

impl RunnerCommandStatus {
    pub fn not_installed() -> Self {
        Self {
            state: RunnerCommandState::NotInstalled,
            path: None,
            shadowed_by: None,
        }
    }

    pub fn unsupported() -> Self {
        Self {
            state: RunnerCommandState::Unsupported,
            path: None,
            shadowed_by: None,
        }
    }
}

pub const fn runner_command_name(debug: bool) -> &'static str {
    if debug {
        "runner-dev"
    } else {
        "runner"
    }
}

pub fn command_link_is_owned(path: &Path, sidecar: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
        && std::fs::read_link(path).is_ok_and(|target| target == sidecar)
}

pub fn path_entry_matches(entry: &Path, expected: &Path, style: PathStyle) -> bool {
    match style {
        PathStyle::Unix => entry == expected,
        PathStyle::Windows => normalize_windows_path(entry) == normalize_windows_path(expected),
    }
}

pub fn path_is_within(path: &Path, root: &Path, style: PathStyle) -> bool {
    match style {
        PathStyle::Unix => path.starts_with(root),
        PathStyle::Windows => {
            let path = normalize_windows_path(path);
            let root = normalize_windows_path(root);
            path == root
                || path
                    .strip_prefix(&root)
                    .is_some_and(|suffix| suffix.starts_with('\\'))
        }
    }
}

pub fn path_entries(value: &str, style: PathStyle) -> Vec<PathBuf> {
    let separator = match style {
        PathStyle::Unix => ':',
        PathStyle::Windows => ';',
    };
    value
        .split(separator)
        .filter(|entry| !entry.is_empty())
        .map(PathBuf::from)
        .collect()
}

pub fn inspect_unix_command(
    sidecar: &Path,
    login_path: &str,
    local_bin: &Path,
    system_bin: &Path,
    debug: bool,
) -> RunnerCommandStatus {
    let name = runner_command_name(debug);
    let candidates = [local_bin.join(name), system_bin.join(name)];
    if let Some(path) = candidates
        .iter()
        .find(|path| command_link_is_owned(path, sidecar))
    {
        let shadowed_by = shadowing_executable(
            login_path,
            path.parent().unwrap_or(Path::new("")),
            name,
            PathStyle::Unix,
        );
        return RunnerCommandStatus {
            state: if shadowed_by.is_some() {
                RunnerCommandState::Shadowed
            } else {
                RunnerCommandState::Installed
            },
            path: Some(path.clone()),
            shadowed_by,
        };
    }
    if let Some(path) = candidates.iter().find(|path| path_present(path)) {
        return RunnerCommandStatus {
            state: RunnerCommandState::Foreign,
            path: Some(path.clone()),
            shadowed_by: None,
        };
    }
    RunnerCommandStatus::not_installed()
}

pub fn inspect_windows_command(
    sidecar_dir: &Path,
    user_path: &str,
    search_path: &str,
    debug: bool,
) -> RunnerCommandStatus {
    if debug {
        return RunnerCommandStatus::unsupported();
    }
    if !path_entries(user_path, PathStyle::Windows)
        .iter()
        .any(|entry| path_entry_matches(entry, sidecar_dir, PathStyle::Windows))
    {
        return RunnerCommandStatus::not_installed();
    }
    let shadowed_by =
        shadowing_executable(search_path, sidecar_dir, "runner.exe", PathStyle::Windows);
    RunnerCommandStatus {
        state: if shadowed_by.is_some() {
            RunnerCommandState::Shadowed
        } else {
            RunnerCommandState::Installed
        },
        path: Some(sidecar_dir.to_path_buf()),
        shadowed_by,
    }
}

fn shadowing_executable(
    search_path: &str,
    installed_dir: &Path,
    name: &str,
    style: PathStyle,
) -> Option<PathBuf> {
    for entry in path_entries(search_path, style) {
        if path_entry_matches(&entry, installed_dir, style) {
            break;
        }
        let candidate = entry.join(name);
        if executable_file(&candidate, style) {
            return Some(candidate);
        }
    }
    None
}

fn path_present(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

fn executable_file(path: &Path, style: PathStyle) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    match style {
        PathStyle::Windows => true,
        PathStyle::Unix => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                metadata.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                true
            }
        }
    }
}

fn normalize_windows_path(path: &Path) -> String {
    let mut value = path.to_string_lossy().replace('/', "\\");
    while value.len() > 3 && value.ends_with('\\') {
        value.pop();
    }
    value.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_paths_ignore_case_and_trailing_separator() {
        assert!(path_entry_matches(
            Path::new(r"C:\Runner\bin\"),
            Path::new(r"c:\runner\BIN"),
            PathStyle::Windows,
        ));
        assert!(!path_entry_matches(
            Path::new(r"C:\Runner\bin-old"),
            Path::new(r"C:\Runner\bin"),
            PathStyle::Windows,
        ));
        assert!(path_is_within(
            Path::new(r"C:\Runner\data\missions\one"),
            Path::new(r"c:\runner\DATA\"),
            PathStyle::Windows,
        ));
        assert!(!path_is_within(
            Path::new(r"C:\Runner\data-old"),
            Path::new(r"C:\Runner\data"),
            PathStyle::Windows,
        ));
    }

    #[test]
    fn command_names_are_build_specific() {
        assert_eq!(runner_command_name(false), "runner");
        assert_eq!(runner_command_name(true), "runner-dev");
    }
}
