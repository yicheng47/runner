use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use runner_core::companion::{
    ActionKind, ActionRequest, ActionTarget, Catalog, ClientMessage, FolderPage, OverviewRequest,
};
use runner_core::protocol::{ClientError, ClientResult, DaemonClient};
use serde::de::DeserializeOwned;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum FrontPageTarget {
    Local,
    Remote {
        computer: String,
        fingerprint: String,
        generation: u64,
    },
}

#[derive(Default)]
struct AccessState {
    valid: bool,
    features: Option<HashSet<String>>,
}

pub(crate) struct RemoteAccess {
    pub(crate) target: FrontPageTarget,
    state: RwLock<AccessState>,
}

impl RemoteAccess {
    pub(crate) fn new(target: FrontPageTarget) -> Arc<Self> {
        Arc::new(Self {
            target,
            state: RwLock::new(AccessState {
                valid: true,
                features: None,
            }),
        })
    }

    pub(crate) fn invalidate(&self) {
        let mut state = self.state.write().unwrap();
        state.valid = false;
        state.features = None;
    }

    pub(crate) fn set_features(&self, features: Vec<String>) {
        let mut state = self.state.write().unwrap();
        if state.valid {
            state.features = Some(features.into_iter().collect());
        }
    }

    pub(crate) fn supports(&self, features: &[&str]) -> bool {
        let state = self.state.read().unwrap();
        state.valid
            && state
                .features
                .as_ref()
                .is_some_and(|known| features.iter().all(|feature| known.contains(*feature)))
    }

    /// Whether this host is known not to serve `feature`, unlike `supports`, which also says no
    /// before the features are discovered.
    pub(crate) fn lacks(&self, feature: &str) -> bool {
        let state = self.state.read().unwrap();
        state.valid
            && state
                .features
                .as_ref()
                .is_some_and(|known| !known.contains(feature))
    }

    pub(crate) fn valid(&self) -> bool {
        self.state.read().unwrap().valid
    }
}

#[derive(Clone)]
pub(crate) struct FrontPageClient {
    pub(crate) target: FrontPageTarget,
    daemon: DaemonClient,
    access: Option<Arc<RemoteAccess>>,
}

impl FrontPageClient {
    pub(crate) fn local(daemon: DaemonClient) -> Self {
        Self {
            target: FrontPageTarget::Local,
            daemon,
            access: None,
        }
    }

    pub(crate) fn remote(daemon: DaemonClient, access: Arc<RemoteAccess>) -> Self {
        Self {
            target: access.target.clone(),
            daemon,
            access: Some(access),
        }
    }

    pub(crate) fn is_remote(&self) -> bool {
        self.access.is_some()
    }

    pub(crate) fn supports(&self, features: &[&str]) -> bool {
        self.access
            .as_ref()
            .is_none_or(|access| access.supports(features))
    }

    pub(crate) fn valid(&self) -> bool {
        self.access.as_ref().is_none_or(|access| access.valid())
    }

    pub(crate) fn discover(&self) -> ClientResult<()> {
        let FrontPageTarget::Remote { computer, .. } = &self.target else {
            return Ok(());
        };
        if !self.valid() {
            return Err(ClientError::msg("The selected computer changed."));
        }
        let result = self.daemon.remote_request(
            computer.clone(),
            ClientMessage::Overview(OverviewRequest { id: 0 }),
        )?;
        // Only the followed overview owns state; this reply may predate a push.
        self.access.as_ref().unwrap().set_features(result.features);
        Ok(())
    }

    fn action<T: DeserializeOwned>(
        &self,
        feature: &str,
        request: ActionRequest,
    ) -> ClientResult<T> {
        let FrontPageTarget::Remote { computer, .. } = &self.target else {
            return Err(ClientError::msg("This operation requires a remote target."));
        };
        if !self.supports(&[feature]) {
            return Err(ClientError::msg(
                "This action is unavailable on the selected computer.",
            ));
        }
        let result = self
            .daemon
            .remote_request(computer.clone(), ClientMessage::Action(Box::new(request)))?;
        if !self.valid() {
            return Err(ClientError::msg("The selected computer changed."));
        }
        serde_json::from_value(result.result)
            .map_err(|_| ClientError::msg("The computer returned an invalid action result."))
    }

    pub(crate) fn usage(&self, popover: bool) -> ClientResult<runner_core::companion::Usage> {
        self.action("usage", ActionRequest::Usage { id: 0, popover })
    }

