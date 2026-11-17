use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use gpui::{AppContext as _, Context};
use runner_core::app_paths::{
    is_existing_dir, normalize_checkout_path, same_checkout_path, same_normalized_checkout_path,
};
use runner_core::protocol::changes::ChangesCommits;
use runner_core::protocol::node::NodeType;
use runner_core::protocol::worktree::WorktreeCheckout;

use super::AppStore;
use crate::surfaces::sidebar::node_project_id;
use crate::PaneLayout;

/// Cache key identifying a working directory and optional project context for
/// derived git worktree queries.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct WorktreeQueryKey {
    pub(crate) cwd: String,
    pub(crate) project_id: Option<String>,
}

impl AppStore {
    /// Cached worktree checkout state for `(cwd, project_id)`, falling back to any
    /// cached entry for the same `cwd` when callers supply differing project hints.
    pub(crate) fn worktree_checkout_for(
        &self,
        cwd: &str,
        project_id: Option<&str>,
    ) -> Option<&WorktreeCheckout> {
        let exact = WorktreeQueryKey {
            cwd: cwd.to_owned(),
            project_id: project_id.map(str::to_owned),
        };
        if let Some(found) = self.worktree_checkouts.get(&exact) {
            return Some(found);
        }
        let target = normalize_checkout_path(Path::new(cwd));
        self.worktree_checkouts
            .iter()
            .find(|(key, _)| key.cwd == cwd || same_checkout_path(Path::new(&key.cwd), &target))
            .map(|(_, value)| value)
    }

    /// Cached `auto_base` for a linked worktree root, used by Info panels.
    pub(crate) fn worktree_auto_base_for(&self, top: &Path) -> Option<Option<&str>> {
        let normalized = normalize_checkout_path(top);
        self.worktree_auto_bases
            .iter()
            .find(|(path, _)| same_normalized_checkout_path(path, &normalized))
            .map(|(_, base)| base.as_deref())
    }

    /// Ensures `(cwd, project_id)` is tracked and watched if not yet in cache.
    pub(crate) fn ensure_worktree_checkout(
        &mut self,
        cwd: &str,
        project_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if cwd.trim().is_empty() {
            return;
        }
        let key = WorktreeQueryKey {
            cwd: cwd.to_owned(),
            project_id: project_id.map(str::to_owned),
        };
        if !self.worktree_extra_keys.insert(key) {
            return;
        }
        self.sync_worktree_checkouts(cx);
    }

    /// Ensures `auto_base` is loaded (or refreshed via watcher) for a linked worktree root.
    pub(crate) fn ensure_worktree_auto_base(&mut self, top: &Path, cx: &mut Context<Self>) {
        let normalized = normalize_checkout_path(top);
        if self.worktree_auto_bases.contains_key(&normalized) {
            return;
        }
        self.fetch_worktree_auto_bases(vec![normalized], cx);
    }

    /// Recomputes active local checkout keys from sessions, tabs, and missions,
    /// fetching any newly visible keys and synchronizing `#634` watches.
    pub(crate) fn sync_worktree_checkouts(&mut self, cx: &mut Context<Self>) {
        let desired = self.desired_worktree_keys();
        let removed_any = self
            .worktree_checkouts
            .keys()
            .any(|key| !desired.contains(key));
        if removed_any {
            self.worktree_checkouts
                .retain(|key, _| desired.contains(key));
            self.revisions.worktrees = self.revisions.worktrees.wrapping_add(1);
            self.sync_worktree_watches(cx);
        }
        let missing = desired
            .into_iter()
            .filter(|key| !self.worktree_checkouts.contains_key(key))
            .collect::<Vec<_>>();
        self.fetch_worktree_keys(missing, false, cx);
    }

    /// Handles a daemon `daemon/reconnected` notice: daemon watches were reset, so
    /// clear our watch set and re-resolve all active checkouts.
    pub(crate) fn rewatch_worktrees_after_reconnect(&mut self, cx: &mut Context<Self>) {
        self.watched_worktree_roots.clear();
        self.sync_worktree_watches(cx);
        self.refresh_all_worktrees_after_lag(cx);
    }

    /// Refreshes cached checkout states and bases after broadcast lag without
    /// re-subscribing file watches on an intact daemon connection.
    pub(crate) fn refresh_all_worktrees_after_lag(&mut self, cx: &mut Context<Self>) {
        let keys = self.desired_worktree_keys().into_iter().collect::<Vec<_>>();
        self.fetch_worktree_keys(keys, true, cx);
        let bases = self.worktree_auto_bases.keys().cloned().collect::<Vec<_>>();
        self.fetch_worktree_auto_bases(bases, cx);
    }

