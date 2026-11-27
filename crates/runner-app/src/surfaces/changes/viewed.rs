use super::*;

impl ChangesView {
    pub(crate) fn on_viewer_viewed_error(&mut self, msg: String, cx: &mut Context<Self>) {
        if self.recover_from_root_error(&msg, cx) {
            return;
        }
        self.error = Some(msg.into());
        self.load_snapshot_inner(false, false, false, cx);
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_viewer_viewed_optimistic(
        &mut self,
        root: &Path,
        target: &SelectedTarget,
        path: &str,
        old_blob: Option<&str>,
        new_blob: Option<&str>,
        viewed: bool,
        cx: &mut Context<Self>,
    ) {
        let Body::Root(state) = &mut self.body else {
            return;
        };
        if !picker::same_checkout_path(&state.root.path, root) || &state.selected_target != target {
            return;
        }
        let mut updated = false;
        for file in &mut state.files {
            if file.path == path
                && file.old_blob_id.as_deref() == old_blob
                && file.new_blob_id.as_deref() == new_blob
            {
                file.viewed = viewed;
                if viewed {
                    file.changed_since_viewed = false;
                }
                updated = true;
            }
        }
        if updated {
            state.totals.viewed = state.files.iter().filter(|f| f.viewed).count();
            cx.notify();
        }
    }

    // ── Viewed marks & flat/tree toggle ────────────────────────────────

    pub(super) fn toggle_viewed_for_path(&mut self, path: &str, cx: &mut Context<Self>) {
        let (root_path, selected_target, old_blob, new_blob, next_viewed, next_changed) = {
            let Body::Root(state) = &mut self.body else {
                return;
            };
            let Some(file) = state.files.iter_mut().find(|f| f.path == path) else {
                return;
            };
            let next_viewed = !file.viewed;
            file.viewed = next_viewed;
            if next_viewed {
                file.changed_since_viewed = false;
            }
            let next_changed = file.changed_since_viewed;
            let old_blob = file.old_blob_id.clone();
            let new_blob = file.new_blob_id.clone();
            state.totals.viewed = state.files.iter().filter(|f| f.viewed).count();
            (
                state.root.path.clone(),
                state.selected_target.clone(),
                old_blob,
                new_blob,
                next_viewed,
                next_changed,
            )
        };
        self.viewer.update(cx, |viewer, cx| {
            viewer.apply_changes_viewed_optimistic(
                &root_path,
                &selected_target,
                path,
                old_blob.as_deref(),
                new_blob.as_deref(),
                next_viewed,
                next_changed,
                cx,
            );
        });
        let req_path = path.to_owned();
        let target = selected_target.to_protocol();
        self.send_viewed_toggle(
            root_path,
            target,
            req_path,
            old_blob,
            new_blob,
            next_viewed,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn send_viewed_toggle(
        &mut self,
        root_path: PathBuf,
        target: runner_core::protocol::changes::ChangesTarget,
        req_path: String,
        old_blob: Option<String>,
        new_blob: Option<String>,
        next_viewed: bool,
        cx: &mut Context<Self>,
    ) {
        self.request(
            cx,
            move |client| {
                client.changes_set_viewed(
                    &root_path,
                    &target,
                    &req_path,
                    old_blob.as_deref(),
                    new_blob.as_deref(),
                    next_viewed,
                )
            },
            |this, result, cx| {
                if let Err(error) = result {
                    let msg = error.to_string();
                    if this.recover_from_root_error(&msg, cx) {
                        return;
                    }
                    this.error = Some(msg.into());
                    this.load_snapshot_inner(false, false, false, cx);
                }
            },
        );
        cx.notify();
    }

    pub(super) fn apply_viewed_event(&mut self, event: &ChangesViewed, cx: &mut Context<Self>) {
        let mut matched_checkout = false;
        if let Body::Root(state) = &mut self.body
            && state
                .commits
                .as_ref()
                .is_some_and(|c| picker::same_checkout_path(&c.checkout_top, &event.checkout_top))
        {
            matched_checkout = true;
            let event_top = runner_core::app_paths::normalize_checkout_path(&event.checkout_top);
            let root_norm = runner_core::app_paths::normalize_checkout_path(&state.root.path);
            let event_abs = absolute(&event_top, &event.path);
            for file in &mut state.files {
                let file_abs = absolute(&root_norm, &file.path);
                if runner_core::app_paths::same_normalized_checkout_path(&file_abs, &event_abs)
                    && file.old_blob_id == event.old_blob_id
                    && file.new_blob_id == event.new_blob_id
                {
                    file.viewed = event.viewed;
                    if event.viewed {
                        file.changed_since_viewed = false;
                    }
                }
            }
            state.totals.viewed = state.files.iter().filter(|f| f.viewed).count();
        }
        if matched_checkout {
            if self.shown {
                self.load_snapshot_inner(false, true, false, cx);
            }
            cx.notify();
        }
    }
}
