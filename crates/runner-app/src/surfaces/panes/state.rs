use super::*;

pub(crate) struct PaneRename {
    pub(crate) session_id: String,
    pub(crate) original: String,
    pub(crate) input: Entity<runner_app::ui::TextField>,
}

#[allow(clippy::enum_variant_names)]
pub(crate) enum TerminalCloseTarget {
    ArchiveChatPane {
        session_id: String,
        close: PendingPaneClose,
    },
    ArchiveAll {
        session_ids: Vec<String>,
        source: ArchiveAllSource,
        confirmation_body: crate::i18n::Text,
    },
    ArchiveMission {
        mission_id: String,
    },
}

pub(crate) struct TerminalCloseConfirm {
    pub(crate) id: u64,
    pub(crate) target: TerminalCloseTarget,
    pub(crate) focus: FocusHandle,
    pub(crate) previous_focus: Option<FocusHandle>,
    pub(crate) worktree_options: Vec<crate::surfaces::sidebar::WorktreeRemoveOption>,
    pub(crate) worktree_check_generation: u64,
}

impl TerminalCloseConfirm {
    pub(crate) fn open(
        pending: &mut Option<Self>,
        target: TerminalCloseTarget,
        window: &mut Window,
        cx: &mut App,
    ) {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        if pending.is_some() {
            return;
        }
        let previous_focus = window.focused(cx);
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        *pending = Some(Self {
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            target,
            focus,
            previous_focus,
            worktree_options: Vec::new(),
            worktree_check_generation: 0,
        });
    }

