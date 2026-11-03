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
    repo::mission::set_link(&tx, &mission.id, Some(&chat.row.id), !input.no_report)?;
    let mission = super::get(&tx, &mission.id)?;
    tx.commit()?;
    Ok(mission)
}

pub fn mission_unlink(core: &AppCore, mission_id: &str) -> Result<Mission> {
    let mut conn = core.db.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let mission = super::get(&tx, mission_id)?;
    repo::mission::set_link(&tx, &mission.id, None, true)?;
    let mission = super::get(&tx, &mission.id)?;
    tx.commit()?;
    Ok(mission)
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
}
