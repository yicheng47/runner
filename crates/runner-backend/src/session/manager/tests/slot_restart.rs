use super::*;

#[test]
fn worker_restart_reuses_row_replaces_claude_key_and_delivers_first_turn_once() {
    let (pool, app_data, id) = slot_respawn_fixture("claude-code", false);
    let prior = crate::repo::session::get_row(&pool.get().unwrap(), &id)
        .unwrap()
        .unwrap()
        .agent_session_key;
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());
    let out = mgr
        .restart(&id, None, None, app_data.path(), pool.clone(), capture())
        .unwrap();
    let spec = fake.last_spawn_spec().unwrap();
    let body = router::prompt::compose_worker_first_turn(Some("SLOT_BRIEF"), Some("TEAM_RULES"));
    assert_eq!(spec.args.last(), Some(&body));
    assert!(!spec.args.iter().any(|arg| arg == "--resume"));
    assert_eq!(out.id, id);
    let row = crate::repo::session::get_row(&pool.get().unwrap(), &id)
        .unwrap()
        .unwrap();
    assert_ne!(row.agent_session_key, prior);
    assert!(has_arg_pair(
        &spec.args,
        "--session-id",
        row.agent_session_key.as_deref().unwrap()
    ));
    assert_eq!(
        pool.get()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(fake.bytes_writes().is_empty());
    assert!(fake.keys().is_empty());
    mgr.kill(&id).unwrap();
}

#[test]
fn lead_restart_and_missing_conversation_resume_deliver_launch_prompt() {
    for fresh in [true, false] {
        let (pool, app_data, id) = slot_respawn_fixture("claude-code", true);
        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, fake.clone());
        if fresh {
            mgr.restart(&id, None, None, app_data.path(), pool.clone(), capture())
                .unwrap();
        } else {
            router::runtime::with_conversation_home(app_data.path(), || {
                mgr.resume(&id, None, None, app_data.path(), pool.clone(), capture())
            })
            .unwrap();
        }
        let spec = fake.last_spawn_spec().unwrap();
        let body = spec.args.last().unwrap();
        for text in [
            "SLOT_BRIEF",
            "TEAM_RULES",
            "LATEST_GOAL",
            "runner msg read",
            "`slot`",
        ] {
            assert!(body.contains(text), "missing {text}: {body}");
        }
        assert!(!spec.args.iter().any(|arg| arg == "--resume"));
        assert!(fake.bytes_writes().is_empty());
        mgr.kill(&id).unwrap();
    }
}

#[test]
fn missing_worker_conversation_resume_delivers_cold_start_first_turn() {
    for runtime in [
        "claude-code",
        "codex",
        "trae",
        "copilot",
        "antigravity",
        "opencode",
    ] {
        let (pool, app_data, id) = slot_respawn_fixture(runtime, false);
        if !matches!(runtime, "claude-code" | "copilot") {
            pool.get()
                .unwrap()
                .execute("UPDATE sessions SET agent_session_key = NULL", [])
                .unwrap();
        }
        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, fake.clone());
        router::runtime::with_conversation_home(app_data.path(), || {
            mgr.resume(&id, None, None, app_data.path(), pool.clone(), capture())
        })
        .unwrap();
        let spec = fake.last_spawn_spec().unwrap();
        let body =
            router::prompt::compose_worker_first_turn(Some("SLOT_BRIEF"), Some("TEAM_RULES"));
        assert!(
            spec.args.last().unwrap().starts_with(&body),
            "{runtime}: {:?}",
            spec.args
        );
        assert!(!spec.args.iter().any(|arg| matches!(
            arg.as_str(),
            "resume" | "--resume" | "--conversation" | "--session"
        )));
        mgr.kill(&id).unwrap();
    }
}

#[test]
fn codex_restart_clears_prior_key_and_uses_a_new_capture_marker_each_time() {
    for runtime in ["codex", "trae"] {
        let (pool, app_data, id) = slot_respawn_fixture(runtime, false);
        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, fake.clone());
        let mut bodies = Vec::new();
        for _ in 0..2 {
            mgr.restart(&id, None, None, app_data.path(), pool.clone(), capture())
                .unwrap();
            let spec = fake.last_spawn_spec().unwrap();
            let body = spec.args.last().unwrap().clone();
            assert!(body.contains("runner-codex-session-key-capture:"));
            assert!(!spec.args.iter().any(|arg| arg == "resume"));
            assert!(crate::repo::session::get_row(&pool.get().unwrap(), &id)
                .unwrap()
                .unwrap()
                .agent_session_key
                .is_none());
            let state = mgr.session_state(&id).unwrap();
            let state = state.lock().unwrap();
            let capture = state
                .handle
                .as_ref()
                .unwrap()
                .codex_capture
                .as_ref()
                .unwrap();
            assert!(body.contains(capture.prompt_marker.as_deref().unwrap()));
            bodies.push(body);
        }
        assert_ne!(bodies[0], bodies[1]);
        assert!(!fake.stops.lock().unwrap().is_empty());
        assert_eq!(fake.spawn_count(), 2);
        mgr.kill(&id).unwrap();
    }
}