    pub(crate) fn start_runtime_task(
        &self,
        runtime: runner_core::protocol::Runtime,
    ) -> ClientResult<runner_core::protocol::RuntimeUpdateTask> {
        if !self.is_remote() {
            return self.daemon.runtime_task_start(runtime);
        }
        self.action(
            "runtimeTaskStart",
            ActionRequest::RuntimeTaskStart {
                id: 0,
                runtime: runtime.key().into(),
            },
        )
    }

    pub(crate) fn runtime_task(
        &self,
        runtime: runner_core::protocol::Runtime,
    ) -> ClientResult<Option<runner_core::protocol::RuntimeUpdateTask>> {
        if !self.is_remote() {
            return self
                .daemon
                .runtime_task_list()
                .map(|tasks| tasks.into_iter().find(|task| task.runtime == runtime));
        }
        self.action(
            "runtimeTask",
            ActionRequest::RuntimeTask {
                id: 0,
                runtime: runtime.key().into(),
            },
        )
    }

    pub(crate) fn task_terminal(
        &self,
        id: &str,
    ) -> Option<(
        String,
        DaemonClient,
        crate::surfaces::remote_client::transport::AttachmentLifetime,
    )> {
        let FrontPageTarget::Remote { computer, .. } = &self.target else {
            return None;
        };
        let config = runner_core::remote::RemoteTerminal {
            computer: computer.clone(),
            session: id.into(),
            cols: 80,
            rows: 24,
            take_control: true,
        };
        let address = config.address();
        let (client, lifetime) = crate::surfaces::remote_client::transport::RemoteTransport::client(
            self.daemon.clone(),
            config,
            "Agent update".into(),
            Arc::new(std::sync::atomic::AtomicBool::new(true)),
        );
        Some((address, client, lifetime))
    }

    pub(crate) fn catalog(&self) -> ClientResult<Catalog> {
        self.action("catalog", ActionRequest::Catalog { id: 0 })
    }

    pub(crate) fn role_working_dir(&self, role: String) -> ClientResult<Option<String>> {
        let record: runner_core::companion::RoleRecord =
            self.action("role", ActionRequest::Role { id: 0, role })?;
        Ok(record.working_dir)
    }

    pub(crate) fn mark_viewed(&self, session: String) -> ClientResult<()> {
        if !self.is_remote() {
            return self.daemon.mark_direct_sessions_viewed(&[session]);
        }
        self.action("markViewed", ActionRequest::MarkViewed { id: 0, session })
    }

    pub(crate) fn folders(&self, path: Option<String>) -> ClientResult<FolderPage> {
        self.action("folders", ActionRequest::Folders { id: 0, path })
    }

    pub(crate) fn start_chat(&self, input: ChatStart) -> ClientResult<String> {
        let ChatStart {
            role,
            runtime,
            model,
            effort,
            speed,
            project,
            cwd,
            size,
        } = input;
        if !self.is_remote() {
            let scope = runner_core::protocol::project::ProjectScope::or_root(project);
            let speed = speed.map(|s| {
                if s == "fast" {
                    runner_core::protocol::model::CodexSpeed::Fast
                } else {
                    runner_core::protocol::model::CodexSpeed::Standard
                }
            });
            return if let Some(role) = role {
                self.daemon
                    .session_start_direct_with_speed(
                        role,
                        runtime,
                        model,
                        effort,
                        speed,
                        scope,
                        cwd,
                        Some(size.0),
                        Some(size.1),
                    )
                    .map(|s| s.id)
            } else {
                self.daemon
                    .session_start_runtime_with_speed(
                        runtime.as_deref().unwrap_or_default(),
                        scope,
                        cwd,
                        Some(size.0),
                        Some(size.1),
                        model,
                        effort,
                        speed,
                    )
                    .map(|s| s.session.id)
            };
        }
        self.action::<SessionResult>(
            "startChat",
            ActionRequest::StartChat {
                id: 0,
                role,
                runtime,
                model,
                effort,
                speed,
                project,
                cwd,
                cols: size.0,
                rows: size.1,
            },
        )
        .map(|s| s.session)
    }

