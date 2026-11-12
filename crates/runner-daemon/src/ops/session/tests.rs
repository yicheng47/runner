use super::*;
use crate::db;
use chrono::Utc;
use rusqlite::params;

#[test]
fn live_counts_group_direct_mission_and_fork_sessions_by_runtime() {
    let conn = db::open_in_memory().unwrap().get().unwrap();
    crate::test_support::insert_test_role(&conn, "r-codex", "coder", "codex", "codex");
    conn.execute(
        "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES ('c1', 'Crew', '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO missions (id, crew_id, title, status, started_at)
             VALUES ('m1', 'c1', 'M', 'running', '2026-07-01T00:00:00Z')",
        [],
    )
    .unwrap();
    let insert = |id: &str, runtime: Option<&str>, role: Option<&str>, mission: Option<&str>| {
        let mut row = crate::test_support::test_session_row(id, SessionStatus::Running);
        row.agent_runtime = runtime.map(str::to_owned);
        row.role_id = role.map(str::to_owned);
        row.mission_id = mission.map(str::to_owned);
        crate::test_support::insert_session_row(&conn, &row).unwrap();
    };
    insert("direct", Some("codex"), None, None);
    insert("slot", None, Some("r-codex"), Some("m1"));
    insert("fork", Some("codex"), Some("r-codex"), None);
    insert("claude", Some("claude-code"), None, None);
    insert("drawer", Some("shell"), None, None);
    insert("stopped", Some("codex"), None, None);
    let live = ["direct", "slot", "fork", "claude", "drawer", "agent-update"].map(String::from);
    let counts = count_by_runtime(&conn, &live).unwrap();
    assert_eq!(counts.get(&Runtime::Codex), Some(&3));
    assert_eq!(counts.get(&Runtime::ClaudeCode), Some(&1));
    assert_eq!(counts.get(&Runtime::Shell), Some(&1));
    assert_eq!(counts.get(&Runtime::Copilot), None);
    assert_eq!(counts.values().sum::<usize>(), 5);
}

#[cfg(unix)]
#[test]
fn shell_command_uses_environment_value_with_zsh_fallback() {
    assert_eq!(
        resolve_shell_command(Some("/bin/fish".into())).unwrap(),
        "/bin/fish"
    );
    #[cfg(target_os = "linux")]
    let fallback = crate::shell_path::account_login_shell().unwrap();
    #[cfg(not(target_os = "linux"))]
    let fallback = "/bin/zsh";
    assert_eq!(resolve_shell_command(None).unwrap(), fallback);
    assert_eq!(
        resolve_shell_command(Some("  ".into())).unwrap(),
        resolve_shell_command(None).unwrap()
    );
}

#[cfg(windows)]
#[test]
fn shell_command_prefers_pwsh_then_powershell_then_comspec() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    let comspec = r"C:\Windows\System32\cmd.exe";
    assert_eq!(windows_shell_command(path, comspec.into()), comspec);
    let powershell = dir.path().join("powershell.exe");
    std::fs::write(&powershell, "").unwrap();
    assert_eq!(
        windows_shell_command(path, comspec.into()),
        powershell.to_str().unwrap()
    );
    let pwsh = dir.path().join("pwsh.exe");
    std::fs::write(&pwsh, "").unwrap();
    assert_eq!(
        windows_shell_command(path, comspec.into()),
        pwsh.to_str().unwrap()
    );
    assert_eq!(
        resolve_shell_command(Some("C:/Git/bin/bash.exe".into())).unwrap(),
        "C:/Git/bin/bash.exe"
    );
}

#[test]
fn close_shell_row_removes_pane_slot_and_session_together() {
    let pool = db::open_in_memory().unwrap();
    let mut conn = pool.get().unwrap();
    let now = Utc::now().to_rfc3339();
    for (id, runtime) in [("shell", "shell"), ("chat", "codex")] {
        conn.execute(
            "INSERT INTO sessions
                    (id, status, started_at, agent_runtime, agent_command)
                 VALUES (?1, 'stopped', ?2, ?3, '/bin/sh')",
            params![id, now, runtime],
        )
        .unwrap();
    }
    let tab = repo::node::create_tab(
        &conn,
        None,
        "split",
        0,
        &serde_json::json!({
            "preset": "cols-2",
            "slots": ["shell", "chat"],
            "sizes": {},
        })
        .to_string(),
    )
    .unwrap();

    close_shell_row(&mut conn, "shell").unwrap();

    assert!(repo::session::get_row(&conn, "shell").unwrap().is_none());
    assert!(repo::session::get_row(&conn, "chat").unwrap().is_some());
    let layout: serde_json::Value = serde_json::from_str(
        repo::node::get(&conn, &tab.id)
            .unwrap()
            .unwrap()
            .layout
            .as_deref()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(layout["slots"], serde_json::json!(["chat"]));
    assert_eq!(layout["tree"]["kind"], "leaf");
    assert_eq!(layout["tree"]["id"], "p2");
}

#[test]
fn close_shell_row_rejects_chat_without_mutation() {
    let pool = db::open_in_memory().unwrap();
    let mut conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions
                (id, status, started_at, agent_runtime, agent_command)
             VALUES ('chat', 'stopped', ?1, 'codex', 'codex')",
        params![Utc::now().to_rfc3339()],
    )
    .unwrap();
    let tab = repo::node::create_tab(
        &conn,
        None,
        "chat",
        0,
        &serde_json::json!({
            "preset": "single",
            "slots": ["chat"],
            "sizes": {},
        })
        .to_string(),
    )
    .unwrap();

    let error = close_shell_row(&mut conn, "chat").unwrap_err();

    assert!(error.to_string().contains("not a shell session"));
    assert!(repo::session::get_row(&conn, "chat").unwrap().is_some());
    assert!(repo::node::get(&conn, &tab.id).unwrap().is_some());
}

