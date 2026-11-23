use super::*;

#[derive(Clone)]
pub(crate) struct PaneCloseFocus {
    tab_id: String,
    pane_id: String,
    focus: Option<FocusHandle>,
    session_id: Option<String>,
    epoch: u64,
}

impl PaneCloseFocus {
    pub(crate) fn capture(root: &NativeRoot, window: &Window, cx: &App) -> Option<Self> {
        let layout = root.front_page_layout()?;
        Some(Self {
            tab_id: layout.id.clone(),
            pane_id: layout.focused_pane_id.clone(),
            focus: window.focused(cx),
            session_id: layout.focused_session_id().map(str::to_owned),
            epoch: root.remote_client.ui.epoch,
        })
    }

    pub(crate) fn is_current(&self, root: &NativeRoot, window: &Window, cx: &App) -> bool {
        let origin_removed = if root.remote_client.is_active() {
            root.remote_client.tab_layout(&self.tab_id).is_none()
        } else {
            !root
                .tabs
                .tabs()
                .iter()
                .any(|layout| layout.id == self.tab_id)
        };
        root.front_page_is_chat()
            && root.remote_client.ui.epoch == self.epoch
            && window.focused(cx) == self.focus
            && (origin_removed
                || root.front_page_layout().is_some_and(|layout| {
                    layout.id == self.tab_id
                        && (layout.focused_pane_id == self.pane_id
                            || !layout
                                .root
                                .leaves()
                                .iter()
                                .any(|leaf| leaf.id == self.pane_id))
                }))
    }

    pub(crate) fn defer_restore(self, window: &mut Window, cx: &mut Context<NativeRoot>) {
        cx.defer_in(window, move |root, window, cx| {
            root._pane_close_focus_subscription = None;
            if self.restore_remote_if_closed(root, window, cx) {
                return;
            }
            // A remote acknowledgement can precede the overview's new topology.
            let clients = root.app_store.read(cx).remote_clients.clone();
            root._pane_close_focus_subscription =
                Some(cx.observe_in(&clients, window, move |root, _, window, cx| {
                    root.sync_remote_state(cx);
                    if self.restore_remote_if_closed(root, window, cx) {
                        root._pane_close_focus_subscription = None;
                    }
                }));
        });
    }

    fn restore_remote_if_closed(
        &self,
        root: &NativeRoot,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if !self.is_current(root, window, cx) {
            return true;
        }
        if root.remote_client.pane_close_pending(&self.tab_id) {
            return false;
        }
        if root
            .remote_client
            .tab_layout(&self.tab_id)
            .is_some_and(|layout| {
                layout
                    .root
                    .leaves()
                    .iter()
                    .any(|leaf| leaf.id == self.pane_id && leaf.session_id == self.session_id)
            })
        {
            return false;
        }
        root.focus_active_terminal(window, cx);
        true
    }
}