    pub(crate) fn launch_chat(&self, launch: &mut ChatLaunch) -> ClientResult<ChatLaunched> {
        let session = if let Some(session) = &launch.started {
            session.clone()
        } else {
            let session = self.start_chat(launch.input.clone())?;
            launch.started = Some(session.clone());
            session
        };
        let layout = if let Some((layout, pane)) = &launch.placement {
            let mut layout = layout.clone();
            layout
                .assign_session(pane, &session)
                .map_err(|error| ClientError::msg(error.to_string()))?;
            if self.is_remote() {
                self.layout(&layout, layout.parent_id.clone())?;
            } else {
                self.launcher_layout(&layout, launch.input.project.clone())?;
            }
            Some(layout)
        } else {
            None
        };
        Ok(ChatLaunched { session, layout })
    }

    pub(crate) fn launcher_layout(
        &self,
        layout: &runner_app::pane_layout::PaneLayout,
        project: Option<String>,
    ) -> ClientResult<()> {
        if self.is_remote() {
            return self.layout(layout, project);
        }
        let nodes = self.daemon.node_list()?;
        let existing = nodes.iter().find(|node| node.id == layout.id);
        let existing_project =
            existing.and_then(|node| crate::surfaces::sidebar::node_project_id(&nodes, node));
        let parent = if existing.is_some() && existing_project == project {
            existing.and_then(|node| node.parent_id.clone())
        } else {
            project
                .as_ref()
                .map(|project| {
                    nodes
                        .iter()
                        .find(|node| {
                            node.node_type == runner_core::protocol::node::NodeType::Project
                                && node.ref_id.as_ref() == Some(project)
                        })
                        .map(|node| node.id.clone())
                        .ok_or_else(|| {
                            ClientError::msg("The selected project is no longer available.")
                        })
                })
                .transpose()?
        };
        if let Some(existing) = existing.filter(|existing| existing.parent_id != parent) {
            let mut order = nodes
                .iter()
                .filter(|node| node.parent_id == parent && node.id != existing.id)
                .collect::<Vec<_>>();
            order.sort_by_key(|node| node.position);
            let mut order = order
                .into_iter()
                .map(|node| node.id.clone())
                .collect::<Vec<_>>();
            order.push(existing.id.clone());
            self.daemon
                .node_move(existing.id.clone(), parent.clone(), order)?;
        }
        let mut draft = layout.clone();
        draft.parent_id = parent;
        self.layout(&draft, project)
    }

    pub(crate) fn start_shell(
        &self,
        project: Option<String>,
        cwd: Option<String>,
        size: (u16, u16),
    ) -> ClientResult<String> {
        if !self.is_remote() {
            return self
                .daemon
                .session_start_shell_in(
                    runner_core::protocol::project::ProjectScope::or_root(project),
                    cwd,
                    Some(size.0),
                    Some(size.1),
                )
                .map(|s| s.id);
        }
        self.action::<SessionResult>(
            "startShell",
            ActionRequest::StartShell {
                id: 0,
                project,
                cwd,
                cols: size.0,
                rows: size.1,
            },
        )
        .map(|s| s.session)
    }

    pub(crate) fn start_role_chat(
        &self,
        role: String,
        cwd: Option<String>,
        size: (u16, u16),
    ) -> ClientResult<String> {
        if !self.is_remote() {
            return self
                .daemon
                .session_start_direct(
                    role,
                    None,
                    None,
                    None,
                    runner_core::protocol::project::ProjectScope::Root,
                    cwd,
                    Some(size.0),
                    Some(size.1),
                )
                .map(|session| session.id);
        }
        self.start_chat(ChatStart {
            role: Some(role),
            runtime: None,
            model: None,
            effort: None,
            speed: None,
            project: None,
            cwd,
            size,
        })
    }

    pub(crate) fn layout(
        &self,
        layout: &runner_app::pane_layout::PaneLayout,
        project: Option<String>,
    ) -> ClientResult<()> {
        if !self.is_remote() {
            let input = layout
                .upsert_input()
                .map_err(|e| ClientError::msg(e.to_string()))?;
            return self.daemon.node_tab_upsert(input).map(drop);
        }
        self.action::<serde_json::Value>(
            "layout",
            ActionRequest::Layout {
                id: 0,
                target: ActionTarget {
                    kind: ActionKind::Tab,
                    id: layout.id.clone(),
                },
                project,
                name: layout.name.clone().unwrap_or_default(),
                panes: super::model::action_pane(&layout.root),
                shells: layout.drawer_shells().to_vec(),
            },
        )
        .map(drop)
    }

