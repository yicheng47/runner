//! Where a subject's Files view starts.

use std::path::{Path, PathBuf};

use runner_core::protocol::files::{
    FilesRoot, FilesRootOutcome, FilesRootSource, GitCheckout, WorktreeEntry,
};

use super::git::{checkout_top, Git};

#[derive(Debug, PartialEq)]
pub struct Resolved {
    pub outcome: FilesRootOutcome,
    /// The saved root no longer exists and should be cleared.
    pub forget_saved: bool,
}

/// Resolution order: a saved root that still exists; otherwise the subject's
/// own checkout from `own`, its working directory (`None` for a chat or
/// terminal outside a project). A saved root that is gone is forgotten and
/// resolution continues; an own working directory that is gone is missing.
pub fn resolve(git: Option<&Git>, saved: Option<&Path>, own: Option<&Path>) -> Resolved {
    let mut forget_saved = false;
    if let Some(saved) = saved {
        if saved.is_dir() {
            return Resolved {
                outcome: FilesRootOutcome::Root(describe(git, saved, FilesRootSource::Saved, own)),
                forget_saved,
            };
        }
        forget_saved = true;
    }
    let outcome = match own {
        None => FilesRootOutcome::ChooseFolder,
        Some(cwd) if !cwd.is_dir() => FilesRootOutcome::Missing {
            path: cwd.to_path_buf(),
        },
        Some(cwd) => FilesRootOutcome::Root(describe(git, cwd, FilesRootSource::Own, Some(cwd))),
    };
    Resolved {
        outcome,
        forget_saved,
    }
}

/// A saved root is the folder chosen; an own root is the top of the checkout
/// holding `dir`, or `dir` itself outside git. One `git worktree list` names
/// the checkout, its branch and commit.
fn describe(
    git: Option<&Git>,
    dir: &Path,
    source: FilesRootSource,
    cwd: Option<&Path>,
) -> FilesRoot {
    let checkout = git
        .filter(|_| checkout_top(dir).is_some())
        .and_then(|git| git.worktrees(dir).ok())
        .and_then(|worktrees| containing(worktrees, dir));
    let path = match (&checkout, source) {
        (Some(checkout), FilesRootSource::Own) => checkout.top.clone(),
        _ => dir.to_path_buf(),
    };
    let named = checkout.as_ref().map_or(path.as_path(), |git| &git.top);
    FilesRoot {
        name: named
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| named.display().to_string()),
        cwd: cwd.and_then(|cwd| relative_below(&path, cwd)),
        git: checkout,
        path,
        source,
    }
}

/// The worktree whose folder holds `dir` most closely.
fn containing(worktrees: Vec<WorktreeEntry>, dir: &Path) -> Option<GitCheckout> {
    let dir = dir.canonicalize().ok()?;
    worktrees
        .into_iter()
        .filter_map(|worktree| {
            let top = worktree.path.canonicalize().ok()?;
            dir.starts_with(&top)
                .then(|| (top.components().count(), worktree))
        })
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, worktree)| GitCheckout {
            top: worktree.path,
            worktree: worktree.kind,
            branch: worktree.branch,
            commit: worktree.commit,
        })
}

