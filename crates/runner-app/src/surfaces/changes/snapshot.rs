use super::*;

struct SnapshotPayload {
    commits: ChangesCommits,
    wanted_base: Option<String>,
    uncommitted: ChangesFiles,
    branch: Option<ChangesFiles>,
    commit_files: Option<(String, ChangesFiles)>,
}

impl ChangesView {
    pub(super) fn load_snapshot(&mut self, cx: &mut Context<Self>) {
        self.load_snapshot_inner(true, true, true, cx);
    }

    pub(super) fn load_snapshot_inner(
        &mut self,
        reload_same_diff: bool,
        clear_error: bool,
        allow_preview_replace: bool,
        cx: &mut Context<Self>,
    ) {
        let settings_snapshot = files::settings(cx).cloned();
        let Body::Root(state) = &mut self.body else {
            return;
        };
        state.snapshot_seq += 1;
        state.files_seq += 1;
        let seq = state.snapshot_seq;
        let files_seq = state.files_seq;
        let root_path = state.root.path.clone();
        let known_top_and_branch = state
            .commits
            .as_ref()
            .map(|c| (c.checkout_top.clone(), c.branch.clone()))
            .or_else(|| {
                state
                    .root
                    .git
                    .as_ref()
                    .map(|g| (g.top.clone(), g.branch.clone()))
            });
        let initial_base = known_top_and_branch.and_then(|(top, branch)| {
            settings_snapshot
                .as_ref()?
                .changes_base(&top, branch.as_deref())
                .map(str::to_owned)
        });
        let wanted_commit = match &state.selected_target {
            SelectedTarget::Commit(id) => Some(id.clone()),
            _ => None,
        };
        #[cfg(test)]
        let hold_rx = self.hold_snapshots.then(|| {
            let (tx, rx) = futures::channel::oneshot::channel();
            self.held_snapshots.push(tx);
            rx
        });
        let client = self.client.clone();
        let generation = self.generation;
        let task = cx.background_spawn(async move {
            let mut commits = client.changes_commits(&root_path, initial_base.as_deref())?;
            let wanted_base = settings_snapshot
                .as_ref()
                .and_then(|s| s.changes_base(&commits.checkout_top, commits.branch.as_deref()))
                .map(str::to_owned);
            if wanted_base != initial_base {
                commits = client.changes_commits(&root_path, wanted_base.as_deref())?;
            }
            let effective_base = (!commits.base_missing)
                .then_some(wanted_base.as_deref())
                .flatten();
            let uncommitted = client.changes_files(
                &root_path,
                &runner_core::protocol::changes::ChangesTarget::Uncommitted,
                None,
            )?;
            let branch = if has_all_changes(&commits) {
                Some(client.changes_files(
                    &root_path,
                    &runner_core::protocol::changes::ChangesTarget::Branch,
                    effective_base,
                )?)
            } else {
                None
            };
            let commit_files = if let Some(id) = wanted_commit {
                if commits.commits.iter().any(|c| c.id == id) {
                    client
                        .changes_files(
                            &root_path,
                            &runner_core::protocol::changes::ChangesTarget::Commit(id.clone()),
                            None,
                        )
                        .ok()
                        .map(|files| (id, files))
                } else {
                    None
                }
            } else {
                None
            };
            Ok(SnapshotPayload {
                commits,
                wanted_base,
                uncommitted,
                branch,
                commit_files,
            })
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            #[cfg(test)]
            if let Some(rx) = hold_rx {
                let _ = rx.await;
            }
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.apply_snapshot(
                        seq,
                        files_seq,
                        reload_same_diff,
                        clear_error,
                        allow_preview_replace,
                        result,
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_snapshot(
        &mut self,
        seq: u64,
        files_seq: u64,
        reload_same_diff: bool,
        clear_error: bool,
        allow_preview_replace: bool,
        result: Result<SnapshotPayload, ClientError>,
        cx: &mut Context<Self>,
    ) {
        let is_current = match &self.body {
            Body::Root(state) => self.shown && seq >= state.snapshot_seq,
            _ => false,
        };
        if !is_current {
            return;
        }
        let payload = match result {
            Ok(payload) => payload,
            Err(error) => {
                let msg = error.to_string();
                if self.recover_from_root_error(&msg, cx) {
                    return;
                }
                self.error = Some(msg.into());
                cx.notify();
                return;
            }
        };
        let Body::Root(state) = &mut self.body else {
            return;
        };
        if clear_error {
            self.error = None;
        }
        let prev_top_and_branch = state
            .commits
            .as_ref()
            .map(|c| (c.checkout_top.clone(), c.branch.clone()))
            .or_else(|| {
                state
                    .root
                    .git
                    .as_ref()
                    .map(|g| (g.top.clone(), g.branch.clone()))
            });
        let new_top_and_branch = (
            payload.commits.checkout_top.clone(),
            payload.commits.branch.clone(),
        );
        let branch_changed = prev_top_and_branch
            .as_ref()
            .is_some_and(|(prev_top, prev_branch)| {
                !picker::same_checkout_path(prev_top, &new_top_and_branch.0)
                    || prev_branch != &new_top_and_branch.1
            });
        let mut forget_missing_base: Option<(PathBuf, Option<String>)> = None;
        if payload.commits.base_missing {
            if let Some(missing_ref) = payload.wanted_base.clone() {
                state.missing_base_notice = Some(missing_ref);
                state.chosen_base = None;
                forget_missing_base = Some(new_top_and_branch);
            } else if branch_changed {
                state.missing_base_notice = None;
                state.chosen_base = None;
            }
        } else {
            if branch_changed
                || payload.wanted_base.is_some()
                || payload.wanted_base != state.chosen_base
            {
                state.missing_base_notice = None;
            }
            state.chosen_base = payload.wanted_base;
        }
        let uncommitted_count = payload.uncommitted.files.len();
        let branch_count = payload.branch.as_ref().map_or(0, |b| b.files.len());
        let prev_target = state.selected_target.clone();
        let prev_files_seq = state.files_seq;
        let next_target = if !state.initial_target_chosen {
            state.initial_target_chosen = true;
            choose_initial_target(&payload.commits, uncommitted_count)
        } else {
            reconcile_target(&state.selected_target, &payload.commits, uncommitted_count)
        };

        let active_files_opt = match &next_target {
            SelectedTarget::Uncommitted => Some(payload.uncommitted),
            SelectedTarget::Branch => Some(payload.branch.unwrap_or(ChangesFiles {
                checkout_top: payload.commits.checkout_top.clone(),
                files: Vec::new(),
                totals: ChangesTotals::default(),
            })),
            SelectedTarget::Commit(id) => payload
                .commit_files
                .filter(|(loaded_id, _)| loaded_id == id)
                .map(|(_, files)| files),
        };

        state.commits = Some(payload.commits);
        state.uncommitted_count = uncommitted_count;
        state.branch_count = branch_count;
        state.selected_target = next_target.clone();
        self.status_stale = false;
        if let Some((top, branch)) = forget_missing_base {
            crate::app_store::global_app_store(cx).update(cx, |store, cx| {
                store.update_settings(
                    |settings| settings.set_changes_base(&top, branch.as_deref(), None),
                    true,
                    cx,
                );
            });
        }
        if self.base_picker.is_some() {
            self.rebuild_base_picker();
        }
        if self.checkout_picker.is_some() {
            self.rebuild_checkout_picker(cx);
        }
        if next_target != SelectedTarget::Branch
            && let Body::Root(state) = &self.body
        {
            let root_path = state.root.path.clone();
            let chosen_base = state.chosen_base.clone();
            let resolved_base = state
                .commits
                .as_ref()
                .and_then(|c| c.base.clone().or_else(|| c.auto_base.clone()));
            let branch_name = state
                .commits
                .as_ref()
                .and_then(|c| c.branch.clone())
                .or_else(|| state.root.git.as_ref().and_then(|g| g.branch.clone()));
            let has_head = state.commits.as_ref().map_or_else(
                || state.root.git.as_ref().is_some_and(|g| g.commit.is_some()),
                |c| c.head_commit.is_some(),
            );
            self.viewer.update(cx, |viewer, cx| {
                viewer.sync_branch_base(
                    &root_path,
                    chosen_base.as_deref(),
                    resolved_base.as_deref(),
                    branch_name.as_deref(),
                    has_head,
                    true,
                    cx,
                );
            });
        }
        if files_seq < prev_files_seq && next_target == prev_target && active_files_opt.is_none() {
            cx.notify();
            return;
        }
        let active_files = active_files_opt.unwrap_or_else(|| {
            let Body::Root(state) = &self.body else {
                unreachable!()
            };
            ChangesFiles {
                checkout_top: state
                    .commits
                    .as_ref()
                    .map(|c| c.checkout_top.clone())
                    .unwrap_or_default(),
                files: Vec::new(),
                totals: ChangesTotals::default(),
            }
        });
        if let Body::Root(state) = &mut self.body {
            state.files_seq += 1;
        }
        self.apply_files_list(
            active_files,
            reload_same_diff && files_seq >= prev_files_seq,
            allow_preview_replace,
            cx,
        );
    }

    pub(super) fn select_target(&mut self, target: SelectedTarget, cx: &mut Context<Self>) {
        let Body::Root(state) = &mut self.body else {
            return;
        };
        if state.selected_target == target {
            return;
        }
        state.selected_target = target.clone();
        state.selected_path = None;
        state.highlighted_list_row = 0;
        state.files_seq += 1;
        let files_seq = state.files_seq;
        let root_path = state.root.path.clone();
        let chosen_base = matches!(target, SelectedTarget::Branch)
            .then(|| state.chosen_base.clone())
            .flatten();
        let proto = target.to_protocol();
        self.request(
            cx,
            move |client| client.changes_files(&root_path, &proto, chosen_base.as_deref()),
            move |this, result, cx| {
                let is_current = match &this.body {
                    Body::Root(state) => this.shown && files_seq >= state.files_seq,
                    _ => false,
                };
                if !is_current {
                    return;
                }
                match result {
                    Ok(files) => {
                        if let Body::Root(state) = &mut this.body {
                            match state.selected_target {
                                SelectedTarget::Uncommitted => {
                                    state.uncommitted_count = files.files.len();
                                }
                                SelectedTarget::Branch => {
                                    state.branch_count = files.files.len();
                                }
                                SelectedTarget::Commit(_) => {}
                            }
                        }
                        this.apply_files_list(files, false, false, cx);
                    }
                    Err(error) => {
                        let msg = error.to_string();
                        if this.recover_from_root_error(&msg, cx) {
                            return;
                        }
                        this.error = Some(msg.into());
                        cx.notify();
                    }
                }
            },
        );
        cx.notify();
    }

    fn apply_files_list(
        &mut self,
        mut changes_files: ChangesFiles,
        reload_same_diff: bool,
        allow_preview_replace: bool,
        cx: &mut Context<Self>,
    ) {
        changes_files.files.sort_by(|a, b| a.path.cmp(&b.path));
        let tree_mode = files::settings(cx).is_some_and(|s| s.changes_tree_view);
        self.tree_view_pref = tree_mode;

        let reconcile = {
            let Body::Root(state) = &mut self.body else {
                return;
            };
            let prev_path = state.selected_path.clone();
            let prev_highlight = state
                .list_rows
                .get(state.highlighted_list_row)
                .map(row_identity);
            let prev_highlight_idx = state.highlighted_list_row;
            state.files = changes_files.files;
            state.totals = changes_files.totals;
            state.list_rows = if tree_mode {
                build_tree_rows(&state.files, &state.collapsed_folders)
            } else {
                build_flat_rows(&state.files)
            };
            let resolved_base = state
                .commits
                .as_ref()
                .and_then(|c| c.base.clone().or_else(|| c.auto_base.clone()));
            let branch_name = state
                .commits
                .as_ref()
                .and_then(|c| c.branch.clone())
                .or_else(|| state.root.git.as_ref().and_then(|g| g.branch.clone()));
            let has_head = state.commits.as_ref().map_or_else(
                || state.root.git.as_ref().is_some_and(|g| g.commit.is_some()),
                |c| c.head_commit.is_some(),
            );
            (
                state.root.path.clone(),
                state.selected_target.clone(),
                state.chosen_base.clone(),
                resolved_base,
                branch_name,
                has_head,
                state.files.clone(),
                state.list_rows.clone(),
                prev_path,
                prev_highlight,
                prev_highlight_idx,
            )
        };
        let (
            root_path,
            target,
            chosen_base,
            resolved_base,
            branch_name,
            has_head,
            files_snapshot,
            rows_snapshot,
            prev_path,
            prev_highlight,
            prev_highlight_idx,
        ) = reconcile;

        self.viewer.update(cx, |viewer, cx| {
            viewer.reconcile_changes_files(
                &root_path,
                &target,
                chosen_base.as_deref(),
                resolved_base.as_deref(),
                branch_name.as_deref(),
                has_head,
                &files_snapshot,
                &rows_snapshot,
                tree_mode,
                reload_same_diff,
                allow_preview_replace,
                cx,
            );
        });

        let active_key = self
            .viewer
            .read(cx)
            .tabs
            .active()
            .map(|tab| tab.key.clone());
        if let Body::Root(state) = &mut self.body {
            let next_path = active_key
                .filter(|key| {
                    key.target == state.selected_target
                        && state.files.iter().any(|f| f.path == key.path)
                })
                .map(|key| key.path);
            let same_path = next_path.is_some() && next_path == prev_path;
            state.selected_path = next_path;
            if state.selected_path.is_some() {
                let wanted = if same_path {
                    prev_highlight
                } else {
                    state
                        .selected_path
                        .as_ref()
                        .map(|p| RowIdentity::File(p.clone()))
                };
                state.highlighted_list_row = resolve_highlighted_row(
                    wanted.as_ref(),
                    state.selected_path.as_deref(),
                    prev_highlight_idx,
                    &state.list_rows,
                );
                if !same_path {
                    self.files_scroll
                        .scroll_to_item(state.highlighted_list_row, ScrollStrategy::Nearest);
                }
            } else {
                state.highlighted_list_row = if state.list_rows.is_empty() {
                    0
                } else {
                    resolve_highlighted_row(
                        prev_highlight.as_ref(),
                        None,
                        prev_highlight_idx,
                        &state.list_rows,
                    )
                };
            }
        }
        if let Some(saved) = self.pending_restore.take() {
            self.apply_saved_changes(saved, cx);
        }
        cx.notify();
    }

    pub(super) fn select_file_by_path(&mut self, path: String, pin: bool, cx: &mut Context<Self>) {
        let req = {
            let Body::Root(state) = &mut self.body else {
                return;
            };
            let Some(file) = state.files.iter().find(|f| f.path == path).cloned() else {
                return;
            };
            if let Some(row_idx) = state
                .list_rows
                .iter()
                .position(|r| matches!(r, ListRow::File { path: p, .. } if p == &path))
            {
                state.highlighted_list_row = row_idx;
                self.files_scroll
                    .scroll_to_item(row_idx, ScrollStrategy::Nearest);
            }
            state.selected_path = Some(path.clone());
            let resolved_base = state
                .commits
                .as_ref()
                .and_then(|c| c.base.clone().or_else(|| c.auto_base.clone()));
            let branch_name = state
                .commits
                .as_ref()
                .and_then(|c| c.branch.clone())
                .or_else(|| state.root.git.as_ref().and_then(|g| g.branch.clone()));
            let commit_short_id = match &state.selected_target {
                SelectedTarget::Commit(id) => state
                    .commits
                    .as_ref()
                    .and_then(|c| c.commits.iter().find(|commit| &commit.id == id))
                    .map(|c| c.short_id.clone())
                    .or_else(|| Some(id[..id.len().min(7)].to_owned())),
                _ => None,
            };
            let has_head = state.commits.as_ref().map_or_else(
                || state.root.git.as_ref().is_some_and(|g| g.commit.is_some()),
                |c| c.head_commit.is_some(),
            );
            OpenChangesRequest {
                root: state.root.clone(),
                path,
                target: state.selected_target.clone(),
                chosen_base: state.chosen_base.clone(),
                resolved_base,
                branch_name,
                commit_short_id,
                file,
                has_head,
            }
        };
        self.viewer
            .update(cx, |viewer, cx| viewer.open_changes_file(req, pin, cx));
        cx.notify();
    }
}
