use super::*;
use crate::db;
use crate::ops::crew::CreateCrewInput;
use crate::ops::role::{self as runner_cmd, CreateRoleInput};
use rusqlite::params;

fn pool() -> db::DbPool {
    db::open_in_memory().unwrap()
}

// Unit coverage here locks the pure resolver's precedence / abstention
// only; the resolver-to-fork seam is exercised by the manager tests.
#[test]
fn mission_fork_size_uses_hint_when_caller_absent() {
    assert_eq!(
        mission_fork_size(None, Some((161, 45))),
        (Some((161, 45)), "mission-hint"),
    );
}

#[test]
fn mission_fork_size_abstains_without_caller_or_hint() {
    assert_eq!(mission_fork_size(None, None), (None, "DEFAULT_PTY_SIZE"));
}

#[test]
fn mission_fork_size_prefers_caller_over_hint() {
    // The UI path measures a real pane; a stale hint must not override it.
    assert_eq!(
        mission_fork_size(Some((120, 30)), Some((161, 45))),
        (Some((120, 30)), "caller-supplied"),
    );
}

fn seed_crew(conn: &Connection, name: &str) -> String {
    let crew = crew::create(
        conn,
        CreateCrewInput {
            name: name.into(),
            ..Default::default()
        },
    )
    .unwrap();
    crew.id
}

fn add_role(conn: &mut Connection, crew_id: &str, handle: &str) -> String {
    // Roles are config templates; in-mission identity is on
    // the slot. Test fixtures use the role handle as both the
    // template name and the slot_handle for simplicity.
    let r = runner_cmd::create(
        conn,
        CreateRoleInput {
            handle: handle.into(),
            display_name: handle.into(),
            runtime: crate::model::Runtime::Trae,
            command: "/bin/sh".into(),
            args: vec![],
            working_dir: None,
            system_prompt: None,
            env: std::collections::HashMap::new(),
            model: None,
            effort: None,
            codex_speed: None,
            permission_mode: crate::router::runtime::PermissionMode::Auto,
        },
    )
    .unwrap();
    slot::create(conn, crew_id, &r.id, handle, None, None)
        .unwrap()
        .slot
        .id
}

fn start_message_test_mission(conn: &mut Connection, app_data_dir: &Path) -> (String, String) {
    let crew_id = seed_crew(conn, "Message crew");
    add_role(conn, &crew_id, "lead");
    add_role(conn, &crew_id, "reviewer");
    let mission = start(
        conn,
        app_data_dir,
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id.clone()),
            title: "Message mission".into(),
            goal_override: Some("Ship it".into()),
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap()
    .mission;
    (crew_id, mission.id)
}

fn append_session_status(
    log: &EventLog,
    crew_id: &str,
    mission_id: &str,
    handle: &str,
    state: &str,
) {
    log.append(EventDraft::signal(
        Some(crew_id),
        mission_id.to_string(),
        handle,
        SignalType::new("session_status"),
        serde_json::json!({ "state": state }),
    ))
    .unwrap();
}

#[test]
fn ensure_first_turn_fits_guards_the_argv_ceiling() {
    // #247: the composed body must stay under the argv ceiling even
    // when every per-field persist cap is individually satisfied
    // (brief 16 KB + goal 8 KB + uncapped team conventions can
    // together overflow). Bodies at the limit pass; a byte over is
    // rejected with a slot-named, actionable error.
    let max = crate::router::runtime::FIRST_TURN_ARGV_MAX_BYTES;
    ensure_first_turn_fits("lead", &"x".repeat(max))
        .expect("a body exactly at the ceiling must be accepted");

    let err = ensure_first_turn_fits("reviewer", &"x".repeat(max + 1))
        .expect_err("a body one byte past the ceiling must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("reviewer"),
        "error must name the slot; got: {msg}"
    );
    assert!(
        msg.contains("argv ceiling"),
        "error must explain the ceiling; got: {msg}",
    );
}

#[test]
fn start_rejects_goal_override_over_cap() {
    // Plan 0007: validation at persist time keeps the composed
    // launch prompt under the runtime argv ceiling. mission_start
    // refuses a goal_override over MAX_MISSION_GOAL_BYTES.
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "C");
    add_role(&mut conn, &crew_id, "lead");
    let tmp = tempfile::tempdir().unwrap();

    let oversized = "Z".repeat(MAX_MISSION_GOAL_BYTES + 1);
    let err = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id),
            title: "Try".into(),
            goal_override: Some(oversized),
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("goal"), "expected goal-size error, got {msg}");
}

#[test]
fn start_rejects_crew_with_no_roles() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "Empty");
    let tmp = tempfile::tempdir().unwrap();

    let err = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id),
            title: "Try".into(),
            goal_override: None,
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("no slots"),
        "expected 'no slots' error, got {msg}"
    );
}

#[test]
fn start_rejects_empty_title() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "A");
    add_role(&mut conn, &crew_id, "coder");
    let tmp = tempfile::tempdir().unwrap();

    let err = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id),
            title: "   ".into(),
            goal_override: None,
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap_err();
    assert!(format!("{err}").contains("title must not be empty"));
}

#[test]
fn post_message_appends_broadcast_message() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let (crew_id, mission_id) = start_message_test_mission(&mut conn, tmp.path());

    let event = post_message(
        tmp.path(),
        &conn,
        PostMessageInput {
            mission_id: mission_id.clone(),
            from: None,
            text: "Heads up".into(),
            to: Default::default(),
        },
    )
    .unwrap();

    assert_eq!(event.kind, EventKind::Message);
    assert_eq!(event.crew_id.as_deref(), Some(crew_id.as_str()));
    assert_eq!(event.mission_id, mission_id);
    assert_eq!(event.from, "human");
    assert!(event.recipients().is_empty());
    assert_eq!(event.signal_type, None);
    assert_eq!(event.payload, serde_json::json!({ "text": "Heads up" }));

    let mission_dir = event_log::mission_dir(tmp.path(), Some(&crew_id), &event.mission_id);
    let entries = EventLog::open(&mission_dir).unwrap().read_from(0).unwrap();
    assert_eq!(entries.last().unwrap().event.id, event.id);
}

#[test]
fn post_message_appends_targeted_roster_message() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let (_crew_id, mission_id) = start_message_test_mission(&mut conn, tmp.path());

    let event = post_message(
        tmp.path(),
        &conn,
        PostMessageInput {
            mission_id,
            from: None,
            text: "  Please review\n".into(),
            to: Some("reviewer".into()).into(),
        },
    )
    .unwrap();

    assert_eq!(event.kind, EventKind::Message);
    assert_eq!(event.from, "human");
    assert_eq!(
        event.recipients().first().map(String::as_str),
        Some("reviewer")
    );
    assert_eq!(
        event.payload,
        serde_json::json!({ "text": "Please review" })
    );
}

#[test]
fn post_message_uses_validated_roster_handle_as_sender() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let (_crew_id, mission_id) = start_message_test_mission(&mut conn, tmp.path());

    let event = post_message(
        tmp.path(),
        &conn,
        PostMessageInput {
            mission_id: mission_id.clone(),
            from: Some("reviewer".into()),
            text: "Reviewed".into(),
            to: Default::default(),
        },
    )
    .unwrap();
    assert_eq!(event.from, "reviewer");

    let error = post_message(
        tmp.path(),
        &conn,
        PostMessageInput {
            mission_id,
            from: Some("missing".into()),
            text: "Nope".into(),
            to: Default::default(),
        },
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "@missing is not in the mission roster");
}