/// `dir` relative to `root` with `/` separators, when strictly below it.
pub fn relative_below(root: &Path, dir: &Path) -> Option<String> {
    let root = root.canonicalize().ok()?;
    let dir = dir.canonicalize().ok()?;
    let relative: PathBuf = dir.strip_prefix(&root).ok()?.to_path_buf();
    let parts = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::super::fixture::{git, repo};
    use super::*;
    use runner_core::protocol::files::WorktreeKind;

    fn root(resolved: Resolved) -> FilesRoot {
        match resolved.outcome {
            FilesRootOutcome::Root(root) => root,
            other => panic!("expected a root, got {other:?}"),
        }
    }

    fn same(a: &Path, b: &Path) -> bool {
        a.canonicalize().unwrap() == b.canonicalize().unwrap()
    }

    #[test]
    fn outside_a_project_asks_for_a_folder() {
        let git = Git::find(std::env::var("PATH").unwrap_or_default());
        assert_eq!(
            resolve(git.as_ref(), None, None),
            Resolved {
                outcome: FilesRootOutcome::ChooseFolder,
                forget_saved: false
            }
        );
    }

    #[test]
    fn in_a_project_opens_the_checkout_from_the_top() {
        let fixture = repo();
        let root = root(resolve(Some(&fixture.git), None, Some(fixture.path())));
        assert!(same(&root.path, fixture.path()));
        assert_eq!(root.source, FilesRootSource::Own);
        assert_eq!(root.cwd, None);
        let checkout = root.git.unwrap();
        assert_eq!(checkout.worktree, WorktreeKind::Main);
        assert_eq!(checkout.branch.as_deref(), Some("main"));
        assert_eq!(checkout.commit.as_ref().map(String::len), Some(7));
        assert_eq!(
            root.name,
            fixture.path().file_name().unwrap().to_string_lossy()
        );
    }

    #[test]
    fn a_working_directory_below_the_top_is_tagged() {
        let fixture = repo();
        let cwd = fixture.path().join("src").join("deep");
        let root = root(resolve(Some(&fixture.git), None, Some(&cwd)));
        assert!(same(&root.path, fixture.path()));
        assert_eq!(root.cwd.as_deref(), Some("src/deep"));
    }

    #[test]
    fn a_linked_worktree_opens_on_itself() {
        let fixture = repo();
        let worktree = fixture.path().join(".worktrees").join("feat-x");
        git(
            fixture.path(),
            &["worktree", "add", "-q", ".worktrees/feat-x", "-b", "feat/x"],
        );
        let root = root(resolve(Some(&fixture.git), None, Some(&worktree)));
        assert!(same(&root.path, &worktree));
        assert_eq!(root.name, "feat-x");
        let checkout = root.git.unwrap();
        assert_eq!(checkout.worktree, WorktreeKind::Linked);
        assert_eq!(checkout.branch.as_deref(), Some("feat/x"));
    }

    #[test]
    fn a_folder_outside_git_is_plain() {
        let fixture = repo();
        let plain = tempfile::tempdir().unwrap();
        let root = root(resolve(Some(&fixture.git), None, Some(plain.path())));
        assert_eq!(root.path, plain.path());
        assert_eq!(root.git, None);
        let without_git = super::resolve(None, None, Some(fixture.path()));
        assert_eq!(self::root(without_git).git, None);
    }

    #[test]
    fn a_saved_root_wins_and_keeps_the_chosen_folder() {
        let fixture = repo();
        let saved = fixture.path().join("src");
        let resolved = resolve(Some(&fixture.git), Some(&saved), Some(fixture.path()));
        assert!(!resolved.forget_saved);
        let root = root(resolved);
        assert_eq!(root.path, saved);
        assert_eq!(root.source, FilesRootSource::Saved);
        assert_eq!(
            root.name,
            fixture.path().file_name().unwrap().to_string_lossy()
        );
        assert!(same(&root.git.unwrap().top, fixture.path()));
    }

    #[test]
    fn a_saved_root_that_is_gone_is_forgotten() {
        let fixture = repo();
        let gone = fixture.path().join("gone");
        let resolved = resolve(Some(&fixture.git), Some(&gone), Some(fixture.path()));
        assert!(resolved.forget_saved);
        let root = root(resolved);
        assert_eq!(root.source, FilesRootSource::Own);
        assert!(same(&root.path, fixture.path()));
    }

    #[test]
    fn an_own_checkout_that_is_gone_is_missing() {
        let fixture = repo();
        let gone = fixture.path().join(".worktrees").join("merged");
        assert_eq!(
            resolve(Some(&fixture.git), None, Some(&gone)).outcome,
            FilesRootOutcome::Missing { path: gone }
        );
    }
}
