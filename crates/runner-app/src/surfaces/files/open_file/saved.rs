use super::*;

#[derive(Clone, Debug)]
pub(crate) struct SavedViewerTab {
    pub(crate) key: TabKey,
    pub(crate) preview: bool,
    pub(crate) mode: DocMode,
    pub(crate) has_changes_target: bool,
    pub(crate) diff_base: Option<String>,
    pub(crate) resolved_base: Option<String>,
    pub(crate) branch_name: Option<String>,
    pub(crate) commit_short_id: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct SavedViewerState {
    pub(crate) root: Option<PathBuf>,
    pub(crate) tabs: Vec<SavedViewerTab>,
    pub(crate) active: Option<TabKey>,
    pub(crate) tree_hidden: bool,
}

impl UnifiedViewer {
    // ── LRU Eviction State Persistence ─────────────────────────────────

    pub(crate) fn saved_state(&self) -> Option<SavedViewerState> {
        if self.tabs.is_empty() && !self.tree_hidden {
            return if self.root.is_none() {
                self.pending_restore.clone()
            } else {
                None
            };
        }
        let tabs = self
            .tabs
            .tabs()
            .iter()
            .map(|tab| {
                let doc = self.docs.get(&tab.key);
                SavedViewerTab {
                    key: tab.key.clone(),
                    preview: tab.preview,
                    mode: doc.map_or(DocMode::Code, |d| d.mode),
                    has_changes_target: doc.is_some_and(|d| d.has_changes_target),
                    diff_base: doc.and_then(|d| d.diff_base.clone()),
                    resolved_base: doc.and_then(|d| d.resolved_base.clone()),
                    branch_name: doc.and_then(|d| d.branch_name.clone()),
                    commit_short_id: doc.and_then(|d| d.commit_short_id.clone()),
                }
            })
            .collect();
        Some(SavedViewerState {
            root: self.root.as_ref().map(|root| root.path.clone()),
            tabs,
            active: self.tabs.active().map(|t| t.key.clone()),
            tree_hidden: self.tree_hidden,
        })
    }

    pub(crate) fn restore_saved_state(&mut self, saved: SavedViewerState, cx: &mut Context<Self>) {
        if self.root.is_some() {
            self.apply_saved_state(saved, cx);
        } else {
            self.pending_restore = Some(saved);
        }
    }

    pub(super) fn apply_saved_state(&mut self, saved: SavedViewerState, cx: &mut Context<Self>) {
        let same_root = match (self.root.as_ref(), saved.root.as_ref()) {
            (Some(r), Some(s)) => runner_core::app_paths::same_checkout_path(&r.path, s),
            (None, None) => true,
            _ => false,
        };
        if !same_root {
            self.notify_owner(cx);
            cx.notify();
            return;
        }
        self.tree_hidden = saved.tree_hidden;
        let markdown_source = super::settings(cx)
            .is_some_and(|settings| settings.markdown_open_as == MarkdownOpenAs::Source);
        let has_head = self.root_has_head();
        for item in saved.tabs {
            let outside = item.key.is_outside();
            let source = !is_svg(&item.key.path) && markdown_source;
            self.tabs.push_restored(item.key.clone(), item.preview);
            let mut doc = OpenDocument::new_code(source, !outside && has_head);
            if !outside {
                doc.mode = item.mode;
                doc.has_changes_target = item.has_changes_target;
                doc.diff_base = item.diff_base;
                doc.resolved_base = item.resolved_base;
                doc.branch_name = item.branch_name;
                doc.commit_short_id = item.commit_short_id;
                if doc.mode == DocMode::Diff {
                    doc.content = Content::Idle;
                    doc.diff = FileTabDiff::Loading;
                }
            }
            let load_diff = !outside && item.mode == DocMode::Diff;
            self.docs.insert(item.key.clone(), doc);
            if load_diff {
                self.load_tab_diff(item.key, cx);
            } else {
                self.read_file(item.key, cx);
            }
        }
        if let Some(active) = saved.active
            && let Some(index) = self.tabs.position_key(&active)
        {
            self.tabs.activate(index);
        }
        if self.shown && self.ignored_refresh.is_none() && self.has_tabs() {
            self.ignored_refresh = Some(self.refresh_ignored_tabs(cx));
        }
        self.ensure_uncommitted_paths(cx);
        self.tab_shown(cx);
    }
}