impl NativeRoot {
    pub(crate) fn close_pane(
        &mut self,
        pane_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let remote = self.front_page_client(cx).is_remote();
        let Some(mut draft) = self.front_page_layout().cloned() else {
            return;
        };
        if self.launcher_tab_pending(&draft.id, cx) {
            return;
        }
        let closing_session_id = draft
            .root
            .leaves()
            .into_iter()
            .find(|leaf| leaf.id == pane_id)
            .and_then(|leaf| leaf.session_id.clone());
        let focused = draft.focused_pane_id == pane_id;
        let focus = focused
            .then(|| PaneCloseFocus::capture(self, window, cx))
            .flatten();
        if !draft.close_pane(pane_id) {
            cx.notify();
            return;
        }
        if remote {
            let Some(action) = self.remote_client.prepare_layout(&draft, cx) else {
                return;
            };
            self.remote_client.track_pane_close(&draft);
            let client = action.client.clone();
            let task = cx.background_spawn(async move {
                client
                    .layout(&draft, draft.parent_id.clone())
                    .map_err(|error| error.to_string())?;
                Ok(crate::surfaces::front_page::actions::Completion {
                    layout: Some(draft),
                    ..Default::default()
                })
            });
            cx.spawn_in(window, async move |weak, cx| {
                let result = task.await;
                let _ = weak.update_in(cx, |root, window, cx| {
                    WorkspaceAction::complete_observed(
                        root,
                        action,
                        result,
                        |done| done,
                        |_, _, succeeded, cx| {
                            if succeeded {
                                if let Some(focus) = focus {
                                    focus.defer_restore(window, cx);
                                }
                            }
                        },
                        cx,
                    );
                });
            })
            .detach();
            cx.notify();
            return;
        }
        *self.tabs.active_mut().unwrap() = draft.clone();
        let saved = self.save_workspace_layout(draft, false, cx);
        let result = saved
            .and_then(|_| self.reload_tabs(cx))
            .and_then(|_| self.ensure_active_tab_attached(window, cx))
            .map(|()| true);
        match result {
            Ok(true) => {
                if self.pane_rename.as_ref().is_some_and(|rename| {
                    closing_session_id.as_deref() == Some(rename.session_id.as_str())
                }) {
                    self.pane_rename = None;
                    self._pane_rename_focus_subscription = None;
                }
                self.chat_error = None;
                self.remember_active_role(cx);
                self.mark_active_tab_viewed(window, cx);
                if focused {
                    self.focus_active_terminal(window, cx);
                }
                self.record_current_runtime_location();
            }
            Ok(false) => {}
            Err(error) => self.chat_error = Some(error.to_string().into()),
        }
        cx.notify();
    }