    pub(crate) fn stop(&self, session: &str) -> ClientResult<()> {
        if !self.is_remote() {
            return self.daemon.session_kill(session);
        }
        self.action(
            "stop",
            ActionRequest::Stop {
                id: 0,
                session: session.into(),
            },
        )
    }

    pub(crate) fn resume(&self, session: &str, size: (u16, u16)) -> ClientResult<String> {
        self.resume_session(session, size, false)
    }

    pub(crate) fn resume_session(
        &self,
        session: &str,
        size: (u16, u16),
        launch_claim: bool,
    ) -> ClientResult<String> {
        if !self.is_remote() {
            return if launch_claim {
                self.daemon
                    .session_resume_on_launch(session, Some(size.0), Some(size.1))
            } else {
                self.daemon
                    .session_resume(session, Some(size.0), Some(size.1))
            }
            .map(|session| session.id);
        }
        self.action::<SessionResult>(
            "resume",
            ActionRequest::Resume {
                id: 0,
                session: session.into(),
                cols: size.0,
                rows: size.1,
            },
        )
        .map(|s| s.session)
    }

    pub(crate) fn restart(&self, session: &str, size: (u16, u16)) -> ClientResult<String> {
        if !self.is_remote() {
            return self
                .daemon
                .session_restart(session, Some(size.0), Some(size.1))
                .map(|s| s.id);
        }
        self.action::<SessionResult>(
            "restart",
            ActionRequest::Restart {
                id: 0,
                session: session.into(),
                cols: size.0,
                rows: size.1,
            },
        )
        .map(|s| s.session)
    }

    pub(crate) fn fork(
        &self,
        session: &str,
        title: Option<String>,
        size: (u16, u16),
    ) -> ClientResult<String> {
        if !self.is_remote() {
            return self
                .daemon
                .session_fork(session, title, Some(size.0), Some(size.1))
                .map(|s| s.id);
        }
        self.action::<SessionResult>(
            "fork",
            ActionRequest::Fork {
                id: 0,
                session: session.into(),
                title,
                cols: size.0,
                rows: size.1,
            },
        )
        .map(|s| s.session)
    }

    pub(crate) fn close(&self, session: &str) -> ClientResult<()> {
        if !self.is_remote() {
            return self.daemon.session_close(session);
        }
        self.action(
            "close",
            ActionRequest::Close {
                id: 0,
                session: session.into(),
            },
        )
    }

    pub(crate) fn rename(&self, target: ActionTarget, name: String) -> ClientResult<()> {
        self.rename_title(target, Some(name))
    }

    pub(crate) fn rename_title(
        &self,
        target: ActionTarget,
        name: Option<String>,
    ) -> ClientResult<()> {
        if !self.is_remote() {
            return match target.kind {
                ActionKind::Session => self.daemon.session_rename(&target.id, name),
                ActionKind::Tab => self
                    .daemon
                    .node_rename(target.id, name.unwrap_or_default())
                    .map(drop),
                ActionKind::Mission => self
                    .daemon
                    .mission_rename_impl(target.id, name.unwrap_or_default())
                    .map(drop),
                ActionKind::Project => self
                    .daemon
                    .project_rename(target.id, name.unwrap_or_default())
                    .map(drop),
            };
        }
        self.action(
            "rename",
            ActionRequest::Rename {
                id: 0,
                target,
                name: name.unwrap_or_default(),
            },
        )
    }

    pub(crate) fn pin(
        &self,
        node: &super::model::SidebarNodeDisplay,
        pinned: bool,
    ) -> ClientResult<()> {
        if !self.is_remote() {
            return self
                .daemon
                .node_set_pinned(node.id.clone(), pinned)
                .map(drop);
        }
        self.action(
            "pin",
            ActionRequest::Pin {
                id: 0,
                target: node.target(),
                pinned,
            },
        )
    }

    pub(crate) fn move_item(
        &self,
        node: &super::model::SidebarNodeDisplay,
        project: Option<&super::model::SidebarNodeDisplay>,
        order: &[super::model::SidebarNodeDisplay],
    ) -> ClientResult<Option<Vec<runner_core::protocol::node::NodeRow>>> {
        if !self.is_remote() {
            return self
                .daemon
                .node_move(
                    node.id.clone(),
                    project.map(|project| project.id.clone()),
                    order.iter().map(|node| node.id.clone()).collect(),
                )
                .map(Some);
        }
        self.action::<()>(
            "move",
            ActionRequest::Move {
                id: 0,
                target: node.target(),
                project: project.map(|project| project.target().id),
                order: order.iter().map(|node| node.target()).collect(),
            },
        )
        .map(|()| None)
    }

