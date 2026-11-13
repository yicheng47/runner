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
        self.front_page = client;
        if let crate::surfaces::front_page::FrontPageTarget::Remote { computer, .. } =
            &self.front_page.target
        {
            let saved = self
                .app_store
                .read(cx)
                .settings
                .remote_sidebars
                .get(computer)
                .cloned();
            if self.remote_settings.is_none() {
                let mut settings = self.app_store.read(cx).settings.clone();
                saved.clone().unwrap_or_default().apply_to(&mut settings);
                self.remote_settings = Some(settings);
                self.remote_sidebar_seed_pending = saved.is_none();
            }
            if let Some(saved) = saved.filter(|_| self.remote_sidebar_seed_pending) {
                saved.apply_to(self.remote_settings.as_mut().unwrap());
                self.remote_sidebar_seed_pending = false;
            }
            if self.remote_sidebar_seed_pending && self.remote_overview(cx).is_some() {
                let nodes = self.source_nodes(cx);
                let collapsed_projects = self
                    .source_projects(cx)
                    .into_iter()
                    .filter(|project| {
                        !nodes.iter().any(|node| {
                            matches!(node.node_type, NodeType::Tab | NodeType::Mission)
                                && node.pinned_position.is_none()
                                && node.parent_id.as_deref() == Some(project.id.as_str())
                        })
                    })
                    .map(|project| project.id)
                    .collect();
                self.remote_sidebar_seed_pending = false;
                self.update_app_settings(cx, true, |settings| {
                    settings.sidebar_collapsed_projects = collapsed_projects;
                    true
                });
            }
        }
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
    pub(super) fn remote_overview(&self, cx: &App) -> Option<runner_core::companion::Overview> {
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
                    let attention = super::rows::status_attention(rollup.priority_value()).max(
                        if members
                            .iter()
                            .any(|member| self.archiving_sessions.contains(&member.session_id))
                        {
                            AttentionState::Working
                        } else {
                            AttentionState::None
                        },
                    );
                    Some(SidebarRow::Tab {
                        node,
                        layout,
                        members,
                        attention,
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
                                    startup_error: None,
                                    status: Status::NeedsYou,
                                    reason: Some(runner_core::companion::NeedsYouReason::Answer),
                                    unread: false,
                                    error_acknowledged: false,
                                    interrupted_since: None,
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
                            linked_chat: mission.linked_chat.clone().map(|chat| {
                                runner_core::protocol::LinkedChat {
                                    session_id: chat.session_id,
                                    title: chat.title,
                                    archived: chat.archived,
                                    reports: chat.reports,
                                }
                            }),
                        },
                        any_session_live: live,
                        session_statuses: statuses,
                        phase: mission.phase.as_ref().and_then(|phase| phase.protocol()),
                    };
                    let attention = self.mission_attention(&summary);
                    Some(SidebarRow::Mission {
                        node,
                        summary,
                        attention,
                    })
                }
                NodeType::Project => None,
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn remote_row_attentions(&self, cx: &App) -> HashMap<String, AttentionState> {
        self.remote_rows(cx)
            .into_iter()
            .map(|row| match row {
                SidebarRow::Tab {
                    node, attention, ..
                }
                | SidebarRow::Mission {
                    node, attention, ..
                } => (node.ref_id.unwrap_or(node.id), attention),
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
            SidebarMenuAction::NewChat(_) => self.front_page.supports(&["layout"]),
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
            SidebarMenuAction::LinkToChat(_) => self.front_page.supports(&["linkMission"]),
            SidebarMenuAction::UnlinkMission(_) => self.front_page.supports(&["unlinkMission"]),
            SidebarMenuAction::StopMission(_) => self.front_page.supports(&["stopMission"]),
            SidebarMenuAction::ResumeMission(_) => self.front_page.supports(&["resumeMission"]),
            SidebarMenuAction::DeleteProject(_) => self.front_page.supports(&["deleteProject"]),
        }
    }
}

impl Sidebar {
    pub(super) fn tab_selected(&self, tab: &str, cx: &App) -> bool {
        if self.front_page.is_remote() {
            return matches!(&self.remote_selection, Some(crate::surfaces::remote_client::RemoteItem::Tab(id)) if id == tab);
        }
        self.shell.upgrade().is_some_and(|shell| { let shell = shell.read(cx); if shell.remote_client.is_active() { matches!(shell.remote_client.active_item(), Some(crate::surfaces::remote_client::RemoteItem::Tab(id)) if id == tab) } else { super::elements::chat_tab_row_active(&shell.route, shell.tabs.active_tab_id(), tab) } })
    }
    pub(super) fn mission_selected(&self, mission: &str, cx: &App) -> bool {
        if self.front_page.is_remote() {
            return matches!(&self.remote_selection, Some(crate::surfaces::remote_client::RemoteItem::Mission(id)) if id == mission);
        }
        self.shell.upgrade().is_some_and(|shell| { let shell = shell.read(cx); if shell.remote_client.is_active() { matches!(shell.remote_client.active_item(), Some(crate::surfaces::remote_client::RemoteItem::Mission(id)) if id == mission) } else { matches!(&shell.route, AppRoute::Mission(id) if id == mission) } })
    }
}