#[tokio::test]
async fn person_signals_persist_resolved_recipients_before_conversion() {
    let temp = tempfile::tempdir().unwrap();
    let state = crate::test_support::test_core_in(temp.path().to_path_buf());
    let mission_id = {
        let mut conn = state.db.get().unwrap();
        start_message_test_mission(&mut conn, &state.app_data_dir).1
    };
    for payload in [
        serde_json::json!({"text":"@lead @reviewer check"}),
        serde_json::json!({"text":"@human explicit wins","target":["lead","reviewer"]}),
    ] {
        let event = mission_signal_impl(
            &state,
            PostSignalInput {
                mission_id: mission_id.clone(),
                from: None,
                signal_type: "human_said".into(),
                payload,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            event.payload["target"],
            serde_json::json!(["lead", "reviewer"])
        );
        assert_eq!(event.payload["delivery"], "inbox");
    }
    let error = mission_signal_impl(
        &state,
        PostSignalInput {
            mission_id,
            from: None,
            signal_type: "human_said".into(),
            payload: serde_json::json!({"text":"@human check"}),
        },
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("valid handles:"));
}

#[test]
fn socket_posts_match_the_shared_resolver() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let (_, mission_id) = start_message_test_mission(&mut conn, tmp.path());
    let handles = vec!["lead".into(), "reviewer".into()];
    for (text, targets) in [
        ("@lead @reviewer check", vec![]),
        ("@lead: check", vec![]),
        ("@lead, @reviewer: check", vec![]),
        ("@reviewer's audit", vec![]),
        ("@", vec![]),
        ("@human explicit wins", vec!["reviewer".into()]),
        ("ask @lead", vec![]),
    ] {
        let expected = runner_core::addressing::resolve(text, &targets, &handles).unwrap();
        let event = post_message(
            tmp.path(),
            &conn,
            PostMessageInput {
                mission_id: mission_id.clone(),
                from: None,
                text: text.into(),
                to: runner_core::Recipients::new(targets),
            },
        )
        .unwrap();
        assert_eq!(event.recipients(), expected.recipients.as_slice());
        assert_eq!(event.payload["text"], expected.text);
    }
}

#[tokio::test]
async fn mission_signal_preserves_human_calls_and_gates_roster_handles() {
    let temp = tempfile::tempdir().unwrap();
    let state = crate::test_support::test_core_in(temp.path().to_path_buf());
    let mission_id = {
        let mut conn = state.db.get().unwrap();
        let (_crew_id, mission_id) = start_message_test_mission(&mut conn, &state.app_data_dir);
        mission_id
    };

    let human = mission_signal_impl(
        &state,
        PostSignalInput {
            mission_id: mission_id.clone(),
            from: None,
            signal_type: "human_said".into(),
            payload: serde_json::json!({"text": "hello"}),
        },
    )
    .await
    .unwrap();
    assert_eq!(human.from, "human");

    let roster = mission_signal_impl(
        &state,
        PostSignalInput {
            mission_id: mission_id.clone(),
            from: Some("reviewer".into()),
            signal_type: "ask_human".into(),
            payload: serde_json::json!({"question": "ready?"}),
        },
    )
    .await
    .unwrap();
    assert_eq!(roster.from, "reviewer");

    let unknown = mission_signal_impl(
        &state,
        PostSignalInput {
            mission_id: mission_id.clone(),
            from: Some("missing".into()),
            signal_type: "ask_human".into(),
            payload: serde_json::json!({}),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(unknown.to_string(), "@missing is not in the mission roster");

    for signal_type in [
        "human_said",
        "human_response",
        "human_question",
        "mission_goal",
    ] {
        let error = mission_signal_impl(
            &state,
            PostSignalInput {
                mission_id: mission_id.clone(),
                from: Some("reviewer".into()),
                signal_type: signal_type.into(),
                payload: serde_json::json!({}),
            },
        )
        .await
        .unwrap_err();
        assert!(
            error.to_string().contains("reserved"),
            "{signal_type}: {error}"
        );
    }
}

#[tokio::test]
async fn human_response_must_name_a_pending_question() {
    let temp = tempfile::tempdir().unwrap();
    let state = crate::test_support::test_core_in(temp.path().to_path_buf());
    let mission = {
        let mut conn = state.db.get().unwrap();
        let (_crew_id, mission_id) = start_message_test_mission(&mut conn, &state.app_data_dir);
        get(&conn, &mission_id).unwrap()
    };
    let answer = |question_id: &str| {
        mission_signal_impl(
            &state,
            PostSignalInput {
                mission_id: mission.id.clone(),
                from: None,
                signal_type: "human_response".into(),
                payload: serde_json::json!({"question_id": question_id, "choice": "yes"}),
            },
        )
    };

    let error = answer("missing").await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "missing is not a pending question; the mission has none pending"
    );

    let ask = mission_signal_impl(
        &state,
        PostSignalInput {
            mission_id: mission.id.clone(),
            from: Some("reviewer".into()),
            signal_type: "ask_human".into(),
            payload: serde_json::json!({"prompt": "Proceed?", "choices": ["yes", "no"]}),
        },
    )
    .await
    .unwrap();
    let log = EventLog::open(&event_log::mission_dir(
        &state.app_data_dir,
        mission.crew_id.as_deref(),
        &mission.id,
    ))
    .unwrap();
    let card = log
            .append(EventDraft::signal(
                mission.crew_id.as_deref(),
                mission.id.clone(),
                "router",
                SignalType::new("human_question"),
                serde_json::json!({"triggered_by": ask.id, "prompt": "Proceed?", "choices": ["yes", "no"]}),
            ))
            .unwrap();

    let error = answer("missing").await.unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "missing is not a pending question; pending question IDs: {}",
            card.id
        )
    );

    let response = answer(&ask.id).await.unwrap();
    assert_eq!(response.payload["question_id"], card.id.as_str());
    assert_eq!(response.payload["delivery"], "inbox");

    let error = answer(&card.id).await.unwrap_err();
    assert!(
        error.to_string().contains("the mission has none pending"),
        "an answered question is no longer pending: {error}"
    );
}

#[test]
fn mission_resume_skips_sessions_already_marked_running() {
    let temp = tempfile::tempdir().unwrap();
    let state = crate::test_support::test_core_in(temp.path().to_path_buf());
    let (mission_id, slot_id) = {
        let mut conn = state.db.get().unwrap();
        let crew_id = seed_crew(&conn, "Resume crew");
        let slot_id = add_role(&mut conn, &crew_id, "lead");
        let mission_id = start(
            &mut conn,
            &state.app_data_dir,
            StartMissionInput {
                crew_id: Some(crew_id),
                project_id: None,
                title: "Resume mission".into(),
                goal_override: None,
                cwd: None,
                started_by_session_id: None,
                no_report: false,
            },
        )
        .unwrap()
        .mission
        .id;
        (mission_id, slot_id)
    };
    let role_id = {
        let conn = state.db.get().unwrap();
        let role_id: String = conn
            .query_row(
                "SELECT role_id FROM slots WHERE id = ?1",
                [&slot_id],
                |row| row.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, mission_id, role_id, slot_id, status, started_at)
                 VALUES ('already-running', ?1, ?2, ?3, 'running', '2026-09-18T00:00:00Z')",
            params![mission_id, role_id, slot_id],
        )
        .unwrap();
        role_id
    };
    assert!(!role_id.is_empty());

    let output = mission_resume(&state, &mission_id).unwrap();
    assert!(output.resumed_session_ids.is_empty());
    assert_eq!(output.sessions.len(), 1);
    assert_eq!(output.sessions[0].session.id, "already-running");
}

