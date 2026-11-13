// A mission's linked chat (#748): the direct agent chat it reports to and
// is grouped with. `mission_start` links the chat that started it; these
// requests set, move and remove the link by hand.

use rusqlite::Connection;

use crate::error::{Error, Result};
use crate::model::{Mission, Runtime};
use crate::ops::mission::MissionLinkInput;
use crate::repo;
use crate::repo::session::DirectSessionRow;
use crate::AppCore;

/// The direct agent chat `session_id` names, or why it is not one.
pub(crate) fn direct_agent_chat(conn: &Connection, session_id: &str) -> Result<DirectSessionRow> {
    let Some(chat) = repo::session::get_direct(conn, session_id)? else {
        return Err(Error::msg(if repo::session::exists(conn, session_id)? {
            format!(
                "session {session_id} is a mission member; only a direct agent chat can be linked"
            )
        } else {
            format!("no chat {session_id}")
        }));
    };
    if chat
        .row
        .agent_runtime
        .as_deref()
        .and_then(Runtime::parse)
        .is_some_and(Runtime::is_shell)
    {
        return Err(Error::msg(format!(
            "session {session_id} is a terminal; only a direct agent chat can be linked"
        )));
    }
    Ok(chat)
}

/// Whether `chat` may take a link from a mission in `project_id`.
pub(crate) fn ensure_linkable(chat: &DirectSessionRow, project_id: Option<&str>) -> Result<()> {
    if chat.row.archived_at.is_some() {
        return Err(Error::msg(format!(
            "chat {} is archived; unarchive it before linking a mission",
            chat.row.id
        )));
    }
    if chat.row.project_id.as_deref() != project_id {
        return Err(Error::msg(format!(
            "chat {} is in another project; a mission links only to a chat in its own project",
            chat.row.id
        )));
    }
    Ok(())
}

pub fn mission_link(core: &AppCore, input: MissionLinkInput) -> Result<Mission> {
    let mut conn = core.db.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let mission = super::get(&tx, &input.mission_id)?;
    if mission.archived_at.is_some() {
        return Err(Error::msg(format!(
            "mission {} is archived; unarchive it before linking it",
            mission.id
        )));
    }
    let chat = direct_agent_chat(&tx, &input.session_id)?;
    ensure_linkable(&chat, mission.project_id.as_deref())?;
    let previous = mission.linked_chat.map(|chat| chat.session_id);
    repo::mission::set_link(&tx, &mission.id, Some(&chat.row.id), !input.no_report)?;
    // A linked mission takes its group's pin: it has none of its own.
    let unpinned = match repo::node::find_by_ref(&tx, repo::node::NodeType::Mission, &mission.id)? {
        Some(node) if node.pinned_position.is_some() => {
            repo::node::set_pinned(&tx, &node.id, false)?;
            repo::mission::set_pinned_at(&tx, &mission.id, None)?;
            true
        }
        _ => false,
    };
    let mission = super::get(&tx, &mission.id)?;
    tx.commit()?;
    drop(conn);
    if unpinned {
        core.events
            .emit("chat/layout-changed", &serde_json::json!({}));
    }
    // The chat hears about a new link unless its own agent made it.
    let reported = !input.no_report
        && previous.as_deref() != Some(chat.row.id.as_str())
        && input.caller_session_id.as_deref() != Some(chat.row.id.as_str());
    if reported {
        core.reports.linked(&chat.row.id, &mission.id);
    } else if previous.as_deref() != Some(chat.row.id.as_str()) || input.no_report {
        core.reports.unlinked(&mission.id);
    }
    Ok(mission)
}

pub fn mission_unlink(core: &AppCore, mission_id: &str) -> Result<Mission> {
    let mut conn = core.db.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let mission = super::get(&tx, mission_id)?;
    let tab = match &mission.linked_chat {
        Some(chat) => chat_tab(&tx, &chat.session_id)?,
        None => None,
    };
    repo::mission::set_link(&tx, &mission.id, None, true)?;
    let placed = mission.linked_chat.is_some() && place_unlinked(&tx, &mission.id, tab.as_ref())?;
    let mission = super::get(&tx, &mission.id)?;
    tx.commit()?;
    drop(conn);
    core.reports.unlinked(&mission.id);
    if placed {
        core.events
            .emit("chat/layout-changed", &serde_json::json!({}));
    }
    Ok(mission)
}

