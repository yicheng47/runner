use super::*;

impl UnifiedViewer {
    // ── Root & Watch Management ────────────────────────────────────────

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

    pub(crate) fn resolve(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        if self.context.remote {
            self.root = None;
            self.reset_tabs(None, cx);
            self.sync_watch(cx);
            cx.notify();
            return;
        }
        let subject = subject_protocol(&self.context);
        #[cfg(test)]
        if let Some(gate) = self.resolve_gate.take() {
            let client = self.client.clone();
            let generation = self.generation;
            let task = cx.background_spawn(async move {
                let _ = gate.await;
                client.files_root(&subject)
            });
            cx.spawn(async move |this, cx| {
                let result = task.await;
                let _ = this.update(cx, |this, cx| {
                    if this.generation == generation
                        && let Ok(outcome) = result
                    {
                        this.apply_root_outcome(outcome, cx);
                    }
                });
            })
            .detach();
            return;
        }
        self.request(
            cx,
            move |client| client.files_root(&subject),
            |this, result, cx| {
                if let Ok(outcome) = result {
                    this.apply_root_outcome(outcome, cx);
                }
            },
        );
    }

    pub(crate) fn sync_root(&mut self, outcome: FilesRootOutcome, cx: &mut Context<Self>) {
        match (&outcome, &self.root) {
            (FilesRootOutcome::Root(next), Some(current))
                if runner_core::app_paths::same_checkout_path(&current.path, &next.path) =>
            {
                if current != next {
                    self.root = Some(next.clone());
                    cx.notify();
                }
                return;
            }
            (FilesRootOutcome::ChooseFolder | FilesRootOutcome::Missing { .. }, None)
                if self.pending_restore.is_none()
                    && self.tabs.tabs().iter().all(|tab| tab.key.is_outside()) =>
            {
                return;
            }
            _ => {}
        }
        self.apply_root_outcome(outcome, cx);
    }

    pub(super) fn apply_root_outcome(&mut self, outcome: FilesRootOutcome, cx: &mut Context<Self>) {
        let same = match (&outcome, &self.root) {
            (FilesRootOutcome::Root(next), Some(current)) => {
                runner_core::app_paths::same_checkout_path(&current.path, &next.path)
            }
            (FilesRootOutcome::ChooseFolder | FilesRootOutcome::Missing { .. }, None) => true,
            _ => false,
        };
        if !same {
            self.generation += 1;
        }
        match outcome {
            FilesRootOutcome::Root(next_root) => {
                let next_root_path = next_root.path.clone();
                if same {
                    self.root = Some(next_root);
                    self.read_again_after_resolve(cx);
                    self.refresh_uncommitted_paths(true, cx);
                    self.reconcile_outside_tabs_for_root(next_root_path, cx);
                } else {
                    self.reset_root_tabs(None, cx);
                    self.status = None;
                    self.status_stale = true;
                    self.uncommitted_files = None;
                    self.root = Some(next_root);
                    if let Some(saved) = self.pending_restore.take() {
                        self.apply_saved_state(saved, cx);
                    }
                    self.read_again_after_resolve(cx);
                    self.reconcile_outside_tabs_for_root(next_root_path, cx);
                    self.notify_owner(cx);
                }
            }
            FilesRootOutcome::ChooseFolder | FilesRootOutcome::Missing { .. } => {
                self.root = None;
                self.status = None;
                self.uncommitted_files = None;
                self.reset_root_tabs(None, cx);
                if let Some(saved) = self.pending_restore.take() {
                    self.apply_saved_state(saved, cx);
                }
                self.read_again_after_resolve(cx);
                self.notify_owner(cx);
            }
        }
        self.watched_root_changed(cx);
        cx.notify();
    }

    fn reconcile_outside_tabs_for_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        let outside: Vec<(TabKey, PathBuf)> = self
            .tabs
            .tabs()
            .iter()
            .filter_map(|tab| {
                tab.key
                    .outside_path()
                    .map(|path| (tab.key.clone(), path.to_path_buf()))
            })
            .collect();
        if outside.is_empty() {
            return;
        }
        let generation = self.generation;
        let check_root = root.clone();
        let task = cx.background_spawn(async move {
            outside
                .into_iter()
                .filter_map(|(key, path)| {
                    super::super::links::relative_in_canonical(&check_root, &path)
                        .map(|relative| (key, relative))
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let promotions = task.await;
            if promotions.is_empty() {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation
                    || this.root_path().is_none_or(|current| {
                        !runner_core::app_paths::same_checkout_path(current, &root)
                    })
                {
                    return;
                }
                this.promote_outside_tabs(promotions, cx);
            });
        })
        .detach();
    }

