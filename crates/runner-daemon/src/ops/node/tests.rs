use chrono::{DateTime, FixedOffset, Utc};
use tokio::sync::broadcast;

use super::*;
use crate::events::AppEvent;
use crate::test_support::{test_core, test_core_in};

#[test]
fn overlapping_tab_upserts_reconcile_current_membership_and_input_owner() {
    use runner_core::companion::Controller;
    use std::sync::{Arc, Barrier};
    let state = test_core();
    let a = create_tab(&state, &["a", "p"]);
    let b = create_tab(&state, &["b"]);
    let events: Arc<dyn crate::session::manager::SessionEvents> = Arc::new(state.session_events());
    for id in ["a", "p", "b"] {
        state
            .sessions
            .prepare_unlisted_terminal(id, (80, 24), &state.db, &events)
            .unwrap();
    }
    let input = state.sessions.record_terminal_input("p");
    state
        .sessions
        .browser_resize("a", 1, "First", 80, 24, true, false, &state.db)
        .unwrap();
    state
        .sessions
        .browser_resize("b", 2, "Second", 80, 24, true, false, &state.db)
        .unwrap();
    let committed = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    let worker = {
        let state = state.clone();
        let committed = committed.clone();
        let resume = resume.clone();
        std::thread::spawn(move || {
            let (row, _) = persist_tab_layout(
                &state,
                NodeTabUpsertInput {
                    id: a.id,
                    parent_id: None,
                    name: "A".into(),
                    layout: serde_json::json!({"slots": ["a", "p"]}).to_string(),
                },
            )
            .unwrap();
            committed.wait();
            resume.wait();
            state
                .sessions
                .reconcile_tab_control(&row.id, &state.db)
                .unwrap();
        })
    };
    committed.wait();
    node_tab_upsert(
        &state,
        NodeTabUpsertInput {
            id: b.id.clone(),
            parent_id: None,
            name: "B".into(),
            layout: serde_json::json!({"slots": ["b", "p"]}).to_string(),
        },
    )
    .unwrap();
    resume.wait();
    worker.join().unwrap();
    assert_eq!(
        repo::node::find_for_session(&state.db.get().unwrap(), "p")
            .unwrap()
            .unwrap()
            .id,
        b.id
    );
    assert_eq!(state.sessions.browser_controller("a", 1), Controller::You);
    assert_eq!(state.sessions.browser_controller("p", 2), Controller::You);
    assert!(state.sessions.browser_input("p", 1, b"stale").is_err());
    assert!(input.try_recv().is_err());
    state.sessions.browser_input("p", 2, b"current").unwrap();
    assert_eq!(
        input
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap(),
        b"current"
    );
    state.sessions.drain_terminal_workers();
}

fn create_tab(state: &AppCore, session_ids: &[&str]) -> NodeRow {
    let layout = serde_json::json!({
        "preset": if session_ids.len() == 1 { "single" } else { "cols-2" },
        "slots": session_ids,
        "sizes": {},
    });
    let conn = state.db.get().unwrap();
    for id in session_ids {
        conn.execute("INSERT OR IGNORE INTO sessions(id, status, agent_runtime, agent_command) VALUES (?1, 'running', 'codex', 'codex')", [id]).unwrap();
    }
    repo::node::create_tab(&conn, None, "chat", 0, &layout.to_string()).unwrap()
}

/// Count `chat/tab-attention-changed` events delivered since the
/// receiver subscribed. Every emit in these tests happens on the
/// calling thread, so draining afterwards observes them all.
fn drain_attention_count(rx: &mut broadcast::Receiver<AppEvent>) -> usize {
    let mut count = 0;
    while let Ok(ev) = rx.try_recv() {
        if ev.name == ATTENTION_CHANGED_EVENT {
            count += 1;
        }
    }
    count
}

fn parsed(value: Option<&str>) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(value.expect("timestamp")).unwrap()
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

#[test]
fn tab_delete_removes_an_empty_tab_and_emits_layout_changed() {
    let state = test_core();
    let tab = create_tab(&state, &[]);
    let mut events = state.events.subscribe();
    node_tab_delete(&state, &tab.id).unwrap();
    assert!(repo::node::get(&state.db.get().unwrap(), &tab.id)
        .unwrap()
        .is_none());
    assert_eq!(events.try_recv().unwrap().name, LAYOUT_CHANGED_EVENT);
    assert!(events.try_recv().is_err());
}

#[test]
fn tab_delete_refuses_project_mission_and_occupied_tab_nodes() {
    let state = test_core();
    seed_mission(&state, "mission", None);
    let (project, mission) = {
        let conn = state.db.get().unwrap();
        let project = repo::project::create(&conn, "Project", "/tmp/project").unwrap();
        (
            repo::node::ensure_project_node(&conn, &project.id).unwrap(),
            repo::node::ensure_mission_node(&conn, "mission", None).unwrap(),
        )
    };
    let tab = create_tab(&state, &["chat"]);
    let mut events = state.events.subscribe();
    for node in [project, mission, tab] {
        assert!(node_tab_delete(&state, &node.id).is_err());
        assert!(repo::node::get(&state.db.get().unwrap(), &node.id)
            .unwrap()
            .is_some());
    }
    assert!(events.try_recv().is_err());
}

