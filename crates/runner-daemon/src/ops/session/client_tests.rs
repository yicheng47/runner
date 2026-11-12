use crate::ops::project::ProjectScope;
use crate::ops::session;
use crate::AppCore;
use runner_core::protocol::StartDirectSessionArgs;
#[derive(Default)]
struct TrackingRuntime {
    stops: std::sync::atomic::AtomicUsize,
    outputs: std::sync::Mutex<
        std::collections::HashMap<
            String,
            std::sync::mpsc::Sender<crate::session::runtime::RuntimeOutput>,
        >,
    >,
}

impl crate::session::runtime::SessionRuntime for TrackingRuntime {
    fn spawn(
        &self,
        spec: crate::session::runtime::SpawnSpec,
    ) -> crate::session::runtime::RuntimeResult<(
        crate::session::runtime::RuntimeSession,
        crate::session::runtime::OutputStream,
    )> {
        let (sender, receiver) = std::sync::mpsc::channel();
        self.outputs
            .lock()
            .unwrap()
            .insert(spec.session_id.clone(), sender);
        let session = crate::session::runtime::RuntimeSession {
            runtime: "tracking".into(),
            session_id: spec.session_id,
        };
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        Ok((
            session,
            crate::session::runtime::OutputStream::new(receiver, stop),
        ))
    }

    fn stop(
        &self,
        session: &crate::session::runtime::RuntimeSession,
    ) -> crate::session::runtime::RuntimeResult<()> {
        self.stops.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.outputs.lock().unwrap().remove(&session.session_id);
        Ok(())
    }

    fn send_bytes(
        &self,
        _session: &crate::session::runtime::RuntimeSession,
        _bytes: &[u8],
    ) -> crate::session::runtime::RuntimeResult<()> {
        Ok(())
    }

    fn send_key(
        &self,
        _session: &crate::session::runtime::RuntimeSession,
        _key: &str,
    ) -> crate::session::runtime::RuntimeResult<()> {
        Ok(())
    }

    fn resize(
        &self,
        _session: &crate::session::runtime::RuntimeSession,
        _cols: u16,
        _rows: u16,
    ) -> crate::session::runtime::RuntimeResult<()> {
        Ok(())
    }

    fn status(
        &self,
        _session: &crate::session::runtime::RuntimeSession,
    ) -> crate::session::runtime::RuntimeResult<Option<crate::session::runtime::SessionStatus>>
    {
        Ok(Some(crate::session::runtime::SessionStatus {
            alive: false,
            exit_code: Some(0),
            ..Default::default()
        }))
    }
}

fn result_json(result: impl serde::Serialize) -> serde_json::Value {
    serde_json::to_value(result).unwrap()
}

fn tracking_handler(app_data_dir: std::path::PathBuf) -> AppCore {
    let mut core = crate::test_support::test_core_in(app_data_dir);
    core.sessions = crate::session::SessionManager::new(
        std::sync::Arc::clone(&core.runtime_shell_env),
        std::sync::Arc::clone(&core.runtime_discovery),
        std::sync::Arc::new(TrackingRuntime::default()),
    );
    core
}

fn assert_project_row_and_tab(handler: &AppCore, session_id: &str, project_id: &str) {
    let mut conn = handler.db.get().unwrap();
    let row = crate::repo::session::get_row(&conn, session_id)
        .unwrap()
        .unwrap();
    assert_eq!(row.project_id.as_deref(), Some(project_id));

    let nodes = crate::repo::node::list_with_repair(&mut conn).unwrap();
    let project_node = nodes
        .iter()
        .find(|node| {
            node.node_type == crate::repo::node::NodeType::Project
                && node.ref_id.as_deref() == Some(project_id)
        })
        .unwrap();
    let tab = nodes
        .iter()
        .find(|node| crate::repo::node::session_ids(node) == [session_id])
        .unwrap();
    assert_eq!(tab.parent_id.as_deref(), Some(project_node.id.as_str()));
}

