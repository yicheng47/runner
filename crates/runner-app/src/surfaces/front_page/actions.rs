use super::FrontPageClient;
use crate::*;
use runner_core::companion::{ActionKind, ActionTarget};

#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) enum SessionAction {
    Resume,
    Fork,
}

#[derive(Default)]
pub(crate) struct Completion {
    pub(crate) session: Option<String>,
    pub(crate) reopened_sessions: Vec<String>,
    pub(crate) layout: Option<PaneLayout>,
    pub(crate) layouts: Vec<PaneLayout>,
    pub(crate) activate_tab: Option<String>,
}

pub(crate) struct WorkspaceAction {
    pub(crate) client: FrontPageClient,
    pub(crate) key: String,
    pub(crate) epoch: u64,
    pub(crate) remote: Option<(HashMap<String, PaneLayout>, u64)>,
    pub(crate) pending_archive: Option<u64>,
}

pub(crate) struct ShellStart {
    pub(crate) draft: PaneLayout,
    pub(crate) pane: Option<String>,
    pub(crate) project: Option<String>,
    pub(crate) cwd: Option<String>,
    pub(crate) size: (u16, u16),
}

pub(crate) struct ArchiveCompletion {
    pub(crate) removed: Vec<String>,
    pub(crate) archived_missions: Vec<String>,
    pub(crate) error: Option<String>,
    pub(crate) completion: Completion,
}

impl WorkspaceAction {
    pub(crate) fn prepare(
        root: &mut NativeRoot,
        key: String,
        required: &[&str],
        cx: &mut Context<NativeRoot>,
    ) -> Option<WorkspaceAction> {
        let client = root.front_page_client(cx);
        root.remote_client.prepare_action(client, key, required, cx)
    }

    pub(crate) fn complete<T>(
        root: &mut NativeRoot,
        action: WorkspaceAction,
        result: Result<T, String>,
        remote_completion: impl FnOnce(T) -> Completion,
        cx: &mut Context<NativeRoot>,
    ) -> Option<Result<T, String>> {
        Self::complete_observed(root, action, result, remote_completion, |_, _, _, _| {}, cx)
    }

    pub(crate) fn complete_observed<T>(
        root: &mut NativeRoot,
        mut action: WorkspaceAction,
        result: Result<T, String>,
        remote_completion: impl FnOnce(T) -> Completion,
        observe: impl FnOnce(&mut NativeRoot, &FrontPageClient, bool, &mut Context<NativeRoot>),
        cx: &mut Context<NativeRoot>,
    ) -> Option<Result<T, String>> {
        if let Some(token) = action.pending_archive.take() {
            root.remote_client.finish_pending_archive(token, cx);
        }
        let client = action.client.clone();
        let succeeded = result.is_ok();
        if action.remote.is_none() {
            observe(root, &client, succeeded, cx);
            return Some(result);
        }
        if root
            .remote_client
            .complete_action(action, result, remote_completion, cx)
        {
            observe(root, &client, succeeded, cx);
            root.sync_remote_state(cx);
            cx.notify();
        }
        None
    }

    pub(crate) fn close(
        root: &mut NativeRoot,
        session: &str,
        pane: Option<&str>,
        drawer: bool,
        cx: &mut Context<NativeRoot>,
    ) -> Option<(
        WorkspaceAction,
        impl FnOnce() -> Result<Completion, String> + Send + 'static,
    )> {
        let client = root.front_page_client(cx);
        let (draft, shell) = if client.is_remote() {
            root.remote_client.close_layout(session, pane, drawer, cx)?
        } else {
            (None, true)
        };
        let mut required = vec![if shell { "close" } else { "archive" }];
        if draft
            .as_ref()
            .is_some_and(|draft| !draft.all_session_ids().is_empty())
        {
            required.push("layout");
        }
        let key = draft
            .as_ref()
            .map(|draft| format!("layout:{}", draft.id))
            .unwrap_or_else(|| format!("session:{session}"));
        let action = WorkspaceAction::prepare(root, key, &required, cx)?;
        if client.is_remote() && pane.is_some() {
            if let Some(draft) = &draft {
                root.remote_client.track_pane_close(draft);
            }
        }
        let session = session.to_owned();
        let operation = move || {
            if shell {
                client.close(&session)
            } else {
                client.archive(ActionTarget {
                    kind: ActionKind::Session,
                    id: session,
                })
            }
            .map_err(|error| error.to_string())?;
            if let Some(draft) = &draft {
                if !draft.all_session_ids().is_empty() {
                    client
                        .layout(draft, draft.parent_id.clone())
                        .map_err(|error| error.to_string())?;
                }
            }
            Ok(Completion {
                layout: draft,
                ..Default::default()
            })
        };
        Some((action, operation))
    }

