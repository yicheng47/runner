use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
};

use runner_core::{companion::*, protocol::terminal::TerminalAttachment};
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::{events::AppEvent, repo, AppCore};

pub type Result<T> = std::result::Result<T, Failure>;
#[derive(Debug, Clone)]
pub struct Failure(
    pub ErrorCode,
    pub &'static str,
    pub Option<runner_core::protocol::RuntimeVersionGate>,
);

pub struct Attachment {
    pub terminal: TerminalAttachment,
    pub resync: Arc<dyn Fn() -> Result<TerminalAttachment> + Send + Sync>,
}

pub trait Core: Send + Sync {
    fn action(&self, _: ActionRequest) -> Result<serde_json::Value> {
        Err(Failure(
            ErrorCode::UnknownRequest,
            "The host does not serve that request.",
            None,
        ))
    }
    fn feed(&self, _: FeedRequest) -> Result<FeedPage> {
        Err(Failure(
            ErrorCode::UnknownRequest,
            "The host does not serve that request.",
            None,
        ))
    }
    fn feed_cursor(&self, _: &str) -> Result<u64> {
        Err(Failure(
            ErrorCode::UnknownRequest,
            "The host does not serve that request.",
            None,
        ))
    }
    fn feed_since(&self, _: &str, _: u64) -> Result<super::feed::Updates> {
        Err(Failure(
            ErrorCode::UnknownRequest,
            "The host does not serve that request.",
            None,
        ))
    }
    fn answer(&self, _: AnswerRequest) -> Result<()> {
        Err(Failure(
            ErrorCode::UnknownRequest,
            "The host does not serve that request.",
            None,
        ))
    }
    fn post(&self, _: PostRequest) -> Result<()> {
        Err(Failure(
            ErrorCode::UnknownRequest,
            "The host does not serve that request.",
            None,
        ))
    }
    fn overview(&self) -> Result<Overview>;
    fn statuses(&self) -> Result<BTreeMap<String, SessionStatus>>;
    fn subscribe(&self) -> Result<broadcast::Receiver<AppEvent>>;
    fn attach(self: Arc<Self>, session: &str, history: usize, channel: u64) -> Result<Attachment>;
    fn resize(
        &self,
        session: &str,
        channel: u64,
        name: &str,
        size: (u16, u16),
        cell_size: (u16, u16),
        control: Control,
    ) -> Result<()>;
    fn input(&self, session: &str, channel: u64, bytes: &[u8]) -> Result<()>;
    fn detach(&self, session: &str, subscriber: u64, channel: u64, release: bool);
    fn closed_reason(&self, session: &str) -> ClosedReason;
    fn release_channel(&self, channel: u64);
    fn sync_overview_watches(
        &self,
        _channel: u64,
        _to_watch: Vec<std::path::PathBuf>,
        _to_unwatch: Vec<std::path::PathBuf>,
    ) {
    }
}

const COMPANION_WATCH_CLIENT_BASE: u64 = 1u64 << 62;

fn companion_watch_client(channel: u64) -> u64 {
    COMPANION_WATCH_CLIENT_BASE ^ channel
}
#[derive(Clone, Copy)]
pub enum Control {
    Take,
    IfFree,
    Viewport,
}