#[test]
fn intact_slot_resume_keeps_key_and_delivers_no_first_turn() {
    for runtime in ["codex", "trae"] {
        let (pool, app_data, id) = slot_respawn_fixture(runtime, false);
        let prior = crate::repo::session::get_row(&pool.get().unwrap(), &id)
            .unwrap()
            .unwrap()
            .agent_session_key
            .unwrap();
        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, fake.clone());
        mgr.resume(&id, None, None, app_data.path(), pool.clone(), capture())
            .unwrap();
        let spec = fake.last_spawn_spec().unwrap();
        assert_eq!(&spec.args[..2], &["resume", prior.as_str()]);
        assert!(!spec
            .args
            .iter()
            .any(|arg| arg.contains("SLOT_BRIEF")
                || arg.contains("runner-codex-session-key-capture:")));
        assert!(fake.bytes_writes().is_empty());
        mgr.kill(&id).unwrap();
    }
}

#[test]
fn restart_claim_prevents_resume_and_restart_while_stopping() {
    let (pool, app_data, id) = slot_respawn_fixture("codex", false);
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());
    mgr.restart(&id, None, None, app_data.path(), pool.clone(), capture())
        .unwrap();
    let (entered, release) = fake.arm_stop_gate();
    thread::scope(|scope| {
        let task =
            scope.spawn(|| mgr.restart(&id, None, None, app_data.path(), pool.clone(), capture()));
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        let restart = mgr
            .restart(&id, None, None, app_data.path(), pool.clone(), capture())
            .unwrap_err();
        let resume = mgr
            .resume(&id, None, None, app_data.path(), pool.clone(), capture())
            .unwrap_err();
        release.send(()).unwrap();
        task.join().unwrap().unwrap();
        for error in [restart, resume] {
            assert!(error.to_string().contains("is already being resumed"));
        }
    });
    assert_eq!(fake.spawn_count(), 2);
    mgr.kill(&id).unwrap();
}

#[test]
fn restart_while_resuming_does_not_kill_or_spawn_again() {
    let (pool, app_data, id) = slot_respawn_fixture("codex", false);
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    let task_mgr = mgr.clone();
    let task_pool = pool.clone();
    let path = app_data.path().to_owned();
    let task_id = id.clone();
    *fake.spawn_hook.lock().unwrap() = Some(Box::new(move || {
        tx.send(
            task_mgr
                .restart(&task_id, None, None, &path, task_pool.clone(), capture())
                .unwrap_err()
                .to_string(),
        )
        .unwrap();
    }));
    mgr.resume(&id, None, None, app_data.path(), pool.clone(), capture())
        .unwrap();
    assert!(rx.recv().unwrap().contains("is already being resumed"));
    assert!(fake.stops.lock().unwrap().is_empty());
    assert_eq!(fake.spawn_count(), 1);
    mgr.kill(&id).unwrap();
}

#[test]
fn restart_uses_current_pane_size_for_stopped_and_running_slots() {
    let (pool, app_data, id) = slot_respawn_fixture("codex", false);
    crate::repo::session::update_last_size(&pool.get().unwrap(), &id, 80, 24).unwrap();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());
    for (cols, rows) in [(180, 50), (220, 60)] {
        mgr.restart(
            &id,
            Some(cols),
            Some(rows),
            app_data.path(),
            pool.clone(),
            capture(),
        )
        .unwrap();
        assert_eq!(
            fake.last_spawn_spec().unwrap().initial_size,
            Some((cols, rows))
        );
        let row = crate::repo::session::get_row(&pool.get().unwrap(), &id)
            .unwrap()
            .unwrap();
        assert_eq!(row.last_cols.zip(row.last_rows), Some((cols, rows)));
    }
    mgr.kill(&id).unwrap();
}