#[test]
fn terminal_sessions_cannot_be_archived() {
    let pool = db::open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO sessions
                (id, status, started_at, agent_runtime, agent_command)
             VALUES ('shell', 'stopped', ?1, 'shell', '/bin/zsh')",
        params![Utc::now().to_rfc3339()],
    )
    .unwrap();

    let error = ensure_archivable_session(&conn, "shell").unwrap_err();

    assert!(error
        .to_string()
        .contains("terminal sessions cannot be archived"));
    assert!(repo::session::get_row(&conn, "shell").unwrap().is_some());
}

/// Mirrors the SELECT in `session_list_recent_direct` so we can
/// exercise the ORDER BY without constructing `AppCore`. Returns
/// (session_id, status, pinned) in the order the tray will render.
fn list_recent_direct(conn: &rusqlite::Connection) -> Vec<(String, SessionStatus, bool)> {
    repo::session::list_recent_direct(conn)
        .unwrap()
        .into_iter()
        .map(|row| {
            let pinned = row.row.pinned_at.is_some();
            (row.row.id, row.row.status, pinned)
        })
        .collect()
}

#[test]
fn pinned_sessions_sort_above_running_and_stopped() {
    let pool = db::open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    let now = Utc::now();
    let role_id = ulid::Ulid::new().to_string();
    crate::test_support::insert_test_role(&conn, &role_id, "r", "shell", "/bin/sh");

    let insert =
        |id: &str, status: &str, started_offset_secs: i64, pinned: bool, archived: bool| {
            let status = match status {
                "running" => SessionStatus::Running,
                "stopped" => SessionStatus::Stopped,
                _ => unreachable!(),
            };
            let mut row = crate::test_support::test_session_row(id, status);
            row.role_id = Some(role_id.clone());
            row.started_at = Some(now - chrono::Duration::seconds(started_offset_secs));
            row.pinned_at = pinned.then_some(now);
            row.archived_at = archived.then_some(now);
            crate::test_support::insert_session_row(&conn, &row).unwrap();
        };

    // newest running, but not pinned
    insert("running-new", "running", 60, false, false);
    // older running, but pinned
    insert("running-pinned", "running", 600, true, false);
    // very old stopped, pinned
    insert("stopped-pinned-old", "stopped", 3600, true, false);
    // recent stopped, not pinned
    insert("stopped-recent", "stopped", 120, false, false);
    // archived: must not appear at all, even if pinned
    insert("pinned-archived", "stopped", 30, true, true);

    let rows = list_recent_direct(&conn);
    let ids: Vec<&str> = rows.iter().map(|(id, _, _)| id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            // pinned bucket sorted by recency: pinned-running (600s
            // old) is more recent than pinned-stopped (3600s old).
            "running-pinned",
            "stopped-pinned-old",
            // unpinned bucket: running before stopped, then recency.
            "running-new",
            "stopped-recent",
        ]
    );
    // Sanity: archived row never makes the list, regardless of pin.
    assert!(
        !ids.contains(&"pinned-archived"),
        "archived rows must be excluded from the SESSION tray"
    );
}

fn seed_role(conn: &rusqlite::Connection) -> String {
    let role_id = ulid::Ulid::new().to_string();
    crate::test_support::insert_test_role(conn, &role_id, "r", "shell", "/bin/sh");
    role_id
}

#[test]
fn resolve_direct_start_defaults_cwd_from_project() {
    let pool = db::open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    let role_id = seed_role(&conn);
    let project = repo::project::create(&conn, "Runner", "/project").unwrap();

    let (role, project_id, cwd) =
        resolve_direct_start(&conn, &role_id, Some(&project.id), None).unwrap();

    assert_eq!(role.id, role_id);
    assert_eq!(project_id.as_deref(), Some(project.id.as_str()));
    assert_eq!(cwd.as_deref(), Some("/project"));
}