    /// Refreshes cached checkout states whose repo or worktree root matches `root`
    /// when a `#634` watcher notification arrives.
    pub(crate) fn refresh_worktrees_for_root(&mut self, root: &Path, cx: &mut Context<Self>) {
        let normalized_root = normalize_checkout_path(root);
        let keys = self
            .desired_worktree_keys()
            .into_iter()
            .filter(|key| {
                if let Some(state) = self.worktree_checkouts.get(key) {
                    worktree_checkout_matches_root(state, Path::new(&key.cwd), &normalized_root)
                } else {
                    same_checkout_path(Path::new(&key.cwd), &normalized_root)
                }
            })
            .collect::<Vec<_>>();
        self.fetch_worktree_keys(keys, true, cx);

        let bases = self
            .worktree_auto_bases
            .keys()
            .filter(|path| same_normalized_checkout_path(path, &normalized_root))
            .cloned()
            .collect::<Vec<_>>();
        self.fetch_worktree_auto_bases(bases, cx);
    }

    fn desired_worktree_keys(&self) -> BTreeSet<WorktreeQueryKey> {
        let mut keys = self.worktree_extra_keys.clone();
        let sessions_by_id = self
            .sessions
            .iter()
            .filter(|session| session.archived_at.is_none())
            .map(|session| (session.session_id.as_str(), session))
            .collect::<BTreeMap<_, _>>();

        for node in &self.nodes {
            if node.node_type != NodeType::Tab {
                continue;
            }
            let node_project = node_project_id(&self.nodes, node);
            let Ok(layout) = PaneLayout::from_node_row(node) else {
                continue;
            };
            for session_id in layout.session_ids() {
                if let Some(session) = sessions_by_id.get(session_id.as_str())
                    && let Some(cwd) = session.cwd.as_deref().filter(|c| !c.trim().is_empty())
                {
                    keys.insert(WorktreeQueryKey {
                        cwd: cwd.to_owned(),
                        project_id: node_project.clone().or_else(|| session.project_id.clone()),
                    });
                }
            }
        }

        for session in &self.sessions {
            if session.archived_at.is_some() || self.mission_agent_ids.contains(&session.session_id)
            {
                continue;
            }
            if let Some(cwd) = session.cwd.as_deref().filter(|c| !c.trim().is_empty()) {
                keys.insert(WorktreeQueryKey {
                    cwd: cwd.to_owned(),
                    project_id: session.project_id.clone(),
                });
            }
        }

        for summary in &self.missions {
            if summary.mission.archived_at.is_some() {
                continue;
            }
            let Some(cwd) = summary
                .mission
                .cwd
                .as_deref()
                .filter(|c| !c.trim().is_empty())
            else {
                continue;
            };
            let node_project = self
                .nodes
                .iter()
                .find(|node| {
                    node.node_type == NodeType::Mission
                        && node.ref_id.as_deref() == Some(summary.mission.id.as_str())
                })
                .and_then(|node| node_project_id(&self.nodes, node));
            keys.insert(WorktreeQueryKey {
                cwd: cwd.to_owned(),
                project_id: node_project.or_else(|| summary.mission.project_id.clone()),
            });
        }

        keys
    }