    pub(crate) fn shell(
        root: &mut NativeRoot,
        mut input: ShellStart,
        cx: &mut Context<NativeRoot>,
    ) -> Option<(
        WorkspaceAction,
        impl FnOnce() -> Result<Completion, String> + Send + 'static,
    )> {
        let client = root.front_page_client(cx);
        let remote_target = client.is_remote();
        let preparation = root.remote_client.prepare_shell_input(&mut input)?;
        let staged = preparation.staged;
        let pending = preparation.pending;
        let required = if remote_target && pending.is_some() {
            vec!["layout"]
        } else if remote_target {
            vec!["startShell", "layout"]
        } else {
            vec!["startShell"]
        };
        let action =
            WorkspaceAction::prepare(root, format!("layout:{}", input.draft.id), &required, cx)?;
        let operation = move || {
            let session = if let Some((session, _)) = pending {
                session
            } else {
                let session = client
                    .start_shell(input.project, input.cwd, input.size)
                    .map_err(|error| error.to_string())?;
                if remote_target {
                    if let Some(pane) = input.pane {
                        input
                            .draft
                            .assign_session(&pane, &session)
                            .map_err(|error| error.to_string())?;
                    } else {
                        input.draft.add_drawer_shell(session.clone());
                    }
                    *staged.as_ref().unwrap().lock().unwrap() =
                        Some((session.clone(), input.draft.clone()));
                }
                session
            };
            let layout = if remote_target {
                client
                    .layout(&input.draft, input.draft.parent_id.clone())
                    .map_err(|error| error.to_string())?;
                Some(input.draft)
            } else {
                None
            };
            Ok(Completion {
                session: Some(session),
                layout,
                ..Default::default()
            })
        };
        Some((action, operation))
    }

