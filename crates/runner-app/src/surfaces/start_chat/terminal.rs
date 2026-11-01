use super::*;

impl NativeRoot {
    pub(super) fn focused_empty_chat_pane(&self) -> Option<String> {
        (self.route == AppRoute::Chat)
            .then(|| self.tabs.active())
            .flatten()
            .and_then(|layout| {
                layout
                    .root
                    .leaves()
                    .into_iter()
                    .find(|leaf| leaf.id == layout.focused_pane_id && leaf.session_id.is_none())
                    .map(|leaf| leaf.id.clone())
            })
    }

    pub(crate) fn new_terminal_action(
        &mut self,
        _: &NewTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let remote = self.front_page_client(cx).is_remote();
        if !remote && self.create_modal_open() {
            return;
        }
        if self.launcher_start_pending(cx) || self.start_launcher_terminal(window, cx) {
            return;
        }
        let pane = if remote {
            self.front_page_layout().and_then(new_terminal_empty_pane)
        } else {
            self.focused_empty_chat_pane()
        };
        if let Some(pane_id) = pane {
            let original = self.front_page_layout().unwrap().clone();
            let (scope, cwd) = if remote {
                (
                    ProjectScope::or_root(original.parent_id.clone()),
                    original
                        .focused_session_id()
                        .map(str::to_owned)
                        .or_else(|| original.session_ids().first().cloned())
                        .and_then(|id| self.front_page_session(&id, cx))
                        .and_then(|session| session.cwd),
                )
            } else {
                self.terminal_start_location(cx)
            };
            self.spawn_terminal_in_pane(pane_id, original, scope, cwd, window, cx);
        } else {
            self.new_terminal_tab(
                ProjectScope::or_root(self.active_project_id(cx)),
                window,
                cx,
            );
        }
    }

    pub(crate) fn new_mission_action(
        &mut self,
        _: &NewMission,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_mission_launcher(
            None,
            ProjectScope::or_root(self.active_project_id(cx)),
            window,
            cx,
        );
    }

    pub(crate) fn new_terminal_tab(
        &mut self,
        scope: ProjectScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let remote = self.front_page_client(cx).is_remote();
        let mut draft = PaneLayout::single(None, &[]);
        if remote {
            draft.id = runner_core::event_log::UlidGen::new()
                .next()
                .expect("new tab ULID");
        }
        draft.parent_id = scope.project_id().map(str::to_owned);
        let size = if remote {
            self.estimated_terminal_size(&draft, &draft.focused_pane_id, window, cx)
        } else {
            (INITIAL_COLS, INITIAL_ROWS)
        };
        let project_cwd = scope.project_id().and_then(|project_id| {
            self.app_store
                .read(cx)
                .projects
                .iter()
                .find(|project| project.id == project_id)
                .map(|project| project.cwd.as_str())
        });
        let cwd = (!remote)
            .then(|| {
                terminal_working_dir(
                    None,
                    project_cwd,
                    &self.settings(cx).default_working_dir,
                    runner_core::app_paths::home_dir()
                        .as_deref()
                        .and_then(|home| home.to_str()),
                )
            })
            .flatten();
        let input = crate::surfaces::front_page::actions::ShellStart {
            draft: draft.clone(),
            pane: Some(draft.focused_pane_id.clone()),
            project: scope.project_id().map(str::to_owned),
            cwd,
            size,
        };
        self.launch_workspace_shell(input, window, cx, move |this, result, window, cx| {
            let spawned_id = result.as_ref().ok().cloned();
            let result = result.and_then(|spawned| {
                this.refresh_sessions(cx);
                this.reload_tabs(cx)?;
                if !this.tabs.activate_session(&spawned) {
                    bail!("terminal tab was not created");
                }
                this.sync_active_project_from_active_tab(cx);
                this.set_route(AppRoute::Chat, cx);
                this.ensure_active_tab_attached(window, cx)?;
                Ok(spawned)
            });

            match result {
                Ok(session_id) => {
                    this.chat_error = None;
                    this.mark_active_tab_viewed(window, cx);
                    this.sync_active_chat_detail(cx);
                    this.begin_chat_transition(
                        &session_id,
                        chat_lifecycle::TransitionKind::Starting,
                        Some(0),
                        window,
                        cx,
                    );
                }
                Err(error) => {
                    if let Some(session_id) = spawned_id {
                        let _ = this.front_page_client(cx).close(&session_id);
                    }
                    this.refresh_sessions(cx);
                    let _ = this.reload_tabs(cx);
                    this.chat_error = Some(error.to_string().into());
                }
            }
            cx.notify();
        });
    }

