use super::*;

impl ChangesView {
    pub(crate) fn receive(&mut self, event: FilesEvent, cx: &mut Context<Self>) {
        self.on_files_event(&event, cx);
        self.viewer
            .update(cx, |viewer, cx| viewer.on_files_event(&event, cx));
    }

    pub(crate) fn viewer(&self) -> Entity<UnifiedViewer> {
        self.viewer.clone()
    }

    pub(crate) fn widening(&self, cx: &App) -> Option<bool> {
        self.viewer.read(cx).widening()
    }

    pub(crate) fn selected_target(&self) -> Option<&SelectedTarget> {
        match &self.body {
            Body::Root(state) => Some(&state.selected_target),
            _ => None,
        }
    }

    pub(crate) fn selected_path(&self) -> Option<&str> {
        match &self.body {
            Body::Root(state) => state.selected_path.as_deref(),
            _ => None,
        }
    }

    pub(crate) fn focus_place(&self) -> FocusPlace {
        self.focus_place
    }

    pub(crate) fn unified_diff_view(&self, cx: &App) -> Option<Entity<UnifiedDiffView>> {
        self.viewer.read(cx).tab_diff_view()
    }

    pub(crate) fn set_hold_diffs(&mut self, hold: bool, cx: &mut Context<Self>) {
        self.viewer
            .update(cx, |viewer, _| viewer.set_hold_diffs(hold));
    }

    pub(crate) fn release_held_diffs(&mut self, cx: &mut Context<Self>) -> usize {
        self.viewer
            .update(cx, |viewer, _| viewer.release_held_diffs())
    }

    pub(crate) fn set_hold_snapshots(&mut self, hold: bool) {
        self.hold_snapshots = hold;
    }

    pub(crate) fn held_snapshots_len(&self) -> usize {
        self.held_snapshots.len()
    }

    pub(crate) fn release_held_snapshots(&mut self) -> usize {
        let mut released = 0;
        for tx in self.held_snapshots.drain(..) {
            let _ = tx.send(());
            released += 1;
        }
        released
    }

    pub(crate) fn reload_diff_for_test(&mut self, path: &str, cx: &mut Context<Self>) {
        let target = match &self.body {
            Body::Root(state) => state.selected_target.clone(),
            _ => SelectedTarget::Uncommitted,
        };
        let key = TabKey::new(path, target);
        self.viewer
            .update(cx, |viewer, cx| viewer.load_tab_diff(key, cx));
    }

    pub(crate) fn highlighted_row(&self) -> Option<&ListRow> {
        match &self.body {
            Body::Root(state) => state.list_rows.get(state.highlighted_list_row),
            _ => None,
        }
    }

    pub(crate) fn is_diff_loading(&self, cx: &App) -> bool {
        self.viewer.read(cx).is_diff_loading()
    }

    pub(crate) fn watched(&self) -> Option<&Path> {
        self.watched.as_deref()
    }
}
