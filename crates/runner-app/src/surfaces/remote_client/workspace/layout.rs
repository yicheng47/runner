use super::*;

pub(crate) fn sidebar_nodes(overview: &Overview) -> Vec<SidebarNode> {
    overview.sidebar.clone().unwrap_or_else(|| {
        let mut nodes = overview
            .projects
            .iter()
            .enumerate()
            .map(|(position, project)| SidebarNode {
                kind: SidebarNodeKind::Project,
                id: project.id.clone(),
                parent: None,
                position: position as i64,
                pinned_position: None,
            })
            .collect::<Vec<_>>();
        for (position, tab) in overview.tabs.iter().enumerate() {
            nodes.push(SidebarNode {
                kind: SidebarNodeKind::Tab,
                id: tab.id.clone(),
                parent: tab.project.clone(),
                position: position as i64,
                pinned_position: tab.pinned.then_some(position as i64),
            });
        }
        for (position, mission) in overview.missions.iter().enumerate() {
            let position = (overview.tabs.len() + position) as i64;
            nodes.push(SidebarNode {
                kind: SidebarNodeKind::Mission,
                id: mission.id.clone(),
                parent: mission.project.clone(),
                position,
                pinned_position: mission.pinned.then_some(position),
            });
        }
        nodes
    })
}

impl RemoteClient {
    pub(crate) fn session_tab_layout(&self, session: &str) -> Option<&PaneLayout> {
        self.workspace
            .as_ref()?
            .layouts
            .values()
            .find(|layout| layout.contains_session(session))
    }

    pub(crate) fn layout(&self) -> Option<&PaneLayout> {
        let remote = self.workspace.as_ref()?;
        let Some(Item::Tab(id)) = &remote.active else {
            return None;
        };
        remote
            .resize_draft
            .as_ref()
            .filter(|layout| &layout.id == id)
            .or_else(|| remote.layouts.get(id))
    }

    pub(crate) fn layout_mut(&mut self) -> Option<&mut PaneLayout> {
        let remote = self.workspace.as_mut()?;
        let Some(Item::Tab(id)) = &remote.active else {
            return None;
        };
        remote.layouts.get_mut(id)
    }

    pub(crate) fn pane_bounds_mut(&mut self) -> Option<&mut HashMap<PaneKey, gpui::Size<Pixels>>> {
        self.workspace
            .as_mut()
            .map(|remote| &mut remote.pane_bounds)
    }

    pub(crate) fn resize_split(&mut self, id: &str, ratio: f32, cx: &mut Context<NativeRoot>) {
        let draft = self.layout().cloned();
        let Some(remote) = self.workspace.as_mut() else {
            return;
        };
        if remote.resize_draft.is_none() {
            remote.resize_draft = draft;
        }
        if let Some(draft) = &mut remote.resize_draft {
            draft.set_split_sizes(id, [ratio, 100. - ratio]);
        }
        remote.resizing_split = Some(id.into());
        cx.notify();
    }

    pub(crate) fn finish_split_resize(&mut self) -> Option<PaneLayout> {
        let remote = self.workspace.as_mut()?;
        remote.resizing_split = None;
        remote.resize_draft.take()
    }

    pub(crate) fn finish_drawer_resize(&mut self, cx: &mut Context<NativeRoot>) {
        if self
            .workspace
            .as_mut()
            .is_some_and(|remote| std::mem::take(&mut remote.drawer_resizing))
        {
            cx.notify();
        }
    }

    pub(crate) fn set_sidebar_width(&mut self, width: f32, cx: &mut Context<NativeRoot>) {
        let Some(remote) = self.workspace.as_mut() else {
            return;
        };
        remote.sidebar_width = width;
        remote.mission_workspace.update(cx, |workspace, cx| {
            workspace.set_remote_sidebar_width(width, cx)
        });
        cx.notify();
    }

    pub(crate) fn set_sidebar_collapsed(&mut self, collapsed: bool, cx: &mut Context<NativeRoot>) {
        let Some(remote) = self.workspace.as_mut() else {
            return;
        };
        remote.sidebar_collapsed = collapsed;
        remote.mission_workspace.update(cx, |workspace, cx| {
            workspace.set_sidebar_collapsed(collapsed, cx)
        });
        cx.notify();
    }

    pub(crate) fn focus_session_layout(&mut self, session: &str) -> Option<String> {
        let remote = self.workspace.as_mut()?;
        let tab = remote
            .layouts
            .values()
            .find(|layout| layout.all_session_ids().iter().any(|id| id == session))
            .map(|layout| layout.id.clone())?;
        if let Some(layout) = remote.layouts.get_mut(&tab) {
            if layout.drawer_shells().iter().any(|id| id == session) {
                layout.activate_drawer_shell(session);
            } else {
                layout.focus_session(session);
            }
        }
        Some(tab)
    }

    pub(crate) fn set_error(&mut self, error: String) {
        if let Some(remote) = self.workspace.as_mut() {
            remote.action_error = Some(error);
        }
    }

    pub(crate) fn activate_mission(&mut self, id: String) {
        if let Some(remote) = self.workspace.as_mut() {
            remote.activate_mission = Some(id);
        }
    }

    pub(crate) fn reopen_sessions(
        &mut self,
        target: &crate::surfaces::front_page::FrontPageTarget,
        ids: Vec<String>,
    ) -> bool {
        let Some(remote) = self
            .workspace
            .as_mut()
            .filter(|remote| &remote.front_page.target == target)
        else {
            return false;
        };
        remote.reopen_sessions.extend(ids);
        true
    }
}

