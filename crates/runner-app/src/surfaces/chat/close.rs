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
mod tests {
    use super::*;
    use crate::surfaces::start_chat::{tests::modal_harness, ChatMode};
    use runner_core::protocol::{
        ClientError, DaemonClient, EventSubscription, Request, Response, Transport,
    };

    struct CloseAlreadyApplied(runner_daemon::daemon::InProcessTransport);

    impl Transport for CloseAlreadyApplied {
        fn call(&self, request: Request) -> std::result::Result<Response, ClientError> {
            if matches!(request, Request::session_close { .. }) {
                return Ok(Response::session_close(Ok(())));
            }
            self.0.call(request)
        }

        fn subscribe(&self) -> Box<dyn EventSubscription> {
            self.0.subscribe()
        }
    }

    #[test]
    fn shell_close_restores_native_focus_after_the_daemon_collapses_the_pane() {
        check_async_close("shell", false);
    }

    #[test]
    fn chat_archive_restores_native_focus_after_collapse_and_respects_navigation() {
        check_async_close("codex", false);
    }

    #[test]
    fn sidebar_archive_settles_native_focus_after_exit_and_preserves_navigation() {
        check_async_close("codex", true);
    }

    fn check_async_close(runtime: &str, archive_all: bool) {
        for variant in [
            "preapplied",
            "exit-first",
            "exit-first-other-tab",
            "exit-first-other-input",
            "exit-first-other-pane",
            "focused",
            "unfocused",
            "other-tab",
            "other-input",
            "other-pane",
        ] {
            if archive_all
                && !matches!(
                    variant,
                    "exit-first" | "exit-first-other-tab" | "exit-first-other-input"
                )
            {
                continue;
            }
            if runtime != "shell" && variant == "preapplied" {
                continue;
            }
            let exit_first = variant.starts_with("exit-first");
            let navigation = variant.strip_prefix("exit-first-").unwrap_or(variant);
            let close_focused = variant != "unfocused";
            let mut harness =
                modal_harness(1400., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
            harness.act(|root, window, cx| {
                root.start_chat_modal = None;
                let core = root.app_store.read(cx).test_core.clone();
                let conn = core.db.get().unwrap();
                conn.execute("INSERT INTO sessions(id,status,agent_runtime,agent_command) VALUES ('closing','stopped',?1,'/bin/sh'),('surviving','running','shell','/bin/sh'),('elsewhere','running','shell','/bin/sh')", [runtime]).unwrap();
                drop(conn);
                let events: Arc<dyn runner_daemon::session::manager::SessionEvents> = Arc::new(core.session_events());
                if exit_first {
                    core.db.get().unwrap().execute("UPDATE sessions SET status = 'running' WHERE id = 'closing'", []).unwrap();
                }
                for id in ["surviving", "elsewhere"].into_iter().chain(exit_first.then_some("closing")) {
                    core.sessions.prepare_unlisted_terminal(id, (80, 24), &core.db, &events).unwrap();
                    root.app_store.update(cx, |store, _| store.bridge.attach(id).unwrap());
                }
                let mut layout = PaneLayout::single(Some("closing"), &[]);
                layout.id = "01ARZ3NDEKTSV4RRFFQ69G5FAV".into();
                let survivor = layout.split("p1", SplitOrientation::Row).unwrap();
                layout.assign_session(&survivor, "surviving").unwrap();
                if navigation == "other-pane" {
                    let third = layout.split(&survivor, SplitOrientation::Column).unwrap();
                    layout.assign_session(&third, "elsewhere").unwrap();
                }
                let mut other = PaneLayout::single(Some("elsewhere"), &[]);
                other.id = "01ARZ3NDEKTSV4RRFFQ69G5FAX".into();
                other.split("p1", SplitOrientation::Row).unwrap();
                other.focus_pane("p1");
                if archive_all || navigation == "other-tab" {
                    root.core(cx).node_tab_upsert(other.upsert_input().unwrap()).unwrap();
                }
                layout.focus_pane(if close_focused { "p1" } else { &survivor });
                root.core(cx).node_tab_upsert(layout.upsert_input().unwrap()).unwrap();
                root.refresh_sessions(cx);
                root.reload_tabs(cx).unwrap();
                root.tabs.activate(&layout.id);
                root.route = AppRoute::Chat;
                root.ensure_active_tab_attached(window, cx).unwrap();
                root.focus_active_terminal(window, cx);
            });
            let mut input_focus = None;
            harness.act(|root, window, cx| {
                if variant == "preapplied" {
                    let core = root.app_store.read(cx).test_core.clone();
                    runner_daemon::ops::session::session_close(&core, "closing").unwrap();
                    root.app_store.update(cx, |store, _| {
                        store.client = DaemonClient::new(Arc::new(CloseAlreadyApplied(
                            runner_daemon::daemon::InProcessTransport(core),
                        )));
                    });
                }
                if archive_all {
                    root.archive_all_sessions(
                        vec!["closing".into(), "surviving".into()],
                        ArchiveAllSource::Sidebar,
                        window,
                        cx,
                    );
                } else if runtime == "shell" {
                    root.close_terminal_pane("p1", "closing", window, cx);
                    if variant == "preapplied" {
                        root.reload_tabs(cx).unwrap();
                        assert!(!root.attached["surviving"].terminal_focus.is_focused(window));
                    }
                } else {
                    let before = window.focused(cx);
                    root.request_close_chat_pane("p1", "closing", window, cx);
                    assert!(root.terminal_close_confirm.is_some());
                    root.cancel_terminal_close(window, cx);
                    assert_eq!(window.focused(cx), before);
                    root.request_close_chat_pane("p1", "closing", window, cx);
                    root.confirm_terminal_close(window, cx);
                }
                match navigation {
                    "other-tab" => root.activate_tab("01ARZ3NDEKTSV4RRFFQ69G5FAX", window, cx),
                    "other-input" => {
                        if archive_all {
                            root.activate_tab("01ARZ3NDEKTSV4RRFFQ69G5FAX", window, cx);
                        }
                        let find = root.attached[if archive_all {
                            "elsewhere"
                        } else {
                            "surviving"
                        }]
                        .terminal_find
                        .clone();
                        find.update(cx, |find, cx| find.open(window, cx));
                    }
                    "other-pane" => {
                        let pane = root
                            .tabs
                            .active()
                            .unwrap()
                            .root
                            .leaves()
                            .into_iter()
                            .find(|leaf| leaf.session_id.as_deref() == Some("elsewhere"))
                            .unwrap()
                            .id
                            .clone();
                        root.focus_terminal(&pane, "elsewhere", window, cx);
                    }
                    _ => {}
                }
                if navigation == "other-input" {
                    input_focus = window.focused(cx);
                }
                if exit_first {
                    if navigation == "exit-first" {
                        assert!(root.attached["closing"].terminal_focus.is_focused(window));
                    }
                    let before_exit = window.focused(cx);
                    root.app_store
                        .read(cx)
                        .test_core
                        .db
                        .get()
                        .unwrap()
                        .execute(
                            "UPDATE sessions SET status = 'stopped' WHERE id = 'closing'",
                            [],
                        )
                        .unwrap();
                    root.handle_chat_lifecycle_event(
                        runner_core::protocol::ClientEvent {
                            name: "session/exit".into(),
                            payload: serde_json::json!({"session_id":"closing", "exit_code":0}),
                        },
                        window,
                        cx,
                    );
                    assert_eq!(
                        window.focused(cx),
                        before_exit,
                        "{runtime}/{variant}: exit stole focus"
                    );
                }
            });
            harness.act(|root, window, cx| {
                assert!(!root.stopping_sessions.contains("closing"));
                assert!(root.pending_pane_closes.is_empty());
                if archive_all {
                    assert_eq!(
                        root.tabs.active_tab_id(),
                        Some("01ARZ3NDEKTSV4RRFFQ69G5FAX")
                    );
                    assert_eq!(root.tabs.tabs().len(), 1);
                    if navigation == "other-input" {
                        assert_eq!(window.focused(cx), input_focus);
                    } else {
                        assert!(root.attached["elsewhere"].terminal_focus.is_focused(window));
                    }
                    return;
                }
                if navigation == "other-tab" {
                    assert_eq!(
                        root.tabs.active_tab_id(),
                        Some("01ARZ3NDEKTSV4RRFFQ69G5FAX")
                    );
                    assert_eq!(root.tabs.active().unwrap().root.leaves().len(), 2);
                    assert!(root.attached["elsewhere"].terminal_focus.is_focused(window));
                    return;
                }
                assert_eq!(
                    root.tabs.active().unwrap().root.leaves().len(),
                    if navigation == "other-pane" { 2 } else { 1 }
                );
                if navigation == "other-input" {
                    assert_eq!(window.focused(cx), input_focus);
                    return;
                }
                let survivor = if navigation == "other-pane" {
                    "elsewhere"
                } else {
                    "surviving"
                };
                assert!(
                    root.attached[survivor].terminal_focus.is_focused(window),
                    "{runtime}/{variant}"
                );
            });
            if variant == "exit-first" {
                harness.visual.simulate_keystrokes("x");
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                let mut error = None;
                while error.is_none() {
                    harness.act(|root, _, _| error = root.chat_error.clone());
                    assert!(
                        std::time::Instant::now() < deadline,
                        "input did not reach the survivor"
                    );
                    std::thread::yield_now();
                }
                let target = if archive_all {
                    "elsewhere"
                } else {
                    "surviving"
                };
                assert_eq!(error, Some(format!("session not found: {target}").into()));
            }
        }
    }

    fn git_run(dir: &std::path::Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git command");
        assert!(
            out.status.success(),
            "git {:?} in {} failed: {}",
            args,
            dir.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    fn git_branch_exists(repo: &std::path::Path, branch: &str) -> bool {
        std::process::Command::new("git")
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ])
            .current_dir(repo)
            .status()
            .is_ok_and(|s| s.success())
    }

    fn init_archive_git_repo(root: &std::path::Path) -> std::path::PathBuf {
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git_run(&repo, &["init", "-b", "main"]);
        git_run(&repo, &["config", "user.name", "Runner Test"]);
        git_run(&repo, &["config", "user.email", "test@runner.local"]);
        std::fs::write(repo.join("README.md"), "# test\n").unwrap();
        git_run(&repo, &["add", "."]);
        git_run(&repo, &["commit", "-m", "initial"]);
        repo
    }

    fn wait_for_condition<F: FnMut() -> bool>(
        visual: &gpui::VisualTestContext,
        description: &str,
        mut predicate: F,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            visual.run_until_parked();
            if predicate() {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {description}"
            );
            std::thread::yield_now();
        }
    }