    pub(crate) fn reorder_pinned(
        &self,
        order: &[super::model::SidebarNodeDisplay],
    ) -> ClientResult<Option<Vec<runner_core::protocol::node::NodeRow>>> {
        if !self.is_remote() {
            return self
                .daemon
                .node_reorder_pinned(order.iter().map(|node| node.id.clone()).collect())
                .map(Some);
        }
        self.action::<()>(
            "reorderPinned",
            ActionRequest::ReorderPinned {
                id: 0,
                order: order.iter().map(|node| node.target()).collect(),
            },
        )
        .map(|()| None)
    }

    pub(crate) fn create_project(&self, name: String, path: String) -> ClientResult<String> {
        if !self.is_remote() {
            return self
                .daemon
                .project_create(name, path)
                .map(|project| project.id);
        }
        #[derive(serde::Deserialize)]
        struct Created {
            project: String,
        }
        self.action::<Created>(
            "createProject",
            ActionRequest::CreateProject { id: 0, name, path },
        )
        .map(|result| result.project)
    }

    pub(crate) fn delete_project(&self, project: String, force: bool) -> ClientResult<()> {
        if !self.is_remote() {
            return if force {
                self.daemon.project_delete(project).map(drop)
            } else {
                self.daemon.project_delete_checked(project, false).map(drop)
            };
        }
        self.action::<serde_json::Value>(
            "deleteProject",
            ActionRequest::DeleteProject {
                id: 0,
                project,
                force,
            },
        )
        .map(drop)
    }

    pub(crate) fn stop_mission(&self, mission: String) -> ClientResult<()> {
        self.stop_mission_state(mission).map(drop)
    }

    pub(crate) fn stop_mission_state(&self, mission: String) -> ClientResult<MissionStop> {
        if !self.is_remote() {
            let state = self.daemon.mission_stop_impl(mission.clone())?;
            let sessions = self.daemon.session_list(&mission)?;
            return Ok(MissionStop {
                mission: Some(state),
                sessions,
            });
        }
        self.action::<()>("stopMission", ActionRequest::StopMission { id: 0, mission })
            .map(|()| MissionStop {
                mission: None,
                sessions: Vec::new(),
            })
    }

    pub(crate) fn rename_mission_state(
        &self,
        mission: String,
        title: String,
    ) -> ClientResult<Option<runner_core::protocol::model::Mission>> {
        if !self.is_remote() {
            return self.daemon.mission_rename_impl(mission, title).map(Some);
        }
        self.rename(
            ActionTarget {
                kind: ActionKind::Mission,
                id: mission,
            },
            title,
        )
        .map(|()| None)
    }

    /// Link a mission to a direct agent chat, or move its link (#748). The
    /// chat hears about it: the app is not the chat's own agent.
    pub(crate) fn link_mission(
        &self,
        mission: String,
        session: String,
    ) -> ClientResult<Option<runner_core::protocol::model::Mission>> {
        if !self.is_remote() {
            return self
                .daemon
                .mission_link(runner_core::protocol::MissionLinkInput {
                    mission_id: mission,
                    session_id: session,
                    no_report: false,
                    caller_session_id: None,
                })
                .map(Some);
        }
        self.action::<()>(
            "linkMission",
            ActionRequest::LinkMission {
                id: 0,
                mission,
                session,
            },
        )
        .map(|()| None)
    }

    pub(crate) fn unlink_mission(
        &self,
        mission: String,
    ) -> ClientResult<Option<runner_core::protocol::model::Mission>> {
        if !self.is_remote() {
            return self.daemon.mission_unlink(&mission).map(Some);
        }
        self.action::<()>(
            "unlinkMission",
            ActionRequest::UnlinkMission { id: 0, mission },
        )
        .map(|()| None)
    }

    pub(crate) fn archive_mission_state(
        &self,
        mission: String,
    ) -> ClientResult<Option<runner_core::protocol::model::Mission>> {
        if !self.is_remote() {
            return self.daemon.mission_archive_impl(mission).map(Some);
        }
        self.archive(ActionTarget {
            kind: ActionKind::Mission,
            id: mission,
        })
        .map(|()| None)
    }

