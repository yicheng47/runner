use super::menus::sidebar_create_menu_entries;
use super::*;
use crate::surfaces::front_page::model::{MissionFields, ProjectDisplay};
use runner_core::companion::{MissionState, Status};

impl Sidebar {
    pub(crate) fn set_front_page_target(
        &mut self,
        client: crate::surfaces::front_page::FrontPageClient,
        cx: &mut Context<Self>,
    ) {
        if self.front_page.target != client.target {
            self.dismiss_transients(cx);
        }
        if client.is_remote() && self.remote_settings.is_none() {
            let mut settings = self.app_store.read(cx).settings.clone();
            settings.sidebar_collapsed_projects.clear();
            settings.sidebar_collapsed_active_groups.clear();
            settings.sidebar_projects_open = true;
            settings.sidebar_chats_open = true;
            settings.sidebar_active_only = false;
            self.remote_settings = Some(settings);
        }
        self.front_page = client;
        self.refresh_create_menu(cx);
        cx.notify();
    }

    pub(crate) fn set_remote_presentation(
        &mut self,
        layouts: HashMap<String, PaneLayout>,
        sessions: HashMap<String, SessionDisplay>,
        selection: Option<crate::surfaces::remote_client::RemoteItem>,
        cx: &mut Context<Self>,
    ) {
        if self.remote_selection != selection {
            self.active_project_id = match selection.as_ref() {
                Some(crate::surfaces::remote_client::RemoteItem::Tab(id)) => {
                    layouts.get(id).and_then(|layout| layout.parent_id.clone())
                }
                Some(crate::surfaces::remote_client::RemoteItem::Mission(id)) => {
                    self.remote_overview(cx).and_then(|overview| {
                        overview
                            .missions
                            .into_iter()
                            .find(|mission| &mission.id == id)
                            .and_then(|mission| mission.project)
                    })
                }
                _ => None,
            };
        }
        self.remote_layouts = layouts;
        self.remote_sessions = sessions;
        self.remote_selection = selection;
        cx.notify();
    }
    pub(crate) fn active_only(&self, cx: &App) -> bool {
        self.settings(cx).sidebar_active_only
    }
    pub(crate) fn toggle_active_only(&mut self, cx: &mut Context<Self>) {
        self.update_app_settings(cx, true, |settings| {
            settings.sidebar_active_only = !settings.sidebar_active_only;
            true
        });
    }
    fn remote_overview(&self, cx: &App) -> Option<runner_core::companion::Overview> {
        let crate::surfaces::front_page::FrontPageTarget::Remote { computer, .. } =
            &self.front_page.target
        else {
            return None;
        };
        self.app_store
            .read(cx)
            .remote_clients
            .read(cx)
            .overviews
            .get(computer)
            .cloned()
    }

    pub(crate) fn source_nodes(&self, cx: &App) -> Vec<NodeRow> {
        if self.front_page.is_remote() {
            self.remote_overview(cx)
                .map(|overview| {
                    crate::surfaces::remote_client::sidebar_nodes(&overview)
                        .into_iter()
                        .map(Into::into)
                        .collect()
                })
                .unwrap_or_default()
        } else {
            self.app_store
                .read(cx)
                .nodes
                .iter()
                .cloned()
                .map(Into::into)
                .collect()
        }
    }