#[test]
fn resolve_direct_start_explicit_cwd_overrides_project() {
    let pool = db::open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    let role_id = seed_role(&conn);
    let project = repo::project::create(&conn, "Runner", "/project").unwrap();

    let (_, project_id, cwd) =
        resolve_direct_start(&conn, &role_id, Some(&project.id), Some("/override".into())).unwrap();

    assert_eq!(project_id.as_deref(), Some(project.id.as_str()));
    assert_eq!(cwd.as_deref(), Some("/override"));
}

#[test]
fn resolve_direct_start_explicit_project_beats_role_working_dir() {
    let pool = db::open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    let role_id = seed_role(&conn);
    conn.execute(
        "UPDATE roles SET working_dir = '/role' WHERE id = ?1",
        [&role_id],
    )
    .unwrap();
    let project = repo::project::create(&conn, "Runner", "/project").unwrap();

    let (_, project_id, cwd) =
        resolve_direct_start(&conn, &role_id, Some(&project.id), None).unwrap();

    assert_eq!(project_id.as_deref(), Some(project.id.as_str()));
    assert_eq!(cwd.as_deref(), Some("/project"));
}

#[test]
fn resolve_direct_start_unknown_project_creates_no_session() {
    let pool = db::open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    let role_id = seed_role(&conn);

    let error = resolve_direct_start(&conn, &role_id, Some("missing"), None).unwrap_err();

    assert_eq!(error.to_string(), "project not found: missing");
    let session_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(session_count, 0);
}

#[test]
fn resolve_direct_start_in_root_keeps_the_role_working_dir_without_a_project() {
    let temp = tempfile::tempdir().unwrap();
    let project_cwd = temp.path().join("runner");
    let role_cwd = project_cwd.join("crates");
    std::fs::create_dir_all(&role_cwd).unwrap();
    let role_cwd = role_cwd.to_string_lossy().into_owned();
    let pool = db::open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    let role_id = seed_role(&conn);
    conn.execute(
        "UPDATE roles SET working_dir = ?2 WHERE id = ?1",
        rusqlite::params![role_id, role_cwd],
    )
    .unwrap();
    let project =
        repo::project::create(&conn, "Runner", project_cwd.to_string_lossy().as_ref()).unwrap();

    let (_, project_id, cwd) =
        resolve_direct_start_in(&conn, &role_id, &ProjectScope::Root, None).unwrap();
    assert_eq!(project_id, None);
    assert_eq!(cwd.as_deref(), Some(role_cwd.as_str()));

    let (_, project_id, cwd) = resolve_direct_start(&conn, &role_id, None, None).unwrap();
    assert_eq!(project_id.as_deref(), Some(project.id.as_str()));
    assert_eq!(cwd.as_deref(), Some(role_cwd.as_str()));
}

fn insert_direct_session(conn: &rusqlite::Connection, role_id: &str, archived: bool) -> String {
    let id = ulid::Ulid::new().to_string();
    let now = Utc::now();
    let mut row = crate::test_support::test_session_row(&id, SessionStatus::Stopped);
    row.role_id = Some(role_id.into());
    row.started_at = Some(now);
    row.archived_at = archived.then_some(now);
    crate::test_support::insert_session_row(conn, &row).unwrap();
    id
}

#[test]
fn session_delete_removes_an_antigravity_log_only_with_its_row() {
    let app_data = tempfile::tempdir().unwrap();
    let state = crate::test_support::test_core_in(app_data.path().to_path_buf());
    let conn = state.db.get().unwrap();
    let role_id = seed_role(&conn);
    let archived = insert_direct_session(&conn, &role_id, true);
    let active = insert_direct_session(&conn, &role_id, false);
    drop(conn);
    crate::runtimes::antigravity::agy_capture::prepare_log(app_data.path(), &archived);
    let log = crate::runtimes::antigravity::agy_capture::log_path(app_data.path(), &archived);
    std::fs::write(&log, "log").unwrap();
    crate::runtimes::antigravity::agy_capture::prepare_log(app_data.path(), &active);
    let active_log = crate::runtimes::antigravity::agy_capture::log_path(app_data.path(), &active);
    std::fs::write(&active_log, "log").unwrap();

    assert!(session_delete(&state, &active).is_err());
    assert!(active_log.exists());
    session_delete(&state, &archived).unwrap();
    assert!(!log.exists());
}