#[test]
fn direct_start_requires_a_role_or_runtime() {
    let core = crate::test_support::test_core();
    let args = StartDirectSessionArgs {
        role_id: None,
        runtime: None,
        model: None,
        effort: None,
        speed: None,
        project_id: None,
        cwd: None,
    };
    assert_eq!(
        session::session_start_chat(&core, args)
            .unwrap_err()
            .to_string(),
        "one of role_id or runtime is required"
    );
}

#[test]
fn role_direct_start_infers_project_from_working_dir_for_the_row_and_tab() {
    let temp = tempfile::tempdir().unwrap();
    let app_data_dir = temp.path().join("app-data");
    let project_cwd = temp.path().join("runner");
    let cwd = project_cwd.join(".worktrees/role-chat");
    std::fs::create_dir_all(&app_data_dir).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    let handler = tracking_handler(app_data_dir);
    let project = {
        let conn = handler.db.get().unwrap();
        crate::test_support::insert_test_role(
            &conn,
            "role",
            "coder",
            "test",
            std::env::current_exe().unwrap().to_string_lossy().as_ref(),
        );
        conn.execute(
            "UPDATE roles SET working_dir = ?2 WHERE id = ?1",
            rusqlite::params!["role", cwd.to_string_lossy().as_ref()],
        )
        .unwrap();
        crate::repo::project::create(&conn, "Runner", project_cwd.to_string_lossy().as_ref())
            .unwrap()
    };

    let output = result_json(
        session::session_start_chat(
            &handler,
            StartDirectSessionArgs {
                role_id: Some("role".into()),
                runtime: None,
                model: None,
                effort: None,
                speed: None,
                project_id: None,
                cwd: None,
            },
        )
        .unwrap(),
    );
    let session_id = output["id"].as_str().unwrap();

    assert_eq!(output["project_id"], project.id);
    assert_project_row_and_tab(&handler, session_id, &project.id);
    session::session_kill(&handler, session_id).unwrap();
}

#[test]
fn runtime_direct_request_infers_project_for_the_session_row_and_tab() {
    let temp = tempfile::tempdir().unwrap();
    let app_data_dir = temp.path().join("app-data");
    let project_cwd = temp.path().join("runner");
    let cwd = project_cwd.join(".worktrees/runtime-chat");
    std::fs::create_dir_all(&app_data_dir).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    let handler = tracking_handler(app_data_dir);
    let project = {
        let conn = handler.db.get().unwrap();
        crate::repo::project::create(&conn, "Runner", project_cwd.to_string_lossy().as_ref())
            .unwrap()
    };

    let output = result_json(
        session::session_start_chat(
            &handler,
            StartDirectSessionArgs {
                role_id: None,
                runtime: Some(crate::model::Runtime::Codex),
                model: None,
                effort: None,
                speed: None,
                project_id: None,
                cwd: Some(cwd.to_string_lossy().into_owned()),
            },
        )
        .unwrap(),
    );
    let session_id = output["id"].as_str().unwrap();

    assert_eq!(output["project_id"], project.id);
    assert_project_row_and_tab(&handler, session_id, &project.id);
    session::session_kill(&handler, session_id).unwrap();
}