    pub(crate) fn archive(
        root: &mut NativeRoot,
        ids: Vec<String>,
        local_plan: Option<Vec<(String, super::client::ArchiveSessionOperation)>>,
        pane_close: Option<PendingPaneClose>,
        cx: &mut Context<NativeRoot>,
    ) -> Option<(
        WorkspaceAction,
        impl FnOnce() -> Result<ArchiveCompletion, String> + Send + 'static,
    )> {
        use super::client::ArchiveSessionOperation;
        let client = root.front_page_client(cx);
        let remote_target = client.is_remote();
        let mut layouts = root.remote_client.archive_layouts(&ids)?;
        let plan = if let Some(plan) = local_plan {
            plan
        } else {
            let mut plan = ids
                .iter()
                .filter_map(|id| {
                    root.front_page_session(id, cx).map(|session| {
                        let operation = if session.agent_runtime == "shell" {
                            ArchiveSessionOperation::CloseTerminal
                        } else {
                            let running = session.status == SessionStatus::Running;
                            let linked_missions = root
                                .linked_missions_for_chats(std::slice::from_ref(id), cx)
                                .into_iter()
                                .map(|mission| mission.id)
                                .collect::<Vec<_>>();
                            if linked_missions.is_empty() {
                                ArchiveSessionOperation::ArchiveChat { running }
                            } else {
                                ArchiveSessionOperation::ArchiveChatWithMissions {
                                    running,
                                    linked_missions,
                                }
                            }
                        };
                        (id.clone(), operation)
                    })
                })
                .collect::<Vec<_>>();
            plan.sort_by_key(|(id, operation)| {
                (
                    *operation == ArchiveSessionOperation::CloseTerminal,
                    root.active_focused_session_id().as_ref() == Some(id),
                )
            });
            plan
        };
        if let Some(close) = &pane_close {
            for layout in &mut layouts {
                if layout.id == close.tab_id {
                    layout.close_pane(&close.pane_id);
                }
            }
        }
        let mut required = vec!["archive"];
        if pane_close.is_none()
            || layouts.iter().any(|layout| {
                layout
                    .all_session_ids()
                    .iter()
                    .any(|session| !ids.contains(session))
            })
        {
            required.push("layout");
        }
        if plan
            .iter()
            .any(|(_, operation)| *operation == ArchiveSessionOperation::CloseTerminal)
        {
            required.push("close");
        }
        let key = pane_close
            .as_ref()
            .map(|close| format!("layout:{}", close.tab_id))
            .unwrap_or_else(|| "archive-sessions".into());
        let mut archive_sessions = Vec::new();
        let mut archive_missions = Vec::new();
        if remote_target {
            archive_sessions.extend(ids.iter().cloned());
            for (_, operation) in &plan {
                archive_missions.extend(operation.linked_missions().iter().cloned());
            }
            if root
                .remote_client
                .has_pending_archive(&archive_sessions, &archive_missions, cx)
            {
                return None;
            }
        }
        let mut action = WorkspaceAction::prepare(root, key, &required, cx)?;
        if remote_target {
            action.pending_archive =
                root.remote_client
                    .begin_pending_archive(&archive_sessions, &archive_missions, cx);
        }
        if remote_target {
            if let Some(close) = &pane_close {
                if let Some(layout) = layouts.iter().find(|layout| layout.id == close.tab_id) {
                    root.remote_client.track_pane_close(layout);
                }
            }
        }
        let operation = move || {
            let (removed, archived_missions, error) = client.archive_sessions(plan);
            if remote_target {
                if let Some(error) = &error {
                    return Err(error.clone());
                }
            }
            let mut completed = Vec::new();
            for mut layout in layouts {
                for id in &ids {
                    layout.remove_session(id);
                    layout.remove_drawer_shell(id);
                }
                if !layout.all_session_ids().is_empty() {
                    client
                        .layout(&layout, layout.parent_id.clone())
                        .map_err(|error| error.to_string())?;
                }
                completed.push(layout);
            }
            Ok(ArchiveCompletion {
                removed,
                archived_missions,
                error,
                completion: Completion {
                    layouts: completed,
                    ..Default::default()
                },
            })
        };
        Some((action, operation))
    }
}

impl NativeRoot {
    #[cfg(test)]
    pub(crate) fn remote_session_action(
        &mut self,
        session: &str,
        action: SessionAction,
        size: (u16, u16),
        cx: &mut Context<Self>,
    ) {
        let size = (size.0.max(20), size.1.max(8));
        let feature = match action {
            SessionAction::Resume => "resume",
            SessionAction::Fork => "fork",
        };
        let session = session.to_owned();
        self.remote_client.remote_action(
            format!("session:{session}"),
            &[feature],
            move |client| {
                let next = match action {
                    SessionAction::Resume => Some(client.resume(&session, size)?),
                    SessionAction::Fork => Some(client.fork(&session, None, size)?),
                };
                Ok(Completion {
                    session: next,
                    ..Default::default()
                })
            },
            cx,
        );
    }

    pub(crate) fn save_workspace_layout(
        &mut self,
        draft: PaneLayout,
        activate: bool,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        if self.launcher_tab_pending(&draft.id, cx) {
            anyhow::bail!("A start is pending in this tab.");
        }
        let remote_target = self.front_page_client(cx).is_remote();
        let action = if remote_target {
            self.remote_client.prepare_layout(&draft, cx)
        } else {
            WorkspaceAction::prepare(self, format!("layout:{}", draft.id), &["layout"], cx)
        };
        let Some(action) = action else {
            return Ok(());
        };
        let client = action.client.clone();
        let operation = move || {
            client
                .layout(&draft, draft.parent_id.clone())
                .map_err(|error| error.to_string())?;
            Ok(Completion {
                activate_tab: activate.then(|| draft.id.clone()),
                layout: Some(draft),
                ..Default::default()
            })
        };
        if remote_target {
            let task = cx.background_spawn(async move { operation() });
            cx.spawn(async move |weak, cx| {
                let result = task.await;
                let _ = weak.update(cx, |root, cx| {
                    WorkspaceAction::complete(root, action, result, |done| done, cx);
                });
            })
            .detach();
            cx.notify();
            Ok(())
        } else {
            operation().map(drop).map_err(anyhow::Error::msg)
        }
    }