    pub(crate) fn pin_mission_state(
        &self,
        mission: String,
        pinned: bool,
    ) -> ClientResult<Option<runner_core::protocol::model::Mission>> {
        if !self.is_remote() {
            return self.daemon.mission_pin_impl(mission, pinned).map(Some);
        }
        self.action::<()>(
            "pin",
            ActionRequest::Pin {
                id: 0,
                target: ActionTarget {
                    kind: ActionKind::Mission,
                    id: mission,
                },
                pinned,
            },
        )
        .map(|()| None)
    }

    #[cfg(test)]
    pub(crate) fn pin_chat(
        &self,
        session: &str,
        pinned: bool,
        tab: Option<&super::model::SidebarNodeDisplay>,
    ) -> ClientResult<()> {
        if !self.is_remote() {
            return self.daemon.session_pin(session, pinned);
        }
        let tab = tab.ok_or_else(|| ClientError::msg("The chat tab is no longer available."))?;
        self.pin(tab, pinned)
    }

    pub(crate) fn archive_chat(&self, session: &str, running: bool) -> ClientResult<()> {
        if !self.is_remote() {
            if running {
                let _ = self.daemon.session_kill(session);
            }
            return self.daemon.session_archive(session);
        }
        self.archive(ActionTarget {
            kind: ActionKind::Session,
            id: session.into(),
        })
    }

    pub(crate) fn archive_chat_with_missions(
        &self,
        session: &str,
        linked_missions: &[String],
    ) -> ClientResult<Vec<String>> {
        if !self.is_remote() {
            return self
                .daemon
                .session_archive_direct_with_missions(session, true);
        }
        let value: serde_json::Value = self.action(
            "archive",
            ActionRequest::Archive {
                id: 0,
                target: ActionTarget {
                    kind: ActionKind::Session,
                    id: session.into(),
                },
                with_missions: true,
            },
        )?;
        if let Some(archived) = value
            .get("archivedMissions")
            .and_then(|entry| entry.as_array())
        {
            return Ok(archived
                .iter()
                .filter_map(|id| id.as_str().map(str::to_owned))
                .collect());
        }
        // Older remote hosts ignore `withMissions` and return null; archive
        // each resolved linked mission explicitly so both peers end up matching.
        let mut archived = Vec::with_capacity(linked_missions.len());
        for mission_id in linked_missions {
            self.archive(ActionTarget {
                kind: ActionKind::Mission,
                id: mission_id.clone(),
            })?;
            archived.push(mission_id.clone());
        }
        Ok(archived)
    }

    pub(crate) fn archive_sessions(
        &self,
        plan: Vec<(String, ArchiveSessionOperation)>,
    ) -> (Vec<String>, Vec<String>, Option<String>) {
        let mut removed = Vec::new();
        let mut archived_missions = Vec::new();
        for (session, operation) in plan {
            let result = match operation {
                ArchiveSessionOperation::CloseTerminal => self.close(&session).map(|()| Vec::new()),
                ArchiveSessionOperation::ArchiveChat { running } => {
                    self.archive_chat(&session, running).map(|()| Vec::new())
                }
                ArchiveSessionOperation::ArchiveChatWithMissions {
                    linked_missions, ..
                } => self.archive_chat_with_missions(&session, &linked_missions),
            };
            match result {
                Ok(missions) => {
                    removed.push(session);
                    for mission_id in missions {
                        if !archived_missions.contains(&mission_id) {
                            archived_missions.push(mission_id);
                        }
                    }
                }
                Err(error) => return (removed, archived_missions, Some(error.to_string())),
            }
        }
        (removed, archived_missions, None)
    }

    pub(crate) fn resume_mission(
        &self,
        mission: String,
        size: (u16, u16),
    ) -> ClientResult<Vec<String>> {
        let result = self.resume_mission_state(mission, size)?;
        if let Some(error) = result.error {
            return Err(ClientError::msg(error));
        }
        Ok(result.sessions)
    }

    pub(crate) fn resume_mission_state(
        &self,
        mission: String,
        size: (u16, u16),
    ) -> ClientResult<MissionResume> {
        if !self.is_remote() {
            return super::local::resume_mission(&self.daemon, &mission, size);
        }
        #[derive(serde::Deserialize)]
        struct Resumed {
            sessions: Vec<String>,
        }
        self.action::<Resumed>(
            "resumeMission",
            ActionRequest::ResumeMission {
                id: 0,
                mission,
                cols: size.0,
                rows: size.1,
            },
        )
        .map(|result| MissionResume {
            sessions: result.sessions,
            local_sessions: Vec::new(),
            error: None,
        })
    }

