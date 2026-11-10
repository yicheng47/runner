//! The root picker's list: Choose folder…, then the repository's worktrees,
//! then projects and recent folders; and what its field's text means.

use std::path::{Path, PathBuf};

use runner_core::protocol::files::{WorktreeEntry, WorktreeKind};

use crate::text_util::{abbreviate_home, HomePath};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PickerKind {
    Worktree(WorktreeKind),
    Recent,
    Project,
    ChooseFolder,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PickerItem {
    pub(crate) kind: PickerKind,
    pub(crate) title: String,
    pub(crate) subtitle: String,
    /// `None` for Choose folder….
    pub(crate) path: Option<PathBuf>,
    pub(crate) current: bool,
}

pub(crate) fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

pub(crate) struct PickerSources<'a> {
    pub(crate) current: &'a Path,
    pub(crate) worktrees: &'a [WorktreeEntry],
    pub(crate) recent: &'a [PathBuf],
    /// `(name, folder)`; empty unless the subject is outside a project.
    pub(crate) projects: &'a [(String, PathBuf)],
    pub(crate) home: Option<&'a Path>,
    pub(crate) main_checkout_label: &'a str,
    pub(crate) detached_label: &'a str,
    pub(crate) choose_folder_label: &'a str,
}

pub(crate) fn picker_items(sources: PickerSources<'_>) -> Vec<PickerItem> {
    let mut worktrees = sources.worktrees.iter().collect::<Vec<_>>();
    worktrees.sort_by_key(|worktree| worktree.kind != WorktreeKind::Main);
    let mut items = vec![PickerItem {
        kind: PickerKind::ChooseFolder,
        title: sources.choose_folder_label.to_owned(),
        subtitle: String::new(),
        path: None,
        current: false,
    }];
    items.extend(worktrees.into_iter().map(|worktree| {
        let mut subtitle = vec![worktree
            .branch
            .clone()
            .unwrap_or_else(|| sources.detached_label.to_owned())];
        subtitle.extend(worktree.commit.clone());
        if worktree.kind == WorktreeKind::Main {
            subtitle.push(sources.main_checkout_label.to_owned());
        }
        PickerItem {
            kind: PickerKind::Worktree(worktree.kind),
            title: folder_name(&worktree.path),
            subtitle: subtitle.join(" · "),
            current: worktree.path == sources.current,
            path: Some(worktree.path.clone()),
        }
    }));
    let listed = |items: &[PickerItem], path: &Path| {
        items.iter().any(|item| item.path.as_deref() == Some(path))
    };
    for (name, path) in sources.projects {
        if !listed(&items, path) {
            items.push(PickerItem {
                kind: PickerKind::Project,
                title: name.clone(),
                subtitle: abbreviate_home(HomePath::Native(path, sources.home)),
                current: path == sources.current,
                path: Some(path.clone()),
            });
        }
    }
    for path in sources.recent {
        if !listed(&items, path) {
            items.push(PickerItem {
                kind: PickerKind::Recent,
                title: folder_name(path),
                subtitle: abbreviate_home(HomePath::Native(path, sources.home)),
                current: path == sources.current,
                path: Some(path.clone()),
            });
        }
    }
    items
}

/// The row Enter takes until the arrows move: the first folder listed, so
/// that Enter does not open the system picker from Choose folder… above it.
pub(crate) fn default_highlight(visible: usize) -> usize {
    usize::from(visible > 1)
}

/// The items whose name, branch or path contains `query`, ignoring case;
/// Choose folder… always stays.
pub(crate) fn filter(items: &[PickerItem], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            query.is_empty()
                || item.kind == PickerKind::ChooseFolder
                || item.title.to_lowercase().contains(&query)
                || item.subtitle.to_lowercase().contains(&query)
        })
        .map(|(index, _)| index)
        .collect()
}

