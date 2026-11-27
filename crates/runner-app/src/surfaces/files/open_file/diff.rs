use super::*;

impl UnifiedViewer {
    pub(super) fn reconcile_diff_tabs(
        &mut self,
        refresh_active_diffs: bool,
        viewed_snapshot_rev: Option<u64>,
        authoritative_uncommitted: bool,
        cx: &mut Context<Self>,
    ) {
        let clean_keys: Vec<TabKey> = self
            .docs
            .iter()
            .filter(|(key, doc)| {
                !key.is_outside()
                    && key.target == SelectedTarget::Uncommitted
                    && ((!doc.has_changes_target && matches!(doc.content, Content::Deleted(_)))
                        || ((!doc.has_changes_target || authoritative_uncommitted)
                            && self.has_uncommitted_diff(&key.path) == Some(false)))
            })
            .map(|(k, _)| k.clone())
            .collect();
        let mut to_read = Vec::new();
        for key in clean_keys {
            if let Some(doc) = self.docs.get_mut(&key) {
                let was_diff = doc.mode == DocMode::Diff;
                doc.mode = DocMode::Code;
                doc.clear_uncommitted_diff_hints();
                if was_diff || matches!(doc.content, Content::Idle) {
                    to_read.push(key.clone());
                }
            }
            if !self.changes_rail_shown
                && let Some(idx) = self.tabs.position_key(&key)
            {
                self.tabs.pin(idx);
            }
        }
        for key in to_read {
            self.read_file(key, cx);
        }
        // Apply fresh authoritative metadata and viewed marks from `changes_files(Uncommitted)`
        // atomically per blob pair so a newer viewed toggle on an older version can never
        // attach to a newer blob pair.
        if let Some(started_rev) = viewed_snapshot_rev
            && let Some(files) = self.uncommitted_files.as_ref()
        {
            for (key, doc) in &mut self.docs {
                if key.is_outside() || key.target != SelectedTarget::Uncommitted {
                    continue;
                }
                let Some(entry) = files.get(&key.path) else {
                    continue;
                };
                doc.working_tree_missing = Some(entry.status == FileStatus::Deleted);
                doc.apply_list_entry_viewed(entry, started_rev, &mut self.viewed_revision, cx);
            }
        }
        // Only fetch Code-mode hunks for modified files whose blob pair is not yet cached,
        // so unrelated edits elsewhere in the checkout do not re-run `changes_diff`.
        let code_hunk_keys: Vec<TabKey> = self
            .docs
            .iter()
            .filter(|(key, doc)| {
                if key.is_outside()
                    || doc.mode != DocMode::Code
                    || key.target != SelectedTarget::Uncommitted
                    || doc.is_working_tree_deleted(key)
                {
                    return false;
                }
                let Some(entry) = self
                    .uncommitted_files
                    .as_ref()
                    .and_then(|files| files.get(&key.path))
                else {
                    return false;
                };
                let blobs_known = entry.old_blob_id.is_some() || entry.new_blob_id.is_some();
                if !blobs_known {
                    return refresh_active_diffs || doc.hunk_lines.is_empty();
                }
                doc.hunk_blobs.as_ref()
                    != Some(&(entry.old_blob_id.clone(), entry.new_blob_id.clone()))
            })
            .map(|(k, _)| k.clone())
            .collect();
        for key in code_hunk_keys {
            self.load_code_hunks(key, cx);
        }
        if refresh_active_diffs {
            let active_diff_keys: Vec<TabKey> = self
                .docs
                .iter()
                .filter(|(key, doc)| {
                    if key.target != SelectedTarget::Uncommitted || doc.mode != DocMode::Diff {
                        return false;
                    }
                    // If `Loaded` already matches the fresh blob pair from `changes_files`,
                    // its viewed state was updated above and no diff re-parse is needed.
                    if let Some(entry) = self
                        .uncommitted_files
                        .as_ref()
                        .and_then(|files| files.get(&key.path))
                        && (entry.old_blob_id.is_some() || entry.new_blob_id.is_some())
                        && matches!(
                            &doc.diff,
                            FileTabDiff::Loaded(loaded)
                                if loaded.old_blob_id == entry.old_blob_id
                                    && loaded.new_blob_id == entry.new_blob_id
                        )
                    {
                        return false;
                    }
                    true
                })
                .map(|(k, _)| k.clone())
                .collect();
            for key in active_diff_keys {
                self.load_tab_diff(key, cx);
            }
        }
    }