    fn promote_outside_tabs(&mut self, promotions: Vec<(TabKey, String)>, cx: &mut Context<Self>) {
        use super::super::tabs::PromoteOutsideOutcome;

        let has_head = self.root_has_head();
        let markdown_source = super::settings(cx)
            .is_some_and(|settings| settings.markdown_open_as == MarkdownOpenAs::Source);
        let mut promoted_any = false;
        let mut keys_to_read = Vec::new();
        for (old_key, relative) in promotions {
            match self.tabs.promote_outside(&old_key, relative) {
                Some(PromoteOutsideOutcome::MergedIntoExisting(new_key)) => {
                    promoted_any = true;
                    if let Some(old_doc) = self.docs.remove(&old_key) {
                        old_doc.drop_image_textures(None, cx);
                    }
                    if !self.docs.contains_key(&new_key) {
                        let source = !is_svg(&new_key.path) && markdown_source;
                        self.docs
                            .insert(new_key.clone(), OpenDocument::new_code(source, has_head));
                        keys_to_read.push(new_key);
                    }
                }
                Some(PromoteOutsideOutcome::Replaced(new_key)) => {
                    promoted_any = true;
                    if let Some(mut doc) = self.docs.remove(&old_key) {
                        doc.has_head = has_head;
                        doc.ignored = false;
                        doc.stamp = None;
                        doc.image_hash = None;
                        doc.rendered = None;
                        let open_line = self.open_line_handler(new_key.clone(), cx);
                        match &doc.content {
                            Content::Text(view)
                            | Content::Deleted(Some(DeletedContent::Text(view))) => {
                                view.update(cx, |view, _| view.set_open_line(open_line));
                            }
                            _ => {}
                        }
                        self.docs.insert(new_key.clone(), doc);
                    } else {
                        let source = !is_svg(&new_key.path) && markdown_source;
                        self.docs
                            .insert(new_key.clone(), OpenDocument::new_code(source, has_head));
                    }
                    keys_to_read.push(new_key);
                }
                None => {}
            }
        }
        if !promoted_any {
            return;
        }
        self.note_ignored_tabs();
        for key in keys_to_read {
            self.read_file(key, cx);
        }
        self.refresh_uncommitted_paths(true, cx);
        self.sync_watch(cx);
        self.tab_shown(cx);
    }

    pub(crate) fn set_status_snapshot(
        &mut self,
        root: &Path,
        status: Arc<PreparedStatus>,
        cx: &mut Context<Self>,
    ) {
        if self
            .root_path()
            .is_some_and(|r| runner_core::app_paths::same_checkout_path(r, root))
        {
            self.apply_status(status, cx);
        }
    }

    pub(crate) fn set_shown(
        &mut self,
        shown: bool,
        rail_watches: bool,
        changes_rail_shown: bool,
        cx: &mut Context<Self>,
    ) {
        let was_shown = self.shown;
        if self.shown == shown
            && self.rail_watches == rail_watches
            && self.changes_rail_shown == changes_rail_shown
        {
            return;
        }
        self.shown = shown;
        self.rail_watches = rail_watches;
        self.changes_rail_shown = changes_rail_shown;
        if !shown {
            self.list_open = false;
            self.open_in_open = false;
            self.status_stale = true;
            self.ignored_refresh = None;
            for doc in self.docs.values() {
                doc.drop_image_textures(None, cx);
            }
        } else {
            if self.ignored_refresh.is_none() && self.has_tabs() {
                self.ignored_refresh = Some(self.refresh_ignored_tabs(cx));
            }
            if !was_shown && self.has_tabs() {
                for doc in self.docs.values_mut() {
                    doc.viewed_stale = true;
                }
                if rail_watches
                    && changes_rail_shown
                    && self.status_stale
                    && let Some(root) = self.root_path().map(PathBuf::from)
                {
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
                self.reload_changed(None, cx);
                self.refresh_uncommitted_paths(true, cx);
                self.refresh_non_uncommitted_targets(false, cx);
            }
        }
        self.sync_watch(cx);
    }

    fn watched_root_changed(&mut self, cx: &mut Context<Self>) {
        let same = match (self.watched.as_deref(), self.root_path()) {
            (Some(a), Some(b)) => runner_core::app_paths::same_checkout_path(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
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
        let wanted = (self.shown && self.has_tabs() && !self.rail_watches)
            .then(|| self.root_path().map(Path::to_path_buf))
            .flatten();
        let same = match (self.watched.as_deref(), wanted.as_deref()) {
            (Some(a), Some(b)) => runner_core::app_paths::same_checkout_path(a, b),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        self.unwatch(cx);
        let Some(root) = wanted else {
            return;
        };
        self.watched = Some(root.clone());
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
            let Ok(Ok(status)) = status_rx.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation && this.watched.is_some() {
                    this.apply_status(status, cx);
                    this.notify_owner(cx);
                }
            });
        })
        .detach();
    }