#[test]
fn mission_resume_leaves_the_link_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let state = crate::test_support::test_core_in(temp.path().to_path_buf());
    let mission_id = {
        let mut conn = state.db.get().unwrap();
        let mut chat = crate::test_support::test_session_row("chat", SessionStatus::Running);
        chat.agent_runtime = Some("codex".into());
        crate::test_support::insert_session_row(&conn, &chat).unwrap();
        let crew_id = seed_crew(&conn, "Linked resume crew");
        let slot_id = add_role(&mut conn, &crew_id, "lead");
        let mission_id = start(
            &mut conn,
            &state.app_data_dir,
            StartMissionInput {
                crew_id: Some(crew_id),
                project_id: None,
                title: "Linked resume".into(),
                goal_override: None,
                cwd: None,
                started_by_session_id: Some("chat".into()),
                no_report: true,
            },
        )
        .unwrap()
        .mission
        .id;
        conn.execute(
            "INSERT INTO sessions (id, mission_id, role_id, slot_id, status, started_at)
                 SELECT 'stopped-lead', ?1, role_id, id, 'stopped', '2026-09-18T00:00:00Z'
                   FROM slots WHERE id = ?2",
            params![mission_id, slot_id],
        )
        .unwrap();
        mission_id
    };

    let output = mission_resume_with(&state, &mission_id, |_, _| Ok(())).unwrap();

    assert_eq!(output.resumed_session_ids, ["stopped-lead"]);
    let chat = mission_get(&state, &mission_id)
        .unwrap()
        .linked_chat
        .unwrap();
    assert_eq!((chat.session_id.as_str(), chat.reports), ("chat", false));
}

#[test]
fn mission_resume_reports_completed_sessions_when_a_later_resume_fails() {
    let temp = tempfile::tempdir().unwrap();
    let state = crate::test_support::test_core_in(temp.path().to_path_buf());
    let mission_id = {
        let mut conn = state.db.get().unwrap();
        let crew_id = seed_crew(&conn, "Partial resume crew");
        let good_slot = add_role(&mut conn, &crew_id, "good");
        let broken_slot = add_role(&mut conn, &crew_id, "broken");
        let mission_id = start(
            &mut conn,
            &state.app_data_dir,
            StartMissionInput {
                crew_id: Some(crew_id),
                project_id: None,
                title: "Partial resume mission".into(),
                goal_override: None,
                cwd: None,
                started_by_session_id: None,
                no_report: false,
            },
        )
        .unwrap()
        .mission
        .id;
        let good_role: String = conn
            .query_row(
                "SELECT role_id FROM slots WHERE id = ?1",
                [&good_slot],
                |row| row.get(0),
            )
            .unwrap();
        let broken_role: String = conn
            .query_row(
                "SELECT role_id FROM slots WHERE id = ?1",
                [&broken_slot],
                |row| row.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, mission_id, role_id, slot_id, status, started_at)
                 VALUES ('resume-ok', ?1, ?2, ?3, 'stopped', '2026-09-18T00:00:00Z')",
            params![mission_id, good_role, good_slot],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, mission_id, role_id, slot_id, status, started_at)
                 VALUES ('resume-fails', ?1, ?2, ?3, 'stopped', '2026-09-18T00:00:01Z')",
            params![mission_id, broken_role, broken_slot],
        )
        .unwrap();
        mission_id
    };

    let error = mission_resume_with(&state, &mission_id, |_, session_id| {
        if session_id == "resume-ok" {
            Ok(())
        } else {
            Err(Error::msg("synthetic resume failure"))
        }
    })
    .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("resume-fails"), "{message}");
    assert!(message.contains("@broken"), "{message}");
    assert!(
        message.contains("resumed session ids: [resume-ok]"),
        "{message}"
    );

    let output = mission_resume_with(&state, &mission_id, |_, _| {
        Err(Error::msg("session is already being resumed"))
    })
    .unwrap();
    assert!(output.resumed_session_ids.is_empty());
    assert_eq!(output.sessions.len(), 2);
}

#[test]
fn post_message_rejects_unknown_target() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let (_crew_id, mission_id) = start_message_test_mission(&mut conn, tmp.path());

    let err = post_message(
        tmp.path(),
        &conn,
        PostMessageInput {
            mission_id,
            from: None,
            text: "Hello?".into(),
            to: Some("missing".into()).into(),
        },
    )
    .unwrap_err();

    assert!(
        err.to_string().contains("@missing"),
        "error must name the unknown handle; got {err}"
    );
}

#[test]
fn post_message_rejects_human_target() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let (_crew_id, mission_id) = start_message_test_mission(&mut conn, tmp.path());

    let err = post_message(
        tmp.path(),
        &conn,
        PostMessageInput {
            mission_id,
            from: None,
            text: "Echo".into(),
            to: Some("human".into()).into(),
        },
    )
    .unwrap_err();

    assert!(
        err.to_string().contains("@human"),
        "error must explain the refused target; got {err}"
    );
}

#[test]
fn post_message_rejects_empty_or_whitespace_text() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let (_crew_id, mission_id) = start_message_test_mission(&mut conn, tmp.path());

    for text in ["", "  \n\t"] {
        let err = post_message(
            tmp.path(),
            &conn,
            PostMessageInput {
                mission_id: mission_id.clone(),
                from: None,
                text: text.into(),
                to: Default::default(),
            },
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("must not be empty"),
            "error must explain the text guard; got {err}"
        );
    }
}

#[test]
fn post_message_rejects_non_running_mission() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let (_crew_id, mission_id) = start_message_test_mission(&mut conn, tmp.path());
    stop(&mut conn, tmp.path(), &mission_id).unwrap();

    let err = post_message(
        tmp.path(),
        &conn,
        PostMessageInput {
            mission_id,
            from: None,
            text: "Too late".into(),
            to: Default::default(),
        },
    )
    .unwrap_err();

    assert!(
        err.to_string().contains("not running"),
        "error must explain the mission guard; got {err}"
    );
}

#[test]
fn start_writes_two_opening_events_and_sidecar() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "Alpha");
    add_role(&mut conn, &crew_id, "lead");
    let project = repo::project::create(&conn, "Runner", "/tmp/work").unwrap();
    let tmp = tempfile::tempdir().unwrap();

    let out = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: Some(project.id.clone()),
            crew_id: Some(crew_id.clone()),
            title: "first mission".into(),
            goal_override: None,
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();

    assert_eq!(out.mission.title, "first mission");
    assert_eq!(out.mission.project_id.as_deref(), Some(project.id.as_str()));
    assert_eq!(out.mission.cwd.as_deref(), Some("/tmp/work"));
    assert_eq!(out.mission.status, MissionStatus::Running);
    // A mission without an explicit goal starts with an empty goal.
    assert_eq!(out.goal, "");

    // Event log has mission_start + mission_goal.
    let mission_dir = event_log::mission_dir(tmp.path(), Some(&crew_id), &out.mission.id);
    let log = EventLog::open(&mission_dir).unwrap();
    let entries = log.read_from(0).unwrap();
    assert_eq!(entries.len(), 2, "expected two opening events");

    let first = &entries[0].event;
    assert_eq!(first.kind, EventKind::Signal);
    assert_eq!(first.from, "system");
    assert_eq!(
        first.signal_type.as_ref().unwrap().as_str(),
        "mission_start"
    );
    assert_eq!(first.payload["title"], "first mission");
    assert_eq!(first.payload["cwd"], "/tmp/work");
    assert_eq!(first.payload["permission_mode"], "bypass");

    let second = &entries[1].event;
    assert_eq!(second.kind, EventKind::Signal);
    assert_eq!(second.from, "human");
    assert_eq!(
        second.signal_type.as_ref().unwrap().as_str(),
        "mission_goal"
    );
    assert_eq!(second.payload["text"], "");
    // mission_goal must sort strictly after mission_start.
    assert!(second.id > first.id);

    // No signal_types sidecar — CLI validation is enum-based now.
    let stale_sidecar = event_log::crew_dir(tmp.path(), &crew_id).join("signal_types.json");
    assert!(
        !stale_sidecar.exists(),
        "mission_start must not write the legacy signal_types.json sidecar"
    );
}