    /// Lightweight diff-hunk fetch for Code mode so modified files opened
    /// directly from `Files` show the `[ ]` hunk indicator and support `[`/`]`
    /// without waiting for the user to visit Diff mode first.
    pub(super) fn load_code_hunks(&mut self, key: TabKey, cx: &mut Context<Self>) {
        if key.is_outside() {
            return;
        }
        let Some(root) = self.root_path().map(PathBuf::from) else {
            return;
        };
        self.next_hunk_read += 1;
        let seq = self.next_hunk_read;
        let requested_base = match self.docs.get_mut(&key) {
            Some(doc) if doc.mode == DocMode::Code => {
                doc.hunk_read = seq;
                doc.expected_base(&key).map(str::to_owned)
            }
            _ => return,
        };
        let relative = key.path.clone();
        let proto_target = key.target.to_protocol();
        let req_base = requested_base.clone();
        self.request(
            cx,
            move |client| {
                let diff = client.changes_diff(
                    &root,
                    &proto_target,
                    &relative,
                    req_base.as_deref(),
                    false,
                )?;
                let lines = match &diff.body {
                    runner_core::protocol::changes::DiffBody::Text { hunks, .. } => {
                        hunks.iter().map(|h| h.new_start.max(1)).collect::<Vec<_>>()
                    }
                    _ => Vec::new(),
                };
                Ok((lines, diff.old_blob_id, diff.new_blob_id))
            },
            move |this, result, cx| {
                let Some(doc) = this.docs.get_mut(&key) else {
                    return;
                };
                if doc.hunk_read != seq || requested_base.as_deref() != doc.expected_base(&key) {
                    return;
                }
                if let Ok((lines, old_blob, new_blob)) = result {
                    doc.hunk_lines = lines;
                    doc.hunk_blobs = Some((old_blob, new_blob));
                    cx.notify();
                }
            },
        );
    }