#[test]
fn session_delete_only_deletes_archived_direct_rows() {
    let state = crate::test_support::test_core();
    let (active, archived, slot_bound) = {
        let conn = state.db.get().unwrap();
        let role_id = seed_role(&conn);
        let active = insert_direct_session(&conn, &role_id, false);
        let archived = insert_direct_session(&conn, &role_id, true);
        // Archived slot-bound row: must survive this path — it dies
        // with its mission, not through chat delete.
        let slot_bound = ulid::Ulid::new().to_string();
        let mut row = crate::test_support::test_session_row(&slot_bound, SessionStatus::Stopped);
        row.role_id = Some(role_id.clone());
        row.slot_id = Some("slot-1".into());
        row.archived_at = row.started_at;
        crate::test_support::insert_session_row(&conn, &row).unwrap();
        (active, archived, slot_bound)
    };

    let refused = session_delete(&state, &active).unwrap_err().to_string();
    assert!(
        refused.contains("not archived"),
        "active chats must be archived before deletion: {refused}"
    );
    let refused = session_delete(&state, &slot_bound).unwrap_err().to_string();
    assert!(
        refused.contains("mission-scoped"),
        "slot-bound rows must not be deletable as chats: {refused}"
    );
    session_delete(&state, &archived).unwrap();

    let conn = state.db.get().unwrap();
    let remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(remaining, 2, "only the archived direct row is gone");
}

