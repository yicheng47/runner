use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RowIdentity {
    Folder(String),
    File(String),
}

pub(super) fn row_identity(row: &ListRow) -> RowIdentity {
    match row {
        ListRow::Folder { path, .. } => RowIdentity::Folder(path.clone()),
        ListRow::File { path, .. } => RowIdentity::File(path.clone()),
    }
}

fn find_nearest_visible_ancestor_folder(path: &str, rows: &[ListRow]) -> Option<usize> {
    let mut current = path;
    while let Some((parent, _)) = current.rsplit_once('/') {
        if let Some(idx) = rows
            .iter()
            .position(|r| matches!(r, ListRow::Folder { path: p, .. } if p == parent))
        {
            return Some(idx);
        }
        current = parent;
    }
    None
}

pub(super) fn resolve_highlighted_row(
    wanted: Option<&RowIdentity>,
    selected_path: Option<&str>,
    fallback_idx: usize,
    rows: &[ListRow],
) -> usize {
    if rows.is_empty() {
        return 0;
    }
    if let Some(identity) = wanted {
        match identity {
            RowIdentity::Folder(folder) => {
                if let Some(idx) = rows
                    .iter()
                    .position(|r| matches!(r, ListRow::Folder { path, .. } if path == folder))
                {
                    return idx;
                }
                if let Some(idx) = find_nearest_visible_ancestor_folder(folder, rows) {
                    return idx;
                }
                let prefix = format!("{folder}/");
                if let Some(idx) = rows.iter().position(
                    |r| matches!(r, ListRow::File { path, .. } if path.starts_with(&prefix)),
                ) {
                    return idx;
                }
            }
            RowIdentity::File(file) => {
                if let Some(idx) = rows
                    .iter()
                    .position(|r| matches!(r, ListRow::File { path, .. } if path == file))
                {
                    return idx;
                }
                if let Some(idx) = find_nearest_visible_ancestor_folder(file, rows) {
                    return idx;
                }
            }
        }
    }
    if let Some(selected) = selected_path {
        if let Some(idx) = rows
            .iter()
            .position(|r| matches!(r, ListRow::File { path, .. } if path == selected))
        {
            return idx;
        }
        if let Some(idx) = find_nearest_visible_ancestor_folder(selected, rows) {
            return idx;
        }
    }
    fallback_idx.min(rows.len().saturating_sub(1))
}

impl ChangesView {
    pub(super) fn set_tree_view(&mut self, tree_view: bool, cx: &mut Context<Self>) {
        crate::app_store::global_app_store(cx).update(cx, |store, cx| {
            store.update_settings(
                |settings| {
                    let changed = settings.changes_tree_view != tree_view;
                    settings.changes_tree_view = tree_view;
                    changed
                },
                true,
                cx,
            );
        });
        self.rebuild_list_rows(tree_view, cx);
    }

    fn rebuild_list_rows(&mut self, tree_view: bool, cx: &mut Context<Self>) {
        self.tree_view_pref = tree_view;
        if let Body::Root(state) = &mut self.body {
            let prev_highlight = state
                .list_rows
                .get(state.highlighted_list_row)
                .map(row_identity);
            let prev_idx = state.highlighted_list_row;
            state.list_rows = if tree_view {
                build_tree_rows(&state.files, &state.collapsed_folders)
            } else {
                build_flat_rows(&state.files)
            };
            state.highlighted_list_row = resolve_highlighted_row(
                prev_highlight.as_ref(),
                state.selected_path.as_deref(),
                prev_idx,
                &state.list_rows,
            );
            self.files_scroll
                .scroll_to_item(state.highlighted_list_row, ScrollStrategy::Nearest);
            cx.notify();
        }
    }

    pub(super) fn toggle_folder(&mut self, folder_path: &str, cx: &mut Context<Self>) {
        let Body::Root(state) = &mut self.body else {
            return;
        };
        if !state.collapsed_folders.remove(folder_path) {
            state.collapsed_folders.insert(folder_path.to_owned());
        }
        let tree_mode = self.tree_view_pref;
        self.rebuild_list_rows(tree_mode, cx);
    }

    // ── Settings, focus & keyboard ─────────────────────────────────────