/// The folder `text` names when it reads as a path rather than a filter:
/// absolute, or starting at the home folder with `~`.
pub(crate) fn typed_path(text: &str, home: Option<&Path>) -> Option<PathBuf> {
    let text = text.trim();
    if text == "~" {
        return home.map(Path::to_path_buf);
    }
    if let Some(rest) = text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
        return home.map(|home| home.join(rest));
    }
    let path = Path::new(text);
    path.is_absolute().then(|| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worktree(path: &str, branch: Option<&str>, kind: WorktreeKind) -> WorktreeEntry {
        WorktreeEntry {
            path: path.into(),
            branch: branch.map(Into::into),
            commit: Some("9094032".into()),
            kind,
            locked: false,
            prunable: false,
        }
    }

    fn items(current: &str, projects: &[(String, PathBuf)]) -> Vec<PickerItem> {
        let worktrees = [
            worktree(
                "/r/runner/.worktrees/feat-x",
                Some("feat/x"),
                WorktreeKind::Linked,
            ),
            worktree("/r/runner", Some("main"), WorktreeKind::Main),
            worktree("/r/runner/.worktrees/detached", None, WorktreeKind::Linked),
        ];
        let recent = [PathBuf::from("/r/runner"), PathBuf::from("/home/me/notes")];
        picker_items(PickerSources {
            current: Path::new(current),
            worktrees: &worktrees,
            recent: &recent,
            projects,
            home: Some(Path::new("/home/me")),
            main_checkout_label: "main checkout",
            detached_label: "detached",
            choose_folder_label: "Choose folder…",
        })
    }

    #[test]
    fn choose_folder_comes_first_then_worktrees_from_the_main_checkout_then_folders() {
        let projects = [("quill".to_owned(), PathBuf::from("/home/me/quill"))];
        let items = items("/r/runner/.worktrees/feat-x", &projects);
        let rows = items
            .iter()
            .map(|item| {
                let subtitle = item.subtitle.replace(std::path::MAIN_SEPARATOR, "/");
                (item.title.as_str(), subtitle, item.current)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            rows,
            [
                ("Choose folder…", String::new(), false),
                ("runner", "main · 9094032 · main checkout".into(), false),
                ("feat-x", "feat/x · 9094032".into(), true),
                ("detached", "detached · 9094032".into(), false),
                ("quill", "~/quill".into(), false),
                ("notes", "~/notes".into(), false),
            ]
        );
        assert_eq!(items[0].path, None);
        assert_eq!(items[4].kind, PickerKind::Project);
        assert_eq!(items[5].kind, PickerKind::Recent);
    }

    #[test]
    fn typing_filters_by_name_branch_or_path_and_keeps_choose_folder() {
        let items = items("/r/runner", &[]);
        let titles = |query: &str| {
            filter(&items, query)
                .into_iter()
                .map(|index| items[index].title.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(titles("FEAT"), ["Choose folder…", "feat-x"]);
        assert_eq!(titles("main"), ["Choose folder…", "runner"]);
        assert_eq!(titles("NOTES"), ["Choose folder…", "notes"]);
        assert_eq!(titles("zzz"), ["Choose folder…"]);
        assert_eq!(titles("  ").len(), items.len());
    }

    #[test]
    fn enter_starts_on_the_first_folder_unless_only_choose_folder_is_left() {
        assert_eq!(default_highlight(1), 0);
        assert_eq!(default_highlight(2), 1);
        assert_eq!(default_highlight(6), 1);
    }

    #[test]
    fn a_typed_path_is_absolute_or_from_home() {
        let home = Path::new("/home/me");
        assert_eq!(
            typed_path("~/repos/x ", Some(home)),
            Some(PathBuf::from("/home/me/repos/x"))
        );
        assert_eq!(typed_path("~", Some(home)), Some(home.to_path_buf()));
        assert_eq!(typed_path("runner", Some(home)), None);
        assert_eq!(typed_path("~/x", None), None);
        #[cfg(unix)]
        assert_eq!(typed_path("/tmp", None), Some(PathBuf::from("/tmp")));
        #[cfg(windows)]
        assert_eq!(typed_path(r"C:\tmp", None), Some(PathBuf::from(r"C:\tmp")));
    }
}