#[test]
fn session_delete_removes_attention_with_foreign_keys_off() {
    use crate::test_support::{foreign_keys_off, row_count};
    let state = crate::test_support::test_core();
    {
        let conn = state.db.get().unwrap();
        foreign_keys_off(&conn);
        conn.execute_batch(
            "INSERT INTO sessions (id, status, archived_at)
                 VALUES ('archived', 'stopped', '2026-07-01T00:00:00Z'),
                        ('active', 'stopped', NULL);",
        )
        .unwrap();
        for id in ["archived", "active"] {
            repo::session_attention::record_completion(&conn, id, false, 100).unwrap();
        }
    }

    let refused = session_delete(&state, "active").unwrap_err().to_string();
    assert!(refused.contains("not archived"), "{refused}");
    session_delete(&state, "archived").unwrap();

    let conn = state.db.get().unwrap();
    assert_eq!(
        row_count(&conn, "SELECT foreign_keys FROM pragma_foreign_keys"),
        0
    );
    let left: Vec<(String, i64)> = conn
        .prepare(
            "SELECT s.id, (SELECT COUNT(*) FROM session_attention a WHERE a.session_id = s.id)
                   FROM sessions s",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(left, [("active".to_string(), 1)]);
    assert_eq!(
        row_count(&conn, "SELECT COUNT(*) FROM session_attention"),
        1
    );
}

#[test]
fn drawer_upgrade_closes_only_chat_drawer_shells_and_drops_the_field() {
    let state = crate::test_support::test_core();
    let mut conn = state.db.get().unwrap();
    conn.execute_batch(
        "INSERT INTO sessions (id, status, agent_runtime, agent_command, resume_on_launch)
            VALUES ('old-shell', 'stopped', 'shell', 'zsh', 1),
                   ('mission-shell', 'stopped', 'shell', 'zsh', 1),
                   ('chat', 'stopped', 'codex', 'codex', 0);",
    )
    .unwrap();
    let chat = repo::node::create_tab(
        &conn,
        None,
        "Named chat",
        0,
        &serde_json::json!({"slots":["chat"], "drawer":{"open":true,"shells":["old-shell"]}})
            .to_string(),
    )
    .unwrap();
    let mut mission = chat.clone();
    mission.id = "mission-node".into();
    mission.node_type = repo::node::NodeType::Mission;
    mission.layout =
        Some(serde_json::json!({"drawer":{"open":true,"shells":["mission-shell"]}}).to_string());
    repo::node::upsert(&conn, &mission).unwrap();
    retire_chat_drawers(&mut conn).unwrap();
    retire_chat_drawers(&mut conn).unwrap();
    assert!(repo::session::get_row(&conn, "old-shell")
        .unwrap()
        .is_none());
    assert!(repo::session::get_row(&conn, "mission-shell")
        .unwrap()
        .is_some());
    assert!(repo::session::get_row(&conn, "chat").unwrap().is_some());
    let upgraded = repo::node::get(&conn, &chat.id).unwrap().unwrap();
    assert_eq!(upgraded.name, chat.name);
    let layout: serde_json::Value =
        serde_json::from_str(upgraded.layout.as_deref().unwrap()).unwrap();
    assert!(layout.get("drawer").is_none());
    assert_eq!(layout["slots"], serde_json::json!(["chat"]));
    assert_eq!(
        repo::node::get(&conn, &mission.id).unwrap().unwrap().layout,
        mission.layout
    );
}

#[test]
fn fast_shell_exit_during_launch_resume_closes_after_releasing_the_claim() {
    let state = crate::test_support::test_core();
    let mut conn = state.db.get().unwrap();
    conn.execute_batch(
        "INSERT INTO sessions (id, status, agent_runtime, agent_command, resume_on_launch)
            VALUES ('fast-exit', 'stopped', 'shell', 'zsh', 2),
                   ('still-queued', 'stopped', 'shell', 'zsh', 1),
                   ('chat-exit', 'stopped', 'codex', 'codex', 2);",
    )
    .unwrap();
    for id in ["fast-exit", "still-queued", "chat-exit"] {
        finish_launch_resume(&mut conn, id).unwrap();
    }
    assert!(repo::session::get_row(&conn, "fast-exit")
        .unwrap()
        .is_none());
    assert!(repo::session::get_row(&conn, "still-queued")
        .unwrap()
        .is_some());
    assert!(repo::session::get_row(&conn, "chat-exit")
        .unwrap()
        .is_some());
}

#[test]
fn exited_shell_cleanup_preserves_planned_resume_and_chats() {
    let state = crate::test_support::test_core();
    let mut conn = state.db.get().unwrap();
    conn.execute_batch(
        "INSERT INTO sessions (id, status, agent_runtime, agent_command, resume_on_launch)
            VALUES ('stopped', 'stopped', 'shell', 'zsh', 0),
                   ('crashed', 'crashed', 'shell', 'zsh', 0),
                   ('running', 'running', 'shell', 'zsh', 0),
                   ('planned', 'stopped', 'shell', 'zsh', 1),
                   ('claim', 'stopped', 'shell', 'zsh', 2),
                   ('chat', 'stopped', 'codex', 'codex', 0);",
    )
    .unwrap();
    for id in ["stopped", "crashed", "running", "planned", "claim", "chat"] {
        repo::node::create_tab(
            &conn,
            None,
            "",
            0,
            &serde_json::json!({"preset":"single", "slots":[id]}).to_string(),
        )
        .unwrap();
    }
    close_exited_shells(&mut conn).unwrap();
    close_exited_shells(&mut conn).unwrap();
    for id in ["stopped", "crashed"] {
        assert!(repo::session::get_row(&conn, id).unwrap().is_none());
    }
    let mut ids = repo::node::list(&conn)
        .unwrap()
        .iter()
        .flat_map(repo::node::session_ids)
        .collect::<Vec<_>>();
    ids.sort();
    assert_eq!(ids, ["chat", "claim", "planned", "running"]);
}

#[test]
fn session_close_removes_attention_and_tab_with_foreign_keys_off() {
    use crate::test_support::{foreign_keys_off, row_count};
    let state = crate::test_support::test_core();
    {
        let conn = state.db.get().unwrap();
        foreign_keys_off(&conn);
        conn.execute_batch(
            "INSERT INTO sessions (id, status, agent_runtime, agent_command)
                 VALUES ('terminal', 'stopped', 'shell', '/bin/zsh'),
                        ('other', 'stopped', 'shell', '/bin/zsh');",
        )
        .unwrap();
        for id in ["terminal", "other"] {
            repo::session_attention::record_completion(&conn, id, false, 100).unwrap();
            repo::node::create_tab(
                &conn,
                None,
                "",
                repo::node::next_position(&conn, None).unwrap(),
                &format!(r#"{{"preset":"single","slots":["{id}"],"sizes":{{}}}}"#),
            )
            .unwrap();
        }
    }

    session_close(&state, "terminal").unwrap();

    let conn = state.db.get().unwrap();
    assert_eq!(
        row_count(&conn, "SELECT foreign_keys FROM pragma_foreign_keys"),
        0
    );
    assert_eq!(
        row_count(&conn, "SELECT COUNT(*) FROM sessions WHERE id = 'terminal'"),
        0
    );
    assert_eq!(
        row_count(
            &conn,
            "SELECT COUNT(*) FROM session_attention WHERE session_id = 'terminal'"
        ),
        0
    );
    assert_eq!(
        row_count(&conn, "SELECT COUNT(*) FROM session_attention"),
        1
    );
    let tabs: Vec<Vec<String>> = repo::node::list(&conn)
        .unwrap()
        .iter()
        .map(repo::node::session_ids)
        .collect();
    assert_eq!(tabs, [vec!["other".to_string()]]);
}

#[test]
fn session_get_returns_archived_row() {
    // Whole reason this command exists: listRecentDirect filters
    // archived rows, so RoleChat needs an unfiltered fallback to
    // detect an archived direct-URL navigation and render
    // read-only. A future refactor that adds `archived_at IS NULL`
    // to get_direct's WHERE breaks the chat-page lockdown — this
    // test fails if it ever does.
    let pool = db::open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    let role_id = seed_role(&conn);
    let session_id = insert_direct_session(&conn, &role_id, /*archived*/ true);

    let row = get_direct(&conn, &session_id).unwrap();
    let row = row.expect("archived row must be returned");
    assert_eq!(row.session_id, session_id);
    assert!(
        row.archived_at.is_some(),
        "archived_at must be populated for archived rows"
    );
}

#[test]
fn session_get_populates_archived_at_when_set() {
    // Belt-and-suspenders for the column round-trip: archived
    // direct sessions returned by get_direct must carry the
    // timestamp the row was archived with, not a recoded `now()`.
    let pool = db::open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    let role_id = seed_role(&conn);
    let id = ulid::Ulid::new().to_string();
    let archived_at = "2025-01-01T00:00:00+00:00";
    let mut session = crate::test_support::test_session_row(&id, SessionStatus::Stopped);
    session.role_id = Some(role_id);
    session.archived_at = Some(archived_at.parse().unwrap());
    crate::test_support::insert_session_row(&conn, &session).unwrap();

    let row = get_direct(&conn, &id).unwrap().unwrap();
    let got = row.archived_at.expect("archived_at populated").to_rfc3339();
    assert_eq!(got, archived_at);
}

#[test]
fn session_get_returns_agent_session_key_for_direct_chat() {
    let pool = db::open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    let role_id = seed_role(&conn);
    let id = ulid::Ulid::new().to_string();
    let key = uuid::Uuid::new_v4().to_string();
    let mut session = crate::test_support::test_session_row(&id, SessionStatus::Stopped);
    session.role_id = Some(role_id);
    session.agent_session_key = Some(key.clone());
    crate::test_support::insert_session_row(&conn, &session).unwrap();

    let row = get_direct(&conn, &id).unwrap().unwrap();
    assert_eq!(row.agent_session_key.as_deref(), Some(key.as_str()));
    assert!(row.resumable);
}

#[test]
fn session_get_runtime_chat_model_and_effort() {
    let pool = db::open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    for (id, model, effort, expected_model, expected_effort) in [
        (
            "set",
            Some(" gpt-5.6-sol "),
            Some(" xhigh "),
            Some("gpt-5.6-sol"),
            Some("xhigh"),
        ),
        ("unset", None, None, None, None),
        ("blank", Some("  "), Some("\t"), None, None),
    ] {
        let mut session = crate::test_support::test_session_row(id, SessionStatus::Stopped);
        session.agent_runtime = Some("codex".into());
        session.agent_command = Some("codex".into());
        session.launch_definition = Some(crate::test_support::session_definition("codex", "codex"));
        session.launch_definition.as_mut().unwrap().model = model
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned);
        session.launch_definition.as_mut().unwrap().effort = effort
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned);
        crate::test_support::insert_session_row(&conn, &session).unwrap();
        let entry = get_direct(&conn, id).unwrap().unwrap();
        assert_eq!(entry.agent_model.as_deref(), expected_model);
        assert_eq!(entry.agent_effort.as_deref(), expected_effort);
    }
    for row in repo::session::list_recent_direct(&conn).unwrap() {
        let entry = direct_entry_from_repo(row, false).unwrap();
        let detail = get_direct(&conn, &entry.session_id).unwrap().unwrap();
        assert_eq!(entry.agent_model, detail.agent_model);
        assert_eq!(entry.agent_effort, detail.agent_effort);
    }
}