    pub(crate) fn archive(&self, target: ActionTarget) -> ClientResult<()> {
        if !self.is_remote() {
            return match target.kind {
                ActionKind::Session => self.daemon.session_archive_direct(&target.id),
                ActionKind::Tab => self.daemon.node_tab_delete(&target.id),
                ActionKind::Mission => self.daemon.mission_archive_impl(target.id).map(drop),
                ActionKind::Project => Err(ClientError::msg("Projects use deleteProject.")),
            };
        }
        self.action(
            "archive",
            ActionRequest::Archive {
                id: 0,
                target,
                with_missions: false,
            },
        )
    }

    pub(crate) fn start_mission(
        &self,
        crew: String,
        title: String,
        goal: Option<String>,
        project: Option<String>,
        cwd: Option<String>,
        size: (u16, u16),
    ) -> ClientResult<String> {
        if !self.is_remote() {
            return self
                .daemon
                .mission_start_impl_with_size(
                    runner_core::protocol::mission::MissionStart {
                        crew_id: (!crew.is_empty()).then_some(crew),
                        scope: runner_core::protocol::project::ProjectScope::or_root(project),
                        title,
                        goal_override: goal,
                        cwd,
                        started_by_session_id: None,
                        no_report: false,
                    },
                    Some(size),
                )
                .map(|output| output.mission.id);
        }
        #[derive(serde::Deserialize)]
        struct Started {
            mission: String,
        }
        self.action::<Started>(
            "startMission",
            ActionRequest::StartMission {
                id: 0,
                crew,
                title,
                goal,
                project,
                cwd,
                cols: size.0,
                rows: size.1,
            },
        )
        .map(|result| result.mission)
    }

    pub(crate) fn mission_layout(&self, mission: String, shells: Vec<String>) -> ClientResult<()> {
        if !self.is_remote() {
            let row = self
                .daemon
                .node_list()?
                .into_iter()
                .find(|row| {
                    row.node_type == runner_core::protocol::node::NodeType::Mission
                        && row.ref_id.as_deref() == Some(&mission)
                })
                .ok_or_else(|| ClientError::msg("mission node is missing"))?;
            let mut layout = runner_app::pane_layout::MissionLayout::from_node_row(&row)
                .map_err(|error| ClientError::msg(error.to_string()))?;
            let open = layout.drawer.open();
            let active = layout.drawer.active_shell().map(str::to_owned);
            let height = layout.drawer.height();
            layout.drawer = Default::default();
            for shell in shells {
                layout.drawer.add(shell);
            }
            if let Some(active) = active {
                layout.drawer.activate(&active);
            }
            layout.drawer.set_height(height);
            layout
                .drawer
                .set_open(open && !layout.drawer.shells().is_empty());
            return self.save_mission_presentation(Some(&row.id), &layout);
        }
        self.action::<serde_json::Value>(
            "layout",
            ActionRequest::Layout {
                id: 0,
                target: ActionTarget {
                    kind: ActionKind::Mission,
                    id: mission,
                },
                project: None,
                name: String::new(),
                panes: runner_core::companion::ActionPane::Leaf {
                    id: "drawer".into(),
                    session: None,
                },
                shells,
            },
        )
        .map(drop)
    }

    fn message<T: DeserializeOwned>(
        &self,
        feature: &str,
        request: ClientMessage,
    ) -> ClientResult<T> {
        let FrontPageTarget::Remote { computer, .. } = &self.target else {
            return Err(ClientError::msg("This operation requires a remote target."));
        };
        if !self.supports(&[feature]) {
            return Err(ClientError::msg(
                "This action is unavailable on the selected computer.",
            ));
        }
        let result = self.daemon.remote_request(computer.clone(), request)?;
        if !self.valid() {
            return Err(ClientError::msg("The selected computer changed."));
        }
        serde_json::from_value(result.result)
            .map_err(|_| ClientError::msg("The computer returned an invalid result."))
    }

    pub(crate) fn feed(
        &self,
        mission: String,
        before: Option<String>,
    ) -> ClientResult<runner_core::companion::FeedPage> {
        self.message(
            "feed",
            ClientMessage::Feed(runner_core::companion::FeedRequest {
                id: 0,
                mission,
                before,
                limit: 100,
            }),
        )
    }