#[test]
fn completed_mission_rejects_resume_and_restart() {
    let (pool, app_data, id) = slot_respawn_fixture("codex", false);
    let mission_id = crate::repo::session::get_row(&pool.get().unwrap(), &id)
        .unwrap()
        .unwrap()
        .mission_id
        .unwrap();
    pool.get()
        .unwrap()
        .execute(
            "UPDATE missions SET status = 'completed' WHERE id = ?1",
            params![mission_id],
        )
        .unwrap();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());
    let error = mgr
        .restart(&id, None, None, app_data.path(), pool.clone(), capture())
        .unwrap_err();
    assert!(error.to_string().contains("mission is not running"));
    assert_eq!(fake.spawn_count(), 0);
    for error in [
        mgr.resume(&id, None, None, app_data.path(), pool.clone(), capture())
            .unwrap_err(),
        mgr.resume_on_launch(&id, None, None, app_data.path(), pool.clone(), capture())
            .unwrap_err(),
    ] {
        assert!(error.to_string().contains("mission is not running"));
    }
    assert_eq!(fake.spawn_count(), 0);
}

#[test]
fn restart_notification_failure_warns_without_failing_the_completed_restart() {
    let (pool, app_data, id) = slot_respawn_fixture("claude-code", true);
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());
    let core = crate::AppCore {
        db: pool.clone(),
        app_data_dir: app_data.path().to_owned(),
        sessions: mgr.clone(),
        runtime_shell_env: mgr.shell_env.clone(),
        runtime_discovery: mgr.discovery_state.clone(),
        usage: Arc::new(crate::usage::UsageService::default()),
        buses: crate::event_bus::BusRegistry::new(),
        routers: crate::router::RouterRegistry::new(),
        mission_grid_hint: Arc::new(Mutex::new(None)),
        mcp: Arc::new(crate::mcp::McpHandle::new()),
        windows: Arc::new(crate::windows::WindowRegistry::new()),
        events: crate::events::EventChannel::new(),
        session_event_observer: Default::default(),
        app_version: "0.0.0-test".into(),
    };
    let row = crate::repo::session::get_row(&pool.get().unwrap(), &id)
        .unwrap()
        .unwrap();
    let mission_id = row.mission_id.unwrap();
    let conn = pool.get().unwrap();
    let mission = crate::ops::mission::get(&conn, &mission_id).unwrap();
    let crew = crate::ops::crew::get(&conn, &mission.crew_id).unwrap();
    let roster = crate::ops::slot::list(&conn, &crew.id).unwrap();
    drop(conn);
    let log = Arc::new(
        runner_core::event_log::EventLog::open(&app_data.path().join("failed-router-log")).unwrap(),
    );
    let router = router::Router::new(
        mission_id.clone(),
        crew.id,
        crew.name,
        &roster,
        crate::ops::mission::all_known_signals(),
        crew.system_prompt_addendum,
        log.clone(),
        mgr.clone(),
        Arc::new(router::ChannelRouterUiNotifier(core.events.clone())),
    )
    .unwrap();
    router.register_sessions(&[("slot".into(), id.clone())]);
    core.routers.register(mission_id.clone(), router);
    std::fs::rename(log.path(), log.path().with_extension("saved")).unwrap();
    std::fs::create_dir(log.path()).unwrap();
    let mut events = core.events.subscribe();
    let result = crate::ops::session::session_restart(&core, &id, Some(180), Some(50));
    let row = crate::repo::session::get_row(&pool.get().unwrap(), &id)
        .unwrap()
        .unwrap();
    let mut warning = None;
    let mut updated = false;
    while let Ok(event) = events.try_recv() {
        if event.name == "session/warning" {
            warning = Some(event.payload);
        }
        updated |= event.name == "session/updated";
    }
    core.routers.unregister(&mission_id);
    mgr.kill(&id).unwrap();
    assert_eq!(result.unwrap().id, id);
    assert_eq!(row.status, crate::model::SessionStatus::Running);
    assert_ne!(row.agent_session_key, None);
    assert_eq!(
        fake.last_spawn_spec().unwrap().initial_size,
        Some((180, 50))
    );
    let warning = warning.expect("the successful restart must surface its notification failure");
    assert_eq!(warning["kind"], "slot_restart_notification_failed");
    assert!(warning["message"]
        .as_str()
        .unwrap()
        .contains("restarted, but its restart notification could not be recorded"));
    assert!(updated);
}