#[test]
fn session_get_role_chat_keeps_recorded_settings() {
    let pool = db::open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    let role_id = seed_role(&conn);
    conn.execute(
        "UPDATE roles SET model = 'fable', effort = 'high' WHERE id = ?1",
        [&role_id],
    )
    .unwrap();
    let id = insert_direct_session(&conn, &role_id, false);
    conn.execute(
        "UPDATE roles SET model = 'sonnet', effort = 'xhigh', command = 'changed' WHERE id = ?1",
        [&role_id],
    )
    .unwrap();
    let entry = get_direct(&conn, &id).unwrap().unwrap();
    assert_eq!(entry.agent_model.as_deref(), Some("fable"));
    assert_eq!(entry.agent_effort.as_deref(), Some("high"));
    assert_eq!(entry.agent_command, "/bin/sh");
}

#[test]
fn session_get_role_chat_model_only_override_matches_resume() {
    let pool = db::open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    let role_id = seed_role(&conn);
    conn.execute(
        "UPDATE roles SET model = 'fable', effort = 'high' WHERE id = ?1",
        [&role_id],
    )
    .unwrap();
    let mut session =
        crate::test_support::test_session_row("model-override", SessionStatus::Stopped);
    session.role_id = Some(role_id.clone());
    session.agent_runtime = Some("claude-code".into());
    session.launch_definition = Some(crate::test_support::session_definition(
        "claude-code",
        "claude",
    ));
    session.launch_definition.as_mut().unwrap().model = Some("sonnet".into());
    session.launch_definition.as_mut().unwrap().effort = Some("high".into());
    crate::test_support::insert_session_row(&conn, &session).unwrap();
    conn.execute(
        "UPDATE roles SET model = 'opus', effort = 'xhigh' WHERE id = ?1",
        [&role_id],
    )
    .unwrap();
    let entry = get_direct(&conn, "model-override").unwrap().unwrap();
    assert_eq!(entry.agent_model.as_deref(), Some("sonnet"));
    assert_eq!(entry.agent_effort.as_deref(), Some("high"));
}