#[test]
fn direct_request_persists_speed_for_role_and_runtime_chats() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("app-data")).unwrap();
    let handler = tracking_handler(temp.path().join("app-data"));
    {
        let conn = handler.db.get().unwrap();
        crate::test_support::insert_test_role(
            &conn,
            "role",
            "coder",
            "test",
            std::env::current_exe().unwrap().to_string_lossy().as_ref(),
        );
        conn.execute(
            "UPDATE roles SET runtime = 'codex', runtime_options_json = ?1 WHERE id = 'role'",
            [crate::repo::serde::runtime_speed_json::value(Some(
                crate::model::CodexSpeed::Fast,
            ))],
        )
        .unwrap();
    }
    for (role_id, speed, expected) in [
        (Some("role"), None, Some(crate::model::CodexSpeed::Fast)),
        (
            Some("role"),
            Some(crate::model::CodexSpeed::Standard),
            Some(crate::model::CodexSpeed::Standard),
        ),
        (
            Some("role"),
            Some(crate::model::CodexSpeed::Fast),
            Some(crate::model::CodexSpeed::Fast),
        ),
        (None, None, None),
        (
            None,
            Some(crate::model::CodexSpeed::Standard),
            Some(crate::model::CodexSpeed::Standard),
        ),
        (
            None,
            Some(crate::model::CodexSpeed::Fast),
            Some(crate::model::CodexSpeed::Fast),
        ),
    ] {
        let output = result_json(
            session::session_start_chat(
                &handler,
                StartDirectSessionArgs {
                    role_id: role_id.map(str::to_owned),
                    runtime: role_id.is_none().then_some(crate::model::Runtime::Codex),
                    model: None,
                    effort: None,
                    speed,
                    project_id: None,
                    cwd: Some(temp.path().to_string_lossy().into_owned()),
                },
            )
            .unwrap(),
        );
        let session_id = output["id"].as_str().unwrap();
        let detail = result_json(session::session_get_with_status(&handler, session_id).unwrap());
        assert_eq!(
            detail["agent_speed"],
            serde_json::to_value(expected).unwrap()
        );
        session::session_kill(&handler, session_id).unwrap();
        let row = {
            let conn = handler.db.get().unwrap();
            crate::repo::session::get_row(&conn, session_id)
                .unwrap()
                .unwrap()
        };
        assert_eq!(
            row.launch_definition.as_ref().unwrap().codex_speed,
            expected
        );
    }

    let output = result_json(
        session::session_start_chat(
            &handler,
            StartDirectSessionArgs {
                role_id: None,
                runtime: Some(crate::model::Runtime::ClaudeCode),
                model: None,
                effort: None,
                speed: Some(crate::model::CodexSpeed::Fast),
                project_id: None,
                cwd: Some(temp.path().to_string_lossy().into_owned()),
            },
        )
        .unwrap(),
    );
    let session_id = output["id"].as_str().unwrap();
    let conn = handler.db.get().unwrap();
    assert_eq!(
        crate::repo::session::get_row(&conn, session_id)
            .unwrap()
            .unwrap()
            .launch_definition
            .as_ref()
            .unwrap()
            .codex_speed,
        None
    );
    drop(conn);
    session::session_kill(&handler, session_id).unwrap();
}

#[test]
fn mission_session_detail_resolves_inherited_runtime_and_keeps_speed_source() {
    let temp = tempfile::tempdir().unwrap();
    let handler = tracking_handler(temp.path().join("app-data"));
    {
        let conn = handler.db.get().unwrap();
        let now = chrono::Utc::now();
        crate::test_support::insert_test_role(&conn, "role", "coder", "codex", "codex");
        conn.execute(
            "UPDATE roles SET runtime_options_json = ?1 WHERE id = 'role'",
            [crate::repo::serde::runtime_speed_json::value(Some(
                crate::model::CodexSpeed::Standard,
            ))],
        )
        .unwrap();
        crate::repo::crew::insert(
            &conn,
            &crate::repo::crew::CrewRow {
                id: "crew".into(),
                name: "Pair".into(),
                system_prompt_addendum: None,
                created_at: now,
                updated_at: now,
            },
        )
        .unwrap();
        crate::test_support::insert_test_slot(
            &conn,
            "explicit-slot",
            "crew",
            "role",
            "explicit",
            0,
            true,
        );
        crate::test_support::insert_test_slot(
            &conn,
            "inherited-slot",
            "crew",
            "role",
            "inherited",
            1,
            false,
        );
        conn.execute(
            "UPDATE slots SET runtime_options_json = ?1 WHERE id = 'explicit-slot'",
            [crate::repo::serde::runtime_speed_json::value(Some(
                crate::model::CodexSpeed::Fast,
            ))],
        )
        .unwrap();
        crate::repo::mission::insert(
            &conn,
            &crate::repo::mission::MissionRow {
                id: "mission".into(),
                crew_id: Some("crew".into()),
                project_id: None,
                title: "Speed".into(),
                status: crate::model::MissionStatus::Running,
                goal_override: None,
                cwd: None,
                started_at: now,
                stopped_at: None,
                pinned_at: None,
                archived_at: None,
                linked_session_id: None,
                report_to_chat: true,
            },
        )
        .unwrap();
        for (id, speed) in [
            ("explicit", Some(crate::model::CodexSpeed::Fast)),
            ("inherited", None),
        ] {
            let mut row =
                crate::test_support::test_session_row(id, crate::model::SessionStatus::Stopped);
            row.mission_id = Some("mission".into());
            row.role_id = Some("role".into());
            row.slot_id = Some(format!("{id}-slot"));
            row.agent_runtime = Some("codex".into());
            row.launch_definition = Some(crate::test_support::session_definition("codex", "codex"));
            row.launch_definition.as_mut().unwrap().codex_speed = speed;
            crate::test_support::insert_session_row(&conn, &row).unwrap();
        }
    }
    for (id, speed) in [
        ("explicit", Some(crate::model::CodexSpeed::Fast)),
        ("inherited", None),
    ] {
        let detail = result_json(session::session_get_with_status(&handler, id).unwrap());
        assert_eq!(detail["agent_runtime"], "codex");
        assert_eq!(detail["agent_speed"], serde_json::to_value(speed).unwrap());
    }
}

