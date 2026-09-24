use std::sync::Arc;

use crate::db;
use crate::test_support::{insert_test_role, insert_test_slot, row_count};

/// Every delete path, run through its app entry point on a real
/// database with the pragma on, leaves no dangling reference behind.
/// The foreign-key list is pinned too: a new foreign key fails here
/// until the owning delete in `ops` handles it.
#[test]
fn every_delete_leaves_foreign_key_check_empty_on_a_real_database() {
    let tmp = tempfile::tempdir().unwrap();
    let mut core = crate::test_support::test_core_in(tmp.path().to_path_buf());
    core.db = Arc::new(db::open_pool(&tmp.path().join("runner.db")).unwrap());
    let conn = core.db.get().unwrap();

    let foreign_keys: Vec<(String, String, String, String)> = conn
        .prepare(
            "SELECT m.name, p.\"from\", p.\"table\", p.on_delete
               FROM sqlite_master m JOIN pragma_foreign_key_list(m.name) p
              WHERE m.type = 'table'
              ORDER BY 1, 2",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let expected = [
        ("missions", "crew_id", "crews", "CASCADE"),
        ("missions", "project_id", "projects", "SET NULL"),
        ("nodes", "parent_id", "nodes", "RESTRICT"),
        ("session_attention", "session_id", "sessions", "CASCADE"),
        ("sessions", "mission_id", "missions", "SET NULL"),
        ("sessions", "project_id", "projects", "SET NULL"),
        ("sessions", "role_id", "roles", "CASCADE"),
        ("slots", "crew_id", "crews", "CASCADE"),
        ("slots", "role_id", "roles", "CASCADE"),
    ]
    .map(|(a, b, c, d)| (a.to_owned(), b.to_owned(), c.to_owned(), d.to_owned()));
    assert_eq!(foreign_keys, expected);
    assert_eq!(
        row_count(&conn, "SELECT foreign_keys FROM pragma_foreign_keys"),
        1
    );

    insert_test_role(&conn, "r-doomed", "doomed", "shell", "sh");
    insert_test_role(&conn, "r-kept", "kept", "shell", "sh");
    let project = crate::repo::project::create(&conn, "P", "/tmp/p").unwrap();
    crate::repo::node::ensure_project_node(&conn, &project.id).unwrap();
    conn.execute_batch(&format!(
        "INSERT INTO crews (id, name, created_at, updated_at)
         VALUES ('c-doomed', 'Doomed', '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z'),
                ('c-kept', 'Kept', '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z');
         INSERT INTO missions
             (id, crew_id, title, status, started_at, stopped_at, archived_at, project_id)
         VALUES ('m-deleted', 'c-kept', 'M', 'completed', '2026-07-01T00:00:00Z',
                 '2026-07-01T01:00:00Z', '2026-07-01T01:00:00Z', NULL),
                ('m-of-crew', 'c-doomed', 'M', 'completed', '2026-07-01T00:00:00Z',
                 '2026-07-01T01:00:00Z', '2026-07-01T01:00:00Z', '{p}'),
                ('m-kept', 'c-kept', 'M', 'completed', '2026-07-01T00:00:00Z',
                 '2026-07-01T01:00:00Z', '2026-07-01T01:00:00Z', '{p}');
         INSERT INTO sessions
             (id, mission_id, role_id, slot_id, status, archived_at, project_id,
              agent_runtime, agent_command)
         VALUES ('m-deleted-lead', 'm-deleted', 'r-kept', 'x', 'stopped', NULL, NULL,
                 NULL, NULL),
                ('m-of-crew-lead', 'm-of-crew', 'r-kept', 'x', 'stopped', NULL, '{p}',
                 NULL, NULL),
                ('m-kept-lead', 'm-kept', 'r-doomed', 'x', 'stopped', NULL, NULL,
                 NULL, NULL),
                ('chat-deleted', NULL, 'r-kept', NULL, 'stopped',
                 '2026-07-01T01:00:00Z', NULL, NULL, NULL),
                ('chat-of-role', NULL, 'r-doomed', NULL, 'stopped',
                 '2026-07-01T01:00:00Z', NULL, NULL, NULL),
                ('chat-of-project', NULL, 'r-kept', NULL, 'stopped',
                 '2026-07-01T01:00:00Z', '{p}', NULL, NULL),
                ('terminal', NULL, NULL, NULL, 'stopped', NULL, NULL, 'shell', '/bin/zsh');",
        p = project.id
    ))
    .unwrap();
    for (id, crew, role, handle, position, lead) in [
        ("s-doomed-crew", "c-doomed", "r-kept", "lead", 0, true),
        ("s-doomed-role", "c-kept", "r-doomed", "lead", 0, true),
        ("s-kept", "c-kept", "r-kept", "kept", 1, false),
    ] {
        insert_test_slot(&conn, id, crew, role, handle, position, lead);
    }
    for session in [
        "m-deleted-lead",
        "m-of-crew-lead",
        "m-kept-lead",
        "chat-deleted",
        "chat-of-role",
        "chat-of-project",
        "terminal",
    ] {
        crate::repo::session_attention::record_completion(&conn, session, false, 100).unwrap();
    }
    crate::repo::node::create_tab(
        &conn,
        None,
        "",
        crate::repo::node::next_position(&conn, None).unwrap(),
        r#"{"preset":"single","slots":["terminal"],"sizes":{}}"#,
    )
    .unwrap();
    drop(conn);

    let violations = || {
        let conn = core.db.get().unwrap();
        row_count(&conn, "SELECT COUNT(*) FROM pragma_foreign_key_check")
    };
    assert_eq!(violations(), 0, "seeded graph");
    let check = |entity: &str, deleted: crate::error::Result<()>| {
        deleted.unwrap_or_else(|error| panic!("{entity} delete failed: {error}"));
        assert_eq!(violations(), 0, "after the {entity} delete");
    };
    check(
        "session",
        crate::ops::session::session_delete(&core, "chat-deleted"),
    );
    check(
        "shell session",
        crate::ops::session::session_close(&core, "terminal"),
    );
    check(
        "slot",
        crate::ops::slot::slot_delete(&core, "s-doomed-role"),
    );
    check(
        "mission",
        crate::ops::mission::mission_delete(&core, "m-deleted"),
    );
    let project_deleted = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(crate::ops::project::project_delete(
            &core,
            project.id.clone(),
        ));
    check("project", project_deleted.map(|_| ()));
    check("crew", crate::ops::crew::crew_delete(&core, "c-doomed"));
    check("role", crate::ops::role::role_delete(&core, "r-doomed"));

    let conn = core.db.get().unwrap();
    let survivors: Vec<String> = conn
        .prepare("SELECT id FROM sessions ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(survivors, ["chat-of-project"]);
    assert_eq!(
        row_count(&conn, "SELECT COUNT(*) FROM session_attention"),
        1
    );
    assert_eq!(
        row_count(
            &conn,
            "SELECT COUNT(*) FROM missions WHERE project_id IS NOT NULL"
        ),
        0
    );
}