    pub(crate) fn remote_layout(
        &mut self,
        draft: PaneLayout,
        activate: bool,
        cx: &mut Context<Self>,
    ) {
        if self.remote_client.is_active() {
            let _ = self.save_workspace_layout(draft, activate, cx);
        }
    }

    #[cfg(test)]
    pub(crate) fn remote_close_session(
        &mut self,
        session: &str,
        pane: Option<&str>,
        drawer: bool,
        cx: &mut Context<Self>,
    ) {
        if self.remote_client.is_local() {
            return;
        }
        let Some((action, operation)) = WorkspaceAction::close(self, session, pane, drawer, cx)
        else {
            return;
        };
        let task = cx.background_spawn(async move { operation() });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |root, cx| {
                WorkspaceAction::complete(root, action, result, |done| done, cx);
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn remote_start_shell(
        &mut self,
        draft: PaneLayout,
        pane: Option<String>,
        size: (u16, u16),
        cx: &mut Context<Self>,
    ) {
        let cwd = draft
            .focused_session_id()
            .map(str::to_owned)
            .or_else(|| draft.session_ids().first().cloned())
            .and_then(|id| self.front_page_session(&id, cx))
            .and_then(|session| session.cwd);
        self.remote_start_shell_in(draft, pane, cwd, size, cx);
    }

    pub(crate) fn launch_workspace_shell(
        &mut self,
        input: ShellStart,
        window: &mut Window,
        cx: &mut Context<Self>,
        finish_local: impl FnOnce(&mut Self, Result<String>, &mut Window, &mut Context<Self>),
    ) {
        let Some((action, operation)) = WorkspaceAction::shell(self, input, cx) else {
            return;
        };
        if action.client.is_remote() {
            let task = cx.background_spawn(async move { operation() });
            cx.spawn(async move |weak, cx| {
                let result = task.await;
                let _ = weak.update(cx, |root, cx| {
                    WorkspaceAction::complete(root, action, result, |done| done, cx);
                });
            })
            .detach();
            cx.notify();
        } else {
            let result = operation()
                .map(|done| done.session.unwrap())
                .map_err(anyhow::Error::msg);
            finish_local(self, result, window, cx);
        }
    }

    pub(crate) fn remote_start_shell_in(
        &mut self,
        draft: PaneLayout,
        pane: Option<String>,
        cwd: Option<String>,
        size: (u16, u16),
        cx: &mut Context<Self>,
    ) {
        if self.remote_client.is_local() {
            return;
        }
        let input = ShellStart {
            project: draft.parent_id.clone(),
            draft,
            pane,
            cwd,
            size,
        };
        let Some((action, operation)) = WorkspaceAction::shell(self, input, cx) else {
            return;
        };
        let task = cx.background_spawn(async move { operation() });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |root, cx| {
                WorkspaceAction::complete(root, action, result, |done| done, cx);
            });
        })
        .detach();
        cx.notify();
    }
}