#[test]
fn start_infers_project_from_cwd_for_the_row_and_sidebar_node() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "Alpha");
    add_role(&mut conn, &crew_id, "lead");
    let tmp = tempfile::tempdir().unwrap();
    let project_cwd = tmp.path().join("runner");
    let cwd = project_cwd.join(".worktrees/feat-680");
    std::fs::create_dir_all(&cwd).unwrap();
    let project =
        repo::project::create(&conn, "Runner", project_cwd.to_string_lossy().as_ref()).unwrap();

    let out = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id),
            title: "inferred project".into(),
            goal_override: None,
            cwd: Some(cwd.to_string_lossy().into_owned()),
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();

    let stored = repo::mission::get(&conn, &out.mission.id).unwrap().unwrap();
    assert_eq!(stored.project_id.as_deref(), Some(project.id.as_str()));
    let project_node = repo::node::find_by_ref(&conn, repo::node::NodeType::Project, &project.id)
        .unwrap()
        .unwrap();
    let mission_node =
        repo::node::find_by_ref(&conn, repo::node::NodeType::Mission, &out.mission.id)
            .unwrap()
            .unwrap();
    assert_eq!(mission_node.parent_id, Some(project_node.id));
}

#[test]
fn start_in_the_root_scope_ignores_a_cwd_inside_a_project() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "Alpha");
    add_role(&mut conn, &crew_id, "lead");
    let tmp = tempfile::tempdir().unwrap();
    let project_cwd = tmp.path().join("runner");
    let cwd = project_cwd.join(".worktrees/fix-718");
    std::fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.to_string_lossy().into_owned();
    repo::project::create(&conn, "Runner", project_cwd.to_string_lossy().as_ref()).unwrap();

    let out = start(
        &mut conn,
        tmp.path(),
        MissionStart {
            crew_id: Some(crew_id),
            scope: ProjectScope::Root,
            title: "recents mission".into(),
            goal_override: None,
            cwd: Some(cwd.clone()),
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();

    let stored = repo::mission::get(&conn, &out.mission.id).unwrap().unwrap();
    assert_eq!(stored.project_id, None);
    assert_eq!(stored.cwd.as_deref(), Some(cwd.as_str()));
    let mission_node =
        repo::node::find_by_ref(&conn, repo::node::NodeType::Mission, &out.mission.id)
            .unwrap()
            .unwrap();
    assert_eq!(mission_node.parent_id, None);
}

#[test]
fn start_explicit_cwd_overrides_project_default() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "Alpha");
    add_role(&mut conn, &crew_id, "lead");
    let project = repo::project::create(&conn, "Runner", "/project").unwrap();
    let tmp = tempfile::tempdir().unwrap();

    let out = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: Some(project.id),
            crew_id: Some(crew_id),
            title: "override cwd".into(),
            goal_override: None,
            cwd: Some("/override".into()),
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();

    assert_eq!(out.mission.cwd.as_deref(), Some("/override"));
}

#[test]
fn start_records_bypass_permissions() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "Alpha");
    add_role(&mut conn, &crew_id, "lead");
    let tmp = tempfile::tempdir().unwrap();

    let out = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id.clone()),
            title: "Bypass".into(),
            goal_override: None,
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();
    let mission_dir = event_log::mission_dir(tmp.path(), Some(&crew_id), &out.mission.id);
    let entries = EventLog::open(&mission_dir).unwrap().read_from(0).unwrap();
    let first = &entries[0].event;
    assert_eq!(
        first.signal_type.as_ref().unwrap().as_str(),
        "mission_start"
    );
    assert_eq!(first.payload["permission_mode"], "bypass");
}

#[test]
fn start_unknown_project_creates_no_mission() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "Alpha");
    add_role(&mut conn, &crew_id, "lead");
    let tmp = tempfile::tempdir().unwrap();

    let error = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: Some("missing".into()),
            crew_id: Some(crew_id),
            title: "unknown project".into(),
            goal_override: None,
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap_err();

    assert_eq!(error.to_string(), "project not found: missing");
    let mission_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM missions", [], |row| row.get(0))
        .unwrap();
    let session_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mission_count, 0);
    assert_eq!(session_count, 0);
}

#[test]
fn start_uses_explicit_mission_goal() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "A");
    add_role(&mut conn, &crew_id, "lead");
    let tmp = tempfile::tempdir().unwrap();

    let out = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id),
            title: "m".into(),
            goal_override: Some("override goal".into()),
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();

    assert_eq!(out.goal, "override goal");
}

#[test]
fn a_resumed_member_keeps_empty_mission_goal() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "A");
    add_role(&mut conn, &crew_id, "lead");
    let tmp = tempfile::tempdir().unwrap();

    let out = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id.clone()),
            title: "m".into(),
            goal_override: None,
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();

    let crew = crew::get(&conn, &crew_id).unwrap();
    let roster = slot::list(&conn, &crew_id).unwrap();
    let log = EventLog::open(&event_log::mission_dir(
        tmp.path(),
        Some(&crew_id),
        &out.mission.id,
    ))
    .unwrap();
    let (system_prompt, first_turn) = crate::router::prompt::member_prompt_channels(
        crate::runtimes::for_key(Runtime::ClaudeCode.key()),
        &roster[0],
        &roster::snapshot(Some(&crew), &roster),
        &log,
        false,
    );
    let prompt = format!(
        "{}{}",
        system_prompt.unwrap_or_default(),
        first_turn.unwrap_or_default()
    );
    assert!(!prompt.contains("== Mission goal =="), "{prompt}");
}

#[test]
fn stop_marks_completed_and_appends_event() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "A");
    add_role(&mut conn, &crew_id, "lead");
    let tmp = tempfile::tempdir().unwrap();

    let out = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id.clone()),
            title: "m".into(),
            goal_override: Some("go".into()),
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();

    let stopped = stop(&mut conn, tmp.path(), &out.mission.id).unwrap();
    assert_eq!(stopped.status, MissionStatus::Completed);
    assert!(stopped.stopped_at.is_some());

    let log = EventLog::open(&event_log::mission_dir(
        tmp.path(),
        Some(&crew_id),
        &out.mission.id,
    ))
    .unwrap();
    let entries = log.read_from(0).unwrap();
    assert_eq!(entries.len(), 3, "start + goal + stopped");
    let last = &entries[2].event;
    assert_eq!(
        last.signal_type.as_ref().unwrap().as_str(),
        "mission_stopped"
    );
    assert_eq!(last.from, "system");
}

#[test]
fn delete_refuses_non_archived_mission() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "A");
    add_role(&mut conn, &crew_id, "lead");
    let tmp = tempfile::tempdir().unwrap();

    let out = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id),
            title: "m".into(),
            goal_override: None,
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();

    let err = delete_archived(&mut conn, tmp.path(), &out.mission.id).unwrap_err();
    assert!(
        err.to_string().contains("not archived"),
        "expected archive-first refusal, got: {err}"
    );

    let unknown = delete_archived(&mut conn, tmp.path(), "no-such-mission").unwrap_err();
    assert!(unknown.to_string().contains("not found"));
}

