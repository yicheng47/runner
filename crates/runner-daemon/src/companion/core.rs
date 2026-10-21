use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
};

use runner_core::{companion::*, protocol::terminal::TerminalAttachment};
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::{events::AppEvent, repo, AppCore};

pub type Result<T> = std::result::Result<T, Failure>;
#[derive(Debug)]
pub struct Failure(pub ErrorCode, pub &'static str);

pub struct Attachment {
    pub terminal: TerminalAttachment,
    pub resync: Arc<dyn Fn() -> Result<TerminalAttachment> + Send + Sync>,
}

pub trait Core: Send + Sync {
    fn overview(&self) -> Result<Overview>;
    fn subscribe(&self) -> Result<broadcast::Receiver<AppEvent>>;
    fn attach(self: Arc<Self>, session: &str, history: usize) -> Result<Attachment>;
    fn resize(
        &self,
        session: &str,
        channel: u64,
        name: &str,
        size: (u16, u16),
        control: Control,
    ) -> Result<()>;
    fn input(&self, session: &str, bytes: &[u8]) -> Result<()>;
    fn detach(&self, session: &str, subscriber: u64, channel: u64, release: bool);
    fn closed_reason(&self, session: &str) -> ClosedReason;
    fn release_channel(&self, channel: u64);
}
#[derive(Clone, Copy)]
pub enum Control {
    Take,
    IfFree,
    Viewport,
}

pub struct DaemonCore(pub Weak<AppCore>);
impl DaemonCore {
    fn core(&self) -> Result<std::sync::Arc<AppCore>> {
        self.0
            .upgrade()
            .ok_or(Failure(ErrorCode::Failed, "The host has stopped."))
    }
    fn terminal(&self, session: &str) -> Result<std::sync::Arc<AppCore>> {
        let core = self.core()?;
        let conn = core.db.get().map_err(failed)?;
        let row = repo::session::get_row(&conn, session).map_err(failed)?;
        if row.is_none() {
            return Err(Failure(ErrorCode::NotFound, "No session has that id."));
        }
        if !core.sessions.session_live(session) {
            return Err(Failure(
                ErrorCode::NotRunning,
                "The session has no live terminal.",
            ));
        }
        drop(conn);
        Ok(core)
    }
}
fn failed(_: impl std::fmt::Display) -> Failure {
    Failure(
        ErrorCode::Failed,
        "The host could not complete the request.",
    )
}
impl Core for DaemonCore {
    fn overview(&self) -> Result<Overview> {
        overview(self.core()?.as_ref()).map_err(failed)
    }
    fn subscribe(&self) -> Result<broadcast::Receiver<AppEvent>> {
        Ok(self.core()?.events.subscribe())
    }
    fn attach(self: Arc<Self>, session: &str, history: usize) -> Result<Attachment> {
        let (terminal, source) = self
            .terminal(session)?
            .sessions
            .attach_companion_terminal(session, history)
            .map_err(failed)?;
        Ok(Attachment {
            terminal,
            resync: Arc::new(move || source.attach().map_err(failed)),
        })
    }
    fn resize(
        &self,
        session: &str,
        channel: u64,
        name: &str,
        size: (u16, u16),
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
    fn input(&self, session: &str, bytes: &[u8]) -> Result<()> {
        let core = self.terminal(session)?;
        core.sessions
            .terminal_model(session)
            .map_err(failed)?
            .observe_input(&super::classify_input(bytes));
        core.sessions
            .queue_terminal_input(session, bytes)
            .map_err(failed)
    }
    fn detach(&self, session: &str, subscriber: u64, channel: u64, release: bool) {
        if let Ok(core) = self.core() {
            core.sessions.detach_terminal(session, subscriber);
            if release {
                core.sessions.release_browser(session, channel);
            }
        }
    }
    fn release_channel(&self, channel: u64) {
        if let Ok(core) = self.core() {
            core.sessions.release_browser_channel(channel);
        }
    }
    fn closed_reason(&self, session: &str) -> ClosedReason {
        let Ok(core) = self.core() else {
            return ClosedReason::Unknown;
        };
        let Ok(conn) = core.db.get() else {
            return ClosedReason::Unknown;
        };
        let Ok(Some(row)) = repo::session::get_row(&conn, session) else {
            return ClosedReason::Unknown;
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

fn overview(core: &AppCore) -> crate::error::Result<Overview> {
    let nodes = crate::ops::node::node_list(core)?;
    let statuses = crate::ops::session::session_status_snapshot(core)?;
    let mut overview = Overview {
        projects: vec![],
        tabs: vec![],
        missions: vec![],
        sessions: BTreeMap::new(),
    };
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
                            lead: row.lead,
                        }
                    })
                    .collect();
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
                });
            }
        }
    }
    let conn = core.db.get()?;
    for id in ids {
        if overview.sessions.contains_key(&id) {
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
        let status = statuses
            .get(&id)
            .map(SessionStatus::from)
            .unwrap_or(SessionStatus {
                status: Status::Idle,
                reason: None,
                unread: false,
            });
        overview.sessions.insert(
            id,
            Session {
                kind,
                title: row
                    .title
                    .or_else(|| {
                        row.live_title.as_deref().and_then(|title| {
                            runner_core::protocol::session_title::provider_title(
                                title,
                                row.cwd.as_deref(),
                            )
                        })
                    })
                    .unwrap_or_default(),
                runtime: runtime.filter(|_| !shell),
                cwd: row.cwd,
                status,
            },
        );
    }
    Ok(overview)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

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
        assert_eq!(Arc::strong_count(&core), 1);
        assert!(matches!(
            Arc::new(DaemonCore(backend.0.clone())).attach("missing", 0),
            Err(Failure(ErrorCode::NotFound, _))
        ));
        assert!(matches!(
            Arc::new(DaemonCore(backend.0.clone())).attach("a", 0),
            Err(Failure(ErrorCode::NotRunning, _))
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
        let now = chrono::Utc::now();
        crate::test_support::insert_test_role(&conn, "agent-role", "coder", "codex", "codex");
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
                project_id: None,
                title: "Test mission".into(),
                status: crate::model::MissionStatus::Running,
                goal_override: None,
                cwd: Some("/tmp/m5".into()),
                started_at: now,
                stopped_at: None,
                pinned_at: None,
                archived_at: None,
            },
        )
        .unwrap();
        let mut agent = repo::session::SessionRowDb::new_running("agent".into());
        agent.mission_id = Some("mission".into());
        agent.slot_id = Some("slot".into());
        agent.role_id = Some("agent-role".into());
        agent.agent_runtime = Some("codex".into());
        repo::session::insert(&conn, &agent).unwrap();
        let mut shell = repo::session::SessionRowDb::new_running("shell".into());
        shell.role_id = Some("shell-role".into());
        shell.agent_runtime = Some("shell".into());
        repo::session::insert(&conn, &shell).unwrap();
        let node = repo::node::ensure_mission_node(&conn, "mission", None).unwrap();
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
                lead: true
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
        assert_eq!(overview.sessions["shell"].kind, SessionKind::Shell);
        assert_eq!(overview.sessions["shell"].runtime, None);
        assert!(overview.tabs.is_empty());
    }
}