    pub(super) fn source_projects(&self, cx: &App) -> Vec<ProjectDisplay> {
        if self.front_page.is_remote() {
            self.remote_overview(cx)
                .map(|overview| {
                    overview
                        .projects
                        .into_iter()
                        .map(|project| ProjectDisplay {
                            id: project.id,
                            name: project.name,
                            cwd: String::new(),
                            local: None,
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            self.app_store
                .read(cx)
                .projects
                .iter()
                .cloned()
                .map(Into::into)
                .collect()
        }
    }

    pub(super) fn remote_rows(&self, cx: &App) -> Vec<SidebarRow> {
        let Some(overview) = self.remote_overview(cx) else {
            return Vec::new();
        };
        self.source_nodes(cx)
            .into_iter()
            .filter_map(|node| match node.node_type {
                NodeType::Tab => {
                    let layout = self.remote_layouts.get(&node.id)?.clone();
                    let members = layout
                        .session_ids()
                        .iter()
                        .filter_map(|id| self.remote_sessions.get(id).cloned())
                        .collect::<Vec<_>>();
                    if members.is_empty() {
                        return None;
                    }
                    let rollup = self.tab_status_rollup(&members, cx);
                    Some(SidebarRow::Tab {
                        node,
                        layout,
                        members,
                        attention: super::rows::status_attention(rollup.priority_value()),
                    })
                }
                NodeType::Mission => {
                    let mission = overview
                        .missions
                        .iter()
                        .find(|mission| mission.id == node.id)?;
                    let mut statuses = mission
                        .members
                        .iter()
                        .filter_map(|member| {
                            overview
                                .sessions
                                .get(&member.session)
                                .filter(|session| {
                                    session.kind != runner_core::companion::SessionKind::Shell
                                })
                                .map(|session| {
                                    (
                                        member.session.clone(),
                                        crate::surfaces::remote_client::model::agent_status(
                                            &session.status,
                                        ),
                                    )
                                })
                        })
                        .collect::<Vec<_>>();
                    let live = mission.members.iter().any(|member| {
                        overview
                            .sessions
                            .get(&member.session)
                            .is_some_and(|session| {
                                !matches!(
                                    session.status.status.effective(),
                                    Status::Stopped | Status::Error
                                )
                            })
                    });
                    if mission.state == MissionState::Running && !live {
                        for (_, status) in &mut statuses {
                            status.unread_since = None;
                        }
                    }
                    if !mission.asks.is_empty() {
                        statuses.push((
                            String::new(),
                            crate::surfaces::remote_client::model::agent_status(
                                &runner_core::companion::SessionStatus {
                                    status: Status::NeedsYou,
                                    reason: Some(runner_core::companion::NeedsYouReason::Answer),
                                    unread: false,
                                },
                            ),
                        ));
                    }
                    let summary = MissionSummary {
                        mission: MissionFields {
                            id: mission.id.clone(),
                            title: mission.title.clone(),
                            running: mission.state == MissionState::Running,
                            project_id: mission.project.clone(),
                            cwd: None,
                            archived_at: None,
                        },
                        any_session_live: live,
                        session_statuses: statuses,
                    };
                    let rollup = self.mission_status_rollup(&summary);
                    Some(SidebarRow::Mission {
                        node,
                        summary,
                        attention: super::rows::status_attention(rollup.priority_value()),
                    })
                }
                NodeType::Project => None,
            })
            .collect()
    }

    pub(super) fn session_status(
        &self,
        id: &str,
        cx: &App,
    ) -> Option<runner_core::protocol::AgentStatus> {
        if self.front_page.is_remote() {
            self.remote_overview(cx)?
                .sessions
                .get(id)
                .map(|session| crate::surfaces::remote_client::model::agent_status(&session.status))
        } else {
            self.app_store.read(cx).session_statuses.get(id).cloned()
        }
    }
}

impl Sidebar {
    pub(super) fn menu_action_supported(&self, action: &SidebarMenuAction, cx: &App) -> bool {
        if !self.front_page.is_remote() {
            return true;
        }
        match action {
            SidebarMenuAction::NewChat(_) => self.front_page.supports(&["catalog", "startChat"]),
            SidebarMenuAction::NewTerminal(_) => {
                self.front_page.supports(&["startShell", "layout"])
            }
            SidebarMenuAction::NewMission(_) => false,
            SidebarMenuAction::TogglePin { .. } => self.front_page.supports(&["pin"]),
            SidebarMenuAction::Rename(_) => self.front_page.supports(&["rename"]),
            SidebarMenuAction::ForkChat(id) => {
                self.front_page.supports(&["catalog", "fork"])
                    && self
                        .remote_sessions
                        .get(id)
                        .is_some_and(|session| session.native_fork)
            }
            SidebarMenuAction::ArchiveTab { session_ids, .. } => {
                let shells = self.remote_overview(cx).is_some_and(|overview| {
                    session_ids.iter().any(|id| {
                        overview.sessions.get(id).is_some_and(|session| {
                            session.kind == runner_core::companion::SessionKind::Shell
                        })
                    })
                });
                self.front_page.supports(if shells {
                    &["archive", "close", "layout"]
                } else {
                    &["archive", "layout"]
                })
            }
            SidebarMenuAction::CloseTerminalTab { .. } => {
                self.front_page.supports(&["close", "archive", "layout"])
            }
            SidebarMenuAction::ArchiveMission(_) => self.front_page.supports(&["archive"]),
            SidebarMenuAction::StopMission(_) => self.front_page.supports(&["stopMission"]),
            SidebarMenuAction::ResumeMission(_) => self.front_page.supports(&["resumeMission"]),
            SidebarMenuAction::DeleteProject(_) => self.front_page.supports(&["deleteProject"]),
        }
    }

    pub(super) fn refresh_create_menu(&mut self, cx: &mut Context<Self>) {
        let entries = sidebar_create_menu_entries(&self.settings(cx).keymap_overrides)
            .into_iter()
            .filter(|(_, action)| self.menu_action_supported(action, cx))
            .collect::<Vec<_>>();
        self.create_actions = entries.iter().map(|(_, action)| action.clone()).collect();
        self.create_menu.update(cx, |menu, cx| {
            menu.set_items(entries.into_iter().map(|(item, _)| item).collect(), cx)
        });
    }
}

impl Sidebar {
    pub(super) fn remote_commit_drop(
        &self,
        dragged_id: &str,
        target: &DropTarget,
        cx: &mut Context<Self>,
    ) {
        use crate::surfaces::sidebar_logic::{
            complete_unpinned_scope_order, ordered_pinned_node_ids_after_drop,
            ordered_root_node_ids_after_project_drop, ordered_visible_node_ids_after_drop,
            DropKind,
        };
        let nodes = self.source_nodes(cx);
        let Some(node) = nodes.iter().find(|node| node.id == dragged_id).cloned() else {
            return;
        };
        let rows = self.resolved_sidebar_rows(cx);
        let pinned = target.kind == DropKind::Pinned;
        let feature = if pinned { "reorderPinned" } else { "move" };
        if !self.front_page.supports(&[feature]) {
            return;
        }
        let order = match target.kind {
            DropKind::Pinned => ordered_pinned_node_ids_after_drop(
                &nodes,
                &rows
                    .iter()
                    .filter(|row| row.node().pinned_position.is_some())
                    .map(|row| row.node().id.clone())
                    .collect::<Vec<_>>(),
                dragged_id,
                target.index,
            ),
            DropKind::Project => {
                ordered_root_node_ids_after_project_drop(&nodes, dragged_id, target.index)
            }
            DropKind::Leaf => {
                let visible = self
                    .scope_rows(&rows, target.parent_id.as_deref())
                    .iter()
                    .map(|row| row.node().id.clone())
                    .collect::<Vec<_>>();
                let visible =
                    ordered_visible_node_ids_after_drop(&visible, dragged_id, target.index);
                complete_unpinned_scope_order(
                    &nodes,
                    target.parent_id.as_deref(),
                    dragged_id,
                    &visible,
                )
            }
        };
        let order = order
            .iter()
            .filter_map(|id| nodes.iter().find(|node| &node.id == id).cloned())
            .collect::<Vec<_>>();
        let project = target
            .parent_id
            .as_ref()
            .and_then(|id| nodes.iter().find(|node| &node.id == id))
            .cloned();
        if let Some(shell) = self.shell.upgrade() {
            shell.update(cx, |root, cx| {
                root.remote_action(
                    "sidebar-order".into(),
                    &[feature],
                    move |client| {
                        if pinned {
                            client.reorder_pinned(&order)?;
                        } else {
                            client.move_item(&node, project.as_ref(), &order)?;
                        }
                        Ok(Default::default())
                    },
                    cx,
                )
            });
        }
    }

    pub(super) fn tab_selected(&self, tab: &str, cx: &App) -> bool {
        if self.front_page.is_remote() {
            return matches!(&self.remote_selection, Some(crate::surfaces::remote_client::RemoteItem::Tab(id)) if id == tab);
        }
        self.shell.upgrade().is_some_and(|shell| { let shell = shell.read(cx); if let Some(remote) = &shell.remote_window { matches!(&remote.active, Some(crate::surfaces::remote_client::RemoteItem::Tab(id)) if id == tab) } else { super::elements::chat_tab_row_active(&shell.route, shell.tabs.active_tab_id(), tab) } })
    }
    pub(super) fn mission_selected(&self, mission: &str, cx: &App) -> bool {
        if self.front_page.is_remote() {
            return matches!(&self.remote_selection, Some(crate::surfaces::remote_client::RemoteItem::Mission(id)) if id == mission);
        }
        self.shell.upgrade().is_some_and(|shell| { let shell = shell.read(cx); if let Some(remote) = &shell.remote_window { matches!(&remote.active, Some(crate::surfaces::remote_client::RemoteItem::Mission(id)) if id == mission) } else { matches!(&shell.route, AppRoute::Mission(id) if id == mission) } })
    }
}
