use super::*;

impl UnifiedViewer {
    pub(crate) fn pin_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.tabs.pin(index) {
            cx.notify();
        }
    }

    pub(crate) fn activate_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        self.tabs.activate(index);
        self.tab_shown(cx);
    }

    pub(crate) fn cycle_tab(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.cycle(back);
        self.tab_shown(cx);
        self.focus_active(window, cx);
    }

    pub(crate) fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self.tabs.close(index)
            && let Some(old_doc) = self.docs.remove(&key)
        {
            old_doc.drop_image_textures(Some(window), cx);
        }
        if self.tabs.is_empty() {
            let was_maximized = self.maximized;
            self.reset_tabs(Some(window), cx);
            if was_maximized {
                self.maximized = false;
                if let Some(request) = self.on_request_maximize.clone() {
                    request(false, window, cx);
                }
            }
            self.sync_watch(cx);
            self.tab_shown(cx);
            self.notify_owner(cx);
            if let Some(escape) = self.chrome.on_escape_to_rail.clone() {
                escape(window, cx);
            }
            return;
        }
        self.tab_shown(cx);
        self.focus_active(window, cx);
    }

    pub(super) fn tab_shown(&mut self, cx: &mut Context<Self>) {
        let active_key = self.tabs.active().map(|t| t.key.clone());
        if let Some(on_tab) = self.chrome.on_tab_activated.clone() {
            let key_clone = active_key.clone();
            cx.defer(move |cx| on_tab(key_clone.as_ref(), cx));
        }
        let Some(index) = self.tabs.active_index() else {
            self.find = None;
            cx.notify();
            return;
        };
        self.strip.scroll_to_item(index);
        if let Some(key) = active_key {
            let (needs_read, needs_diff, needs_hunks) =
                self.docs.get(&key).map_or((false, false, false), |doc| {
                    let needs_read =
                        doc.mode == DocMode::Code && matches!(doc.content, Content::Idle);
                    let needs_diff =
                        doc.mode == DocMode::Diff && matches!(doc.diff, FileTabDiff::Idle);
                    let needs_hunks = doc.mode == DocMode::Code
                        && doc.hunk_lines.is_empty()
                        && !matches!(doc.diff, FileTabDiff::Loaded(_))
                        && !doc.is_working_tree_deleted(&key)
                        && self.tab_can_diff(&key, doc);
                    (needs_read, needs_diff, needs_hunks)
                });
            if needs_read {
                self.read_file(key.clone(), cx);
            }
            if needs_diff {
                self.load_tab_diff(key.clone(), cx);
            } else if needs_hunks {
                self.load_code_hunks(key.clone(), cx);
            }
            self.refresh_ignored(key, cx);
        }
        self.search(cx);
        self.notify_owner(cx);
        cx.notify();
    }

    pub(super) fn reset_tabs(&mut self, mut window: Option<&mut Window>, cx: &mut App) {
        for doc in self.docs.values() {
            doc.drop_image_textures(window.as_deref_mut(), cx);
        }
        self.tabs = FileTabs::default();
        self.docs.clear();
        self.find = None;
        self.list_open = false;
        self.open_in_open = false;
        self.ignored_refresh = None;
    }

    // ── Focus & Keyboard ───────────────────────────────────────────────

    pub(super) fn on_focus_lost(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ours = window
            .focus_lost_restore_target(cx)
            .is_some_and(|target| target == self.root_focus || target == self.viewer_focus);
        if ours {
            if self.has_tabs() {
                self.focus_active(window, cx);
            } else if let Some(escape) = self.chrome.on_escape_to_rail.clone() {
                escape(window, cx);
            }
        }
    }

    pub(crate) fn focus_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(diff) = self.active_diff_view() {
            diff.read(cx).focus_handle().clone().focus(window, cx);
            if let Some(on_in) = self.chrome.on_diff_focus_in.clone() {
                on_in(window, cx);
            }
        } else if self.active_showing_diff() {
            if let Some(canvas) = self.active_image_canvas() {
                canvas.read(cx).focus_handle().clone().focus(window, cx);
            } else {
                self.viewer_focus.focus(window, cx);
            }
            if let Some(on_in) = self.chrome.on_diff_focus_in.clone() {
                on_in(window, cx);
            }
        } else if let Some(view) = self.active_view() {
            view.read(cx).focus_handle().clone().focus(window, cx);
        } else if let Some(page) = self.active_rendered() {
            page.read(cx).focus_handle().clone().focus(window, cx);
        } else if let Some(canvas) = self.active_image_canvas() {
            canvas.read(cx).focus_handle().clone().focus(window, cx);
        } else {
            self.viewer_focus.focus(window, cx);
        }
    }

    pub(super) fn escape_to_rail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(escape) = self.chrome.on_escape_to_rail.clone() {
            escape(window, cx);
            cx.notify();
        }
    }

    pub(crate) fn jump_hunk(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        if let Some(diff) = self.active_diff_view().cloned() {
            return diff.update(cx, |v, cx| v.jump_hunk(forward, cx));
        }
        let hunk_lines = self.active_hunk_lines();
        if !hunk_lines.is_empty()
            && let Some(view) = self.active_view().cloned()
        {
            return view.update(cx, |v, cx| v.jump_to_hunk(&hunk_lines, forward, cx));
        }
        false
    }

    pub(super) fn on_viewer_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        let m = keystroke.modifiers;
        if m.platform || m.control || m.alt {
            return;
        }
        match keystroke.key.as_str() {
            "escape" if !m.shift => {
                if self.find.is_some() {
                    return;
                }
                if self.chrome.show_rail_column {
                    cx.stop_propagation();
                    self.escape_to_rail(window, cx);
                }
            }
            "]" if !m.shift && self.jump_hunk(true, cx) => {
                cx.stop_propagation();
            }
            "[" if !m.shift && self.jump_hunk(false, cx) => {
                cx.stop_propagation();
            }
            _ => {}
        }
    }

    pub(super) fn zoom(
        &mut self,
        step: i16,
        app: Box<dyn gpui::Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.viewer_focus.contains_focused(window, cx)
            || self.root_focus.contains_focused(window, cx)
        {
            if let Some(canvas) = self.active_image_canvas().cloned() {
                canvas.update(cx, |canvas, cx| canvas.step_zoom(step > 0, cx));
            } else {
                self.zoom_text(step, cx);
            }
        } else {
            window.dispatch_action(app, cx);
        }
    }

    fn zoom_text(&mut self, step: i16, cx: &mut Context<Self>) {
        use crate::app_settings::{TERMINAL_FONT_SIZE_MAX, TERMINAL_FONT_SIZE_MIN};
        crate::app_store::global_app_store(cx).update(cx, |store, cx| {
            store.update_settings(
                |settings| {
                    let size = Some(
                        settings
                            .files_font_size()
                            .saturating_add_signed(step)
                            .clamp(TERMINAL_FONT_SIZE_MIN, TERMINAL_FONT_SIZE_MAX),
                    );
                    let changed = settings.files_font_size != size;
                    settings.files_font_size = size;
                    changed
                },
                true,
                cx,
            );
        });
    }

    // ── Code | Diff Mode Switch ────────────────────────────────────────

    fn tab_can_diff(&self, key: &TabKey, doc: &OpenDocument) -> bool {
        if doc.has_changes_target || key.target != SelectedTarget::Uncommitted {
            return true;
        }
        !doc.is_working_tree_deleted(key) && self.has_uncommitted_diff(&key.path) == Some(true)
    }

    pub(crate) fn active_can_diff(&self) -> bool {
        let Some(tab) = self.tabs.active() else {
            return false;
        };
        let Some(doc) = self.docs.get(&tab.key) else {
            return false;
        };
        self.tab_can_diff(&tab.key, doc)
    }

    pub(crate) fn active_showing_diff(&self) -> bool {
        let Some(tab) = self.tabs.active() else {
            return false;
        };
        self.active_can_diff()
            && self
                .docs
                .get(&tab.key)
                .is_some_and(|doc| doc.mode == DocMode::Diff)
    }

    pub(crate) fn active_deleted(&self) -> bool {
        self.tabs.active().is_some_and(|tab| {
            self.docs
                .get(&tab.key)
                .is_some_and(|doc| doc.is_working_tree_deleted(&tab.key))
        })
    }

    pub(crate) fn active_diff_view(&self) -> Option<&Entity<UnifiedDiffView>> {
        let tab = self.tabs.active()?;
        if !self.active_can_diff() {
            return None;
        }
        let doc = self.docs.get(&tab.key)?;
        if doc.mode != DocMode::Diff {
            return None;
        }
        match &doc.diff {
            FileTabDiff::Loaded(loaded) => match &loaded.kind {
                LoadedDiffKind::Unified(entity) => Some(entity),
                LoadedDiffKind::Svg {
                    unified,
                    source: true,
                    ..
                } => Some(unified),
                _ => None,
            },
            _ => None,
        }
    }

    pub(super) fn active_hunk_lines(&self) -> Vec<u32> {
        let Some(tab) = self.tabs.active() else {
            return Vec::new();
        };
        let Some(doc) = self.docs.get(&tab.key) else {
            return Vec::new();
        };
        match &doc.diff {
            FileTabDiff::Loaded(loaded) => loaded.hunk_new_lines(),
            _ => doc.hunk_lines.clone(),
        }
    }

    pub(crate) fn set_active_mode(
        &mut self,
        target_mode: DocMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(active_idx) = self.tabs.active_index() else {
            return;
        };
        let key = self.tabs.tabs()[active_idx].key.clone();
        match target_mode {
            DocMode::Code => {
                if self.active_deleted() {
                    return;
                }
                // Flipping Diff -> Code promotes an italic preview tab to pinned (#1421).
                self.tabs.pin(active_idx);
                let target_line = self
                    .active_diff_view()
                    .and_then(|diff| diff.read(cx).current_new_line())
                    .or_else(|| self.active_hunk_lines().first().copied());
                let (need_read, existing_view) = match self.docs.get_mut(&key) {
                    Some(doc) => {
                        doc.mode = DocMode::Code;
                        let need_read = matches!(doc.content, Content::Idle);
                        if need_read {
                            doc.pending_code_line = target_line;
                        }
                        let view = match &doc.content {
                            Content::Text(v) => Some(v.clone()),
                            _ => None,
                        };
                        (need_read, view)
                    }
                    None => return,
                };
                if need_read {
                    self.read_file(key, cx);
                } else if let (Some(view), Some(line)) = (existing_view, target_line) {
                    view.update(cx, |v, cx| v.scroll_to_line(line, cx));
                }
                self.search(cx);
                self.focus_active(window, cx);
            }
            DocMode::Diff => {
                if !self.active_can_diff() {
                    return;
                }
                let code_line = self.active_view().map(|v| v.read(cx).line_in_view());
                if let Some(view) = self.active_view().cloned() {
                    view.update(cx, |v, cx| v.set_find(Arc::default(), None, cx));
                }
                self.find = None;
                let (need_load, existing_diff) = match self.docs.get_mut(&key) {
                    Some(doc) => {
                        doc.mode = DocMode::Diff;
                        let expected_base = doc.expected_base(&key).map(str::to_owned);
                        let has_blob_hints =
                            doc.hint_old_blob_id.is_some() || doc.hint_new_blob_id.is_some();
                        let loaded_valid = !doc.viewed_stale
                            && matches!(
                                &doc.diff,
                                FileTabDiff::Loaded(loaded)
                                    if loaded.target == key.target
                                        && loaded.base == expected_base
                                        && (!has_blob_hints
                                            || (loaded.old_blob_id == doc.hint_old_blob_id
                                                && loaded.new_blob_id == doc.hint_new_blob_id))
                            );
                        let existing = if loaded_valid {
                            match &doc.diff {
                                FileTabDiff::Loaded(loaded) => match &loaded.kind {
                                    LoadedDiffKind::Unified(e) => Some(e.clone()),
                                    LoadedDiffKind::Svg {
                                        unified,
                                        source: true,
                                        ..
                                    } => Some(unified.clone()),
                                    _ => None,
                                },
                                _ => None,
                            }
                        } else {
                            None
                        };
                        let need_load = !loaded_valid;
                        if need_load {
                            doc.pending_diff_line = code_line;
                        }
                        (need_load, existing)
                    }
                    None => return,
                };
                if need_load {
                    self.load_tab_diff(key, cx);
                } else if let (Some(diff), Some(line)) = (existing_diff, code_line) {
                    diff.update(cx, |v, cx| v.scroll_to_new_line(line, cx));
                }
                self.focus_active(window, cx);
            }
        }
        cx.notify();
    }
}
