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

    fn apply_status(&mut self, status: Arc<PreparedStatus>, cx: &mut Context<Self>) {
        if let Body::Root(state) = &mut self.body {
            self.status_error = None;
            state.tree.set_status(status);
            self.note_ignored_tabs();
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
        }
    }
}
