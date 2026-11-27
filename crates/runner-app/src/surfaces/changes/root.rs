use super::*;

impl ChangesView {
    pub(super) fn request<T: Send + 'static>(
        &self,
        cx: &mut Context<Self>,
        call: impl FnOnce(&DaemonClient) -> Result<T, ClientError> + Send + 'static,
        then: impl FnOnce(&mut Self, Result<T, ClientError>, &mut Context<Self>) + 'static,
    ) {
        let client = self.client.clone();
        let generation = self.generation;
        let task = cx.background_spawn(async move { call(&client) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    then(this, result, cx);
                }
            });
        })
        .detach();
    }

    pub(super) fn resolve(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        if self.context.remote {
            self.body = Body::Remote;
            self.sync_watch(cx);
            cx.notify();
            return;
        }
        let subject = subject(&self.context);
        self.request(
            cx,
            move |client| client.files_root(&subject),
            |this, result, cx| this.apply_outcome(result, cx),
        );
    }

    pub(super) fn set_root(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.context.remote {
            return;
        }
        self.base_picker = None;
        self.checkout_picker = None;
        self.removed_checkout_notice = None;
        self.root_change += 1;
        let change = self.root_change;
        let subject = subject(&self.context);
        let client = self.client.clone();
        let previous = self.root_chain.take();
        let (reply, result) = futures::channel::oneshot::channel();
        self.root_chain = Some(cx.background_spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            let _ = reply.send(client.files_set_root(&subject, path.as_deref()));
        }));
        cx.spawn(async move |this, cx| {
            let Ok(result) = result.await else { return };
            let _ = this.update(cx, |this, cx| {
                if this.root_change != change {
                    return;
                }
                match result {
                    Ok(outcome) => {
                        let panel_subject = this.context.subject.clone();
                        let origin = cx.entity_id();
                        let client = this.client.clone();
                        this.apply_outcome(Ok(outcome), cx);
                        files::hub(&client, cx).update(cx, |_, cx| {
                            cx.emit(FilesEvent::RootChanged {
                                subject: panel_subject,
                                origin,
                            });
                        });
                    }
                    Err(error) => {
                        this.error = Some(error.to_string().into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn apply_outcome(
        &mut self,
        outcome: Result<FilesRootOutcome, ClientError>,
        cx: &mut Context<Self>,
    ) {
        let old_removed_other: Option<String> = match &self.body {
            Body::Root(old)
                if is_viewing_other_checkout(&old.root)
                    && !runner_core::app_paths::is_existing_dir(&old.root.path) =>
            {
                Some(
                    old.commits
                        .as_ref()
                        .and_then(|c| c.branch.clone())
                        .or_else(|| old.root.git.as_ref().and_then(|g| g.branch.clone()))
                        .unwrap_or_else(|| old.root.name.clone()),
                )
            }
            _ => None,
        };
        let same = matches!(
            (&outcome, &self.body),
            (Ok(FilesRootOutcome::Root(root)), Body::Root(state))
                if root.path == state.root.path && root.git.is_some()
        );
        if !same {
            self.generation += 1;
            self.base_picker = None;
            self.checkout_picker = None;
        }
        let sync_outcome = outcome.as_ref().ok().cloned();
        self.error = None;
        match outcome {
            Ok(FilesRootOutcome::Root(root)) => {
                if is_viewing_other_checkout(&root) {
                    self.removed_checkout_notice = None;
                } else if let Some(removed) =
                    root.removed_checkout.clone().or(old_removed_other.clone())
                {
                    self.removed_checkout_notice = Some(removed);
                    if old_removed_other.is_some() {
                        let panel_subject = self.context.subject.clone();
                        let origin = cx.entity_id();
                        let client = self.client.clone();
                        files::hub(&client, cx).update(cx, |_, cx| {
                            cx.emit(FilesEvent::RootChanged {
                                subject: panel_subject,
                                origin,
                            });
                        });
                    }
                }
                if root.git.is_none() {
                    self.pending_restore = None;
                    self.body = Body::NotInGit;
                    self.notify_owner(cx);
                } else if let Body::Root(state) = &mut self.body
                    && same
                {
                    state.root = root;
                    if self.shown {
                        self.load_snapshot_inner(true, true, false, cx);
                    }
                } else {
                    self.files_scroll = UniformListScrollHandle::new();
                    let entity_id = cx.entity_id();
                    let base = self.files_scroll.0.borrow().base_handle.clone();
                    self.files_scrollbar = cx.new(|_| Scrollbar::app(base, entity_id));
                    self.commits_scroll = ScrollHandle::new();
                    let chosen_base = root.git.as_ref().and_then(|git| {
                        files::settings(cx)?
                            .changes_base(&git.top, git.branch.as_deref())
                            .map(str::to_owned)
                    });
                    self.body = Body::Root(Box::new(RootChanges {
                        root,
                        commits: None,
                        chosen_base,
                        missing_base_notice: None,
                        selected_target: SelectedTarget::Uncommitted,
                        initial_target_chosen: false,
                        uncommitted_count: 0,
                        branch_count: 0,
                        files: Vec::new(),
                        totals: ChangesTotals::default(),
                        collapsed_folders: BTreeSet::new(),
                        list_rows: Vec::new(),
                        selected_path: None,
                        highlighted_list_row: 0,
                        snapshot_seq: 0,
                        files_seq: 0,
                    }));
                    if let Body::Root(state) = &self.body
                        && self.pending_restore.as_ref().is_some_and(|pending| {
                            !picker::same_checkout_path(&state.root.path, &pending.root)
                        })
                    {
                        self.pending_restore = None;
                    }
                    self.notify_owner(cx);
                    if self.shown {
                        self.load_snapshot_inner(true, true, false, cx);
                    }
                }
            }
            Ok(FilesRootOutcome::ChooseFolder) => {
                self.pending_restore = None;
                self.body = Body::Choose;
                self.load_recent(cx);
                self.notify_owner(cx);
            }
            Ok(FilesRootOutcome::Missing { path }) => {
                self.pending_restore = None;
                self.body = Body::Missing { path };
                self.notify_owner(cx);
            }
            Err(error) => {
                self.pending_restore = None;
                self.body = Body::Failed(error.to_string().into());
                self.notify_owner(cx);
            }
        }
        if let Some(root_outcome) = sync_outcome {
            self.viewer
                .update(cx, |viewer, cx| viewer.sync_root(root_outcome, cx));
        }
        self.watched_root_changed(cx);
        cx.notify();
    }

    pub(super) fn show_choose_screen(&mut self, cx: &mut Context<Self>) {
        self.body = Body::Choose;
        self.load_recent(cx);
        self.watched_root_changed(cx);
        cx.notify();
    }

    fn load_recent(&mut self, cx: &mut Context<Self>) {
        self.request(
            cx,
            |client| client.files_recent_folders(),
            |this, result, cx| {
                if let Ok(recent) = result {
                    this.recent = recent;
                    cx.notify();
                }
            },
        );
    }

    pub(super) fn apply_typed_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.choose_field.read(cx).text().to_owned();
        match picker::typed_path(&text, Self::home(cx).as_deref()) {
            Some(path) => {
                self.set_root(Some(path), cx);
                self.list_focus.focus(window, cx);
            }
            None => {
                self.error = Some(crate::i18n::text("ui.type_a_full_path").into());
                cx.notify();
            }
        }
    }

    pub(super) fn browse(&mut self, cx: &mut Context<Self>) {
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(crate::i18n::text("ui.choose_folder").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = selected.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |this, cx| this.set_root(Some(path), cx));
        })
        .detach();
    }

    // ── Watch & live events ────────────────────────────────────────────

    fn watched_root_changed(&mut self, cx: &mut Context<Self>) {
        if self.watched.as_deref() != self.watched_git_root() {
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

    pub(super) fn sync_watch(&mut self, cx: &mut Context<Self>) {
        let wanted = self
            .shown
            .then(|| self.watched_git_root().map(Path::to_path_buf))
            .flatten();
        if let Some(root) = &wanted
            && !runner_core::app_paths::is_existing_dir(root)
        {
            self.unwatch(cx);
            self.resolve(cx);
            return;
        }
        if self.watched == wanted {
            return;
        }
        self.unwatch(cx);
        let Some(root) = wanted else {
            return;
        };
        let has_root = matches!(&self.body, Body::Root(_));
        self.watched = Some(root.clone());
        self.chain_watch(cx, move |client| {
            let _ = client.files_watch(&root);
        });
        if has_root {
            self.load_snapshot_inner(true, true, false, cx);
        }
    }

    pub(super) fn on_files_event(&mut self, event: &FilesEvent, cx: &mut Context<Self>) {
        match event {
            FilesEvent::Changed(changed) => {
                if self.shown
                    && self
                        .watched_git_root()
                        .is_some_and(|watched| picker::same_checkout_path(watched, &changed.root))
                {
                    self.load_snapshot(cx);
                }
            }
            FilesEvent::Status { root, .. } => {
                if self.shown
                    && self
                        .watched_git_root()
                        .is_some_and(|watched| picker::same_checkout_path(watched, root))
                {
                    self.load_snapshot(cx);
                }
            }
            FilesEvent::Viewed(viewed) => {
                self.apply_viewed_event(viewed, cx);
            }
            FilesEvent::RootChanged { subject, origin } => {
                if &self.context.subject == subject && cx.entity_id() != *origin {
                    self.resolve(cx);
                }
            }
            FilesEvent::Resync { rewatch } => {
                if self.context.remote {
                    return;
                }
                if !matches!(self.body, Body::Root(_)) {
                    self.resolve(cx);
                    return;
                }
                if !self.shown {
                    return;
                }
                if *rewatch {
                    self.watched = None;
                    self.sync_watch(cx);
                }
                self.load_snapshot(cx);
            }
        }
    }

    // ── Snapshot & diff loading ────────────────────────────────────────

    pub(super) fn recover_from_root_error(&mut self, msg: &str, cx: &mut Context<Self>) -> bool {
        let root_missing = matches!(
            &self.body,
            Body::Root(state) if !runner_core::app_paths::is_existing_dir(&state.root.path)
        ) || msg.contains("is not an existing folder");
        if root_missing {
            self.unwatch(cx);
            self.resolve(cx);
            return true;
        }
        if msg.contains("not inside a git checkout") || msg.contains("git is not on PATH") {
            self.body = Body::NotInGit;
            self.watched_root_changed(cx);
            cx.notify();
            return true;
        }
        false
    }
}
