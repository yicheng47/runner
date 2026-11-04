//! The Files view's service (#634): roots, listings, git status, worktrees,
//! reading and watching, all behind client-protocol requests so a remote
//! client can later be served the same way. Every path a client sends is a
//! root plus a path inside it, checked by [`fs::resolve`].

pub mod fs;
pub mod git;
pub mod root;
pub mod watch;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use runner_core::protocol::files::{
    DirEntry, FileContent, FilesRootOutcome, FilesSubject, GitStatus, WorktreeEntry,
};
use rusqlite::Connection;

use crate::error::{Error, Result};
use crate::events::EventChannel;
use crate::runtime_status::SharedShellEnv;
use git::Git;

/// The last git status per root, from `files_status` or the root's watcher,
/// which marks ignored entries in listings.
pub type StatusCache = Arc<Mutex<HashMap<PathBuf, GitStatus>>>;
const STATUS_CACHE_LIMIT: usize = 64;

fn remember(cache: &StatusCache, root: &Path, status: &GitStatus) {
    let mut cache = cache.lock().unwrap();
    if cache.len() >= STATUS_CACHE_LIMIT && !cache.contains_key(root) {
        cache.clear();
    }
    cache.insert(root.to_path_buf(), status.clone());
}

struct Watched {
    /// Subscriptions per client connection; one client can watch a root twice.
    subscribers: HashMap<u64, usize>,
    _watch: watch::RootWatch,
}

pub struct FilesService {
    shell_env: SharedShellEnv,
    events: EventChannel,
    git_missing_logged: AtomicBool,
    worktree_list_without_z: Arc<AtomicBool>,
    statuses: StatusCache,
    watches: Mutex<HashMap<PathBuf, Watched>>,
}

/// A service on its own shell environment and event channel, for cores
/// that never serve files.
impl Default for FilesService {
    fn default() -> Self {
        Self::new(Default::default(), Default::default())
    }
}

impl FilesService {
    pub fn new(shell_env: SharedShellEnv, events: EventChannel) -> Self {
        Self {
            shell_env,
            events,
            git_missing_logged: AtomicBool::new(false),
            worktree_list_without_z: Default::default(),
            statuses: Default::default(),
            watches: Default::default(),
        }
    }

    /// `git` from the login-shell PATH. Without it every root is plain.
    pub fn git(&self) -> Option<Git> {
        let path = crate::runtime_status::direct_chat_path(&self.shell_env.read().unwrap());
        let git = Git::find(path).map(|mut git| {
            git.worktree_list_without_z = self.worktree_list_without_z.clone();
            git
        });
        if git.is_none() && !self.git_missing_logged.swap(true, Ordering::Relaxed) {
            log::warn!("git is not on PATH; the Files view shows every root as a plain folder");
        }
        git
    }

    /// One `git status` of `root`; empty outside git.
    pub fn status(&self, root: &Path) -> Result<GitStatus> {
        fs::check_root(root)?;
        let status = match (git::checkout_top(root), self.git()) {
            (Some((_, prefix)), Some(git)) => git.status(root, &prefix)?,
            _ => GitStatus::default(),
        };
        remember(&self.statuses, root, &status);
        Ok(status)
    }

    /// Lists a folder without running git: ignored entries are marked from
    /// the root's last status, and unmarked before its first.
    pub fn list(&self, root: &Path, folder: &str) -> Result<Vec<DirEntry>> {
        let cached = self.statuses.lock().unwrap().get(root).cloned();
        fs::list(root, folder, cached.as_ref())
    }

    pub fn worktrees(&self, root: &Path) -> Result<Vec<WorktreeEntry>> {
        fs::check_root(root)?;
        match (git::checkout_top(root), self.git()) {
            (Some(_), Some(git)) => git.worktrees(root),
            _ => Ok(Vec::new()),
        }
    }

    /// Subscribes `client` to `root`'s `files/changed` and `files/status`.
    /// The first subscriber starts the root's watcher, which reports its
    /// first status as soon as it has run.
    pub fn watch(&self, client: u64, root: PathBuf) -> Result<()> {
        fs::check_root(&root)?;
        let mut watches = self.watches.lock().unwrap();
        if let Some(watched) = watches.get_mut(&root) {
            *watched.subscribers.entry(client).or_default() += 1;
            return Ok(());
        }
        let watch = watch::RootWatch::spawn(
            root.clone(),
            self.git(),
            self.events.clone(),
            self.statuses.clone(),
        )?;
        watches.insert(
            root,
            Watched {
                subscribers: HashMap::from([(client, 1)]),
                _watch: watch,
            },
        );
        Ok(())
    }

    pub fn unwatch(&self, client: u64, root: &Path) {
        let mut watches = self.watches.lock().unwrap();
        let Some(watched) = watches.get_mut(root) else {
            return;
        };
        if let Some(count) = watched.subscribers.get_mut(&client) {
            *count -= 1;
            if *count == 0 {
                watched.subscribers.remove(&client);
            }
        }
        if watched.subscribers.is_empty() {
            watches.remove(root);
        }
    }

    /// Releases every watch a disconnected client held.
    pub fn client_gone(&self, client: u64) {
        self.watches.lock().unwrap().retain(|_, watched| {
            watched.subscribers.remove(&client);
            !watched.subscribers.is_empty()
        });
    }