#[test]
fn delete_archived_removes_mission_and_session_rows() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "A");
    add_role(&mut conn, &crew_id, "lead");
    let tmp = tempfile::tempdir().unwrap();

    let out = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id.clone()),
            title: "m".into(),
            goal_override: None,
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();
    let id = out.mission.id.clone();
    // A stopped slot session — the cascade must take it with the
    // mission instead of leaving an orphan (`mission_id` is
    // ON DELETE SET NULL, which would leak it into direct lists).
    conn.execute(
        "INSERT INTO sessions (id, mission_id, slot_id, status, started_at)
             VALUES ('s-del', ?1, 's1', 'stopped', '2026-07-11T00:00:00Z')",
        params![id],
    )
    .unwrap();
    stop(&mut conn, tmp.path(), &id).unwrap();

    // On-disk footprint: the event-log dir (created by start) and
    // the per-slot shim scratch dir (created at spawn — faked here,
    // the bookkeeping-layer start doesn't spawn PTYs).
    let mission_dir = event_log::mission_dir(tmp.path(), Some(&crew_id), &id);
    assert!(mission_dir.exists());
    let scratch_dir = tmp.path().join("missions").join(&id);
    std::fs::create_dir_all(scratch_dir.join("shims").join("lead").join("bin")).unwrap();

    let deleted = delete_archived(&mut conn, tmp.path(), &id).unwrap();
    assert_eq!(deleted.id, id);
    assert_eq!(deleted.crew_id, Some(crew_id));

    let missions: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM missions WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(missions, 0);
    let orphans: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE id = 's-del'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(orphans, 0, "session rows must die with the mission");
    assert!(!mission_dir.exists(), "event-log dir must be removed");
    assert!(!scratch_dir.exists(), "shim scratch dir must be removed");
}

#[test]
fn delete_archived_removes_sessions_and_attention_with_foreign_keys_off() {
    use crate::test_support::{foreign_keys_off, row_count};
    let pool = pool();
    let mut conn = pool.get().unwrap();
    foreign_keys_off(&conn);
    let crew_id = seed_crew(&conn, "A");
    for mission in ["doomed", "kept"] {
        conn.execute(
            "INSERT INTO missions
                    (id, crew_id, title, status, started_at, stopped_at, archived_at)
                 VALUES (?1, ?2, 'Done', 'completed', '2026-07-11T00:00:00Z',
                         '2026-07-11T01:00:00Z', '2026-07-11T01:00:00Z')",
            params![mission, crew_id],
        )
        .unwrap();
    }
    for (session, mission) in [
        ("doomed-lead", Some("doomed")),
        ("doomed-worker", Some("doomed")),
        ("kept-lead", Some("kept")),
        ("direct", None),
    ] {
        conn.execute(
            "INSERT INTO sessions (id, mission_id, slot_id, status, started_at)
                 VALUES (?1, ?2, ?3, 'stopped', '2026-07-11T00:00:00Z')",
            params![session, mission, mission.map(|_| "s1")],
        )
        .unwrap();
        repo::session_attention::record_completion(&conn, session, false, 100).unwrap();
    }
    let tmp = tempfile::tempdir().unwrap();

    delete_archived(&mut conn, tmp.path(), "doomed").unwrap();

    assert_eq!(
        row_count(&conn, "SELECT COUNT(*) FROM missions WHERE id = 'doomed'"),
        0
    );
    assert_eq!(
        row_count(
            &conn,
            "SELECT COUNT(*) FROM sessions WHERE id LIKE 'doomed-%'"
        ),
        0,
        "the mission's sessions are deleted, not left unbound"
    );
    assert_eq!(
        row_count(
            &conn,
            "SELECT COUNT(*) FROM session_attention WHERE session_id LIKE 'doomed-%'"
        ),
        0
    );
    assert_eq!(row_count(&conn, "SELECT COUNT(*) FROM sessions"), 2);
    assert_eq!(
        row_count(&conn, "SELECT COUNT(*) FROM session_attention"),
        2
    );
    assert_eq!(row_count(&conn, "SELECT COUNT(*) FROM missions"), 1);
}

#[test]
fn abort_start_deletes_sessions_and_attention_with_foreign_keys_off() {
    use crate::test_support::{foreign_keys_off, row_count};
    let pool = pool();
    let mut conn = pool.get().unwrap();
    foreign_keys_off(&conn);
    let crew_id = seed_crew(&conn, "A");
    for mission in ["m1", "m2"] {
        conn.execute(
            "INSERT INTO missions (id, crew_id, title, status, started_at)
                 VALUES (?1, ?2, 'M', 'running', '2026-07-11T00:00:00Z')",
            params![mission, crew_id],
        )
        .unwrap();
    }
    for (session, mission) in [("m1-lead", "m1"), ("m1-worker", "m1"), ("m2-lead", "m2")] {
        conn.execute(
            "INSERT INTO sessions (id, mission_id, status) VALUES (?1, ?2, 'stopped')",
            params![session, mission],
        )
        .unwrap();
        repo::session_attention::record_completion(&conn, session, false, 100).unwrap();
    }

    abort_start(&mut conn, "m1").unwrap();

    assert_eq!(row_count(&conn, "SELECT COUNT(*) FROM sessions"), 1);
    assert_eq!(
        row_count(
            &conn,
            "SELECT COUNT(*) FROM session_attention WHERE session_id = 'm2-lead'"
        ),
        1
    );
    assert_eq!(
        row_count(&conn, "SELECT COUNT(*) FROM session_attention"),
        1
    );
    assert_eq!(
        row_count(
            &conn,
            "SELECT COUNT(*) FROM missions WHERE id = 'm1' AND status = 'aborted'"
        ),
        1
    );
}

#[test]
fn stop_rejects_already_stopped_mission() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "A");
    add_role(&mut conn, &crew_id, "lead");
    let tmp = tempfile::tempdir().unwrap();

    let out = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id),
            title: "m".into(),
            goal_override: Some("go".into()),
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();
    stop(&mut conn, tmp.path(), &out.mission.id).unwrap();

    let err = stop(&mut conn, tmp.path(), &out.mission.id).unwrap_err();
    assert!(format!("{err}").contains("already archived"));
}

#[tokio::test]
async fn restore_log_failure_keeps_mission_archived_without_a_sidebar_node() {
    let temp = tempfile::tempdir().unwrap();
    let state = crate::test_support::test_core_in(temp.path().to_path_buf());
    let (crew_id, id) = {
        let mut conn = state.db.get().unwrap();
        start_message_test_mission(&mut conn, temp.path())
    };
    let archived = mission_archive_impl(&state, id.clone()).await.unwrap();
    let log = EventLog::open(&event_log::mission_dir(temp.path(), Some(&crew_id), &id)).unwrap();
    std::fs::rename(log.path(), log.path().with_extension("saved")).unwrap();
    std::fs::create_dir(log.path()).unwrap();

    assert!(mission_unarchive_impl(&state, id.clone()).await.is_err());
    let conn = state.db.get().unwrap();
    let after = get(&conn, &id).unwrap();
    assert_eq!(after.status, MissionStatus::Completed);
    assert_eq!(after.archived_at, archived.archived_at);
    assert_eq!(after.stopped_at, archived.stopped_at);
    assert!(
        repo::node::find_by_ref(&conn, repo::node::NodeType::Mission, &id)
            .unwrap()
            .is_none()
    );
    assert!(state.routers.get(&id).is_none());
    assert!(state.buses.get(&id).is_none());
}

#[tokio::test]
async fn restore_repairs_legacy_completed_unarchived_missions() {
    let temp = tempfile::tempdir().unwrap();
    let state = crate::test_support::test_core_in(temp.path().to_path_buf());
    let id = {
        let mut conn = state.db.get().unwrap();
        start_message_test_mission(&mut conn, temp.path()).1
    };
    mission_archive_impl(&state, id.clone()).await.unwrap();
    state
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE missions SET archived_at = NULL WHERE id = ?1",
            [&id],
        )
        .unwrap();
    let reopened = mission_unarchive_impl(&state, id.clone()).await.unwrap();
    assert_eq!(reopened.status, MissionStatus::Running);
    assert!(state.routers.get(&id).is_some());
    assert!(state.buses.get(&id).is_some());
    mission_archive_impl(&state, id).await.unwrap();
}

