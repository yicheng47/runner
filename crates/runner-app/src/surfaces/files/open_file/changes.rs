use super::*;

impl UnifiedViewer {
    // ── Opening, Switching, Pinning & Closing Tabs ─────────────────────

    pub(crate) fn open_pinned_file(
        &mut self,
        path: String,
        line: Option<u32>,
        cx: &mut Context<Self>,
    ) {
        let outcome = self.tabs.open_pinned(&path);
        let key = outcome.key().clone();
        self.ensure_uncommitted_paths(cx);
        let source = !is_svg(&key.path)
            && super::settings(cx)
                .is_some_and(|settings| settings.markdown_open_as == MarkdownOpenAs::Source);
        let has_head = self.root_has_head();
        let need_read = if outcome.is_new() || !self.docs.contains_key(&key) {
            self.docs
                .insert(key.clone(), OpenDocument::new_code(source, has_head));
            true
        } else if let Some(doc) = self.docs.get_mut(&key) {
            doc.mode = DocMode::Code;
            matches!(doc.content, Content::Idle)
        } else {
            false
        };
        if let Some(line) = line
            && let Some(doc) = self.docs.get_mut(&key)
        {
            doc.line = Some(line);
            doc.mode = DocMode::Code;
            if is_markdown(&key.path) || is_svg(&key.path) {
                doc.source = true;
            }
        }
        self.note_ignored_tabs();
        if need_read {
            self.read_file(key.clone(), cx);
        } else if line.is_some() {
            self.show_line(&key, cx);
        }
        if self.has_uncommitted_diff(&key.path) == Some(true) {
            self.load_code_hunks(key.clone(), cx);
        }
        if self.shown && self.ignored_refresh.is_none() {
            self.ignored_refresh = Some(self.refresh_ignored_tabs(cx));
        }
        self.sync_watch(cx);
        self.tab_shown(cx);
    }

    pub(crate) fn open_changes_file(
        &mut self,
        req: OpenChangesRequest,
        pin: bool,
        cx: &mut Context<Self>,
    ) {
        // Ensure our root matches the Changes rail before opening.
        let same_root = self
            .root
            .as_ref()
            .is_some_and(|r| runner_core::app_paths::same_checkout_path(&r.path, &req.root.path));
        if !same_root {
            self.apply_root_outcome(FilesRootOutcome::Root(req.root.clone()), cx);
        } else {
            self.root = Some(req.root.clone());
        }
        let outcome = self.tabs.open_preview(&req.path, req.target.clone());
        if pin && let Some(active_idx) = self.tabs.active_index() {
            self.tabs.pin(active_idx);
        }
        let source = !is_svg(&req.path)
            && super::settings(cx)
                .is_some_and(|settings| settings.markdown_open_as == MarkdownOpenAs::Source);
        let key = outcome.key().clone();
        if req.target == SelectedTarget::Uncommitted
            && let Some(files) = &mut self.uncommitted_files
        {
            files.insert(req.path.clone(), req.file.clone());
        }
        if req.target == SelectedTarget::Branch {
            self.non_uncommitted_seq += 1;
        }
        let need_diff = match outcome {
            OpenPreviewOutcome::ReplacedPreview { replaced, .. } => {
                if self.tabs.position_key(&replaced).is_none()
                    && let Some(old_doc) = self.docs.remove(&replaced)
                {
                    old_doc.drop_image_textures(None, cx);
                }
                self.docs
                    .insert(key.clone(), OpenDocument::new_diff(&req, source));
                true
            }
            OpenPreviewOutcome::CreatedPreview(_) => {
                self.docs
                    .insert(key.clone(), OpenDocument::new_diff(&req, source));
                true
            }
            OpenPreviewOutcome::FocusedPinned(_) | OpenPreviewOutcome::AlreadyPreview(_) => {
                if let Some(doc) = self.docs.get_mut(&key) {
                    let prev_base = doc.expected_base(&key).map(str::to_owned);
                    let prev_old = doc.hint_old_blob_id.clone();
                    let prev_new = doc.hint_new_blob_id.clone();
                    let expected_base = matches!(key.target, SelectedTarget::Branch)
                        .then_some(req.chosen_base.as_deref())
                        .flatten();
                    let base_changed = matches!(key.target, SelectedTarget::Branch)
                        && prev_base.as_deref() != expected_base;
                    let blobs_changed =
                        prev_old != req.file.old_blob_id || prev_new != req.file.new_blob_id;
                    let matches_loaded = matches!(
                        &doc.diff,
                        FileTabDiff::Loaded(loaded)
                            if loaded.target == key.target
                                && loaded.base.as_deref() == expected_base
                                && loaded.old_blob_id == req.file.old_blob_id
                                && loaded.new_blob_id == req.file.new_blob_id
                    );
                    doc.apply_changes_hints(&req);
                    doc.mode = DocMode::Diff;
                    if let FileTabDiff::Loaded(loaded) = &mut doc.diff
                        && matches_loaded
                    {
                        loaded.set_viewed_state(req.file.viewed, req.file.changed_since_viewed, cx);
                        false
                    } else {
                        if base_changed {
                            doc.diff = FileTabDiff::Loading;
                            doc.hunk_lines.clear();
                            doc.hunk_blobs = None;
                        }
                        !doc.diff_reading
                            || base_changed
                            || blobs_changed
                            || matches!(
                                doc.diff,
                                FileTabDiff::Idle | FileTabDiff::NoChanges | FileTabDiff::Failed(_)
                            )
                    }
                } else {
                    self.docs
                        .insert(key.clone(), OpenDocument::new_diff(&req, source));
                    true
                }
            }
        };
        if need_diff {
            self.load_tab_diff(key, cx);
        }
        if self.shown && self.ignored_refresh.is_none() {
            self.ignored_refresh = Some(self.refresh_ignored_tabs(cx));
        }
        self.sync_watch(cx);
        self.tab_shown(cx);
    }