impl RemoteClient {
    pub(crate) fn session(
        &self,
        id: &str,
        cx: &App,
    ) -> Option<crate::surfaces::front_page::model::SessionDisplay> {
        let remote = self.workspace.as_ref()?;
        let state = self.app_store.read(cx).remote_clients.read(cx);
        let overview = state.overviews.get(&remote.computer.id)?;
        let session = overview.sessions.get(id)?;
        let pinned = overview.tabs.iter().any(|tab| {
            tab.pinned
                && remote
                    .layouts
                    .get(&tab.id)
                    .is_some_and(|layout| layout.contains_session(id))
        });
        let native_fork = remote.catalog.as_ref().is_some_and(|catalog| {
            catalog.runtimes.iter().any(|runtime| {
                Some(&runtime.name) == session.runtime.as_ref() && runtime.native_fork
            })
        });
        Some(crate::surfaces::front_page::model::SessionDisplay::remote(
            id,
            session,
            pinned,
            native_fork,
            remote.front_page.supports(&["resume"]),
        ))
    }
}

impl RemoteClient {
    pub(crate) fn computer(&self) -> Option<&Computer> {
        self.workspace.as_ref().map(|remote| &remote.computer)
    }

    pub(crate) fn active_item(&self) -> Option<&Item> {
        self.workspace.as_ref()?.active.as_ref()
    }

    pub(crate) fn landed_at(&self) -> Option<std::time::Instant> {
        self.workspace.as_ref()?.landed_at
    }

    pub(crate) fn mission_workspace(&self) -> Option<Entity<MissionWorkspace>> {
        self.workspace
            .as_ref()
            .map(|remote| remote.mission_workspace.clone())
    }

    pub(crate) fn sidebar(&self) -> Option<Entity<Sidebar>> {
        self.workspace.as_ref().map(|remote| remote.sidebar.clone())
    }

    pub(crate) fn tab_layout(&self, id: &str) -> Option<&PaneLayout> {
        self.workspace.as_ref()?.layouts.get(id)
    }

    pub(crate) fn pane_closing(&self, pane: &PaneKey) -> bool {
        self.workspace
            .as_ref()
            .and_then(|remote| remote.closing_layouts.get(&pane.tab_id))
            .is_some_and(|layout| {
                !layout
                    .root
                    .leaves()
                    .iter()
                    .any(|leaf| leaf.id == pane.pane_id)
            })
    }

    pub(crate) fn track_pane_close(&mut self, layout: &PaneLayout) {
        self.workspace
            .as_mut()
            .unwrap()
            .closing_layouts
            .insert(layout.id.clone(), layout.clone());
    }

    pub(crate) fn pane_close_pending(&self, tab: &str) -> bool {
        self.workspace
            .as_ref()
            .is_some_and(|remote| remote.closing_layouts.contains_key(tab))
    }

    pub(crate) fn recently_viewed_tab_layout(&self) -> Option<&PaneLayout> {
        let remote = self.workspace.as_ref()?;
        remote
            .viewed_tabs
            .iter()
            .rev()
            .find_map(|id| remote.layouts.get(id))
    }

    pub(crate) fn session_pending(&self, id: &str) -> bool {
        self.workspace
            .as_ref()
            .is_some_and(|remote| remote.actions_pending.contains(&format!("session:{id}")))
    }

    pub(crate) fn pane_bounds(&self) -> Option<&HashMap<PaneKey, gpui::Size<Pixels>>> {
        self.workspace.as_ref().map(|remote| &remote.pane_bounds)
    }

    pub(crate) fn resizing_split(&self) -> Option<Option<&str>> {
        self.workspace
            .as_ref()
            .map(|remote| remote.resizing_split.as_deref())
    }

    pub(crate) fn sidebar_collapsed(&self) -> Option<bool> {
        self.workspace
            .as_ref()
            .map(|remote| remote.sidebar_collapsed)
    }

    pub(crate) fn sidebar_width(&self) -> Option<f32> {
        self.workspace.as_ref().map(|remote| remote.sidebar_width)
    }

    pub(crate) fn mission_form_current(
        &self,
        target: &crate::surfaces::front_page::FrontPageTarget,
        navigation: u64,
        form: u64,
    ) -> bool {
        self.workspace.as_ref().is_some_and(|remote| {
            remote.front_page.target == *target
                && remote.front_page.valid()
                && remote.navigation_generation == navigation
                && remote.form_generation == form
        })
    }

    pub(crate) fn startup_error(
        &self,
        id: &str,
        cx: &App,
    ) -> Option<runner_core::protocol::StartupError> {
        let remote = self.workspace.as_ref()?;
        let state = self.app_store.read(cx).remote_clients.read(cx);
        state
            .overviews
            .get(&remote.computer.id)?
            .sessions
            .get(id)?
            .status
            .startup_error
            .clone()
    }

    pub(crate) fn status(
        &self,
        id: &str,
        cx: &App,
    ) -> Option<runner_app::ui::agent_status::StatusPresentation> {
        let remote = self.workspace.as_ref()?;
        let state = self.app_store.read(cx).remote_clients.read(cx);
        let session = state.overviews.get(&remote.computer.id)?.sessions.get(id)?;
        let status = model::agent_status(&session.status);
        let mut display = runner_app::ui::agent_status::StatusPresentation::new(&status);
        display.waiting_since = None;
        display.estimated = false;
        Some(display)
    }

    pub(crate) fn archive_targets(&self, ids: Vec<String>) -> Vec<String> {
        let layouts = self
            .workspace
            .as_ref()
            .map(|remote| remote.layouts.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        crate::surfaces::sidebar::archive_targets_for_chats(ids, &layouts)
    }

    pub(crate) fn overview<'a>(&self, cx: &'a App) -> Option<&'a Overview> {
        let remote = self.workspace.as_ref()?;
        self.app_store
            .read(cx)
            .remote_clients
            .read(cx)
            .overviews
            .get(&remote.computer.id)
    }
}