    pub(super) fn settings_changed(&mut self, cx: &mut Context<Self>) {
        if self.checkout_picker.is_some() {
            self.rebuild_checkout_picker(cx);
        }
        let external_base_change = match &mut self.body {
            Body::Root(state) => {
                let top_and_branch = state
                    .commits
                    .as_ref()
                    .map(|c| (&c.checkout_top, c.branch.as_deref()))
                    .or_else(|| {
                        state
                            .root
                            .git
                            .as_ref()
                            .map(|g| (&g.top, g.branch.as_deref()))
                    });
                if let Some((top, branch)) = top_and_branch {
                    let next = files::settings(cx)
                        .and_then(|s| s.changes_base(top, branch))
                        .map(str::to_owned);
                    if next != state.chosen_base {
                        state.chosen_base = next;
                        state.missing_base_notice = None;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            _ => false,
        };
        if external_base_change {
            self.load_snapshot_inner(true, true, false, cx);
        }
        let look = Look::current(cx);
        let tree_pref = files::settings(cx).is_some_and(|s| s.changes_tree_view);
        if tree_pref != self.tree_view_pref {
            self.rebuild_list_rows(tree_pref, cx);
        }
        if look != self.look {
            self.look = look;
            cx.notify();
        }
    }

    pub(super) fn on_focus_lost(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = window.focus_lost_restore_target(cx) else {
            return;
        };
        if target == self.root_focus
            || target == self.checkout_button_focus
            || target == self.checkout_return_focus
            || target == self.base_chip_focus
        {
            self.focus_list(window, cx);
        }
    }

    pub(crate) fn focus_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus_place == FocusPlace::Diff {
            self.focus_place = FocusPlace::Files;
        }
        self.list_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn enter_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_place = FocusPlace::Diff;
        self.viewer
            .update(cx, |viewer, cx| viewer.focus_active(window, cx));
        cx.notify();
    }

    pub(super) fn jump_hunk(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        self.viewer
            .update(cx, |viewer, cx| viewer.jump_hunk(forward, cx))
    }

    fn has_checkout_trigger(&self) -> bool {
        let Body::Root(state) = &self.body else {
            return false;
        };
        let viewing_other = is_viewing_other_checkout(&state.root);
        let worktrees_slice = state
            .commits
            .as_ref()
            .map(|c| c.worktrees.as_slice())
            .filter(|w| !w.is_empty())
            .or_else(|| state.root.git.as_ref().map(|g| g.worktrees.as_slice()))
            .unwrap_or(&[]);
        live_worktrees(worktrees_slice).len() > 1 || viewing_other
    }

    fn is_viewing_other(&self) -> bool {
        matches!(&self.body, Body::Root(state) if is_viewing_other_checkout(&state.root))
    }

    fn cycle_list_focus(&mut self, backwards: bool, window: &mut Window, cx: &mut Context<Self>) {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum ListStop {
            Files,
            Commits,
            Base,
            Checkout,
            CheckoutReturn,
        }

        let mut stops = vec![ListStop::Files, ListStop::Commits, ListStop::Base];
        if self.has_checkout_trigger() {
            stops.push(ListStop::Checkout);
            if self.is_viewing_other() {
                stops.push(ListStop::CheckoutReturn);
            }
        }
        let current = if self.checkout_return_focus.is_focused(window) {
            ListStop::CheckoutReturn
        } else if self.checkout_button_focus.is_focused(window) {
            ListStop::Checkout
        } else if self.base_chip_focus.is_focused(window) {
            ListStop::Base
        } else if self.focus_place == FocusPlace::Commits {
            ListStop::Commits
        } else {
            ListStop::Files
        };
        let pos = stops.iter().position(|&s| s == current).unwrap_or(0);
        let next_pos = if backwards {
            pos.checked_sub(1).unwrap_or(stops.len() - 1)
        } else {
            (pos + 1) % stops.len()
        };
        match stops[next_pos] {
            ListStop::Files => {
                self.focus_place = FocusPlace::Files;
                self.list_focus.focus(window, cx);
            }
            ListStop::Commits => {
                self.focus_place = FocusPlace::Commits;
                self.list_focus.focus(window, cx);
            }
            ListStop::Base => {
                self.base_chip_focus.focus(window, cx);
            }
            ListStop::Checkout => {
                self.checkout_button_focus.focus(window, cx);
            }
            ListStop::CheckoutReturn => {
                self.checkout_return_focus.focus(window, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn on_list_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = event.keystroke.modifiers;
        if modifiers.platform || modifiers.control || modifiers.alt {
            return;
        }
        if event.keystroke.key == "tab" {
            cx.stop_propagation();
            self.cycle_list_focus(modifiers.shift, window, cx);
            return;
        }
        if !self.list_focus.is_focused(window) {
            return;
        }
        match event.keystroke.key.as_str() {
            "down" | "j" => {
                cx.stop_propagation();
                self.step_list(1, cx);
            }
            "up" | "k" => {
                cx.stop_propagation();
                self.step_list(-1, cx);
            }
            "left" if self.focus_place == FocusPlace::Files => {
                if self.move_list_left(cx) {
                    cx.stop_propagation();
                }
            }
            "right" if self.focus_place == FocusPlace::Files => {
                if self.move_list_right(cx) {
                    cx.stop_propagation();
                }
            }
            "enter" => {
                cx.stop_propagation();
                if self.focus_place == FocusPlace::Files
                    && let Some((folder_path, _)) = self.highlighted_folder()
                {
                    self.toggle_folder(&folder_path, cx);
                } else if self.focus_place == FocusPlace::Files {
                    let open_path = match &self.body {
                        Body::Root(state) if state.selected_path.is_none() => {
                            match state.list_rows.get(state.highlighted_list_row) {
                                Some(ListRow::File { path, .. }) => Some(path.clone()),
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    if let Some(path) = open_path {
                        self.select_file_by_path(path, false, cx);
                    } else if self.viewer.read(cx).has_tabs() {
                        self.enter_diff(window, cx);
                    }
                } else if self.viewer.read(cx).has_tabs() {
                    self.enter_diff(window, cx);
                }
            }
            "space" if self.focus_place == FocusPlace::Files => {
                cx.stop_propagation();
                if let Some((folder_path, _)) = self.highlighted_folder() {
                    self.toggle_folder(&folder_path, cx);
                } else if let Body::Root(state) = &self.body {
                    let path = match state.list_rows.get(state.highlighted_list_row) {
                        Some(ListRow::File { path, .. }) => Some(path.clone()),
                        _ => state.selected_path.clone(),
                    };
                    if let Some(path) = path {
                        self.toggle_viewed_for_path(&path, cx);
                    }
                }
            }
            "]" => {
                cx.stop_propagation();
                self.jump_hunk(true, cx);
            }
            "[" => {
                cx.stop_propagation();
                self.jump_hunk(false, cx);
            }
            _ => {}
        }
    }

    fn move_list_left(&mut self, cx: &mut Context<Self>) -> bool {
        if let Some((folder_path, true)) = self.highlighted_folder() {
            self.toggle_folder(&folder_path, cx);
            return true;
        }
        let parent_idx = {
            let Body::Root(state) = &self.body else {
                return false;
            };
            let Some(row) = state.list_rows.get(state.highlighted_list_row) else {
                return false;
            };
            find_nearest_visible_ancestor_folder(row.path(), &state.list_rows)
        };
        if let Some(idx) = parent_idx
            && let Body::Root(state) = &mut self.body
        {
            state.highlighted_list_row = idx;
            self.files_scroll
                .scroll_to_item(idx, ScrollStrategy::Nearest);
            cx.notify();
            return true;
        }
        false
    }

    fn move_list_right(&mut self, cx: &mut Context<Self>) -> bool {
        match self.highlighted_folder() {
            Some((folder_path, false)) => {
                self.toggle_folder(&folder_path, cx);
                true
            }
            Some((_, true)) => {
                let has_visible_child = match &self.body {
                    Body::Root(state) => match (
                        state.list_rows.get(state.highlighted_list_row),
                        state.list_rows.get(state.highlighted_list_row + 1),
                    ) {
                        (
                            Some(ListRow::Folder { depth, .. }),
                            Some(
                                ListRow::Folder {
                                    depth: child_depth, ..
                                }
                                | ListRow::File {
                                    depth: child_depth, ..
                                },
                            ),
                        ) => child_depth > depth,
                        _ => false,
                    },
                    _ => false,
                };
                if has_visible_child {
                    self.step_list(1, cx);
                    true
                } else {
                    false
                }
            }
            None => false,
        }
    }

    fn highlighted_folder(&self) -> Option<(String, bool)> {
        let Body::Root(state) = &self.body else {
            return None;
        };
        match state.list_rows.get(state.highlighted_list_row)? {
            ListRow::Folder { path, expanded, .. } => Some((path.clone(), *expanded)),
            ListRow::File { .. } => None,
        }
    }

    fn step_list(&mut self, delta: isize, cx: &mut Context<Self>) {
        match self.focus_place {
            FocusPlace::Commits => {
                let next_target = {
                    let Body::Root(state) = &self.body else {
                        return;
                    };
                    let Some(commits) = &state.commits else {
                        return;
                    };
                    let rows: Vec<CommitRow> =
                        build_commit_rows(commits, state.branch_count, state.uncommitted_count);
                    if rows.is_empty() {
                        return;
                    }
                    let current_idx = rows
                        .iter()
                        .position(|r| r.target() == state.selected_target)
                        .unwrap_or(0);
                    let next_idx = (current_idx as isize + delta)
                        .clamp(0, rows.len().saturating_sub(1) as isize)
                        as usize;
                    self.commits_scroll.scroll_to_item(next_idx);
                    rows[next_idx].target()
                };
                self.select_target(next_target, cx);
            }
            FocusPlace::Files | FocusPlace::Diff => {
                self.focus_place = FocusPlace::Files;
                let file_to_select = {
                    let Body::Root(state) = &mut self.body else {
                        return;
                    };
                    if state.list_rows.is_empty() {
                        return;
                    }
                    let next_idx = (state.highlighted_list_row as isize + delta)
                        .clamp(0, state.list_rows.len().saturating_sub(1) as isize)
                        as usize;
                    state.highlighted_list_row = next_idx;
                    self.files_scroll
                        .scroll_to_item(next_idx, ScrollStrategy::Nearest);
                    match &state.list_rows[next_idx] {
                        ListRow::File { path, .. } => Some(path.clone()),
                        _ => None,
                    }
                };
                if let Some(path) = file_to_select {
                    self.select_file_by_path(path, false, cx);
                } else {
                    cx.notify();
                }
            }
        }
    }
}
