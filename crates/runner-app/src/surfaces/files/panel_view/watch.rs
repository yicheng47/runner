//! Watch lifetime, daemon file events and git-status updates.

use super::*;

impl FilesView {
    pub(crate) fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.shown == shown {
            return;
        }
        self.shown = shown;
        if !shown {
            self.picker = None;
            self.finder = None;
            self.menu = None;
        }
        self.ignored_refresh = shown.then(|| self.refresh_ignored_tabs(cx));
        self.sync_watch(cx);
    }

    pub(super) fn watched_root_changed(&mut self, cx: &mut Context<Self>) {
        if self.watched.as_deref() != self.root_path() {
            self.unwatch(cx);
        }
        self.sync_watch(cx);
    }

    fn chain_watch(
        &mut self,
        cx: &mut Context<Self>,
        call: impl FnOnce(&DaemonClient) + Send + 'static,
    ) {
        let previous = self.watch_chain.take();
        let client = self.client.clone();
        self.watch_chain = Some(cx.background_spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            call(&client);
        }));
    }

    fn unwatch(&mut self, cx: &mut Context<Self>) {
        if let Some(root) = self.watched.take() {
            self.chain_watch(cx, move |client| {
                let _ = client.files_unwatch(&root);
            });
        }
    }

    /// Watches the root while the view is shown, and reads its status once
    /// watching, since a root someone else already watches reports none.
    pub(super) fn sync_watch(&mut self, cx: &mut Context<Self>) {
        let wanted = self
            .shown
            .then(|| self.root_path().map(Path::to_path_buf))
            .flatten();
        if self.watched == wanted {
            return;
        }
        self.unwatch(cx);
        let Some(root) = wanted else {
            return;
        };
        self.watched = Some(root.clone());
        // Changes made while unwatched never arrived, so list and read again.
        if let Some(tree) = self.tree_ref() {
            let folders = tree.loaded_folders();
            self.load(folders, cx);
            self.reload_changed(None, cx);
        }
        let (status_tx, status_rx) = futures::channel::oneshot::channel();
        self.chain_watch(cx, move |client| {
            let _ = status_tx.send(
                client
                    .files_watch(&root)
                    .and_then(|()| client.files_status(&root))
                    .map(PreparedStatus::new),
            );
        });
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let Ok(result) = status_rx.await else { return };
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(status) => this.apply_status(status, cx),
                    Err(error) => {
                        this.status_error = Some(error.to_string().into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    pub(in super::super) fn has_uncommitted_diff(&self, path: &str) -> Option<bool> {
        let Body::Root(state) = &self.body else {
            return Some(false);
        };
        if state.root.git.is_none() {
            return Some(false);
        }
        state
            .uncommitted_paths
            .as_ref()
            .map(|paths| paths.contains(path))
    }

    pub(in super::super) fn clear_uncommitted_path(&mut self, path: &str) {
        if let Body::Root(state) = &mut self.body
            && let Some(paths) = &mut state.uncommitted_paths
        {
            paths.remove(path);
        }
    }

    pub(in super::super) fn ensure_uncommitted_paths(&mut self, cx: &mut Context<Self>) {
        if matches!(&self.body, Body::Root(state) if state.uncommitted_paths.is_none()) {
            self.refresh_uncommitted_paths(false, cx);
        }
    }

    pub(in super::super) fn refresh_uncommitted_paths(
        &mut self,
        refresh_active_diffs: bool,
        cx: &mut Context<Self>,
    ) {
        let has_tabs = self.has_tabs();
        let (root, seq) = match &mut self.body {
            Body::Root(state) => {
                if !has_tabs {
                    state.uncommitted_paths = None;
                    state.uncommitted_seq += 1;
                    return;
                }
                if state.root.git.is_none() {
                    state.uncommitted_paths = Some(Default::default());
                    self.reconcile_diff_tabs(false, cx);
                    return;
                }
                state.uncommitted_seq += 1;
                (state.root.path.clone(), state.uncommitted_seq)
            }
            _ => return,
        };
        self.request(
            cx,
            move |client| {
                client.changes_files(
                    &root,
                    &runner_core::protocol::changes::ChangesTarget::Uncommitted,
                    None,
                )
            },
            move |this, result, cx| {
                let Body::Root(state) = &mut this.body else {
                    return;
                };
                if state.uncommitted_seq != seq {
                    return;
                }
                if let Ok(files) = result {
                    state.uncommitted_paths =
                        Some(files.files.into_iter().map(|entry| entry.path).collect());
                    this.reconcile_diff_tabs(refresh_active_diffs, cx);
                    cx.notify();
                }
            },
        );
    }

    fn apply_status(&mut self, status: Arc<PreparedStatus>, cx: &mut Context<Self>) {
        if let Body::Root(state) = &mut self.body {
            self.status_error = None;
            state.tree.set_status(status);
            self.note_ignored_tabs();
            self.reconcile_diff_tabs(false, cx);
            self.refresh_uncommitted_paths(true, cx);
            cx.notify();
        }
    }

    pub(super) fn on_files_event(&mut self, event: &FilesEvent, cx: &mut Context<Self>) {
        match event {
            // Hidden, each of these is wasted: showing it reads everything again.
            FilesEvent::Changed(changed) => {
                if !self.shown || self.root_path() != Some(changed.root.as_path()) {
                    return;
                }
                let Body::Root(state) = &self.body else {
                    return;
                };
                // No paths: the watcher cannot say what changed, so a folder
                // listed earlier may be out of date however deep it is.
                let folders = if changed.paths.is_empty() {
                    state.tree.loaded_folders()
                } else {
                    state.tree.listed(&changed.folders)
                };
                self.load(folders, cx);
                self.reload_changed(Some(changed), cx);
                self.refresh_uncommitted_paths(false, cx);
            }
            FilesEvent::Status { root, status } => {
                if self.shown && self.root_path() == Some(root.as_path()) {
                    self.apply_status(status.clone(), cx);
                }
            }
            FilesEvent::Resync { rewatch } => {
                if self.context.remote {
                    return;
                }
                let Body::Root(state) = &self.body else {
                    self.resolve(cx);
                    return;
                };
                // Showing it reads everything again, with a new watch.
                if !self.shown {
                    return;
                }
                let folders = state.tree.loaded_folders();
                if *rewatch {
                    // The daemon forgot its watches; nothing to release.
                    self.watched = None;
                    self.sync_watch(cx);
                } else if let Some(root) = self.watched.clone() {
                    self.request(
                        cx,
                        move |client| client.files_status(&root).map(PreparedStatus::new),
                        |this, result, cx| {
                            if let Ok(status) = result {
                                this.apply_status(status, cx);
                            }
                        },
                    );
                }
                self.load(folders, cx);
                self.reload_changed(None, cx);
            }
            FilesEvent::Viewed(_) => {}
            FilesEvent::RootChanged { subject, origin } => {
                if &self.context.subject == subject && cx.entity_id() != *origin {
                    self.resolve(cx);
                }
            }
        }
    }
}