    pub(crate) fn close_terminal_pane(
        &mut self,
        pane_id: &str,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.launcher_session_pending(session_id, cx) {
            return;
        }
        let remote = self.front_page_client(cx).is_remote();
        if !remote
            && self
                .tabs
                .active()
                .is_some_and(|layout| layout.all_session_ids().as_slice() == [session_id])
        {
            self.close_terminal_tab(session_id, window, cx);
            return;
        }
        if !remote {
            self.terminal_close_confirm = None;
            if !self.stopping_sessions.insert(session_id.to_owned()) {
                return;
            }
            self.chat_error = None;
        }
        let Some((operation, close_operation)) =
            WorkspaceAction::close(self, session_id, Some(pane_id), false, cx)
        else {
            return;
        };
        let target = session_id.to_owned();
        let focus = self
            .front_page_layout()
            .filter(|layout| layout.focused_pane_id == pane_id)
            .and_then(|_| PaneCloseFocus::capture(self, window, cx));
        let close = cx.background_spawn(async move { close_operation() });
        let origin_epoch = self.remote_client.ui.epoch;
        cx.spawn_in(window, async move |weak, cx| {
            let result = close.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                let Some(result) = WorkspaceAction::complete_observed(
                    this,
                    operation,
                    result,
                    |done| done,
                    |_, client, succeeded, cx| {
                        if client.is_remote() && succeeded {
                            if let Some(focus) = focus.clone() {
                                focus.defer_restore(window, cx);
                            }
                        }
                    },
                    cx,
                ) else {
                    return;
                };
                let result = result.map(drop);
                if this.remote_client.ui.epoch != origin_epoch {
                    this.stopping_sessions.remove(&target);
                    this.chat_transitions.remove(&target);
                    cx.notify();
                    return;
                }
                this.stopping_sessions.remove(&target);
                match result {
                    Ok(()) => {
                        this.attached.remove(&target);
                        this.session_exit_codes.remove(&target);
                        this.refresh_sessions(cx);
                        // The daemon has already collapsed the shell's pane.
                        let restore_focus = focus
                            .as_ref()
                            .is_some_and(|focus| focus.is_current(this, window, cx));
                        let refresh = this.reload_tabs(cx).and_then(|_| {
                            if this.route == AppRoute::Chat {
                                this.ensure_active_tab_attached(window, cx)?;
                                if restore_focus {
                                    this.settle_active_tab(window, cx);
                                }
                            }
                            Ok(())
                        });
                        if this
                            .pane_rename
                            .as_ref()
                            .is_some_and(|rename| rename.session_id == target)
                        {
                            this.pane_rename = None;
                            this._pane_rename_focus_subscription = None;
                        }
                        if let Err(error) = refresh {
                            this.chat_error = Some(error.to_string().into());
                        }
                    }
                    Err(error) => this.chat_error = Some(error.into()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn request_close_terminal_pane(
        &mut self,
        pane_id: &str,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_terminal_pane(pane_id, session_id, window, cx);
    }

    pub(crate) fn request_close_chat_pane(
        &mut self,
        pane_id: &str,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.launcher_session_pending(session_id, cx)
            || self.sidebar_archiving_session(session_id, cx)
        {
            return;
        }
        if self.remote_client.is_active() {
            let session_ids = self
                .remote_client
                .archive_targets(vec![session_id.to_owned()]);
            if session_ids.len() > 1 {
                self.request_archive_all(None, session_ids, ArchiveAllSource::Chat, window, cx);
                return;
            }
            if let Some(layout) = self.front_page_layout() {
                let target = archive_chat_pane_target(layout, pane_id, session_id);
                TerminalCloseConfirm::open(&mut self.terminal_close_confirm, target, window, cx);
                cx.notify();
            }
            return;
        }

        let session_ids = super::super::sidebar::archive_targets_for_chats(
            vec![session_id.to_owned()],
            self.tabs.tabs(),
        );
        if session_ids.len() > 1 {
            self.request_archive_all(None, session_ids, ArchiveAllSource::Chat, window, cx);
            return;
        }
        let Some(layout) = self.tabs.active().cloned() else {
            return;
        };
        let worktree_options =
            self.emptied_managed_worktrees(std::slice::from_ref(&session_id.to_owned()), &[], cx);
        self.open_archive_confirm(
            archive_chat_pane_target(&layout, pane_id, session_id),
            worktree_options,
            window,
            cx,
        );
    }

    pub(crate) fn request_close_single_pane_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(layout) = self.tabs.active() else {
            return;
        };
        let tab_id = layout.id.clone();
        if self.launcher_tab_pending(&tab_id, cx) {
            return;
        }
        let pane_id = layout.focused_pane_id.clone();
        let session_id = layout.focused_session_id().map(str::to_owned);
        let shell_ids = layout.drawer_shells().to_vec();
        let behavior = tab_close_behavior(
            session_id
                .as_deref()
                .and_then(|id| self.session_entry(id, cx))
                .map(|entry| entry.agent_runtime.as_str()),
            !shell_ids.is_empty(),
        );
        match (behavior, session_id) {
            (TabCloseBehavior::ArchiveChat, Some(session_id)) => {
                self.request_close_chat_pane(&pane_id, &session_id, window, cx);
            }
            (TabCloseBehavior::CloseTerminal, Some(session_id)) => {
                self.request_close_terminal_tab(&tab_id, &session_id, window, cx);
            }
            (TabCloseBehavior::ArchiveDrawerShells, _) => {
                self.request_archive_all(None, shell_ids, ArchiveAllSource::Chat, window, cx);
            }
            (TabCloseBehavior::DeleteEmptyTab, _) => {
                self.prepare_tab_close(&[], cx);
                let result = self
                    .front_page_client(cx)
                    .archive(runner_core::companion::ActionTarget {
                        kind: runner_core::companion::ActionKind::Tab,
                        id: tab_id.clone(),
                    })
                    .map_err(anyhow::Error::from)
                    .and_then(|_| self.reload_tabs(cx))
                    .and_then(|_| self.ensure_active_tab_attached(window, cx));
                self.pending_tab_close = None;
                if let Err(error) = result {
                    self.chat_error = Some(error.to_string().into());
                } else {
                    self.settle_active_tab(window, cx);
                }
                cx.notify();
            }
            (TabCloseBehavior::ArchiveChat | TabCloseBehavior::CloseTerminal, None) => {}
        }
    }

    /// What selecting a tab does beyond attaching it, for a close that moved
    /// the active tab.
    pub(crate) fn settle_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.route == AppRoute::Chat {
            self.remember_active_role(cx);
            self.sync_active_chat_detail(cx);
        }
        self.mark_active_tab_viewed(window, cx);
        self.focus_active_terminal(window, cx);
        self.record_current_runtime_location();
    }

    /// Whether a close comes too soon after a landing to have been aimed at the
    /// tab it landed on.
    pub(crate) fn tab_close_guarded(&self) -> bool {
        let landed = if self.remote_client.is_active() {
            self.remote_client.landed_at()
        } else {
            self.tab_landed_at
        };
        landed.is_some_and(|at| at.elapsed() < TAB_LANDING_CLOSE_GUARD)
    }

    pub(crate) fn prepare_tab_close(&mut self, session_ids: &[String], _cx: &mut Context<Self>) {
        let Some(layout) = self.tabs.active().filter(|layout| {
            layout
                .all_session_ids()
                .iter()
                .all(|id| session_ids.contains(id))
        }) else {
            return;
        };
        let tab_id = layout.id.clone();
        self.pending_tab_close = Some(PendingTabClose { tab_id });
    }

    pub(crate) fn clear_pending_tab_close(&mut self, attempted: &[String]) {
        if self.pending_tab_close.as_ref().is_some_and(|close| {
            self.tabs.tabs().iter().any(|layout| {
                layout.id == close.tab_id
                    && layout
                        .all_session_ids()
                        .iter()
                        .any(|id| attempted.contains(id))
            })
        }) {
            self.pending_tab_close = None;
        }
    }

    pub(crate) fn request_close_terminal_tab(
        &mut self,
        tab_id: &str,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.launcher_tab_pending(tab_id, cx) {
            return;
        }
        self.close_terminal_tab(session_id, window, cx);
    }

    pub(super) fn close_terminal_tab(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.launcher_session_pending(session_id, cx) {
            return;
        }
        let remote = self.front_page_client(cx).is_remote();
        if !remote {
            self.terminal_close_confirm = None;
            if !self.stopping_sessions.insert(session_id.to_owned()) {
                return;
            }
            self.prepare_tab_close(&[session_id.to_owned()], cx);
        }
        let plan = (!remote).then(|| {
            vec![(
                session_id.to_owned(),
                crate::surfaces::front_page::client::ArchiveSessionOperation::CloseTerminal,
            )]
        });
        let Some((operation, close_operation)) =
            WorkspaceAction::archive(self, vec![session_id.to_owned()], plan, None, cx)
        else {
            return;
        };
        let target = session_id.to_owned();
        let close = cx.background_spawn(async move { close_operation() });
        let origin_epoch = self.remote_client.ui.epoch;
        cx.spawn_in(window, async move |weak, cx| {
            let result = close.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                let Some(result) =
                    WorkspaceAction::complete(this, operation, result, |done| done.completion, cx)
                else {
                    return;
                };
                let result = result.and_then(|done| done.error.map_or(Ok(()), Err));
                if this.remote_client.ui.epoch != origin_epoch {
                    this.stopping_sessions.remove(&target);
                    this.chat_transitions.remove(&target);
                    cx.notify();
                    return;
                }
                this.stopping_sessions.remove(&target);
                match result {
                    Ok(()) => {
                        this.attached.remove(&target);
                        this.session_exit_codes.remove(&target);
                        this.refresh_sessions(cx);
                        let refresh = this.reload_tabs(cx).and_then(|_| {
                            if this.route == AppRoute::Chat {
                                this.ensure_active_tab_attached(window, cx)?;
                                this.settle_active_tab(window, cx);
                            }
                            Ok(())
                        });
                        if let Err(error) = refresh {
                            this.error = Some(error.to_string().into());
                        }
                    }
                    Err(error) => this.error = Some(error.into()),
                }
                this.clear_pending_tab_close(std::slice::from_ref(&target));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn request_archive_all(
        &mut self,
        tab_id: Option<&str>,
        session_ids: Vec<String>,
        source: ArchiveAllSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if session_ids.iter().any(|id| {
            self.launcher_session_pending(id, cx) || self.sidebar_archiving_session(id, cx)
        }) {
            return;
        }
        let has_linked_missions = !self.linked_missions_for_chats(&session_ids, cx).is_empty();
        if self.remote_client.is_active() {
            let terminals = session_ids
                .iter()
                .filter(|id| {
                    self.front_page_session(id, cx)
                        .is_some_and(|session| session.agent_runtime == "shell")
                })
                .count();
            let confirmation_body = super::super::sidebar::archive_confirmation_counts(
                session_ids.len() - terminals,
                terminals,
            );
            if confirmation_body.is_none() && !has_linked_missions {
                self.remote_archive_sessions(session_ids, cx);
                return;
            }
            if let Some(tab) = tab_id {
                self.select_remote_item(
                    crate::surfaces::remote_client::RemoteItem::Tab(tab.into()),
                    cx,
                );
            }
            TerminalCloseConfirm::open(
                &mut self.terminal_close_confirm,
                TerminalCloseTarget::ArchiveAll {
                    session_ids,
                    source,
                    confirmation_body: confirmation_body.unwrap_or_else(|| "".into()),
                },
                window,
                cx,
            );
            cx.notify();
            return;
        }

        let confirmation_body = super::super::sidebar::archive_all_confirmation_body(
            &session_ids,
            &self.app_store.read(cx).sessions,
        );
        let worktree_options = self.emptied_managed_worktrees(&session_ids, &[], cx);
        if confirmation_body.is_none() && !has_linked_missions && worktree_options.is_empty() {
            self.archive_all_sessions(session_ids, source, window, cx);
            return;
        }
        if let Some(tab_id) = tab_id {
            self.tabs.activate(tab_id);
            self.set_route(AppRoute::Chat, cx);
            if let Err(error) = self.ensure_active_tab_attached(window, cx) {
                self.chat_error = Some(error.to_string().into());
            }
        }
        self.open_archive_confirm(
            TerminalCloseTarget::ArchiveAll {
                session_ids,
                source,
                confirmation_body: confirmation_body.unwrap_or_else(|| "".into()),
            },
            worktree_options,
            window,
            cx,
        );
    }

    pub(crate) fn confirm_terminal_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(confirm) = self.terminal_close_confirm.take() else {
            return;
        };
        let worktree_removals = super::super::sidebar::checked_worktrees_to_remove(&confirm);
        confirm.restore_focus(window, cx);
        match confirm.target {
            TerminalCloseTarget::ArchiveChatPane { session_id, close } => {
                self.archive_workspace_sessions_with_worktrees(
                    vec![session_id],
                    ArchiveAllSource::Chat,
                    Some(close),
                    worktree_removals,
                    window,
                    cx,
                );
            }
            TerminalCloseTarget::ArchiveAll {
                session_ids,
                source,
                ..
            } => self.archive_workspace_sessions_with_worktrees(
                session_ids,
                source,
                None,
                worktree_removals,
                window,
                cx,
            ),
            TerminalCloseTarget::ArchiveMission { mission_id } => {
                self.execute_archive_mission(&mission_id, worktree_removals, window, cx);
            }
        }
    }

    pub(crate) fn cancel_terminal_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(confirm) = self.terminal_close_confirm.take() {
            confirm.restore_focus(window, cx);
        }
        cx.notify();
    }
}

#[cfg(test)]
#[path = "close_tests.rs"]
mod tests;
