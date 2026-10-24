use super::*;

#[test]
fn codex_speed_follows_role_args_for_direct_and_mission_spawns() {
    use crate::model::CodexSpeed;
    let root = tempfile::tempdir().unwrap();
    for mission in [false, true] {
        for key in [None, Some("11111111-1111-4111-8111-111111111111")] {
            for (speed, tier) in [
                (None, None),
                (Some(CodexSpeed::Standard), Some("default")),
                (Some(CodexSpeed::Fast), Some("fast")),
            ] {
                let mut role = role("codex", &["-c", "service_tier=manual"]);
                role.runtime = "codex".into();
                role.codex_speed = speed;
                let mut spec = SpawnSpec {
                    agent_runtime: None,
                    pending_turn: None,
                    session_id: "speed-spawn".into(),
                    cwd: None,
                    command: role.command.clone(),
                    args: role.args.clone(),
                    env: BTreeMap::new(),
                    mission,
                    shim_dir: None,
                    bundled_bin_dir: None,
                    shell_path: None,
                    initial_size: None,
                };
                mgr_with_fake(None, fake_runtime()).apply_runtime_args(
                    &mut spec,
                    &role,
                    &crate::runtimes::adapter(Runtime::Codex).resume_plan(key),
                    root.path(),
                    None,
                    Some("first turn"),
                    None,
                    None,
                );
                let tiers = spec
                    .args
                    .windows(2)
                    .filter_map(|pair| {
                        (pair[0] == "-c")
                            .then(|| pair[1].strip_prefix("service_tier="))
                            .flatten()
                    })
                    .collect::<Vec<_>>();
                match tier {
                    Some(tier) => assert_eq!(tiers, ["manual", tier]),
                    None => assert_eq!(tiers, ["manual"]),
                }
                if let Some(key) = key {
                    assert_eq!(&spec.args[..2], &["resume", key]);
                    assert!(!spec.args.contains(&"first turn".to_string()));
                } else {
                    assert_eq!(spec.args.last().map(String::as_str), Some("first turn"));
                }
            }
        }
    }
}