    pub(crate) fn new_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(workspace) = self.remote_client.remote_mission_workspace() {
            workspace.update(cx, |workspace, cx| {
                workspace.add_terminal_drawer_shell(window, cx)
            });
            return;
        }
        let remote = self.front_page_client(cx).is_remote();
        if remote && !self.front_page_is_chat() {
            return;
        }
        let mission_drawer_available = matches!(self.route, AppRoute::Mission(_))
            && self.mission_workspace.read(cx).drawer_available(cx);
        let target = new_terminal_target(&self.route, mission_drawer_available && !remote);
        if target == NewTerminalTarget::MissionDrawer {
            self.mission_workspace
                .update(cx, |workspace, workspace_cx| {
                    workspace.add_terminal_drawer_shell(window, workspace_cx)
                });
            return;
        }
        let Some(original) = self.front_page_layout().cloned() else {
            if !remote {
                self.chat_error = Some(crate::i18n::message(
                    "ui.open_a_tab_before_creating_a_terminal",
                ));
                cx.notify();
            }
            return;
        };
        if !remote && self.route != AppRoute::Chat {
            self.set_route(AppRoute::Chat, cx);
            self.activate_tab(&original.id, window, cx);
        }
        if let Some(pane_id) = new_terminal_empty_pane(&original) {
            let (scope, cwd) = if remote {
                (
                    ProjectScope::or_root(original.parent_id.clone()),
                    original
                        .focused_session_id()
                        .map(str::to_owned)
                        .or_else(|| original.session_ids().first().cloned())
                        .and_then(|id| self.front_page_session(&id, cx))
                        .and_then(|session| session.cwd),
                )
            } else {
                self.terminal_start_location(cx)
            };
            self.spawn_terminal_in_pane(pane_id, original, scope, cwd, window, cx);
        } else if self.active_tab_is_terminal(cx) {
            self.split_pane(&original.focused_pane_id, SplitOrientation::Row, window, cx);
        } else {
            self.new_terminal_action(&NewTerminal, window, cx);
        }
    }

    pub(crate) fn toggle_terminal_drawer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(workspace) = self.remote_client.remote_mission_workspace() {
            workspace.update(cx, |workspace, cx| {
                workspace.toggle_terminal_drawer(window, cx)
            });
            return;
        }
        if matches!(self.route, AppRoute::Mission(_)) {
            self.mission_workspace.update(cx, |workspace, cx| {
                workspace.toggle_terminal_drawer(window, cx)
            });
        }
    }

    pub(crate) fn hide_terminal_drawer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(workspace) = self.remote_client.remote_mission_workspace() {
            workspace.update(cx, |workspace, cx| {
                workspace.hide_terminal_drawer(window, cx)
            });
            return;
        }
        if self.remote_client.is_active() {
            if let Some(layout) = self.front_page_layout_mut() {
                layout.set_drawer_open(false);
            }
            self.remote_client.attach_remote_panes(cx);
            cx.notify();
            return;
        }

        if let Some(layout) = self.tabs.active_mut() {
            layout.set_drawer_open(false);
        }
        self.finish_terminal_drawer_update(window, cx);
    }

    pub(super) fn finish_terminal_drawer_update(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = self
            .persist_active_tab(cx)
            .and_then(|_| self.reload_tabs(cx))
            .and_then(|_| self.ensure_active_tab_attached(window, cx));
        match result {
            Ok(()) => {
                self.chat_error = None;
                if self.tabs.active().is_some_and(PaneLayout::drawer_open) {
                    if let Some(session_id) = self
                        .tabs
                        .active()
                        .and_then(PaneLayout::active_drawer_shell)
                        .map(str::to_owned)
                    {
                        self.focus_drawer_terminal(&session_id, window, cx);
                    }
                } else {
                    self.focus_active_terminal(window, cx);
                }
            }
            Err(error) => self.chat_error = Some(error.to_string().into()),
        }
        cx.notify();
    }

    /// Where a terminal started into the active tab opens: the tab's
    /// project, and the focused sibling's cwd before the project's.
    pub(crate) fn terminal_start_location(&self, cx: &App) -> (ProjectScope, Option<String>) {
        let sibling_cwd = self.tabs.active().and_then(|layout| {
            layout
                .focused_session_id()
                .or_else(|| {
                    layout
                        .root
                        .leaves()
                        .into_iter()
                        .find_map(|leaf| leaf.session_id.as_deref())
                })
                .and_then(|session_id| self.session_start_cwd(session_id, cx))
        });
        let project_id = self.active_tab_project_id(cx);
        let project_cwd = project_id.as_deref().and_then(|project_id| {
            self.app_store
                .read(cx)
                .projects
                .iter()
                .find(|project| project.id == project_id)
                .map(|project| project.cwd.as_str())
        });
        let cwd = terminal_working_dir(
            sibling_cwd.as_deref(),
            project_cwd,
            &self.settings(cx).default_working_dir,
            runner_core::app_paths::home_dir()
                .as_deref()
                .and_then(|home| home.to_str()),
        );
        (ProjectScope::or_root(project_id), cwd)
    }

    pub(crate) fn spawn_terminal_in_pane(
        &mut self,
        pane_id: String,
        original: PaneLayout,
        scope: ProjectScope,
        cwd: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let initial_size = if self.front_page_client(cx).is_remote() {
            self.estimated_terminal_size(&original, &pane_id, window, cx)
        } else {
            self.front_page_layout()
                .map(|layout| self.estimated_terminal_size(layout, &pane_id, window, cx))
                .unwrap_or((INITIAL_COLS, INITIAL_ROWS))
        };
        let input = crate::surfaces::front_page::actions::ShellStart {
            draft: original.clone(),
            pane: Some(pane_id.clone()),
            project: scope.project_id().map(str::to_owned),
            cwd,
            size: initial_size,
        };
        self.launch_workspace_shell(input, window, cx, move |this, result, window, cx| {
            let spawned_id = result.as_ref().ok().cloned();
            let result = result.and_then(|spawned| {
                this.refresh_sessions(cx);
                this.tabs.assign_to_active(&pane_id, &spawned)?;
                this.persist_active_tab(cx)?;
                this.reload_tabs(cx)?;
                this.tabs.activate_session(&spawned);
                this.sync_active_project_from_active_tab(cx);
                this.set_route(AppRoute::Chat, cx);
                this.ensure_active_tab_attached(window, cx)?;
                Ok(spawned)
            });

            match result {
                Ok(session_id) => {
                    this.chat_error = None;
                    this.mark_active_tab_viewed(window, cx);
                    this.sync_active_chat_detail(cx);
                    this.begin_chat_transition(
                        &session_id,
                        chat_lifecycle::TransitionKind::Starting,
                        Some(0),
                        window,
                        cx,
                    );
                }
                Err(error) => {
                    if let Some(session_id) = spawned_id {
                        let _ = this.front_page_client(cx).close(&session_id);
                    }
                    let _ = this.front_page_client(cx).layout(&original, None);
                    this.refresh_sessions(cx);
                    let _ = this.reload_tabs(cx);
                    this.chat_error = Some(error.to_string().into());
                }
            }
            cx.notify();
        });
    }
}