    /// Keeps any open `All changes` (`SelectedTarget::Branch`) tabs in sync when the
    /// Changes rail changes base — including while in `Code` mode, while viewing another
    /// target, or when a base change collapses `All changes` down to a single commit.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sync_branch_base(
        &mut self,
        root: &Path,
        chosen_base: Option<&str>,
        resolved_base: Option<&str>,
        branch_name: Option<&str>,
        has_head: bool,
        reload_now: bool,
        cx: &mut Context<Self>,
    ) {
        if self
            .root_path()
            .is_none_or(|r| !runner_core::app_paths::same_checkout_path(r, root))
        {
            return;
        }
        // Supersede any in-flight status-triggered branch refresh that captured an older base.
        self.non_uncommitted_seq += 1;
        let branch_keys: Vec<TabKey> = self
            .tabs
            .tabs()
            .iter()
            .filter(|t| t.key.target == SelectedTarget::Branch)
            .map(|t| t.key.clone())
            .collect();
        let mut to_reload = Vec::new();
        let mut to_load_hunks = Vec::new();
        let mut changed_any = false;
        for key in branch_keys {
            let Some(doc) = self.docs.get_mut(&key) else {
                continue;
            };
            let prev_base = doc.expected_base(&key).map(str::to_owned);
            let base_changed = prev_base.as_deref() != chosen_base
                || (chosen_base.is_none()
                    && resolved_base.is_some()
                    && doc.resolved_base.as_deref() != resolved_base);
            let pending_diff = doc.mode == DocMode::Diff
                && !doc.diff_reading
                && matches!(doc.diff, FileTabDiff::Loading | FileTabDiff::Idle);
            doc.diff_base = chosen_base.map(str::to_owned);
            if resolved_base.is_some() {
                doc.resolved_base = resolved_base.map(str::to_owned);
            }
            doc.branch_name = branch_name.map(str::to_owned);
            doc.branch_known = true;
            doc.has_head = has_head;
            if base_changed {
                changed_any = true;
                doc.hint_status = None;
                doc.hint_counts = None;
                doc.hint_viewed = None;
                doc.hint_old_blob_id = None;
                doc.hint_new_blob_id = None;
                doc.viewed_stale = false;
                doc.hunk_lines.clear();
                doc.hunk_blobs = None;
                doc.hunk_read += 1;
                doc.diff_reading = false;
                doc.diff_read += 1;
                if doc.mode == DocMode::Diff {
                    doc.diff = FileTabDiff::Loading;
                    if reload_now {
                        to_reload.push(key);
                    }
                } else {
                    doc.diff = FileTabDiff::Idle;
                    if reload_now {
                        to_load_hunks.push(key);
                    }
                }
            } else if reload_now && pending_diff {
                // Complete an invalidation queued by `pick_base` when the follow-up snapshot
                // collapsed `All changes` (e.g. 0 commits ahead -> Uncommitted).
                to_reload.push(key);
            }
        }
        for key in to_reload {
            self.load_tab_diff(key, cx);
        }
        for key in to_load_hunks {
            self.load_code_hunks(key, cx);
        }
        if changed_any {
            cx.notify();
        }
    }

    /// Synchronizes open tabs with a refreshed file list from the Changes rail:
    /// updates `viewed` / `changed_since_viewed` / base metadata, reloads diffs
    /// whose blobs or base changed, and reconciles a live preview tab when its
    /// file disappears while `Changes` is active.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reconcile_changes_files(
        &mut self,
        root: &Path,
        target: &SelectedTarget,
        chosen_base: Option<&str>,
        resolved_base: Option<&str>,
        branch_name: Option<&str>,
        has_head: bool,
        files: &[ChangedFile],
        list_rows: &[ListRow],
        tree_mode: bool,
        reload_same_diff: bool,
        allow_preview_replace: bool,
        cx: &mut Context<Self>,
    ) {
        if self
            .root_path()
            .is_none_or(|r| !runner_core::app_paths::same_checkout_path(r, root))
        {
            return;
        }
        let expected_base = matches!(target, SelectedTarget::Branch)
            .then_some(chosen_base)
            .flatten();
        if *target == SelectedTarget::Uncommitted {
            self.uncommitted_seq += 1;
            self.uncommitted_files = Some(
                files
                    .iter()
                    .map(|entry| (entry.path.clone(), entry.clone()))
                    .collect(),
            );
        } else if *target == SelectedTarget::Branch {
            // Supersede any in-flight status-triggered branch refresh that captured an older scope.
            self.non_uncommitted_seq += 1;
        }

        // 1. Only reconcile a live Changes preview tab's disappearance while Changes
        //    is already active — switching rails to Changes must never close or replace
        //    an open viewer document, and an empty Changes list must never collapse the
        //    active tab away without an explicit close action.
        if allow_preview_replace
            && self.changes_rail_shown
            && let Some(preview_idx) = self.tabs.preview_index()
        {
            let preview_key = self.tabs.tabs()[preview_idx].key.clone();
            if &preview_key.target == target && !files.iter().any(|f| f.path == preview_key.path) {
                let preview_was_active = self.tabs.active_index() == Some(preview_idx);
                if !preview_was_active {
                    // Never yank focus away from another tab the user is actively reading.
                    if let Some(closed) = self.tabs.close(preview_idx)
                        && let Some(old_doc) = self.docs.remove(&closed)
                    {
                        old_doc.drop_image_textures(None, cx);
                    }
                } else {
                    let replacement = reconcile_file_selection(
                        Some(&preview_key.path),
                        files,
                        list_rows,
                        tree_mode,
                    );
                    if let Some(next_path) = replacement
                        && let Some(next_file) = files.iter().find(|f| f.path == next_path)
                        && let Some(root_obj) = self.root.clone()
                    {
                        let next_key = TabKey::new(&next_path, target.clone());
                        // If the replacement is already pinned, remove the orphaned preview first
                        // so `open_changes_file` focuses the pinned tab cleanly with no orphan left.
                        if self.tabs.position_key(&next_key).is_some()
                            && let Some(closed) = self.tabs.close(preview_idx)
                            && let Some(old_doc) = self.docs.remove(&closed)
                        {
                            old_doc.drop_image_textures(None, cx);
                        }
                        let req = OpenChangesRequest {
                            root: root_obj,
                            path: next_path,
                            target: target.clone(),
                            chosen_base: chosen_base.map(str::to_owned),
                            resolved_base: resolved_base.map(str::to_owned),
                            branch_name: branch_name.map(str::to_owned),
                            commit_short_id: match target {
                                SelectedTarget::Commit(id) => {
                                    Some(id[..id.len().min(7)].to_owned())
                                }
                                _ => None,
                            },
                            file: next_file.clone(),
                            has_head,
                        };
                        self.open_changes_file(req, false, cx);
                    }
                }
            }
        }

        // 2. Sync existing open tabs matching this target.
        let matching_keys: Vec<TabKey> = self
            .tabs
            .tabs()
            .iter()
            .filter(|t| &t.key.target == target)
            .map(|t| t.key.clone())
            .collect();
        let mut to_reload = Vec::new();
        let mut to_read = Vec::new();
        let mut to_load_hunks = Vec::new();
        for key in matching_keys {
            if let Some(file) = files.iter().find(|f| f.path == key.path) {
                if let Some(doc) = self.docs.get_mut(&key) {
                    let prev_base = doc.expected_base(&key).map(str::to_owned);
                    let prev_old = doc.hint_old_blob_id.clone();
                    let prev_new = doc.hint_new_blob_id.clone();
                    let prev_viewed = doc.hint_viewed;
                    let base_changed = matches!(key.target, SelectedTarget::Branch)
                        && prev_base.as_deref() != expected_base;
                    let blobs_changed =
                        prev_old != file.old_blob_id || prev_new != file.new_blob_id;
                    let next_viewed = (file.viewed, file.changed_since_viewed);
                    doc.diff_base = chosen_base.map(str::to_owned);
                    doc.resolved_base = resolved_base.map(str::to_owned);
                    doc.branch_name = branch_name.map(str::to_owned);
                    doc.branch_known = true;
                    doc.has_head = has_head;
                    doc.hint_status = Some(file.status.clone());
                    doc.hint_counts = Some(file.counts);
                    doc.hint_viewed = Some(next_viewed);
                    doc.hint_old_blob_id = file.old_blob_id.clone();
                    doc.hint_new_blob_id = file.new_blob_id.clone();
                    doc.viewed_stale = false;
                    if blobs_changed || prev_viewed != Some(next_viewed) {
                        self.viewed_revision += 1;
                        doc.latest_viewed = Some((
                            self.viewed_revision,
                            file.old_blob_id.clone(),
                            file.new_blob_id.clone(),
                            file.viewed,
                            file.changed_since_viewed,
                        ));
                    }
                    if *target == SelectedTarget::Uncommitted {
                        doc.working_tree_missing = Some(file.status == FileStatus::Deleted);
                    }
                    let matches_loaded = match &mut doc.diff {
                        FileTabDiff::Loaded(loaded) => {
                            let same = loaded.target == key.target
                                && loaded.base.as_deref() == expected_base
                                && loaded.old_blob_id == file.old_blob_id
                                && loaded.new_blob_id == file.new_blob_id;
                            if same {
                                loaded.set_viewed_state(file.viewed, file.changed_since_viewed, cx);
                            }
                            same
                        }
                        _ => false,
                    };
                    if !matches_loaded {
                        if doc.mode == DocMode::Diff {
                            if base_changed {
                                doc.diff = FileTabDiff::Loading;
                                doc.hunk_lines.clear();
                                doc.hunk_blobs = None;
                            }
                            if !doc.diff_reading
                                || base_changed
                                || blobs_changed
                                || reload_same_diff
                            {
                                to_reload.push(key);
                            }
                        } else {
                            // Invalidate stale loaded/in-flight diff while in Code mode so
                            // switching back to Diff reloads against the new base/blobs.
                            doc.diff = FileTabDiff::Idle;
                            doc.diff_reading = false;
                            doc.diff_read += 1;
                            if base_changed || blobs_changed || doc.hunk_lines.is_empty() {
                                doc.hunk_lines.clear();
                                doc.hunk_blobs = None;
                                to_load_hunks.push(key);
                            }
                        }
                    } else if doc.mode == DocMode::Diff && reload_same_diff {
                        to_reload.push(key);
                    }
                }
            } else if *target == SelectedTarget::Uncommitted
                && let Some(doc) = self.docs.get_mut(&key)
            {
                // Uncommitted changes cleared on a working-tree tab: return to clean Code
                // and promote out of the Changes diff-preview slot so future Changes
                // updates do not treat this open Code document as an orphaned diff preview.
                let was_diff = doc.mode == DocMode::Diff;
                doc.mode = DocMode::Code;
                doc.clear_uncommitted_diff_hints();
                if was_diff || matches!(doc.content, Content::Idle) {
                    to_read.push(key.clone());
                }
                if let Some(idx) = self.tabs.position_key(&key) {
                    self.tabs.pin(idx);
                }
            } else if *target == SelectedTarget::Branch
                && let Some(doc) = self.docs.get_mut(&key)
            {
                // Under the newly chosen base this file is no longer changed in `All changes`:
                // settle cleanly to `NoChanges` in Diff mode (or Idle in Code) with no stale hunks.
                doc.diff_base = chosen_base.map(str::to_owned);
                doc.resolved_base = resolved_base.map(str::to_owned);
                doc.branch_name = branch_name.map(str::to_owned);
                doc.branch_known = true;
                doc.has_head = has_head;
                doc.hint_status = None;
                doc.hint_counts = None;
                doc.hint_viewed = None;
                doc.hint_old_blob_id = None;
                doc.hint_new_blob_id = None;
                doc.viewed_stale = false;
                doc.hunk_lines.clear();
                doc.hunk_blobs = None;
                doc.hunk_read += 1;
                doc.diff = if doc.mode == DocMode::Diff {
                    FileTabDiff::NoChanges
                } else {
                    FileTabDiff::Idle
                };
                doc.diff_reading = false;
                doc.diff_read += 1;
            }
        }
        for key in to_read {
            self.read_file(key, cx);
        }
        for key in to_reload {
            self.load_tab_diff(key, cx);
        }
        for key in to_load_hunks {
            self.load_code_hunks(key, cx);
        }
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_changes_viewed_optimistic(
        &mut self,
        root: &Path,
        target: &SelectedTarget,
        path: &str,
        old_blob: Option<&str>,
        new_blob: Option<&str>,
        viewed: bool,
        changed_since: bool,
        cx: &mut Context<Self>,
    ) {
        if self
            .root_path()
            .is_none_or(|r| !runner_core::app_paths::same_checkout_path(r, root))
        {
            return;
        }
        self.viewed_revision += 1;
        let rev = self.viewed_revision;
        let mut updated = false;
        for (key, doc) in &mut self.docs {
            if key.path != path {
                continue;
            }
            if &key.target == target
                || (doc.hint_old_blob_id.as_deref() == old_blob
                    && doc.hint_new_blob_id.as_deref() == new_blob)
            {
                doc.latest_viewed = Some((
                    rev,
                    old_blob.map(str::to_owned),
                    new_blob.map(str::to_owned),
                    viewed,
                    changed_since,
                ));
            }
            if &key.target == target
                && doc.hint_old_blob_id.as_deref() == old_blob
                && doc.hint_new_blob_id.as_deref() == new_blob
            {
                doc.hint_viewed = Some((viewed, changed_since));
            }
            if let FileTabDiff::Loaded(loaded) = &mut doc.diff
                && loaded.old_blob_id.as_deref() == old_blob
                && loaded.new_blob_id.as_deref() == new_blob
            {
                loaded.set_viewed_state(viewed, changed_since, cx);
                updated = true;
            }
        }
        if updated {
            cx.notify();
        }
    }
}
