use super::*;

impl UnifiedViewer {
    pub(super) fn reprobe_working_tree_exists(&mut self, key: TabKey, cx: &mut Context<Self>) {
        let Some(root) = self.root_path().map(PathBuf::from) else {
            return;
        };
        let relative = key.path.clone();
        self.request(
            cx,
            move |client| {
                Ok(matches!(
                    client.files_read(&root, &relative, false, None),
                    Ok(FileContent::Missing)
                ))
            },
            move |this, result, cx| {
                if let Ok(missing) = result
                    && let Some(doc) = this.docs.get_mut(&key)
                    && doc.working_tree_missing != Some(missing)
                {
                    doc.working_tree_missing = Some(missing);
                    cx.notify();
                }
            },
        );
    }

    // ── Uncommitted Paths & Diff Loading ───────────────────────────────

    pub(super) fn clear_uncommitted_path(&mut self, path: &str) {
        if let Some(files) = &mut self.uncommitted_files {
            files.remove(path);
        }
    }

    pub(super) fn ensure_uncommitted_paths(&mut self, cx: &mut Context<Self>) {
        if self.uncommitted_files.is_none() && self.root.is_some() {
            self.refresh_uncommitted_paths(false, cx);
        }
    }

    pub(crate) fn refresh_uncommitted_paths(
        &mut self,
        refresh_active_diffs: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.has_tabs() {
            self.uncommitted_files = None;
            self.uncommitted_seq += 1;
            return;
        }
        let Some(root) = &self.root else {
            return;
        };
        if root.git.is_none() {
            self.uncommitted_files = Some(HashMap::new());
            self.reconcile_diff_tabs(false, None, true, cx);
            return;
        }
        self.uncommitted_seq += 1;
        let seq = self.uncommitted_seq;
        let started_viewed_rev = self.viewed_revision;
        let root_path = root.path.clone();
        #[cfg(test)]
        let hold_rx = self.hold_uncommitted_lists.then(|| {
            let (tx, rx) = futures::channel::oneshot::channel();
            self.held_uncommitted_lists.push(tx);
            rx
        });
        let client = self.client.clone();
        let generation = self.generation;
        let task = cx.background_spawn(async move {
            client.changes_files(&root_path, &ChangesTarget::Uncommitted, None)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            #[cfg(test)]
            if let Some(rx) = hold_rx {
                let _ = rx.await;
            }
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.uncommitted_seq != seq {
                    return;
                }
                if let Ok(files) = result {
                    this.uncommitted_files = Some(
                        files
                            .files
                            .into_iter()
                            .map(|entry| (entry.path.clone(), entry))
                            .collect(),
                    );
                    this.reconcile_diff_tabs(
                        refresh_active_diffs,
                        Some(started_viewed_rev),
                        true,
                        cx,
                    );
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Refreshes open non-`Uncommitted` tabs (`All changes` / commit) against daemon:
    /// - `branch_only = true`: live `FilesEvent::Status` / `FilesEvent::Viewed` path. Resolves
    ///   the current branch and its saved/effective base via `changes_commits` + settings (matching
    ///   `ChangesView::load_snapshot_inner`) so branch checkouts and moving comparison refs update
    ///   branch metadata, comparison base, `Loaded <-> NoChanges`, and authoritative viewed flags
    ///   while `Files` or `Sessions` is active.
    /// - `branch_only = false`: reconnect (`FilesEvent::Resync`) or re-show after hidden
    ///   (also refreshes authoritative viewed marks across open `Commit` tabs).
    pub(super) fn refresh_non_uncommitted_targets(
        &mut self,
        branch_only: bool,
        cx: &mut Context<Self>,
    ) {
        struct BranchRefresh {
            branch_name: Option<String>,
            effective_base: Option<String>,
            resolved_base: Option<String>,
            has_head: bool,
            files: Option<Vec<ChangedFile>>,
        }

        let Some(root) = &self.root else {
            return;
        };
        if root.git.is_none() {
            return;
        }
        let has_branch_tabs = self.docs.keys().any(|k| k.target == SelectedTarget::Branch);
        let mut commit_targets: Vec<SelectedTarget> = Vec::new();
        if !branch_only {
            for key in self.docs.keys() {
                if matches!(key.target, SelectedTarget::Commit(_))
                    && !commit_targets.contains(&key.target)
                {
                    commit_targets.push(key.target.clone());
                }
            }
        }
        if !has_branch_tabs && commit_targets.is_empty() {
            return;
        }
        let settings_snapshot = super::settings(cx).cloned();
        let initial_base = self
            .docs
            .iter()
            .find(|(k, _)| k.target == SelectedTarget::Branch)
            .and_then(|(k, d)| d.expected_base(k).map(str::to_owned));
        self.non_uncommitted_seq += 1;
        let seq = self.non_uncommitted_seq;
        let started_viewed_rev = self.viewed_revision;
        let root_path = root.path.clone();
        #[cfg(test)]
        let hold_rx = self.hold_non_uncommitted_refreshes.then(|| {
            let (tx, rx) = futures::channel::oneshot::channel();
            self.held_non_uncommitted_refreshes.push(tx);
            rx
        });
        let client = self.client.clone();
        let generation = self.generation;
        let task = cx.background_spawn(async move {
            let branch_refresh = if has_branch_tabs {
                client
                    .changes_commits(&root_path, initial_base.as_deref())
                    .ok()
                    .map(|mut commits| {
                        let wanted_base = settings_snapshot
                            .as_ref()
                            .and_then(|s| {
                                s.changes_base(&commits.checkout_top, commits.branch.as_deref())
                            })
                            .map(str::to_owned);
                        if wanted_base != initial_base
                            && let Ok(refreshed) =
                                client.changes_commits(&root_path, wanted_base.as_deref())
                        {
                            commits = refreshed;
                        }
                        let effective_base = (!commits.base_missing)
                            .then_some(wanted_base.as_deref())
                            .flatten()
                            .map(str::to_owned);
                        let resolved_base =
                            commits.base.clone().or_else(|| commits.auto_base.clone());
                        let files = client
                            .changes_files(
                                &root_path,
                                &ChangesTarget::Branch,
                                effective_base.as_deref(),
                            )
                            .ok()
                            .map(|f| f.files);
                        BranchRefresh {
                            branch_name: commits.branch,
                            effective_base,
                            resolved_base,
                            has_head: commits.head_commit.is_some(),
                            files,
                        }
                    })
            } else {
                None
            };
            let mut commit_results = Vec::with_capacity(commit_targets.len());
            for target in commit_targets {
                let proto = target.to_protocol();
                if let Ok(files) = client.changes_files(&root_path, &proto, None) {
                    commit_results.push((target, files.files));
                }
            }
            (branch_refresh, commit_results)
        });
        cx.spawn(async move |this, cx| {
            let (branch_refresh, commit_results) = task.await;
            #[cfg(test)]
            if let Some(rx) = hold_rx {
                let _ = rx.await;
            }
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.non_uncommitted_seq != seq {
                    return;
                }
                let mut to_reload = Vec::new();
                let mut to_load_hunks = Vec::new();
                let mut changed = false;

                if let Some(branch) = branch_refresh {
                    let branch_keys: Vec<TabKey> = this
                        .docs
                        .keys()
                        .filter(|k| k.target == SelectedTarget::Branch)
                        .cloned()
                        .collect();
                    for key in branch_keys {
                        let Some(doc) = this.docs.get_mut(&key) else {
                            continue;
                        };
                        let prev_base = doc.expected_base(&key).map(str::to_owned);
                        let base_changed = prev_base != branch.effective_base
                            || (branch.effective_base.is_none()
                                && branch.resolved_base.is_some()
                                && doc.resolved_base != branch.resolved_base);
                        if doc.branch_name != branch.branch_name
                            || !doc.branch_known
                            || doc.resolved_base != branch.resolved_base
                            || doc.diff_base != branch.effective_base
                            || doc.has_head != branch.has_head
                        {
                            changed = true;
                        }
                        doc.diff_base = branch.effective_base.clone();
                        doc.resolved_base = branch.resolved_base.clone();
                        doc.branch_name = branch.branch_name.clone();
                        doc.branch_known = true;
                        doc.has_head = branch.has_head;

                        let Some(files) = branch.files.as_ref() else {
                            continue;
                        };
                        if let Some(entry) = files.iter().find(|f| f.path == key.path) {
                            let prev_old = doc.hint_old_blob_id.clone();
                            let prev_new = doc.hint_new_blob_id.clone();
                            let blobs_changed =
                                prev_old != entry.old_blob_id || prev_new != entry.new_blob_id;
                            if doc.apply_list_entry_viewed(
                                entry,
                                started_viewed_rev,
                                &mut this.viewed_revision,
                                cx,
                            ) {
                                changed = true;
                            }
                            let matches_loaded = matches!(
                                &doc.diff,
                                FileTabDiff::Loaded(loaded)
                                    if loaded.base == branch.effective_base
                                        && loaded.old_blob_id == entry.old_blob_id
                                        && loaded.new_blob_id == entry.new_blob_id
                            );
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
                                        || matches!(
                                            doc.diff,
                                            FileTabDiff::NoChanges
                                                | FileTabDiff::Idle
                                                | FileTabDiff::Failed(_)
                                        )
                                    {
                                        to_reload.push(key);
                                    }
                                } else {
                                    doc.diff = FileTabDiff::Idle;
                                    doc.diff_reading = false;
                                    doc.diff_read += 1;
                                    let entry_blobs =
                                        (entry.old_blob_id.clone(), entry.new_blob_id.clone());
                                    if base_changed || doc.hunk_blobs.as_ref() != Some(&entry_blobs)
                                    {
                                        doc.hunk_lines.clear();
                                        doc.hunk_blobs = None;
                                        to_load_hunks.push(key);
                                    }
                                }
                            }
                        } else {
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
                            changed = true;
                        }
                    }
                }

                for (target, files) in commit_results {
                    let matching_keys: Vec<TabKey> = this
                        .docs
                        .keys()
                        .filter(|k| k.target == target)
                        .cloned()
                        .collect();
                    for key in matching_keys {
                        let Some(doc) = this.docs.get_mut(&key) else {
                            continue;
                        };
                        if let Some(entry) = files.iter().find(|f| f.path == key.path)
                            && doc.apply_list_entry_viewed(
                                entry,
                                started_viewed_rev,
                                &mut this.viewed_revision,
                                cx,
                            )
                        {
                            changed = true;
                        }
                    }
                }
                for key in to_reload {
                    this.load_tab_diff(key, cx);
                }
                for key in to_load_hunks {
                    this.load_code_hunks(key, cx);
                }
                if changed {
                    cx.notify();
                }
            });
        })
        .detach();
    }
}