#[test]
fn tab_upsert_writes_its_parent_project_to_every_member_session() {
    let state = test_core();
    let (project, project_node) = {
        let conn = state.db.get().unwrap();
        let project = repo::project::create(&conn, "P", "/tmp/p").unwrap();
        let project_node = repo::node::ensure_project_node(&conn, &project.id).unwrap();
        for id in ["root-pane", "root-drawer"] {
            conn.execute(
                "INSERT INTO sessions (id, status, project_id) VALUES (?1, 'stopped', ?2)",
                rusqlite::params![id, project.id],
            )
            .unwrap();
        }
        for id in ["project-pane", "project-drawer"] {
            conn.execute(
                "INSERT INTO sessions (id, status) VALUES (?1, 'stopped')",
                [id],
            )
            .unwrap();
        }
        (project, project_node)
    };

    node_tab_upsert(
        &state,
        NodeTabUpsertInput {
            id: ulid::Ulid::new().to_string(),
            parent_id: None,
            name: "root".into(),
            layout: r#"{"slots":["root-pane"],"drawer":{"shells":["root-drawer"]}}"#.into(),
        },
    )
    .unwrap();
    node_tab_upsert(
        &state,
        NodeTabUpsertInput {
            id: ulid::Ulid::new().to_string(),
            parent_id: Some(project_node.id),
            name: "project".into(),
            layout: r#"{"slots":["project-pane"],"drawer":{"shells":["project-drawer"]}}"#.into(),
        },
    )
    .unwrap();

    let conn = state.db.get().unwrap();
    for id in ["root-pane", "root-drawer"] {
        let project_id: Option<String> = conn
            .query_row(
                "SELECT project_id FROM sessions WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(project_id, None);
    }
    for id in ["project-pane", "project-drawer"] {
        let project_id: Option<String> = conn
            .query_row(
                "SELECT project_id FROM sessions WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(project_id.as_deref(), Some(project.id.as_str()));
    }
}

fn seed_mission_with_status(state: &AppCore, id: &str, project_id: Option<&str>, status: &str) {
    let conn = state.db.get().unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO crews (id, name, created_at, updated_at)
             VALUES ('c1', 'Crew', '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO missions (id, crew_id, title, status, started_at, project_id)
             VALUES (?1, 'c1', 'M', ?3, '2026-07-01T00:00:00Z', ?2)",
        rusqlite::params![id, project_id, status],
    )
    .unwrap();
}

fn seed_mission(state: &AppCore, id: &str, project_id: Option<&str>) {
    seed_mission_with_status(state, id, project_id, "aborted");
}

#[test]
fn mission_layout_set_requires_a_mission_and_moves_shells_out_of_tabs() {
    let state = test_core();
    let project = {
        let conn = state.db.get().unwrap();
        repo::project::create(&conn, "P", "/tmp/p").unwrap()
    };
    seed_mission(&state, "m1", None);
    let (mission_node, tab) = {
        let conn = state.db.get().unwrap();
        conn.execute(
            "INSERT INTO sessions (id, status, agent_runtime, agent_command, project_id)
                 VALUES ('drawer-shell', 'stopped', 'shell', '/bin/zsh', ?1)",
            [&project.id],
        )
        .unwrap();
        let mission_node = repo::node::ensure_mission_node(&conn, "m1", None).unwrap();
        let tab = repo::node::create_tab(
            &conn,
            None,
            "terminal",
            0,
            r#"{"preset":"single","slots":["drawer-shell"],"sizes":{}}"#,
        )
        .unwrap();
        (mission_node, tab)
    };
    let mut events = state.events.subscribe();
    let layout = r#"{"drawer":{"open":true,"height":280,"shells":["drawer-shell"],"active":0}}"#;

    let row = node_mission_layout_set(&state, &mission_node.id, layout.into()).unwrap();

    assert_eq!(row.layout.as_deref(), Some(layout));
    let conn = state.db.get().unwrap();
    assert!(repo::node::get(&conn, &tab.id).unwrap().is_none());
    assert_eq!(repo::node::session_ids(&row), ["drawer-shell"]);
    let project_id: Option<String> = conn
        .query_row(
            "SELECT project_id FROM sessions WHERE id = 'drawer-shell'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(project_id, None);
    assert_eq!(events.try_recv().unwrap().name, LAYOUT_CHANGED_EVENT);

    let tab = repo::node::create_tab(
        &conn,
        None,
        "chat",
        0,
        r#"{"preset":"single","slots":[],"sizes":{}}"#,
    )
    .unwrap();
    drop(conn);
    let error = node_mission_layout_set(&state, &tab.id, "{}".into()).unwrap_err();
    assert!(error.to_string().contains("is not a mission"));
}

#[test]
fn mission_archive_closes_drawer_shells_and_tolerates_stale_ids() {
    let archive_state = test_core();
    seed_mission(&archive_state, "archive-me", None);
    {
        let conn = archive_state.db.get().unwrap();
        conn.execute(
            "INSERT INTO sessions (id, status, agent_runtime, agent_command)
                 VALUES ('archive-shell', 'stopped', 'shell', '/bin/zsh')",
            [],
        )
        .unwrap();
        let node = repo::node::ensure_mission_node(&conn, "archive-me", None).unwrap();
        conn.execute(
                "UPDATE nodes SET layout = ?2 WHERE id = ?1",
                rusqlite::params![
                    node.id,
                    r#"{"drawer":{"open":true,"height":280,"shells":["archive-shell","missing-shell"],"active":0}}"#
                ],
            )
            .unwrap();
    }

    block_on(crate::ops::mission::mission_archive_impl(
        &archive_state,
        "archive-me".into(),
    ))
    .unwrap();

    let conn = archive_state.db.get().unwrap();
    assert!(repo::session::get_row(&conn, "archive-shell")
        .unwrap()
        .is_none());
    assert!(
        repo::node::find_by_ref(&conn, NodeType::Mission, "archive-me")
            .unwrap()
            .is_none()
    );
}

#[test]
fn mission_delete_closes_shells_if_an_archived_row_retains_a_drawer_node() {
    // Normal archive deletes the node; this covers interrupted or legacy persisted state.
    let delete_state = test_core();
    seed_mission(&delete_state, "delete-me", None);
    {
        let conn = delete_state.db.get().unwrap();
        conn.execute(
            "UPDATE missions SET archived_at = '2026-09-04T00:00:00Z'
                 WHERE id = 'delete-me'",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, status, agent_runtime, agent_command)
                 VALUES ('delete-shell', 'stopped', 'shell', '/bin/zsh')",
            [],
        )
        .unwrap();
        let node = repo::node::ensure_mission_node(&conn, "delete-me", None).unwrap();
        conn.execute(
            "UPDATE nodes SET layout = ?2 WHERE id = ?1",
            rusqlite::params![
                node.id,
                r#"{"drawer":{"open":true,"height":280,"shells":["delete-shell"],"active":0}}"#
            ],
        )
        .unwrap();
    }

    crate::ops::mission::mission_delete(&delete_state, "delete-me").unwrap();

    let conn = delete_state.db.get().unwrap();
    assert!(repo::session::get_row(&conn, "delete-shell")
        .unwrap()
        .is_none());
    assert!(repo::mission::get(&conn, "delete-me").unwrap().is_none());
}

#[test]
fn archive_mission_step_stamps_and_deletes_node_atomically() {
    let state = test_core();
    seed_mission(&state, "m1", None);
    {
        let conn = state.db.get().unwrap();
        repo::node::ensure_mission_node(&conn, "m1", None).unwrap();
    }

    {
        let mut conn = state.db.get().unwrap();
        assert!(matches!(
            archive_mission_step(&mut conn, "m1").unwrap(),
            MissionArchiveStep::Done
        ));
    }

    let conn = state.db.get().unwrap();
    let archived_at: Option<String> = conn
        .query_row(
            "SELECT archived_at FROM missions WHERE id = 'm1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(archived_at.is_some());
    assert!(
        repo::node::find_by_ref(&conn, repo::node::NodeType::Mission, "m1")
            .unwrap()
            .is_none()
    );
}

/// Concurrency guard at the snapshot/action boundary: the caller's
/// status snapshot says `aborted`, but the mission is running when
/// the action executes. The stamp path must NOT fire — the loop
/// re-reads and takes the full archive path, which terminates the
/// run properly (status flips to completed) instead of stamping
/// `archived_at` onto a still-running mission.
#[test]
fn stale_status_snapshot_never_stamps_a_running_mission() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_core_in(dir.path().to_path_buf());
    seed_mission_with_status(&state, "m1", None, "running");
    {
        let conn = state.db.get().unwrap();
        repo::node::ensure_mission_node(&conn, "m1", None).unwrap();
    }

    block_on(archive_child_missions(
        &state,
        &[("m1".to_string(), crate::model::MissionStatus::Aborted)],
    ))
    .unwrap();

    let conn = state.db.get().unwrap();
    let (status, archived_at): (String, Option<String>) = conn
        .query_row(
            "SELECT status, archived_at FROM missions WHERE id = 'm1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        status, "completed",
        "the full archive path ran (stale stamp would have left it 'running')"
    );
    assert!(archived_at.is_some());
    assert!(
        repo::node::find_by_ref(&conn, repo::node::NodeType::Mission, "m1")
            .unwrap()
            .is_none()
    );
}

#[test]
fn project_delete_archives_children_and_unbinds_pointers() {
    let state = test_core();
    let project = {
        let conn = state.db.get().unwrap();
        crate::repo::project::create(&conn, "P", "/tmp/p").unwrap()
    };
    {
        let conn = state.db.get().unwrap();
        conn.execute(
            "INSERT INTO sessions (id, status, project_id) VALUES ('s1', 'stopped', ?1)",
            [&project.id],
        )
        .unwrap();
    }
    seed_mission(&state, "m1", Some(&project.id));
    {
        let conn = state.db.get().unwrap();
        let project_node = repo::node::ensure_project_node(&conn, &project.id).unwrap();
        repo::node::create_tab(
            &conn,
            Some(&project_node.id),
            "",
            0,
            r#"{"preset":"single","slots":["s1"],"sizes":{}}"#,
        )
        .unwrap();
        repo::node::ensure_mission_node(&conn, "m1", Some(&project.id)).unwrap();
    }

    let archived = block_on(crate::ops::project::project_delete_impl(
        &state,
        &project.id,
    ))
    .unwrap();
    assert_eq!(archived.archived_session_ids, vec!["s1".to_string()]);
    assert_eq!(archived.archived_mission_ids, vec!["m1".to_string()]);

    let conn = state.db.get().unwrap();
    assert!(crate::repo::project::get(&conn, &project.id)
        .unwrap()
        .is_none());
    let (session_archived, session_project): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT archived_at, project_id FROM sessions WHERE id = 's1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(session_archived.is_some());
    assert_eq!(
        session_project, None,
        "row delete unbinds the archived chat's pointer"
    );
    let mission_archived: Option<String> = conn
        .query_row(
            "SELECT archived_at FROM missions WHERE id = 'm1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(mission_archived.is_some());
    let remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get(0))
        .unwrap();
    assert_eq!(remaining, 0, "project, tab, and mission nodes all gone");
}

fn insert_chat(conn: &rusqlite::Connection, id: &str) {
    conn.execute(
        "INSERT INTO sessions (id, status) VALUES (?1, 'stopped')",
        [id],
    )
    .unwrap();
}

fn insert_mission(conn: &rusqlite::Connection, id: &str, project_id: &str) {
    conn.execute(
        "INSERT OR IGNORE INTO crews (id, name, created_at, updated_at)
             VALUES ('c1', 'Crew', '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO missions (id, crew_id, title, status, started_at, project_id)
             VALUES (?1, 'c1', 'M', 'running', '2026-07-01T00:00:00Z', ?2)",
        rusqlite::params![id, project_id],
    )
    .unwrap();
}

#[test]
fn project_tab_delete_includes_drawer_shells() {
    let pool = crate::db::open_in_memory().unwrap();
    let mut conn = pool.get().unwrap();
    insert_chat(&conn, "chat");
    conn.execute(
        "INSERT INTO sessions (id, status, agent_runtime, agent_command)
             VALUES ('drawer-terminal', 'stopped', 'shell', '/bin/zsh')",
        [],
    )
    .unwrap();
    let project = repo::project::create(&conn, "Project", "/tmp/project").unwrap();
    let project_node = repo::node::ensure_project_node(&conn, &project.id).unwrap();
    repo::node::create_tab(
            &conn,
            Some(&project_node.id),
            "",
            0,
            r#"{"preset":"single","slots":["chat"],"sizes":{},"drawer":{"open":true,"height":280,"shells":["drawer-terminal"],"active":0}}"#,
        )
        .unwrap();

    let tx = conn.transaction().unwrap();
    let (archived_ids, deleted_ids) =
        delete_container_tabs_and_archive(&tx, &project_node.id).unwrap();
    tx.commit().unwrap();

    assert_eq!(archived_ids, ["chat"]);
    assert_eq!(deleted_ids, ["drawer-terminal"]);
}

#[test]
fn project_tab_delete_archives_chats_and_deletes_terminals() {
    let pool = crate::db::open_in_memory().unwrap();
    let mut conn = pool.get().unwrap();
    insert_chat(&conn, "chat");
    conn.execute(
        "INSERT INTO sessions (id, status, agent_runtime, agent_command)
             VALUES ('terminal', 'stopped', 'shell', '/bin/zsh')",
        [],
    )
    .unwrap();
    let project = repo::project::create(&conn, "Project", "/tmp/project").unwrap();
    let project_node = repo::node::ensure_project_node(&conn, &project.id).unwrap();
    repo::node::create_tab(
        &conn,
        Some(&project_node.id),
        "",
        0,
        r#"{"preset":"cols-2","slots":["chat","terminal"],"sizes":{}}"#,
    )
    .unwrap();
    assert!(conn
        .execute("DELETE FROM nodes WHERE id = ?1", [&project_node.id])
        .is_err());

    let tx = conn.transaction().unwrap();
    let (archived_ids, deleted_ids) =
        delete_container_tabs_and_archive(&tx, &project_node.id).unwrap();
    assert_eq!(archived_ids, ["chat"]);
    assert_eq!(deleted_ids, ["terminal"]);
    repo::node::delete(&tx, &project_node.id).unwrap();
    tx.commit().unwrap();

    let archived: Option<String> = conn
        .query_row(
            "SELECT archived_at FROM sessions WHERE id = 'chat'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(archived.is_some());
    assert!(repo::session::get_row(&conn, "terminal").unwrap().is_none());
    assert!(repo::node::get(&conn, &project_node.id).unwrap().is_none());
    assert!(repo::node::list(&conn).unwrap().is_empty());
}

#[test]
fn project_tab_delete_removes_terminal_attention_with_foreign_keys_off() {
    use crate::test_support::{foreign_keys_off, row_count};
    let pool = crate::db::open_in_memory().unwrap();
    let mut conn = pool.get().unwrap();
    foreign_keys_off(&conn);
    insert_chat(&conn, "chat");
    conn.execute(
        "INSERT INTO sessions (id, status, agent_runtime, agent_command)
             VALUES ('terminal', 'stopped', 'shell', '/bin/zsh'),
                    ('elsewhere', 'stopped', 'shell', '/bin/zsh')",
        [],
    )
    .unwrap();
    for id in ["chat", "terminal", "elsewhere"] {
        repo::session_attention::record_completion(&conn, id, false, 100).unwrap();
    }
    let project = repo::project::create(&conn, "Project", "/tmp/project").unwrap();
    let project_node = repo::node::ensure_project_node(&conn, &project.id).unwrap();
    repo::node::create_tab(
        &conn,
        Some(&project_node.id),
        "",
        0,
        r#"{"preset":"cols-2","slots":["chat","terminal"],"sizes":{}}"#,
    )
    .unwrap();
    let other_tab = repo::node::create_tab(
        &conn,
        None,
        "",
        1,
        r#"{"preset":"single","slots":["elsewhere"],"sizes":{}}"#,
    )
    .unwrap();

    let tx = conn.transaction().unwrap();
    delete_container_tabs_and_archive(&tx, &project_node.id).unwrap();
    tx.commit().unwrap();

    assert!(repo::session::get_row(&conn, "terminal").unwrap().is_none());
    let attention: Vec<String> = conn
        .prepare("SELECT session_id FROM session_attention ORDER BY session_id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        attention,
        ["chat", "elsewhere"],
        "the closed terminal's attention row goes; the archived chat keeps its own"
    );
    let nodes: Vec<String> = repo::node::list(&conn)
        .unwrap()
        .into_iter()
        .map(|n| n.id)
        .collect();
    assert_eq!(nodes, [project_node.id.clone(), other_tab.id]);
    assert_eq!(
        row_count(&conn, "SELECT COUNT(*) FROM nodes WHERE type = 'tab'"),
        1
    );
}

#[test]
fn container_delete_is_blocked_by_remaining_children() {
    let pool = crate::db::open_in_memory().unwrap();
    let mut conn = pool.get().unwrap();
    let project = repo::project::create(&conn, "A", "/tmp/a").unwrap();
    let project_node = repo::node::ensure_project_node(&conn, &project.id).unwrap();
    insert_mission(&conn, "m1", &project.id);
    let mission_node = repo::node::ensure_mission_node(&conn, "m1", Some(&project.id)).unwrap();

    let tx = conn.transaction().unwrap();
    let (archived, deleted) = delete_container_tabs_and_archive(&tx, &project_node.id).unwrap();
    assert!(archived.is_empty());
    assert!(deleted.is_empty());
    assert!(
        repo::node::delete(&tx, &project_node.id).is_err(),
        "RESTRICT must block the project-node delete while a child remains"
    );
    drop(tx); // rollback

    assert!(repo::node::get(&conn, &project_node.id).unwrap().is_some());
    assert_eq!(
        repo::node::get(&conn, &mission_node.id)
            .unwrap()
            .unwrap()
            .parent_id
            .as_deref(),
        Some(project_node.id.as_str())
    );
}

fn seed_project_members(state: &AppCore, name: &str) -> repo::project::ProjectRow {
    let project = {
        let conn = state.db.get().unwrap();
        repo::project::create(&conn, name, &format!("/tmp/{name}")).unwrap()
    };
    seed_mission(state, &format!("{name}-mission"), Some(&project.id));
    let conn = state.db.get().unwrap();
    let (chat, terminal) = (format!("{name}-chat"), format!("{name}-terminal"));
    conn.execute(
        "INSERT INTO sessions (id, status, project_id) VALUES (?1, 'stopped', ?2)",
        rusqlite::params![chat, project.id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO sessions (id, status, agent_runtime, agent_command, project_id)
             VALUES (?1, 'stopped', 'shell', '/bin/zsh', ?2)",
        rusqlite::params![terminal, project.id],
    )
    .unwrap();
    for id in [&chat, &terminal] {
        repo::session_attention::record_completion(&conn, id, false, 100).unwrap();
    }
    let project_node = repo::node::ensure_project_node(&conn, &project.id).unwrap();
    repo::node::create_tab(
        &conn,
        Some(&project_node.id),
        "",
        0,
        &format!(r#"{{"preset":"cols-2","slots":["{chat}","{terminal}"],"sizes":{{}}}}"#),
    )
    .unwrap();
    repo::node::ensure_mission_node(&conn, &format!("{name}-mission"), Some(&project.id)).unwrap();
    project
}

#[test]
fn project_delete_unbinds_members_with_foreign_keys_off() {
    use crate::test_support::{foreign_keys_off, row_count};
    let state = test_core();
    foreign_keys_off(&state.db.get().unwrap());
    let doomed = seed_project_members(&state, "doomed");
    let kept = seed_project_members(&state, "kept");

    block_on(crate::ops::project::project_delete_impl(&state, &doomed.id)).unwrap();

    let conn = state.db.get().unwrap();
    assert_eq!(
        row_count(&conn, "SELECT foreign_keys FROM pragma_foreign_keys"),
        0
    );
    assert!(repo::project::get(&conn, &doomed.id).unwrap().is_none());
    let project_of = |table: &str, id: &str| -> Option<String> {
        conn.query_row(
            &format!("SELECT project_id FROM {table} WHERE id = ?1"),
            [id],
            |row| row.get(0),
        )
        .unwrap()
    };
    assert_eq!(project_of("sessions", "doomed-chat"), None);
    assert_eq!(project_of("missions", "doomed-mission"), None);
    assert_eq!(project_of("sessions", "kept-chat"), Some(kept.id.clone()));
    assert_eq!(
        project_of("sessions", "kept-terminal"),
        Some(kept.id.clone())
    );
    assert_eq!(
        project_of("missions", "kept-mission"),
        Some(kept.id.clone())
    );
    assert_eq!(
        row_count(
            &conn,
            "SELECT COUNT(*) FROM sessions WHERE id = 'doomed-terminal'"
        ),
        0,
        "the project's terminal is closed"
    );
    assert_eq!(
        row_count(
            &conn,
            "SELECT COUNT(*) FROM session_attention WHERE session_id = 'doomed-terminal'"
        ),
        0,
        "with its attention row"
    );
    assert_eq!(
        row_count(&conn, "SELECT COUNT(*) FROM session_attention"),
        3
    );
    let nodes: Vec<(String, Option<String>)> = conn
        .prepare("SELECT type, parent_id FROM nodes ORDER BY type")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        nodes.len(),
        3,
        "only the kept project's nodes remain: {nodes:?}"
    );
    assert!(nodes
        .iter()
        .all(|(kind, parent)| kind == "project" || parent.is_some()));
}

/// Seeds a project whose delete will meet a late child: a mission node
/// the pre-transaction archive sweep cannot see (its mission row is not
/// there to archive), standing in for one another window moved in
/// during the gap. `extra_tab` adds a tab holding those session ids.
fn seed_project_with_late_child(state: &AppCore, extra_tab: &[&str]) -> String {
    let project = seed_project_members(state, "doomed");
    let conn = state.db.get().unwrap();
    let project_node = repo::node::find_by_ref(&conn, NodeType::Project, &project.id)
        .unwrap()
        .unwrap();
    conn.execute(
        "INSERT INTO nodes (id, parent_id, position, type, ref_id, created_at)
             VALUES ('late', ?1, 9, 'mission', 'ghost', '2026-07-01T00:00:00Z')",
        [&project_node.id],
    )
    .unwrap();
    if !extra_tab.is_empty() {
        let layout = serde_json::json!({ "preset": "single", "slots": extra_tab, "sizes": {} });
        repo::node::create_tab(&conn, Some(&project_node.id), "", 1, &layout.to_string()).unwrap();
    }
    project.id
}

/// Nothing the refused transaction owns changed. The member mission's
/// archive commits on its own before the transaction, by design.
fn assert_project_delete_wrote_nothing(state: &AppCore, project_id: &str, node_types: &[&str]) {
    use crate::test_support::row_count;
    let conn = state.db.get().unwrap();
    assert_eq!(
        row_count(&conn, "SELECT foreign_keys FROM pragma_foreign_keys"),
        0
    );
    assert!(repo::project::get(&conn, project_id).unwrap().is_some());
    let found: Vec<String> = conn
        .prepare("SELECT type FROM nodes ORDER BY type")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(found, node_types);
    assert_eq!(
        row_count(
            &conn,
            "SELECT COUNT(*) FROM sessions
                  WHERE id IN ('doomed-chat', 'doomed-terminal')
                    AND archived_at IS NULL AND project_id IS NOT NULL"
        ),
        2,
        "the chat is not archived, the terminal not closed, neither unbound"
    );
    assert_eq!(
        row_count(&conn, "SELECT COUNT(*) FROM session_attention"),
        2
    );
    assert_eq!(
        row_count(
            &conn,
            "SELECT COUNT(*) FROM missions WHERE project_id IS NOT NULL"
        ),
        1
    );
}

#[test]
fn project_delete_refuses_a_late_child_and_writes_nothing_with_foreign_keys_off() {
    let state = test_core();
    crate::test_support::foreign_keys_off(&state.db.get().unwrap());
    let project_id = seed_project_with_late_child(&state, &[]);

    let error = block_on(crate::ops::project::project_delete_impl(
        &state,
        &project_id,
    ))
    .unwrap_err()
    .to_string();

    assert_eq!(error, "sqlite: FOREIGN KEY constraint failed");
    assert_project_delete_wrote_nothing(&state, &project_id, &["mission", "project", "tab"]);
}

#[test]
fn project_delete_reports_an_unremovable_tab_session_before_a_late_child() {
    let state = test_core();
    crate::test_support::foreign_keys_off(&state.db.get().unwrap());
    let project_id = seed_project_with_late_child(&state, &["gone"]);

    let error = block_on(crate::ops::project::project_delete_impl(
        &state,
        &project_id,
    ))
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("session gone is missing or still running"),
        "{error}"
    );
    assert_project_delete_wrote_nothing(&state, &project_id, &["mission", "project", "tab", "tab"]);
}

#[test]
fn armed_final_idle_in_focused_window_completes_and_views_tab() {
    let state = test_core();
    let tab = create_tab(&state, &["a"]);
    state.windows.register("main");
    state
        .windows
        .set_subjects("main", vec![Subject::DirectChat("a".to_string())]);
    state.windows.mark_focused("main");
    state.windows.set_viewed_session("main", Some("a"));
    let events = state.session_events();

    state.sessions.publish_direct_activity(
        "a",
        SessionActivityState::Busy,
        crate::session::state::StatusSource::Test,
        &events,
    );
    state.sessions.arm_completion("a");
    state.sessions.publish_direct_activity(
        "a",
        SessionActivityState::Idle,
        crate::session::state::StatusSource::Test,
        &events,
    );

    let row = repo::node::get(&state.db.get().unwrap(), &tab.id)
        .unwrap()
        .unwrap();
    assert_eq!(row.last_completed_at, row.last_viewed_at);
    assert!(row.last_completed_at.is_some());
}

#[test]
fn armed_final_idle_in_background_marks_tab_unread() {
    let state = test_core();
    let tab = create_tab(&state, &["a"]);
    state.windows.register("main");
    state
        .windows
        .set_subjects("main", vec![Subject::DirectChat("a".to_string())]);
    let events = state.session_events();

    state.sessions.publish_direct_activity(
        "a",
        SessionActivityState::Busy,
        crate::session::state::StatusSource::Test,
        &events,
    );
    state.sessions.arm_completion("a");
    state.sessions.publish_direct_activity(
        "a",
        SessionActivityState::Idle,
        crate::session::state::StatusSource::Test,
        &events,
    );

    let row = repo::node::get(&state.db.get().unwrap(), &tab.id)
        .unwrap()
        .unwrap();
    assert!(row.last_completed_at.is_some());
    assert!(row.last_viewed_at.is_none());
}

#[test]
fn spontaneous_settle_does_not_complete_tab_or_emit_invalidation() {
    let state = test_core();
    let tab = create_tab(&state, &["a"]);
    let mut rx = state.events.subscribe();
    let events = state.session_events();

    state.sessions.publish_direct_activity(
        "a",
        SessionActivityState::Busy,
        crate::session::state::StatusSource::Test,
        &events,
    );
    state.sessions.publish_direct_activity(
        "a",
        SessionActivityState::Idle,
        crate::session::state::StatusSource::Test,
        &events,
    );

    let row = repo::node::get(&state.db.get().unwrap(), &tab.id)
        .unwrap()
        .unwrap();
    assert!(row.last_completed_at.is_none());
    assert!(row.last_viewed_at.is_none());
    assert_eq!(drain_attention_count(&mut rx), 0);
}

#[test]
fn completion_belongs_to_its_member_while_the_peer_keeps_working() {
    let state = test_core();
    let tab = create_tab(&state, &["a", "b"]);
    let events = state.session_events();

    state.sessions.publish_direct_activity(
        "a",
        SessionActivityState::Busy,
        crate::session::state::StatusSource::Test,
        &events,
    );
    state.sessions.publish_direct_activity(
        "b",
        SessionActivityState::Busy,
        crate::session::state::StatusSource::Test,
        &events,
    );
    state.sessions.arm_completion("a");
    state.sessions.publish_direct_activity(
        "a",
        SessionActivityState::Idle,
        crate::session::state::StatusSource::Test,
        &events,
    );

    let row = repo::node::get(&state.db.get().unwrap(), &tab.id)
        .unwrap()
        .unwrap();
    assert!(row.last_completed_at.is_some());
    assert!(state.sessions.agent_status("a").unread_since.is_some());
    assert!(state.sessions.agent_status("b").unread_since.is_none());
    assert!(row.last_viewed_at.is_none());

    state.sessions.publish_direct_activity(
        "b",
        SessionActivityState::Idle,
        crate::session::state::StatusSource::Test,
        &events,
    );

    let row = repo::node::get(&state.db.get().unwrap(), &tab.id)
        .unwrap()
        .unwrap();
    assert!(row.last_completed_at.is_some());
    assert!(row.last_viewed_at.is_none());
}

#[test]
fn provisional_escape_does_not_trigger_or_consume_completion() {
    use crate::session::manager::{SessionActivityEvent, SessionEvents};

    let state = test_core();
    let tab = create_tab(&state, &["a"]);
    let mut rx = state.events.subscribe();
    let events = state.session_events();
    state.sessions.note_forwarder_transition(
        "a",
        SessionActivityState::Idle,
        crate::session::state::StatusSource::Test,
    );
    state.sessions.arm_completion("a");
    events.status(&SessionActivityEvent {
        session_id: "a".into(),
        state: SessionActivityState::Idle,
        source: crate::session::state::StatusSource::InputEscape,
        status: crate::session::status::AgentStatus {
            observation: crate::session::status::AgentObservation {
                outcome: Some(crate::session::status::TurnOutcome::Interrupted),
                ..Default::default()
            },
            ..Default::default()
        },
    });
    let row = repo::node::get(&state.db.get().unwrap(), &tab.id)
        .unwrap()
        .unwrap();
    assert!(row.last_completed_at.is_none());
    assert_eq!(drain_attention_count(&mut rx), 0);
    events.status(&SessionActivityEvent {
        session_id: "a".into(),
        state: SessionActivityState::Idle,
        source: crate::session::state::StatusSource::Hook,
        status: crate::session::status::AgentStatus {
            observation: crate::session::status::AgentObservation {
                activity: crate::session::status::Activity::Ready,
                source: crate::session::status::ObservationSource::Hook,
                outcome: Some(crate::session::status::TurnOutcome::Completed),
                ..Default::default()
            },
            ..Default::default()
        },
    });
    let row = repo::node::get(&state.db.get().unwrap(), &tab.id)
        .unwrap()
        .unwrap();
    assert!(row.last_completed_at.is_some());
    assert_eq!(drain_attention_count(&mut rx), 1);
}

#[test]
fn completion_arm_is_consumed_after_recording() {
    let state = test_core();
    let tab = create_tab(&state, &["a"]);
    let mut rx = state.events.subscribe();
    let events = state.session_events();

    state.sessions.publish_direct_activity(
        "a",
        SessionActivityState::Busy,
        crate::session::state::StatusSource::Test,
        &events,
    );
    state.sessions.arm_completion("a");
    state.sessions.publish_direct_activity(
        "a",
        SessionActivityState::Idle,
        crate::session::state::StatusSource::Test,
        &events,
    );
    let first = repo::node::get(&state.db.get().unwrap(), &tab.id)
        .unwrap()
        .unwrap()
        .last_completed_at
        .expect("armed settle should record completion");

    state.sessions.publish_direct_activity(
        "a",
        SessionActivityState::Busy,
        crate::session::state::StatusSource::Test,
        &events,
    );
    state.sessions.publish_direct_activity(
        "a",
        SessionActivityState::Idle,
        crate::session::state::StatusSource::Test,
        &events,
    );

    let row = repo::node::get(&state.db.get().unwrap(), &tab.id)
        .unwrap()
        .unwrap();
    assert_eq!(row.last_completed_at.as_deref(), Some(first.as_str()));
    assert_eq!(drain_attention_count(&mut rx), 1);
}

#[test]
fn shell_activity_never_records_tab_completion() {
    let state = test_core();
    state
        .db
        .get()
        .unwrap()
        .execute(
            "INSERT INTO sessions
                    (id, status, started_at, agent_runtime, agent_command)
                 VALUES ('shell', 'running', ?1, 'shell', '/bin/zsh')",
            [Utc::now().to_rfc3339()],
        )
        .unwrap();
    let tab = create_tab(&state, &["shell"]);
    let events = state.session_events();
    state.sessions.arm_completion("shell");

    state.sessions.publish_direct_activity(
        "shell",
        SessionActivityState::Busy,
        crate::session::state::StatusSource::Test,
        &events,
    );
    state.sessions.publish_direct_activity(
        "shell",
        SessionActivityState::Idle,
        crate::session::state::StatusSource::Test,
        &events,
    );

    let row = repo::node::get(&state.db.get().unwrap(), &tab.id)
        .unwrap()
        .unwrap();
    assert!(row.last_completed_at.is_none());
    assert!(row.last_viewed_at.is_none());
}

#[test]
fn shell_peer_does_not_block_or_trigger_chat_completion() {
    let state = test_core();
    let conn = state.db.get().unwrap();
    let now = Utc::now().to_rfc3339();
    for (id, runtime, command) in [("chat", "codex", "codex"), ("shell", "shell", "/bin/zsh")] {
        conn.execute(
            "INSERT INTO sessions
                    (id, status, started_at, agent_runtime, agent_command)
                 VALUES (?1, 'running', ?2, ?3, ?4)",
            rusqlite::params![id, now, runtime, command],
        )
        .unwrap();
    }
    drop(conn);
    let tab = create_tab(&state, &["chat", "shell"]);
    let events = state.session_events();
    state.sessions.arm_completion("chat");

    state.sessions.publish_direct_activity(
        "chat",
        SessionActivityState::Busy,
        crate::session::state::StatusSource::Test,
        &events,
    );
    state.sessions.publish_direct_activity(
        "shell",
        SessionActivityState::Busy,
        crate::session::state::StatusSource::Test,
        &events,
    );
    state.sessions.publish_direct_activity(
        "shell",
        SessionActivityState::Idle,
        crate::session::state::StatusSource::Test,
        &events,
    );
    let before = repo::node::get(&state.db.get().unwrap(), &tab.id)
        .unwrap()
        .unwrap();
    assert!(before.last_completed_at.is_none());

    state.sessions.publish_direct_activity(
        "shell",
        SessionActivityState::Busy,
        crate::session::state::StatusSource::Test,
        &events,
    );
    state.sessions.publish_direct_activity(
        "chat",
        SessionActivityState::Idle,
        crate::session::state::StatusSource::Test,
        &events,
    );
    let after = repo::node::get(&state.db.get().unwrap(), &tab.id)
        .unwrap()
        .unwrap();
    assert!(after.last_completed_at.is_some());
}

#[test]
fn late_view_after_archiving_a_split_tab_is_a_noop() {
    let state = test_core();
    let tab = create_tab(&state, &["a", "b"]);
    let next = create_tab(&state, &["next"]);
    node_mark_viewed(&state, "main", &next.id, vec!["next".into()], Some("next"))
        .unwrap()
        .unwrap();
    {
        let conn = state.db.get().unwrap();
        conn.execute(
            "UPDATE sessions SET status = 'stopped' WHERE id IN ('a', 'b')",
            [],
        )
        .unwrap();
        repo::session_attention::record_completion(&conn, "b", false, 100).unwrap();
    }
    for id in ["a", "b"] {
        crate::ops::session::session_archive(&state, id).unwrap();
    }
    let mut events = state.events.subscribe();

    let result = node_mark_viewed(
        &state,
        "main",
        &tab.id,
        vec!["a".into(), "b".into()],
        Some("b"),
    );

    assert!(result.unwrap().is_none());
    assert_eq!(state.windows.focused_direct_sessions("main"), ["next"]);
    assert_eq!(drain_attention_count(&mut events), 0);
    let conn = state.db.get().unwrap();
    assert!(repo::node::get(&conn, &tab.id).unwrap().is_none());
    assert!(repo::session_attention::any_unread(&conn, &["b".into()]).unwrap());
    for id in ["a", "b"] {
        assert!(repo::session::get_row(&conn, id)
            .unwrap()
            .unwrap()
            .archived_at
            .is_some());
    }
}

#[test]
fn viewing_one_pane_preserves_persisted_sibling_unread() {
    let state = test_core();
    let tab = create_tab(&state, &["a", "b"]);
    let conn = state.db.get().unwrap();
    repo::session_attention::record_completion(&conn, "a", false, 100).unwrap();
    let completed = repo::node::record_completion(&conn, &tab.id, false, Utc::now())
        .unwrap()
        .unwrap();
    drop(conn);
    let viewed = node_mark_viewed(
        &state,
        "main",
        &tab.id,
        vec!["a".into(), "b".into()],
        Some("b"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(viewed.last_viewed_at, completed.last_viewed_at);
    mark_direct_sessions_viewed(&state, &["b".into()]).unwrap();
    let conn = state.db.get().unwrap();
    let viewed = repo::node::get(&conn, &tab.id).unwrap().unwrap();
    assert_eq!(viewed.last_viewed_at, completed.last_viewed_at);
    assert!(repo::session_attention::any_unread(&conn, &["a".into()]).unwrap());
    drop(conn);
    mark_direct_sessions_viewed(&state, &["a".into()]).unwrap();
    let conn = state.db.get().unwrap();
    assert!(!repo::session_attention::any_unread(&conn, &["a".into()]).unwrap());
    assert!(repo::node::get(&conn, &tab.id)
        .unwrap()
        .unwrap()
        .last_viewed_at
        .is_some());
}

#[test]
fn split_pane_focus_preserves_sibling_completion_and_error_attention() {
    let state = test_core();
    let tab = create_tab(&state, &["a", "b"]);
    let subjects = vec![
        Subject::DirectChat("a".into()),
        Subject::DirectChat("b".into()),
    ];
    crate::ops::window::report_subjects(&state, "main", subjects.clone(), Some("b")).unwrap();
    crate::ops::window::mark_focused(&state, "main").unwrap();
    let events = state.session_events();
    state.sessions.publish_direct_activity(
        "a",
        SessionActivityState::Busy,
        crate::session::state::StatusSource::Test,
        &events,
    );
    state.sessions.arm_completion("a");
    state.sessions.publish_direct_activity(
        "a",
        SessionActivityState::Idle,
        crate::session::state::StatusSource::Test,
        &events,
    );
    let conn = state.db.get().unwrap();
    conn.execute(
        "UPDATE sessions SET status = 'crashed', stopped_at = ?1 WHERE id = 'a'",
        [Utc::now().to_rfc3339()],
    )
    .unwrap();
    drop(conn);

    node_mark_viewed(
        &state,
        "main",
        &tab.id,
        vec!["a".into(), "b".into()],
        Some("b"),
    )
    .unwrap()
    .unwrap();
    crate::ops::window::report_subjects(&state, "main", subjects.clone(), Some("b")).unwrap();
    crate::ops::window::mark_focused(&state, "main").unwrap();
    let statuses = crate::ops::session::session_status_snapshot(&state).unwrap();
    assert!(statuses["a"].unread_since.is_some());
    assert!(statuses["a"].error_since.is_some());
    assert_eq!(
        state.windows.primary_for(&Subject::DirectChat("a".into())),
        Some("main".into())
    );

    crate::ops::window::report_subjects(&state, "main", subjects.clone(), None).unwrap();
    assert!(
        crate::ops::session::session_status_snapshot(&state).unwrap()["a"]
            .unread_since
            .is_some()
    );
    crate::ops::window::report_subjects(&state, "main", subjects, Some("a")).unwrap();
    let statuses = crate::ops::session::session_status_snapshot(&state).unwrap();
    assert!(statuses["a"].unread_since.is_none());
    assert!(statuses["a"].error_since.is_none());
    assert_eq!(
        statuses["a"].lifecycle,
        crate::session::status::Lifecycle::Error
    );
}

#[test]
fn activation_and_focus_return_advance_viewed_and_emit_invalidation() {
    let state = test_core();
    let tab = create_tab(&state, &["a"]);
    let mut rx = state.events.subscribe();
    let first_completion = Utc::now();
    repo::node::record_completion(&state.db.get().unwrap(), &tab.id, false, first_completion)
        .unwrap();

    let activated = node_mark_viewed(&state, "main", &tab.id, vec!["a".to_string()], Some("a"))
        .unwrap()
        .unwrap();
    assert!(
        parsed(activated.last_viewed_at.as_deref())
            >= parsed(activated.last_completed_at.as_deref())
    );
    assert_eq!(
        state.windows.focused_direct_sessions("main"),
        ["a".to_string()]
    );
    assert_eq!(drain_attention_count(&mut rx), 1);

    state.windows.mark_blurred("main");
    let second_completion = first_completion + chrono::Duration::seconds(1);
    repo::node::record_completion(&state.db.get().unwrap(), &tab.id, false, second_completion)
        .unwrap();
    state.windows.mark_focused("main");
    let visible = state.windows.focused_direct_sessions("main");
    mark_direct_sessions_viewed(&state, &visible).unwrap();

    let focused = repo::node::get(&state.db.get().unwrap(), &tab.id)
        .unwrap()
        .unwrap();
    assert!(
        parsed(focused.last_viewed_at.as_deref()) >= parsed(focused.last_completed_at.as_deref())
    );
    assert_eq!(drain_attention_count(&mut rx), 1);
}