#[test]
fn session_get_role_chat_runtime_override_uses_new_runtime_defaults() {
    let pool = db::open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    let role_id = seed_role(&conn);
    conn.execute("UPDATE roles SET runtime = 'claude-code', command = 'claude', model = 'fable', effort = 'xhigh' WHERE id = ?1", [&role_id]).unwrap();
    for (id, model, effort) in [
        ("defaults", None, None),
        ("chosen", Some("gpt-5.6-sol"), Some("high")),
    ] {
        let mut session = crate::test_support::test_session_row(id, SessionStatus::Stopped);
        session.role_id = Some(role_id.clone());
        session.agent_runtime = Some("codex".into());
        session.agent_command = Some("old-command".into());
        session.launch_definition = Some(crate::test_support::session_definition("codex", "codex"));
        session.launch_definition.as_mut().unwrap().model = model
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned);
        session.launch_definition.as_mut().unwrap().effort = effort
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned);
        crate::test_support::insert_session_row(&conn, &session).unwrap();
        let entry = get_direct(&conn, id).unwrap().unwrap();
        assert_eq!(entry.agent_runtime, "codex");
        assert_eq!(entry.agent_command, "codex");
        assert_eq!(entry.agent_model.as_deref(), model);
        assert_eq!(entry.agent_effort.as_deref(), effort);
    }
}

#[test]
fn role_backed_chats_keep_their_handle_over_a_provider_title() {
    let entry = |handle: Option<&str>, runtime: &str| {
        let mut row = repo::session::SessionRowDb::new_running(ulid::Ulid::new().to_string());
        row.agent_runtime = Some(runtime.into());
        row.agent_command = Some(runtime.into());
        row.live_title = Some("Housekeeping assistant setup".into());
        direct_entry_from_repo(
            repo::session::DirectSessionRow {
                row,
                role_handle: handle.map(str::to_owned),
                role_display_name: handle.map(|_| "Housekeeper".to_owned()),
            },
            false,
        )
        .unwrap()
    };

    // Role injects the role's system prompt as the first turn, so
    // every chat from one role is titled identically. Identity wins.
    let backed = entry(Some("housekeeper"), "claude-code");
    assert_eq!(backed.preferred_title(None), None);
    assert_eq!(backed.preferred_title(Some("Fix the parser")), None);

    // A name the user typed still outranks the handle.
    let mut named = entry(Some("housekeeper"), "claude-code");
    named.title = Some("Release prep".into());
    assert_eq!(named.preferred_title(None).as_deref(), Some("Release prep"));

    // Runtime-only chats have no persona injected, so their title is
    // the user's own first turn and stays useful.
    let plain = entry(None, "codex");
    assert_eq!(
        plain.preferred_title(None).as_deref(),
        Some("Housekeeping assistant setup")
    );
    assert_eq!(
        plain.preferred_title(Some("Fix the parser")).as_deref(),
        Some("Fix the parser")
    );

    // Shell keeps its terminal title either way.
    let shell = entry(Some("term"), "shell");
    assert_eq!(
        shell.preferred_title(Some("~/repos/runner")).as_deref(),
        Some("~/repos/runner")
    );
}

#[test]
fn direct_entry_reports_native_fork_and_row_eligibility() {
    let entry = |runtime: &str, keyed: bool, mission: bool, archived: bool| {
        let mut row = repo::session::SessionRowDb::new_running(ulid::Ulid::new().to_string());
        row.agent_runtime = Some(runtime.into());
        row.agent_command = Some(runtime.into());
        row.agent_session_key = keyed.then(|| uuid::Uuid::new_v4().to_string());
        row.mission_id = mission.then(|| "mission".into());
        row.archived_at = archived.then(Utc::now);
        direct_entry_from_repo(
            repo::session::DirectSessionRow {
                row,
                role_handle: None,
                role_display_name: None,
            },
            false,
        )
        .unwrap()
    };

    for runtime in ["claude-code", "codex", "pi"] {
        let row = entry(runtime, true, false, false);
        assert!(row.native_fork);
        assert!(row.forkable);
    }
    for runtime in ["trae", "copilot", "antigravity"] {
        let row = entry(runtime, true, false, false);
        assert!(!row.native_fork, "{runtime}");
        assert!(!row.forkable, "{runtime}");
    }
    let unkeyed = entry("codex", false, false, false);
    assert!(unkeyed.native_fork);
    assert!(!unkeyed.forkable);
    assert!(!entry("codex", true, true, false).forkable);
    assert!(!entry("codex", true, false, true).forkable);
}

