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
        if confirmation_body.is_none() && !has_linked_missions {
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
}