    #[cfg(test)]
    fn status_runs(&self, root: &Path) -> usize {
        self.watches.lock().unwrap()[root]._watch.status_runs()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn watched_roots(&self) -> Vec<PathBuf> {
        self.watches.lock().unwrap().keys().cloned().collect()
    }
}

/// The subject's saved root and its own working directory (`None` for a
/// chat or terminal outside a project).
fn subject_dirs(
    conn: &Connection,
    subject: &FilesSubject,
) -> Result<(Option<PathBuf>, Option<PathBuf>)> {
    let nonempty = |cwd: Option<String>| cwd.filter(|cwd| !cwd.trim().is_empty());
    let project_cwd = |project_id: &str| -> Result<Option<String>> {
        Ok(crate::repo::project::get(conn, project_id)?.map(|project| project.cwd))
    };
    let (saved, own) = match subject {
        FilesSubject::Session(id) => {
            let row = crate::repo::session::get_row(conn, id)?
                .ok_or_else(|| Error::msg(format!("session {id} not found")))?;
            let own = match row.project_id.as_deref() {
                None => None,
                Some(project_id) => match nonempty(row.cwd) {
                    Some(cwd) => Some(cwd),
                    None => project_cwd(project_id)?,
                },
            };
            (crate::repo::session::files_root(conn, id)?, own)
        }
        FilesSubject::Mission(id) => {
            let mission = crate::repo::mission::get(conn, id)?
                .ok_or_else(|| Error::msg(format!("mission {id} not found")))?;
            let own = match (nonempty(mission.cwd), mission.project_id.as_deref()) {
                (Some(cwd), _) => Some(cwd),
                (None, Some(project_id)) => project_cwd(project_id)?,
                (None, None) => None,
            };
            (crate::repo::mission::files_root(conn, id)?, own)
        }
    };
    Ok((saved.map(PathBuf::from), own.map(PathBuf::from)))
}

fn save_root(conn: &Connection, subject: &FilesSubject, root: Option<&str>) -> Result<()> {
    let updated = match subject {
        FilesSubject::Session(id) => crate::repo::session::set_files_root(conn, id, root)?,
        FilesSubject::Mission(id) => crate::repo::mission::set_files_root(conn, id, root)?,
    };
    if updated == 0 {
        return Err(Error::msg(format!("{subject:?} not found")));
    }
    Ok(())
}

pub fn root(core: &crate::AppCore, subject: &FilesSubject) -> Result<FilesRootOutcome> {
    let (saved, own) = subject_dirs(&*core.db.get()?, subject)?;
    let git = core.files.git();
    let resolved = root::resolve(git.as_ref(), saved.as_deref(), own.as_deref());
    if resolved.forget_saved {
        save_root(&*core.db.get()?, subject, None)?;
    }
    Ok(resolved.outcome)
}

/// Saves `path` as the subject's root and adds it to the recent folders;
/// `None` returns the subject to its own checkout.
pub fn set_root(
    core: &crate::AppCore,
    subject: &FilesSubject,
    path: Option<&Path>,
) -> Result<FilesRootOutcome> {
    let saved = match path {
        Some(path) => {
            fs::check_root(path)?;
            Some(
                path.to_str()
                    .ok_or_else(|| Error::msg(format!("{} is not UTF-8", path.display())))?,
            )
        }
        None => None,
    };
    save_root(&*core.db.get()?, subject, saved)?;
    if let Some(path) = path {
        crate::db::push_files_recent_folder(&core.db, path)?;
    }
    root(core, subject)
}

/// The recent folders that still exist, newest first.
pub fn recent_folders(core: &crate::AppCore) -> Result<Vec<PathBuf>> {
    let mut folders = crate::db::files_recent_folders(&core.db)?;
    folders.retain(|folder| folder.is_dir());
    Ok(folders)
}

/// Reads a file under `root`; `want_bytes` brings a binary file's bytes
/// back too.
pub fn read(root: &Path, path: &str, want_bytes: bool) -> Result<FileContent> {
    fs::read(root, path, want_bytes)
}

#[cfg(test)]
pub(crate) mod fixture {
    use std::path::Path;
    use std::process::Command;

    use super::git::Git;

    pub fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(dir)
            .args([
                "-c",
                "user.name=Runner",
                "-c",
                "user.email=runner@example.com",
            ])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    pub struct Repo {
        pub dir: tempfile::TempDir,
        pub git: Git,
    }

    impl Repo {
        pub fn path(&self) -> &Path {
            self.dir.path()
        }
    }

    /// A committed repository on `main` with `src/deep/x.rs`, `a.txt`,
    /// `old.txt`, `bin.dat` and a `.gitignore` of `target/`, `.worktrees/`
    /// and `*.log`.
    pub fn repo() -> Repo {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        git(path, &["init", "-q", "-b", "main"]);
        git(path, &["config", "core.autocrlf", "false"]);
        std::fs::write(path.join(".gitignore"), "target/\n.worktrees/\n*.log\n").unwrap();
        std::fs::create_dir_all(path.join("src/deep")).unwrap();
        std::fs::write(path.join("src/deep/x.rs"), "fn x() {}\n").unwrap();
        std::fs::write(path.join("a.txt"), "a\n").unwrap();
        std::fs::write(path.join("old.txt"), "old contents\n").unwrap();
        std::fs::write(path.join("bin.dat"), [0u8, 1, 2, 3]).unwrap();
        git(path, &["add", "-A"]);
        git(path, &["commit", "-q", "-m", "init"]);
        Repo {
            dir,
            git: Git::find(std::env::var("PATH").unwrap_or_default()).expect("git on PATH"),
        }
    }
}

#[cfg(test)]
mod tests;
