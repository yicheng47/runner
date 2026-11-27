use super::*;

impl UnifiedViewer {
    pub(crate) fn open_tabs(&self) -> Vec<String> {
        self.tabs
            .tabs()
            .iter()
            .map(|tab| tab.key.path.clone())
            .collect()
    }

    pub(crate) fn active_tab(&self) -> Option<String> {
        self.tabs.active().map(|tab| tab.key.path.clone())
    }

    pub(crate) fn active_tab_entry(&self) -> Option<&FileTab> {
        self.tabs.active()
    }

    pub(crate) fn code_view(&self) -> Option<Entity<CodeView>> {
        self.active_view().cloned()
    }

    pub(crate) fn image_canvas(&self) -> Option<Entity<ImageCanvas>> {
        self.active_image_canvas().cloned()
    }

    pub(crate) fn active_is_image_tab(&self) -> bool {
        self.active_is_image()
    }

    pub(crate) fn showing_diff(&self) -> bool {
        self.active_showing_diff()
    }

    pub(crate) fn tab_diff_view(&self) -> Option<Entity<UnifiedDiffView>> {
        self.active_diff_view().cloned()
    }

    pub(crate) fn read_seq(&self, path: &str) -> Option<u64> {
        self.docs
            .iter()
            .find(|(k, _)| k.path == path)
            .map(|(_, d)| d.read)
    }

    pub(crate) fn reply_text(&mut self, path: &str, read: u64, text: &str, cx: &mut Context<Self>) {
        let Some(key) = self.docs.keys().find(|k| k.path == path).cloned() else {
            return;
        };
        let document = Arc::new(Document::decode(text.as_bytes()));
        self.apply_read(
            key,
            read,
            Ok(Read::Text(
                document,
                None,
                None,
                (text.len() as u64, None),
                None,
            )),
            cx,
        );
    }

    pub(crate) fn reading(&self, path: &str) -> bool {
        self.docs.iter().any(|(k, d)| k.path == path && d.reading)
    }

    pub(crate) fn last_search(&self) -> Option<u64> {
        self.find.as_ref().map(|find| find.search)
    }

    pub(crate) fn reply_search(&mut self, search: u64, count: usize, cx: &mut Context<Self>) {
        let Some(searched) = self
            .active_view()
            .map(|view| view.read(cx).document().clone())
        else {
            return;
        };
        let matches = (0..count)
            .map(|index| super::document::FileMatch {
                line: 0,
                range: index..index + 1,
            })
            .collect();
        self.apply_search(search, searched, Found::Matches(matches, false), 0, cx);
    }

    pub(crate) fn reply_binary(
        &mut self,
        path: &str,
        read: u64,
        size: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self.docs.keys().find(|k| k.path == path).cloned() else {
            return;
        };
        self.apply_read(key, read, Ok(Read::Binary(size, None)), cx);
    }

    pub(crate) fn doc_stamp(&self, path: &str) -> Option<Stamp> {
        self.docs
            .iter()
            .find(|(k, _)| k.path == path)
            .and_then(|(_, d)| d.stamp)
    }

    pub(crate) fn hold_search(&mut self, hold: bool, cx: &mut Context<Self>) {
        self.hold_search = hold;
        if !hold {
            self.search(cx);
        }
    }

    pub(crate) fn set_hold_diffs(&mut self, hold: bool) {
        self.hold_diffs = hold;
    }

    pub(crate) fn status_stale(&self) -> bool {
        self.status_stale
    }

    pub(crate) fn set_fail_next_viewed_write(&mut self, message: Option<String>) {
        self.fail_next_viewed_write = message;
    }

    pub(crate) fn release_held_diffs(&mut self) -> usize {
        let mut released = 0;
        for tx in self.held_diffs.drain(..) {
            let _ = tx.send(());
            released += 1;
        }
        released
    }

    pub(crate) fn held_diffs_len(&self) -> usize {
        self.held_diffs.len()
    }

    pub(crate) fn set_hold_uncommitted_lists(&mut self, hold: bool) {
        self.hold_uncommitted_lists = hold;
    }

    pub(crate) fn release_held_uncommitted_lists(&mut self) -> usize {
        let mut released = 0;
        for tx in self.held_uncommitted_lists.drain(..) {
            let _ = tx.send(());
            released += 1;
        }
        released
    }

    pub(crate) fn held_uncommitted_lists_len(&self) -> usize {
        self.held_uncommitted_lists.len()
    }

    pub(crate) fn set_hold_non_uncommitted_refreshes(&mut self, hold: bool) {
        self.hold_non_uncommitted_refreshes = hold;
    }

    pub(crate) fn release_held_non_uncommitted_refreshes(&mut self) -> usize {
        let mut released = 0;
        for tx in self.held_non_uncommitted_refreshes.drain(..) {
            let _ = tx.send(());
            released += 1;
        }
        released
    }

    pub(crate) fn held_non_uncommitted_refreshes_len(&self) -> usize {
        self.held_non_uncommitted_refreshes.len()
    }

    pub(crate) fn active_diff_viewed(&self) -> Option<(bool, bool)> {
        let tab = self.tabs.active()?;
        let doc = self.docs.get(&tab.key)?;
        match &doc.diff {
            FileTabDiff::Loaded(loaded) => Some((loaded.viewed, loaded.changed_since_viewed)),
            _ => None,
        }
    }

    pub(crate) fn is_diff_loading(&self) -> bool {
        self.tabs
            .active()
            .and_then(|t| self.docs.get(&t.key))
            .is_some_and(|d| d.mode == DocMode::Diff && matches!(d.diff, FileTabDiff::Loading))
    }

    #[allow(clippy::type_complexity)]
    pub(crate) fn loaded_diff_meta(
        &self,
    ) -> Option<(
        &SelectedTarget,
        Option<&str>,
        &str,
        Option<&str>,
        Option<&str>,
    )> {
        let tab = self.tabs.active()?;
        let doc = self.docs.get(&tab.key)?;
        match &doc.diff {
            FileTabDiff::Loaded(loaded) => Some((
                &loaded.target,
                loaded.base.as_deref(),
                &loaded.path,
                loaded.old_blob_id.as_deref(),
                loaded.new_blob_id.as_deref(),
            )),
            _ => None,
        }
    }

    pub(crate) fn step(&mut self, next: bool, cx: &mut Context<Self>) {
        self.step_find(next, cx);
    }

    pub(crate) fn launched(&self) -> Vec<(Option<FileLinkEditor>, PathBuf, Option<u32>)> {
        self.launched.clone()
    }

    pub(crate) fn rendered_view(&self) -> Option<Entity<MarkdownView>> {
        self.active_rendered().cloned()
    }

    pub(crate) fn opened_urls(&self) -> Vec<String> {
        self.opened_urls.clone()
    }

    pub(crate) fn follow_link(&mut self, href: &str, cx: &mut Context<Self>) {
        let Some(from) = self.active_tab() else {
            return;
        };
        self.open_link(markdown::route(href, &from), cx);
    }

    pub(crate) fn find_state(&self) -> Option<(Option<String>, Option<usize>)> {
        let find = self.find.as_ref()?;
        Some((
            Self::find_count(find).map(|(label, _)| label.to_string()),
            find.current,
        ))
    }

    pub(crate) fn watched(&self) -> Option<&Path> {
        self.watched.as_deref()
    }
}