#[test]
fn shell_start_infers_project_for_the_session_row_and_tab() {
    let temp = tempfile::tempdir().unwrap();
    let app_data_dir = temp.path().join("app-data");
    let project_cwd = temp.path().join("runner");
    let cwd = project_cwd.join(".worktrees/shell");
    std::fs::create_dir_all(&app_data_dir).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    let handler = tracking_handler(app_data_dir);
    let project = {
        let conn = handler.db.get().unwrap();
        crate::repo::project::create(&conn, "Runner", project_cwd.to_string_lossy().as_ref())
            .unwrap()
    };

    let spawned = session::session_start_shell(
        &handler,
        None,
        Some(cwd.to_string_lossy().into_owned()),
        None,
        None,
    )
    .unwrap();

    assert_project_row_and_tab(&handler, &spawned.id, &project.id);
    session::session_kill(&handler, &spawned.id).unwrap();
}

fn assert_root_row_and_tab(handler: &AppCore, session_id: &str) {
    let mut conn = handler.db.get().unwrap();
    let row = crate::repo::session::get_row(&conn, session_id)
        .unwrap()
        .unwrap();
    assert_eq!(row.project_id, None);

    let nodes = crate::repo::node::list_with_repair(&mut conn).unwrap();
    let tab = nodes
        .iter()
        .find(|node| crate::repo::node::session_ids(node) == [session_id])
        .unwrap();
    assert_eq!(tab.parent_id, None);
}

fn project_with_nested_cwd(handler: &AppCore, root: &std::path::Path) -> String {
    let project_cwd = root.join("runner");
    let cwd = project_cwd.join(".worktrees/fix-718");
    std::fs::create_dir_all(&cwd).unwrap();
    let conn = handler.db.get().unwrap();
    crate::repo::project::create(&conn, "Runner", project_cwd.to_string_lossy().as_ref()).unwrap();
    cwd.to_string_lossy().into_owned()
}

#[test]
fn root_scoped_role_chat_stays_unfiled_inside_a_project() {
    let temp = tempfile::tempdir().unwrap();
    let app_data_dir = temp.path().join("app-data");
    std::fs::create_dir_all(&app_data_dir).unwrap();
    let handler = tracking_handler(app_data_dir);
    let cwd = project_with_nested_cwd(&handler, temp.path());
    {
        let conn = handler.db.get().unwrap();
        crate::test_support::insert_test_role(
            &conn,
            "role",
            "coder",
            "test",
            std::env::current_exe().unwrap().to_string_lossy().as_ref(),
        );
        conn.execute(
            "UPDATE roles SET working_dir = ?2 WHERE id = ?1",
            rusqlite::params!["role", cwd],
        )
        .unwrap();
    }

    let spawned = session::session_start_direct(
        &handler,
        "role".into(),
        None,
        None,
        None,
        ProjectScope::Root,
        None,
        None,
        None,
    )
    .unwrap();

    assert_root_row_and_tab(&handler, &spawned.id);
    session::session_kill(&handler, &spawned.id).unwrap();
}