    pub(crate) fn load_tab_diff(&mut self, key: TabKey, cx: &mut Context<Self>) {
        if key.is_outside() {
            return;
        }
        let Some(root) = self.root_path().map(PathBuf::from) else {
            return;
        };
        let default_has_head = self.root_has_head();
        self.next_diff_read += 1;
        let seq = self.next_diff_read;
        let started_viewed_rev = self.viewed_revision;
        let (requested_base, need_commits, hint_snapshot, viewed_stale, doc_has_head) =
            match self.docs.get_mut(&key) {
                Some(doc) => {
                    doc.diff_read = seq;
                    doc.diff_reading = true;
                    let req_base = doc.expected_base(&key).map(str::to_owned);
                    let keep_existing = matches!(
                        &doc.diff,
                        FileTabDiff::Loaded(loaded)
                            if loaded.target == key.target
                                && loaded.base == req_base
                                && loaded.path == key.path
                    );
                    if !keep_existing {
                        doc.diff = FileTabDiff::Loading;
                    }
                    let need_commits = matches!(key.target, SelectedTarget::Branch)
                        && (!doc.branch_known || doc.resolved_base.is_none());
                    let hint_snapshot = (
                        doc.hint_old_blob_id.clone(),
                        doc.hint_new_blob_id.clone(),
                        doc.hint_viewed,
                    );
                    (
                        req_base,
                        need_commits,
                        hint_snapshot,
                        doc.viewed_stale,
                        doc.has_head,
                    )
                }
                None => return,
            };
        #[cfg(test)]
        let hold_rx = self.hold_diffs.then(|| {
            let (tx, rx) = futures::channel::oneshot::channel();
            self.held_diffs.push(tx);
            rx
        });
        let relative = key.path.clone();
        let target_is_uncommitted = key.target == SelectedTarget::Uncommitted;
        let proto_target = key.target.to_protocol();
        let req_base = requested_base.clone();
        let want_bytes = raster_image_format(&relative).is_some();
        let client = self.client.clone();
        let generation = self.generation;
        let task = cx.background_spawn(async move {
            let diff = client.changes_diff(
                &root,
                &proto_target,
                &relative,
                req_base.as_deref(),
                want_bytes,
            )?;
            // Only query commits/files when rail metadata or fresh matching version hints are
            // absent; normal Changes rail previews supply both and issue only `changes_diff`.
            let (has_head, branch_update, resolved_base) = if need_commits {
                let commits_opt = client.changes_commits(&root, req_base.as_deref()).ok();
                (
                    commits_opt
                        .as_ref()
                        .map_or(default_has_head || doc_has_head, |c| {
                            c.head_commit.is_some()
                        }),
                    commits_opt.as_ref().map(|c| c.branch.clone()),
                    commits_opt
                        .as_ref()
                        .and_then(|c| c.base.clone().or_else(|| c.auto_base.clone())),
                )
            } else {
                (default_has_head || doc_has_head, None, None)
            };
            let hint_matches = !viewed_stale
                && hint_snapshot.2.is_some()
                && hint_snapshot.0 == diff.old_blob_id
                && hint_snapshot.1 == diff.new_blob_id;
            let viewed_state = if hint_matches {
                hint_snapshot.2
            } else {
                client
                    .changes_files(&root, &proto_target, req_base.as_deref())
                    .ok()
                    .and_then(|files| {
                        files
                            .files
                            .into_iter()
                            .find(|f| {
                                f.path == diff.path
                                    && f.old_blob_id == diff.old_blob_id
                                    && f.new_blob_id == diff.new_blob_id
                            })
                            .map(|f| (f.viewed, f.changed_since_viewed))
                    })
            };
            let working_tree_missing = if target_is_uncommitted {
                Some(diff.status == FileStatus::Deleted)
            } else if diff.status == FileStatus::Deleted {
                Some(matches!(
                    client.files_read(&root, &relative, false, None),
                    Ok(FileContent::Missing)
                ))
            } else {
                None
            };
            let prepared = prepare_diff_body(&diff);
            Ok((
                diff,
                prepared,
                has_head,
                branch_update,
                resolved_base,
                viewed_state,
                working_tree_missing,
            ))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            #[cfg(test)]
            if let Some(rx) = hold_rx {
                let _ = rx.await;
            }
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.apply_tab_diff(key, seq, started_viewed_rev, requested_base, result, cx);
                }
            });
        })
        .detach();
    }

    #[allow(clippy::type_complexity)]
    fn apply_tab_diff(
        &mut self,
        key: TabKey,
        seq: u64,
        started_viewed_rev: u64,
        requested_base: Option<String>,
        result: Result<
            (
                ChangesDiff,
                PreparedDiffBody,
                bool,
                Option<Option<String>>,
                Option<String>,
                Option<(bool, bool)>,
                Option<bool>,
            ),
            ClientError,
        >,
        cx: &mut Context<Self>,
    ) {
        let effective_mode = self.effective_diff_mode(cx);
        let weak = cx.weak_entity();
        let Some(doc) = self.docs.get_mut(&key) else {
            return;
        };
        if doc.diff_read != seq || requested_base.as_deref() != doc.expected_base(&key) {
            return;
        }
        doc.diff_reading = false;
        let mut became_clean = false;
        let mut refresh_target_viewed = false;
        match result {
            Ok((
                diff,
                prepared,
                has_head,
                branch_update,
                resolved_base,
                viewed_state,
                working_tree_missing,
            )) => {
                if let Some(branch_name) = branch_update {
                    doc.branch_name = branch_name;
                    doc.branch_known = true;
                }
                if resolved_base.is_some() {
                    doc.resolved_base = resolved_base;
                }
                doc.has_head = has_head;
                if let Some(missing) = working_tree_missing {
                    doc.working_tree_missing = Some(missing);
                }
                // Bind viewed strictly to `(diff.old_blob_id, diff.new_blob_id)` and preserve
                // any newer viewed toggle/event that landed while this diff was in flight.
                let newer_event_viewed = doc.latest_viewed.as_ref().and_then(
                    |(rev, old_blob, new_blob, viewed, changed_since)| {
                        (*rev > started_viewed_rev
                            && *old_blob == diff.old_blob_id
                            && *new_blob == diff.new_blob_id)
                            .then_some((*viewed, *changed_since))
                    },
                );
                let matching_hint_viewed = (!doc.viewed_stale
                    && doc.hint_old_blob_id == diff.old_blob_id
                    && doc.hint_new_blob_id == diff.new_blob_id)
                    .then_some(doc.hint_viewed)
                    .flatten();
                // If a viewed mark changed on another blob pair of the same path — or cleared on
                // the same pair (where an earlier tracked mark may still keep it amber) — while
                // this diff was in flight, refresh authoritative flags from daemon `changes_files`.
                refresh_target_viewed = matches!(
                    key.target,
                    SelectedTarget::Uncommitted | SelectedTarget::Branch
                ) && doc.latest_viewed.as_ref().is_some_and(
                    |(rev, old_blob, new_blob, viewed, _)| {
                        *rev > started_viewed_rev
                            && (!*viewed
                                || *old_blob != diff.old_blob_id
                                || *new_blob != diff.new_blob_id)
                    },
                );
                let (viewed, changed_since_viewed) = newer_event_viewed
                    .or(viewed_state)
                    .or(matching_hint_viewed)
                    .unwrap_or((false, false));
                doc.viewed_stale = false;
                let file_label: SharedString = file_name(&diff.path).to_owned().into();
                let grammar = super::highlight::grammar_for_path(&diff.path);
                let hunks_copy = match &diff.body {
                    runner_core::protocol::changes::DiffBody::Text { hunks, .. } => hunks.clone(),
                    _ => Vec::new(),
                };
                doc.hunk_lines = hunks_copy.iter().map(|h| h.new_start.max(1)).collect();
                doc.hunk_blobs = Some((diff.old_blob_id.clone(), diff.new_blob_id.clone()));
                let pending_line = doc.pending_diff_line.take();
                let (existing_unified, existing_canvas, existing_svg_source) = match &doc.diff {
                    FileTabDiff::Loaded(existing) => match &existing.kind {
                        LoadedDiffKind::Unified(e) => (Some(e.clone()), None, false),
                        LoadedDiffKind::Image { canvas, .. } => (None, Some(canvas.clone()), false),
                        LoadedDiffKind::Svg {
                            canvas,
                            unified,
                            source,
                            ..
                        } => (Some(unified.clone()), Some(canvas.clone()), *source),
                        _ => (None, None, false),
                    },
                    _ => (None, None, false),
                };
                let make_or_update_unified =
                    |existing: Option<Entity<UnifiedDiffView>>,
                     old_doc: Arc<Document>,
                     new_doc: Arc<Document>,
                     hunks: Vec<DiffHunk>,
                     cx: &mut Context<Self>| {
                        if let Some(entity) = existing {
                            entity.update(cx, |v, cx| {
                                v.set_show_viewed_footer(true);
                                v.set_mode(effective_mode, cx);
                                v.update_content(
                                    old_doc,
                                    new_doc,
                                    &hunks,
                                    grammar,
                                    file_label.clone(),
                                    viewed,
                                    changed_since_viewed,
                                    cx,
                                );
                                if let Some(line) = pending_line {
                                    v.scroll_to_new_line(line, cx);
                                }
                            });
                            entity
                        } else {
                            let esc_weak = weak.clone();
                            let toggle_weak = weak.clone();
                            let focus_weak = weak.clone();
                            let toggle_key = key.clone();
                            cx.new(|cx| {
                                let mut v = UnifiedDiffView::new(
                                    old_doc,
                                    new_doc,
                                    &hunks,
                                    grammar,
                                    file_label.clone(),
                                    viewed,
                                    changed_since_viewed,
                                    cx,
                                );
                                v.set_show_viewed_footer(true);
                                v.set_mode(effective_mode, cx);
                                if let Some(line) = pending_line {
                                    v.scroll_to_new_line(line, cx);
                                }
                                v.set_callbacks(
                                    Rc::new(move |window, cx| {
                                        let _ = esc_weak.update(cx, |this, cx| {
                                            this.escape_to_rail(window, cx);
                                        });
                                    }),
                                    Some(Rc::new(move |_, cx| {
                                        let toggle_key = toggle_key.clone();
                                        let _ = toggle_weak.update(cx, |this, cx| {
                                            this.toggle_viewed_for_tab(&toggle_key, cx);
                                        });
                                    })),
                                    Rc::new(move |window, cx| {
                                        let _ = focus_weak.update(cx, |this, cx| {
                                            if let Some(on_in) =
                                                this.chrome.on_diff_focus_in.clone()
                                            {
                                                on_in(window, cx);
                                            }
                                        });
                                    }),
                                );
                                v
                            })
                        }
                    };
                let make_or_update_canvas =
                    |existing: Option<Entity<ImageCanvas>>,
                     sides: image::DiffSides,
                     cx: &mut Context<Self>| {
                        if let Some(canvas) = existing {
                            canvas.update(cx, |canvas, cx| {
                                canvas.set_diff_sides(effective_mode, sides, cx);
                            });
                            canvas
                        } else {
                            let focus_weak = weak.clone();
                            cx.new(|cx| {
                                let mut canvas = ImageCanvas::new_diff(effective_mode, sides, cx);
                                canvas.set_on_focus_in(Some(Rc::new(move |window, cx| {
                                    let _ = focus_weak.update(cx, |this, cx| {
                                        if let Some(on_in) = this.chrome.on_diff_focus_in.clone() {
                                            on_in(window, cx);
                                        }
                                    });
                                })));
                                canvas
                            })
                        }
                    };
                let drop_unused_canvas = |canvas: Option<Entity<ImageCanvas>>, cx: &mut App| {
                    if let Some(canvas) = canvas {
                        canvas.update(cx, |canvas, cx| canvas.drop_atlas_textures(None, cx));
                    }
                };
                let kind = match prepared {
                    PreparedDiffBody::Unified {
                        old_doc,
                        new_doc,
                        hunks,
                    } => {
                        drop_unused_canvas(existing_canvas, cx);
                        LoadedDiffKind::Unified(make_or_update_unified(
                            existing_unified,
                            old_doc,
                            new_doc,
                            hunks,
                            cx,
                        ))
                    }
                    PreparedDiffBody::Image { sides } => {
                        let one_sided = sides.is_one_sided();
                        let canvas = make_or_update_canvas(existing_canvas, sides, cx);
                        LoadedDiffKind::Image { canvas, one_sided }
                    }
                    PreparedDiffBody::Svg {
                        old_doc,
                        new_doc,
                        hunks,
                        sides,
                    } => {
                        let one_sided = sides.is_one_sided();
                        let canvas = make_or_update_canvas(existing_canvas, sides, cx);
                        let unified =
                            make_or_update_unified(existing_unified, old_doc, new_doc, hunks, cx);
                        LoadedDiffKind::Svg {
                            canvas,
                            unified,
                            source: existing_svg_source,
                            one_sided,
                        }
                    }
                    PreparedDiffBody::CrlfOnly => {
                        drop_unused_canvas(existing_canvas, cx);
                        LoadedDiffKind::CrlfOnly
                    }
                    PreparedDiffBody::Binary { old_size, new_size } => {
                        drop_unused_canvas(existing_canvas, cx);
                        LoadedDiffKind::Binary { old_size, new_size }
                    }
                    PreparedDiffBody::TooLarge { old_size, new_size } => {
                        drop_unused_canvas(existing_canvas, cx);
                        LoadedDiffKind::TooLarge { old_size, new_size }
                    }
                    PreparedDiffBody::ModeOnly { old_mode, new_mode } => {
                        drop_unused_canvas(existing_canvas, cx);
                        LoadedDiffKind::ModeOnly { old_mode, new_mode }
                    }
                    PreparedDiffBody::Submodule {
                        old_commit,
                        new_commit,
                    } => {
                        drop_unused_canvas(existing_canvas, cx);
                        LoadedDiffKind::Submodule {
                            old_commit,
                            new_commit,
                        }
                    }
                };
                doc.hint_status = Some(diff.status.clone());
                doc.hint_counts = Some(diff.counts);
                doc.hint_viewed = Some((viewed, changed_since_viewed));
                doc.hint_old_blob_id = diff.old_blob_id.clone();
                doc.hint_new_blob_id = diff.new_blob_id.clone();
                doc.diff = FileTabDiff::Loaded(Box::new(LoadedFileDiff {
                    path: diff.path,
                    target: key.target.clone(),
                    base: requested_base,
                    old_blob_id: diff.old_blob_id,
                    new_blob_id: diff.new_blob_id,
                    status: diff.status,
                    counts: diff.counts,
                    mode_change: diff.mode_change,
                    conflicted: diff.conflicted,
                    has_head,
                    viewed,
                    changed_since_viewed,
                    hunks: hunks_copy,
                    kind,
                }));
            }
            Err(error) => {
                doc.diff.drop_image_textures(None, cx);
                let message = error.to_string();
                if message.contains("has no changes in target") {
                    if key.target == SelectedTarget::Uncommitted {
                        let was_diff = doc.mode == DocMode::Diff;
                        doc.mode = DocMode::Code;
                        doc.clear_uncommitted_diff_hints();
                        became_clean = true;
                        if was_diff || matches!(doc.content, Content::Idle) {
                            self.read_file(key.clone(), cx);
                        }
                    } else {
                        doc.diff = FileTabDiff::NoChanges;
                        doc.hint_status = None;
                        doc.hint_counts = None;
                        doc.hint_viewed = None;
                        doc.hint_old_blob_id = None;
                        doc.hint_new_blob_id = None;
                        doc.hunk_lines.clear();
                        doc.hunk_blobs = None;
                        doc.viewed_stale = false;
                    }
                } else {
                    doc.diff = FileTabDiff::Failed(message.into());
                }
            }
        }
        if became_clean {
            if !self.changes_rail_shown
                && let Some(idx) = self.tabs.position_key(&key)
            {
                self.tabs.pin(idx);
            }
            self.clear_uncommitted_path(&key.path);
            self.refresh_uncommitted_paths(false, cx);
        } else if refresh_target_viewed {
            match key.target {
                SelectedTarget::Uncommitted => self.refresh_uncommitted_paths(false, cx),
                SelectedTarget::Branch => self.refresh_non_uncommitted_targets(true, cx),
                SelectedTarget::Commit(_) => {}
            }
        }
        cx.notify();
    }
}