impl NativeRoot {
    pub(crate) fn submit_workspace_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = &self.pane_rename else {
            return;
        };
        let input = rename.input.clone();
        let target = ActionTarget {
            kind: ActionKind::Session,
            id: rename.session_id.clone(),
        };
        let original = rename.original.clone();
        if input.read(cx).is_composing() {
            return;
        }
        let client = self.front_page_client(cx);
        if !client.supports(&["rename"]) {
            return;
        }
        let remote = client.is_remote();
        let name = input.read(cx).text().trim().to_owned();
        let (unchanged, title) = if remote {
            (false, Some(name))
        } else {
            match crate::surfaces::chat::pane_rename_change(&original, &name) {
                crate::surfaces::chat::PaneRenameChange::Unchanged => (true, None),
                crate::surfaces::chat::PaneRenameChange::Persist(title) => (false, title),
            }
        };
        let key = format!("rename:{}", target.id);
        if !self.remote_client.begin_named_action(&key) {
            return;
        }
        let epoch = self.remote_client.ui.epoch;
        let guard = client.clone();
        let operation = move || {
            if unchanged {
                Ok(())
            } else {
                client
                    .rename_title(target, title)
                    .map_err(|error| error.to_string())
            }
        };
        if !remote {
            let result = operation();
            self.finish_workspace_rename(input, None, result, window, cx);
        } else {
            let task = cx.background_spawn(async move { operation() });
            cx.spawn_in(window, async move |weak, cx| {
                let result = task.await;
                let _ = weak.update_in(cx, |root, window, cx| {
                    if root.remote_client.ui.epoch != epoch
                        || !guard.valid()
                        || root.front_page_client(cx).target != guard.target
                    {
                        return;
                    }
                    root.finish_workspace_rename(input, Some(key), result, window, cx);
                });
            })
            .detach();
        }
        cx.notify();
    }

    fn finish_workspace_rename(
        &mut self,
        input: Entity<runner_app::ui::TextField>,
        remote_key: Option<String>,
        result: Result<(), String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let remote = remote_key.is_some();
        let kept_focus = input.read(cx).focus_handle().is_focused(window);
        if let Some(key) = remote_key {
            self.remote_client.finish_named_action(&key);
        }
        match result {
            Ok(()) => {
                if !remote
                    || self
                        .pane_rename
                        .as_ref()
                        .is_some_and(|rename| rename.input == input)
                {
                    self.pane_rename = None;
                    self._pane_rename_focus_subscription = None;
                }
                if !remote {
                    self.refresh_sessions(cx);
                    if let Err(error) = self.reload_tabs(cx) {
                        self.chat_error = Some(error.to_string().into());
                    }
                    self.sync_active_chat_detail(cx);
                }
                if kept_focus {
                    self.focus_active_terminal(window, cx);
                }
            }
            Err(error) => {
                if remote {
                    self.remote_client.set_error(error);
                } else {
                    self.chat_error = Some(error.into());
                    if let Some(rename) = self.pane_rename.as_ref().filter(|_| kept_focus) {
                        rename.input.read(cx).focus_handle().focus(window, cx);
                    }
                }
            }
        }
        cx.notify();
    }
}

impl NativeRoot {
    pub(crate) fn remote_archive_sessions(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        if self.remote_client.is_local() {
            return;
        }
        let Some((action, operation)) = WorkspaceAction::archive(self, ids, None, None, cx) else {
            return;
        };
        let task = cx.background_spawn(async move { operation() });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |root, cx| {
                WorkspaceAction::complete(root, action, result, |done| done.completion, cx);
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn close_remote_front_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(workspace) = self.remote_client.remote_mission_workspace() {
            workspace.update(cx, |workspace, cx| workspace.close_visible_tab(window, cx));
            return;
        }
        let Some(layout) = self.front_page_layout().cloned() else {
            return;
        };
        if self.launcher_tab_pending(&layout.id, cx) {
            return;
        }
        if layout.drawer_open() && self.drawer_focus.contains_focused(window, cx) {
            self.hide_terminal_drawer(window, cx);
            return;
        }
        if layout.root.leaves().len() > 1 {
            let pane = layout.focused_pane_id.clone();
            match layout
                .focused_session_id()
                .and_then(|id| self.front_page_session(id, cx))
            {
                Some(session) if session.agent_runtime == "shell" => {
                    self.request_close_terminal_pane(&pane, &session.session_id, window, cx)
                }
                Some(session) => {
                    self.request_close_chat_pane(&pane, &session.session_id, window, cx)
                }
                None => self.close_pane(&pane, window, cx),
            }
        } else if let Some(session) = layout
            .focused_session_id()
            .and_then(|id| self.front_page_session(id, cx))
            .filter(|_| layout.all_session_ids().len() == 1)
        {
            if session.agent_runtime == "shell" {
                self.request_close_terminal_tab(&layout.id, &session.session_id, window, cx);
            } else {
                self.request_close_chat_pane(
                    &layout.focused_pane_id,
                    &session.session_id,
                    window,
                    cx,
                );
            }
        } else if !layout.all_session_ids().is_empty() {
            self.request_archive_all(
                Some(&layout.id),
                layout.all_session_ids(),
                ArchiveAllSource::Chat,
                window,
                cx,
            );
        } else {
            let id = layout.id;
            self.remote_client.remote_action(
                format!("archive-tab:{id}"),
                &["archive"],
                move |client| {
                    client.archive(ActionTarget {
                        kind: ActionKind::Tab,
                        id,
                    })?;
                    Ok(Default::default())
                },
                cx,
            );
        }
    }
}