#[test]
fn root_scoped_runtime_chat_stays_unfiled_inside_a_project() {
    let temp = tempfile::tempdir().unwrap();
    let app_data_dir = temp.path().join("app-data");
    std::fs::create_dir_all(&app_data_dir).unwrap();
    let handler = tracking_handler(app_data_dir);
    let cwd = project_with_nested_cwd(&handler, temp.path());

    let output = session::session_start_runtime(
        &handler,
        crate::model::Runtime::Codex.key(),
        ProjectScope::Root,
        Some(cwd),
        None,
        None,
        None,
        None,
    )
    .unwrap();

    assert_eq!(output.project_id, None);
    assert_root_row_and_tab(&handler, &output.session.id);
    session::session_kill(&handler, &output.session.id).unwrap();
}

#[test]
fn root_scoped_shell_stays_unfiled_inside_a_project() {
    let temp = tempfile::tempdir().unwrap();
    let app_data_dir = temp.path().join("app-data");
    std::fs::create_dir_all(&app_data_dir).unwrap();
    let handler = tracking_handler(app_data_dir);
    let cwd = project_with_nested_cwd(&handler, temp.path());

    let spawned =
        session::session_start_shell_in(&handler, ProjectScope::Root, Some(cwd), None, None)
            .unwrap();

    assert_root_row_and_tab(&handler, &spawned.id);
    session::session_kill(&handler, &spawned.id).unwrap();
}

#[test]
fn session_list_returns_direct_chats_only() {
    let handler = crate::test_support::test_core();
    {
        let conn = handler.db.get().unwrap();
        crate::test_support::insert_test_role(&conn, "role", "coder", "codex", "codex");
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
                 VALUES ('crew', 'Crew', '2026-09-18T00:00:00Z', '2026-09-18T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO missions (id, crew_id, title, status, started_at)
                 VALUES ('mission', 'crew', 'Mission', 'running', '2026-09-18T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, role_id, status, started_at, agent_runtime, agent_command)
                 VALUES ('direct', 'role', 'stopped', '2026-09-18T00:00:00Z', 'codex', 'codex')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, mission_id, role_id, status, started_at)
                 VALUES ('mission-session', 'mission', 'role', 'stopped', '2026-09-18T00:00:00Z')",
            [],
        )
        .unwrap();
    }

    let rows = result_json(session::session_list_with_activity(&handler).unwrap());
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["session_id"], "direct");
    assert_eq!(rows[0]["activity"], "unavailable");
}

#[test]
fn session_get_reports_the_managers_working_and_idle_status() {
    let handler = crate::test_support::test_core();
    {
        let conn = handler.db.get().unwrap();
        crate::test_support::insert_test_role(&conn, "role", "coder", "codex", "codex");
        let mut row =
            crate::test_support::test_session_row("direct", crate::model::SessionStatus::Running);
        row.role_id = Some("role".into());
        crate::test_support::insert_session_row(&conn, &row).unwrap();
    }

    assert!(handler.sessions.note_forwarder_transition(
        "direct",
        crate::session::manager::SessionActivityState::Busy,
        crate::session::state::StatusSource::Forwarder
    ));
    let working = result_json(session::session_get_with_status(&handler, "direct").unwrap());
    assert_eq!(
        working["agent_status"]["observation"]["activity"],
        "working"
    );
    assert_eq!(working["activity"], "busy");

    assert!(handler.sessions.note_forwarder_transition(
        "direct",
        crate::session::manager::SessionActivityState::Idle,
        crate::session::state::StatusSource::Forwarder
    ));
    let idle = result_json(session::session_get_with_status(&handler, "direct").unwrap());
    assert_eq!(idle["agent_status"]["observation"]["activity"], "idle");
    assert_eq!(idle["activity"], "idle");
}