#[test]
fn legacy_completed_unarchived_mission_archives_without_second_terminal_event() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "A");
    add_role(&mut conn, &crew_id, "lead");
    let tmp = tempfile::tempdir().unwrap();

    let out = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id.clone()),
            title: "m".into(),
            goal_override: Some("go".into()),
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();
    let id = out.mission.id.clone();
    let archived = stop(&mut conn, tmp.path(), &id).unwrap();
    let first_stop_at = archived
        .stopped_at
        .expect("first archive stamps stopped_at");

    conn.execute(
        "UPDATE missions SET archived_at = NULL WHERE id = ?1",
        [&id],
    )
    .unwrap();
    repo::node::ensure_mission_node(&conn, &id, None).unwrap();

    let rearchived = stop(&mut conn, tmp.path(), &id).unwrap();
    assert!(
        rearchived.archived_at.is_some(),
        "re-archive must stamp archived_at"
    );
    assert_eq!(
        rearchived.status,
        MissionStatus::Completed,
        "re-archive must not touch the lifecycle status"
    );
    assert_eq!(
        rearchived.stopped_at,
        Some(first_stop_at),
        "re-archive must not move stopped_at"
    );
    assert!(
        repo::node::find_by_ref(&conn, repo::node::NodeType::Mission, &id)
            .unwrap()
            .is_none(),
        "re-archive must drop the sidebar node"
    );

    let log = EventLog::open(&event_log::mission_dir(tmp.path(), Some(&crew_id), &id)).unwrap();
    let stops = log
        .read_from(0)
        .unwrap()
        .iter()
        .filter(|e| {
            e.event
                .signal_type
                .as_ref()
                .is_some_and(|t| t.as_str() == "mission_stopped")
        })
        .count();
    assert_eq!(
        stops, 1,
        "re-archive must not append a second mission_stopped"
    );
}

#[test]
fn list_filters_by_crew_and_orders_by_started_at_desc() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let a = seed_crew(&conn, "A");
    let b = seed_crew(&conn, "B");
    // C5.5: handles are globally unique — give each crew a distinct one.
    add_role(&mut conn, &a, "lead-a");
    add_role(&mut conn, &b, "lead-b");
    let tmp = tempfile::tempdir().unwrap();

    let m1 = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(a.clone()),
            title: "first".into(),
            goal_override: Some("x".into()),
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap()
    .mission;
    // Per #55 concurrent missions on one crew are allowed, so we
    // don't need to stop m1 before starting m2. We also avoid
    // stop() here because it now stamps `archived_at` atomically
    // with the status flip — calling it would hide m1 from
    // list(), which would defeat this test's purpose (verifying
    // the crew filter + ordering, not the archive filter; the
    // latter has its own test).
    std::thread::sleep(std::time::Duration::from_millis(5));
    let m2 = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(a.clone()),
            title: "second".into(),
            goal_override: Some("y".into()),
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap()
    .mission;
    start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(b),
            title: "other crew".into(),
            goal_override: Some("z".into()),
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();

    let for_a = list(&conn, Some(&a)).unwrap();
    assert_eq!(for_a.len(), 2);
    assert_eq!(for_a[0].id, m2.id, "newest first");
    assert_eq!(for_a[1].id, m1.id);

    let all = list(&conn, None).unwrap();
    assert_eq!(all.len(), 3);
}

#[test]
fn concurrent_missions_on_same_crew_are_allowed() {
    // Per #55 the "at most one live mission per crew" guard was
    // lifted: per-mission state (sessions, kill_all_for_mission,
    // shim path, roster sidecar, router) is fully namespaced by
    // mission_id, so a second mission_start on the same crew
    // produces a distinct live mission rather than rejecting.
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "A");
    add_role(&mut conn, &crew_id, "lead");
    let tmp = tempfile::tempdir().unwrap();

    let first = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id.clone()),
            title: "first".into(),
            goal_override: Some("go".into()),
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();

    let second = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id.clone()),
            title: "second".into(),
            goal_override: Some("go".into()),
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();

    assert_ne!(
        first.mission.id, second.mission.id,
        "second mission must get a distinct id",
    );

    // Both rows must show status='running' and reference the same
    // crew. Order by started_at so the assertion doesn't depend on
    // ULID stamping order.
    let mut rows: Vec<(String, String, String)> = conn
        .prepare(
            "SELECT id, status, crew_id FROM missions
                  WHERE crew_id = ?1
                  ORDER BY started_at ASC",
        )
        .unwrap()
        .query_map(params![crew_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(rows.len(), 2, "two mission rows expected: {rows:?}");
    for (_, status, row_crew) in &rows {
        assert_eq!(status, "running");
        assert_eq!(row_crew, &crew_id);
    }
}

#[test]
fn concurrent_stop_appends_exactly_one_terminal_event() {
    // Two threads race to stop the same running mission. Without the
    // conditional UPDATE, both would see `running`, both would flip the
    // row, and both would append `mission_stopped`. With it, exactly one
    // UPDATE affects a row and exactly one log append happens.
    use std::sync::Arc;
    use std::thread;

    // The default `pool()` helper caps at 1 connection + :memory: which
    // gives each connection its own isolated DB — unusable for a race.
    // Use a file-backed DB on disk so multiple pool connections share state.
    let db_tmp = tempfile::tempdir().unwrap();
    let db_path = db_tmp.path().join("race.db");
    let pool = db::open_pool(&db_path).unwrap();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "A");
    add_role(&mut conn, &crew_id, "lead");
    let tmp = Arc::new(tempfile::tempdir().unwrap());

    let out = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id.clone()),
            title: "m".into(),
            goal_override: Some("go".into()),
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();
    drop(conn); // release our pool handle so both threads can grab one

    let pool_a = pool.clone();
    let pool_b = pool.clone();
    let tmp_a = Arc::clone(&tmp);
    let tmp_b = Arc::clone(&tmp);
    let id = out.mission.id.clone();
    let id_a = id.clone();
    let id_b = id.clone();
    let h1 = thread::spawn(move || {
        let mut conn = pool_a.get().unwrap();
        stop(&mut conn, tmp_a.path(), &id_a)
    });
    let h2 = thread::spawn(move || {
        let mut conn = pool_b.get().unwrap();
        stop(&mut conn, tmp_b.path(), &id_b)
    });
    let r1 = h1.join().unwrap();
    let r2 = h2.join().unwrap();

    // Exactly one succeeded; the loser found the row already
    // archived by the winner (#376: the not-running fallthrough
    // archives on the visibility axis, and the archived_at guard
    // rejects it).
    let (ok_count, err_count) = [&r1, &r2].iter().fold((0, 0), |(o, e), r| match r {
        Ok(_) => (o + 1, e),
        Err(err) => {
            assert!(
                format!("{err}").contains("already archived"),
                "loser should report already-archived, got {err}"
            );
            (o, e + 1)
        }
    });
    assert_eq!((ok_count, err_count), (1, 1));

    // Log has exactly one `mission_stopped` event.
    let log = EventLog::open(&event_log::mission_dir(tmp.path(), Some(&crew_id), &id)).unwrap();
    let stopped_events = log
        .read_from(0)
        .unwrap()
        .into_iter()
        .filter(|e| {
            e.event
                .signal_type
                .as_ref()
                .map(|t| t.as_str() == "mission_stopped")
                .unwrap_or(false)
        })
        .count();
    assert_eq!(stopped_events, 1, "exactly one terminal event must land");
}

