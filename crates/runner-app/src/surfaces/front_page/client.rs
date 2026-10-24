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
            .remote_request(computer.clone(), ClientMessage::Action(request))?;
        if !self.valid() {
            return Err(ClientError::msg("The selected computer changed."));
        }
        serde_json::from_value(result.result)
            .map_err(|_| ClientError::msg("The computer returned an invalid action result."))
    }

    pub(crate) fn catalog(&self) -> ClientResult<Catalog> {
        self.action("catalog", ActionRequest::Catalog { id: 0 })
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
        if !self.is_remote() {
            return self
                .daemon
                .session_resume(session, Some(size.0), Some(size.1))
                .map(|s| s.id);
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
        if !self.is_remote() {
            return match target.kind {
                ActionKind::Session => self.daemon.session_rename(&target.id, Some(name)),
                ActionKind::Tab => self.daemon.node_rename(target.id, name).map(drop),
                ActionKind::Mission => self.daemon.mission_rename_impl(target.id, name).map(drop),
                ActionKind::Project => self.daemon.project_rename(target.id, name).map(drop),
            };
        }
        self.action(
            "rename",
            ActionRequest::Rename {
                id: 0,
                target,
                name,
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
    ) -> ClientResult<()> {
        if !self.is_remote() {
            return self
                .daemon
                .node_move(
                    node.id.clone(),
                    project.map(|p| p.id.clone()),
                    order.iter().map(|item| item.id.clone()).collect(),
                )
                .map(drop);
        }
        self.action(
            "move",
            ActionRequest::Move {
                id: 0,
                target: node.target(),
                project: project.map(|p| p.target().id),
                order: order.iter().map(|item| item.target()).collect(),
            },
        )
    }

    pub(crate) fn reorder_pinned(
        &self,
        order: &[super::model::SidebarNodeDisplay],
    ) -> ClientResult<()> {
        if !self.is_remote() {
            return self
                .daemon
                .node_reorder_pinned(order.iter().map(|item| item.id.clone()).collect())
                .map(drop);
        }
        self.action(
            "reorderPinned",
            ActionRequest::ReorderPinned {
                id: 0,
                order: order.iter().map(|item| item.target()).collect(),
            },
        )
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
            return self.daemon.project_delete_checked(project, force).map(drop);
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
        if !self.is_remote() {
            return self.daemon.mission_stop_impl(mission).map(drop);
        }
        self.action("stopMission", ActionRequest::StopMission { id: 0, mission })
    }

    pub(crate) fn resume_mission(
        &self,
        mission: String,
        size: (u16, u16),
    ) -> ClientResult<Vec<String>> {
        if !self.is_remote() {
            return self.daemon.mission_resume(&mission).map(|_| Vec::new());
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
        .map(|result| result.sessions)
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
        self.action("archive", ActionRequest::Archive { id: 0, target })
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

    pub(crate) fn mission_pin(&self, mission: String, pinned: bool) -> ClientResult<()> {
        self.action(
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
        self.message(
            "follow",
            ClientMessage::Follow(runner_core::companion::MissionRequest { id: 0, mission }),
        )
    }

    pub(crate) fn unfollow_feed(&self, mission: String) -> ClientResult<()> {
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