    fn wait_for_confirm_worktrees(
        harness: &mut crate::surfaces::start_chat::tests::ModalHarness,
        expected_len: usize,
        mut ready: impl FnMut(&[crate::surfaces::sidebar::WorktreeRemoveOption]) -> bool,
    ) -> Vec<crate::surfaces::sidebar::WorktreeRemoveOption> {
        let mut snapshot = Vec::new();
        let mut has_confirm = false;
        let visual = harness.visual.clone();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            visual.run_until_parked();
            harness.act(|root, _, _| {
                has_confirm = root.terminal_close_confirm.is_some();
                snapshot = root
                    .terminal_close_confirm
                    .as_ref()
                    .map(|confirm| confirm.worktree_options.clone())
                    .unwrap_or_default();
            });
            if snapshot.len() == expected_len && ready(&snapshot) {
                return snapshot;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for worktree archive options: has_confirm={has_confirm}, expected_len={expected_len}, snapshot={snapshot:?}"
            );
            std::thread::yield_now();
        }
    }

    struct RefuseWorktreeRemoveTransport(runner_daemon::daemon::InProcessTransport);

    impl Transport for RefuseWorktreeRemoveTransport {
        fn call(&self, request: Request) -> std::result::Result<Response, ClientError> {
            if matches!(request, Request::worktree_remove { .. }) {
                return Ok(Response::worktree_remove(Err(ClientError::msg(
                    "refused by hook",
                ))));
            }
            self.0.call(request)
        }

        fn subscribe(&self) -> Box<dyn EventSubscription> {
            self.0.subscribe()
        }
    }

    #[gpui::test]
    fn archive_worktree_clean_and_dirty_options_and_branch_retention(
        _cx: &mut gpui::TestAppContext,
    ) {
        let temp = tempfile::Builder::new()
            .prefix("1213-m2-archive-cd-")
            .tempdir()
            .unwrap();
        let repo = init_archive_git_repo(temp.path());
        let clean_wt = repo.join(".worktrees/clean-branch");
        let dirty_wt = repo.join(".worktrees/dirty-branch");
        git_run(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "clean-branch",
                clean_wt.to_str().unwrap(),
            ],
        );
        git_run(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "dirty-branch",
                dirty_wt.to_str().unwrap(),
            ],
        );
        std::fs::write(dirty_wt.join("uncommitted.txt"), "dirty\n").unwrap();

        let mut harness = modal_harness(1400., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        harness.act(|root, _, cx| {
            root.start_chat_modal = None;
            let core = root.app_store.read(cx).test_core.clone();
            let conn = core.db.get().unwrap();
            let project =
                runner_daemon::repo::project::create(&conn, "ArchiveRepo", &repo.to_string_lossy())
                    .unwrap();
            conn.execute(
                "INSERT INTO sessions(id,title,status,agent_runtime,agent_command,project_id,cwd)
                 VALUES ('dirty-chat','Dirty chat','stopped','codex','codex',?1,?2),
                        ('clean-chat','Clean chat','stopped','codex','codex',?1,?3)",
                [
                    project.id.as_str(),
                    dirty_wt.to_string_lossy().as_ref(),
                    clean_wt.to_string_lossy().as_ref(),
                ],
            )
            .unwrap();
            let mut clean_tab = PaneLayout::single(Some("clean-chat"), &[]);
            clean_tab.id = "01ARZ3NDEKTSV4RRFFQ69G5FAV".into();
            root.core(cx)
                .node_tab_upsert(clean_tab.upsert_input().unwrap())
                .unwrap();
            let mut dirty_tab = PaneLayout::single(Some("dirty-chat"), &[]);
            dirty_tab.id = "01ARZ3NDEKTSV4RRFFQ69G5FAW".into();
            root.core(cx)
                .node_tab_upsert(dirty_tab.upsert_input().unwrap())
                .unwrap();
            root.refresh_sessions(cx);
            root.reload_tabs(cx).unwrap();
            root.app_store.update(cx, |store, cx| {
                store.refresh(crate::app_store::StoreRefreshKind::All, cx);
                store.sync_worktree_checkouts(cx);
            });
        });

        let mut visual = harness.visual.clone();
        wait_for_condition(&visual, "worktree checkouts synced", || {
            let mut synced = false;
            harness.act(|root, _, cx| {
                let store = root.app_store.read(cx);
                synced = store
                    .worktree_checkout_for(dirty_wt.to_str().unwrap(), None)
                    .is_some()
                    && store
                        .worktree_checkout_for(clean_wt.to_str().unwrap(), None)
                        .is_some();
            });
            synced
        });

        // 1. Dirty worktree cannot be removed; confirm keeps directory and branch intact.
        harness.act(|root, window, cx| {
            root.tabs.activate("01ARZ3NDEKTSV4RRFFQ69G5FAW");
            root.route = AppRoute::Chat;
            root.request_close_chat_pane("p1", "dirty-chat", window, cx);
        });
        let options = wait_for_confirm_worktrees(&mut harness, 1, |opts| {
            matches!(
                opts[0].state,
                crate::surfaces::sidebar::WorktreeRemoveState::Dirty { uncommitted }
                    if uncommitted > 0
            )
        });
        assert_eq!(options[0].branch, "dirty-branch");
        visual.run_until_parked();
        assert!(visual
            .debug_bounds("CONFIRM_WORKTREE_DIRTY dirty-branch")
            .is_some());
        harness.act(|root, window, cx| {
            root.toggle_confirm_worktree(0, cx);
            assert!(matches!(
                root.terminal_close_confirm
                    .as_ref()
                    .unwrap()
                    .worktree_options[0]
                    .state,
                crate::surfaces::sidebar::WorktreeRemoveState::Dirty { .. }
            ));
            root.confirm_terminal_close(window, cx);
        });
        wait_for_condition(&visual, "dirty-chat archived", || {
            let mut archived = false;
            harness.act(|root, _, cx| {
                let core = root.app_store.read(cx).test_core.clone();
                let row =
                    runner_daemon::repo::session::get_row(&core.db.get().unwrap(), "dirty-chat")
                        .unwrap()
                        .unwrap();
                archived = row.archived_at.is_some();
            });
            archived
        });
        assert!(dirty_wt.exists());
        assert!(git_branch_exists(&repo, "dirty-branch"));

        // 2. Clean worktree defaults checked, toggles cleanly, and removes folder while keeping branch.
        harness.act(|root, window, cx| {
            root.tabs.activate("01ARZ3NDEKTSV4RRFFQ69G5FAV");
            root.route = AppRoute::Chat;
            root.request_close_chat_pane("p1", "clean-chat", window, cx);
        });
        let options = wait_for_confirm_worktrees(&mut harness, 1, |opts| {
            matches!(
                opts[0].state,
                crate::surfaces::sidebar::WorktreeRemoveState::Clean { checked: true }
            )
        });
        assert_eq!(options[0].branch, "clean-branch");
        visual.run_until_parked();
        assert!(visual
            .debug_bounds("CONFIRM_WORKTREE_CLEAN clean-branch true")
            .is_some());
        harness.act(|root, window, cx| {
            root.toggle_confirm_worktree(0, cx);
            assert!(matches!(
                root.terminal_close_confirm
                    .as_ref()
                    .unwrap()
                    .worktree_options[0]
                    .state,
                crate::surfaces::sidebar::WorktreeRemoveState::Clean { checked: false }
            ));
            root.toggle_confirm_worktree(0, cx);
            root.confirm_terminal_close(window, cx);
        });
        wait_for_condition(&visual, "clean-chat archived and worktree removed", || {
            let mut done = false;
            harness.act(|root, _, cx| {
                let core = root.app_store.read(cx).test_core.clone();
                let row =
                    runner_daemon::repo::session::get_row(&core.db.get().unwrap(), "clean-chat")
                        .unwrap()
                        .unwrap();
                done = row.archived_at.is_some() && !clean_wt.exists();
            });
            done
        });
        assert!(!clean_wt.exists());
        assert!(git_branch_exists(&repo, "clean-branch"));
    }

    #[gpui::test]
    fn archive_worktree_linked_missions_outside_and_shared_worktree_and_mission_archive(
        _cx: &mut gpui::TestAppContext,
    ) {
        let temp = tempfile::Builder::new()
            .prefix("1213-m2-archive-multi-")
            .tempdir()
            .unwrap();
        let repo = init_archive_git_repo(temp.path());
        let m1_wt = repo.join(".worktrees/m1-branch");
        let m2_wt = repo.join(".worktrees/m2-branch");
        let shared_wt = repo.join(".worktrees/shared-branch");
        let solo_wt = repo.join(".worktrees/solo-branch");
        let outside_wt = temp.path().join("outside-wt");
        for (path, branch) in [
            (&m1_wt, "m1-branch"),
            (&m2_wt, "m2-branch"),
            (&shared_wt, "shared-branch"),
            (&solo_wt, "solo-branch"),
            (&outside_wt, "outside-branch"),
        ] {
            git_run(
                &repo,
                &["worktree", "add", "-b", branch, path.to_str().unwrap()],
            );
        }

        let mut harness = modal_harness(1400., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        harness.act(|root, _, cx| {
            root.start_chat_modal = None;
            let core = root.app_store.read(cx).test_core.clone();
            let conn = core.db.get().unwrap();
            let project = runner_daemon::repo::project::create(
                &conn,
                "MultiRepo",
                &repo.to_string_lossy(),
            )
            .unwrap();
            conn.execute_batch(
                "INSERT INTO crews (id, name, created_at, updated_at)
                 VALUES ('crew', 'Crew', '2026-11-17T00:00:00Z', '2026-11-17T00:00:00Z');",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions(id,title,status,agent_runtime,agent_command,project_id,cwd)
                 VALUES ('chat-parent','Parent','stopped','codex','codex',?1,?2),
                        ('chat-survivor','Survivor','stopped','codex','codex',?1,?2)",
                [project.id.as_str(), shared_wt.to_string_lossy().as_ref()],
            )
            .unwrap();
            for (id, cwd, linked) in [
                ("m1", &m1_wt, Some("chat-parent")),
                ("m2", &m2_wt, Some("chat-parent")),
                ("m3", &outside_wt, Some("chat-parent")),
                ("solo-m", &solo_wt, None),
            ] {
                let cwd_str = cwd.to_string_lossy();
                conn.execute(
                    "INSERT INTO missions(id,crew_id,project_id,title,status,cwd,started_at,linked_session_id)
                     VALUES (?1,'crew',?2,?1,'completed',?3,'2026-11-17T00:00:00Z',?4)",
                    [
                        Some(id),
                        Some(project.id.as_str()),
                        Some(cwd_str.as_ref()),
                        linked,
                    ],
                )
                .unwrap();
                runner_daemon::repo::node::ensure_mission_node(&conn, id, Some(&project.id))
                    .unwrap();
            }
            let project_node =
                runner_daemon::repo::node::ensure_project_node(&conn, &project.id).unwrap();
            drop(conn);
            let mut parent_tab = PaneLayout::single(Some("chat-parent"), &[]);
            parent_tab.id = "01ARZ3NDEKTSV4RRFFQ69G5FAX".into();
            let mut input = parent_tab.upsert_input().unwrap();
            input.parent_id = Some(project_node.id);
            root.core(cx).node_tab_upsert(input).unwrap();
            root.refresh_sessions(cx);
            root.reload_tabs(cx).unwrap();
            root.app_store.update(cx, |store, cx| {
                store.refresh_nodes(cx).unwrap();
                store.refresh(crate::app_store::StoreRefreshKind::All, cx);
                store.sync_worktree_checkouts(cx);
            });
        });

        let visual = harness.visual.clone();
        wait_for_condition(&visual, "all five checkouts cached", || {
            let mut ready = false;
            harness.act(|root, _, cx| {
                let store = root.app_store.read(cx);
                let has_missions = store.missions.len() == 4
                    && store
                        .nodes
                        .iter()
                        .any(|n| n.ref_id.as_deref() == Some("m1"));
                let has_checkouts = [&m1_wt, &m2_wt, &shared_wt, &solo_wt, &outside_wt]
                    .into_iter()
                    .all(|path| {
                        store
                            .worktree_checkout_for(path.to_str().unwrap(), None)
                            .is_some()
                    });
                ready = has_missions && has_checkouts;
            });
            ready
        });

        // Archiving chat-parent includes linked missions m1 & m2 (2 rows), omits outside-wt and shared-wt.
        harness.act(|root, window, cx| {
            root.tabs.activate("01ARZ3NDEKTSV4RRFFQ69G5FAX");
            root.route = AppRoute::Chat;
            root.request_close_chat_pane("p1", "chat-parent", window, cx);
        });
        let options = wait_for_confirm_worktrees(&mut harness, 2, |opts| {
            opts.iter().all(|opt| {
                matches!(
                    opt.state,
                    crate::surfaces::sidebar::WorktreeRemoveState::Clean { checked: true }
                )
            })
        });
        let branches = options
            .iter()
            .map(|opt| opt.branch.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(branches, ["m1-branch", "m2-branch"].into_iter().collect());
        harness.act(|root, window, cx| root.confirm_terminal_close(window, cx));
        wait_for_condition(
            &visual,
            "chat-parent and linked mission worktrees removed",
            || !m1_wt.exists() && !m2_wt.exists(),
        );
        assert!(outside_wt.exists());
        assert!(shared_wt.exists());
        assert!(git_branch_exists(&repo, "m1-branch"));
        assert!(git_branch_exists(&repo, "m2-branch"));

        // Direct mission archive removes clean solo worktree while keeping its branch.
        harness.act(|root, window, cx| {
            root.request_archive_mission("solo-m".into(), window, cx);
        });
        let solo_options = wait_for_confirm_worktrees(&mut harness, 1, |opts| {
            matches!(
                opts[0].state,
                crate::surfaces::sidebar::WorktreeRemoveState::Clean { checked: true }
            )
        });
        assert_eq!(solo_options[0].branch, "solo-branch");
        harness.act(|root, window, cx| root.confirm_terminal_close(window, cx));
        wait_for_condition(&visual, "solo-m archived and worktree removed", || {
            let mut done = false;
            harness.act(|root, _, cx| {
                let core = root.app_store.read(cx).test_core.clone();
                let mission = runner_daemon::repo::mission::get(&core.db.get().unwrap(), "solo-m")
                    .unwrap()
                    .unwrap();
                done = mission.archived_at.is_some() && !solo_wt.exists();
            });
            done
        });
        assert!(git_branch_exists(&repo, "solo-branch"));
    }

    #[gpui::test]
    fn archive_worktree_git_refusal_keeps_archive_and_shows_toast(_cx: &mut gpui::TestAppContext) {
        let temp = tempfile::Builder::new()
            .prefix("1213-m2-archive-toast-")
            .tempdir()
            .unwrap();
        let repo = init_archive_git_repo(temp.path());
        let refuse_wt = repo.join(".worktrees/refuse-branch");
        git_run(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "refuse-branch",
                refuse_wt.to_str().unwrap(),
            ],
        );

        let mut harness = modal_harness(1400., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        harness.act(|root, _, cx| {
            root.start_chat_modal = None;
            let core = root.app_store.read(cx).test_core.clone();
            let conn = core.db.get().unwrap();
            let project = runner_daemon::repo::project::create(
                &conn,
                "RefuseRepo",
                &repo.to_string_lossy(),
            )
            .unwrap();
            conn.execute_batch(
                "INSERT INTO crews (id, name, created_at, updated_at)
                 VALUES ('crew', 'Crew', '2026-11-17T00:00:00Z', '2026-11-17T00:00:00Z');",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO missions(id,crew_id,project_id,title,status,cwd,started_at)
                 VALUES ('m-refuse','crew',?1,'Refused mission','completed',?2,'2026-11-17T00:00:00Z')",
                [project.id.as_str(), refuse_wt.to_string_lossy().as_ref()],
            )
            .unwrap();
            runner_daemon::repo::node::ensure_mission_node(&conn, "m-refuse", Some(&project.id))
                .unwrap();
            root.app_store.update(cx, |store, cx| {
                store.client = DaemonClient::new(Arc::new(RefuseWorktreeRemoveTransport(
                    runner_daemon::daemon::InProcessTransport(core),
                )));
                store.refresh(crate::app_store::StoreRefreshKind::All, cx);
                store.sync_worktree_checkouts(cx);
            });
        });

        let visual = harness.visual.clone();
        wait_for_condition(&visual, "refuse-branch checkout synced", || {
            let mut synced = false;
            harness.act(|root, _, cx| {
                synced = root
                    .app_store
                    .read(cx)
                    .worktree_checkout_for(refuse_wt.to_str().unwrap(), None)
                    .is_some();
            });
            synced
        });

        harness.act(|root, window, cx| {
            root.request_archive_mission("m-refuse".into(), window, cx);
        });
        wait_for_confirm_worktrees(&mut harness, 1, |opts| {
            matches!(
                opts[0].state,
                crate::surfaces::sidebar::WorktreeRemoveState::Clean { checked: true }
            )
        });
        harness.act(|root, window, cx| root.confirm_terminal_close(window, cx));

        wait_for_condition(
            &visual,
            "archive completes and error toast displays",
            || {
                let mut toast_shown = false;
                harness.act(|root, _, cx| {
                    let core = root.app_store.read(cx).test_core.clone();
                    let archived =
                        runner_daemon::repo::mission::get(&core.db.get().unwrap(), "m-refuse")
                            .unwrap()
                            .is_some_and(|m| m.archived_at.is_some());
                    if archived && let Some(toast) = root.toasts.active() {
                        let msg = toast.message.as_str();
                        toast_shown = toast.tone == crate::toast::ToastTone::Error
                            && msg.contains("refuse-branch")
                            && msg.contains("refused by hook");
                    }
                });
                toast_shown
            },
        );
        assert!(refuse_wt.exists());
    }

    #[gpui::test]
    fn archive_confirm_monotonic_id_ignores_stale_reply_and_terminal_close_never_prompts(
        _cx: &mut gpui::TestAppContext,
    ) {
        let temp = tempfile::Builder::new()
            .prefix("1213-m2-r2-")
            .tempdir()
            .unwrap();
        let repo = init_archive_git_repo(temp.path());
        let wt = repo.join(".worktrees/r2-branch");
        let term_wt = repo.join(".worktrees/r2-term-wt");
        git_run(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "r2-branch",
                wt.to_str().unwrap(),
            ],
        );
        git_run(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "r2-term",
                term_wt.to_str().unwrap(),
            ],
        );

        let mut harness = modal_harness(1400., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let visual = harness.visual.clone();

        harness.act(|root, _, cx| {
            let core = root.app_store.read(cx).test_core.clone();
            let project = runner_daemon::ops::project::project_create_checked(
                &core,
                "R2 Repo".into(),
                repo.to_string_lossy().into_owned(),
            )
            .unwrap();
            let conn = core.db.get().unwrap();
            for (sid, cwd, runtime, command) in [
                (
                    "chat-r2",
                    wt.to_string_lossy().into_owned(),
                    "codex",
                    "codex",
                ),
                (
                    "term-r2",
                    term_wt.to_string_lossy().into_owned(),
                    "shell",
                    "/bin/sh",
                ),
            ] {
                conn.execute(
                    "INSERT INTO sessions(id, status, agent_runtime, agent_command, project_id, cwd)
                     VALUES (?1, 'stopped', ?2, ?3, ?4, ?5)",
                    [sid, runtime, command, project.id.as_str(), cwd.as_str()],
                )
                .unwrap();
            }
            let project_node =
                runner_daemon::repo::node::ensure_project_node(&conn, &project.id).unwrap();
            drop(conn);
            let mut chat_tab = PaneLayout::single(Some("chat-r2"), &[]);
            chat_tab.id = "01ARZ3NDEKTSV4RRFFQ69G5FB1".into();
            let mut chat_input = chat_tab.upsert_input().unwrap();
            chat_input.parent_id = Some(project_node.id.clone());
            root.core(cx).node_tab_upsert(chat_input).unwrap();

            let mut term_tab = PaneLayout::single(Some("term-r2"), &[]);
            term_tab.id = "01ARZ3NDEKTSV4RRFFQ69G5FB2".into();
            let mut term_input = term_tab.upsert_input().unwrap();
            term_input.parent_id = Some(project_node.id);
            root.core(cx).node_tab_upsert(term_input).unwrap();

            root.app_store.update(cx, |store, cx| {
                store.refresh(crate::app_store::StoreRefreshKind::All, cx);
                store.refresh_nodes(cx).unwrap();
                store.sync_worktree_checkouts(cx);
            });
            root.refresh_sessions(cx);
            root.reload_tabs(cx).unwrap();
        });

        wait_for_condition(&visual, "r2 worktree checkouts cached", || {
            let mut ready = false;
            harness.act(|root, _, cx| {
                let store = root.app_store.read(cx);
                ready = store
                    .worktree_checkout_for(wt.to_str().unwrap(), None)
                    .is_some()
                    && store
                        .worktree_checkout_for(term_wt.to_str().unwrap(), None)
                        .is_some();
            });
            ready
        });

        // 1. Small Issue 2: closing a terminal tab in a managed worktree closes without prompting.
        harness.act(|root, window, cx| {
            root.tabs.activate("01ARZ3NDEKTSV4RRFFQ69G5FB2");
            root.request_close_terminal_tab("01ARZ3NDEKTSV4RRFFQ69G5FB2", "term-r2", window, cx);
            assert!(root.terminal_close_confirm.is_none());
        });
        wait_for_condition(&visual, "terminal tab archived without confirm", || {
            let mut closed = false;
            harness.act(|root, _, _| {
                closed = root
                    .tabs
                    .tabs()
                    .iter()
                    .all(|t| t.id != "01ARZ3NDEKTSV4RRFFQ69G5FB2");
            });
            closed
        });
        assert!(term_wt.exists());

        // 2. R2: open dialog A (suspends after real check returns, before applying reply),
        // cancel A, reopen B, uncheck B, then release stale A reply and confirm A's callback ran
        // without clobbering B's unchecked state.
        let (gate_a, handle_a) = crate::test_support::StagedReplyGate::pair();
        let mut confirm_a_id = 0;
        harness.act(|root, window, cx| {
            root.archive_check_gates.push_back(gate_a);
            root.tabs.activate("01ARZ3NDEKTSV4RRFFQ69G5FB1");
            root.request_close_chat_pane("p1", "chat-r2", window, cx);
            confirm_a_id = root.terminal_close_confirm.as_ref().unwrap().id;
        });
        wait_for_condition(&visual, "dialog A check entered apply gate", || {
            handle_a.is_entered()
        });

        let mut confirm_b_id = 0;
        harness.act(|root, window, cx| {
            root.cancel_terminal_close(window, cx);
            assert!(root.terminal_close_confirm.is_none());
            root.request_close_chat_pane("p1", "chat-r2", window, cx);
            let confirm_b = root.terminal_close_confirm.as_ref().unwrap();
            assert!(confirm_b.id > confirm_a_id);
            confirm_b_id = confirm_b.id;
        });
        let options_b = wait_for_confirm_worktrees(&mut harness, 1, |opts| {
            matches!(
                opts.first().map(|o| &o.state),
                Some(crate::surfaces::sidebar::WorktreeRemoveState::Clean { checked: true })
            )
        });
        assert_eq!(options_b[0].branch, "r2-branch");

        harness.act(|root, _, cx| {
            root.toggle_confirm_worktree(0, cx);
            let confirm = root.terminal_close_confirm.as_ref().unwrap();
            assert!(matches!(
                confirm.worktree_options[0].state,
                crate::surfaces::sidebar::WorktreeRemoveState::Clean { checked: false }
            ));
        });

        // Release stale A reply and wait until A's callback completes against NativeRoot.
        let applied_a = handle_a.release();
        wait_for_condition(&visual, "stale dialog A reply callback finished", || {
            applied_a.is_applied()
        });

        harness.act(|root, window, cx| {
            let confirm = root.terminal_close_confirm.as_ref().unwrap();
            assert_eq!(confirm.id, confirm_b_id);
            assert!(matches!(
                confirm.worktree_options[0].state,
                crate::surfaces::sidebar::WorktreeRemoveState::Clean { checked: false }
            ));
            root.confirm_terminal_close(window, cx);
        });
        wait_for_condition(&visual, "chat-r2 archived", || {
            let mut archived = false;
            harness.act(|root, _, cx| {
                let core = root.app_store.read(cx).test_core.clone();
                archived =
                    runner_daemon::repo::session::get_row(&core.db.get().unwrap(), "chat-r2")
                        .unwrap()
                        .is_some_and(|row| row.archived_at.is_some());
            });
            archived
        });
        assert!(wt.exists());
    }
}