#[test]
fn read_events_returns_appended_in_order() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "A");
    add_role(&mut conn, &crew_id, "lead");
    let tmp = tempfile::tempdir().unwrap();

    let out = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id.clone()),
            title: "m".into(),
            goal_override: None,
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();

    // mission_start + mission_goal — that's all the opening events
    // plant. Mirrors what the workspace UI sees on first mount.
    let events = read_events(tmp.path(), &conn, &out.mission.id).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0].signal_type.as_ref().unwrap().as_str(),
        "mission_start"
    );
    assert_eq!(
        events[1].signal_type.as_ref().unwrap().as_str(),
        "mission_goal"
    );
}

#[test]
fn mission_activity_from_log_defaults_busy_until_all_live_slots_idle() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "A");
    add_role(&mut conn, &crew_id, "lead");
    let tmp = tempfile::tempdir().unwrap();

    let out = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id.clone()),
            title: "m".into(),
            goal_override: None,
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();
    let mission_dir = event_log::mission_dir(tmp.path(), Some(&crew_id), &out.mission.id);
    let log = EventLog::open(&mission_dir).unwrap();
    let live_handles = vec!["lead".to_string(), "reviewer".to_string()];

    assert_eq!(
        mission_activity_from_log(&mission_dir, &[]),
        None,
        "no live sessions keeps paused/no-live activity empty"
    );
    assert_eq!(
        mission_activity_from_log(&mission_dir, &live_handles),
        Some(MissionActivityState::Busy),
        "live slot without session_status defaults busy"
    );

    append_session_status(&log, &crew_id, &out.mission.id, "lead", "idle");
    assert_eq!(
        mission_activity_from_log(&mission_dir, &live_handles),
        Some(MissionActivityState::Busy),
        "one missing live slot still reads busy"
    );

    append_session_status(&log, &crew_id, &out.mission.id, "reviewer", "idle");
    assert_eq!(
        mission_activity_from_log(&mission_dir, &live_handles),
        Some(MissionActivityState::Idle),
        "all live slots idle reads idle"
    );

    append_session_status(&log, &crew_id, &out.mission.id, "lead", "busy");
    assert_eq!(
        mission_activity_from_log(&mission_dir, &live_handles),
        Some(MissionActivityState::Busy),
        "latest busy status wins"
    );

    append_session_status(&log, &crew_id, &out.mission.id, "lead", "idle");
    assert_eq!(
        mission_activity_from_log(&mission_dir, &live_handles),
        Some(MissionActivityState::Idle),
        "latest idle status wins after busy"
    );

    log.append(EventDraft::signal(
        Some(&crew_id),
        out.mission.id.clone(),
        "lead",
        SignalType::new("runner_status"),
        serde_json::json!({ "state": "busy" }),
    ))
    .unwrap();
    assert_eq!(
        mission_activity_from_log(&mission_dir, &live_handles),
        Some(MissionActivityState::Busy),
        "a runner_status row from a pre-#632 log counts the same"
    );
}

#[test]
fn mission_activity_uses_running_slot_handles_only() {
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "A");
    let lead_slot_id = add_role(&mut conn, &crew_id, "lead");
    let worker_slot_id = add_role(&mut conn, &crew_id, "worker");
    let tmp = tempfile::tempdir().unwrap();

    let out = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id.clone()),
            title: "m".into(),
            goal_override: None,
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();

    let lead_role_id = repo::slot::get(&conn, &lead_slot_id)
        .unwrap()
        .unwrap()
        .role_id;
    let worker_role_id = repo::slot::get(&conn, &worker_slot_id)
        .unwrap()
        .unwrap()
        .role_id;
    let mut lead_session =
        crate::test_support::test_session_row(&new_id(), crate::model::SessionStatus::Running);
    lead_session.mission_id = Some(out.mission.id.clone());
    lead_session.role_id = Some(lead_role_id);
    lead_session.slot_id = Some(lead_slot_id);
    repo::session::insert(&conn, &lead_session).unwrap();
    let mut worker_session =
        crate::test_support::test_session_row(&new_id(), crate::model::SessionStatus::Stopped);
    worker_session.mission_id = Some(out.mission.id.clone());
    worker_session.role_id = Some(worker_role_id);
    worker_session.slot_id = Some(worker_slot_id);
    repo::session::insert(&conn, &worker_session).unwrap();

    let handles = live_session_handles(&conn, &out.mission.id).unwrap();
    assert_eq!(
        handles,
        vec!["lead".to_string()],
        "projection must key live sessions by slot_handle"
    );
    assert!(
        !all_mission_sessions_live(&conn, &out.mission.id).unwrap(),
        "one stopped slot keeps the mission-level live state false"
    );

    let mission_dir = event_log::mission_dir(tmp.path(), Some(&crew_id), &out.mission.id);
    let log = EventLog::open(&mission_dir).unwrap();
    append_session_status(&log, &crew_id, &out.mission.id, "lead", "idle");
    append_session_status(&log, &crew_id, &out.mission.id, "worker", "busy");
    assert_eq!(
        mission_activity_from_log(&mission_dir, &handles),
        Some(MissionActivityState::Idle),
        "stopped slot statuses must not keep the mission busy"
    );

    conn.execute(
        "UPDATE sessions SET status = 'running' WHERE mission_id = ?1",
        params![out.mission.id],
    )
    .unwrap();
    assert!(
        all_mission_sessions_live(&conn, &out.mission.id).unwrap(),
        "all running slots make the mission-level live state true"
    );
}

#[test]
fn pending_ask_count_from_log_pairs_questions_with_responses() {
    // A mission whose router isn't mounted (post-restart, terminal
    // status, etc.) still needs an accurate pending-ask count for the
    // Missions list flag. Append two human_question events and answer
    // only the second one; the helper must report 1 unanswered.
    use runner_core::model::{EventDraft, EventKind, SignalType};

    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "A");
    add_role(&mut conn, &crew_id, "lead");
    let tmp = tempfile::tempdir().unwrap();

    let out = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id.clone()),
            title: "m".into(),
            goal_override: None,
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();

    let mission_dir = event_log::mission_dir(tmp.path(), Some(&crew_id), &out.mission.id);
    assert_eq!(
        count_pending_asks_from_log(&mission_dir),
        0,
        "fresh mission has no pending asks"
    );

    let log = EventLog::open(&mission_dir).unwrap();
    let q1 = log
        .append(EventDraft::signal(
            Some(&crew_id),
            out.mission.id.clone(),
            "router",
            SignalType::new("human_question"),
            serde_json::json!({"prompt": "?"}),
        ))
        .unwrap();
    let _q2 = log
        .append(EventDraft::signal(
            Some(&crew_id),
            out.mission.id.clone(),
            "router",
            SignalType::new("human_question"),
            serde_json::json!({"prompt": "??"}),
        ))
        .unwrap();
    assert_eq!(count_pending_asks_from_log(&mission_dir), 2);

    // Resolve only the first question.
    log.append(EventDraft {
        crew_id: Some(crew_id.clone()),
        mission_id: out.mission.id.clone(),
        kind: EventKind::Signal,
        from: "human".into(),
        to: Default::default(),
        signal_type: Some(SignalType::new("human_response")),
        payload: serde_json::json!({"question_id": q1.id, "choice": "yes"}),
    })
    .unwrap();
    assert_eq!(count_pending_asks_from_log(&mission_dir), 1);
}

#[test]
fn read_events_unknown_mission_errors() {
    let pool = pool();
    let conn = pool.get().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let err = read_events(tmp.path(), &conn, "01HMISSING").unwrap_err();
    assert!(
        format!("{err}").contains("mission not found"),
        "expected not-found, got {err}"
    );
}