#[test]
fn codex_spawn_composes_hooks_without_changing_user_home_and_respects_overrides() {
    use runner_core::protocol::hook::{GENERATION_ENV, SESSION_ENV};
    let root = tempfile::tempdir().unwrap();
    for args in [
        vec![],
        vec!["--config", "hooks.Stop=[]"],
        vec!["--disable", "hooks"],
    ] {
        let mut role = role("codex", &args);
        role.runtime = "codex".into();
        let mut generations = Vec::new();
        for key in [None, Some("11111111-1111-4111-8111-111111111111")] {
            let mut spec = SpawnSpec {
                agent_runtime: None,
                pending_turn: None,
                session_id: "codex-spawn".into(),
                cwd: None,
                command: role.command.clone(),
                args: role.args.clone(),
                env: BTreeMap::from([("CODEX_HOME".into(), "user home".into())]),
                mission: false,
                shim_dir: None,
                bundled_bin_dir: None,
                shell_path: None,
                initial_size: None,
            };
            mgr_with_fake(None, fake_runtime()).apply_runtime_args(
                &mut spec,
                &role,
                &crate::runtimes::adapter(Runtime::Codex).resume_plan(key),
                root.path(),
                None,
                Some("first turn"),
                None,
                None,
            );
            assert_eq!(spec.env["CODEX_HOME"], "user home");
            assert_eq!(spec.pending_turn, Some(key.is_none()));
            let injected = args.is_empty();
            assert_eq!(spec.env.contains_key(SESSION_ENV), injected);
            assert_eq!(spec.env.contains_key(GENERATION_ENV), injected);
            assert_eq!(
                spec.args
                    .iter()
                    .any(|arg| arg.starts_with("hooks.UserPromptSubmit=")),
                injected
            );
            assert!(!spec.env.contains_key("RUNNER_CLAUDE_STATUS_PATH"));
            if injected {
                let generation = spec.env[GENERATION_ENV].clone();
                assert!(uuid::Uuid::parse_str(&generation).is_ok());
                generations.push(generation);
                assert_eq!(spec.env[SESSION_ENV], "codex-spawn");
            }
            assert_eq!(
                spec.args.iter().any(|arg| arg == "first turn"),
                key.is_none()
            );
            for arg in &role.args {
                assert!(spec.args.contains(arg));
            }
        }
        if generations.len() == 2 {
            assert_ne!(generations[0], generations[1]);
        }
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn codex_observations_preserve_delivery_and_drafts_and_interrupt_attention() {
    use crate::runtimes::codex::codex_status::CodexStatusWatcher;
    let core = crate::test_support::test_core();
    core.db.get().unwrap().execute("INSERT INTO sessions(id, status, agent_runtime) VALUES ('codex-status', 'running', 'codex')", []).unwrap();
    install_test_session_handle(&core.sessions, "codex-status");
    core.sessions.note_forwarder_transition(
        "codex-status",
        SessionActivityState::Busy,
        crate::session::state::StatusSource::Forwarder,
    );
    let (hooks, receiver) =
        crate::session::hook_queue::TestHookRoute::new(Runtime::Codex, "current".into());
    let mut watcher = CodexStatusWatcher::from_receiver(receiver);
    for event in [
        "UserPromptSubmit",
        "PreToolUse",
        "PermissionRequest",
        "Interrupt",
        "PostToolUse",
        "Stop",
    ] {
        hooks.admit(serde_json::json!({"generation":"current","hook_event_name":event,"session_id":"main","turn_id":"one","tool_name":"request_user_input","tool_use_id":"question"})).unwrap();
    }
    let events = core.session_events();
    let mut count = 0;
    watcher
        .drain_events(
            0,
            |event| {
                let before = core.sessions.agent_status("codex-status");
                let feedback =
                    core.sessions
                        .publish_agent_event("codex-status", event.clone(), &events);
                if core.sessions.agent_status("codex-status") == before {
                    return feedback;
                }
                count += 1;
                let token = match core.sessions.reserve_delivery("codex-status").unwrap() {
                    router::DeliveryReservation::Ready(token) => token,
                    other => panic!("{other:?}"),
                };
                core.sessions.finish_delivery("codex-status", token);
                let state = core.sessions.session_state("codex-status").unwrap();
                state.lock().unwrap().model.apply(
                    SessionEvent::TestPending(true),
                    crate::session::clock::state_now(),
                );
                core.sessions
                    .publish_agent_event("codex-status", event, &events);
                assert_eq!(
                    core.sessions.reserve_delivery("codex-status").unwrap(),
                    router::DeliveryReservation::LocalInputPending
                );
                assert!(state.lock().unwrap().model.local_input_pending());
                state.lock().unwrap().model.apply(
                    SessionEvent::TestPending(false),
                    crate::session::clock::state_now(),
                );
                feedback
            },
            |_| {},
        )
        .unwrap();
    assert_eq!(count, 3);
    assert_eq!(
        core.sessions
            .agent_status("codex-status")
            .observation
            .outcome,
        Some(TurnOutcome::Interrupted)
    );
    assert!(core
        .sessions
        .agent_status("codex-status")
        .unread_since
        .is_none());
    assert!(!core
        .sessions
        .take_completion_armed(&["codex-status".into()]));
    core.sessions.status_bridge_failed("codex-status", &events);
    assert_eq!(
        core.sessions
            .agent_status("codex-status")
            .observation
            .source,
        ObservationSource::Baseline
    );
    assert!(core
        .sessions
        .agent_status("codex-status")
        .unread_since
        .is_none());
}

#[cfg(unix)]
fn codex_redraw_role(root: &Path) -> Role {
    use std::os::unix::fs::PermissionsExt;
    let frame: serde_json::Value =
        serde_json::from_str(include_str!("../../fixtures/codex-0.155.1-idle.json")).unwrap();
    std::fs::write(root.join("readiness"), frame["readiness"].as_str().unwrap()).unwrap();
    std::fs::write(root.join("redraw"), frame["redraw"].as_str().unwrap()).unwrap();
    let command = root.join("codex-fixture");
    std::fs::write(
        &command,
        r#"#!/bin/sh
stty raw -echo
printf '%s\n' "$@" > "$FIXTURE_ROOT/args"
printf '%s' "$RUNNER_HOOK_GENERATION" > "$FIXTURE_ROOT/generation"
cat "$FIXTURE_ROOT/readiness"
i=0
while [ "$i" -lt 200 ]; do
  if [ ! -f "$FIXTURE_ROOT/quiet" ]; then cat "$FIXTURE_ROOT/redraw"; fi
  sleep 0.08
  i=$((i + 1))
done
"#,
    )
    .unwrap();
    std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut role = role(command.to_str().unwrap(), &[]);
    role.runtime = "codex".into();
    role.env.insert(
        "CODEX_HOME".into(),
        root.join("home").to_string_lossy().into(),
    );
    role.env
        .insert("FIXTURE_ROOT".into(), root.to_string_lossy().into());
    role
}

/// The hook generation the fixture recorded as it started. A baseline Idle
/// comes from two seconds of silence and can land before the shell has run
/// at all on a loaded machine, so wait for the fixture rather than read.
#[cfg(unix)]
fn fixture_generation(root: &Path) -> String {
    let path = root.join("generation");
    let deadline = Instant::now() + ci_scaled_budget(Duration::from_secs(10));
    loop {
        if let Some(generation) = std::fs::read_to_string(&path)
            .ok()
            .filter(|generation| !generation.is_empty())
        {
            return generation;
        }
        assert!(
            Instant::now() < deadline,
            "the codex fixture never recorded its hook generation"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
fn codex_fixture_report(
    root: &Path,
    id: &str,
    event: &str,
    turn: &str,
) -> runner_core::protocol::hook::HookReport {
    runner_core::protocol::hook::HookReport {
        bridge_unavailable: false,
        version: runner_core::protocol::hook::VERSION,
        runtime: Runtime::Codex,
        session_id: id.into(),
        generation: fixture_generation(root),
        event: event.into(),
        caller_thread_id: Some("fixture-conversation".into()),
        payload: serde_json::json!({
            "session_id": "fixture-conversation", "turn_id": turn,
            "hook_event_name": event, "source": "startup"
        }),
    }
}

#[cfg(unix)]
fn admit_codex_fixture_hook(
    manager: &SessionManager,
    root: &Path,
    id: &str,
    event: &str,
    turn: &str,
) {
    drop(
        manager
            .admit_hook(codex_fixture_report(root, id, event, turn))
            .unwrap(),
    );
}

#[cfg(unix)]
fn wait_for_observation(
    manager: &SessionManager,
    id: &str,
    activity: Activity,
    source: ObservationSource,
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let observation = manager.agent_status(id).observation;
        if observation.activity == activity && observation.source == source {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "expected {activity:?}/{source:?}, got {observation:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(unix)]
#[test]
fn codex_pre_hook_startup_ignores_continuing_idle_redraw() {
    for launch in ["fresh", "unkeyed-direct", "keyed-mission"] {
        let app_data = tempfile::tempdir().unwrap();
        let role = codex_redraw_role(app_data.path());
        let pool = pool_with_schema();
        let manager = manager_with_runtime(
            Default::default(),
            Arc::new(crate::session::pty_runtime::PtyRuntime::new()),
        );
        let events = capture_for(&manager);
        let spawned = if launch == "keyed-mission" {
            let (mission, slot) = seed_mission_rows(&pool, &role);
            let mut row = crate::repo::session::SessionRowDb::new_running("resumed-slot".into());
            row.status = crate::model::SessionStatus::Stopped;
            row.mission_id = Some(mission.id);
            row.slot_id = Some(slot.id);
            row.role_id = Some(role.id.clone());
            row.cwd = Some(app_data.path().to_string_lossy().into());
            row.agent_session_key = Some(uuid::Uuid::new_v4().to_string());
            crate::test_support::insert_session_row(&pool.get().unwrap(), &row).unwrap();
            manager
                .resume(
                    &row.id,
                    None,
                    None,
                    app_data.path(),
                    pool.clone(),
                    events.clone(),
                )
                .unwrap()
        } else {
            insert_crew_role(&pool, "pre-hook", &role.id);
            update_role_row(&pool.get().unwrap(), &role);
            let spawned = manager
                .spawn_direct(
                    &role,
                    None,
                    None,
                    None,
                    None,
                    Some(app_data.path().to_str().unwrap()),
                    None,
                    None,
                    app_data.path(),
                    pool.clone(),
                    events.clone(),
                    None,
                )
                .unwrap();
            if launch == "unkeyed-direct" {
                wait_for_output_event(&events, &spawned.id);
                manager.kill(&spawned.id).unwrap();
                events.output.lock().unwrap().clear();
                manager
                    .resume(
                        &spawned.id,
                        None,
                        None,
                        app_data.path(),
                        pool.clone(),
                        events.clone(),
                    )
                    .unwrap()
            } else {
                spawned
            }
        };
        wait_for_output_event(&events, &spawned.id);
        thread::sleep(Duration::from_millis(2300));
        let observation = manager.agent_status(&spawned.id).observation;
        assert!(
            !app_data
                .path()
                .join("session-status")
                .join(format!("{}.ndjson", spawned.id))
                .exists(),
            "obsolete hook feed must not be created: {launch}"
        );
        let armed = manager
            .session_state(&spawned.id)
            .unwrap()
            .lock()
            .unwrap()
            .model
            .hook_status_armed();
        let args = std::fs::read_to_string(app_data.path().join("args")).unwrap();
        manager.kill(&spawned.id).unwrap();
        assert!(!armed, "{launch}");
        assert_eq!(observation.source, ObservationSource::Baseline, "{launch}");
        assert_eq!(observation.activity, Activity::Idle, "{launch}");
        assert_eq!(observation.outcome, None, "{launch}");
        assert_eq!(args.starts_with("resume\n"), launch == "keyed-mission");
        assert!(
            events.output.lock().unwrap().len() > 10,
            "redraw must continue: {launch}"
        );
    }
}

#[cfg(unix)]
#[test]
fn codex_pre_hook_submissions_and_hook_takeover() {
    for input in ["user", "router", "paste"] {
        let app_data = tempfile::tempdir().unwrap();
        let role = codex_redraw_role(app_data.path());
        let pool = pool_with_schema();
        insert_crew_role(&pool, "pre-hook", &role.id);
        let events = capture();
        let manager = manager_with_runtime(
            Default::default(),
            Arc::new(crate::session::pty_runtime::PtyRuntime::new()),
        );
        let spawned = manager
            .spawn_direct(
                &role,
                None,
                None,
                None,
                None,
                Some(app_data.path().to_str().unwrap()),
                None,
                None,
                app_data.path(),
                pool.clone(),
                events.clone(),
                None,
            )
            .unwrap();
        let id = &spawned.id;
        wait_for_observation(&manager, id, Activity::Idle, ObservationSource::Baseline);
        manager
            .inject_direct_stdin(id, b"\r", events.as_ref())
            .unwrap();
        wait_for_observation(&manager, id, Activity::Idle, ObservationSource::Baseline);
        match input {
            "user" => {
                manager
                    .inject_direct_stdin(id, b"draft", events.as_ref())
                    .unwrap();
                thread::sleep(Duration::from_millis(150));
                assert_eq!(
                    manager.agent_status(id).observation.activity,
                    Activity::Idle
                );
                assert!(matches!(
                    manager.reserve_delivery(id).unwrap(),
                    router::DeliveryReservation::RecentlyTyping(_)
                ));
                manager
                    .inject_direct_stdin(id, b"\r", events.as_ref())
                    .unwrap();
            }
            "router" => {
                let deadline = Instant::now() + Duration::from_secs(3);
                let token = loop {
                    match manager.reserve_delivery(id).unwrap() {
                        router::DeliveryReservation::Ready(token) => break token,
                        router::DeliveryReservation::RecentlyTyping(_)
                            if Instant::now() < deadline =>
                        {
                            thread::sleep(Duration::from_millis(10))
                        }
                        other => panic!("{other:?}"),
                    }
                };
                assert!(manager.inject_reserved(id, token, b"routed work").unwrap());
                assert!(manager.inject_reserved(id, token, b"\r").unwrap());
                manager.finish_delivery(id, token);
            }
            _ => manager.inject_paste(id, b"automatic paste").unwrap(),
        }
        wait_for_observation(&manager, id, Activity::Working, ObservationSource::Baseline);
        admit_codex_fixture_hook(&manager, app_data.path(), id, "SessionStart", "one");
        std::fs::write(app_data.path().join("quiet"), "").unwrap();
        thread::sleep(Duration::from_millis(150));
        assert_eq!(
            manager.agent_status(id).observation.activity,
            Activity::Working
        );
        assert_eq!(
            manager.agent_status(id).observation.source,
            ObservationSource::Baseline
        );
        admit_codex_fixture_hook(&manager, app_data.path(), id, "UserPromptSubmit", "one");
        wait_for_observation(&manager, id, Activity::Working, ObservationSource::Hook);
        thread::sleep(Duration::from_millis(2300));
        assert_eq!(
            manager.agent_status(id).observation.activity,
            Activity::Working
        );
        std::fs::remove_file(app_data.path().join("quiet")).unwrap();
        admit_codex_fixture_hook(&manager, app_data.path(), id, "Stop", "one");
        wait_for_observation(&manager, id, Activity::Ready, ObservationSource::Hook);
        assert_eq!(
            manager.agent_status(id).observation.outcome,
            Some(TurnOutcome::Completed)
        );
        manager
            .inject_direct_stdin(id, b"\r", events.as_ref())
            .unwrap();
        admit_codex_fixture_hook(&manager, app_data.path(), id, "UserPromptSubmit", "two");
        wait_for_observation(&manager, id, Activity::Working, ObservationSource::Hook);
        admit_codex_fixture_hook(&manager, app_data.path(), id, "Interrupt", "two");
        wait_for_observation(&manager, id, Activity::Unavailable, ObservationSource::Hook);
        assert_eq!(
            manager.agent_status(id).observation.outcome,
            Some(TurnOutcome::Interrupted)
        );
        manager.kill(id).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn codex_pre_hook_early_escape_accepts_interrupt_as_first_hook() {
    let app_data = tempfile::tempdir().unwrap();
    let role = codex_redraw_role(app_data.path());
    let pool = pool_with_schema();
    insert_crew_role(&pool, "early-escape", &role.id);
    let events = capture();
    let manager = manager_with_runtime(
        Default::default(),
        Arc::new(crate::session::pty_runtime::PtyRuntime::new()),
    );
    let spawned = manager
        .spawn_direct(
            &role,
            None,
            None,
            None,
            None,
            Some(app_data.path().to_str().unwrap()),
            None,
            None,
            app_data.path(),
            pool,
            events.clone(),
            None,
        )
        .unwrap();
    let id = &spawned.id;
    wait_for_observation(&manager, id, Activity::Idle, ObservationSource::Baseline);
    manager
        .inject_direct_stdin(id, b"work", events.as_ref())
        .unwrap();
    manager
        .inject_direct_stdin(id, b"\r", events.as_ref())
        .unwrap();
    wait_for_observation(&manager, id, Activity::Working, ObservationSource::Baseline);
    manager
        .inject_direct_stdin(id, b"\x1b", events.as_ref())
        .unwrap();
    let transcript = app_data.path().join("rollout.jsonl");
    std::fs::write(&transcript, "").unwrap();
    assert!(!app_data
        .path()
        .join("session-status")
        .join(format!("{id}.ndjson"))
        .exists());
    let mut report = codex_fixture_report(app_data.path(), id, "Interrupt", "one");
    report.payload["transcript_path"] = serde_json::json!(transcript);
    drop(manager.admit_hook(report).unwrap());
    wait_for_observation(&manager, id, Activity::Unavailable, ObservationSource::Hook);
    assert_eq!(
        manager.agent_status(id).observation.outcome,
        Some(TurnOutcome::Interrupted)
    );
    std::fs::write(
        transcript,
        concat!(
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"turn_aborted\",",
            "\"turn_id\":\"one\",\"reason\":\"interrupted\"}}\n"
        ),
    )
    .unwrap();
    wait_for_observation(&manager, id, Activity::Ready, ObservationSource::Hook);
    thread::sleep(Duration::from_millis(150));
    let observation = manager.agent_status(id).observation;
    assert_eq!(observation.activity, Activity::Ready);
    assert_eq!(observation.outcome, Some(TurnOutcome::Interrupted));
    admit_codex_fixture_hook(&manager, app_data.path(), id, "UserPromptSubmit", "two");
    wait_for_observation(&manager, id, Activity::Working, ObservationSource::Hook);
    manager.kill(id).unwrap();
}

#[cfg(unix)]
#[test]
fn codex_pre_hook_automatic_mission_turn_stays_working() {
    for lead in [false, true] {
        let app_data = tempfile::tempdir().unwrap();
        let role = codex_redraw_role(app_data.path());
        let pool = pool_with_schema();
        let (mission, mut slot) = seed_mission_rows(&pool, &role);
        slot.lead = lead;
        let manager = manager_with_runtime(
            Default::default(),
            Arc::new(crate::session::pty_runtime::PtyRuntime::new()),
        );
        let events = capture_for(&manager);
        let spawned = manager
            .spawn(
                &mission,
                &role,
                &slot,
                app_data.path(),
                app_data.path().join("events.ndjson"),
                pool.clone(),
                events.clone(),
                Some("automatic first turn".into()),
            )
            .unwrap();
        wait_for_output_event(&events, &spawned.id);
        std::fs::write(app_data.path().join("quiet"), "").unwrap();
        admit_codex_fixture_hook(
            &manager,
            app_data.path(),
            &spawned.id,
            "SessionStart",
            "one",
        );
        thread::sleep(Duration::from_millis(2300));
        let observation = manager.agent_status(&spawned.id).observation;
        let args = std::fs::read_to_string(app_data.path().join("args")).unwrap();
        admit_codex_fixture_hook(
            &manager,
            app_data.path(),
            &spawned.id,
            "UserPromptSubmit",
            "one",
        );
        wait_for_observation(
            &manager,
            &spawned.id,
            Activity::Working,
            ObservationSource::Hook,
        );
        admit_codex_fixture_hook(&manager, app_data.path(), &spawned.id, "Stop", "one");
        wait_for_observation(
            &manager,
            &spawned.id,
            Activity::Ready,
            ObservationSource::Hook,
        );
        let outcome = manager.agent_status(&spawned.id).observation.outcome;
        manager.kill(&spawned.id).unwrap();
        assert!(args.contains("automatic first turn"));
        assert_eq!(observation.activity, Activity::Working);
        assert_eq!(observation.source, ObservationSource::Baseline);
        assert_eq!(outcome, Some(TurnOutcome::Completed));
    }
}

#[cfg(unix)]
#[test]
fn codex_pre_hook_native_commands_and_failed_bridges_use_output_fallback() {
    for mode in ["native-command", "disabled", "failed"] {
        let app_data = tempfile::tempdir().unwrap();
        let mut role = codex_redraw_role(app_data.path());
        if mode == "disabled" {
            role.args = vec!["--disable".into(), "hooks".into()];
        }
        let pool = pool_with_schema();
        insert_crew_role(&pool, "pre-hook", &role.id);
        let events = capture();
        let manager = manager_with_runtime(
            Default::default(),
            Arc::new(crate::session::pty_runtime::PtyRuntime::new()),
        );
        let spawned = manager
            .spawn_direct(
                &role,
                None,
                None,
                None,
                None,
                Some(app_data.path().to_str().unwrap()),
                None,
                None,
                app_data.path(),
                pool,
                events.clone(),
                None,
            )
            .unwrap();
        let id = &spawned.id;
        wait_for_observation(&manager, id, Activity::Idle, ObservationSource::Baseline);
        manager
            .inject_direct_stdin(
                id,
                if mode == "native-command" {
                    b"/status"
                } else {
                    b"work"
                },
                events.as_ref(),
            )
            .unwrap();
        manager
            .inject_direct_stdin(id, b"\r", events.as_ref())
            .unwrap();
        wait_for_observation(&manager, id, Activity::Working, ObservationSource::Baseline);
        if mode == "failed" {
            let mut report = codex_fixture_report(app_data.path(), id, "bridge_unavailable", "one");
            report.bridge_unavailable = true;
            drop(manager.admit_hook(report).unwrap());
        }
        std::fs::write(app_data.path().join("quiet"), "").unwrap();
        wait_for_observation(&manager, id, Activity::Idle, ObservationSource::Baseline);
        let observation = manager.agent_status(id).observation;
        let armed = manager
            .session_state(id)
            .unwrap()
            .lock()
            .unwrap()
            .model
            .hook_status_armed();
        manager.kill(id).unwrap();
        assert_eq!(observation.outcome, None);
        assert!(!armed);
    }
}

#[cfg(windows)]
#[test]
fn codex_windows_batch_pending_prompt_is_recorded_before_argv_suppression() {
    let root = tempfile::tempdir().unwrap();
    let mut role = role("codex.cmd", &[]);
    role.runtime = "codex".into();
    let mut spec = SpawnSpec {
        command: role.command.clone(),
        ..Default::default()
    };
    mgr_with_fake(None, fake_runtime()).apply_runtime_args(
        &mut spec,
        &role,
        &crate::runtimes::adapter(Runtime::Codex).resume_plan(None),
        root.path(),
        None,
        Some("automatic prompt"),
        None,
        None,
    );
    assert_eq!(spec.pending_turn, Some(true));
    assert!(!spec.args.iter().any(|arg| arg == "automatic prompt"));
}

#[test]
fn codex_native_attention_publication_holds_delivery_and_preserves_drafts() {
    use crate::runtimes::TerminalEvent;
    use crate::session::state::agent::AgentEvent;
    #[derive(Default)]
    struct DeliveryEvents(Mutex<Vec<router::SessionDeliveryEvent>>);
    impl router::SessionDeliveryListener for DeliveryEvents {
        fn session_delivery_event(&self, _: &str, event: router::SessionDeliveryEvent) {
            self.0.lock().unwrap().push(event);
        }
    }
    for mission in [false, true] {
        let manager = mgr_with_fake(None, fake_runtime());
        let id = "native-codex";
        install_test_session_handle(&manager, id);
        let root = tempfile::tempdir().unwrap();
        let event_log = Arc::new(EventLog::open(root.path()).unwrap());
        if mission {
            manager
                .session_state(id)
                .unwrap()
                .lock()
                .unwrap()
                .mission_status_sink = Some(ForwarderEmitCtx {
                crew_id: Some("crew".into()),
                mission_id: "mission".into(),
                handle: "codex".into(),
                event_log: event_log.clone(),
            });
        }
        let delivered = Arc::new(DeliveryEvents::default());
        let listener: Arc<dyn router::SessionDeliveryListener> = delivered.clone();
        manager.register_delivery_listener(id, Arc::downgrade(&listener));
        let events = capture();
        manager.publish_agent_event(
            id,
            AgentEvent::Batch {
                runtime: Runtime::Codex,
                events: vec![AgentEvent::TurnStarted],
            },
            events.as_ref(),
        );
        let publish = |event| {
            if mission {
                manager.publish_mission_terminal_event(id, event, events.as_ref());
            } else {
                manager.publish_direct_terminal_event(id, event, events.as_ref());
            }
        };
        let mut terminal = crate::runtimes::adapter(Runtime::Codex)
            .terminal_adapter(Some(false))
            .unwrap();
        terminal.accept_event(&AgentEvent::TurnStarted);
        terminal.on_output(b"\x1b]7501;state=blocked:app=codex\x1b\\");
        let blocked = TerminalEvent::CodexAttention(terminal.take_codex_attention().unwrap());
        assert_eq!(blocked, TerminalEvent::CodexAttention(true));
        publish(blocked);
        let wait = manager.agent_status(id).observation.interactions[0].clone();
        let count = events.status.lock().unwrap().len();
        publish(blocked);
        assert_eq!(events.status.lock().unwrap().len(), count);
        assert_eq!(
            manager.reserve_delivery(id).unwrap(),
            router::DeliveryReservation::HumanInteraction
        );
        manager.mark_status_viewed(&[id.into()]);
        assert!(manager.status_snapshot()[id].observation.needs_you());
        let state = manager.session_state(id).unwrap();
        state.lock().unwrap().model.apply(
            SessionEvent::TestPending(true),
            crate::session::clock::state_now(),
        );
        manager.publish_agent_event(
            id,
            AgentEvent::Batch {
                runtime: Runtime::Codex,
                events: vec![AgentEvent::Working {
                    detail: Some(crate::session::status::WorkDetail::UsingTools),
                }],
            },
            events.as_ref(),
        );
        assert_eq!(
            manager.agent_status(id).observation.interactions,
            vec![wait]
        );
        for report in [
            b"\x1b]7501;state=working:app=codex\x07".as_slice(),
            b"\x1b]7501;state=idle:app=codex\x07",
            b"\x1b]7501;state=clear\x1b\\",
        ] {
            terminal.on_output(report);
            publish(TerminalEvent::CodexAttention(
                terminal.take_codex_attention().unwrap(),
            ));
            assert!(!manager.agent_status(id).observation.needs_you());
            assert_eq!(
                manager.agent_status(id).observation.activity,
                Activity::Working
            );
            assert_eq!(manager.agent_status(id).observation.outcome, None);
            assert_eq!(manager.agent_status(id).unread_since, None);
            assert_eq!(
                manager.reserve_delivery(id).unwrap(),
                router::DeliveryReservation::LocalInputPending
            );
            publish(blocked);
        }
        publish(TerminalEvent::CodexAttention(false));
        assert!(delivered
            .0
            .lock()
            .unwrap()
            .contains(&router::SessionDeliveryEvent::ReadinessChanged));
        state.lock().unwrap().model.apply(
            SessionEvent::TestPending(false),
            crate::session::clock::state_now(),
        );
        assert!(matches!(
            manager.reserve_delivery(id).unwrap(),
            router::DeliveryReservation::Ready(_)
        ));
        if mission {
            let rows = event_log.read_from(0).unwrap();
            let statuses = events.status.lock().unwrap();
            assert_eq!(rows.len(), statuses.len());
            for (row, status) in rows.iter().zip(statuses.iter()) {
                assert_eq!(
                    row.event.payload["status"],
                    serde_json::to_value(&status.status).unwrap()
                );
            }
            assert!(rows
                .iter()
                .any(
                    |row| row.event.payload["status"]["observation"]["interactions"][0]["reason"]
                        == "unknown"
                ));
        }
    }
}

#[test]
fn codex_native_clear_never_runs_completion_callback_even_when_armed() {
    let core = crate::test_support::test_core();
    core.db.get().unwrap().execute("INSERT INTO sessions(id, status, agent_runtime) VALUES ('native-attention', 'running', 'codex')", []).unwrap();
    install_test_session_handle(&core.sessions, "native-attention");
    let events = core.session_events();
    core.sessions.publish_direct_activity(
        "native-attention",
        SessionActivityState::Idle,
        StatusSource::Forwarder,
        &events,
    );
    let state = core.sessions.session_state("native-attention").unwrap();
    state.lock().unwrap().model.apply(
        SessionEvent::ArmCompletion,
        crate::session::clock::state_now(),
    );
    for blocked in [true, true, false] {
        core.sessions.publish_direct_terminal_event(
            "native-attention",
            crate::runtimes::TerminalEvent::CodexAttention(blocked),
            &events,
        );
    }
    let status = core.sessions.agent_status("native-attention");
    assert_eq!(status.unread_since, None);
    assert_eq!(status.observation.outcome, None);
    assert!(
        state.lock().unwrap().model.completion_armed(),
        "native attention must not consume hook completion"
    );
    assert!(!crate::repo::session_attention::any_unread(
        &core.db.get().unwrap(),
        &["native-attention".into()]
    )
    .unwrap());
}

#[test]
fn codex_native_forwarder_publishes_without_a_mounted_client_for_chats_and_missions() {
    use crate::runtimes::TerminalEvent;
    use crate::session::state::agent::AgentEvent as A;
    for mission in [false, true] {
        let fake = fake_runtime();
        let manager = mgr_with_fake(None, fake.clone());
        let id = "native-forwarder";
        let (runtime, output) = fake
            .spawn(SpawnSpec {
                session_id: id.into(),
                ..Default::default()
            })
            .unwrap();
        install_test_session_handle(&manager, id);
        let root = tempfile::tempdir().unwrap();
        let log = Arc::new(EventLog::open(root.path()).unwrap());
        let sink = mission.then(|| ForwarderEmitCtx {
            crew_id: Some("crew".into()),
            mission_id: "mission".into(),
            handle: "codex".into(),
            event_log: log.clone(),
        });
        {
            let state = manager.session_state(id).unwrap();
            let mut state = state.lock().unwrap();
            state.handle.as_mut().unwrap().stop = output.stop_flag();
            state.mission_status_sink = sink.clone();
            assert!(state.subscribers.is_empty());
        }
        let batch = |event| RuntimeOutput::AgentEvent {
            event: A::Batch {
                runtime: Runtime::Codex,
                events: vec![event],
            },
            feedback: None,
        };
        {
            let spawns = fake.spawns.lock().unwrap();
            let tx = spawns[0].tx.as_ref().unwrap();
            for item in [
                batch(A::TurnStarted),
                RuntimeOutput::TerminalEvent(TerminalEvent::CodexAttention(true)),
                RuntimeOutput::TerminalEvent(TerminalEvent::CodexAttention(true)),
                batch(A::Working {
                    detail: Some(crate::session::status::WorkDetail::UsingTools),
                }),
                RuntimeOutput::TerminalEvent(TerminalEvent::CodexAttention(false)),
                batch(A::TurnEnded {
                    outcome: TurnOutcome::Interrupted,
                }),
                RuntimeOutput::TerminalEvent(TerminalEvent::CodexAttention(true)),
                RuntimeOutput::TerminalEvent(TerminalEvent::CodexAttention(false)),
            ] {
                tx.send(item).unwrap();
            }
        }
        fake.close_spawn(0);
        let events = capture();
        manager
            .start_forwarder_thread(
                id.into(),
                String::new(),
                mission.then(|| "mission".into()),
                runtime,
                output,
                pool_with_schema(),
                events.clone(),
                role("fake", &[]),
                false,
                false,
                sink,
                root.path().to_path_buf(),
            )
            .join()
            .unwrap();
        let statuses = events.status.lock().unwrap();
        assert_eq!(statuses.len(), 7);
        assert!(statuses[1].status.observation.needs_you());
        assert_eq!(
            statuses[1].status.observation.interactions,
            statuses[2].status.observation.interactions
        );
        assert_eq!(statuses[3].state, SessionActivityState::Busy);
        assert!(!statuses[3].status.observation.needs_you());
        assert_eq!(statuses[3].status.observation.outcome, None);
        assert_eq!(
            statuses[6].status.observation.outcome,
            Some(TurnOutcome::Interrupted)
        );
        assert!(statuses
            .iter()
            .all(|status| status.status.unread_since.is_none()));
        if mission {
            let rows = log.read_from(0).unwrap();
            assert_eq!(rows.len(), statuses.len());
            for (row, status) in rows.iter().zip(statuses.iter()) {
                assert_eq!(
                    row.event.payload["status"],
                    serde_json::to_value(&status.status).unwrap()
                );
                assert_eq!(
                    row.event.payload["state"],
                    serde_json::to_value(status.state).unwrap()
                );
            }
        }
    }
}

#[test]
fn codex_native_attention_ignores_reports_from_an_older_launch() {
    let manager = mgr_with_fake(None, fake_runtime());
    let id = "native-generation";
    install_test_session_handle(&manager, id);
    let events = capture();
    let older = Arc::new(AtomicBool::new(false));
    for blocked in [true, false] {
        manager.publish_model_event_for_launch(
            id,
            SessionEvent::CodexAttention {
                blocked,
                live: true,
            },
            Some(&older),
            events.as_ref(),
        );
        assert!(events.status.lock().unwrap().is_empty());
    }
    manager.publish_direct_terminal_event(
        id,
        crate::runtimes::TerminalEvent::CodexAttention(true),
        events.as_ref(),
    );
    manager.publish_model_event_for_launch(
        id,
        SessionEvent::CodexAttention {
            blocked: false,
            live: true,
        },
        Some(&older),
        events.as_ref(),
    );
    assert!(manager.agent_status(id).observation.needs_you());
    manager.session_state(id).unwrap().lock().unwrap().killed = true;
    manager.publish_direct_terminal_event(
        id,
        crate::runtimes::TerminalEvent::CodexAttention(false),
        events.as_ref(),
    );
    assert!(manager.agent_status(id).observation.needs_you());
}

#[test]
fn codex_native_attention_preserves_real_hook_completion_in_both_clear_orderings() {
    use crate::runtimes::TerminalEvent;
    use crate::session::state::agent::AgentEvent as A;
    for clear_first in [false, true] {
        for outcome in [
            TurnOutcome::Completed,
            TurnOutcome::Failed,
            TurnOutcome::Interrupted,
        ] {
            let core = crate::test_support::test_core();
            core.db.get().unwrap().execute_batch("
                INSERT INTO sessions(id, status, agent_runtime) VALUES ('native-completion', 'running', 'codex');
                CREATE TABLE completion_writes (session_id TEXT);
                CREATE TRIGGER completion_insert AFTER INSERT ON session_attention WHEN NEW.unread_since IS NOT NULL BEGIN INSERT INTO completion_writes VALUES (NEW.session_id); END;
                CREATE TRIGGER completion_update AFTER UPDATE ON session_attention WHEN NEW.unread_since IS NOT NULL BEGIN INSERT INTO completion_writes VALUES (NEW.session_id); END;
            ").unwrap();
            let id = "native-completion";
            install_test_session_handle(&core.sessions, id);
            let events = core.session_events();
            let hook = |event| {
                core.sessions.publish_agent_event(
                    id,
                    A::Batch {
                        runtime: Runtime::Codex,
                        events: vec![event],
                    },
                    &events,
                );
            };
            let native = |blocked| {
                core.sessions.publish_direct_terminal_event(
                    id,
                    TerminalEvent::CodexAttention(blocked),
                    &events,
                )
            };
            let writes = || {
                core.db.get().unwrap().query_row("SELECT count(*) FROM completion_writes WHERE session_id = 'native-completion'", [], |row| row.get::<_, i64>(0)).unwrap()
            };
            hook(A::TurnStarted);
            native(true);
            native(true);
            if clear_first {
                native(false);
                native(false);
                assert_eq!(writes(), 0, "native clear alone is not a completion");
            }
            hook(A::TurnEnded { outcome });
            assert_eq!(writes(), i64::from(outcome == TurnOutcome::Completed));
            assert_eq!(
                core.sessions.agent_status(id).observation.needs_you(),
                !clear_first
            );
            let unread = core.sessions.agent_status(id).unread_since;
            assert_eq!(unread.is_some(), outcome == TurnOutcome::Completed);
            native(false);
            native(false);
            hook(A::TurnEnded { outcome });
            hook(A::ToolEnded {
                owner: None,
                count: 0,
                interrupted: false,
                transcript: false,
            });
            native(true);
            native(false);
            let status = core.sessions.agent_status(id);
            assert_eq!(status.unread_since, unread);
            assert_eq!(status.observation.outcome, Some(outcome));
            assert!(!status.observation.needs_you());
            assert_eq!(
                writes(),
                i64::from(outcome == TurnOutcome::Completed),
                "duplicates/late reports cannot record another response"
            );
            assert_eq!(
                crate::repo::session_attention::any_unread(&core.db.get().unwrap(), &[id.into()])
                    .unwrap(),
                outcome == TurnOutcome::Completed
            );
            assert!(!core.sessions.take_completion_armed(&[id.into()]));
        }
    }
}

#[test]
fn codex_native_only_output_does_not_fabricate_completion_but_mixed_output_keeps_activity() {
    use crate::session::pty_runtime::IdleDetector;
    for split_bytes in [false, true] {
        let core = crate::test_support::test_core();
        let id = "native-byte-evidence";
        core.db
            .get()
            .unwrap()
            .execute(
                "INSERT INTO sessions(id, status, agent_runtime) VALUES (?1, 'running', 'codex')",
                [id],
            )
            .unwrap();
        install_test_session_handle(&core.sessions, id);
        let events = core.session_events();
        let start = Instant::now();
        let mut detector = IdleDetector::with_adapter_at(
            Duration::from_secs(2),
            crate::runtimes::adapter(Runtime::Codex).terminal_adapter(Some(false)),
            start,
        );
        detector.hooks_unavailable();
        let publish = |event| {
            core.sessions
                .publish_direct_terminal_event(id, event, &events)
        };
        publish(
            detector
                .tick_event_at(start + Duration::from_secs(3))
                .unwrap(),
        );
        let state = core.sessions.session_state(id).unwrap();
        state.lock().unwrap().model.apply(
            SessionEvent::ArmCompletion,
            crate::session::clock::state_now(),
        );
        for (index, control) in [
            b"\x1b]7501;state=blocked:app=codex\x1b\\".as_slice(),
            b"\x1b]7501;state=blocked:app=codex\x07",
            b"\x1b]7501;state=clear\x1b\\",
            b"\x1b]7501;state=working:app=codex\x07",
            b"\x1b]7501;state=idle:app=codex\x1b\\",
            b"\x1b]7501;state=clear\x07",
        ]
        .iter()
        .enumerate()
        {
            let time = start + Duration::from_secs(4 + index as u64 * 4);
            let chunk_size = if split_bytes { 1 } else { control.len() };
            for chunk in control.chunks(chunk_size) {
                let transition = detector.on_output_event_at(chunk, false, time);
                for event in detector
                    .take_codex_attention()
                    .into_iter()
                    .chain(transition)
                {
                    publish(event);
                }
            }
            if let Some(event) = detector.tick_event_at(time + Duration::from_secs(3)) {
                publish(event);
            }
            let status = core.sessions.agent_status(id);
            assert_eq!(status.unread_since, None);
            assert_eq!(status.observation.outcome, None);
            assert!(state.lock().unwrap().model.completion_armed());
            assert!(!crate::repo::session_attention::any_unread(
                &core.db.get().unwrap(),
                &[id.into()]
            )
            .unwrap());
            let rows: i64 = core
                .db
                .get()
                .unwrap()
                .query_row(
                    "SELECT count(*) FROM session_attention WHERE session_id = ?1",
                    [id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                rows, 0,
                "native-only controls must not invoke completion bookkeeping"
            );
        }
        let mixed = b"\x1b]7501;state=working:app=codex\x07ordinary response";
        let time = start + Duration::from_secs(40);
        let chunk_size = if split_bytes { 1 } else { mixed.len() };
        for chunk in mixed.chunks(chunk_size) {
            let transition = detector.on_output_event_at(chunk, false, time);
            for event in detector
                .take_codex_attention()
                .into_iter()
                .chain(transition)
            {
                publish(event);
            }
        }
        assert_eq!(
            core.sessions.agent_status(id).observation.activity,
            Activity::Working
        );
        publish(
            detector
                .tick_event_at(time + Duration::from_secs(3))
                .unwrap(),
        );
        assert!(core.sessions.agent_status(id).unread_since.is_some());
        assert!(
            crate::repo::session_attention::any_unread(&core.db.get().unwrap(), &[id.into()])
                .unwrap()
        );
        assert!(!state.lock().unwrap().model.completion_armed());
    }
}
