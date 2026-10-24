use super::FrontPageClient;
use crate::*;
use runner_core::companion::{ActionKind, ActionTarget};
use runner_core::protocol::ClientResult;

#[derive(Clone, Copy)]
pub(crate) enum SessionAction {
    Stop,
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

impl NativeRoot {
    pub(crate) fn remote_action(
        &mut self,
        key: String,
        required: &[&str],
        action: impl FnOnce(FrontPageClient) -> ClientResult<Completion> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let client = self.front_page_client(cx);
        let Some(remote) = self.remote_window.as_mut() else {
            return;
        };
        if key.starts_with("layout:") && remote.actions_pending.contains("archive-sessions") {
            return;
        }
        if !client.supports(required) || !remote.actions_pending.insert(key.clone()) {
            return;
        }
        remote.action_error = None;
        let before = remote.layouts.clone();
        let navigation_generation = remote.navigation_generation;
        let epoch = self.client_ui.epoch;
        let target = client.target.clone();
        let guard = client.clone();
        let required = required.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let work = cx.background_spawn(async move {
            if !client.supports(&required.iter().map(String::as_str).collect::<Vec<_>>()) {
                return Err(runner_core::protocol::ClientError::msg(
                    "The selected computer changed.",
                ));
            }
            action(client)
        });
        cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |root, cx| {
                if root.client_ui.epoch != epoch
                    || root.front_page_client(cx).target != target
                    || !guard.valid()
                {
                    return;
                }
                let Some(remote) = root.remote_window.as_mut() else {
                    return;
                };
                remote.actions_pending.remove(&key);
                match result {
                    Ok(done) => {
                        root.apply_remote_completion(done, &before, navigation_generation, cx);
                    }
                    Err(error) => {
                        if remote.navigation_generation == navigation_generation {
                            remote.action_error = Some(error.to_string());
                        }
                        if let Some(tab) = key.strip_prefix("layout:") {
                            remote.layout_hints.remove(tab);
                            if remote.navigation_generation != navigation_generation {
                                let mut staged = remote.shell_placement.lock().unwrap();
                                if staged.as_ref().is_some_and(|(_, layout)| layout.id == tab) {
                                    *staged = None;
                                }
                            }
                        }
                    }
                }
                root.sync_remote_state(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn apply_remote_completion(
        &mut self,
        done: Completion,
        before: &HashMap<String, PaneLayout>,
        navigation_generation: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(remote) = self.remote_window.as_mut() else {
            return;
        };
        let mut activate = done.layout.is_none();
        let mut placement = None;
        for mut layout in done.layout.into_iter().chain(done.layouts) {
            {
                let mut staged = remote.shell_placement.lock().unwrap();
                if staged
                    .as_ref()
                    .is_some_and(|(_, pending)| pending.id == layout.id)
                {
                    *staged = None;
                }
            }
            let current = self
                .app_store
                .read(cx)
                .remote_clients
                .read(cx)
                .overviews
                .get(&remote.computer.id)
                .and_then(|overview| overview.tabs.iter().find(|tab| tab.id == layout.id));
            let superseded = match current {
                Some(tab) => {
                    !super::model::matches_layout(&layout, tab)
                        && before
                            .get(&layout.id)
                            .is_none_or(|previous| !super::model::matches_layout(previous, tab))
                }
                None => before.contains_key(&layout.id),
            };
            if !superseded {
                if let Some(session) = &done.session {
                    if let Some(leaf) = layout
                        .root
                        .leaves()
                        .into_iter()
                        .find(|leaf| leaf.session_id.as_ref() == Some(session))
                    {
                        placement = Some((layout.id.clone(), Some(leaf.id.clone())));
                        activate = true;
                    } else if layout.drawer_shells().contains(session) {
                        placement = Some((layout.id.clone(), None));
                        activate = true;
                    }
                }
                if remote.navigation_generation != navigation_generation {
                    if let Some(previous) = remote.layouts.get(&layout.id) {
                        super::model::retain_presentation(&mut layout, previous);
                    }
                }
                remote.layout_hints.insert(layout.id.clone(), layout);
            }
        }
        remote.reopen_sessions.extend(done.reopened_sessions);
        if let Some(session) = done.session {
            remote.reopen_sessions.insert(session.clone());
            if activate && remote.navigation_generation == navigation_generation {
                remote.activate_session = Some(session);
                remote.activate_placement = placement;
            }
        }
        if let Some(tab) = done
            .activate_tab
            .filter(|_| remote.navigation_generation == navigation_generation)
        {
            remote.activate_tab = Some(tab);
        }
    }

    pub(crate) fn remote_session_action(
        &mut self,
        session: &str,
        action: SessionAction,
        size: (u16, u16),
        cx: &mut Context<Self>,
    ) {
        let size = (size.0.max(20), size.1.max(8));
        let feature = match action {
            SessionAction::Stop => "stop",
            SessionAction::Resume => "resume",
            SessionAction::Fork => "fork",
        };
        let session = session.to_owned();
        self.remote_action(
            format!("session:{session}"),
            &[feature],
            move |client| {
                let next = match action {
                    SessionAction::Stop => {
                        client.stop(&session)?;
                        None
                    }
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

    pub(crate) fn remote_layout(
        &mut self,
        draft: PaneLayout,
        activate: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(remote) = &self.remote_window else {
            return;
        };
        if remote.actions_pending.contains("archive-sessions") {
            return;
        }
        if remote.layout_hints.contains_key(&draft.id) {
            return;
        }
        let project = draft.parent_id.clone();
        let key = format!("layout:{}", draft.id);
        if !self.front_page_client(cx).supports(&["layout"])
            || remote.actions_pending.contains(&key)
        {
            return;
        }
        self.remote_window
            .as_mut()
            .unwrap()
            .layout_hints
            .insert(draft.id.clone(), draft.clone());
        self.remote_action(
            key,
            &["layout"],
            move |client| {
                client.layout(&draft, project)?;
                Ok(Completion {
                    activate_tab: activate.then(|| draft.id.clone()),
                    layout: Some(draft),
                    ..Default::default()
                })
            },
            cx,
        );
    }

    pub(crate) fn remote_close_session(
        &mut self,
        session: &str,
        pane: Option<&str>,
        drawer: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(mut draft) = self.front_page_layout().cloned() else {
            return;
        };
        if self
            .remote_window
            .as_ref()
            .is_some_and(|remote| remote.layout_hints.contains_key(&draft.id))
        {
            return;
        }
        let Some(display) = self.front_page_session(session, cx) else {
            return;
        };
        let shell = display.agent_runtime == "shell";
        let session = session.to_owned();
        if drawer {
            draft.remove_drawer_shell(&session);
        } else if let Some(pane) = pane {
            draft.close_pane(pane);
            draft.remove_session(&session);
        } else {
            draft.remove_session(&session);
        }
        let mut required = if shell {
            vec!["close"]
        } else {
            vec!["archive"]
        };
        if !draft.all_session_ids().is_empty() {
            required.push("layout");
        }
        self.remote_action(
            format!("layout:{}", draft.id),
            &required,
            move |client| {
                if shell {
                    client.close(&session)?;
                } else {
                    client.archive(ActionTarget {
                        kind: ActionKind::Session,
                        id: session,
                    })?;
                }
                if !draft.all_session_ids().is_empty() {
                    client.layout(&draft, draft.parent_id.clone())?;
                }
                Ok(Completion {
                    layout: Some(draft),
                    ..Default::default()
                })
            },
            cx,
        );
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

    pub(crate) fn remote_start_shell_in(
        &mut self,
        mut draft: PaneLayout,
        pane: Option<String>,
        cwd: Option<String>,
        size: (u16, u16),
        cx: &mut Context<Self>,
    ) {
        let Some(remote) = &self.remote_window else {
            return;
        };
        if remote.actions_pending.contains("archive-sessions")
            || remote
                .actions_pending
                .iter()
                .any(|key| key.starts_with("layout:"))
        {
            return;
        }
        let staged = remote.shell_placement.clone();
        let pending = staged.lock().unwrap().clone();
        if let Some((_, pending)) = &pending {
            draft = pending.clone();
        }
        if remote.layout_hints.contains_key(&draft.id) {
            return;
        }
        let required = if pending.is_some() {
            vec!["layout"]
        } else {
            vec!["startShell", "layout"]
        };
        self.remote_action(
            format!("layout:{}", draft.id),
            &required,
            move |client| {
                let session = if let Some((session, _)) = pending {
                    session
                } else {
                    let session = client.start_shell(
                        draft.parent_id.clone(),
                        cwd,
                        (size.0.max(20), size.1.max(8)),
                    )?;
                    if let Some(pane) = pane {
                        draft
                            .assign_session(&pane, &session)
                            .map_err(|e| runner_core::protocol::ClientError::msg(e.to_string()))?;
                    } else {
                        draft.add_drawer_shell(session.clone());
                    }
                    *staged.lock().unwrap() = Some((session.clone(), draft.clone()));
                    session
                };
                client.layout(&draft, draft.parent_id.clone())?;
                Ok(Completion {
                    session: Some(session),
                    layout: Some(draft),
                    ..Default::default()
                })
            },
            cx,
        );
    }
}

impl NativeRoot {
    pub(crate) fn submit_remote_rename(
        &mut self,
        pane: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (input, target) = if pane {
            let Some(rename) = self.pane_rename.as_ref() else {
                return;
            };
            (
                rename.input.clone(),
                ActionTarget {
                    kind: ActionKind::Session,
                    id: rename.session_id.clone(),
                },
            )
        } else {
            let Some(modal) = self.chat_rename_modal.as_mut() else {
                return;
            };
            if modal.submitting {
                return;
            }
            let target = match &modal.target {
                ChatRenameTarget::Session { session_id, .. } => ActionTarget {
                    kind: ActionKind::Session,
                    id: session_id.clone(),
                },
                ChatRenameTarget::Tab { tab_id, .. } => ActionTarget {
                    kind: ActionKind::Tab,
                    id: tab_id.clone(),
                },
            };
            (modal.input.clone(), target)
        };
        if input.read(cx).is_composing() {
            return;
        }
        let client = self.front_page_client(cx);
        if !client.supports(&["rename"]) {
            return;
        }
        let key = format!("rename:{}", target.id);
        let remote = self.remote_window.as_mut().unwrap();
        if !remote.actions_pending.insert(key.clone()) {
            return;
        }
        let name = input.read(cx).text().trim().to_owned();
        if let Some(modal) = self
            .chat_rename_modal
            .as_mut()
            .filter(|modal| modal.input == input)
        {
            modal.submitting = true;
            modal.error = None;
        }
        let epoch = self.client_ui.epoch;
        let guard = client.clone();
        let work = cx.background_spawn(async move { client.rename(target, name) });
        cx.spawn_in(window, async move |weak, cx| {
            let result = work.await;
            let _ = weak.update_in(cx, |root, window, cx| {
                if root.client_ui.epoch != epoch
                    || !guard.valid()
                    || root.front_page_client(cx).target != guard.target
                {
                    return;
                }
                let remote = root.remote_window.as_mut().unwrap();
                remote.actions_pending.remove(&key);
                match result {
                    Ok(()) => {
                        if root
                            .pane_rename
                            .as_ref()
                            .is_some_and(|rename| rename.input == input)
                        {
                            root.pane_rename = None;
                            root._pane_rename_focus_subscription = None;
                        }
                        if root
                            .chat_rename_modal
                            .as_ref()
                            .is_some_and(|modal| modal.input == input)
                        {
                            root.chat_rename_modal = None;
                        }
                        root.focus_active_terminal(window, cx);
                    }
                    Err(error) => {
                        if let Some(modal) = root
                            .chat_rename_modal
                            .as_mut()
                            .filter(|modal| modal.input == input)
                        {
                            modal.submitting = false;
                            modal.error = Some(error.to_string());
                        } else {
                            remote.action_error = Some(error.to_string());
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl NativeRoot {
    pub(crate) fn remote_archive_sessions(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        let Some(remote) = &self.remote_window else {
            return;
        };
        let layouts = remote
            .layouts
            .values()
            .filter(|layout| layout.all_session_ids().iter().any(|id| ids.contains(id)))
            .cloned()
            .collect::<Vec<_>>();
        if layouts.iter().any(|layout| {
            remote.layout_hints.contains_key(&layout.id)
                || remote
                    .actions_pending
                    .contains(&format!("layout:{}", layout.id))
        }) {
            return;
        }
        let mut plan = ids
            .iter()
            .filter_map(|id| {
                self.front_page_session(id, cx)
                    .map(|session| (id.clone(), session.agent_runtime == "shell"))
            })
            .collect::<Vec<_>>();
        plan.sort_by_key(|(id, shell)| {
            (
                *shell,
                self.active_focused_session_id().as_ref() == Some(id),
            )
        });
        let mut required = vec!["archive", "layout"];
        if plan.iter().any(|(_, shell)| *shell) {
            required.push("close");
        }
        self.remote_action(
            "archive-sessions".into(),
            &required,
            move |client| {
                for (id, shell) in &plan {
                    if *shell {
                        client.close(id)?;
                    } else {
                        client.archive(ActionTarget {
                            kind: ActionKind::Session,
                            id: id.clone(),
                        })?;
                    }
                }
                let mut completed = Vec::new();
                for mut layout in layouts {
                    for id in &ids {
                        layout.remove_session(id);
                        layout.remove_drawer_shell(id);
                    }
                    if !layout.all_session_ids().is_empty() {
                        client.layout(&layout, layout.parent_id.clone())?;
                    }
                    completed.push(layout);
                }
                Ok(Completion {
                    layouts: completed,
                    ..Default::default()
                })
            },
            cx,
        );
    }

    pub(crate) fn close_remote_front_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(workspace) = self.remote_mission_workspace() {
            workspace.update(cx, |workspace, cx| workspace.close_visible_tab(window, cx));
            return;
        }
        let Some(layout) = self.front_page_layout().cloned() else {
            return;
        };
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
            self.remote_action(
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