    pub(crate) fn follow_feed(&self, mission: String) -> ClientResult<()> {
        if !self.is_remote() {
            return Ok(());
        }
        self.message(
            "follow",
            ClientMessage::Follow(runner_core::companion::MissionRequest { id: 0, mission }),
        )
    }

    pub(crate) fn unfollow_feed(&self, mission: String) -> ClientResult<()> {
        if !self.is_remote() {
            return Ok(());
        }
        self.message(
            "unfollow",
            ClientMessage::Unfollow(runner_core::companion::MissionRequest { id: 0, mission }),
        )
    }

    pub(crate) fn answer(
        &self,
        mission: String,
        question: String,
        text: String,
    ) -> ClientResult<()> {
        if !self.is_remote() {
            return self
                .daemon
                .mission_signal_impl(runner_core::protocol::mission::PostSignalInput {
                    mission_id: mission,
                    from: None,
                    signal_type: "human_response".into(),
                    payload: serde_json::json!({"question_id": question, "choice": text}),
                })
                .map(drop);
        }
        self.message(
            "answer",
            ClientMessage::Answer(runner_core::companion::AnswerRequest {
                id: 0,
                mission,
                question,
                text,
            }),
        )
    }

    pub(crate) fn post(&self, mission: String, to: Vec<String>, text: String) -> ClientResult<()> {
        if !self.is_remote() {
            return self
                .daemon
                .mission_post_impl(runner_core::protocol::mission::PostMessageInput {
                    mission_id: mission,
                    from: None,
                    to: runner_core::model::Recipients::new(to),
                    text,
                })
                .map(drop);
        }
        self.message(
            "post",
            ClientMessage::Post(runner_core::companion::PostRequest {
                id: 0,
                mission,
                to,
                text,
            }),
        )
    }

    pub(crate) fn save_mission_presentation(
        &self,
        node: Option<&str>,
        layout: &runner_app::pane_layout::MissionLayout,
    ) -> ClientResult<()> {
        if self.is_remote() {
            return Ok(());
        }
        let node = node.ok_or_else(|| ClientError::msg("mission node is missing"))?;
        let layout = layout
            .serialize()
            .map_err(|error| ClientError::msg(error.to_string()))?;
        self.daemon.node_mission_layout_set(node, layout).map(drop)
    }
}

#[derive(serde::Deserialize)]
struct SessionResult {
    session: String,
}

#[derive(Clone)]
pub(crate) struct ChatStart {
    pub(crate) role: Option<String>,
    pub(crate) runtime: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
    pub(crate) speed: Option<String>,
    pub(crate) project: Option<String>,
    pub(crate) cwd: Option<String>,
    pub(crate) size: (u16, u16),
}

pub(crate) struct MissionResume {
    pub(crate) sessions: Vec<String>,
    pub(crate) local_sessions: Vec<runner_core::protocol::session::SessionRow>,
    pub(crate) error: Option<String>,
}

pub(crate) fn is_concurrent_resume_error(error: &str) -> bool {
    [
        "is already being resumed",
        "is already running — attach instead",
    ]
    .iter()
    .any(|fragment| error.contains(fragment))
}

pub(crate) struct MissionStop {
    pub(crate) mission: Option<runner_core::protocol::model::Mission>,
    pub(crate) sessions: Vec<runner_core::protocol::session::SessionRow>,
}

pub(crate) struct ChatLaunch {
    pub(crate) input: ChatStart,
    pub(crate) placement: Option<(runner_app::pane_layout::PaneLayout, String)>,
    pub(crate) started: Option<String>,
}

pub(crate) struct ChatLaunched {
    pub(crate) session: String,
    pub(crate) layout: Option<runner_app::pane_layout::PaneLayout>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ArchiveSessionOperation {
    ArchiveChat {
        running: bool,
    },
    ArchiveChatWithMissions {
        running: bool,
        linked_missions: Vec<String>,
    },
    CloseTerminal,
}

impl ArchiveSessionOperation {
    pub(crate) fn linked_missions(&self) -> &[String] {
        match self {
            Self::ArchiveChatWithMissions {
                linked_missions, ..
            } => linked_missions,
            Self::ArchiveChat { .. } | Self::CloseTerminal => &[],
        }
    }
}