/// The tab holding a chat: the head of its missions' sidebar group.
pub(crate) fn chat_tab(conn: &Connection, chat: &str) -> Result<Option<repo::node::NodeRow>> {
    Ok(repo::node::list(conn)?.into_iter().find(|node| {
        node.node_type == repo::node::NodeType::Tab
            && repo::node::session_ids(node).iter().any(|id| id == chat)
    }))
}

/// The tab heading a mission's sidebar group: the one holding its linked,
/// unarchived chat. A group's pin belongs to this tab, never to its missions.
pub(crate) fn group_head(
    conn: &Connection,
    mission_id: &str,
) -> Result<Option<repo::node::NodeRow>> {
    let mission = super::get(conn, mission_id)?;
    match mission.linked_chat.filter(|chat| !chat.archived) {
        Some(chat) => chat_tab(conn, &chat.session_id),
        None => Ok(None),
    }
}

/// Where an unlinked mission's row goes. From an unpinned group it lands
/// right below the group; from a pinned group, whose pin was the group's, it
/// goes back to its project unpinned, where a newly started mission appears.
/// Returns whether its node moved.
fn place_unlinked(
    tx: &rusqlite::Transaction<'_>,
    mission_id: &str,
    tab: Option<&repo::node::NodeRow>,
) -> Result<bool> {
    let Some(mission) = repo::node::find_by_ref(tx, repo::node::NodeType::Mission, mission_id)?
    else {
        return Ok(false);
    };
    let Some(tab) = tab.filter(|tab| tab.parent_id == mission.parent_id) else {
        return Ok(false);
    };
    repo::node::set_pinned(tx, &mission.id, false)?;
    let mut siblings: Vec<_> = repo::node::list(tx)?
        .into_iter()
        .filter(|node| {
            node.parent_id == mission.parent_id
                && node.pinned_position.is_none()
                && node.id != mission.id
        })
        .collect();
    siblings.sort_by_key(|node| node.position);
    let mut order: Vec<String> = siblings.into_iter().map(|node| node.id).collect();
    let at = if tab.pinned_position.is_some() {
        // Projects lead the root's order, as everywhere else in the sidebar.
        let nodes = repo::node::list(tx)?;
        order
            .iter()
            .position(|id| {
                nodes
                    .iter()
                    .find(|node| &node.id == id)
                    .is_some_and(|node| node.node_type != repo::node::NodeType::Project)
            })
            .unwrap_or(order.len())
    } else {
        order
            .iter()
            .position(|id| *id == tab.id)
            .map_or(order.len(), |i| i + 1)
    };
    order.insert(at, mission.id.clone());
    repo::node::move_and_reorder(tx, &mission.id, mission.parent_id.as_deref(), &order)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SessionStatus;
    use crate::ops::mission::{feed, start, MissionStart};
    use crate::ops::project::ProjectScope;
    use crate::test_support::{insert_session_row, test_core_in, test_session_row};

    struct Fixture {
        _dir: tempfile::TempDir,
        core: AppCore,
        project: String,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let core = test_core_in(dir.path().to_path_buf());
        let project = {
            let conn = core.db.get().unwrap();
            let project = repo::project::create(&conn, "Project", "/tmp/link-project").unwrap();
            repo::node::ensure_project_node(&conn, &project.id).unwrap();
            project.id
        };
        Fixture {
            _dir: dir,
            core,
            project,
        }
    }

    impl Fixture {
        fn session(&self, id: &str, runtime: &str, project: Option<&str>) {
            let mut row = test_session_row(id, SessionStatus::Running);
            row.agent_runtime = Some(runtime.into());
            row.project_id = project.map(str::to_owned);
            row.title = Some(format!("{id} title"));
            insert_session_row(&self.core.db.get().unwrap(), &row).unwrap();
        }

        fn start(&self, starter: Option<&str>, project: Option<&str>, no_report: bool) -> Mission {
            start(
                &mut self.core.db.get().unwrap(),
                &self.core.app_data_dir,
                MissionStart {
                    crew_id: None,
                    scope: project
                        .map_or(ProjectScope::Root, |id| ProjectScope::Project(id.into())),
                    title: "Linked".into(),
                    goal_override: None,
                    cwd: Some("/tmp".into()),
                    started_by_session_id: starter.map(str::to_owned),
                    no_report,
                },
            )
            .unwrap()
            .mission
        }

        fn slot(&self, id: &str) {
            let mission = self.start(None, None, false);
            let mut row = test_session_row(id, SessionStatus::Running);
            row.agent_runtime = Some("codex".into());
            row.mission_id = Some(mission.id);
            row.slot_id = Some("slot".into());
            insert_session_row(&self.core.db.get().unwrap(), &row).unwrap();
        }

        fn link(&self, mission: &str, chat: &str) -> Result<Mission> {
            mission_link(
                &self.core,
                MissionLinkInput {
                    mission_id: mission.into(),
                    session_id: chat.into(),
                    no_report: false,
                    caller_session_id: None,
                },
            )
        }

        fn stop_chat(&self, id: &str) {
            self.core
                .db
                .get()
                .unwrap()
                .execute("UPDATE sessions SET status = 'stopped' WHERE id = ?1", [id])
                .unwrap();
        }

        fn stop_mission(&self, id: &str) {
            super::super::stop(
                &mut self.core.db.get().unwrap(),
                &self.core.app_data_dir,
                id,
            )
            .unwrap();
        }

        fn get(&self, mission: &str) -> Mission {
            super::super::get(&self.core.db.get().unwrap(), mission).unwrap()
        }
    }

    fn linked_to(mission: &Mission) -> Option<&str> {
        mission
            .linked_chat
            .as_ref()
            .map(|chat| chat.session_id.as_str())
    }

    #[test]
    fn starting_from_a_direct_agent_chat_links_it_and_records_the_starter() {
        let f = fixture();
        f.session("chat", "codex", None);

        let mission = f.start(Some("chat"), None, false);

        assert_eq!(
            mission.linked_chat,
            Some(crate::model::LinkedChat {
                session_id: "chat".into(),
                title: "chat title".into(),
                archived: false,
                reports: true,
            })
        );
        let status = feed::mission_status(&f.core, &mission.id).unwrap();
        assert_eq!(
            status.started_by,
            Some(runner_core::protocol::MissionStarter {
                session_id: "chat".into(),
                title: "chat title".into(),
            })
        );
        assert_eq!(linked_to(&status.mission), Some("chat"));
    }

    #[test]
    fn a_terminal_a_slot_or_no_session_starts_an_unlinked_mission_without_a_starter() {
        let f = fixture();
        f.session("terminal", "shell", None);
        f.slot("slot-session");

        for starter in [
            Some("terminal"),
            Some("slot-session"),
            Some("missing"),
            None,
        ] {
            let mission = f.start(starter, None, false);
            assert_eq!(mission.linked_chat, None, "{starter:?}");
            let status = feed::mission_status(&f.core, &mission.id).unwrap();
            assert_eq!(status.started_by, None, "{starter:?}");
        }
    }

    #[test]
    fn no_report_links_without_reports() {
        let f = fixture();
        f.session("chat", "claude-code", None);

        let mission = f.start(Some("chat"), None, true);

        let chat = mission.linked_chat.unwrap();
        assert_eq!(chat.session_id, "chat");
        assert!(!chat.reports);
    }

    #[test]
    fn a_chat_in_another_project_starts_the_mission_unlinked_but_is_its_starter() {
        let f = fixture();
        f.session("chat", "codex", Some(&f.project));

        let mission = f.start(Some("chat"), None, false);

        assert_eq!(mission.linked_chat, None);
        let status = feed::mission_status(&f.core, &mission.id).unwrap();
        assert_eq!(status.started_by.unwrap().session_id, "chat");
        let mission = f.start(Some("chat"), Some(&f.project), false);
        assert_eq!(linked_to(&mission), Some("chat"));
    }

    #[test]
    fn linking_moves_a_link_and_unlinking_clears_it() {
        let f = fixture();
        f.session("first", "codex", None);
        f.session("second", "claude-code", None);
        let mission = f.start(Some("first"), None, true);

        let moved = f.link(&mission.id, "second").unwrap();

        let chat = moved.linked_chat.unwrap();
        assert_eq!(chat.session_id, "second");
        assert!(chat.reports, "a new link reports unless asked not to");
        let status = feed::mission_status(&f.core, &mission.id).unwrap();
        assert_eq!(
            status.started_by.unwrap().session_id,
            "first",
            "the starter never changes"
        );
        assert_eq!(
            mission_unlink(&f.core, &mission.id).unwrap().linked_chat,
            None
        );
        assert_eq!(f.get(&mission.id).linked_chat, None);
    }

    #[test]
    fn linking_refuses_archived_chats_terminals_slots_other_projects_and_archived_missions() {
        let f = fixture();
        f.session("chat", "codex", None);
        f.session("archived", "codex", None);
        f.session("terminal", "shell", None);
        f.session("elsewhere", "codex", Some(&f.project));
        f.slot("slot-session");
        f.stop_chat("archived");
        repo::session::archive(&f.core.db.get().unwrap(), "archived", chrono::Utc::now()).unwrap();
        let mission = f.start(Some("chat"), None, false);

        for (chat, refusal) in [
            ("archived", "is archived"),
            ("terminal", "is a terminal"),
            ("slot-session", "is a mission member"),
            ("elsewhere", "in another project"),
            ("missing", "no chat missing"),
        ] {
            let error = f.link(&mission.id, chat).unwrap_err().to_string();
            assert!(error.contains(refusal), "{chat}: {error}");
            assert_eq!(linked_to(&f.get(&mission.id)), Some("chat"), "{chat}");
        }

        f.stop_mission(&mission.id);
        let error = f.link(&mission.id, "chat").unwrap_err().to_string();
        assert!(
            error.contains("mission") && error.contains("is archived"),
            "{error}"
        );
    }

    #[test]
    fn archiving_the_chat_keeps_the_link_and_deleting_it_clears_the_link() {
        let f = fixture();
        f.session("chat", "codex", None);
        let mission = f.start(Some("chat"), None, false);

        f.stop_chat("chat");
        crate::ops::session::session_archive_direct(&f.core, "chat").unwrap();
        let chat = f.get(&mission.id).linked_chat.unwrap();
        assert!(chat.archived);

        crate::ops::session::session_delete(&f.core, "chat").unwrap();
        assert_eq!(f.get(&mission.id).linked_chat, None);
    }

    fn published(
        events: &mut tokio::sync::broadcast::Receiver<crate::events::AppEvent>,
    ) -> Vec<&'static str> {
        std::iter::from_fn(|| events.try_recv().ok())
            .map(|event| event.name)
            .collect()
    }

    fn tab(f: &Fixture, id: &str, project: Option<&str>) {
        let parent_id = project.map(|project| {
            repo::node::ensure_project_node(&f.core.db.get().unwrap(), project)
                .unwrap()
                .id
        });
        crate::ops::node::node_tab_upsert(
            &f.core,
            runner_core::protocol::NodeTabUpsertInput {
                id: id.into(),
                parent_id,
                name: "Tab".into(),
                layout: r#"{"slots":["chat"]}"#.into(),
            },
        )
        .unwrap();
    }

    #[test]
    fn moving_the_chat_to_another_project_drops_the_link_and_publishes_it() {
        let f = fixture();
        let other = {
            let conn = f.core.db.get().unwrap();
            repo::project::create(&conn, "Other", "/tmp/link-other")
                .unwrap()
                .id
        };
        f.session("chat", "codex", Some(&f.project));
        let mission = f.start(Some("chat"), Some(&f.project), false);
        let mut events = f.core.events.subscribe();

        tab(&f, &ulid::Ulid::new().to_string(), Some(&f.project));
        assert_eq!(linked_to(&f.get(&mission.id)), Some("chat"));
        assert!(!published(&mut events).contains(&"mission/changed"));

        tab(&f, &ulid::Ulid::new().to_string(), Some(&other));
        assert_eq!(f.get(&mission.id).linked_chat, None);
        assert!(published(&mut events).contains(&"mission/changed"));
    }

    #[test]
    fn moving_the_chats_tab_or_the_mission_to_another_project_drops_the_link() {
        let f = fixture();
        f.session("chat", "codex", None);
        let tab_id = ulid::Ulid::new().to_string();
        tab(&f, &tab_id, None);
        let moved = f.start(Some("chat"), None, false);
        let stays = f.start(Some("chat"), None, false);

        crate::ops::mission::mission_set_project(
            &f.core,
            moved.id.clone(),
            Some(f.project.clone()),
        )
        .unwrap();
        assert_eq!(f.get(&moved.id).linked_chat, None);
        assert_eq!(linked_to(&f.get(&stays.id)), Some("chat"));

        let mut events = f.core.events.subscribe();
        let target = repo::node::ensure_project_node(&f.core.db.get().unwrap(), &f.project)
            .unwrap()
            .id;
        let order =
            crate::ops::node::append_order(&f.core.db.get().unwrap(), Some(&target), &tab_id)
                .unwrap();
        crate::ops::node::node_move(&f.core, tab_id, Some(target), order).unwrap();
        assert_eq!(f.get(&stays.id).linked_chat, None);
        assert!(published(&mut events).contains(&"mission/changed"));
    }

    #[test]
    fn unlinking_places_the_mission_right_below_its_chats_tab() {
        let f = fixture();
        f.session("chat", "codex", None);
        f.session("other", "codex", None);
        let chat_tab = ulid::Ulid::new().to_string();
        tab(&f, &chat_tab, None);
        crate::ops::node::node_tab_upsert(
            &f.core,
            runner_core::protocol::NodeTabUpsertInput {
                id: ulid::Ulid::new().to_string(),
                parent_id: None,
                name: "Other".into(),
                layout: r#"{"slots":["other"]}"#.into(),
            },
        )
        .unwrap();
        let mission = f.start(Some("chat"), None, false);
        let root_order = |f: &Fixture| -> Vec<String> {
            let mut nodes: Vec<_> = repo::node::list(&f.core.db.get().unwrap())
                .unwrap()
                .into_iter()
                .filter(|node| node.parent_id.is_none() && node.pinned_position.is_none())
                .collect();
            nodes.sort_by_key(|node| node.position);
            nodes.into_iter().map(|node| node.id).collect()
        };
        let mission_node = repo::node::find_by_ref(
            &f.core.db.get().unwrap(),
            repo::node::NodeType::Mission,
            &mission.id,
        )
        .unwrap()
        .unwrap()
        .id;
        assert_eq!(
            root_order(&f)[0],
            mission_node,
            "the mission node was made first"
        );

        let mut events = f.core.events.subscribe();
        mission_unlink(&f.core, &mission.id).unwrap();

        let order = root_order(&f);
        let tab_at = order.iter().position(|id| *id == chat_tab).unwrap();
        assert_eq!(order[tab_at + 1], mission_node);
        assert!(published(&mut events).contains(&"chat/layout-changed"));
    }

    fn node_of(f: &Fixture, mission: &str) -> repo::node::NodeRow {
        repo::node::find_by_ref(
            &f.core.db.get().unwrap(),
            repo::node::NodeType::Mission,
            mission,
        )
        .unwrap()
        .unwrap()
    }

    fn root_unpinned(f: &Fixture) -> Vec<String> {
        let mut nodes: Vec<_> = repo::node::list(&f.core.db.get().unwrap())
            .unwrap()
            .into_iter()
            .filter(|node| node.parent_id.is_none() && node.pinned_position.is_none())
            .collect();
        nodes.sort_by_key(|node| node.position);
        nodes.into_iter().map(|node| node.id).collect()
    }

    #[test]
    fn pinning_a_linked_mission_pins_its_group_and_linking_drops_its_own_pin() {
        let f = fixture();
        f.session("chat", "codex", None);
        let chat_tab = ulid::Ulid::new().to_string();
        tab(&f, &chat_tab, None);
        let loose = f.start(None, None, false);
        crate::daemon::block_on(super::super::mission_pin_impl(
            &f.core,
            loose.id.clone(),
            true,
        ))
        .unwrap();
        assert!(node_of(&f, &loose.id).pinned_position.is_some());

        f.link(&loose.id, "chat").unwrap();
        assert!(
            node_of(&f, &loose.id).pinned_position.is_none(),
            "a linked mission takes its group's pin"
        );

        crate::daemon::block_on(super::super::mission_pin_impl(
            &f.core,
            loose.id.clone(),
            true,
        ))
        .unwrap();
        let head = repo::node::get(&f.core.db.get().unwrap(), &chat_tab)
            .unwrap()
            .unwrap();
        assert!(head.pinned_position.is_some(), "the group is pinned");
        assert!(node_of(&f, &loose.id).pinned_position.is_none());

        crate::ops::node::node_set_pinned(&f.core, node_of(&f, &loose.id).id, false).unwrap();
        let head = repo::node::get(&f.core.db.get().unwrap(), &chat_tab)
            .unwrap()
            .unwrap();
        assert!(
            head.pinned_position.is_none(),
            "unpinning from the mission unpins the group"
        );
    }

    #[test]
    fn unlinking_from_a_pinned_group_returns_the_mission_unpinned_to_the_top() {
        let f = fixture();
        f.session("chat", "codex", None);
        f.session("other", "codex", None);
        let chat_tab = ulid::Ulid::new().to_string();
        tab(&f, &chat_tab, None);
        let mission = f.start(Some("chat"), None, false);
        crate::ops::node::node_tab_upsert(
            &f.core,
            runner_core::protocol::NodeTabUpsertInput {
                id: ulid::Ulid::new().to_string(),
                parent_id: None,
                name: "Other".into(),
                layout: r#"{"slots":["other"]}"#.into(),
            },
        )
        .unwrap();
        crate::ops::node::node_set_pinned(&f.core, chat_tab.clone(), true).unwrap();
        assert_ne!(
            root_unpinned(&f).first(),
            Some(&node_of(&f, &mission.id).id)
        );

        mission_unlink(&f.core, &mission.id).unwrap();

        let node = node_of(&f, &mission.id);
        assert!(node.pinned_position.is_none(), "the pin was the group's");
        assert_eq!(
            root_unpinned(&f).first(),
            Some(&node.id),
            "where a newly started mission appears"
        );
    }

    #[test]
    fn linked_here_lists_the_chats_unarchived_missions_oldest_first() {
        let f = fixture();
        f.session("chat", "codex", None);
        f.session("other", "codex", None);
        let first = f.start(Some("chat"), None, false);
        let second = f.start(Some("chat"), None, false);
        let archived = f.start(Some("chat"), None, false);
        f.start(Some("other"), None, false);
        f.stop_mission(&archived.id);

        let linked =
            crate::daemon::block_on(super::super::mission_list_linked(&f.core, "chat")).unwrap();

        let ids: Vec<_> = linked
            .iter()
            .map(|summary| summary.mission.id.as_str())
            .collect();
        assert_eq!(ids, [first.id.as_str(), second.id.as_str()]);
    }

    #[test]
    fn archiving_the_chat_with_missions_archives_only_its_unarchived_linked_missions() {
        let f = fixture();
        f.session("chat", "codex", None);
        f.session("other", "codex", None);
        let running = f.start(Some("chat"), None, false);
        let settled_stopped = f.start(Some("chat"), None, false);
        crate::daemon::block_on(crate::ops::mission::mission_stop_impl(
            &f.core,
            settled_stopped.id.clone(),
        ))
        .unwrap();
        let already_archived = f.start(Some("chat"), None, false);
        f.stop_mission(&already_archived.id);
        let other_mission = f.start(Some("other"), None, false);

        // Attach a drawer shell to the running linked mission to verify mission_archive_impl teardown.
        f.session("drawer-shell", "shell", None);
        let running_node = node_of(&f, &running.id);
        crate::ops::node::node_mission_layout_set(
            &f.core,
            &running_node.id,
            r#"{"drawer":{"shells":["drawer-shell"]}}"#.into(),
        )
        .unwrap();

        let mut events = f.core.events.subscribe();
        let archived_ids =
            crate::ops::session::session_archive_direct_with_missions(&f.core, "chat", true)
                .unwrap();

        assert_eq!(
            archived_ids,
            vec![running.id.clone(), settled_stopped.id.clone()]
        );
        assert!(
            crate::ops::session::session_get(&f.core, "chat")
                .unwrap()
                .unwrap()
                .archived_at
                .is_some(),
            "chat itself is stopped and archived"
        );
        for id in [&running.id, &settled_stopped.id, &already_archived.id] {
            let mission = f.get(id);
            assert!(mission.archived_at.is_some(), "{id} should be archived");
            let linked = mission.linked_chat.expect("linked_session_id stays intact");
            assert_eq!(linked.session_id, "chat");
            assert!(linked.archived);
        }
        assert!(
            repo::session::get_row(&f.core.db.get().unwrap(), "drawer-shell")
                .unwrap()
                .is_none(),
            "mission drawer shell is permanently closed during mission_archive_impl teardown"
        );
        assert!(
            f.get(&other_mission.id).archived_at.is_none(),
            "another chat's linked mission stays unarchived"
        );
        assert!(published(&mut events).contains(&"mission/changed"));
    }

    #[test]
    fn archiving_the_chat_without_with_missions_leaves_linked_missions_unarchived() {
        let f = fixture();
        f.session("chat", "codex", None);
        let linked = f.start(Some("chat"), None, false);

        let archived_ids =
            crate::ops::session::session_archive_direct_with_missions(&f.core, "chat", false)
                .unwrap();

        assert!(archived_ids.is_empty());
        let mission = f.get(&linked.id);
        assert!(mission.archived_at.is_none());
        assert_eq!(linked_to(&mission), Some("chat"));
        assert!(mission.linked_chat.unwrap().archived);
    }
}