    pub(crate) fn restore_focus(&self, window: &mut Window, cx: &mut App) {
        if let Some(previous_focus) = &self.previous_focus {
            previous_focus.focus(window, cx);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingPaneClose {
    pub(crate) tab_id: String,
    pub(crate) pane_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingTabClose {
    pub(crate) tab_id: String,
}

#[derive(Clone, Copy)]
pub(crate) enum ArchiveAllSource {
    Chat,
    Sidebar,
}

/// Pane menus, copy controls, bounds, maximize, rename, drag-drop, split
/// resizing, and close/archive confirmation state owned by `NativeRoot`.
#[derive(Default)]
pub(crate) struct PaneState {
    pub(crate) session_key_copies: HashMap<String, Entity<CopyValueButton>>,
    pub(crate) worktree_copies: HashMap<String, Entity<CopyValueButton>>,
    pub(crate) action_menus: HashMap<String, Entity<PopoverMenu>>,
    pub(crate) split_menus: HashMap<SplitMenuKey, Entity<PopoverMenu>>,
    pub(crate) bounds: HashMap<PaneKey, Size<Pixels>>,
    /// Tab id → the pane maximized in it; view state only, never persisted.
    pub(crate) maximized: HashMap<String, String>,
    pub(crate) drop_target: Option<(PaneKey, DropSide)>,
    pub(crate) rename: Option<PaneRename>,
    pub(crate) _rename_focus_subscription: Option<Subscription>,
    pub(crate) _close_focus_subscription: Option<Subscription>,
    pub(crate) close_confirm: Option<TerminalCloseConfirm>,
    pub(crate) pending_closes: HashMap<String, PendingPaneClose>,
    pub(crate) pending_tab_close: Option<PendingTabClose>,
    pub(crate) split_sizes_dirty: bool,
    pub(crate) resizing_split: Option<String>,
}

impl PaneState {
    pub(crate) fn retain_split_menus_for_tab(&mut self, tab_id: &str, panes: &HashSet<&str>) {
        self.split_menus
            .retain(|key, _| key.tab_id == tab_id && panes.contains(key.pane_id.as_str()));
    }

    pub(crate) fn retain_action_menus(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.action_menus.retain(|id, _| keep(id));
    }

    /// The pane a tab shows alone. Maximize lasts only while that pane is in a
    /// split and holds the tab's focus, so anything that moves the focus to
    /// another pane, closes the pane or collapses the split shows the split again.
    pub(crate) fn maximized_leaf<'a>(&self, layout: &'a PaneLayout) -> Option<&'a PaneLeaf> {
        let pane_id = self.maximized.get(&layout.id)?;
        let leaves = layout.root.leaves();
        (leaves.len() > 1 && layout.focused_pane_id == *pane_id)
            .then(|| leaves.into_iter().find(|leaf| leaf.id == *pane_id))
            .flatten()
    }

    pub(crate) fn is_maximized(&self, layout: &PaneLayout) -> bool {
        self.maximized_leaf(layout).is_some()
    }

    pub(crate) fn is_pane_maximized(&self, layout: &PaneLayout, pane_id: &str) -> bool {
        self.maximized_leaf(layout)
            .is_some_and(|leaf| leaf.id == pane_id)
    }

    /// Maximizes `pane_id`, which the caller has focused, or restores the split
    /// when it already is maximized. Returns whether the pane is maximized now.
    pub(crate) fn toggle_maximized(&mut self, layout: &PaneLayout, pane_id: &str) -> bool {
        if self
            .maximized_leaf(layout)
            .is_some_and(|leaf| leaf.id == pane_id)
        {
            self.maximized.remove(&layout.id);
            return false;
        }
        self.maximized.insert(layout.id.clone(), pane_id.to_owned());
        if self.maximized_leaf(layout).is_none() {
            self.maximized.remove(&layout.id);
            return false;
        }
        true
    }

    pub(crate) fn prune_maximized(&mut self, layout: &PaneLayout) {
        if self.maximized.contains_key(&layout.id) && self.maximized_leaf(layout).is_none() {
            self.maximized.remove(&layout.id);
        }
    }

    pub(crate) fn clear_maximized_tab(&mut self, tab_id: &str) -> bool {
        self.maximized.remove(tab_id).is_some()
    }

    pub(crate) fn clear_maximized(&mut self) {
        self.maximized.clear();
    }

    /// Whether a pane of `layout` shows `session_id`; a pane hidden behind a
    /// maximized neighbour does not, though it keeps running.
    pub(crate) fn shows_session(&self, layout: &PaneLayout, session_id: &str) -> bool {
        match self.maximized_leaf(layout) {
            Some(leaf) => leaf.session_id.as_deref() == Some(session_id),
            None => layout.contains_session(session_id),
        }
    }

    pub(crate) fn drop_preview_for(&self, pane: &PaneKey) -> Option<DropSide> {
        self.drop_target
            .as_ref()
            .and_then(|(key, side)| (key == pane).then_some(*side))
    }

    pub(crate) fn is_drop_target(&self, pane: &PaneKey) -> bool {
        self.drop_target
            .as_ref()
            .is_some_and(|(key, _)| key == pane)
    }

    pub(crate) fn update_drop_target(&mut self, next: Option<(PaneKey, DropSide)>) -> bool {
        if self.drop_target != next {
            self.drop_target = next;
            true
        } else {
            false
        }
    }

    pub(crate) fn take_drop_target(&mut self) -> Option<(PaneKey, DropSide)> {
        self.drop_target.take()
    }

    pub(crate) fn clear_drop_target(&mut self) {
        self.drop_target = None;
    }

    pub(crate) fn is_renaming(&self) -> bool {
        self.rename.is_some()
    }

    pub(crate) fn rename_input_for(
        &self,
        session_id: &str,
    ) -> Option<Entity<runner_app::ui::TextField>> {
        self.rename
            .as_ref()
            .filter(|rename| rename.session_id == session_id)
            .map(|rename| rename.input.clone())
    }

    pub(crate) fn clear_rename(&mut self) {
        self.rename = None;
        self._rename_focus_subscription = None;
    }

    pub(crate) fn clear_rename_for_session(&mut self, session_id: &str) {
        if self
            .rename
            .as_ref()
            .is_some_and(|rename| rename.session_id == session_id)
        {
            self.clear_rename();
        }
    }

    pub(crate) fn clear_rename_matching_input(
        &mut self,
        input: &Entity<runner_app::ui::TextField>,
    ) {
        if self
            .rename
            .as_ref()
            .is_some_and(|rename| &rename.input == input)
        {
            self.clear_rename();
        }
    }

    pub(crate) fn has_close_confirm(&self) -> bool {
        self.close_confirm.is_some()
    }

    pub(crate) fn clear_close_confirm(&mut self) {
        self.close_confirm = None;
    }

    pub(crate) fn take_close_confirm(&mut self) -> Option<TerminalCloseConfirm> {
        self.close_confirm.take()
    }

    pub(crate) fn has_pending_close(&self, session_id: &str) -> bool {
        self.pending_closes.contains_key(session_id)
    }

    pub(crate) fn is_pane_closing(&self, pane: &PaneKey) -> bool {
        self.pending_closes
            .values()
            .any(|close| close.tab_id == pane.tab_id && close.pane_id == pane.pane_id)
    }

    pub(crate) fn track_pending_close(&mut self, session_id: String, close: PendingPaneClose) {
        self.pending_closes.insert(session_id, close);
    }

    pub(crate) fn take_pending_closes(
        &mut self,
        attempted: &[String],
        removed: &[String],
    ) -> Vec<PendingPaneClose> {
        attempted
            .iter()
            .filter_map(|session_id| {
                let close = self.pending_closes.remove(session_id)?;
                removed.contains(session_id).then_some(close)
            })
            .collect()
    }

    pub(crate) fn set_pending_tab_close(&mut self, tab_id: String) {
        self.pending_tab_close = Some(PendingTabClose { tab_id });
    }

    pub(crate) fn clear_pending_tab_close(&mut self) {
        self.pending_tab_close = None;
    }

    pub(crate) fn clear_pending_tab_close_if(
        &mut self,
        predicate: impl FnOnce(&PendingTabClose) -> bool,
    ) {
        if self.pending_tab_close.as_ref().is_some_and(predicate) {
            self.pending_tab_close = None;
        }
    }

    pub(crate) fn take_pending_tab_close_if(
        &mut self,
        predicate: impl FnOnce(&mut PendingTabClose) -> bool,
    ) -> Option<PendingTabClose> {
        self.pending_tab_close.take_if(predicate)
    }

    pub(crate) fn mark_split_sizes_dirty(&mut self) {
        self.split_sizes_dirty = true;
    }

    pub(crate) fn begin_split_resize(&mut self, split_id: &str) -> bool {
        if self.resizing_split.as_deref() != Some(split_id) {
            self.resizing_split = Some(split_id.to_owned());
            true
        } else {
            false
        }
    }

    pub(crate) fn take_resizing_split(&mut self) -> bool {
        self.resizing_split.take().is_some()
    }

    pub(crate) fn take_split_sizes_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.split_sizes_dirty, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_state_tracks_maximize_visibility_and_pruning() {
        let mut state = PaneState::default();
        let mut layout = PaneLayout::single(Some("chat-a"), &[]);
        layout.id = "tab-1".into();
        let left = layout.focused_pane_id.clone();
        let right = layout.split(&left, SplitOrientation::Row).unwrap();
        layout.assign_session(&right, "chat-b").unwrap();
        layout.focus_pane(&left);

        assert!(!state.is_maximized(&layout));
        assert!(state.shows_session(&layout, "chat-a"));
        assert!(state.shows_session(&layout, "chat-b"));

        assert!(state.toggle_maximized(&layout, &left));
        assert!(state.is_maximized(&layout));
        assert!(state.is_pane_maximized(&layout, &left));
        assert!(!state.is_pane_maximized(&layout, &right));
        assert!(state.shows_session(&layout, "chat-a"));
        assert!(!state.shows_session(&layout, "chat-b"));

        layout.focus_pane(&right);
        assert!(!state.is_maximized(&layout));
        state.prune_maximized(&layout);
        assert!(state.maximized.is_empty());

        assert!(state.toggle_maximized(&layout, &right));
        assert!(state.clear_maximized_tab("tab-1"));
        assert!(!state.clear_maximized_tab("tab-1"));
        assert!(state.maximized.is_empty());
    }

    #[test]
    fn pane_state_tracks_drop_targets_and_split_resizes() {
        let mut state = PaneState::default();
        let pane = PaneKey::new("tab-1", "p1");
        let other = PaneKey::new("tab-1", "p2");

        assert!(state.update_drop_target(Some((pane.clone(), DropSide::Left))));
        assert!(!state.update_drop_target(Some((pane.clone(), DropSide::Left))));
        assert_eq!(state.drop_preview_for(&pane), Some(DropSide::Left));
        assert_eq!(state.drop_preview_for(&other), None);
        assert!(state.is_drop_target(&pane));
        assert!(!state.is_drop_target(&other));
        assert_eq!(
            state.take_drop_target(),
            Some((pane.clone(), DropSide::Left))
        );
        assert!(state.drop_target.is_none());

        state.update_drop_target(Some((pane, DropSide::Right)));
        state.clear_drop_target();
        assert!(state.drop_target.is_none());

        assert!(!state.take_resizing_split());
        assert!(!state.take_split_sizes_dirty());
        assert!(state.begin_split_resize("split-1"));
        assert!(!state.begin_split_resize("split-1"));
        state.mark_split_sizes_dirty();
        assert!(state.take_resizing_split());
        assert!(state.split_sizes_dirty);
        assert!(state.take_split_sizes_dirty());
        assert!(!state.take_resizing_split());
        assert!(!state.take_split_sizes_dirty());
    }

    #[test]
    fn pane_state_tracks_pending_pane_and_tab_closes() {
        let mut state = PaneState::default();
        let pane_a = PaneKey::new("tab-1", "p1");
        let pane_b = PaneKey::new("tab-1", "p2");
        state.track_pending_close(
            "s1".into(),
            PendingPaneClose {
                tab_id: "tab-1".into(),
                pane_id: "p1".into(),
            },
        );
        assert!(state.has_pending_close("s1"));
        assert!(state.is_pane_closing(&pane_a));
        assert!(!state.is_pane_closing(&pane_b));

        let taken = state.take_pending_closes(&["s1".into()], &["s1".into()]);
        assert_eq!(
            taken,
            vec![PendingPaneClose {
                tab_id: "tab-1".into(),
                pane_id: "p1".into(),
            }]
        );
        assert!(!state.has_pending_close("s1"));
        assert!(!state.is_pane_closing(&pane_a));

        state.set_pending_tab_close("tab-1".into());
        assert!(state
            .take_pending_tab_close_if(|close| close.tab_id == "other")
            .is_none());
        assert_eq!(
            state.take_pending_tab_close_if(|close| close.tab_id == "tab-1"),
            Some(PendingTabClose {
                tab_id: "tab-1".into(),
            })
        );
        assert!(state.pending_tab_close.is_none());

        state.set_pending_tab_close("tab-2".into());
        state.clear_pending_tab_close_if(|close| close.tab_id == "other");
        assert!(state.pending_tab_close.is_some());
        state.clear_pending_tab_close_if(|close| close.tab_id == "tab-2");
        assert!(state.pending_tab_close.is_none());
    }
}