pub struct DaemonCore(pub Weak<AppCore>);
impl DaemonCore {
    pub(super) fn core(&self) -> Result<std::sync::Arc<AppCore>> {
        self.0
            .upgrade()
            .ok_or(Failure(ErrorCode::Failed, "The host has stopped.", None))
    }
    fn terminal(&self, session: &str) -> Result<std::sync::Arc<AppCore>> {
        let core = self.core()?;
        let row_exists = {
            let conn = core.db.get().map_err(failed)?;
            repo::session::get_row(&conn, session)
                .map_err(failed)?
                .is_some()
        };
        let update_task = core
            .sessions
            .runtime_updates
            .lock()
            .unwrap()
            .tasks
            .iter()
            .any(|task| task.id == session);
        if !row_exists && !update_task {
            return Err(Failure(
                ErrorCode::NotFound,
                "No session has that id.",
                None,
            ));
        }
        if !core.sessions.session_live(session) && !update_task {
            return Err(Failure(
                ErrorCode::NotRunning,
                "The session has no live terminal.",
                None,
            ));
        }
        Ok(core)
    }
}
pub(super) fn failed(_: impl std::fmt::Display) -> Failure {
    Failure(
        ErrorCode::Failed,
        "The host could not complete the request.",
        None,
    )
}
impl Core for DaemonCore {
    fn statuses(&self) -> Result<BTreeMap<String, SessionStatus>> {
        crate::ops::session::session_status_snapshot(self.core()?.as_ref())
            .map(|statuses| {
                statuses
                    .into_iter()
                    .map(|(id, status)| (id, SessionStatus::from(&status)))
                    .collect()
            })
            .map_err(failed)
    }
    fn action(&self, request: ActionRequest) -> Result<serde_json::Value> {
        DaemonCore::action(self, request)
    }
    fn feed(&self, request: FeedRequest) -> Result<FeedPage> {
        self.feed_page(request)
    }
    fn feed_cursor(&self, mission: &str) -> Result<u64> {
        DaemonCore::feed_cursor(self, mission)
    }
    fn feed_since(&self, mission: &str, cursor: u64) -> Result<super::feed::Updates> {
        self.updates(mission, cursor)
    }
    fn answer(&self, request: AnswerRequest) -> Result<()> {
        self.answer_question(request)
    }
    fn post(&self, request: PostRequest) -> Result<()> {
        self.post_message(request)
    }
    fn overview(&self) -> Result<Overview> {
        overview(self.core()?.as_ref()).map_err(failed)
    }
    fn subscribe(&self) -> Result<broadcast::Receiver<AppEvent>> {
        Ok(self.core()?.events.subscribe())
    }
    fn attach(self: Arc<Self>, session: &str, history: usize, channel: u64) -> Result<Attachment> {
        let core = self.terminal(session)?;
        let (terminal, source) = core
            .sessions
            .attach_companion_terminal(session, history)
            .map_err(failed)?;
        core.sessions
            .browser_attached(session, terminal.subscriber_id, channel, &core.db)
            .map_err(failed)?;
        let session = session.to_owned();
        let core = Arc::downgrade(&core);
        Ok(Attachment {
            terminal,
            resync: Arc::new(move || {
                let attachment = source.attach().map_err(failed)?;
                let core = core.upgrade().ok_or(Failure(
                    ErrorCode::Failed,
                    "The host has stopped.",
                    None,
                ))?;
                core.sessions
                    .browser_attached(&session, attachment.subscriber_id, channel, &core.db)
                    .map_err(failed)?;
                Ok(attachment)
            }),
        })
    }
    fn resize(
        &self,
        session: &str,
        channel: u64,
        name: &str,
        size: (u16, u16),
        cell_size: (u16, u16),
        control: Control,
    ) -> Result<()> {
        let core = self.terminal(session)?;
        core.sessions
            .browser_resize(
                session,
                channel,
                name,
                size.0,
                size.1,
                cell_size.0,
                cell_size.1,
                matches!(control, Control::Take),
                matches!(control, Control::Viewport),
                &core.db,
            )
            .map_err(failed)?;
        if matches!(control, Control::Take) {
            crate::ops::node::mark_direct_sessions_viewed(&core, &[session.to_owned()])
                .map_err(failed)?;
        }
        Ok(())
    }
    fn input(&self, session: &str, channel: u64, bytes: &[u8]) -> Result<()> {
        let core = self.terminal(session)?;
        core.sessions
            .browser_input(session, channel, bytes)
            .map_err(failed)?;
        if !bytes.is_empty() {
            let status = core.sessions.agent_status(session);
            let needs_viewed = status.unread_since.is_some()
                || status.error_since.is_some()
                || ((bytes.contains(&b'\r') || bytes.contains(&b'\n'))
                    && crate::repo::session_attention::any_unread(
                        &*core.db.get().map_err(failed)?,
                        std::slice::from_ref(&session.to_owned()),
                    )
                    .map_err(failed)?);
            if needs_viewed {
                crate::ops::node::mark_direct_sessions_viewed(&core, &[session.to_owned()])
                    .map_err(failed)?;
            }
        }
        Ok(())
    }
    fn detach(&self, session: &str, subscriber: u64, channel: u64, release: bool) {
        if let Ok(core) = self.core() {
            core.sessions.detach_terminal(session, subscriber);
            if let Err(error) = core
                .sessions
                .browser_detached(session, subscriber, channel, release, &core.db)
            {
                log::warn!("release terminal control: {error}");
            }
        }
    }
    fn release_channel(&self, channel: u64) {
        if let Ok(core) = self.core() {
            core.sessions.release_browser_channel(channel);
            core.files.client_gone(companion_watch_client(channel));
        }
    }
    fn sync_overview_watches(
        &self,
        channel: u64,
        to_watch: Vec<std::path::PathBuf>,
        to_unwatch: Vec<std::path::PathBuf>,
    ) {
        let Ok(core) = self.core() else {
            return;
        };
        let client_id = companion_watch_client(channel);
        for root in to_unwatch {
            core.files.unwatch(client_id, &root);
        }
        for root in to_watch {
            let _ = core.files.watch(client_id, root);
        }
    }
    fn closed_reason(&self, session: &str) -> ClosedReason {
        let Ok(core) = self.core() else {
            return ClosedReason::Unknown;
        };
        let row = {
            let Ok(conn) = core.db.get() else {
                return ClosedReason::Unknown;
            };
            let Ok(Some(row)) = repo::session::get_row(&conn, session) else {
                return ClosedReason::Unknown;
            };
            row
        };
        if row.archived_at.is_some() {
            return ClosedReason::Archived;
        }
        if core.sessions.terminal_was_stopped(session) {
            return ClosedReason::Stopped;
        }
        if matches!(
            row.status,
            crate::model::SessionStatus::Stopped | crate::model::SessionStatus::Crashed
        ) {
            ClosedReason::Exited
        } else {
            ClosedReason::Unknown
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Layout {
    tree: Option<Tree>,
    slots: Vec<Option<String>>,
    preset: String,
    sizes: BTreeMap<String, serde_json::Value>,
    drawer: Drawer,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct Drawer {
    shells: Vec<String>,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Tree {
    Leaf {},
    Split {
        orientation: SplitDirection,
        sizes: [f64; 2],
        a: Box<Tree>,
        b: Box<Tree>,
    },
}
fn split(direction: SplitDirection, sizes: [f64; 2], a: Pane, b: Pane) -> Pane {
    let total = sizes[0] + sizes[1];
    let sizes = if total.is_finite() && sizes.iter().all(|size| *size > 0.) {
        [sizes[0] / total, sizes[1] / total]
    } else {
        [0.5, 0.5]
    };
    Pane::Split {
        split: direction,
        sizes,
        a: Box::new(a),
        b: Box::new(b),
    }
}
impl Tree {
    fn pane(self, slots: &mut std::slice::Iter<'_, Option<String>>) -> Pane {
        match self {
            Self::Leaf {} => Pane::Leaf {
                session: slots.next().cloned().flatten(),
            },
            Self::Split {
                orientation,
                sizes,
                a,
                b,
            } => split(orientation, sizes, a.pane(slots), b.pane(slots)),
        }
    }
}
impl Layout {
    fn pane(&mut self) -> Pane {
        if let Some(tree) = self.tree.take() {
            return tree.pane(&mut self.slots.iter());
        }
        let leaf = |i| Pane::Leaf {
            session: self.slots.get(i).cloned().flatten(),
        };
        let direction = if self.preset.starts_with("rows") {
            SplitDirection::Column
        } else {
            SplitDirection::Row
        };
        let size = |id: &str, fallback| {
            self.sizes
                .get(id)
                .and_then(|value| serde_json::from_value::<[f64; 2]>(value.clone()).ok())
                .unwrap_or(fallback)
        };
        match self.preset.as_str() {
            "cols-2" | "cols2" => split(
                direction,
                size("cols-2:outer", [50., 50.]),
                leaf(0),
                leaf(1),
            ),
            "rows-2" | "rows2" => split(
                direction,
                size("rows-2:outer", [50., 50.]),
                leaf(0),
                leaf(1),
            ),
            "main-2" | "main2" => split(
                direction,
                size("main-2:outer", [60., 40.]),
                leaf(0),
                split(
                    SplitDirection::Column,
                    size("main-2:inner", [50., 50.]),
                    leaf(1),
                    leaf(2),
                ),
            ),
            "cols-3" | "cols3" | "rows-3" | "rows3" => {
                let prefix = if self.preset.starts_with("rows") {
                    "rows-3"
                } else {
                    "cols-3"
                };
                split(
                    direction.clone(),
                    size(&format!("{prefix}:outer"), [33.33, 66.67]),
                    leaf(0),
                    split(
                        direction,
                        size(&format!("{prefix}:inner"), [50., 50.]),
                        leaf(1),
                        leaf(2),
                    ),
                )
            }
            _ => leaf(0),
        }
    }
}
fn sidebar_order(nodes: &[repo::node::NodeRow]) -> Vec<&repo::node::NodeRow> {
    fn children<'a>(
        nodes: &'a [repo::node::NodeRow],
        parent: Option<&str>,
        out: &mut Vec<&'a repo::node::NodeRow>,
    ) {
        for node in nodes
            .iter()
            .filter(|node| node.parent_id.as_deref() == parent && node.pinned_position.is_none())
        {
            out.push(node);
            if node.node_type == repo::node::NodeType::Project {
                children(nodes, Some(&node.id), out);
            }
        }
    }
    let mut out: Vec<_> = nodes
        .iter()
        .filter(|node| node.pinned_position.is_some())
        .collect();
    children(nodes, None, &mut out);
    out
}

fn derive_overview_checkout(
    core: &AppCore,
    cwd: Option<&str>,
    project_id: Option<&str>,
    checkout_cache: &mut std::collections::HashMap<(String, Option<String>), Option<Checkout>>,
    auto_base_cache: &mut std::collections::HashMap<std::path::PathBuf, Option<String>>,
) -> Option<Checkout> {
    let cwd = cwd.map(str::trim).filter(|cwd| !cwd.is_empty())?;
    let key = (cwd.to_owned(), project_id.map(str::to_owned));
    if let Some(cached) = checkout_cache.get(&key) {
        return cached.clone();
    }
    let derived =
        crate::files::worktree::worktree_checkout(core, std::path::Path::new(cwd), project_id)
            .ok()
            .map(|checkout| {
                let auto_base = match &checkout {
                    runner_core::protocol::worktree::WorktreeCheckout::Git {
                        top,
                        kind: runner_core::protocol::files::WorktreeKind::Linked,
                        missing: false,
                        ..
                    } => auto_base_cache
                        .entry(top.clone())
                        .or_insert_with(|| {
                            crate::files::changes::worktree_auto_base(&core.files, top)
                        })
                        .clone(),
                    _ => None,
                };
                Checkout::from_protocol(&checkout, auto_base)
            });
    checkout_cache.insert(key, derived.clone());
    derived
}

fn overview(core: &AppCore) -> crate::error::Result<Overview> {
    let nodes = crate::ops::node::node_list(core)?;
    let statuses = crate::ops::session::session_status_snapshot(core)?;
    let mut overview = Overview {
        projects: vec![],
        tabs: vec![],
        missions: vec![],
        sessions: BTreeMap::new(),
        sidebar: None,
        terminal_palette: Some(crate::session::manager::terminal::palette(core).into()),
    };
    let mut checkout_cache = std::collections::HashMap::new();
    let mut auto_base_cache = std::collections::HashMap::new();
    let mut ids = Vec::new();
    for node in sidebar_order(&nodes) {
        match node.node_type {
            repo::node::NodeType::Project => {
                let conn = core.db.get()?;
                if let Some(project) = node
                    .ref_id
                    .as_deref()
                    .map(|id| repo::project::get(&conn, id))
                    .transpose()?
                    .flatten()
                {
                    overview.projects.push(Project {
                        id: project.id,
                        name: project.name,
                    });
                }
            }
            repo::node::NodeType::Tab => {
                let conn = core.db.get()?;
                let mut layout: Layout = node
                    .layout
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()?
                    .unwrap_or_default();
                ids.extend(repo::node::session_ids(node));
                overview.tabs.push(Tab {
                    id: node.id.clone(),
                    project: repo::node::effective_project(&conn, node.parent_id.as_deref())?,
                    name: node.name.clone().unwrap_or_default(),
                    pinned: node.pinned_position.is_some(),
                    panes: layout.pane(),
                    shells: layout.drawer.shells,
                });
            }
            repo::node::NodeType::Mission => {
                let Some(id) = node.ref_id.as_deref() else {
                    continue;
                };
                let snapshot = crate::ops::mission::feed::mission_status(core, id)?;
                if snapshot.mission.archived_at.is_some() {
                    continue;
                }
                let layout: Layout = node
                    .layout
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()?
                    .unwrap_or_default();
                ids.extend(layout.drawer.shells.iter().cloned());
                let members = snapshot
                    .sessions
                    .iter()
                    .map(|row| {
                        ids.push(row.session.id.clone());
                        Member {
                            handle: row.handle.clone(),
                            session: row.session.id.clone(),
                            lead: false,
                        }
                    })
                    .collect();
                let mission_checkout = derive_overview_checkout(
                    core,
                    snapshot.mission.cwd.as_deref(),
                    snapshot.mission.project_id.as_deref(),
                    &mut checkout_cache,
                    &mut auto_base_cache,
                );
                overview.missions.push(Mission {
                    id: id.into(),
                    project: snapshot.mission.project_id,
                    title: snapshot.mission.title,
                    crew: snapshot.crew.map(|crew| crew.name).unwrap_or_default(),
                    state: if snapshot.mission.status == crate::model::MissionStatus::Running {
                        MissionState::Running
                    } else {
                        MissionState::Stopped
                    },
                    pinned: node.pinned_position.is_some(),
                    members,
                    asks: snapshot
                        .pending_asks
                        .into_iter()
                        .map(|ask| Ask {
                            question: ask.question_id,
                            from: ask.asker,
                            prompt: ask.prompt,
                            choices: ask
                                .choices
                                .and_then(|choices| serde_json::from_value(choices).ok()),
                        })
                        .collect(),
                    shells: layout.drawer.shells,
                    linked_chat: snapshot.mission.linked_chat.map(Into::into),
                    phase: snapshot.phase.map(Into::into),
                    cwd: snapshot.mission.cwd,
                    checkout: mission_checkout,
                    removed_handles: snapshot.removed_handles,
                });
            }
        }
    }
    let mut pending_sessions = Vec::new();
    {
        let conn = core.db.get()?;
        for id in ids {
            if overview.sessions.contains_key(&id)
                || pending_sessions.iter().any(|(seen, _, _)| seen == &id)
            {
                continue;
            }
            let Some(row) = repo::session::get_row(&conn, &id)? else {
                continue;
            };
            if row.archived_at.is_some() {
                continue;
            }
            let runtime = repo::session::effective_runtime(&conn, &id)?;
            let shell = runtime
                .as_deref()
                .and_then(crate::model::Runtime::parse)
                .is_some_and(crate::model::Runtime::is_shell);
            let kind = if shell {
                SessionKind::Shell
            } else if row.mission_id.is_some() {
                SessionKind::Agent
            } else {
                SessionKind::Chat
            };
            let handle = if row.mission_id.is_some() {
                overview
                    .missions
                    .iter()
                    .flat_map(|mission| &mission.members)
                    .find(|member| member.session == id)
                    .map(|member| member.handle.clone())
            } else {
                repo::session::get_direct(&conn, &id)?.and_then(|entry| entry.role_handle)
            };
            let runtime_key = runtime.as_deref().unwrap_or_default();
            let display_name = crate::runtimes::for_key(runtime_key)
                .catalog()
                .map(|catalog| catalog.display_name)
                .unwrap_or(runtime_key);
            let command = row
                .launch_definition
                .as_ref()
                .map(|definition| definition.command.as_str())
                .or(row.agent_command.as_deref())
                .unwrap_or_default();
            let title = runner_core::protocol::session_title::preferred_title(
                row.title.as_deref(),
                row.live_title
                    .as_deref()
                    .filter(|_| row.mission_id.is_none()),
                None,
                runtime_key,
                row.cwd.as_deref(),
            )
            .unwrap_or_else(|| {
                runner_core::protocol::session_title::default_label(
                    runtime_key,
                    command,
                    handle.as_deref(),
                    display_name,
                )
            });
            let status = statuses
                .get(&id)
                .map(SessionStatus::from)
                .unwrap_or(SessionStatus {
                    startup_error: None,
                    status: Status::Idle,
                    reason: None,
                    unread: false,
                    error_acknowledged: false,
                    interrupted_since: None,
                });
            let known_setting = |value: Option<&str>| {
                value
                    .map(str::trim)
                    .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("default"))
                    .map(str::to_owned)
            };
            let model = (!shell)
                .then(|| {
                    known_setting(
                        row.launch_definition
                            .as_ref()
                            .and_then(|definition| definition.model.as_deref()),
                    )
                })
                .flatten();
            let effort = (!shell)
                .then(|| {
                    known_setting(
                        row.launch_definition
                            .as_ref()
                            .and_then(|definition| definition.effort.as_deref()),
                    )
                })
                .flatten();
            pending_sessions.push((
                id,
                row.project_id,
                Session {
                    kind,
                    title,
                    runtime: runtime.filter(|_| !shell),
                    model,
                    effort,
                    cwd: row.cwd,
                    status,
                    checkout: None,
                },
            ));
        }
    }
    for (id, project_id, mut session) in pending_sessions {
        session.checkout = derive_overview_checkout(
            core,
            session.cwd.as_deref(),
            project_id.as_deref(),
            &mut checkout_cache,
            &mut auto_base_cache,
        );
        overview.sessions.insert(id, session);
    }
    overview.sidebar = Some(
        nodes
            .iter()
            .filter_map(|node| {
                let (kind, id, parent) = match node.node_type {
                    repo::node::NodeType::Project => {
                        let project = overview
                            .projects
                            .iter()
                            .find(|project| node.ref_id.as_deref() == Some(&project.id))?;
                        (SidebarNodeKind::Project, project.id.clone(), None)
                    }
                    repo::node::NodeType::Tab => {
                        let tab = overview.tabs.iter().find(|tab| tab.id == node.id)?;
                        (SidebarNodeKind::Tab, tab.id.clone(), tab.project.clone())
                    }
                    repo::node::NodeType::Mission => {
                        let mission = overview
                            .missions
                            .iter()
                            .find(|mission| node.ref_id.as_deref() == Some(&mission.id))?;
                        (
                            SidebarNodeKind::Mission,
                            mission.id.clone(),
                            mission.project.clone(),
                        )
                    }
                };
                Some(SidebarNode {
                    kind,
                    id,
                    parent,
                    position: node.position,
                    pinned_position: node.pinned_position,
                })
            })
            .collect(),
    );
    Ok(overview)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn overview_session_titles_follow_the_latest_name() {
        let core = crate::test_support::test_core();
        let conn = core.db.get().unwrap();
        for (id, handle) in [("role-one", "planner"), ("role-two", "coder")] {
            crate::test_support::insert_test_role(&conn, id, handle, "pi", "pi");
        }
        let project = repo::project::create(&conn, "Project", "/tmp/titles").unwrap();
        let parent = repo::node::ensure_project_node(&conn, &project.id).unwrap();
        let mut tabs = Vec::new();
        for (position, (id, role, runtime, command, title, live, expected)) in [
            ("one", Some("role-one"), "pi", "pi", None, "pi", "@planner"),
            (
                "two",
                Some("role-two"),
                "pi",
                "pi",
                None,
                "π - runner-biz-plan - titles",
                "runner-biz-plan",
            ),
            (
                "named",
                Some("role-two"),
                "pi",
                "pi",
                Some("Release prep"),
                "π - Injected prompt - titles",
                "Release prep",
            ),
            (
                "topic",
                None,
                "pi",
                "pi",
                None,
                "π - Fix the parser - titles",
                "Fix the parser",
            ),
            ("plain", None, "pi", "pi", None, "π - titles", "pi"),
            ("shell", None, "shell", "/bin/zsh", None, "", "zsh"),
            (
                "shell-live",
                None,
                "shell",
                "/bin/zsh",
                None,
                "⠋ /tmp/titles",
                "⠋ /tmp/titles",
            ),
            (
                "shell-named",
                None,
                "shell",
                "/bin/zsh",
                Some("Build logs"),
                "/tmp/titles",
                "Build logs",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let mut row = repo::session::SessionRowDb::new_running(id.into());
            row.status = crate::model::SessionStatus::Stopped;
            row.role_id = role.map(str::to_owned);
            row.agent_runtime = Some(runtime.into());
            row.agent_command = Some(command.into());
            row.cwd = Some("/tmp/titles".into());
            row.title = title.map(str::to_owned);
            row.live_title = Some(live.into());
            repo::session::insert(&conn, &row).unwrap();
            let node = repo::node::create_tab(
                &conn,
                Some(&parent.id),
                "",
                position as i64,
                &serde_json::json!({"slots":[id]}).to_string(),
            )
            .unwrap();
            if role.is_some() {
                repo::node::set_pinned(&conn, &node.id, true).unwrap();
            }
            tabs.push((id, node.id, expected));
        }
        drop(conn);
        let before = overview(&core).unwrap();
        for (id, tab, expected) in &tabs {
            assert_eq!(before.sessions[*id].title, *expected, "{id}");
            let tab = before.tabs.iter().find(|item| item.id == *tab).unwrap();
            assert!(tab.name.is_empty());
            assert_eq!(tab.project.as_deref(), Some(project.id.as_str()));
        }
        assert_eq!(before.tabs.iter().filter(|tab| tab.pinned).count(), 3);
        assert_eq!(before.sessions["shell"].runtime, None);
        assert_eq!(before.sessions["one"].runtime.as_deref(), Some("pi"));
        let conn = core.db.get().unwrap();
        assert_eq!(
            repo::session::set_live_title(&conn, "one", Some("New provider topic"), None).unwrap(),
            1
        );
        assert_eq!(
            repo::session::set_live_title(&conn, "named", Some("Agent rename"), None).unwrap(),
            1
        );
        repo::session::set_title(&conn, "two", Some("User session name")).unwrap();
        repo::node::rename(&conn, &tabs[0].1, "User tab name").unwrap();
        drop(conn);
        let after = overview(&core).unwrap();
        assert_eq!(after.sessions["one"].title, "New provider topic");
        assert_eq!(after.sessions["two"].title, "User session name");
        assert_eq!(after.sessions["named"].title, "Agent rename");
        assert_eq!(
            after
                .tabs
                .iter()
                .find(|tab| tab.id == tabs[0].1)
                .unwrap()
                .name,
            "User tab name"
        );
    }

    #[test]
    fn overview_reads_nodes_layout_status_and_unread_without_retaining_the_core() {
        let core = Arc::new(crate::test_support::test_core());
        let conn = core.db.get().unwrap();
        crate::test_support::insert_test_role(&conn, "role", "coder", "codex", "codex");
        let project = repo::project::create(&conn, "Project", "/tmp/m5").unwrap();
        let parent = repo::node::ensure_project_node(&conn, &project.id).unwrap();
        for id in ["a", "b"] {
            let mut row = repo::session::SessionRowDb::new_running(id.into());
            row.role_id = Some("role".into());
            row.agent_runtime = Some("codex".into());
            row.project_id = Some(project.id.clone());
            row.cwd = Some("/tmp/m5".into());
            row.title = Some(format!("Chat {id}"));
            repo::session::insert(&conn, &row).unwrap();
        }
        repo::session_attention::record_completion(&conn, "a", false, 100).unwrap();
        let node = repo::node::NodeRow { id: "tab".into(), parent_id: Some(parent.id), position: 0, node_type: repo::node::NodeType::Tab, name: Some("Split".into()), ref_id: None, layout: Some(serde_json::json!({"slots":["a","b"],"tree":{"kind":"split","id":"s1","orientation":"column","sizes":[30,70],"a":{"kind":"leaf","id":"p1","session_id":"a"},"b":{"kind":"leaf","id":"p2","session_id":"b"}}}).to_string()), pinned_position: Some(0), last_completed_at: None, last_viewed_at: None, created_at: chrono::Utc::now().to_rfc3339() };
        repo::node::upsert(&conn, &node).unwrap();
        drop(conn);
        let backend = DaemonCore(Arc::downgrade(&core));
        let overview = backend.overview().unwrap();
        assert_eq!(
            overview.projects,
            vec![Project {
                id: project.id.clone(),
                name: "Project".into()
            }]
        );
        assert_eq!(overview.tabs.len(), 1);
        assert_eq!(
            overview.tabs[0].project.as_deref(),
            Some(project.id.as_str())
        );
        assert!(overview.tabs[0].pinned);
        assert!(
            matches!(&overview.tabs[0].panes, Pane::Split { split: SplitDirection::Column, sizes, .. } if *sizes == [0.3,0.7])
        );
        assert_eq!(overview.sessions["a"].title, "Chat a");
        assert!(overview.sessions["a"].status.unread);
        assert!(!overview.sessions["b"].status.unread);
        assert_eq!(overview.sessions["a"].runtime.as_deref(), Some("codex"));
        let sidebar = overview.sidebar.as_ref().unwrap();
        assert!(sidebar.contains(&SidebarNode {
            kind: SidebarNodeKind::Project,
            id: project.id.clone(),
            parent: None,
            position: parent.position,
            pinned_position: None,
        }));
        assert!(sidebar.contains(&SidebarNode {
            kind: SidebarNodeKind::Tab,
            id: node.id.clone(),
            parent: Some(project.id.clone()),
            position: node.position,
            pinned_position: Some(0),
        }));
        assert_eq!(Arc::strong_count(&core), 1);
        assert!(matches!(
            Arc::new(DaemonCore(backend.0.clone())).attach("missing", 0, 1),
            Err(Failure(ErrorCode::NotFound, _, _))
        ));
        assert!(matches!(
            Arc::new(DaemonCore(backend.0.clone())).attach("a", 0, 1),
            Err(Failure(ErrorCode::NotRunning, _, _))
        ));
        crate::ops::node::mark_direct_sessions_viewed(&core, &["a".into()]).unwrap();
        assert!(!backend.overview().unwrap().sessions["a"].status.unread);
        let moved = crate::ops::node::node_tab_upsert(&core, runner_core::protocol::node::NodeTabUpsertInput {
            id: ulid::Ulid::new().to_string(), parent_id: node.parent_id.clone(), name: "Moved".into(),
            layout: serde_json::json!({"slots":["a"],"tree":{"kind":"leaf","id":"p1","session_id":"a"}}).to_string(),
        }).unwrap();
        let after_move = backend.overview().unwrap();
        let source = after_move
            .tabs
            .iter()
            .find(|tab| tab.id == node.id)
            .unwrap();
        assert!(
            matches!(&source.panes, Pane::Split { a, b, .. } if **a == Pane::Leaf { session: None } && **b == Pane::Leaf { session: Some("b".into()) })
        );
        assert_eq!(
            after_move
                .tabs
                .iter()
                .find(|tab| tab.id == moved.id)
                .unwrap()
                .panes,
            Pane::Leaf {
                session: Some("a".into())
            }
        );
        let stored: serde_json::Value = serde_json::from_str(
            repo::node::get(&core.db.get().unwrap(), &node.id)
                .unwrap()
                .unwrap()
                .layout
                .as_deref()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(stored["tree"]["a"]["session_id"], "a");
        assert_eq!(stored["slots"][0], serde_json::Value::Null);
        core.db
            .get()
            .unwrap()
            .execute("UPDATE sessions SET status = 'stopped' WHERE id = 'a'", [])
            .unwrap();
        crate::ops::session::session_archive(&core, "a").unwrap();
        let after_archive = backend.overview().unwrap();
        assert!(!after_archive.sessions.contains_key("a"));
        assert!(
            matches!(&after_archive.tabs.iter().find(|tab| tab.id == node.id).unwrap().panes, Pane::Split { a, .. } if **a == Pane::Leaf { session: None })
        );
        drop(core);
        assert!(backend.overview().is_err());
    }

    #[test]
    fn overview_carries_the_host_palette_and_announces_its_changes() {
        let core = crate::test_support::test_core();
        let wire = runner_terminal::terminal::palette_to_wire;
        assert_eq!(
            overview(&core).unwrap().terminal_palette,
            Some(wire(runner_terminal::palette::RUNNER).into())
        );
        let mut events = core.events.subscribe();
        let light = wire(runner_terminal::palette::ROSE_PINE_DAWN);
        for _ in 0..2 {
            crate::session::manager::terminal::set_palette(&core, light.clone()).unwrap();
        }
        assert_eq!(
            overview(&core).unwrap().terminal_palette,
            Some(light.into())
        );
        let announced = std::iter::from_fn(|| events.try_recv().ok())
            .filter(|event| event.name == "terminal/palette")
            .count();
        assert_eq!(announced, 1);
    }
    #[test]
    fn legacy_layouts_keep_their_directions_and_saved_fractions() {
        for (preset, direction, count) in [
            ("single", SplitDirection::Row, 1),
            ("cols-2", SplitDirection::Row, 2),
            ("rows-2", SplitDirection::Column, 2),
            ("cols-3", SplitDirection::Row, 3),
            ("rows-3", SplitDirection::Column, 3),
            ("main-2", SplitDirection::Row, 3),
        ] {
            let mut layout: Layout = serde_json::from_value(serde_json::json!({"preset":preset,"slots":["a","b","c"],"sizes":{"cols-2:outer":[40,60]}})).unwrap();
            let pane = layout.pane();
            if count > 1 {
                assert!(matches!(&pane, Pane::Split { split, .. } if *split == direction));
            }
            if preset == "cols-2" {
                assert!(matches!(
                    pane,
                    Pane::Split {
                        sizes: [0.4, 0.6],
                        ..
                    }
                ));
            }
        }
    }
    #[test]
    fn overview_includes_mission_members_asks_and_drawer_shells() {
        let temp = tempfile::tempdir().unwrap();
        let core = crate::test_support::test_core_in(temp.path().into());
        let conn = core.db.get().unwrap();
        let project = repo::project::create(&conn, "Mission project", "/tmp/m5").unwrap();
        let now = chrono::Utc::now();
        crate::test_support::insert_test_role(
            &conn,
            "agent-role",
            "coder-template",
            "codex",
            "codex",
        );
        crate::test_support::insert_test_role(&conn, "shell-role", "shell", "shell", "zsh");
        repo::crew::insert(
            &conn,
            &repo::crew::CrewRow {
                id: "crew".into(),
                name: "Test crew".into(),
                system_prompt_addendum: None,
                created_at: now,
                updated_at: now,
            },
        )
        .unwrap();
        crate::test_support::insert_test_slot(
            &conn,
            "slot",
            "crew",
            "agent-role",
            "coder",
            0,
            true,
        );
        repo::mission::insert(
            &conn,
            &repo::mission::MissionRow {
                id: "mission".into(),
                crew_id: Some("crew".into()),
                project_id: Some(project.id.clone()),
                title: "Test mission".into(),
                status: crate::model::MissionStatus::Running,
                goal_override: None,
                cwd: Some("/tmp/m5".into()),
                started_at: now,
                stopped_at: None,
                pinned_at: None,
                archived_at: None,
                linked_session_id: None,
                report_to_chat: true,
            },
        )
        .unwrap();
        let mut agent = repo::session::SessionRowDb::new_running("agent".into());
        agent.mission_id = Some("mission".into());
        agent.slot_id = Some("slot".into());
        agent.role_id = Some("agent-role".into());
        agent.agent_runtime = Some("codex".into());
        agent.live_title = Some("Injected role prompt topic".into());
        repo::session::insert(&conn, &agent).unwrap();
        let mut shell = repo::session::SessionRowDb::new_running("shell".into());
        shell.role_id = Some("shell-role".into());
        shell.agent_runtime = Some("shell".into());
        shell.agent_command = Some("/bin/zsh".into());
        repo::session::insert(&conn, &shell).unwrap();
        let mut node =
            repo::node::ensure_mission_node(&conn, "mission", Some(&project.id)).unwrap();
        node.position = 4;
        repo::node::upsert(&conn, &node).unwrap();
        repo::node::set_pinned(&conn, &node.id, true).unwrap();
        repo::mission::set_pinned_at(&conn, "mission", Some(now)).unwrap();
        repo::node::set_layout(&conn, &node.id, r#"{"drawer":{"shells":["shell"]}}"#).unwrap();
        let mission_dir = runner_core::event_log::mission_dir(temp.path(), Some("crew"), "mission");
        let roster = crate::ops::mission::roster::snapshot(
            repo::crew::get(&conn, "crew").unwrap().as_ref(),
            &crate::ops::slot::list(&conn, "crew").unwrap(),
        );
        crate::ops::mission::roster::write(&mission_dir, &roster).unwrap();
        let log = runner_core::event_log::EventLog::open(&mission_dir).unwrap();
        let ask = log.append(runner_core::model::EventDraft::signal(Some("crew"), "mission", "human", "human_question", serde_json::json!({"prompt":"Proceed?","choices":["yes","no"],"on_behalf_of":"coder"}))).unwrap();
        drop(conn);
        let overview = overview(&core).unwrap();
        assert_eq!(overview.missions.len(), 1);
        let mission = &overview.missions[0];
        assert_eq!(mission.crew, "Test crew");
        assert_eq!(mission.state, MissionState::Running);
        assert_eq!(
            mission.members,
            vec![Member {
                handle: "coder".into(),
                session: "agent".into(),
                lead: false
            }]
        );
        assert_eq!(
            mission.asks,
            vec![Ask {
                question: ask.id,
                from: "coder".into(),
                prompt: "Proceed?".into(),
                choices: Some(vec!["yes".into(), "no".into()])
            }]
        );
        assert_eq!(mission.shells, ["shell"]);
        assert_eq!(overview.sessions["agent"].kind, SessionKind::Agent);
        assert_eq!(overview.sessions["agent"].title, "@coder");
        assert_eq!(overview.sessions["shell"].kind, SessionKind::Shell);
        assert_eq!(overview.sessions["shell"].title, "zsh");
        assert_eq!(overview.sessions["shell"].runtime, None);
        let stored_node = repo::node::get(&core.db.get().unwrap(), &node.id)
            .unwrap()
            .unwrap();
        assert!(overview.sidebar.as_ref().unwrap().contains(&SidebarNode {
            kind: SidebarNodeKind::Mission,
            id: "mission".into(),
            parent: Some(project.id.clone()),
            position: stored_node.position,
            pinned_position: stored_node.pinned_position,
        }));
        assert_eq!(stored_node.position, 4);
        assert_eq!(stored_node.pinned_position, Some(0));
        assert!(overview.tabs.is_empty());
    }

    #[test]
    fn input_and_mark_viewed_clear_running_codex_unread_and_subsequent_completion_sets_it_again() {
        use runner_core::companion::ActionRequest;
        use std::sync::Arc;
        use std::time::Duration;

        let temp = tempfile::tempdir().unwrap();
        let core = Arc::new(crate::test_support::test_core_in(temp.path().into()));
        let session_ids = ["s1".to_owned()];
        {
            let conn = core.db.get().unwrap();
            conn.execute(
                "INSERT INTO sessions(id, status, agent_runtime, agent_command) VALUES ('s1', 'running', 'codex', 'codex')",
                [],
            )
            .unwrap();
            let layout = serde_json::json!({"preset": "single", "slots": ["s1"], "sizes": {}});
            repo::node::create_tab(&conn, None, "chat", 0, &layout.to_string()).unwrap();
        }
        let events_iface: Arc<dyn crate::session::manager::SessionEvents> =
            Arc::new(core.session_events());
        core.sessions
            .prepare_unlisted_terminal("s1", (80, 24), &core.db, &events_iface)
            .unwrap();
        core.sessions
            .browser_resize("s1", 1, "Runner", 80, 24, 0, 0, true, false, &core.db)
            .unwrap();
        core.sessions.test_attach_live("s1", "");
        let input_rx = core.sessions.record_terminal_input("s1");

        let events = core.session_events();
        core.sessions.publish_direct_activity(
            "s1",
            crate::session::manager::SessionActivityState::Busy,
            crate::session::state::StatusSource::Test,
            &events,
        );
        core.sessions.arm_completion("s1");
        core.sessions.publish_direct_activity(
            "s1",
            crate::session::manager::SessionActivityState::Idle,
            crate::session::state::StatusSource::Test,
            &events,
        );
        assert!(core.sessions.agent_status("s1").unread_since.is_some());
        assert!(
            repo::session_attention::any_unread(&core.db.get().unwrap(), &session_ids).unwrap()
        );

        let backend = DaemonCore(Arc::downgrade(&core));
        assert!(backend.statuses().unwrap()["s1"].unread);
        assert!(overview(&core).unwrap().sessions["s1"].status.unread);

        backend.input("s1", 1, b"follow up prompt\r").unwrap();
        assert_eq!(
            input_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            b"follow up prompt\r"
        );
        assert!(core.sessions.agent_status("s1").unread_since.is_none());
        assert!(
            !repo::session_attention::any_unread(&core.db.get().unwrap(), &session_ids).unwrap()
        );
        assert!(!backend.statuses().unwrap()["s1"].unread);
        assert!(!overview(&core).unwrap().sessions["s1"].status.unread);

        core.sessions.publish_direct_activity(
            "s1",
            crate::session::manager::SessionActivityState::Busy,
            crate::session::state::StatusSource::Test,
            &events,
        );
        core.sessions.arm_completion("s1");
        core.sessions.publish_direct_activity(
            "s1",
            crate::session::manager::SessionActivityState::Idle,
            crate::session::state::StatusSource::Test,
            &events,
        );
        assert!(core.sessions.agent_status("s1").unread_since.is_some());
        assert!(
            repo::session_attention::any_unread(&core.db.get().unwrap(), &session_ids).unwrap()
        );
        assert!(backend.statuses().unwrap()["s1"].unread);

        backend
            .action(ActionRequest::MarkViewed {
                id: 1,
                session: "s1".into(),
            })
            .unwrap();
        assert!(core.sessions.agent_status("s1").unread_since.is_none());
        assert!(
            !repo::session_attention::any_unread(&core.db.get().unwrap(), &session_ids).unwrap()
        );
        assert!(!backend.statuses().unwrap()["s1"].unread);
        assert!(!overview(&core).unwrap().sessions["s1"].status.unread);

        core.sessions.drain_terminal_workers();
    }

    #[test]
    fn overview_includes_session_and_mission_checkouts_and_tracks_git_switch() {
        use std::process::Command;

        let git = |dir: &std::path::Path, args: &[&str]| {
            let output = Command::new("git")
                .args(args)
                .current_dir(dir)
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.com")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.com")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&output.stderr)
            );
        };

        let temp = tempfile::Builder::new()
            .prefix("1213-m2-comp-core-")
            .tempdir()
            .unwrap();
        let origin = temp.path().join("origin.git");
        let main_repo = temp.path().join("repo");
        std::fs::create_dir_all(&origin).unwrap();
        git(&origin, &["init", "--bare", "-q"]);
        std::fs::create_dir_all(&main_repo).unwrap();
        git(&main_repo, &["init", "-q", "-b", "main"]);
        std::fs::write(main_repo.join("README.md"), "base\n").unwrap();
        git(&main_repo, &["add", "-A"]);
        git(&main_repo, &["commit", "-q", "-m", "init"]);
        git(
            &main_repo,
            &["remote", "add", "origin", &origin.to_string_lossy()],
        );
        git(&main_repo, &["push", "-q", "-u", "origin", "main"]);
        git(&main_repo, &["remote", "set-head", "origin", "main"]);

        let core = std::sync::Arc::new(crate::test_support::test_core_in(temp.path().into()));
        let wt = main_repo.join(".worktrees/feat-alpha");
        git(
            &main_repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feat/alpha",
                &wt.to_string_lossy(),
            ],
        );
        git(&wt, &["branch", "feat/beta"]);

        let conn = core.db.get().unwrap();
        let now = chrono::Utc::now();
        let project = repo::project::create(&conn, "Repo", &main_repo.to_string_lossy()).unwrap();
        crate::test_support::insert_test_role(&conn, "role", "coder", "codex", "codex");
        repo::crew::insert(
            &conn,
            &repo::crew::CrewRow {
                id: "crew".into(),
                name: "Duo".into(),
                system_prompt_addendum: None,
                created_at: now,
                updated_at: now,
            },
        )
        .unwrap();
        crate::test_support::insert_test_slot(&conn, "sl", "crew", "role", "coder", 0, true);
        let wt_str = runner_core::app_paths::normalize_checkout_path(&wt)
            .to_string_lossy()
            .into_owned();
        repo::mission::insert(
            &conn,
            &repo::mission::MissionRow {
                id: "m-wt".into(),
                crew_id: Some("crew".into()),
                project_id: Some(project.id.clone()),
                title: "Worktree mission".into(),
                status: crate::model::MissionStatus::Completed,
                goal_override: None,
                cwd: Some(wt_str.clone()),
                started_at: now,
                stopped_at: Some(now),
                pinned_at: None,
                archived_at: None,
                linked_session_id: None,
                report_to_chat: true,
            },
        )
        .unwrap();
        let mut s = repo::session::SessionRowDb::new_running("s-wt".into());
        s.status = crate::model::SessionStatus::Stopped;
        s.role_id = Some("role".into());
        s.agent_runtime = Some("codex".into());
        s.project_id = Some(project.id.clone());
        s.cwd = Some(wt_str.clone());
        s.title = Some("Chat".into());
        repo::session::insert(&conn, &s).unwrap();
        let project_node = repo::node::ensure_project_node(&conn, &project.id).unwrap();
        let layout = serde_json::json!({"preset": "single", "slots": ["s-wt"], "sizes": {}});
        repo::node::create_tab(
            &conn,
            Some(&project_node.id),
            "chat",
            0,
            &layout.to_string(),
        )
        .unwrap();
        repo::node::ensure_mission_node(&conn, "m-wt", Some(&project.id)).unwrap();
        drop(conn);

        let ov = overview(&core).unwrap();
        let mission = ov.missions.iter().find(|m| m.id == "m-wt").unwrap();
        assert_eq!(mission.cwd.as_deref(), Some(wt_str.as_str()));
        match mission.checkout.as_ref().unwrap() {
            Checkout::Git {
                kind,
                managed,
                branch,
                missing,
                auto_base,
                ..
            } => {
                assert_eq!(*kind, CheckoutKind::Linked);
                assert!(*managed);
                assert_eq!(branch.as_deref(), Some("feat/alpha"));
                assert!(!*missing);
                assert_eq!(auto_base.as_deref(), Some("origin/main"));
            }
            other => panic!("expected git checkout, got {other:?}"),
        }
        assert_eq!(ov.sessions["s-wt"].checkout, mission.checkout);

        git(&wt, &["switch", "-q", "feat/beta"]);
        let ov2 = overview(&core).unwrap();
        let mission2 = ov2.missions.iter().find(|m| m.id == "m-wt").unwrap();
        assert!(matches!(
            mission2.checkout.as_ref(),
            Some(Checkout::Git { branch: Some(b), .. }) if b == "feat/beta"
        ));
    }

    #[test]
    fn overview_sessions_carry_launch_definition_model_and_effort() {
        let temp = tempfile::tempdir().unwrap();
        let core = crate::test_support::test_core_in(temp.path().into());
        let conn = core.db.get().unwrap();
        let now = chrono::Utc::now();
        crate::test_support::insert_test_role(&conn, "r-codex", "reviewer", "codex", "codex");
        crate::test_support::insert_test_role(&conn, "r-pi", "qa", "pi", "pi");
        crate::test_support::insert_test_role(&conn, "r-shell", "shell", "shell", "/bin/sh");
        repo::crew::insert(
            &conn,
            &repo::crew::CrewRow {
                id: "crew-rt".into(),
                name: "Mixed".into(),
                system_prompt_addendum: None,
                created_at: now,
                updated_at: now,
            },
        )
        .unwrap();
        crate::test_support::insert_test_slot(
            &conn, "sl-rev", "crew-rt", "r-codex", "reviewer", 0, false,
        );
        crate::test_support::insert_test_slot(&conn, "sl-qa", "crew-rt", "r-pi", "qa", 1, false);
        crate::test_support::insert_test_slot(
            &conn,
            "sl-def-model",
            "crew-rt",
            "r-codex",
            "def-model",
            2,
            false,
        );
        crate::test_support::insert_test_slot(
            &conn,
            "sl-def-effort",
            "crew-rt",
            "r-codex",
            "def-effort",
            3,
            false,
        );
        crate::test_support::insert_test_slot(
            &conn, "sl-sh", "crew-rt", "r-shell", "shell", 4, false,
        );
        repo::mission::insert(
            &conn,
            &repo::mission::MissionRow {
                id: "m-rt".into(),
                crew_id: Some("crew-rt".into()),
                project_id: None,
                title: "Mixed runtimes".into(),
                status: crate::model::MissionStatus::Running,
                goal_override: None,
                cwd: Some("/tmp".into()),
                started_at: now,
                stopped_at: None,
                pinned_at: None,
                archived_at: None,
                linked_session_id: None,
                report_to_chat: true,
            },
        )
        .unwrap();
        for (id, role_id, slot_id, runtime, model, effort) in [
            (
                "s-rev",
                "r-codex",
                "sl-rev",
                "codex",
                Some("gpt-6-astra"),
                Some("xhigh"),
            ),
            ("s-qa", "r-pi", "sl-qa", "pi", Some("deepseek-chat"), None),
            (
                "s-def-model",
                "r-codex",
                "sl-def-model",
                "codex",
                Some(" default "),
                Some("high"),
            ),
            (
                "s-def-effort",
                "r-codex",
                "sl-def-effort",
                "codex",
                Some("gpt-6-astra"),
                Some("DEFAULT"),
            ),
            (
                "s-sh",
                "r-shell",
                "sl-sh",
                "shell",
                Some("ignored"),
                Some("high"),
            ),
        ] {
            let mut s = repo::session::SessionRowDb::new_running(id.into());
            s.mission_id = Some("m-rt".into());
            s.role_id = Some(role_id.into());
            s.slot_id = Some(slot_id.into());
            s.agent_runtime = Some(runtime.into());
            let mut def = crate::test_support::session_definition(runtime, runtime);
            def.model = model.map(str::to_owned);
            def.effort = effort.map(str::to_owned);
            s.launch_definition = Some(def);
            repo::session::insert(&conn, &s).unwrap();
        }
        repo::node::ensure_mission_node(&conn, "m-rt", None).unwrap();
        drop(conn);

        let ov = overview(&core).unwrap();
        assert_eq!(ov.sessions["s-rev"].runtime.as_deref(), Some("codex"));
        assert_eq!(ov.sessions["s-rev"].model.as_deref(), Some("gpt-6-astra"));
        assert_eq!(ov.sessions["s-rev"].effort.as_deref(), Some("xhigh"));
        assert_eq!(ov.sessions["s-qa"].runtime.as_deref(), Some("pi"));
        assert_eq!(ov.sessions["s-qa"].model.as_deref(), Some("deepseek-chat"));
        assert_eq!(ov.sessions["s-qa"].effort, None);
        assert_eq!(ov.sessions["s-def-model"].model, None);
        assert_eq!(ov.sessions["s-def-model"].effort.as_deref(), Some("high"));
        assert_eq!(
            ov.sessions["s-def-effort"].model.as_deref(),
            Some("gpt-6-astra")
        );
        assert_eq!(ov.sessions["s-def-effort"].effort, None);
        assert_eq!(ov.sessions["s-sh"].kind, SessionKind::Shell);
        assert_eq!(ov.sessions["s-sh"].runtime, None);
        assert_eq!(ov.sessions["s-sh"].model, None);
        assert_eq!(ov.sessions["s-sh"].effort, None);
    }
}
