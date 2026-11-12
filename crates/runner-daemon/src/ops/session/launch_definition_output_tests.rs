use super::*;

#[test]
fn direct_and_mission_detail_show_recorded_launch_fields_without_env() {
    let state = crate::test_support::test_core();
    let conn = state.db.get().unwrap();
    crate::test_support::insert_test_role(&conn, "recorded-role", "coder", "codex", "codex");
    conn.execute_batch(
            "INSERT INTO crews (id, name, created_at, updated_at) VALUES ('crew', 'Crew', '2026-10-12T00:00:00Z', '2026-10-12T00:00:00Z');
             INSERT INTO missions (id, crew_id, title, status, started_at) VALUES ('mission', 'crew', 'Mission', 'running', '2026-10-12T00:00:00Z');"
        ).unwrap();
    let mut definition = crate::test_support::session_definition("codex", "configured-command");
    definition
        .env
        .insert("API_KEY".into(), "SECRET_878_SENTINEL".into());
    definition.model = Some("recorded-model".into());
    definition.effort = Some("xhigh".into());
    definition.codex_speed = Some(crate::model::CodexSpeed::Fast);
    for (id, mission) in [("direct", None), ("mission-slot", Some("mission"))] {
        let mut row =
            crate::test_support::test_session_row(id, crate::model::SessionStatus::Stopped);
        row.role_id = Some("recorded-role".into());
        row.mission_id = mission.map(str::to_owned);
        row.agent_runtime = Some("codex".into());
        row.agent_command = Some("/resolved/command".into());
        row.launch_definition = Some(definition.clone());
        repo::session::insert(&conn, &row).unwrap();
    }
    conn.execute("UPDATE roles SET model = 'edited-model', effort = 'high', command = 'edited-command', runtime = 'pi' WHERE id = 'recorded-role'", []).unwrap();
    drop(conn);
    for id in ["direct", "mission-slot"] {
        let detail = session_get_with_status(&state, id).unwrap();
        assert_eq!(detail["agent_runtime"], "codex");
        assert_eq!(detail["agent_command"], "configured-command");
        assert_eq!(detail["agent_model"], "recorded-model");
        assert_eq!(detail["agent_effort"], "xhigh");
        assert_eq!(detail["agent_speed"], "fast");
        assert!(detail.get("launch_definition_json").is_none());
        assert!(detail.get("launch_definition").is_none());
        assert!(!detail.to_string().contains("SECRET_878_SENTINEL"));
    }
    let direct = session_get(&state, "direct").unwrap().unwrap();
    assert_eq!(direct.agent_model.as_deref(), Some("recorded-model"));
    assert_eq!(direct.agent_effort.as_deref(), Some("xhigh"));
    assert_eq!(direct.agent_command, "configured-command");
}
