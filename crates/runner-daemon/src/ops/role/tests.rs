use super::*;
use crate::db;
use crate::router::runtime::PermissionMode;

fn ctx() -> db::DbPool {
    db::open_in_memory().unwrap()
}

fn make(conn: &Connection, handle: &str) -> Role {
    create(
        conn,
        CreateRoleInput {
            handle: handle.into(),
            display_name: format!("{handle} display"),
            runtime: crate::model::Runtime::Trae,
            command: "sh".into(),
            args: vec![],
            working_dir: None,
            system_prompt: None,
            env: HashMap::new(),
            model: None,
            effort: None,
            // Auto is the form default, but a no-op for trae, so
            // existing tests that expect `args == []` keep passing.
            codex_speed: None,
            permission_mode: PermissionMode::Auto,
        },
    )
    .unwrap()
}

#[test]
fn create_inserts_global_role_without_crew() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let r = make(&conn, "alpha");
    assert_eq!(r.handle, "alpha");
}

#[test]
fn codex_speed_round_trips_and_clears_on_runtime_change() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let role = create(
        &conn,
        serde_json::from_value(serde_json::json!({
            "handle": "speed",
            "display_name": "Speed",
            "runtime": "codex",
            "command": "codex",
            "codex_speed": "fast"
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(role.codex_speed, Some(CodexSpeed::Fast));
    assert_eq!(
        get(&conn, &role.id).unwrap().codex_speed,
        Some(CodexSpeed::Fast)
    );
    let role = update(
        &conn,
        &role.id,
        serde_json::from_value(serde_json::json!({"codex_speed": null})).unwrap(),
    )
    .unwrap();
    assert_eq!(role.codex_speed, None);
    let role = update(
        &conn,
        &role.id,
        serde_json::from_value(serde_json::json!({"codex_speed": "fast"})).unwrap(),
    )
    .unwrap();
    assert_eq!(role.codex_speed, Some(CodexSpeed::Fast));
    assert_eq!(list(&conn).unwrap()[0].codex_speed, Some(CodexSpeed::Fast));
    let role = update(
        &conn,
        &role.id,
        serde_json::from_value(serde_json::json!({"display_name": "Speed renamed"})).unwrap(),
    )
    .unwrap();
    assert_eq!(role.codex_speed, Some(CodexSpeed::Fast));
    let role = update(
        &conn,
        &role.id,
        UpdateRoleInput {
            codex_speed: Some(Some(CodexSpeed::Standard)),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(role.codex_speed, Some(CodexSpeed::Standard));
    let role = update(
        &conn,
        &role.id,
        UpdateRoleInput {
            runtime: Some(Runtime::ClaudeCode),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(role.codex_speed, None);
    let role = update(
        &conn,
        &role.id,
        UpdateRoleInput {
            runtime: Some(Runtime::Codex),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(role.codex_speed, None);
}

#[test]
fn runtime_json_inputs_reject_unknown_names() {
    let input = serde_json::json!({
        "handle": "agent", "display_name": "Agent", "command": "custom-cli",
        "runtime": "aider-future"
    });
    assert!(serde_json::from_value::<CreateRoleInput>(input).is_err());
    assert!(
        serde_json::from_value::<UpdateRoleInput>(serde_json::json!({
            "runtime": "aider-future"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<runner_core::protocol::StartDirectSessionArgs>(
            serde_json::json!({
                "role_id": "agent",
                "runtime": "aider-future"
            })
        )
        .is_err()
    );
}

#[test]
fn get_and_list_keep_runtime_json_strings() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let role = make(&conn, "agent");
    update(
        &conn,
        &role.id,
        UpdateRoleInput {
            runtime: Some(Runtime::Codex),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(get(&conn, &role.id).unwrap()).unwrap()["runtime"],
        "codex"
    );
    assert_eq!(
        serde_json::to_value(list(&conn).unwrap()).unwrap()[0]["runtime"],
        "codex"
    );
}

#[test]
fn list_returns_all_roles_alphabetical() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    make(&conn, "bravo");
    make(&conn, "alpha");
    let roles = list(&conn).unwrap();
    assert_eq!(roles.len(), 2);
    assert_eq!(roles[0].handle, "alpha");
    assert_eq!(roles[1].handle, "bravo");
}

#[test]
fn list_page_filters_handle_and_display_name_case_insensitively() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let role = create(
        &conn,
        CreateRoleInput {
            handle: "handle-needle".into(),
            display_name: "Display Needle".into(),
            runtime: Runtime::Codex,
            command: "/bin/command-needle".into(),
            args: vec!["--args-needle".into()],
            working_dir: Some("/tmp/working-dir-needle".into()),
            system_prompt: Some("System Prompt Needle".into()),
            env: HashMap::new(),
            model: Some("model-needle".into()),
            effort: Some("effort-needle".into()),
            codex_speed: None,
            permission_mode: PermissionMode::Auto,
        },
    )
    .unwrap();
    let mut row = repo::role::RoleRow::from(&role);
    row.runtime = "Runtime-Needle".into();
    repo::role::update(&conn, &row).unwrap();
    make(&conn, "decoy");

    for query in ["HANDLE-NEEDLE", "display needle"] {
        let page = list_with_activity_page(&conn, 1, 8, query).unwrap();
        assert_eq!(page.filtered_count, 1, "query {query:?}");
        assert_eq!(page.items[0].role.id, role.id, "query {query:?}");
    }

    for query in [
        "runtime-needle",
        "command-needle",
        "args-needle",
        "model-needle",
        "effort-needle",
        "working-dir-needle",
        "system prompt needle",
    ] {
        let page = list_with_activity_page(&conn, 1, 8, query).unwrap();
        assert_eq!(page.filtered_count, 0, "query {query:?}");
        assert!(page.items.is_empty(), "query {query:?}");
    }
}

#[test]
fn list_page_treats_like_wildcards_as_literals() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let percent = make(&conn, "percent");
    update(
        &conn,
        &percent.id,
        UpdateRoleInput {
            display_name: Some("100% literal".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let underscore = make(&conn, "underscore");
    update(
        &conn,
        &underscore.id,
        UpdateRoleInput {
            display_name: Some("under_score literal".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let backslash = make(&conn, "backslash");
    update(
        &conn,
        &backslash.id,
        UpdateRoleInput {
            display_name: Some("back\\slash literal".into()),
            ..Default::default()
        },
    )
    .unwrap();
    make(&conn, "plain");

    let percent_page = list_with_activity_page(&conn, 1, 8, "%").unwrap();
    assert_eq!(percent_page.filtered_count, 1);
    assert_eq!(percent_page.items[0].role.id, percent.id);
    let underscore_page = list_with_activity_page(&conn, 1, 8, "_").unwrap();
    assert_eq!(underscore_page.filtered_count, 1);
    assert_eq!(underscore_page.items[0].role.id, underscore.id);
    let backslash_page = list_with_activity_page(&conn, 1, 8, "\\").unwrap();
    assert_eq!(backslash_page.filtered_count, 1);
    assert_eq!(backslash_page.items[0].role.id, backslash.id);
}

#[test]
fn list_page_applies_limit_offset_and_clamps_empty_pages() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    for index in 0..10 {
        make(&conn, &format!("role-{index:02}"));
    }

    let second = list_with_activity_page(&conn, 2, 4, "").unwrap();
    assert_eq!(second.total_count, 10);
    assert_eq!(second.filtered_count, 10);
    assert_eq!(second.items.len(), 4);
    assert_eq!(second.items[0].role.handle, "role-04");
    assert_eq!(second.items[3].role.handle, "role-07");

    let clamped = list_with_activity_page(&conn, 99, 8, "").unwrap();
    assert_eq!(clamped.items.len(), 2);
    assert_eq!(clamped.items[0].role.handle, "role-08");
}

#[test]
fn unique_handle_globally() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    make(&conn, "shared");
    let err = create(
        &conn,
        CreateRoleInput {
            handle: "shared".into(),
            display_name: "Dup".into(),
            runtime: crate::model::Runtime::Trae,
            command: "sh".into(),
            args: vec![],
            working_dir: None,
            system_prompt: None,
            env: HashMap::new(),
            model: None,
            effort: None,
            codex_speed: None,
            permission_mode: PermissionMode::Auto,
        },
    )
    .unwrap_err();
    assert!(err.to_string().to_lowercase().contains("unique"));
}

#[test]
fn update_preserves_unset_fields() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let r = make(&conn, "alpha");
    let updated = update(
        &conn,
        &r.id,
        UpdateRoleInput {
            display_name: Some("renamed".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(updated.display_name, "renamed");
    assert_eq!(updated.handle, r.handle, "handle is unaffected by update");
    assert_eq!(updated.runtime, r.runtime, "unchanged field preserved");
}

const SHELL_RUNTIME_ERROR: &str =
        "unknown runtime 'shell' — valid runtimes: codex, claude-code, antigravity, pi, copilot, trae, cursor, grok, opencode";

#[test]
fn create_rejects_shell_runtime_with_the_slot_override_error() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let err = create(
        &conn,
        CreateRoleInput {
            handle: "terminal".into(),
            display_name: "Terminal".into(),
            runtime: Runtime::Shell,
            command: "/bin/zsh".into(),
            args: vec![],
            working_dir: None,
            system_prompt: None,
            env: HashMap::new(),
            model: None,
            effort: None,
            codex_speed: None,
            permission_mode: PermissionMode::Auto,
        },
    )
    .unwrap_err();
    assert_eq!(err.to_string(), SHELL_RUNTIME_ERROR);
    assert_eq!(
        crate::ops::slot::validate_runtime_override(Some("shell"))
            .unwrap_err()
            .to_string(),
        SHELL_RUNTIME_ERROR
    );
    assert!(list(&conn).unwrap().is_empty());
}

#[test]
fn update_rejects_setting_shell_runtime() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let r = make(&conn, "alpha");
    let err = update(
        &conn,
        &r.id,
        UpdateRoleInput {
            runtime: Some(Runtime::Shell),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(err.to_string(), SHELL_RUNTIME_ERROR);
    assert_eq!(get(&conn, &r.id).unwrap().runtime, "trae");
}

#[test]
fn legacy_shell_role_updates_only_without_a_runtime_and_still_deletes() {
    let pool = ctx();
    let mut conn = pool.get().unwrap();
    let ts = now();
    repo::role::insert(
        &conn,
        &repo::role::RoleRow {
            id: "legacy-shell".into(),
            handle: "legacy-shell".into(),
            display_name: "Legacy shell".into(),
            runtime: "shell".into(),
            command: "/bin/zsh".into(),
            args_json: Some(Vec::new()),
            working_dir: None,
            system_prompt: None,
            env_json: Some(HashMap::new()),
            model: None,
            effort: None,
            codex_speed: None,
            created_at: ts,
            updated_at: ts,
        },
    )
    .unwrap();

    let renamed = update(
        &conn,
        "legacy-shell",
        UpdateRoleInput {
            display_name: Some("Renamed".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(renamed.display_name, "Renamed");
    assert_eq!(renamed.runtime, "shell");

    let err = update(
        &conn,
        "legacy-shell",
        UpdateRoleInput {
            runtime: Some(Runtime::Shell),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(err.to_string(), SHELL_RUNTIME_ERROR);

    delete(&mut conn, "legacy-shell").unwrap();
    assert!(list(&conn).unwrap().is_empty());
}

#[test]
fn update_runtime_clears_only_inheriting_slot_agent_overrides() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let r = make(&conn, "alpha");
    conn.execute(
        "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES ('c1', 'Crew', '2026-04-22T00:00:00Z', '2026-04-22T00:00:00Z')",
        [],
    )
    .unwrap();
    crate::test_support::insert_test_slot(&conn, "s-inherit", "c1", &r.id, "inherit", 0, true);
    crate::test_support::insert_test_slot(&conn, "s-pinned", "c1", &r.id, "pinned", 1, false);
    repo::slot::set_model_override(&conn, "s-inherit", Some("old-model")).unwrap();
    repo::slot::set_effort_override(&conn, "s-inherit", Some("high")).unwrap();
    repo::slot::set_runtime_override(&conn, "s-pinned", Some("claude-code")).unwrap();
    repo::slot::set_model_override(&conn, "s-pinned", Some("opus")).unwrap();
    repo::slot::set_effort_override(&conn, "s-pinned", Some("max")).unwrap();

    update(
        &conn,
        &r.id,
        UpdateRoleInput {
            runtime: Some(crate::model::Runtime::Codex),
            command: Some("codex".into()),
            ..Default::default()
        },
    )
    .unwrap();

    let overrides_for = |slot_id: &str| -> (Option<String>, Option<String>) {
        let slot = repo::slot::get(&conn, slot_id).unwrap().unwrap();
        (slot.model_override, slot.effort_override)
    };
    assert_eq!(overrides_for("s-inherit"), (None, None));
    assert_eq!(
        overrides_for("s-pinned"),
        (Some("opus".into()), Some("max".into())),
    );
}

#[test]
fn delete_removes_row() {
    let pool = ctx();
    let mut conn = pool.get().unwrap();
    let r = make(&conn, "alpha");
    delete(&mut conn, &r.id).unwrap();
    let count = repo::role::count(&conn).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn delete_refuses_role_with_unarchived_direct_chat() {
    let pool = ctx();
    let mut conn = pool.get().unwrap();
    let r = make(&conn, "alpha");
    let mut session =
        crate::test_support::test_session_row("direct-live", crate::model::SessionStatus::Stopped);
    session.role_id = Some(r.id.clone());
    repo::session::insert(&conn, &session).unwrap();

    let err = delete(&mut conn, &r.id).unwrap_err().to_string();
    assert!(err.contains("unarchived chats"));
    let role_count = i64::from(repo::role::get(&conn, &r.id).unwrap().is_some());
    let session_count = i64::from(
        repo::session::get_row(&conn, "direct-live")
            .unwrap()
            .is_some(),
    );
    assert_eq!(role_count, 1, "role must survive refused delete");
    assert_eq!(session_count, 1, "chat must survive refused delete");
}

#[test]
fn delete_removes_role_sessions_before_role_row() {
    let pool = ctx();
    let mut conn = pool.get().unwrap();
    let r = make(&conn, "alpha");
    conn.execute(
        "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES ('crew-1', 'Crew', '2026-04-22T00:00:00Z',
                     '2026-04-22T00:00:00Z')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO missions (id, crew_id, title, status, started_at)
             VALUES ('mission-1', 'crew-1', 'Mission', 'running',
                     '2026-04-22T00:00:00Z')",
        [],
    )
    .unwrap();
    let mut mission_session = crate::test_support::test_session_row(
        "mission-session",
        crate::model::SessionStatus::Running,
    );
    mission_session.mission_id = Some("mission-1".into());
    mission_session.role_id = Some(r.id.clone());
    mission_session.slot_id = Some("slot-1".into());
    repo::session::insert(&conn, &mission_session).unwrap();
    let mut archived_direct = crate::test_support::test_session_row(
        "archived-direct",
        crate::model::SessionStatus::Stopped,
    );
    archived_direct.role_id = Some(r.id.clone());
    archived_direct.archived_at = Some(chrono::Utc::now());
    repo::session::insert(&conn, &archived_direct).unwrap();

    assert!(delete(&mut conn, &r.id)
        .unwrap_err()
        .to_string()
        .contains("unarchived missions"));
    repo::mission::complete_and_archive_if_running(&conn, "mission-1", chrono::Utc::now()).unwrap();
    delete(&mut conn, &r.id).unwrap();
    let session_count = ["mission-session", "archived-direct"]
        .into_iter()
        .filter(|id| repo::session::get_row(&conn, id).unwrap().is_some())
        .count();
    let mission_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM missions WHERE id = 'mission-1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(session_count, 0, "role sessions are hard-deleted");
    assert_eq!(mission_count, 1, "role delete does not delete missions");
}

#[test]
fn delete_removes_sessions_and_attention_with_foreign_keys_off() {
    use crate::test_support::{foreign_keys_off, row_count};
    let pool = ctx();
    let mut conn = pool.get().unwrap();
    foreign_keys_off(&conn);
    let doomed = make(&conn, "doomed");
    let kept = make(&conn, "kept");
    conn.execute_batch(
        "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES ('crew-a', 'A', '2026-04-22T00:00:00Z', '2026-04-22T00:00:00Z');
             INSERT INTO missions (id, crew_id, title, status, started_at)
             VALUES ('mission-a', 'crew-a', 'Mission', 'running', '2026-04-22T00:00:00Z');",
    )
    .unwrap();
    // The doomed role's slot was removed after the mission started, so
    // its mission session outlives it and the role is in no crew.
    for (id, role_id, mission_id) in [
        ("doomed-mission", &doomed.id, Some("mission-a")),
        ("doomed-archived", &doomed.id, None),
        ("kept-chat", &kept.id, None),
    ] {
        let mut row =
            crate::test_support::test_session_row(id, crate::model::SessionStatus::Stopped);
        row.role_id = Some(role_id.clone());
        row.mission_id = mission_id.map(Into::into);
        row.slot_id = mission_id.map(|_| "removed-slot".into());
        if id == "doomed-archived" {
            row.archived_at = Some(chrono::Utc::now());
        }
        repo::session::insert(&conn, &row).unwrap();
        repo::session_attention::record_completion(&conn, id, false, 100).unwrap();
    }

    assert!(delete(&mut conn, &doomed.id)
        .unwrap_err()
        .to_string()
        .contains("unarchived missions"));
    repo::mission::complete_and_archive_if_running(&conn, "mission-a", chrono::Utc::now()).unwrap();
    delete(&mut conn, &doomed.id).unwrap();

    assert_eq!(row_count(&conn, "SELECT COUNT(*) FROM roles"), 1);
    assert_eq!(
        row_count(
            &conn,
            "SELECT COUNT(*) FROM sessions WHERE id IN ('doomed-mission', 'doomed-archived')"
        ),
        0,
        "the role's sessions go with it"
    );
    assert_eq!(
        row_count(
            &conn,
            "SELECT COUNT(*) FROM session_attention WHERE session_id != 'kept-chat'"
        ),
        0,
        "and their attention rows"
    );
    assert_eq!(
        row_count(
            &conn,
            "SELECT COUNT(*) FROM session_attention WHERE session_id = 'kept-chat'"
        ),
        1
    );
    assert_eq!(row_count(&conn, "SELECT COUNT(*) FROM missions"), 1);
}

#[test]
fn delete_refuses_role_in_a_crew_and_changes_nothing() {
    use crate::test_support::{insert_test_slot, row_count};
    let pool = ctx();
    let mut conn = pool.get().unwrap();
    let r = make(&conn, "shared");
    conn.execute_batch(
        "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES ('crew-a', 'Alpha', '2026-04-22T00:00:00Z', '2026-04-22T00:00:00Z'),
                    ('crew-b', 'Beta', '2026-04-22T00:00:00Z', '2026-04-22T00:00:00Z');",
    )
    .unwrap();
    insert_test_slot(&conn, "a-lead", "crew-a", &r.id, "lead", 0, true);
    insert_test_slot(&conn, "b-one", "crew-b", &r.id, "one", 0, true);
    insert_test_slot(&conn, "b-two", "crew-b", &r.id, "two", 1, false);
    let mut archived = crate::test_support::test_session_row(
        "archived-chat",
        crate::model::SessionStatus::Stopped,
    );
    archived.role_id = Some(r.id.clone());
    archived.archived_at = Some(chrono::Utc::now());
    repo::session::insert(&conn, &archived).unwrap();

    let err = delete(&mut conn, &r.id).unwrap_err().to_string();

    assert_eq!(
        err,
        "role @shared is used by crews; remove it from them before deleting this role: Alpha, Beta"
    );
    assert!(repo::role::get(&conn, &r.id).unwrap().is_some());
    assert_eq!(row_count(&conn, "SELECT COUNT(*) FROM slots"), 3);
    assert_eq!(
        row_count(
            &conn,
            "SELECT COUNT(*) FROM sessions WHERE id = 'archived-chat'"
        ),
        1
    );
}

#[test]
fn activity_direct_session_id_ignores_slot_bound_orphans() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let r = make(&conn, "alpha");
    let mut session =
        crate::test_support::test_session_row("slot-orphan", crate::model::SessionStatus::Running);
    session.role_id = Some(r.id.clone());
    session.slot_id = Some("slot-old".into());
    repo::session::insert(&conn, &session).unwrap();

    let got = activity(&conn, &r.id).unwrap();
    assert_eq!(
        got.direct_session_id, None,
        "slot-bound orphan must not be treated as direct chat activity"
    );
}

#[test]
fn delete_on_missing_id_errors_cleanly() {
    let pool = ctx();
    let mut conn = pool.get().unwrap();
    let err = delete(&mut conn, "does-not-exist").unwrap_err();
    assert!(err.to_string().contains("not found"));
}

#[test]
fn handle_must_be_lowercase_slug() {
    assert!(validate_handle("lead").is_ok());
    assert!(validate_handle("impl-1").is_ok());
    assert!(validate_handle("worker_2").is_ok());
    assert!(validate_handle("0worker").is_ok());

    assert!(validate_handle("").is_err());
    assert!(validate_handle("Lead").is_err());
    assert!(validate_handle("lead bot").is_err());
    assert!(validate_handle("lead!").is_err());
    assert!(validate_handle("-lead").is_err());
    assert!(validate_handle(&"x".repeat(33)).is_err());
}

#[test]
fn create_applies_codex_bypass_flags_by_default() {
    // Form's "Skip approval prompts" toggle defaults to on.
    // For codex, that means the canonical
    // `--ask-for-approval never --sandbox workspace-write` pair
    // lands on the `args` column at create time — keeping the
    // role template stable if the recommended default ever
    // shifts (per #45 task 2).
    let pool = ctx();
    let conn = pool.get().unwrap();
    let r = create(
        &conn,
        CreateRoleInput {
            handle: "codex-tester".into(),
            display_name: "C".into(),
            runtime: crate::model::Runtime::Codex,
            command: "codex".into(),
            args: vec!["--debug".into()],
            working_dir: None,
            system_prompt: None,
            env: HashMap::new(),
            model: None,
            effort: None,
            codex_speed: None,
            permission_mode: PermissionMode::Bypass,
        },
    )
    .unwrap();
    assert_eq!(
        r.args,
        vec![
            "--debug".to_string(),
            "--ask-for-approval".to_string(),
            "never".to_string(),
            "--sandbox".to_string(),
            "workspace-write".to_string(),
        ],
    );
}

#[test]
fn create_applies_claude_code_bypass_flag_by_default() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let r = create(
        &conn,
        CreateRoleInput {
            handle: "claude-tester".into(),
            display_name: "Claude".into(),
            runtime: crate::model::Runtime::ClaudeCode,
            command: "claude".into(),
            args: vec![],
            working_dir: None,
            system_prompt: None,
            env: HashMap::new(),
            model: None,
            effort: None,
            codex_speed: None,
            permission_mode: PermissionMode::Bypass,
        },
    )
    .unwrap();
    assert_eq!(
        r.args,
        vec![
            "--permission-mode".to_string(),
            "bypassPermissions".to_string(),
        ],
    );
}

#[test]
fn create_trae_auto_omits_flags_and_update_bakes_bypass() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let r = create(
        &conn,
        CreateRoleInput {
            handle: "trae-tester".into(),
            display_name: "TRAE".into(),
            runtime: crate::model::Runtime::Trae,
            command: "traecli".into(),
            args: vec!["--debug".into()],
            working_dir: None,
            system_prompt: None,
            env: HashMap::new(),
            model: None,
            effort: None,
            codex_speed: None,
            permission_mode: PermissionMode::Auto,
        },
    )
    .unwrap();
    assert_eq!(r.args, vec!["--debug".to_string()]);

    let r = update(
        &conn,
        &r.id,
        UpdateRoleInput {
            permission_mode: Some(PermissionMode::Bypass),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        r.args,
        vec![
            "--debug".to_string(),
            "--permission-mode".to_string(),
            "bypass_permissions".to_string(),
        ],
    );
}

#[test]
fn copilot_create_and_update_bake_only_the_selected_permission_mode() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let role = create(
        &conn,
        CreateRoleInput {
            handle: "copilot-tester".into(),
            display_name: "Copilot".into(),
            runtime: crate::model::Runtime::Copilot,
            command: "copilot".into(),
            args: vec![
                "--debug".into(),
                "--yolo".into(),
                "--allow-tool".into(),
                "shell".into(),
            ],
            working_dir: None,
            system_prompt: None,
            env: HashMap::new(),
            model: None,
            effort: None,
            codex_speed: None,
            permission_mode: PermissionMode::AcceptEdits,
        },
    )
    .unwrap();
    assert_eq!(role.args, ["--debug", "--allow-tool=write"]);
    for (mode, expected) in [
        (PermissionMode::Bypass, vec!["--debug", "--yolo"]),
        (PermissionMode::Default, vec!["--debug"]),
        (PermissionMode::Auto, vec!["--debug"]),
    ] {
        let updated = update(
            &conn,
            &role.id,
            UpdateRoleInput {
                permission_mode: Some(mode),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(updated.args, expected);
    }
}

#[test]
fn antigravity_create_and_update_bake_only_the_selected_permission_mode() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let role = create(
        &conn,
        CreateRoleInput {
            handle: "agy-tester".into(),
            display_name: "Antigravity".into(),
            runtime: crate::model::Runtime::Antigravity,
            command: "agy".into(),
            args: vec![
                "--sandbox".into(),
                "-mode=plan".into(),
                "-dangerously-skip-permissions".into(),
            ],
            working_dir: None,
            system_prompt: None,
            env: HashMap::new(),
            model: None,
            effort: None,
            codex_speed: None,
            permission_mode: PermissionMode::AcceptEdits,
        },
    )
    .unwrap();
    assert_eq!(role.args, ["--sandbox", "--mode", "accept-edits"]);
    for (mode, expected) in [
        (
            PermissionMode::Bypass,
            vec!["--sandbox", "--dangerously-skip-permissions"],
        ),
        (PermissionMode::Default, vec!["--sandbox"]),
        (PermissionMode::Auto, vec!["--sandbox"]),
    ] {
        let updated = update(
            &conn,
            &role.id,
            UpdateRoleInput {
                permission_mode: Some(mode),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(updated.args, expected);
    }
}

#[test]
fn create_omits_bypass_flags_when_toggle_off() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let r = create(
        &conn,
        CreateRoleInput {
            handle: "paranoid".into(),
            display_name: "P".into(),
            runtime: crate::model::Runtime::Codex,
            command: "codex".into(),
            args: vec!["--debug".into()],
            working_dir: None,
            system_prompt: None,
            env: HashMap::new(),
            model: None,
            effort: None,
            codex_speed: None,
            permission_mode: PermissionMode::Default,
        },
    )
    .unwrap();
    assert_eq!(r.args, vec!["--debug".to_string()]);
}

#[test]
fn create_does_not_duplicate_existing_bypass_flags() {
    // CLI / API users who pass a permission flag themselves AND
    // pick a permission_mode shouldn't end up with both shapes on
    // the row. The strip-and-replace round-trip drops the
    // user-supplied flag (including the legacy
    // `--dangerously-skip-permissions` shape) and writes the
    // canonical `--permission-mode <value>` form for the chosen
    // mode.
    let pool = ctx();
    let conn = pool.get().unwrap();
    let r = create(
        &conn,
        CreateRoleInput {
            handle: "explicit".into(),
            display_name: "E".into(),
            runtime: crate::model::Runtime::ClaudeCode,
            command: "claude".into(),
            args: vec!["--dangerously-skip-permissions".into()],
            working_dir: None,
            system_prompt: None,
            env: HashMap::new(),
            model: None,
            effort: None,
            codex_speed: None,
            permission_mode: PermissionMode::Bypass,
        },
    )
    .unwrap();
    assert_eq!(
        r.args,
        vec![
            "--permission-mode".to_string(),
            "bypassPermissions".to_string(),
        ],
        "legacy flag stripped; canonical --permission-mode shape written",
    );
}

#[test]
fn create_no_op_for_runtimes_without_permission_modes() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let r = create(
        &conn,
        CreateRoleInput {
            handle: "pi-tester".into(),
            display_name: "pi-tester".into(),
            runtime: crate::model::Runtime::Pi,
            command: "pi".into(),
            args: vec!["--custom".into()],
            working_dir: None,
            system_prompt: None,
            env: HashMap::new(),
            model: None,
            effort: None,
            codex_speed: None,
            permission_mode: PermissionMode::Bypass,
        },
    )
    .unwrap();
    assert_eq!(r.args, vec!["--custom".to_string()]);
}

#[test]
fn update_skip_approval_toggle_round_trips_for_codex() {
    // Off → strips both halves of the codex bypass pair without
    // touching the unrelated user arg. On → re-adds them.
    // No duplicates accumulate across multiple round-trips.
    let pool = ctx();
    let conn = pool.get().unwrap();
    let r = create(
        &conn,
        CreateRoleInput {
            handle: "codex-rt".into(),
            display_name: "C".into(),
            runtime: crate::model::Runtime::Codex,
            command: "codex".into(),
            args: vec!["--debug".into()],
            working_dir: None,
            system_prompt: None,
            env: HashMap::new(),
            model: None,
            effort: None,
            codex_speed: None,
            permission_mode: PermissionMode::Bypass,
        },
    )
    .unwrap();
    assert!(r.args.contains(&"--ask-for-approval".to_string()));

    // Cycle to Default — strips both halves of the codex pair
    // without touching the unrelated user arg.
    let r = update(
        &conn,
        &r.id,
        UpdateRoleInput {
            permission_mode: Some(PermissionMode::Default),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        r.args,
        vec!["--debug".to_string()],
        "Default mode must strip both --ask-for-approval and --sandbox cleanly",
    );

    // Cycle back to Bypass.
    let r = update(
        &conn,
        &r.id,
        UpdateRoleInput {
            permission_mode: Some(PermissionMode::Bypass),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        r.args,
        vec![
            "--debug".to_string(),
            "--ask-for-approval".to_string(),
            "never".to_string(),
            "--sandbox".to_string(),
            "workspace-write".to_string(),
        ],
    );

    // Re-applying Bypass a second time must not double up.
    let r = update(
        &conn,
        &r.id,
        UpdateRoleInput {
            permission_mode: Some(PermissionMode::Bypass),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        r.args
            .iter()
            .filter(|a| a.as_str() == "--ask-for-approval")
            .count(),
        1,
        "re-applying Bypass must not duplicate flags: {:?}",
        r.args,
    );
}

#[test]
fn update_skip_approval_toggle_round_trips_for_claude_code() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let r = create(
        &conn,
        CreateRoleInput {
            handle: "claude-rt".into(),
            display_name: "C".into(),
            runtime: crate::model::Runtime::ClaudeCode,
            command: "claude".into(),
            args: vec!["--mcp-debug".into()],
            working_dir: None,
            system_prompt: None,
            env: HashMap::new(),
            model: None,
            effort: None,
            codex_speed: None,
            permission_mode: PermissionMode::Bypass,
        },
    )
    .unwrap();
    assert_eq!(
        r.args,
        vec![
            "--mcp-debug".to_string(),
            "--permission-mode".to_string(),
            "bypassPermissions".to_string(),
        ],
    );

    let r = update(
        &conn,
        &r.id,
        UpdateRoleInput {
            permission_mode: Some(PermissionMode::Default),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(r.args, vec!["--mcp-debug".to_string()]);

    let r = update(
        &conn,
        &r.id,
        UpdateRoleInput {
            permission_mode: Some(PermissionMode::Bypass),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        r.args,
        vec![
            "--mcp-debug".to_string(),
            "--permission-mode".to_string(),
            "bypassPermissions".to_string(),
        ],
    );
}

#[test]
fn update_without_toggle_field_preserves_args_verbatim() {
    // Programmatic patches that don't surface the toggle (e.g. a
    // CLI patch that only updates `display_name`) must not
    // accidentally rewrite the args column. `None` toggle = no-op.
    let pool = ctx();
    let conn = pool.get().unwrap();
    let r = create(
        &conn,
        CreateRoleInput {
            handle: "preserve".into(),
            display_name: "P".into(),
            runtime: crate::model::Runtime::Codex,
            command: "codex".into(),
            args: vec!["--debug".into()],
            working_dir: None,
            system_prompt: None,
            env: HashMap::new(),
            model: None,
            effort: None,
            codex_speed: None,
            permission_mode: PermissionMode::Bypass,
        },
    )
    .unwrap();
    let before = r.args.clone();

    let r = update(
        &conn,
        &r.id,
        UpdateRoleInput {
            display_name: Some("renamed".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        r.args, before,
        "args must be untouched when no toggle is sent"
    );
    assert_eq!(r.display_name, "renamed");
}

#[test]
fn update_runtime_switch_strips_prior_bypass_flags() {
    // Switching runtime alongside the toggle must also clean up
    // the old runtime's bypass flags so they don't survive as
    // orphans on the new runtime.
    let pool = ctx();
    let conn = pool.get().unwrap();
    let r = create(
        &conn,
        CreateRoleInput {
            handle: "switcher".into(),
            display_name: "S".into(),
            runtime: crate::model::Runtime::ClaudeCode,
            command: "claude".into(),
            args: vec![],
            working_dir: None,
            system_prompt: None,
            env: HashMap::new(),
            model: None,
            effort: None,
            codex_speed: None,
            permission_mode: PermissionMode::Bypass,
        },
    )
    .unwrap();
    assert_eq!(
        r.args,
        vec![
            "--permission-mode".to_string(),
            "bypassPermissions".to_string(),
        ],
    );

    // Switch to codex with the same mode. Old (claude-code)
    // flag pair must be stripped, new (codex) flag pair must be
    // applied.
    let r = update(
        &conn,
        &r.id,
        UpdateRoleInput {
            runtime: Some(crate::model::Runtime::Codex),
            command: Some("codex".into()),
            permission_mode: Some(PermissionMode::Bypass),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        !r.args.contains(&"--permission-mode".to_string()),
        "claude-code's flag must be stripped on runtime switch: {:?}",
        r.args,
    );
    assert!(
        r.args.contains(&"--ask-for-approval".to_string()),
        "codex bypass pair must be applied on runtime switch: {:?}",
        r.args,
    );
}

fn make_agent_role(
    conn: &Connection,
    handle: &str,
    runtime: Runtime,
    args: &[&str],
    permission_mode: PermissionMode,
) -> Role {
    create(
        conn,
        CreateRoleInput {
            handle: handle.into(),
            display_name: handle.into(),
            runtime,
            command: "agent".into(),
            args: args.iter().map(|arg| (*arg).into()).collect(),
            working_dir: None,
            system_prompt: None,
            env: HashMap::new(),
            model: None,
            effort: None,
            codex_speed: None,
            permission_mode,
        },
    )
    .unwrap()
}

fn update_args(conn: &Connection, role: &Role, args: &[&str]) -> Role {
    update(
        conn,
        &role.id,
        UpdateRoleInput {
            args: Some(args.iter().map(|arg| (*arg).into()).collect()),
            ..Default::default()
        },
    )
    .unwrap()
}

fn switch_runtime(conn: &Connection, role: &Role, runtime: Runtime) -> Role {
    update(
        conn,
        &role.id,
        UpdateRoleInput {
            runtime: Some(runtime),
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn runtime_change_without_mode_rewrites_codex_bypass_for_claude_code() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let role = make_agent_role(
        &conn,
        "codex-to-claude",
        Runtime::Codex,
        &["--debug", "--add-dir", "/tmp/shared"],
        PermissionMode::Bypass,
    );
    let role = switch_runtime(&conn, &role, Runtime::ClaudeCode);
    assert_eq!(
        role.args,
        [
            "--debug",
            "--add-dir",
            "/tmp/shared",
            "--permission-mode",
            "bypassPermissions"
        ],
    );
}

#[test]
fn runtime_change_without_mode_rewrites_claude_code_bypass_for_codex() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let role = make_agent_role(
        &conn,
        "claude-to-codex",
        Runtime::ClaudeCode,
        &["--mcp-debug"],
        PermissionMode::Bypass,
    );
    let role = switch_runtime(&conn, &role, Runtime::Codex);
    assert_eq!(
        role.args,
        [
            "--mcp-debug",
            "--ask-for-approval",
            "never",
            "--sandbox",
            "workspace-write"
        ],
    );
}

#[test]
fn runtime_change_without_mode_keeps_other_modes_or_falls_back_to_default() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    for (index, (from, mode, to, expected)) in [
        (
            Runtime::Codex,
            PermissionMode::Auto,
            Runtime::ClaudeCode,
            &["--keep", "--permission-mode", "auto"][..],
        ),
        (
            Runtime::ClaudeCode,
            PermissionMode::AcceptEdits,
            Runtime::Copilot,
            &["--keep", "--allow-tool=write"][..],
        ),
        (
            Runtime::ClaudeCode,
            PermissionMode::AcceptEdits,
            Runtime::Codex,
            &["--keep"][..],
        ),
        (
            Runtime::ClaudeCode,
            PermissionMode::Auto,
            Runtime::Antigravity,
            &["--keep"][..],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let role = make_agent_role(&conn, &format!("mode-{index}"), from, &["--keep"], mode);
        let role = switch_runtime(&conn, &role, to);
        assert_eq!(role.args, expected, "{from} {mode:?} -> {to}");
    }
}

#[test]
fn runtime_change_with_sent_args_and_no_mode_stores_them_as_sent() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let role = make_agent_role(
        &conn,
        "sent-args",
        Runtime::Codex,
        &[],
        PermissionMode::Bypass,
    );
    let sent = vec!["--debug".to_string()];
    let role = update(
        &conn,
        &role.id,
        UpdateRoleInput {
            runtime: Some(Runtime::ClaudeCode),
            args: Some(sent.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(role.args, sent);
}

#[test]
fn runtime_change_to_or_from_a_runtime_without_permission_flags() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let role = make_agent_role(
        &conn,
        "codex-to-pi",
        Runtime::Codex,
        &["--debug"],
        PermissionMode::Bypass,
    );
    assert_eq!(switch_runtime(&conn, &role, Runtime::Pi).args, ["--debug"]);

    let role = make_agent_role(
        &conn,
        "pi-to-claude",
        Runtime::Pi,
        &["--mode", "json"],
        PermissionMode::Bypass,
    );
    assert_eq!(role.args, ["--mode", "json"]);
    assert_eq!(
        switch_runtime(&conn, &role, Runtime::ClaudeCode).args,
        ["--mode", "json"],
    );
}

#[test]
fn update_with_mode_repairs_another_runtimes_leftover_flags() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    // The issue's role: switched from Codex before the fix, then given the mode again.
    let role = make_agent_role(
        &conn,
        "demo-release",
        Runtime::ClaudeCode,
        &[],
        PermissionMode::Default,
    );
    update_args(
        &conn,
        &role,
        &[
            "--debug",
            "--ask-for-approval",
            "never",
            "--sandbox",
            "workspace-write",
            "--permission-mode",
            "bypassPermissions",
        ],
    );
    let repaired = update(
        &conn,
        &role.id,
        UpdateRoleInput {
            runtime: Some(Runtime::ClaudeCode),
            permission_mode: Some(PermissionMode::Bypass),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        repaired.args,
        ["--debug", "--permission-mode", "bypassPermissions"]
    );

    // Grok's and Cursor's tables strip `--sandbox <value>`, which would split Codex's
    // leftover flags if they ran first on the switch away from them.
    for runtime in [Runtime::Grok, Runtime::Cursor] {
        let role = make_agent_role(
            &conn,
            &format!("repair-switch-{runtime}"),
            runtime,
            &[],
            PermissionMode::Default,
        );
        update_args(
            &conn,
            &role,
            &[
                "--debug",
                "--ask-for-approval",
                "never",
                "--sandbox",
                "workspace-write",
            ],
        );
        let repaired = update(
            &conn,
            &role.id,
            UpdateRoleInput {
                runtime: Some(Runtime::ClaudeCode),
                permission_mode: Some(PermissionMode::Bypass),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            repaired.args,
            ["--debug", "--permission-mode", "bypassPermissions"],
            "{runtime}",
        );
    }

    // Flags these runtimes accept themselves share names with other runtimes'
    // value-taking permission flags, and survive the repair.
    for (index, (runtime, planted, mode, expected)) in [
        (
            Runtime::Antigravity,
            &[
                "--sandbox",
                "--ask-for-approval",
                "never",
                "--sandbox",
                "workspace-write",
            ][..],
            PermissionMode::Bypass,
            &["--sandbox", "--dangerously-skip-permissions"][..],
        ),
        (
            Runtime::Copilot,
            &["--mode", "autopilot", "--plan", "--mode", "accept-edits"][..],
            PermissionMode::AcceptEdits,
            &["--mode", "autopilot", "--plan", "--allow-tool=write"][..],
        ),
        (
            Runtime::Pi,
            &["--mode", "json", "--permission-mode", "bypassPermissions"][..],
            PermissionMode::Default,
            &["--mode", "json"][..],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let role = make_agent_role(
            &conn,
            &format!("repair-{index}"),
            runtime,
            &[],
            PermissionMode::Default,
        );
        update_args(&conn, &role, planted);
        let repaired = update(
            &conn,
            &role.id,
            UpdateRoleInput {
                permission_mode: Some(mode),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(repaired.args, expected, "{runtime}");
    }
}

#[test]
fn update_without_runtime_change_or_mode_leaves_leftover_flags_alone() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let role = make_agent_role(
        &conn,
        "left-alone",
        Runtime::ClaudeCode,
        &[],
        PermissionMode::Default,
    );
    let planted = [
        "--debug",
        "--ask-for-approval",
        "never",
        "--sandbox",
        "workspace-write",
    ];
    update_args(&conn, &role, &planted);
    for input in [
        UpdateRoleInput {
            display_name: Some("renamed".into()),
            ..Default::default()
        },
        UpdateRoleInput {
            runtime: Some(Runtime::ClaudeCode),
            ..Default::default()
        },
    ] {
        assert_eq!(update(&conn, &role.id, input).unwrap().args, planted);
    }
}

#[test]
fn activity_counts_zero_for_brand_new_role() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let r = make(&conn, "alpha");
    let a = activity(&conn, &r.id).unwrap();
    assert_eq!(a.active_sessions, 0);
    assert_eq!(a.active_missions, 0);
    assert_eq!(a.crew_count, 0);
    assert!(a.last_started_at.is_none());
}

#[test]
fn create_rejects_system_prompt_over_cap() {
    // Plan 0007: validation at persist time keeps the composed
    // launch / worker / direct-chat body under
    // `router::runtime::FIRST_TURN_ARGV_MAX_BYTES` so spawn-time
    // argv delivery is guaranteed to fit. Reject oversized
    // `system_prompt` payloads here instead of relying on a
    // post-spawn paste fallback (which the plan retired).
    let pool = ctx();
    let conn = pool.get().unwrap();
    let oversized = "X".repeat(MAX_SYSTEM_PROMPT_BYTES + 1);
    let err = create(
        &conn,
        CreateRoleInput {
            handle: "too-long".into(),
            display_name: "T".into(),
            runtime: crate::model::Runtime::ClaudeCode,
            command: "claude".into(),
            args: vec![],
            working_dir: None,
            system_prompt: Some(oversized),
            env: Default::default(),
            model: None,
            effort: None,
            codex_speed: None,
            permission_mode: crate::router::runtime::PermissionMode::AcceptEdits,
        },
    )
    .expect_err("oversize system_prompt must be rejected");
    assert!(
        err.to_string().contains("system_prompt"),
        "error should mention system_prompt; got {err}"
    );
}

#[test]
fn update_rejects_system_prompt_over_cap() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let r = make(&conn, "victim");
    let oversized = "X".repeat(MAX_SYSTEM_PROMPT_BYTES + 1);
    let err = update(
        &conn,
        &r.id,
        UpdateRoleInput {
            display_name: None,
            runtime: None,
            command: None,
            args: None,
            working_dir: None,
            system_prompt: Some(Some(oversized)),
            env: None,
            model: None,
            effort: None,
            codex_speed: None,
            permission_mode: None,
        },
    )
    .expect_err("oversize system_prompt must be rejected on update");
    assert!(err.to_string().contains("system_prompt"));
}

#[test]
fn activity_counts_running_sessions() {
    let pool = ctx();
    let conn = pool.get().unwrap();
    let r = make(&conn, "alpha");
    // Insert a running session by hand — C6 will own this path later.
    let mut session =
        crate::test_support::test_session_row("s1", crate::model::SessionStatus::Running);
    session.role_id = Some(r.id.clone());
    session.cwd = Some("/tmp".into());
    repo::session::insert(&conn, &session).unwrap();
    let a = activity(&conn, &r.id).unwrap();
    assert_eq!(a.active_sessions, 1);
    assert_eq!(a.active_missions, 0, "direct session has no mission");
    assert!(a.last_started_at.is_some());
}
