//! One watcher per watched root, on its own thread.
//!
//! Events under a folder the last status reported ignored (`target/`,
//! `.worktrees/`, `node_modules/`) are dropped as they arrive, so a build
//! never reaches the debounce. The rest collect until the root has been quiet
//! for [`DEBOUNCE`], or for at most [`MAX_WAIT`] while changes keep coming;
//! then one `files/changed` goes out and, when the files, the git directory's
//! `index` or `HEAD` or a branch changed, one `git status` runs. Status runs on
//! this thread, so changes during it coalesce into the next, and through the
//! root's [`StatusRuns`], so it waits out one that a view asked for.
//!
//! macOS and Windows watch the root recursively. Linux's inotify needs a
//! watch per folder, so it watches each folder that is not ignored, adding
//! folders as they appear; when the watch limit is reached it falls back to a
//! status every [`POLL`] while the root is watched. Windows always runs that
//! status as a safety net: notify 6 drops a `ReadDirectoryChangesW` buffer
//! overflow without a word, and a build's churn in ignored folders fills the
//! same buffer as an agent's edits.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use runner_core::protocol::files::{
    FilesChanged, FilesStatusChanged, GitStatus, FILES_CHANGED_EVENT, FILES_STATUS_EVENT,
};

use super::git::{Git, RepoPaths};
use super::{PreparedWatch, StatusRuns, StatusSince};
use crate::error::{Error, Result};
use crate::events::EventChannel;

pub const DEBOUNCE: Duration = Duration::from_millis(200);
pub const MAX_WAIT: Duration = Duration::from_secs(2);
pub const POLL: Duration = Duration::from_secs(5);
const IDLE: Duration = Duration::from_secs(60);

enum Message {
    Fs(notify::Result<Event>),
    Stop,
}

/// Keeps a root's watcher thread running; dropping it stops the thread.
pub struct RootWatch {
    tx: Sender<Message>,
    #[cfg_attr(not(test), allow(dead_code))]
    status_runs: Arc<AtomicUsize>,
}

impl RootWatch {
    pub fn spawn(
        root: PathBuf,
        prepared: PreparedWatch,
        events: EventChannel,
        runs: Arc<StatusRuns>,
    ) -> Result<Self> {
        let (tx, rx) = channel();
        let status_runs = Arc::new(AtomicUsize::new(0));
        let watch = Watch {
            root,
            root_canonical: prepared.root_canonical,
            git: prepared.git,
            repo: None,
            git_dir_canonical: None,
            common_dir_canonical: None,
            status: GitStatus::default(),
            events,
            runs,
            status_runs: status_runs.clone(),
            pending: Pending::default(),
            #[cfg(target_os = "linux")]
            watched: BTreeSet::new(),
            polling: cfg!(windows),
        };
        let notify_tx = tx.clone();
        std::thread::Builder::new()
            .name("files-watch".into())
            .spawn(move || watch.run(notify_tx, rx))
            .map_err(|error| Error::msg(format!("spawn files watcher: {error}")))?;
        Ok(Self { tx, status_runs })
    }

    #[cfg(test)]
    pub fn status_runs(&self) -> usize {
        self.status_runs.load(Ordering::SeqCst)
    }
}

impl Drop for RootWatch {
    fn drop(&mut self) {
        let _ = self.tx.send(Message::Stop);
    }
}

#[derive(Default)]
struct Pending {
    paths: BTreeSet<String>,
    rescan: bool,
    git: bool,
    first: Option<Instant>,
    last: Option<Instant>,
}

impl Pending {
    fn touch(&mut self) {
        let now = Instant::now();
        self.first.get_or_insert(now);
        self.last = Some(now);
    }

    fn due(&self) -> Option<Instant> {
        Some((self.last? + DEBOUNCE).min(self.first? + MAX_WAIT))
    }
}

struct Watch {
    root: PathBuf,
    root_canonical: PathBuf,
    git: Option<Git>,
    repo: Option<RepoPaths>,
    git_dir_canonical: Option<PathBuf>,
    common_dir_canonical: Option<PathBuf>,
    status: GitStatus,
    events: EventChannel,
    runs: Arc<StatusRuns>,
    status_runs: Arc<AtomicUsize>,
    pending: Pending,
    #[cfg(target_os = "linux")]
    watched: BTreeSet<PathBuf>,
    /// Run a status every [`POLL`], reporting it and the root as changed
    /// when it differs.
    polling: bool,
}