#[test]
fn session_stop_preserves_a_resumable_row() {
    let handler = crate::test_support::test_core();
    {
        let conn = handler.db.get().unwrap();
        crate::test_support::insert_test_role(&conn, "role", "coder", "codex", "codex");
        let mut row =
            crate::test_support::test_session_row("direct", crate::model::SessionStatus::Stopped);
        row.role_id = Some("role".into());
        row.agent_session_key = Some("conversation".into());
        crate::test_support::insert_session_row(&conn, &row).unwrap();
    }
    session::session_stop(&handler, "direct").unwrap();
    let row = crate::repo::session::get_row(&handler.db.get().unwrap(), "direct")
        .unwrap()
        .unwrap();
    assert_eq!(row.agent_session_key.as_deref(), Some("conversation"));
    assert_eq!(row.status, crate::model::SessionStatus::Stopped);

    let error = session::session_stop(&handler, "missing").unwrap_err();
    assert!(error.to_string().contains("session not found"));
}

#[test]
fn session_archive_archives_a_chat_and_refuses_a_mission_session() {
    let handler = crate::test_support::test_core();
    {
        let conn = handler.db.get().unwrap();
        crate::test_support::insert_test_role(&conn, "role", "coder", "codex", "codex");
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
                 VALUES ('crew', 'Crew', '2026-09-18T00:00:00Z', '2026-09-18T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO missions (id, crew_id, title, status, started_at)
                 VALUES ('mission', 'crew', 'Mission', 'running', '2026-09-18T00:00:00Z')",
            [],
        )
        .unwrap();
        for (id, mission_id) in [("direct", None), ("mission-session", Some("mission"))] {
            let mut row =
                crate::test_support::test_session_row(id, crate::model::SessionStatus::Stopped);
            row.role_id = Some("role".into());
            row.mission_id = mission_id.map(str::to_owned);
            crate::test_support::insert_session_row(&conn, &row).unwrap();
        }
    }
    session::session_archive_direct(&handler, "direct").unwrap();
    assert!(
        crate::repo::session::get_row(&handler.db.get().unwrap(), "direct")
            .unwrap()
            .unwrap()
            .archived_at
            .is_some()
    );
    let error = session::session_archive_direct(&handler, "mission-session").unwrap_err();
    assert!(error.to_string().contains("mission-scoped"));
}

#[test]
fn session_archive_refuses_a_running_terminal_without_stopping_it() {
    let mut core = crate::test_support::test_core();
    let runtime = std::sync::Arc::new(TrackingRuntime::default());
    core.sessions = crate::session::SessionManager::new(
        std::sync::Arc::clone(&core.runtime_shell_env),
        std::sync::Arc::clone(&core.runtime_discovery),
        runtime.clone(),
    );
    let temp = tempfile::tempdir().unwrap();
    let spawned = session::session_start_shell(
        &core,
        None,
        Some(temp.path().to_string_lossy().into_owned()),
        None,
        None,
    )
    .unwrap();
    let handler = core;

    let error = session::session_archive_direct(&handler, &(spawned.id.clone())).unwrap_err();

    assert!(error
        .to_string()
        .contains("terminals close rather than archive"));
    assert_eq!(runtime.stops.load(std::sync::atomic::Ordering::SeqCst), 0);
    let row = crate::repo::session::get_row(&handler.db.get().unwrap(), &spawned.id)
        .unwrap()
        .unwrap();
    assert_eq!(row.status, crate::model::SessionStatus::Running);
    assert!(row.archived_at.is_none());
    assert_eq!(
        handler.sessions.agent_status(&spawned.id).lifecycle,
        crate::session::status::Lifecycle::Running
    );

    session::session_kill(&handler, &spawned.id).unwrap();
}
