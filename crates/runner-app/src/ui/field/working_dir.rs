use super::*;
use crate::text_util::{abbreviate_home, HomePath};

pub type WorkingDirField = BrowseField;

pub fn working_dir_placeholder(owner_path: Option<&str>, default_path: &str) -> String {
    owner_path
        .filter(|path| !path.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| effective_working_dir("", false, default_path))
        .unwrap_or_else(|| crate::i18n::text("ui.home_directory").to_owned())
}

pub fn working_dir_text_field(
    focus_handle: FocusHandle,
    text: impl Into<String>,
    placeholder: impl Into<crate::i18n::Text>,
) -> TextField {
    TextField::new(focus_handle, text, placeholder, true).compact_path()
}

pub(super) fn home_dir() -> Option<String> {
    std::env::home_dir().map(|home| home.to_string_lossy().into_owned())
}

/// A path's displays, longest first: the path with home shown as `~`, then
/// with more and more middle folders folded into `…`, down to the root or
/// `~` and the last folder alone.
pub fn compact_path_candidates(path: &str, home: Option<&str>) -> Vec<String> {
    let separator = if path.contains('\\') && !path.contains('/') {
        '\\'
    } else {
        '/'
    };
    let trimmed = path.trim_end_matches(separator);
    // A bare drive (`C:`) is drive-relative on Windows; the root keeps its
    // separator.
    let path = if trimmed.is_empty() || trimmed.ends_with(':') {
        path
    } else {
        trimmed
    };
    let abbreviated = abbreviate_home(HomePath::Text(path, home, separator));
    let mut parts = abbreviated.split(separator);
    let head = parts.next().unwrap_or_default().to_owned();
    let folders = parts.filter(|part| !part.is_empty()).collect::<Vec<_>>();
    let mut candidates = vec![abbreviated.clone()];
    for keep in (1..folders.len()).rev() {
        let tail = folders[folders.len() - keep..].join(&separator.to_string());
        candidates.push(format!("{head}{separator}…{separator}{tail}"));
    }
    // A relative path's head is a folder too, and can be long: fold it as
    // well so the last folder still shows. A root, `~` or a drive stays.
    if let Some(last) = folders.last() {
        if !head.is_empty() && head != "~" && !head.ends_with(':') {
            candidates.push(format!("…{separator}{last}"));
        }
    }
    candidates
}

/// The longest display that fits, else the shortest.
pub(crate) fn pick_compact_path(
    candidates: Vec<String>,
    mut fits: impl FnMut(&str) -> bool,
) -> String {
    let last = candidates.last().cloned().unwrap_or_default();
    candidates
        .into_iter()
        .find(|candidate| fits(candidate))
        .unwrap_or(last)
}

pub fn effective_working_dir(
    explicit_path: &str,
    owner_has_working_dir: bool,
    default_path: &str,
) -> Option<String> {
    let explicit = explicit_path.trim();
    if !explicit.is_empty() {
        return Some(explicit.to_owned());
    }
    if owner_has_working_dir {
        return None;
    }
    let default_path = default_path.trim();
    (!default_path.is_empty())
        .then(|| default_path.to_owned())
        .or_else(|| {
            runner_core::app_paths::home_dir()
                .and_then(|home| home.into_os_string().into_string().ok())
        })
}
