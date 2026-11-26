use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Debug, serde::Serialize, Clone, Deserialize)]
pub struct ProjectDeleteOutcome {
    pub archived_mission_ids: Vec<String>,
    pub archived_session_ids: Vec<String>,
}

/// The project a new session or mission joins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectScope {
    /// The project that owns the working directory, if any.
    Infer,
    /// No project, whatever the working directory.
    Root,
    Project(String),
}

impl ProjectScope {
    /// For callers that may pass only a directory: the CLI and socket tools.
    pub fn or_infer(project_id: Option<String>) -> Self {
        project_id.map_or(Self::Infer, Self::Project)
    }

    /// For starts from inside the app, which belong to the place they were
    /// started from: a project, or the sidebar root.
    pub fn or_root(project_id: Option<String>) -> Self {
        project_id.map_or(Self::Root, Self::Project)
    }

    pub fn project_id(&self) -> Option<&str> {
        match self {
            Self::Project(project_id) => Some(project_id),
            Self::Infer | Self::Root => None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectRow {
    pub id: String,
    pub name: String,
    pub cwd: String,
    pub position: i64,
    pub created_at: String,
}

/// Which of `folders` holds `path`: the index of the deepest, compared by
/// path components so `runner-web` is not inside `runner`. With `resolve`,
/// both paths are compared as they exist on this computer when they do, so
/// a symlink or macOS's `/private` prefix still matches; without it, as
/// another computer's paths are, written with either separator.
pub fn deepest_folder_of<'a>(
    folders: impl IntoIterator<Item = &'a str>,
    path: &str,
    resolve: bool,
) -> Option<usize> {
    let target = Path::new(path);
    let resolved_target = resolve.then(|| target.canonicalize().ok()).flatten();
    folders
        .into_iter()
        .enumerate()
        .filter_map(|(index, folder)| {
            let depth = if resolve {
                let folder = Path::new(folder);
                let resolved_folder = folder.canonicalize().ok();
                match (resolved_target.as_deref(), resolved_folder.as_deref()) {
                    (Some(target), Some(folder)) => target
                        .starts_with(folder)
                        .then(|| folder.components().count()),
                    _ => target
                        .starts_with(folder)
                        .then(|| folder.components().count()),
                }
            } else {
                let folder = lexical_components(folder);
                let target = lexical_components(path);
                target.starts_with(&folder).then_some(folder.len())
            }?;
            Some((depth, index))
        })
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, index)| index)
}

fn lexical_components(path: &str) -> Vec<&str> {
    let (root, path) = if let Some(rest) = path.strip_prefix(r"\\?\") {
        match rest.get(..4) {
            Some(unc) if unc.eq_ignore_ascii_case(r"UNC\") => (Some("//"), &rest[4..]),
            _ => (None, rest),
        }
    } else if path.starts_with("\\\\") || path.starts_with("//") {
        (Some("//"), path)
    } else if path.starts_with(['/', '\\']) {
        (Some("/"), path)
    } else {
        (None, path)
    };
    root.into_iter()
        .chain(
            path.split(['/', '\\'])
                .filter(|part| !part.is_empty() && *part != "."),
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::deepest_folder_of;

    #[test]
    fn another_computers_paths_match_by_components_with_either_separator() {
        let folders = [
            "/Users/jason/repos/yicheng47",
            "/Users/jason/repos/yicheng47/runner",
            r"C:\host\work",
            r"\\nas\share",
        ];
        let find = |path| deepest_folder_of(folders, path, false);

        assert_eq!(find("/Users/jason/repos/yicheng47/runner"), Some(1));
        assert_eq!(
            find("/Users/jason/repos/yicheng47/runner/.worktrees/feat-968/"),
            Some(1)
        );
        assert_eq!(find("/Users/jason/repos/yicheng47/runner-web"), Some(0));
        assert_eq!(find(r"C:\host\work\app"), Some(2));
        assert_eq!(find(r"C:\host\workshop"), None);
        assert_eq!(find(r"\\nas\share\media"), Some(3));
        assert_eq!(find("/tmp/scratch"), None);
        assert_eq!(find("Users/jason/repos/yicheng47/runner"), None);
        assert_eq!(find("/nas/share/media"), None);
        assert_eq!(find(r"\\?\C:\host\work\app"), Some(2));
        assert_eq!(find(r"\\?\UNC\nas\share\media"), Some(3));
        let verbatim = [r"\\?\C:\host\work", r"\\?\UNC\nas\share"];
        assert_eq!(
            deepest_folder_of(verbatim, r"C:\host\work\app", false),
            Some(0)
        );
        assert_eq!(
            deepest_folder_of(verbatim, r"\\nas\share\media", false),
            Some(1)
        );
    }

    #[test]
    fn this_computers_paths_compare_where_they_exist() {
        let temp = tempfile::tempdir().unwrap();
        let runner = temp.path().join("runner");
        let worktree = runner.join(".worktrees/feat-968");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::create_dir_all(temp.path().join("runner-web")).unwrap();
        let runner = runner.to_string_lossy().into_owned();
        let missing = temp
            .path()
            .join("missing/app")
            .to_string_lossy()
            .into_owned();
        let folders = [runner.as_str(), missing.as_str()];
        let find = |path: &std::path::Path| {
            deepest_folder_of(folders, path.to_string_lossy().as_ref(), true)
        };

        assert_eq!(find(&worktree), Some(0));
        assert_eq!(find(&temp.path().join("runner-web")), None);
        assert_eq!(find(&temp.path().join("missing/app/src")), Some(1));
        #[cfg(unix)]
        {
            let alias = temp.path().join("alias");
            std::os::unix::fs::symlink(&runner, &alias).unwrap();
            assert_eq!(find(&alias.join(".worktrees/feat-968")), Some(0));
        }
    }
}
