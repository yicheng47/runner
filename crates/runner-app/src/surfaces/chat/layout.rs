use super::*;

impl NativeRoot {
    pub(crate) fn apply_pane_drop(
        &mut self,
        drag: &PaneDrag,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((target, side)) = self.pane_drop.take() else {
            return;
        };
        cx.notify();
        if drag.tab_id != target.tab_id
            || drag.pane_id == target.pane_id
            || self
                .tabs
                .active()
                .is_none_or(|layout| layout.id != drag.tab_id)
        {
            return;
        }
        self.move_pane(&drag.pane_id, &target.pane_id, side, window, cx);
    }

    pub(crate) fn move_pane(
        &mut self,
        pane_id: &str,
        target_id: &str,
        side: DropSide,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let remote = self.front_page_client(cx).is_remote();
        if !remote && self.route != AppRoute::Chat {
            return;
        }
        let Some(mut draft) = self.front_page_layout().cloned() else {
            return;
        };
        if self.launcher_tab_pending(&draft.id, cx) {
            return;
        }
        let bounds = self
            .front_page_pane_bounds()
            .get(&PaneKey::new(&draft.id, target_id))
            .copied();
        if !drop_allowed(bounds, side, self.settings(cx).app_zoom) {
            return;
        }
        if let Err(error) = draft.move_to(pane_id, target_id, side) {
            if !remote {
                self.chat_error = Some(error.to_string().into());
                cx.notify();
            }
            return;
        }
        if !remote {
            *self.tabs.active_mut().unwrap() = draft.clone();
        }
        let saved = self.save_workspace_layout(draft, false, cx);
        if remote {
            return;
        }
        let result = saved
            .and_then(|_| self.reload_tabs(cx))
            .and_then(|_| self.ensure_active_tab_attached(window, cx));
        match result {
            Ok(()) => {
                self.chat_error = None;
                self.remember_active_role(cx);
                self.mark_active_tab_viewed(window, cx);
                self.focus_active_terminal(window, cx);
            }
            Err(error) => self.chat_error = Some(error.to_string().into()),
        }
        cx.notify();
    }

    pub(crate) fn split_pane(
        &mut self,
        pane_id: &str,
        orientation: SplitOrientation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let remote = self.front_page_client(cx).is_remote();
        if !remote && self.route != AppRoute::Chat {
            return;
        }
        let Some(original) = self.front_page_layout().cloned() else {
            return;
        };
        if self.launcher_tab_pending(&original.id, cx) {
            return;
        }
        let bounds = self.split_pane_bounds(&original.id, pane_id, cx);
        if !matches!(
            split_decision(bounds, orientation, self.settings(cx).app_zoom),
            SplitDecision::Allowed
        ) {
            if !remote {
                self.chat_error = Some(crate::i18n::message("ui.too_small_to_split"));
                cx.notify();
            }
            return;
        }
        if self.maximized_panes.remove(&original.id).is_some() {
            cx.notify();
        }
        let terminal_location = self.active_tab_is_terminal(cx).then(|| {
            let session = original
                .root
                .leaves()
                .into_iter()
                .find(|leaf| leaf.id == pane_id)
                .and_then(|leaf| leaf.session_id.as_deref());
            if remote {
                (
                    ProjectScope::or_root(original.parent_id.clone()),
                    session
                        .and_then(|id| self.front_page_session(id, cx))
                        .and_then(|session| session.cwd),
                )
            } else {
                session
                    .and_then(|id| {
                        self.session_entry(id, cx).map(|entry| {
                            (
                                ProjectScope::or_root(entry.project_id.clone()),
                                self.session_start_cwd(id, cx),
                            )
                        })
                    })
                    .unwrap_or_else(|| self.terminal_start_location(cx))
            }
        });
        let mut draft = original.clone();
        let next = match draft.split(pane_id, orientation) {
            Ok(next) => next,
            Err(error) => {
                if !remote {
                    self.chat_error = Some(error.to_string().into());
                    cx.notify();
                }
                return;
            }
        };
        if !remote {
            *self.tabs.active_mut().unwrap() = draft.clone();
        }
        if let Some((scope, cwd)) = terminal_location {
            self.spawn_terminal_in_pane(
                next,
                if remote { draft } else { original },
                scope,
                cwd,
                window,
                cx,
            );
            return;
        }
        let saved = self.save_workspace_layout(draft, false, cx);
        if remote {
            return;
        }
        let result = saved
            .and_then(|_| self.reload_tabs(cx))
            .and_then(|_| self.ensure_active_tab_attached(window, cx));
        match result {
            Ok(()) => {
                self.chat_error = None;
                self.remember_active_role(cx);
                self.mark_active_tab_viewed(window, cx);
                self.focus_active_terminal(window, cx);
            }
            Err(error) => self.chat_error = Some(error.to_string().into()),
        }
        cx.notify();
    }

