use super::*;

impl UnifiedViewer {
    // ── Viewed Sync ────────────────────────────────────────────────────

    pub(super) fn toggle_viewed_for_tab(&mut self, key: &TabKey, cx: &mut Context<Self>) {
        let Some(root_path) = self.root_path().map(PathBuf::from) else {
            return;
        };
        self.viewed_revision += 1;
        let write_rev = self.viewed_revision;
        let (old_blob, new_blob, prev_viewed, prev_changed, next_viewed) = {
            let Some(doc) = self.docs.get_mut(key) else {
                return;
            };
            let FileTabDiff::Loaded(loaded) = &mut doc.diff else {
                return;
            };
            let prev_viewed = loaded.viewed;
            let prev_changed = loaded.changed_since_viewed;
            let next_viewed = !prev_viewed;
            let next_changed = if next_viewed { false } else { prev_changed };
            loaded.set_viewed_state(next_viewed, next_changed, cx);
            if doc.hint_old_blob_id == loaded.old_blob_id
                && doc.hint_new_blob_id == loaded.new_blob_id
            {
                doc.hint_viewed = Some((next_viewed, next_changed));
            }
            doc.latest_viewed = Some((
                write_rev,
                loaded.old_blob_id.clone(),
                loaded.new_blob_id.clone(),
                next_viewed,
                next_changed,
            ));
            (
                loaded.old_blob_id.clone(),
                loaded.new_blob_id.clone(),
                prev_viewed,
                prev_changed,
                next_viewed,
            )
        };
        if let Some(on_viewed) = self.chrome.on_viewed_toggled.clone() {
            on_viewed(
                &root_path,
                &key.target,
                &key.path,
                old_blob.as_deref(),
                new_blob.as_deref(),
                next_viewed,
                cx,
            );
        }
        let target = key.target.to_protocol();
        let req_path = key.path.clone();
        let err_key = key.clone();
        let err_root = root_path.clone();
        let err_old = old_blob.clone();
        let err_new = new_blob.clone();
        #[cfg(test)]
        let forced_error = self.fail_next_viewed_write.take();
        self.request(
            cx,
            move |client| {
                #[cfg(test)]
                if let Some(msg) = forced_error {
                    return Err(ClientError::msg(msg));
                }
                client.changes_set_viewed(
                    &root_path,
                    &target,
                    &req_path,
                    old_blob.as_deref(),
                    new_blob.as_deref(),
                    next_viewed,
                )
            },
            move |this, result, cx| {
                if let Err(error) = result {
                    this.reconcile_failed_viewed_write(
                        &err_root,
                        &err_key,
                        write_rev,
                        err_old.as_deref(),
                        err_new.as_deref(),
                        prev_viewed,
                        prev_changed,
                        error.to_string(),
                        cx,
                    );
                }
            },
        );
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    fn reconcile_failed_viewed_write(
        &mut self,
        root_path: &Path,
        key: &TabKey,
        write_rev: u64,
        old_blob: Option<&str>,
        new_blob: Option<&str>,
        prev_viewed: bool,
        prev_changed: bool,
        error_msg: String,
        cx: &mut Context<Self>,
    ) {
        let mut reverted = false;
        if let Some(doc) = self.docs.get_mut(key) {
            let superseded = doc.latest_viewed.as_ref().is_some_and(|(rev, o, n, ..)| {
                *rev > write_rev && o.as_deref() == old_blob && n.as_deref() == new_blob
            });
            if !superseded {
                if let FileTabDiff::Loaded(loaded) = &mut doc.diff
                    && loaded.old_blob_id.as_deref() == old_blob
                    && loaded.new_blob_id.as_deref() == new_blob
                {
                    loaded.set_viewed_state(prev_viewed, prev_changed, cx);
                    reverted = true;
                }
                if doc.hint_old_blob_id.as_deref() == old_blob
                    && doc.hint_new_blob_id.as_deref() == new_blob
                {
                    doc.hint_viewed = Some((prev_viewed, prev_changed));
                    reverted = true;
                }
                if reverted {
                    self.viewed_revision += 1;
                    doc.latest_viewed = Some((
                        self.viewed_revision,
                        old_blob.map(str::to_owned),
                        new_blob.map(str::to_owned),
                        prev_viewed,
                        prev_changed,
                    ));
                }
            }
        }
        if reverted && let Some(on_viewed) = self.chrome.on_viewed_toggled.clone() {
            on_viewed(
                root_path,
                &key.target,
                &key.path,
                old_blob,
                new_blob,
                prev_viewed,
                cx,
            );
        }
        if let Some(on_err) = self.chrome.on_viewed_error.clone() {
            on_err(error_msg, cx);
        }
        cx.notify();
    }

    pub(super) fn apply_viewed_event(&mut self, event: &ChangesViewed, cx: &mut Context<Self>) {
        let Some(root_path) = self.root_path() else {
            return;
        };
        let event_top = runner_core::app_paths::normalize_checkout_path(&event.checkout_top);
        let root_norm = runner_core::app_paths::normalize_checkout_path(root_path);
        let event_abs = absolute(&event_top, &event.path);
        self.viewed_revision += 1;
        let rev = self.viewed_revision;
        let mut updated = false;
        let mut refresh_uncommitted = false;
        let mut refresh_branch = false;
        for (key, doc) in &mut self.docs {
            let doc_abs = absolute(&root_norm, &key.path);
            if !runner_core::app_paths::same_normalized_checkout_path(&doc_abs, &event_abs) {
                continue;
            }
            let changed = if event.viewed {
                false
            } else if let FileTabDiff::Loaded(loaded) = &doc.diff
                && loaded.old_blob_id == event.old_blob_id
                && loaded.new_blob_id == event.new_blob_id
            {
                loaded.changed_since_viewed
            } else if doc.hint_old_blob_id == event.old_blob_id
                && doc.hint_new_blob_id == event.new_blob_id
            {
                doc.hint_viewed.is_some_and(|(_, c)| c)
            } else {
                false
            };
            // Always record the latest viewed event for this path/blob pair even while
            // a diff load is still in flight so the returning read cannot clobber it.
            doc.latest_viewed = Some((
                rev,
                event.old_blob_id.clone(),
                event.new_blob_id.clone(),
                event.viewed,
                changed,
            ));
            let same_loaded = matches!(
                &doc.diff,
                FileTabDiff::Loaded(loaded)
                    if loaded.old_blob_id == event.old_blob_id
                        && loaded.new_blob_id == event.new_blob_id
            );
            let same_hint = doc.hint_old_blob_id == event.old_blob_id
                && doc.hint_new_blob_id == event.new_blob_id;
            if let FileTabDiff::Loaded(loaded) = &mut doc.diff
                && same_loaded
            {
                loaded.set_viewed_state(event.viewed, changed, cx);
                if same_hint {
                    doc.hint_viewed = Some((event.viewed, changed));
                }
                updated = true;
            } else if same_hint {
                doc.hint_viewed = Some((event.viewed, changed));
                updated = true;
            }
            // Refresh authoritative daemon flags for mutable targets whenever either:
            // - another blob pair changed (set or clear), OR
            // - the viewed mark was cleared (even for the same pair, because an earlier
            //   tracked mark like v1 can still make an unchecked v2 `changed_since_viewed = true`).
            if !event.viewed || (!same_loaded && !same_hint) {
                match key.target {
                    SelectedTarget::Uncommitted => refresh_uncommitted = true,
                    SelectedTarget::Branch => refresh_branch = true,
                    SelectedTarget::Commit(_) => {}
                }
            }
        }
        if refresh_uncommitted {
            self.refresh_uncommitted_paths(false, cx);
        }
        if refresh_branch {
            self.refresh_non_uncommitted_targets(true, cx);
        }
        if updated {
            cx.notify();
        }
    }
}