#[test]
fn start_rolls_back_row_when_log_append_fails() {
    // Force `EventLog::open` to fail by giving it an `app_data_dir` that
    // can't be created (we preemptively occupy the path with a regular
    // file so `create_dir_all` bails). The mission row must not survive
    // the failure.
    use std::fs;

    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "A");
    add_role(&mut conn, &crew_id, "lead");

    let tmp = tempfile::tempdir().unwrap();
    // Block the `crews/` subtree by making it a file instead of a dir.
    fs::write(tmp.path().join("crews"), b"blocked").unwrap();

    let err = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id.clone()),
            title: "m".into(),
            goal_override: Some("go".into()),
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap_err();
    assert!(
        matches!(err, Error::Io(_)),
        "expected IO failure from FS, got {err:?}"
    );

    // No phantom mission.
    let missions = list(&conn, Some(&crew_id)).unwrap();
    assert!(
        missions.is_empty(),
        "mission row must be rolled back; found {missions:?}"
    );
}

#[test]
fn list_excludes_archived_missions() {
    // `archived_at IS NULL` filter at SQL hides archived missions
    // from every list surface (sidebar, ⌘K palette, summary). Open
    // by direct URL still resolves through `get()`.
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "A");
    add_role(&mut conn, &crew_id, "lead");
    let tmp = tempfile::tempdir().unwrap();

    let out = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id.clone()),
            title: "live".into(),
            goal_override: Some("go".into()),
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();

    // Visible while running.
    assert_eq!(
        list(&conn, Some(&crew_id)).unwrap().len(),
        1,
        "running mission must list"
    );

    // Archive it (stop() flips status AND stamps archived_at atomically).
    let archived = stop(&mut conn, tmp.path(), &out.mission.id).unwrap();
    assert!(archived.archived_at.is_some());

    // Hidden from list().
    assert!(
        list(&conn, Some(&crew_id)).unwrap().is_empty(),
        "archived mission must not appear in list"
    );
    // Still resolves by id via get().
    let fetched = get(&conn, &out.mission.id).unwrap();
    assert!(fetched.archived_at.is_some());
    assert_eq!(fetched.status, MissionStatus::Completed);
}

#[test]
fn stop_sets_archived_at_alongside_status() {
    // stop() is the only path to status='completed' (only called
    // from mission_archive); for a running mission the same UPDATE
    // stamps archived_at, so completed-but-unarchived only ever
    // arises from an explicit restore (impl 0026, re-archived via
    // the visibility-only path — #376).
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "A");
    add_role(&mut conn, &crew_id, "lead");
    let tmp = tempfile::tempdir().unwrap();

    let out = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id),
            title: "m".into(),
            goal_override: Some("go".into()),
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();
    let stopped = stop(&mut conn, tmp.path(), &out.mission.id).unwrap();

    assert_eq!(stopped.status, MissionStatus::Completed);
    let stopped_ts = stopped.stopped_at.expect("stopped_at populated");
    let archived_ts = stopped.archived_at.expect("archived_at populated");
    assert_eq!(stopped_ts, archived_ts, "must share the timestamp");
}

#[test]
fn aborted_missions_stay_visible_in_list() {
    // 0004 backfill is narrow: only `status='completed'` rows get
    // archived_at; aborted rows (spawn-failure rollback) stay in
    // the visible list because they're triage state, not archive.
    // Simulate the production rollback path's UPDATE.
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "A");
    add_role(&mut conn, &crew_id, "lead");
    let tmp = tempfile::tempdir().unwrap();

    let out = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id.clone()),
            title: "m".into(),
            goal_override: Some("go".into()),
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();
    // Mirror mission_start's rollback path: flip to aborted
    // without setting archived_at.
    conn.execute(
        "UPDATE missions SET status = 'aborted', stopped_at = ?1 WHERE id = ?2",
        params![now().to_rfc3339(), out.mission.id],
    )
    .unwrap();

    let listed = list(&conn, Some(&crew_id)).unwrap();
    assert_eq!(listed.len(), 1, "aborted mission must remain in list");
    assert!(
        listed[0].archived_at.is_none(),
        "aborted mission must not be archived"
    );
    assert_eq!(listed[0].status, MissionStatus::Aborted);
}

#[test]
fn migration_backfills_archived_at_for_completed_rows() {
    // The 0004 migration runs once on first open. Simulate the
    // pre-migration state by inserting a `completed` row with
    // `archived_at` already nulled out (which is how the column
    // would look pre-migration), then re-run the migration body
    // and assert the backfill stamped archived_at = stopped_at.
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "A");
    add_role(&mut conn, &crew_id, "lead");
    let tmp = tempfile::tempdir().unwrap();

    // Seed a "pre-migration" completed mission. Use the production
    // stop() then NULL out archived_at to mimic the state of rows
    // that existed before the 0004 column landed.
    let out = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id.clone()),
            title: "old".into(),
            goal_override: Some("g".into()),
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap();
    let stopped = stop(&mut conn, tmp.path(), &out.mission.id).unwrap();
    let original_stopped_at = stopped.stopped_at.unwrap().to_rfc3339();
    conn.execute(
        "UPDATE missions SET archived_at = NULL WHERE id = ?1",
        params![out.mission.id],
    )
    .unwrap();

    // Sanity: row currently looks pre-migration.
    let pre: Option<String> = conn
        .query_row(
            "SELECT archived_at FROM missions WHERE id = ?1",
            params![out.mission.id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(pre.is_none());

    // Re-run the 0004 backfill statement and assert the post state.
    conn.execute(
        "UPDATE missions
                SET archived_at = stopped_at
              WHERE status = 'completed' AND archived_at IS NULL",
        [],
    )
    .unwrap();

    let backfilled = get(&conn, &out.mission.id).unwrap();
    let backfilled_archived_at = backfilled
        .archived_at
        .expect("backfill must populate archived_at")
        .to_rfc3339();
    assert_eq!(backfilled_archived_at, original_stopped_at);
}

#[test]
fn running_ids_filters_status_and_archive() {
    // Startup reconciler should mount router + bus for `running`
    // missions only — archived rows and completed/aborted rows
    // are out of scope. Stable order by started_at so the
    // iteration is reproducible across restarts.
    let pool = pool();
    let mut conn = pool.get().unwrap();
    let crew_id = seed_crew(&conn, "C");
    add_role(&mut conn, &crew_id, "lead");
    let tmp = tempfile::tempdir().unwrap();

    let m_running_first = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id.clone()),
            title: "running-1".into(),
            goal_override: Some("g".into()),
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap()
    .mission
    .id;
    let m_running_second = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id.clone()),
            title: "running-2".into(),
            goal_override: Some("g".into()),
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap()
    .mission
    .id;
    let m_completed = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id.clone()),
            title: "completed".into(),
            goal_override: Some("g".into()),
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap()
    .mission
    .id;
    stop(&mut conn, tmp.path(), &m_completed).unwrap();

    // A third running row that we manually mark archived to
    // simulate a "soft-deleted while running" case the reconciler
    // must ignore.
    let m_archived = start(
        &mut conn,
        tmp.path(),
        StartMissionInput {
            project_id: None,
            crew_id: Some(crew_id),
            title: "running-archived".into(),
            goal_override: Some("g".into()),
            cwd: None,
            started_by_session_id: None,
            no_report: false,
        },
    )
    .unwrap()
    .mission
    .id;
    conn.execute(
        "UPDATE missions SET archived_at = ?2 WHERE id = ?1",
        params![m_archived, Utc::now().to_rfc3339()],
    )
    .unwrap();

    let ids = repo::mission::running_ids(&conn).unwrap();
    assert_eq!(
        ids,
        vec![m_running_first, m_running_second],
        "only non-archived running rows, ordered by started_at"
    );
}