    fn apply_status(&mut self, status: Arc<PreparedStatus>, cx: &mut Context<Self>) {
        if self
            .status
            .as_ref()
            .is_some_and(|cur| Arc::ptr_eq(cur, &status))
        {
            self.status_stale = false;
            return;
        }
        self.status = Some(status);
        self.status_stale = false;
        if self.has_tabs() {
            self.note_ignored_tabs();
            self.reconcile_diff_tabs(false, None, false, cx);
            self.refresh_uncommitted_paths(true, cx);
            self.refresh_non_uncommitted_targets(true, cx);
        }
        cx.notify();
    }

    pub(crate) fn on_files_event(&mut self, event: &FilesEvent, cx: &mut Context<Self>) {
        match event {
            FilesEvent::Changed(changed) => {
                if !self.shown || !self.has_tabs() {
                    return;
                }
                if self
                    .root_path()
                    .is_none_or(|r| !runner_core::app_paths::same_checkout_path(r, &changed.root))
                {
                    return;
                }
                self.reload_changed(Some(changed), cx);
                self.refresh_uncommitted_paths(false, cx);
            }
            FilesEvent::Status { root, status } => {
                if self.shown
                    && self
                        .root_path()
                        .is_some_and(|r| runner_core::app_paths::same_checkout_path(r, root))
                {
                    self.apply_status(status.clone(), cx);
                    if self.watched.is_some() {
                        self.notify_owner(cx);
                    }
                }
            }
            FilesEvent::Viewed(viewed) => {
                if self.shown {
                    self.apply_viewed_event(viewed, cx);
                }
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
                if self.root.is_none() {
                    self.resolve(cx);
                    if self.shown && self.has_tabs() {
                        self.reload_changed(None, cx);
                    }
                    return;
                }
                for doc in self.docs.values_mut() {
                    doc.viewed_stale = true;
                }
                if !self.shown {
                    return;
                }
                if *rewatch {
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
                self.reload_changed(None, cx);
                self.refresh_uncommitted_paths(true, cx);
                self.refresh_non_uncommitted_targets(false, cx);
            }
        }
    }

    // ── Settings & Resize ──────────────────────────────────────────────

    pub(super) fn settings_changed(&mut self, cx: &mut Context<Self>) {
        let diff_mode = super::settings(cx)
            .map_or(DiffViewMode::Unified, |settings| settings.changes_diff_mode);
        if diff_mode != self.diff_mode_pref {
            self.diff_mode_pref = diff_mode;
            self.sync_effective_diff_mode(cx);
            cx.notify();
        }
        let look = super::Look::current(cx);
        if look == self.look {
            return;
        }
        self.look = look;
        for doc in self.docs.values() {
            match &doc.content {
                Content::Text(view) | Content::Deleted(Some(DeletedContent::Text(view))) => {
                    view.update(cx, |_, cx| cx.notify());
                }
                Content::Image { canvas, .. }
                | Content::Deleted(Some(DeletedContent::Image { canvas, .. })) => {
                    canvas.update(cx, |_, cx| cx.notify());
                }
                _ => {}
            }
            if let Some(rendered) = &doc.rendered {
                rendered.update(cx, |_, cx| cx.notify());
            }
            if let Some(canvas) = &doc.svg_canvas {
                canvas.update(cx, |_, cx| cx.notify());
            }
            if let FileTabDiff::Loaded(loaded) = &doc.diff {
                match &loaded.kind {
                    LoadedDiffKind::Unified(diff) => diff.update(cx, |_, cx| cx.notify()),
                    LoadedDiffKind::Image { canvas, .. } => canvas.update(cx, |_, cx| cx.notify()),
                    LoadedDiffKind::Svg {
                        canvas, unified, ..
                    } => {
                        canvas.update(cx, |_, cx| cx.notify());
                        unified.update(cx, |_, cx| cx.notify());
                    }
                    _ => {}
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn diff_width(&self) -> f32 {
        self.widened
            .map_or(crate::app_settings::FILE_VIEWER_DEFAULT, |w| w.file)
    }

    pub(super) fn effective_diff_mode(&self, cx: &App) -> DiffViewMode {
        let preferred = super::settings(cx).map_or(self.diff_mode_pref, |s| s.changes_diff_mode);
        side_panel::effective_diff_mode(preferred, self.diff_width(), self.maximized)
    }

    pub(super) fn sync_effective_diff_mode(&self, cx: &mut Context<Self>) {
        let mode = self.effective_diff_mode(cx);
        for doc in self.docs.values() {
            if let FileTabDiff::Loaded(loaded) = &doc.diff {
                match &loaded.kind {
                    LoadedDiffKind::Unified(diff) => diff.update(cx, |v, cx| v.set_mode(mode, cx)),
                    LoadedDiffKind::Image { canvas, .. } => {
                        canvas.update(cx, |c, cx| c.set_diff_mode(mode, cx));
                    }
                    LoadedDiffKind::Svg {
                        canvas, unified, ..
                    } => {
                        canvas.update(cx, |c, cx| c.set_diff_mode(mode, cx));
                        unified.update(cx, |v, cx| v.set_mode(mode, cx));
                    }
                    _ => {}
                }
            }
        }
    }

    pub(super) fn select_diff_mode(
        &mut self,
        mode: DiffViewMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.diff_mode_pref = mode;
        crate::app_store::global_app_store(cx).update(cx, |store, cx| {
            store.update_settings(
                |settings| {
                    let changed = settings.changes_diff_mode != mode;
                    settings.changes_diff_mode = mode;
                    changed
                },
                true,
                cx,
            );
        });
        if !self.root_focus.contains_focused(window, cx) {
            self.focus_active(window, cx);
        }
        if mode == DiffViewMode::Split
            && !self.maximized
            && self.diff_width() < side_panel::SPLIT_MIN_DIFF_WIDTH
            && let Some(request) = self.on_request_maximize.clone()
        {
            self.maximized = true;
            request(true, window, cx);
        }
        self.sync_effective_diff_mode(cx);
        cx.notify();
    }

    /// Sizes the right rail `tree` wide from dragging the inner divider, trading
    /// width with the unified viewer so the total panel width stays constant across
    /// Files, Changes, and Sessions (#1421).
    pub(crate) fn resize_tree(&mut self, tree: f32, cx: &mut Context<Self>) {
        let Some(widened) = self.widened.filter(|w| w.tree) else {
            return;
        };
        self.tree_resizing = true;
        if self.maximized {
            let total = widened.file + widened.tree_width;
            let tree = crate::app_settings::clamp_file_tree_width(tree)
                .min((total - widened.standard).max(crate::app_settings::FILE_TREE_MIN));
            crate::app_store::global_app_store(cx).update(cx, |store, cx| {
                store.update_settings(
                    |settings| {
                        let changed = settings.file_tree_width != tree;
                        settings.file_tree_width = tree;
                        changed
                    },
                    false,
                    cx,
                );
            });
        } else {
            let (file, tree) = widened.resize_tree(tree);
            crate::app_store::global_app_store(cx).update(cx, |store, cx| {
                store.update_settings(
                    |settings| {
                        let changed =
                            settings.file_tree_width != tree || settings.file_viewer_width != file;
                        settings.file_tree_width = tree;
                        settings.file_viewer_width = file;
                        changed
                    },
                    false,
                    cx,
                );
            });
        }
        self.notify_owner(cx);
        cx.notify();
    }

    pub(crate) fn finish_tree_resize(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.tree_resizing) {
            crate::app_store::global_app_store(cx)
                .read(cx)
                .save_settings();
            cx.notify();
        }
    }
}
