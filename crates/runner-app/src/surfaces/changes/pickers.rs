use super::*;

pub(in super::super) struct BasePickerState {
    pub(super) field: Entity<TextField>,
    pub(super) query: String,
    pub(super) branches: ChangesBranches,
    pub(super) items: Vec<BasePickerItem>,
    pub(super) highlighted: usize,
    pub(super) moved: bool,
    pub(super) scroll: ScrollHandle,
    pub(super) reveal: Cell<bool>,
    pub(super) return_focus: FocusHandle,
    _observe: Subscription,
}

pub(in super::super) struct CheckoutPickerState {
    pub(super) field: Entity<TextField>,
    pub(super) query: String,
    pub(super) worktrees: Vec<WorktreeEntry>,
    pub(super) items: Vec<CheckoutPickerItem>,
    pub(super) highlighted: usize,
    pub(super) moved: bool,
    pub(super) scroll: ScrollHandle,
    pub(super) reveal: Cell<bool>,
    pub(super) return_focus: FocusHandle,
    _observe: Subscription,
}

impl ChangesView {
    // ── Base & Checkout pickers ────────────────────────────────────────

    pub(super) fn toggle_base_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(picker) = self.base_picker.take() {
            picker.return_focus.focus(window, cx);
            cx.notify();
            return;
        }
        self.checkout_picker = None;
        let root_path = match &mut self.body {
            Body::Root(state) => {
                state.missing_base_notice = None;
                state.root.path.clone()
            }
            _ => return,
        };
        let return_focus = if self.base_chip_focus.is_focused(window) {
            self.base_chip_focus.clone()
        } else {
            self.list_focus.clone()
        };
        let root = cx.weak_entity();
        let field = cx.new(move |field_cx| {
            let mut field = TextField::new(
                field_cx.focus_handle(),
                "",
                crate::i18n::message("changes.filter_branches"),
                false,
            )
            .text_size(theme::text_ui())
            .key_interceptor(Rc::new(move |event, window, cx| {
                root.update(cx, |this, cx| this.on_base_picker_key(event, window, cx))
                    .unwrap_or(false)
            }));
            field.set_bare(true, field_cx);
            field
        });
        let observe = cx.observe(&field, |this, field, cx| {
            let query = field.read(cx).text().to_owned();
            if let Some(picker) = &mut this.base_picker
                && picker.query != query
            {
                picker.query = query;
                picker.moved = false;
                let visible = filter_base_picker_items(&picker.items, &picker.query);
                picker.highlighted = default_base_highlight(&picker.items, &visible, &picker.query);
                picker.scroll.set_offset(gpui::Point::default());
                cx.notify();
            }
        });
        field.read(cx).focus_handle().focus(window, cx);
        self.base_picker = Some(BasePickerState {
            field,
            query: String::new(),
            branches: ChangesBranches::default(),
            items: Vec::new(),
            highlighted: 0,
            moved: false,
            scroll: ScrollHandle::new(),
            reveal: Cell::new(false),
            return_focus,
            _observe: observe,
        });
        self.rebuild_base_picker();
        self.request(
            cx,
            move |client| client.changes_branches(&root_path),
            |this, result, cx| {
                if let (Some(picker), Ok(branches)) = (&mut this.base_picker, result) {
                    picker.branches = branches;
                    this.rebuild_base_picker();
                    cx.notify();
                }
            },
        );
        cx.notify();
    }

    pub(super) fn close_base_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(picker) = self.base_picker.take() {
            picker.return_focus.focus(window, cx);
            cx.notify();
        }
    }

    pub(super) fn rebuild_base_picker(&mut self) {
        let (chosen_base, auto_base, current_branch) = match &self.body {
            Body::Root(state) => {
                let auto = state.commits.as_ref().and_then(|c| c.auto_base.clone());
                let branch = state
                    .commits
                    .as_ref()
                    .and_then(|c| c.branch.clone())
                    .or_else(|| state.root.git.as_ref().and_then(|g| g.branch.clone()));
                (state.chosen_base.clone(), auto, branch)
            }
            _ => return,
        };
        let Some(picker) = &mut self.base_picker else {
            return;
        };
        picker.items = build_base_picker_items(
            chosen_base.as_deref(),
            auto_base.as_deref(),
            current_branch.as_deref(),
            &picker.branches,
            crate::i18n::text("changes.base_auto"),
            crate::i18n::text("changes.base_none"),
        );
        let visible = filter_base_picker_items(&picker.items, &picker.query);
        picker.highlighted = if picker.moved {
            picker.highlighted.min(visible.len().saturating_sub(1))
        } else {
            default_base_highlight(&picker.items, &visible, &picker.query)
        };
    }

    fn on_base_picker_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(picker) = &mut self.base_picker else {
            return false;
        };
        let visible = filter_base_picker_items(&picker.items, &picker.query);
        match event.keystroke.key.as_str() {
            "down" => {
                picker.highlighted = (picker.highlighted + 1).min(visible.len().saturating_sub(1));
                picker.moved = true;
                picker.reveal.set(true);
            }
            "up" => {
                picker.highlighted = picker.highlighted.saturating_sub(1);
                picker.moved = true;
                picker.reveal.set(true);
            }
            "escape" => {
                self.close_base_picker(window, cx);
                return true;
            }
            "enter" => {
                if let Some(index) = visible.get(picker.highlighted).copied()
                    && let Some(item) = picker.items.get(index).cloned()
                {
                    self.pick_base(item.base_ref, window, cx);
                }
                return true;
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    pub(super) fn pick_base(
        &mut self,
        next_base: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let return_focus = self.base_picker.take().map(|p| p.return_focus);
        let (top_and_branch, sync_info) = match &mut self.body {
            Body::Root(state) => {
                state.missing_base_notice = None;
                state.chosen_base = next_base.clone();
                let top_and_branch = state
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
                let branch_name = state
                    .commits
                    .as_ref()
                    .and_then(|c| c.branch.clone())
                    .or_else(|| state.root.git.as_ref().and_then(|g| g.branch.clone()));
                let has_head = state.commits.as_ref().map_or_else(
                    || state.root.git.as_ref().is_some_and(|g| g.commit.is_some()),
                    |c| c.head_commit.is_some(),
                );
                let sync_info = (
                    state.root.path.clone(),
                    branch_name,
                    has_head,
                    state.selected_target != SelectedTarget::Branch,
                );
                (top_and_branch, Some(sync_info))
            }
            _ => (None, None),
        };
        if let Some((root_path, branch_name, has_head, reload_now)) = sync_info {
            self.viewer.update(cx, |viewer, cx| {
                viewer.sync_branch_base(
                    &root_path,
                    next_base.as_deref(),
                    next_base.as_deref(),
                    branch_name.as_deref(),
                    has_head,
                    reload_now,
                    cx,
                );
            });
        }
        if let Some((top, branch)) = top_and_branch {
            crate::app_store::global_app_store(cx).update(cx, |store, cx| {
                store.update_settings(
                    |settings| {
                        settings.set_changes_base(&top, branch.as_deref(), next_base.as_deref())
                    },
                    true,
                    cx,
                );
            });
        }
        if let Some(focus) = return_focus {
            focus.focus(window, cx);
        }
        self.load_snapshot_inner(true, true, false, cx);
        cx.notify();
    }

    pub(super) fn toggle_checkout_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(picker) = self.checkout_picker.take() {
            picker.return_focus.focus(window, cx);
            cx.notify();
            return;
        }
        self.base_picker = None;
        self.removed_checkout_notice = None;
        let (root_path, initial_worktrees) = match &self.body {
            Body::Root(state) => {
                let wts = state
                    .commits
                    .as_ref()
                    .map(|c| c.worktrees.clone())
                    .filter(|w| !w.is_empty())
                    .or_else(|| state.root.git.as_ref().map(|g| g.worktrees.clone()))
                    .unwrap_or_default();
                (state.root.path.clone(), wts)
            }
            _ => return,
        };
        let return_focus = if self.checkout_button_focus.is_focused(window) {
            self.checkout_button_focus.clone()
        } else {
            self.list_focus.clone()
        };
        let root = cx.weak_entity();
        let field = cx.new(move |field_cx| {
            let mut field = TextField::new(
                field_cx.focus_handle(),
                "",
                crate::i18n::message("changes.filter_checkouts"),
                false,
            )
            .text_size(theme::text_ui())
            .key_interceptor(Rc::new(move |event, window, cx| {
                root.update(cx, |this, cx| {
                    this.on_checkout_picker_key(event, window, cx)
                })
                .unwrap_or(false)
            }));
            field.set_bare(true, field_cx);
            field
        });
        let observe = cx.observe(&field, |this, field, cx| {
            let query = field.read(cx).text().to_owned();
            if let Some(picker) = &mut this.checkout_picker
                && picker.query != query
            {
                picker.query = query;
                picker.moved = false;
                let visible = filter_checkout_picker_items(&picker.items, &picker.query);
                picker.highlighted =
                    default_checkout_highlight(&picker.items, &visible, &picker.query);
                picker.scroll.set_offset(gpui::Point::default());
                cx.notify();
            }
        });
        field.read(cx).focus_handle().focus(window, cx);
        self.checkout_picker = Some(CheckoutPickerState {
            field,
            query: String::new(),
            worktrees: initial_worktrees,
            items: Vec::new(),
            highlighted: 0,
            moved: false,
            scroll: ScrollHandle::new(),
            reveal: Cell::new(false),
            return_focus,
            _observe: observe,
        });
        self.rebuild_checkout_picker(cx);
        self.request(
            cx,
            move |client| client.files_worktrees(&root_path),
            |this, result, cx| {
                if let (Some(picker), Ok(worktrees)) = (&mut this.checkout_picker, result) {
                    picker.worktrees = worktrees;
                    this.rebuild_checkout_picker(cx);
                    cx.notify();
                }
            },
        );
        cx.notify();
    }

    pub(super) fn close_checkout_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(picker) = self.checkout_picker.take() {
            picker.return_focus.focus(window, cx);
            cx.notify();
        }
    }

    pub(super) fn rebuild_checkout_picker(&mut self, cx: &App) {
        let (current_top, own_checkout_top) = match &self.body {
            Body::Root(state) => {
                let current_top = state
                    .commits
                    .as_ref()
                    .map(|c| c.checkout_top.clone())
                    .or_else(|| state.root.git.as_ref().map(|g| g.top.clone()))
                    .unwrap_or_else(|| state.root.path.clone());
                (current_top, state.root.own_checkout_top.clone())
            }
            _ => return,
        };
        let home = Self::home(cx);
        let context = self.context.clone();
        let Some(picker) = &mut self.checkout_picker else {
            return;
        };
        let session_labels = picker::collect_checkout_session_labels(
            &context,
            own_checkout_top.as_deref(),
            &picker.worktrees,
            cx,
        );
        picker.items = build_checkout_picker_items(
            &current_top,
            &picker.worktrees,
            &session_labels,
            home.as_deref(),
            crate::i18n::text("ui.detached"),
        );
        let visible = filter_checkout_picker_items(&picker.items, &picker.query);
        picker.highlighted = if picker.moved {
            picker.highlighted.min(visible.len().saturating_sub(1))
        } else {
            default_checkout_highlight(&picker.items, &visible, &picker.query)
        };
    }

    fn on_checkout_picker_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(picker) = &mut self.checkout_picker else {
            return false;
        };
        let visible = filter_checkout_picker_items(&picker.items, &picker.query);
        match event.keystroke.key.as_str() {
            "down" => {
                picker.highlighted = (picker.highlighted + 1).min(visible.len().saturating_sub(1));
                picker.moved = true;
                picker.reveal.set(true);
            }
            "up" => {
                picker.highlighted = picker.highlighted.saturating_sub(1);
                picker.moved = true;
                picker.reveal.set(true);
            }
            "escape" => {
                self.close_checkout_picker(window, cx);
                return true;
            }
            "enter" => {
                if let Some(index) = visible.get(picker.highlighted).copied()
                    && let Some(item) = picker.items.get(index).cloned()
                {
                    self.pick_checkout(item.path, window, cx);
                }
                return true;
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    pub(super) fn pick_checkout(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let return_focus = self.checkout_picker.take().map(|p| p.return_focus);
        let is_own = matches!(
            &self.body,
            Body::Root(state)
                if state
                    .root
                    .own_checkout_top
                    .as_deref()
                    .is_some_and(|own| picker::same_checkout_path(own, &path))
        );
        self.set_root((!is_own).then_some(path), cx);
        if let Some(focus) = return_focus {
            focus.focus(window, cx);
        } else {
            self.list_focus.focus(window, cx);
        }
    }
}
