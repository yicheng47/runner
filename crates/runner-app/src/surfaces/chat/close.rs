use super::*;

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
        if !draft.close_pane(pane_id) {
            cx.notify();
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
                self.focus_active_terminal(window, cx);
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
        let pane_id = pane_id.to_owned();
        let close = cx.background_spawn(async move { close_operation() });
        let origin_epoch = self.remote_client.ui.epoch;
        cx.spawn_in(window, async move |weak, cx| {
            let result = close.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                let Some(result) =
                    WorkspaceAction::complete(this, operation, result, |done| done, cx)
                else {
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
                        this.close_pane(&pane_id, window, cx);
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
        if self.launcher_session_pending(session_id, cx) {
            return;
        }
        if self.remote_client.is_active() {
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
        let Some(layout) = self.tabs.active() else {
            return;
        };
        TerminalCloseConfirm::open(
            &mut self.terminal_close_confirm,
            archive_chat_pane_target(layout, pane_id, session_id),
            window,
            cx,
        );
        cx.notify();
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
        if self.remote_client.is_active() {
            if let Some(tab) = tab_id {
                self.select_remote_item(
                    crate::surfaces::remote_client::RemoteItem::Tab(tab.into()),
                    cx,
                );
            }
            let terminals = session_ids
                .iter()
                .filter(|id| {
                    self.front_page_session(id, cx)
                        .is_some_and(|session| session.agent_runtime == "shell")
                })
                .count();
            let Some(confirmation_body) = super::super::sidebar::archive_confirmation_counts(
                session_ids.len() - terminals,
                terminals,
            ) else {
                self.remote_archive_sessions(session_ids, cx);
                return;
            };
            TerminalCloseConfirm::open(
                &mut self.terminal_close_confirm,
                TerminalCloseTarget::ArchiveAll {
                    session_ids,
                    source,
                    confirmation_body,
                },
                window,
                cx,
            );
            cx.notify();
            return;
        }

        let Some(confirmation_body) = super::super::sidebar::archive_all_confirmation_body(
            &session_ids,
            &self.app_store.read(cx).sessions,
        ) else {
            self.archive_all_sessions(session_ids, source, window, cx);
            return;
        };
        if let Some(tab_id) = tab_id {
            self.tabs.activate(tab_id);
            self.set_route(AppRoute::Chat, cx);
            if let Err(error) = self.ensure_active_tab_attached(window, cx) {
                self.chat_error = Some(error.to_string().into());
            }
        }
        TerminalCloseConfirm::open(
            &mut self.terminal_close_confirm,
            TerminalCloseTarget::ArchiveAll {
                session_ids,
                source,
                confirmation_body,
            },
            window,
            cx,
        );
        cx.notify();
    }

    pub(crate) fn confirm_terminal_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(confirm) = self.terminal_close_confirm.take() else {
            return;
        };
        confirm.restore_focus(window, cx);
        match confirm.target {
            TerminalCloseTarget::ArchiveChatPane { session_id, close } => {
                self.archive_workspace_sessions(
                    vec![session_id],
                    ArchiveAllSource::Chat,
                    Some(close),
                    window,
                    cx,
                );
            }
            TerminalCloseTarget::ArchiveAll {
                session_ids,
                source,
                ..
            } => self.archive_all_sessions(session_ids, source, window, cx),
        }
    }

    pub(crate) fn cancel_terminal_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(confirm) = self.terminal_close_confirm.take() {
            confirm.restore_focus(window, cx);
        }
        cx.notify();
    }
}