    fn fetch_worktree_keys(
        &mut self,
        keys: Vec<WorktreeQueryKey>,
        mark_refresh_if_busy: bool,
        cx: &mut Context<Self>,
    ) {
        let mut to_spawn = Vec::new();
        for key in keys {
            if self.worktree_in_flight.contains(&key) {
                if mark_refresh_if_busy {
                    self.worktree_refresh_needed.insert(key);
                }
            } else {
                self.worktree_in_flight.insert(key.clone());
                to_spawn.push(key);
            }
        }
        if to_spawn.is_empty() {
            return;
        }

        let client = self.client.clone();
        let task = cx.background_spawn(async move {
            to_spawn
                .into_iter()
                .map(|key| {
                    let result =
                        client.worktree_checkout(Path::new(&key.cwd), key.project_id.as_deref());
                    (key, result)
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |weak, cx| {
            let results = task.await;
            let _ = weak.update(cx, |this, cx| {
                let desired = this.desired_worktree_keys();
                let mut changed = false;
                let mut rerun = Vec::new();
                for (key, result) in results {
                    this.worktree_in_flight.remove(&key);
                    let needs_rerun = this.worktree_refresh_needed.remove(&key);
                    if !desired.contains(&key) {
                        continue;
                    }
                    if let Ok(state) = result
                        && this.worktree_checkouts.get(&key) != Some(&state)
                    {
                        this.worktree_checkouts.insert(key.clone(), state);
                        changed = true;
                    }
                    if needs_rerun {
                        rerun.push(key);
                    }
                }
                if changed {
                    this.revisions.worktrees = this.revisions.worktrees.wrapping_add(1);
                    this.sync_worktree_watches(cx);
                    cx.notify();
                }
                if !rerun.is_empty() {
                    this.fetch_worktree_keys(rerun, true, cx);
                }
            });
        })
        .detach();
    }

    fn fetch_worktree_auto_bases(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let mut to_spawn = Vec::new();
        for path in paths {
            if self.worktree_base_in_flight.contains(&path) {
                self.worktree_base_refresh_needed.insert(path);
            } else {
                self.worktree_base_in_flight.insert(path.clone());
                to_spawn.push(path);
            }
        }
        if to_spawn.is_empty() {
            return;
        }
        let client = self.client.clone();
        let task = cx.background_spawn(async move {
            to_spawn
                .into_iter()
                .map(|path| {
                    let result = client.changes_commits(&path, None);
                    (path, result)
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |weak, cx| {
            let results = task.await;
            let _ = weak.update(cx, |this, cx| {
                let mut changed = false;
                let mut rerun = Vec::new();
                for (path, result) in results {
                    this.worktree_base_in_flight.remove(&path);
                    let needs_rerun = this.worktree_base_refresh_needed.remove(&path);
                    let next: Option<String> = match result {
                        Ok(ChangesCommits { auto_base, .. }) => auto_base,
                        Err(_) => None,
                    };
                    if this.worktree_auto_bases.get(&path) != Some(&next) {
                        this.worktree_auto_bases.insert(path.clone(), next);
                        changed = true;
                    }
                    if needs_rerun {
                        rerun.push(path);
                    }
                }
                if changed {
                    this.revisions.worktrees = this.revisions.worktrees.wrapping_add(1);
                    cx.notify();
                }
                if !rerun.is_empty() {
                    this.fetch_worktree_auto_bases(rerun, cx);
                }
            });
        })
        .detach();
    }

    /// Registers an active sidebar consumer that needs live `#634` watcher
    /// updates for linked worktree checkouts.
    pub(crate) fn add_worktree_watch_consumer(&mut self, cx: &mut Context<Self>) {
        self.worktree_watch_consumers = self.worktree_watch_consumers.saturating_add(1);
        if self.worktree_watch_consumers == 1 {
            self.sync_worktree_watches(cx);
        }
    }

    /// Releases a sidebar consumer and drops background worktree watches once
    /// no sidebar views remain active.
    pub(crate) fn remove_worktree_watch_consumer(&mut self, cx: &mut Context<Self>) {
        self.worktree_watch_consumers = self.worktree_watch_consumers.saturating_sub(1);
        if self.worktree_watch_consumers == 0 {
            self.sync_worktree_watches(cx);
        }
    }

    fn sync_worktree_watches(&mut self, cx: &mut Context<Self>) {
        let mut desired = BTreeSet::new();
        if self.worktree_watch_consumers > 0 {
            for state in self.worktree_checkouts.values() {
                if let WorktreeCheckout::Git {
                    top,
                    main_checkout,
                    kind: runner_core::protocol::files::WorktreeKind::Linked,
                    missing,
                    ..
                } = state
                {
                    if *missing {
                        desired.insert(normalize_checkout_path(main_checkout));
                    } else {
                        desired.insert(normalize_checkout_path(top));
                    }
                }
            }
        }

        let to_unwatch = self
            .watched_worktree_roots
            .difference(&desired)
            .cloned()
            .collect::<Vec<_>>();
        let to_watch = desired
            .difference(&self.watched_worktree_roots)
            .cloned()
            .collect::<Vec<_>>();
        if to_unwatch.is_empty() && to_watch.is_empty() {
            return;
        }
        self.watched_worktree_roots = desired;
        let previous = self.worktree_watch_task.take();
        let client = self.client.clone();
        self.worktree_watch_task = Some(cx.background_spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            for root in to_unwatch {
                let _ = client.files_unwatch(&root);
            }
            for root in to_watch {
                if is_existing_dir(&root) {
                    let _ = client.files_watch(&root);
                }
            }
        }));
    }
}

fn worktree_checkout_matches_root(
    state: &WorktreeCheckout,
    cwd: &Path,
    normalized_root: &Path,
) -> bool {
    match state {
        WorktreeCheckout::Git {
            top, main_checkout, ..
        } => {
            same_normalized_checkout_path(top, normalized_root)
                || same_normalized_checkout_path(main_checkout, normalized_root)
        }
        WorktreeCheckout::Plain { .. } => same_checkout_path(cwd, normalized_root),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn lagged_event_stream_refreshes_worktrees_and_drops_watches_on_sidebar_release() {
        runner_app::i18n::testing::with_locale(crate::i18n::Locale::English, || {
            let temp = tempfile::Builder::new()
                .prefix("1213-m2-r6-")
                .tempdir()
                .unwrap();
            let repo = temp.path().join("repo");
            let wt = repo.join(".worktrees/feat-alpha");
            std::fs::create_dir_all(&repo).unwrap();
            let git = |dir: &Path, args: &[&str]| {
                let out = Command::new("git")
                    .args(args)
                    .current_dir(dir)
                    .env("GIT_AUTHOR_NAME", "T")
                    .env("GIT_AUTHOR_EMAIL", "t@e")
                    .env("GIT_COMMITTER_NAME", "T")
                    .env("GIT_COMMITTER_EMAIL", "t@e")
                    .output()
                    .unwrap();
                assert!(out.status.success(), "{args:?}");
            };
            git(&repo, &["init", "-q", "-b", "main"]);
            std::fs::write(repo.join("README.md"), "base\n").unwrap();
            git(&repo, &["add", "-A"]);
            git(&repo, &["commit", "-q", "-m", "init"]);
            git(
                &repo,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    "feat/alpha",
                    wt.to_str().unwrap(),
                ],
            );
            git(&wt, &["branch", "feat/beta"]);

            let mut cx = gpui::TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            let core = store.read_with(&cx, |s, _| s.test_core.clone());
            let project = runner_daemon::ops::project::project_create_checked(
                &core,
                "R6 Repo".into(),
                repo.to_string_lossy().into_owned(),
            )
            .unwrap();
            let wt_str = wt.to_string_lossy().into_owned();
            {
                let conn = core.db.get().unwrap();
                conn.execute(
                    "INSERT INTO sessions(id, status, agent_runtime, agent_command, project_id, cwd)
                     VALUES ('s-r6', 'stopped', 'codex', 'codex', ?1, ?2)",
                    [project.id.as_str(), wt_str.as_str()],
                )
                .unwrap();
            }
            store.update(&mut cx, |store, cx| {
                store.refresh(crate::app_store::StoreRefreshKind::All, cx);
            });
            let (sidebar, visual) = cx.add_window_view(|_, cx| {
                crate::surfaces::sidebar::Sidebar::new(
                    gpui::WeakEntity::new_invalid(),
                    store.clone(),
                    None,
                    cx,
                )
            });
            let norm_wt = normalize_checkout_path(&wt);

            let wait_until =
                |visual: &mut gpui::VisualTestContext,
                 label: &str,
                 mut pred: Box<dyn FnMut(&AppStore) -> bool>| {
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                    loop {
                        visual.run_until_parked();
                        let ok = store.read_with(visual, |s, _| pred(s));
                        if ok {
                            return;
                        }
                        assert!(std::time::Instant::now() < deadline, "timed out: {label}");
                        std::thread::yield_now();
                    }
                };

            wait_until(
                visual,
                "initial feat/alpha and watch",
                Box::new({
                    let wt_str = wt_str.clone();
                    let core = core.clone();
                    let norm_wt = norm_wt.clone();
                    move |s| {
                        matches!(
                            s.worktree_checkout_for(&wt_str, None),
                            Some(WorktreeCheckout::Git { branch: Some(b), .. }) if b == "feat/alpha"
                        ) && core.files.watched_roots().contains(&norm_wt)
                    }
                }),
            );

            // Switch branch locally while saturating the event bus (> 8192 channel capacity) to trigger Lagged.
            git(&wt, &["switch", "-q", "feat/beta"]);
            for i in 0..8500u64 {
                core.events.emit("test/flood", &i);
            }

            wait_until(
                visual,
                "lagged stream refreshes to feat/beta",
                Box::new({
                    let wt_str = wt_str.clone();
                    let core = core.clone();
                    let norm_wt = norm_wt.clone();
                    move |s| {
                        matches!(
                            s.worktree_checkout_for(&wt_str, None),
                            Some(WorktreeCheckout::Git { branch: Some(b), .. }) if b == "feat/beta"
                        ) && core.files.watched_roots().contains(&norm_wt)
                    }
                }),
            );

            // Closing the window drops the Sidebar, releasing its consumer lease and unwatching all roots.
            let weak_sidebar = sidebar.downgrade();
            drop(sidebar);
            visual.update(|window, _| window.remove_window());
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                visual.run_until_parked();
                if weak_sidebar.upgrade().is_none() && core.files.watched_roots().is_empty() {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "timed out waiting for sidebar watches to drop: {:?}",
                    core.files.watched_roots()
                );
                std::thread::yield_now();
            }
        });
    }
}