    pub(crate) fn split_pane_right(
        &mut self,
        _: &SplitPaneRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.split_focused_pane(SplitOrientation::Row, window, cx);
    }

    pub(crate) fn split_pane_down(
        &mut self,
        _: &SplitPaneDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.split_focused_pane(SplitOrientation::Column, window, cx);
    }

    pub(crate) fn toggle_maximize_pane_action(
        &mut self,
        _: &ToggleMaximizePane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.front_page_is_chat() {
            return;
        }
        let Some(pane_id) = self
            .front_page_layout()
            .map(|layout| layout.focused_pane_id.clone())
        else {
            return;
        };
        self.toggle_maximize_pane(&pane_id, window, cx);
    }

    pub(crate) fn toggle_maximize_pane(
        &mut self,
        pane_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session_id) = self.front_page_layout().and_then(|layout| {
            layout
                .root
                .leaves()
                .into_iter()
                .find(|leaf| leaf.id == pane_id)
                .map(|leaf| leaf.session_id.clone())
        }) else {
            return;
        };
        match session_id {
            Some(session_id) => self.focus_terminal(pane_id, &session_id, window, cx),
            None => self.focus_pane(pane_id, cx),
        }
        let Some(layout) = self.front_page_layout().cloned() else {
            return;
        };
        toggle_maximized(&mut self.maximized_panes, &layout, pane_id);
        cx.notify();
    }

    pub(super) fn split_focused_pane(
        &mut self,
        orientation: SplitOrientation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pane_id) = self
            .front_page_layout()
            .map(|layout| layout.focused_pane_id.clone())
        else {
            return;
        };
        self.split_pane(&pane_id, orientation, window, cx);
    }

    pub(crate) fn persist_active_tab(&mut self, cx: &mut Context<Self>) -> Result<()> {
        let layout = self
            .front_page_layout()
            .context("active tab is missing")?
            .clone();
        self.save_workspace_layout(layout, false, cx)
    }

    pub(crate) fn resize_split(
        &mut self,
        split_id: &str,
        orientation: SplitOrientation,
        event: &DragMoveEvent<SplitResizeDrag>,
        cx: &mut Context<Self>,
    ) {
        if !self.front_page_client(cx).supports(&["layout"]) {
            return;
        }
        if self
            .front_page_layout()
            .is_some_and(|layout| self.launcher_tab_pending(&layout.id, cx))
        {
            return;
        }
        let position = event.event.position;
        let bounds = event.bounds;
        let ratio = match orientation {
            SplitOrientation::Row => {
                f32::from(position.x - bounds.left()) / f32::from(bounds.size.width)
            }
            SplitOrientation::Column => {
                f32::from(position.y - bounds.top()) / f32::from(bounds.size.height)
            }
        }
        .clamp(0.15, 0.85)
            * 100.;
        if self.remote_client.is_active() {
            self.remote_client.resize_split(split_id, ratio, cx);
            return;
        }
        if self
            .tabs
            .active_mut()
            .is_some_and(|layout| layout.set_split_sizes(split_id, [ratio, 100. - ratio]))
        {
            self.split_sizes_dirty = true;
            cx.notify();
        }
        if self.resizing_split.as_deref() != Some(split_id) {
            self.resizing_split = Some(split_id.to_owned());
            cx.notify();
        }
    }

    pub(crate) fn finish_chat_panel_resize(&mut self, cx: &mut Context<Self>) {
        if !self.chat_panel_resizing {
            return;
        }
        self.chat_panel_resizing = false;
        cx.notify();
    }

    pub(crate) fn finish_split_resize(&mut self, cx: &mut Context<Self>) {
        if self.remote_client.is_active() {
            if let Some(draft) = self.remote_client.finish_split_resize() {
                self.remote_layout(draft, false, cx);
            }
            return;
        }

        if self.resizing_split.take().is_some() {
            cx.notify();
        }
        if !self.split_sizes_dirty {
            return;
        }
        self.split_sizes_dirty = false;
        if let Err(error) = self.persist_active_tab(cx) {
            self.chat_error = Some(error.to_string().into());
        }
    }

    pub(crate) fn finish_terminal_drawer_resize(&mut self, cx: &mut Context<Self>) {
        if self.remote_client.is_active() {
            self.remote_client.finish_drawer_resize(cx);
            return;
        }

        if !self.drawer_resizing {
            return;
        }
        self.drawer_resizing = false;
        if let Err(error) = self.persist_active_tab(cx) {
            self.chat_error = Some(error.to_string().into());
        }
        cx.notify();
    }
}