impl Watch {
    fn run(mut self, notify_tx: Sender<Message>, rx: Receiver<Message>) {
        let mut watcher = match notify::recommended_watcher(move |event| {
            let _ = notify_tx.send(Message::Fs(event));
        }) {
            Ok(watcher) => watcher,
            Err(error) => {
                log::warn!("files watcher for {}: {error}", self.root.display());
                return;
            }
        };
        self.repo = self.git.as_ref().and_then(|git| git.rev_parse(&self.root));
        if let Some(repo) = &self.repo {
            self.git_dir_canonical = repo.git_dir.canonicalize().ok();
            self.common_dir_canonical = repo.common_dir.canonicalize().ok();
            // What moves a branch without touching `index` or `HEAD`, such as
            // `git reset --soft`: its ref, which a linked worktree keeps in the
            // common directory, and a `packed-refs` or reftable there.
            let common = &repo.common_dir;
            let mut watches = vec![
                (repo.git_dir.clone(), RecursiveMode::NonRecursive),
                (common.join("refs/heads"), RecursiveMode::Recursive),
            ];
            if self.common_dir_canonical != self.git_dir_canonical {
                watches.push((common.clone(), RecursiveMode::NonRecursive));
            }
            if common.join("reftable").is_dir() {
                watches.push((common.join("reftable"), RecursiveMode::NonRecursive));
            }
            for (path, mode) in watches {
                if let Err(error) = watcher.watch(&path, mode) {
                    log::warn!("files watcher on {}: {error}", path.display());
                }
            }
        }
        // Every watch is in place before the first status is taken, so a
        // change while it runs arrives as an event and refreshes it. Linux
        // reads the ignored folders first, to leave them unwatched; any status
        // serves for that, since the one after the watches corrects it.
        #[cfg(target_os = "linux")]
        if let Some(status) = self.run_status(StatusSince::Running) {
            self.status = status;
        }
        self.watch_root(&mut watcher);
        if self.repo.is_some() {
            self.refresh_status(&mut watcher, true);
        }
        let mut next_poll = Instant::now() + POLL;
        loop {
            let now = Instant::now();
            let mut wake = self.pending.due().unwrap_or(now + IDLE);
            if self.polling {
                wake = wake.min(next_poll);
            }
            match rx.recv_timeout(wake.saturating_duration_since(now)) {
                Ok(Message::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                Ok(Message::Fs(Ok(event))) => self.take(&mut watcher, event),
                Ok(Message::Fs(Err(error))) => {
                    log::debug!("files watcher on {}: {error}", self.root.display());
                    self.pending.rescan = true;
                    self.pending.touch();
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            if self.pending.due().is_some_and(|due| due <= Instant::now()) {
                self.flush(&mut watcher);
            }
            if self.polling && next_poll <= Instant::now() {
                next_poll = Instant::now() + POLL;
                if self.repo.is_some() && self.refresh_status(&mut watcher, false) {
                    self.emit_changed(vec![String::new()], Vec::new());
                }
            }
        }
    }

    fn relative(&self, path: &Path) -> Option<String> {
        let relative = path
            .strip_prefix(&self.root)
            .or_else(|_| path.strip_prefix(&self.root_canonical))
            .ok()?;
        Some(
            relative
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/"),
        )
    }

    /// Whether `path` is one of the git files a status depends on: the git
    /// directory's `index` or `HEAD`, or a branch in the common directory.
    /// `None` when it is in neither directory.
    fn git_file(&self, path: &Path) -> Option<bool> {
        let repo = self.repo.as_ref()?;
        let inside = |dir: &Path, canonical: &Option<PathBuf>| {
            path.strip_prefix(dir)
                .ok()
                .or_else(|| path.strip_prefix(canonical.as_ref()?).ok())
                .map(Path::to_path_buf)
        };
        let own = inside(&repo.git_dir, &self.git_dir_canonical);
        let common = inside(&repo.common_dir, &self.common_dir_canonical);
        if own.is_none() && common.is_none() {
            return None;
        }
        let state = own.is_some_and(|own| own == Path::new("index") || own == Path::new("HEAD"));
        let branch = common.is_some_and(|common| {
            common == Path::new("packed-refs")
                || common == Path::new("reftable/tables.list")
                || (common.starts_with("refs/heads")
                    && common
                        .extension()
                        .is_none_or(|extension| extension != "lock"))
        });
        Some(state || branch)
    }

    fn ignored(&self, relative: &str) -> bool {
        relative.split('/').any(|part| part == ".git") || self.status.is_ignored(relative)
    }

    #[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
    fn take(&mut self, watcher: &mut RecommendedWatcher, event: Event) {
        let mut relevant = false;
        let mut dropped = false;
        for path in &event.paths {
            if let Some(tracked) = self.git_file(path) {
                if tracked {
                    self.pending.git = true;
                    relevant = true;
                } else {
                    dropped = true;
                }
                continue;
            }
            let Some(relative) = self.relative(path) else {
                dropped = true;
                continue;
            };
            if self.ignored(&relative) {
                dropped = true;
                continue;
            }
            // The root itself changed, as when it is moved: rescan it.
            if relative.is_empty() {
                self.pending.rescan = true;
                relevant = true;
                continue;
            }
            #[cfg(target_os = "linux")]
            self.follow_folders(watcher, &event, path);
            self.pending.paths.insert(relative);
            relevant = true;
        }
        if event.need_rescan() && (relevant || !dropped) {
            self.pending.rescan = true;
            relevant = true;
        }
        if relevant {
            self.pending.touch();
        }
    }

    fn flush(&mut self, watcher: &mut RecommendedWatcher) {
        let pending = std::mem::take(&mut self.pending);
        let files_changed = pending.rescan || !pending.paths.is_empty();
        if pending.rescan {
            self.emit_changed(vec![String::new()], Vec::new());
        } else if !pending.paths.is_empty() {
            let mut folders = BTreeSet::new();
            for path in &pending.paths {
                folders.insert(
                    path.rsplit_once('/')
                        .map_or("", |(parent, _)| parent)
                        .to_owned(),
                );
                if self.root.join(path).is_dir() {
                    folders.insert(path.clone());
                }
            }
            self.emit_changed(
                folders.into_iter().collect(),
                pending.paths.into_iter().collect(),
            );
        }
        if self.repo.is_some() && (files_changed || pending.git) {
            self.refresh_status(watcher, false);
        }
    }

    fn emit_changed(&self, folders: Vec<String>, paths: Vec<String>) {
        self.events.emit(
            FILES_CHANGED_EVENT,
            &FilesChanged {
                root: self.root.clone(),
                folders,
                paths,
            },
        );
    }

    fn run_status(&self, since: StatusSince) -> Option<GitStatus> {
        let (Some(git), Some(repo)) = (&self.git, &self.repo) else {
            return None;
        };
        self.status_runs.fetch_add(1, Ordering::SeqCst);
        self.runs
            .run(&self.root, since, || git.status(&self.root, &repo.prefix))
            .inspect_err(|error| log::warn!("files status of {}: {error}", self.root.display()))
            .ok()
    }

    /// Runs `git status` and reports it when it changed, or always on the
    /// first run. Returns whether it changed.
    #[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
    fn refresh_status(&mut self, watcher: &mut RecommendedWatcher, first: bool) -> bool {
        // An event brought this here, so a run that began before it won't do.
        let Some(status) = self.run_status(StatusSince::Now) else {
            return false;
        };
        let changed = status != self.status;
        #[cfg(target_os = "linux")]
        let ignored_changed = (&status.ignored, status.root_ignored)
            != (&self.status.ignored, self.status.root_ignored);
        self.status = status;
        if first || changed {
            self.events.emit(
                FILES_STATUS_EVENT,
                &FilesStatusChanged {
                    root: self.root.clone(),
                    status: self.status.clone(),
                },
            );
        }
        #[cfg(target_os = "linux")]
        if ignored_changed {
            self.watch_root(watcher);
        }
        changed
    }

    #[cfg(not(target_os = "linux"))]
    fn watch_root(&mut self, watcher: &mut RecommendedWatcher) {
        if let Err(error) = watcher.watch(&self.root, RecursiveMode::Recursive) {
            log::warn!("files watcher on {}: {error}", self.root.display());
        }
    }

    /// Watches every folder that is not ignored, and drops the watches of
    /// folders that became ignored.
    #[cfg(target_os = "linux")]
    fn watch_root(&mut self, watcher: &mut RecommendedWatcher) {
        if self.polling {
            return;
        }
        let wanted = self.folders_under(&self.root.clone());
        for gone in self
            .watched
            .difference(&wanted)
            .cloned()
            .collect::<Vec<_>>()
        {
            let _ = watcher.unwatch(&gone);
            self.watched.remove(&gone);
        }
        self.add_watches(watcher, wanted);
    }

    #[cfg(target_os = "linux")]
    fn folders_under(&self, start: &Path) -> BTreeSet<PathBuf> {
        let mut folders = BTreeSet::new();
        let mut stack = vec![start.to_path_buf()];
        while let Some(folder) = stack.pop() {
            match self.relative(&folder) {
                Some(relative) if !self.ignored(&relative) => {}
                _ => continue,
            }
            if let Ok(entries) = std::fs::read_dir(&folder) {
                for entry in entries.flatten() {
                    if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                        stack.push(entry.path());
                    }
                }
            }
            folders.insert(folder);
        }
        folders
    }

    #[cfg(target_os = "linux")]
    fn add_watches(&mut self, watcher: &mut RecommendedWatcher, folders: BTreeSet<PathBuf>) {
        for folder in folders {
            if self.watched.contains(&folder) {
                continue;
            }
            match watcher.watch(&folder, RecursiveMode::NonRecursive) {
                Ok(()) => {
                    self.watched.insert(folder);
                }
                Err(error) if matches!(error.kind, notify::ErrorKind::MaxFilesWatch) => {
                    log::warn!(
                        "inotify watch limit reached under {}; polling its git status every {} s",
                        self.root.display(),
                        POLL.as_secs()
                    );
                    for folder in std::mem::take(&mut self.watched) {
                        let _ = watcher.unwatch(&folder);
                    }
                    self.polling = true;
                    return;
                }
                Err(error) => log::debug!("files watcher on {}: {error}", folder.display()),
            }
        }
    }

    /// Adds watches for a folder that appeared and forgets those of one
    /// that went away.
    #[cfg(target_os = "linux")]
    fn follow_folders(&mut self, watcher: &mut RecommendedWatcher, event: &Event, path: &Path) {
        if self.polling {
            return;
        }
        match event.kind {
            notify::EventKind::Create(_)
            | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
                if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir()) =>
            {
                let folders = self.folders_under(path);
                self.add_watches(watcher, folders);
            }
            notify::EventKind::Remove(_)
            | notify::EventKind::Modify(notify::event::ModifyKind::Name(_)) => {
                let gone = self
                    .watched
                    .iter()
                    .filter(|folder| folder.starts_with(path))
                    .cloned()
                    .collect::<Vec<_>>();
                for folder in gone {
                    let _ = watcher.unwatch(&folder);
                    self.watched.remove(&folder);
                }
            }
            _ => {}
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use notify::event::CreateKind;
    use notify::EventKind;

    #[test]
    fn a_new_link_to_a_folder_is_not_followed_out_of_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(root.join("real/nested")).unwrap();
        std::fs::create_dir_all(outside.join("dataset/deep")).unwrap();
        let link = root.join("data");
        std::os::unix::fs::symlink(&outside, &link).unwrap();

        let mut watch = Watch {
            root_canonical: root.clone(),
            root: root.clone(),
            git: None,
            repo: None,
            git_dir_canonical: None,
            common_dir_canonical: None,
            status: GitStatus::default(),
            events: EventChannel::new(),
            runs: Default::default(),
            status_runs: Default::default(),
            pending: Pending::default(),
            watched: BTreeSet::new(),
            polling: false,
        };
        let mut watcher = notify::recommended_watcher(|_| {}).unwrap();
        let created = |path: &Path| {
            Event::new(EventKind::Create(CreateKind::Folder)).add_path(path.to_path_buf())
        };

        watch.follow_folders(&mut watcher, &created(&link), &link);
        assert!(watch.watched.is_empty(), "{:?}", watch.watched);

        let real = root.join("real");
        watch.follow_folders(&mut watcher, &created(&real), &real);
        assert_eq!(
            watch.watched,
            BTreeSet::from([real.clone(), real.join("nested")]),
            "a real folder is still followed"
        );
    }
}