#[test]
fn session_list_returns_agent_session_key_for_mission_rows() {
    let pool = db::open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    let role_id = seed_role(&conn);
    let crew_id = ulid::Ulid::new().to_string();
    let slot_id = ulid::Ulid::new().to_string();
    let mission_id = ulid::Ulid::new().to_string();
    let session_id = ulid::Ulid::new().to_string();
    let now = Utc::now().to_rfc3339();
    let key = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES (?1, 'C', ?2, ?2)",
        params![crew_id, now],
    )
    .unwrap();
    crate::test_support::insert_test_slot(&conn, &slot_id, &crew_id, &role_id, "coder", 0, true);
    conn.execute(
        "INSERT INTO missions
                (id, crew_id, title, status, started_at)
             VALUES (?1, ?2, 't', 'running', ?3)",
        params![mission_id, crew_id, now],
    )
    .unwrap();
    let mut session = crate::test_support::test_session_row(&session_id, SessionStatus::Running);
    session.mission_id = Some(mission_id.clone());
    session.role_id = Some(role_id.clone());
    session.slot_id = Some(slot_id.clone());
    session.agent_session_key = Some(key.clone());
    crate::test_support::insert_session_row(&conn, &session).unwrap();

    let rows = list_for_mission(&conn, tempfile::tempdir().unwrap().path(), &mission_id).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].session.id, session_id);
    assert_eq!(rows[0].handle, "coder");
    assert_eq!(rows[0].agent_session_key.as_deref(), Some(key.as_str()));
}

#[test]
fn session_list_prefers_recorded_effective_runtime() {
    // Feature 41: mission rows spawned under a slot runtime
    // override record the effective runtime in
    // `sessions.agent_runtime`; session_list must surface that
    // (not the role row's default) so terminal UX gating and
    // badges track the engine actually running.
    let pool = db::open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    let role_id = seed_role(&conn); // runtime 'shell'
    let crew_id = ulid::Ulid::new().to_string();
    let slot_id = ulid::Ulid::new().to_string();
    let mission_id = ulid::Ulid::new().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES (?1, 'C', ?2, ?2)",
        params![crew_id, now],
    )
    .unwrap();
    crate::test_support::insert_test_slot(&conn, &slot_id, &crew_id, &role_id, "coder", 0, true);
    conn.execute(
        "INSERT INTO missions
                (id, crew_id, title, status, started_at)
             VALUES (?1, ?2, 't', 'running', ?3)",
        params![mission_id, crew_id, now],
    )
    .unwrap();
    let overridden = ulid::Ulid::new().to_string();
    let plain = ulid::Ulid::new().to_string();
    let mut overridden_session =
        crate::test_support::test_session_row(&overridden, SessionStatus::Running);
    overridden_session.mission_id = Some(mission_id.clone());
    overridden_session.role_id = Some(role_id.clone());
    overridden_session.slot_id = Some(slot_id.clone());
    overridden_session.agent_runtime = Some("claude-code".into());
    crate::test_support::insert_session_row(&conn, &overridden_session).unwrap();
    let mut plain_session = crate::test_support::test_session_row(&plain, SessionStatus::Running);
    plain_session.mission_id = Some(mission_id.clone());
    plain_session.role_id = Some(role_id.clone());
    plain_session.slot_id = Some(slot_id.clone());
    crate::test_support::insert_session_row(&conn, &plain_session).unwrap();

    let rows = list_for_mission(&conn, tempfile::tempdir().unwrap().path(), &mission_id).unwrap();
    let runtime_for = |id: &str| {
        rows.iter()
            .find(|r| r.session.id == id)
            .map(|r| r.runtime.clone())
            .unwrap()
    };
    assert_eq!(runtime_for(&overridden), "claude-code");
    assert_eq!(runtime_for(&plain), "shell");
}

#[test]
fn session_get_returns_none_for_unknown_id() {
    let pool = db::open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    let row = get_direct(&conn, "01HZUNKNOWNUNKNOWNUNKNOWN").unwrap();
    assert!(row.is_none());
}

#[test]
fn session_get_returns_none_for_mission_session() {
    // `mission_id IS NULL` filter scopes this command to direct
    // chats only — mission sessions go through `session_list`
    // instead. Dropping that filter would let an archived
    // mission's PTY row leak into RoleChat's lookup and confuse
    // the read-only branch (mission sessions don't have a
    // /roles/:handle/chat URL).
    let pool = db::open_in_memory().unwrap();
    let conn = pool.get().unwrap();
    let role_id = seed_role(&conn);

    // Seed a crew + mission so the FK is satisfied. The mission
    // table's other NOT NULL columns are populated minimally.
    let crew_id = ulid::Ulid::new().to_string();
    let mission_id = ulid::Ulid::new().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES (?1, 'C', ?2, ?2)",
        params![crew_id, now],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO missions
                (id, crew_id, title, status, started_at)
             VALUES (?1, ?2, 't', 'running', ?3)",
        params![mission_id, crew_id, now],
    )
    .unwrap();
    let session_id = ulid::Ulid::new().to_string();
    let mut session = crate::test_support::test_session_row(&session_id, SessionStatus::Stopped);
    session.mission_id = Some(mission_id);
    session.role_id = Some(role_id);
    crate::test_support::insert_session_row(&conn, &session).unwrap();

    let row = get_direct(&conn, &session_id).unwrap();
    assert!(
        row.is_none(),
        "mission sessions must not leak through session_get"
    );
}
