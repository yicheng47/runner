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
                    env_remove: Vec::new(),
                    mission,
                    shim_dir: None,
                    bundled_bin_dir: None,
                    shell_path: None,
                    initial_size: None,
                    initial_cell_size: None,
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
                env_remove: Vec::new(),
                mission: false,
                shim_dir: None,
                bundled_bin_dir: None,
                shell_path: None,
                initial_size: None,
                initial_cell_size: None,
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
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("rollout.jsonl");
    std::fs::write(&transcript, "").unwrap();
    install_test_session_handle(&core.sessions, "codex-status");
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    let terminal = core
        .sessions
        .create_terminal("codex-status", 80, 24, &core.db, &events)
        .unwrap();
    {
        let state = core.sessions.session_state("codex-status").unwrap();
        let mut guard = state.lock().unwrap();
        guard.terminal = Some(terminal);
        guard.model.set_program_status_authority(true);
    }
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
        hooks
            .admit(serde_json::json!({
                "generation": "current",
                "hook_event_name": event,
                "session_id": "main",
                "turn_id": "one",
                "transcript_path": transcript,
            }))
            .unwrap();
    }
    let before_turn = core
        .sessions
        .delivery_snapshot("codex-status")
        .unwrap()
        .turn_started;
    let mut confirmed = 0;
    watcher
        .drain_events(
            0,
            |event| {
                confirmed += usize::from(matches!(
                    event,
                    crate::session::state::agent::AgentEvent::ProgramTurnConfirmed {
                        conversation_changed: false,
                        ..
                    }
                ));
                core.sessions
                    .publish_agent_event("codex-status", event, events.as_ref())
            },
            |_| {},
        )
        .unwrap();
    assert_eq!(confirmed, 1, "only UserPromptSubmit confirms the turn");
    assert_eq!(
        core.sessions
            .delivery_snapshot("codex-status")
            .unwrap()
            .turn_started,
        before_turn + 1
    );

    // OSC 7501 Working holds delivery and preserves LocalInputPending drafts.
    core.sessions.ingest_output_chunk(
        "codex-status",
        None,
        b"\x1b]7501;state=working:app=codex\x1b\\",
        events.as_ref(),
    );
    assert_eq!(
        core.sessions
            .delivery_snapshot("codex-status")
            .unwrap()
            .turn_started,
        before_turn + 1,
        "OSC 7501 Working must not double-count a turn UserPromptSubmit already confirmed"
    );
    assert_eq!(
        core.sessions.reserve_delivery("codex-status").unwrap(),
        router::DeliveryReservation::Unavailable
    );
    let state = core.sessions.session_state("codex-status").unwrap();
    state.lock().unwrap().model.apply(
        SessionEvent::TestPending(true),
        crate::session::clock::state_now(),
    );
    core.sessions.ingest_output_chunk(
        "codex-status",
        None,
        b"\x1b]7501;state=blocked:app=codex:kind=question\x1b\\",
        events.as_ref(),
    );
    assert_eq!(
        core.sessions.reserve_delivery("codex-status").unwrap(),
        router::DeliveryReservation::HumanInteraction
    );
    assert!(state.lock().unwrap().model.local_input_pending());

    // Transcript turn_aborted(interrupted) resolves to Idle/Interrupted without recording unread completion.
    std::fs::write(
        &transcript,
        "{\"type\":\"event_msg\",\"payload\":{\"type\":\"turn_aborted\",\"turn_id\":\"one\",\"reason\":\"interrupted\"}}\n",
    )
    .unwrap();
    watcher
        .drain_events(
            0,
            |event| {
                core.sessions
                    .publish_agent_event("codex-status", event, events.as_ref())
            },
            |_| {},
        )
        .unwrap();
    assert_eq!(
        core.sessions.reserve_delivery("codex-status").unwrap(),
        router::DeliveryReservation::LocalInputPending
    );
    state.lock().unwrap().model.apply(
        SessionEvent::TestPending(false),
        crate::session::clock::state_now(),
    );
    let router::DeliveryReservation::Ready(token) =
        core.sessions.reserve_delivery("codex-status").unwrap()
    else {
        panic!("expected Ready after interrupted turn clears");
    };
    core.sessions.finish_delivery("codex-status", token);
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
    core.sessions
        .status_bridge_failed("codex-status", events.as_ref());
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
                    DirectLaunch {
                        cwd: Some(app_data.path().to_str().unwrap().into()),
                        ..Default::default()
                    },
                    app_data.path(),
                    pool.clone(),
                    events.clone(),
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
                DirectLaunch {
                    cwd: Some(app_data.path().to_str().unwrap().into()),
                    ..Default::default()
                },
                app_data.path(),
                pool.clone(),
                events.clone(),
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
        let turn_before = manager.delivery_snapshot(id).unwrap().turn_started;
        admit_codex_fixture_hook(&manager, app_data.path(), id, "SessionStart", "one");
        std::fs::write(app_data.path().join("quiet"), "").unwrap();
        let transcript = app_data.path().join("rollout.jsonl");
        std::fs::write(&transcript, "").unwrap();
        let mut prompt = codex_fixture_report(app_data.path(), id, "UserPromptSubmit", "one");
        prompt.payload["transcript_path"] = serde_json::json!(transcript);
        drop(manager.admit_hook(prompt).unwrap());
        manager.wait_for_turn_started(id, turn_before);
        assert!(
            !manager
                .session_state(id)
                .unwrap()
                .lock()
                .unwrap()
                .model
                .hook_status_armed(),
            "Codex identity hooks must not arm hook status"
        );
        // OSC 7501 owns Working and Done while identity hooks stay active.
        let host_events: Arc<dyn SessionEvents> = events.clone();
        let terminal = manager
            .create_terminal(id, 80, 24, &pool, &host_events)
            .unwrap();
        manager.session_state(id).unwrap().lock().unwrap().terminal = Some(terminal);
        manager.ingest_output_chunk(
            id,
            None,
            b"\x1b]7501;state=working:app=codex\x1b\\",
            events.as_ref(),
        );
        assert_eq!(
            manager.agent_status(id).observation.activity,
            Activity::Working
        );
        assert!(manager
            .agent_status(id)
            .observation
            .program_status
            .is_some());
        std::fs::remove_file(app_data.path().join("quiet")).unwrap();
        manager.ingest_output_chunk(
            id,
            None,
            b"\x1b]7501;state=done:app=codex\x1b\\",
            events.as_ref(),
        );
        assert_eq!(
            manager.agent_status(id).observation.activity,
            Activity::Idle
        );
        assert_eq!(
            manager.agent_status(id).observation.outcome,
            Some(TurnOutcome::Completed)
        );
        manager
            .inject_direct_stdin(id, b"\r", events.as_ref())
            .unwrap();
        let turn_before_two = manager.delivery_snapshot(id).unwrap().turn_started;
        let mut second = codex_fixture_report(app_data.path(), id, "UserPromptSubmit", "two");
        second.payload["transcript_path"] = serde_json::json!(transcript);
        drop(manager.admit_hook(second).unwrap());
        // Synchronize on hook consumption before appending turn two's abort so the
        // live watcher has already switched its active turn_id from "one" to "two".
        manager.wait_for_turn_started(id, turn_before_two);
        manager.ingest_output_chunk(
            id,
            None,
            b"\x1b]7501;state=working:app=codex\x1b\\",
            events.as_ref(),
        );
        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&transcript)
                .unwrap();
            writeln!(
                file,
                "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"turn_aborted\",\"turn_id\":\"two\",\"reason\":\"interrupted\"}}}}"
            )
            .unwrap();
            file.flush().unwrap();
        }
        events.wait_for_status(id, |ev| {
            ev.status.observation.outcome == Some(TurnOutcome::Interrupted)
        });
        assert_eq!(
            manager.agent_status(id).observation.activity,
            Activity::Idle
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
            DirectLaunch {
                cwd: Some(app_data.path().to_str().unwrap().into()),
                ..Default::default()
            },
            app_data.path(),
            pool.clone(),
            events.clone(),
        )
        .unwrap();
    let id = &spawned.id;
    let host_events: Arc<dyn SessionEvents> = events.clone();
    let terminal = manager
        .create_terminal(id, 80, 24, &pool, &host_events)
        .unwrap();
    manager.session_state(id).unwrap().lock().unwrap().terminal = Some(terminal);
    wait_for_observation(&manager, id, Activity::Idle, ObservationSource::Baseline);
    manager
        .inject_direct_stdin(id, b"work", events.as_ref())
        .unwrap();
    manager
        .inject_direct_stdin(id, b"\r", events.as_ref())
        .unwrap();
    wait_for_observation(&manager, id, Activity::Working, ObservationSource::Baseline);
    let transcript = app_data.path().join("rollout.jsonl");
    std::fs::write(&transcript, "").unwrap();
    assert!(!app_data
        .path()
        .join("session-status")
        .join(format!("{id}.ndjson"))
        .exists());
    let turn_before = manager.delivery_snapshot(id).unwrap().turn_started;
    let mut prompt = codex_fixture_report(app_data.path(), id, "UserPromptSubmit", "one");
    prompt.payload["transcript_path"] = serde_json::json!(transcript);
    drop(manager.admit_hook(prompt).unwrap());
    manager.wait_for_turn_started(id, turn_before);
    manager.ingest_output_chunk(
        id,
        None,
        b"\x1b]7501;state=working:app=codex\x1b\\",
        events.as_ref(),
    );
    manager
        .inject_direct_stdin(id, b"\x1b", events.as_ref())
        .unwrap();
    manager.ingest_output_chunk(
        id,
        None,
        b"\x1b]7501;state=idle:app=codex\x1b\\",
        events.as_ref(),
    );
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&transcript)
            .unwrap();
        writeln!(
            file,
            "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"turn_aborted\",\"turn_id\":\"one\",\"reason\":\"interrupted\"}}}}"
        )
        .unwrap();
        file.flush().unwrap();
    }
    events.wait_for_status(id, |ev| {
        ev.status.observation.outcome == Some(TurnOutcome::Interrupted)
    });
    // A trailing OSC 7501 idle keeps Interrupted intact until the next turn.
    manager.ingest_output_chunk(
        id,
        None,
        b"\x1b]7501;state=idle:app=codex\x1b\\",
        events.as_ref(),
    );
    let observation = manager.agent_status(id).observation;
    assert_eq!(observation.activity, Activity::Idle);
    assert_eq!(observation.outcome, Some(TurnOutcome::Interrupted));
    let turn_before_next = manager.delivery_snapshot(id).unwrap().turn_started;
    let mut next = codex_fixture_report(app_data.path(), id, "UserPromptSubmit", "two");
    next.payload["transcript_path"] = serde_json::json!(transcript);
    drop(manager.admit_hook(next).unwrap());
    manager.wait_for_turn_started(id, turn_before_next);
    manager.ingest_output_chunk(
        id,
        None,
        b"\x1b]7501;state=working:app=codex\x1b\\",
        events.as_ref(),
    );
    let working = manager.agent_status(id).observation;
    assert_eq!(working.activity, Activity::Working);
    assert_eq!(working.outcome, None);
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
        let host_events: Arc<dyn SessionEvents> = events.clone();
        let terminal = manager
            .create_terminal(&spawned.id, 80, 24, &pool, &host_events)
            .unwrap();
        manager
            .session_state(&spawned.id)
            .unwrap()
            .lock()
            .unwrap()
            .terminal = Some(terminal);
        std::fs::write(app_data.path().join("quiet"), "").unwrap();
        admit_codex_fixture_hook(
            &manager,
            app_data.path(),
            &spawned.id,
            "SessionStart",
            "one",
        );
        let before_turn = manager.delivery_snapshot(&spawned.id).unwrap().turn_started;
        admit_codex_fixture_hook(
            &manager,
            app_data.path(),
            &spawned.id,
            "UserPromptSubmit",
            "one",
        );
        manager.wait_for_turn_started(&spawned.id, before_turn);
        manager.ingest_output_chunk(
            &spawned.id,
            Some(&mission.id),
            b"\x1b]7501;state=working:app=codex\x1b\\",
            events.as_ref(),
        );
        let observation = manager.agent_status(&spawned.id).observation;
        let args = std::fs::read_to_string(app_data.path().join("args")).unwrap();
        manager.ingest_output_chunk(
            &spawned.id,
            Some(&mission.id),
            b"\x1b]7501;state=done:app=codex\x1b\\",
            events.as_ref(),
        );
        let finished = manager.agent_status(&spawned.id).observation;
        manager.kill(&spawned.id).unwrap();
        assert!(args.contains("automatic first turn"));
        assert_eq!(observation.activity, Activity::Working);
        assert!(observation.program_status.is_some());
        assert_eq!(finished.activity, Activity::Idle);
        assert_eq!(finished.outcome, Some(TurnOutcome::Completed));
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
                DirectLaunch {
                    cwd: Some(app_data.path().to_str().unwrap().into()),
                    ..Default::default()
                },
                app_data.path(),
                pool,
                events.clone(),
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
    use crate::session::status::WaitReason;
    #[derive(Default)]
    struct DeliveryEvents(Mutex<Vec<router::SessionDeliveryEvent>>);
    impl router::SessionDeliveryListener for DeliveryEvents {
        fn session_delivery_event(&self, _: &str, event: router::SessionDeliveryEvent) {
            self.0.lock().unwrap().push(event);
        }
    }
    for mission in [false, true] {
        let manager = mgr_with_fake(None, fake_runtime());
        let pool = pool_with_schema();
        let id = "native-codex";
        pool.get()
            .unwrap()
            .execute(
                "INSERT INTO sessions(id, status, agent_runtime) VALUES (?1, 'running', 'codex')",
                [id],
            )
            .unwrap();
        install_test_session_handle(&manager, id);
        let root = tempfile::tempdir().unwrap();
        let event_log = Arc::new(EventLog::open(root.path()).unwrap());
        let events = capture();
        let host_events: Arc<dyn SessionEvents> = events.clone();
        let terminal = manager
            .create_terminal(id, 80, 24, &pool, &host_events)
            .unwrap();
        {
            let state = manager.session_state(id).unwrap();
            let mut state = state.lock().unwrap();
            state.terminal = Some(terminal);
            if mission {
                state.mission_status_sink = Some(ForwarderEmitCtx {
                    crew_id: Some("crew".into()),
                    mission_id: "mission".into(),
                    handle: "codex".into(),
                    event_log: event_log.clone(),
                });
            }
        }
        let delivered = Arc::new(DeliveryEvents::default());
        let listener: Arc<dyn router::SessionDeliveryListener> = delivered.clone();
        manager.register_delivery_listener(id, Arc::downgrade(&listener));
        let ingest = |bytes: &[u8]| {
            manager.ingest_output_chunk(id, mission.then_some("mission"), bytes, events.as_ref());
        };
        ingest(b"\x1b]7501;state=working:app=codex\x1b\\");
        let state = manager.session_state(id).unwrap();
        state.lock().unwrap().model.apply(
            SessionEvent::TestPending(true),
            crate::session::clock::state_now(),
        );
        for (blocked, expected_reason, release, expected_activity, expected_outcome) in [
            (
                b"\x1b]7501;state=blocked:app=codex\x1b\\".as_slice(),
                WaitReason::Unknown,
                b"\x1b]7501;state=working:app=codex\x07".as_slice(),
                Activity::Working,
                None,
            ),
            (
                b"\x1b]7501;state=blocked:app=codex:kind=permission\x1b\\",
                WaitReason::Approval,
                b"\x1b]7501;state=idle:app=codex\x07",
                Activity::Idle,
                None,
            ),
            (
                b"\x1b]7501;state=blocked:app=codex:kind=question\x1b\\",
                WaitReason::Answer,
                b"\x1b]7501;state=done:app=codex\x1b\\",
                Activity::Idle,
                Some(TurnOutcome::Completed),
            ),
            (
                b"\x1b]7501;state=blocked:app=codex:kind=auth\x1b\\",
                WaitReason::Unknown,
                b"\x1b]7501;state=error:app=codex\x1b\\",
                Activity::Idle,
                Some(TurnOutcome::Failed),
            ),
            (
                b"\x1b]7501;state=blocked:app=codex:kind=permission\x1b\\",
                WaitReason::Approval,
                b"\x1b]7501;state=clear\x1b\\",
                Activity::Unavailable,
                None,
            ),
        ] {
            ingest(blocked);
            let wait = manager.agent_status(id).observation.interactions[0].clone();
            assert_eq!(wait.reason, expected_reason);
            let count = events.status.lock().unwrap().len();
            ingest(blocked);
            assert_eq!(events.status.lock().unwrap().len(), count);
            assert_eq!(
                manager.reserve_delivery(id).unwrap(),
                router::DeliveryReservation::HumanInteraction
            );
            manager.mark_status_viewed(&[id.into()]);
            assert!(manager.status_snapshot()[id].observation.needs_you());
            ingest(release);
            assert!(!manager.agent_status(id).observation.needs_you());
            assert_eq!(
                manager.agent_status(id).observation.activity,
                expected_activity
            );
            assert_eq!(
                manager.agent_status(id).observation.outcome,
                expected_outcome
            );
            assert_eq!(
                manager.reserve_delivery(id).unwrap(),
                router::DeliveryReservation::LocalInputPending
            );
        }
        assert!(delivered
            .0
            .lock()
            .unwrap()
            .contains(&router::SessionDeliveryEvent::ReadinessChanged));
        ingest(b"\x1b]7501;state=working:app=codex\x1b\\");
        state.lock().unwrap().model.apply(
            SessionEvent::TestPending(false),
            crate::session::clock::state_now(),
        );
        assert_eq!(
            manager.reserve_delivery(id).unwrap(),
            router::DeliveryReservation::Unavailable
        );
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
            for reason in ["unknown", "approval", "answer"] {
                assert!(
                    rows.iter().any(|row| {
                        row.event.payload["status"]["observation"]["interactions"][0]["reason"]
                            == reason
                    }),
                    "missing mission wait reason {reason}"
                );
            }
        }
    }
}

#[test]
fn codex_native_clear_never_runs_completion_callback_even_when_armed() {
    use crate::session::state::agent::AgentEvent;
    let core = crate::test_support::test_core();
    let id = "native-attention";
    core.db
        .get()
        .unwrap()
        .execute(
            "INSERT INTO sessions(id, status, agent_runtime) VALUES (?1, 'running', 'codex')",
            [id],
        )
        .unwrap();
    install_test_session_handle(&core.sessions, id);
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    let terminal = core
        .sessions
        .create_terminal(id, 80, 24, &core.db, &events)
        .unwrap();
    let state = core.sessions.session_state(id).unwrap();
    state.lock().unwrap().terminal = Some(terminal);
    core.sessions.publish_agent_event(
        id,
        AgentEvent::Batch {
            runtime: Runtime::Codex,
            events: vec![AgentEvent::StartupReady],
        },
        events.as_ref(),
    );
    state.lock().unwrap().model.apply(
        SessionEvent::ArmCompletion,
        crate::session::clock::state_now(),
    );
    for report in [
        b"\x1b]7501;state=blocked:app=codex:kind=permission\x1b\\".as_slice(),
        b"\x1b]7501;state=blocked:app=codex:kind=permission\x1b\\",
        b"\x1b]7501;state=working:app=codex\x1b\\",
        b"\x1b]7501;state=blocked:app=codex:kind=question\x1b\\",
        b"\x1b]7501;state=done:app=codex\x1b\\",
        b"\x1b]7501;state=blocked:app=codex:kind=auth\x1b\\",
        b"\x1b]7501;state=error:app=codex\x1b\\",
        b"\x1b]7501;state=blocked:app=codex\x1b\\",
        b"\x1b]7501;state=idle:app=codex\x1b\\",
        b"\x1b]7501;state=clear\x1b\\",
    ] {
        core.sessions
            .ingest_output_chunk(id, None, report, events.as_ref());
    }
    let status = core.sessions.agent_status(id);
    assert_eq!(status.unread_since, None);
    assert_eq!(status.observation.outcome, None);
    assert!(
        state.lock().unwrap().model.completion_armed(),
        "native attention must not consume hook completion"
    );
    assert!(
        !crate::repo::session_attention::any_unread(&core.db.get().unwrap(), &[id.into()]).unwrap()
    );
}

#[test]
fn codex_native_forwarder_publishes_without_a_mounted_client_for_chats_and_missions() {
    use crate::session::state::agent::AgentEvent as A;
    use crate::session::status::WaitReason;
    for mission in [false, true] {
        let fake = fake_runtime();
        let manager = mgr_with_fake(None, fake.clone());
        let pool = pool_with_schema();
        let id = "native-forwarder";
        pool.get()
            .unwrap()
            .execute(
                "INSERT INTO sessions(id, status, agent_runtime) VALUES (?1, 'running', 'codex')",
                [id],
            )
            .unwrap();
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
        let events = capture();
        let host_events: Arc<dyn SessionEvents> = events.clone();
        let terminal = manager
            .create_terminal(id, 80, 24, &pool, &host_events)
            .unwrap();
        {
            let state = manager.session_state(id).unwrap();
            let mut state = state.lock().unwrap();
            state.handle.as_mut().unwrap().stop = output.stop_flag();
            state.terminal = Some(terminal);
            state.mission_status_sink = sink.clone();
            assert!(state.subscribers.is_empty());
        }
        {
            let spawns = fake.spawns.lock().unwrap();
            let tx = spawns[0].tx.as_ref().unwrap();
            for item in [
                RuntimeOutput::AgentEvent {
                    event: A::ProgramTurnConfirmed {
                        conversation_changed: false,
                        prompt: None,
                    },
                    feedback: None,
                },
                RuntimeOutput::Stream(b"\x1b]7501;state=working:app=codex\x1b\\".to_vec()),
                RuntimeOutput::Stream(
                    b"\x1b]7501;state=blocked:app=codex:kind=permission\x1b\\".to_vec(),
                ),
                RuntimeOutput::Stream(
                    b"\x1b]7501;state=blocked:app=codex:kind=permission\x1b\\".to_vec(),
                ),
                RuntimeOutput::Stream(b"\x1b]7501;state=working:app=codex\x1b\\".to_vec()),
                RuntimeOutput::Stream(
                    b"\x1b]7501;state=blocked:app=codex:kind=question\x1b\\".to_vec(),
                ),
                RuntimeOutput::AgentEvent {
                    event: A::ProgramTurnOutcome {
                        outcome: TurnOutcome::Interrupted,
                        policy: crate::session::state::agent::ProgramOutcomePolicy::Immediate,
                    },
                    feedback: None,
                },
                RuntimeOutput::Stream(b"\x1b]7501;state=idle:app=codex\x1b\\".to_vec()),
            ] {
                tx.send(item).unwrap();
            }
        }
        fake.close_spawn(0);
        manager
            .start_forwarder_thread(
                id.into(),
                String::new(),
                mission.then(|| "mission".into()),
                runtime,
                output,
                pool,
                events.clone(),
                role("fake", &[]),
                None,
                false,
                sink,
                root.path().to_path_buf(),
                None,
            )
            .join()
            .unwrap();
        let statuses = events.status.lock().unwrap();
        assert_eq!(statuses.len(), 5);
        assert_eq!(statuses[0].status.observation.activity, Activity::Working);
        assert!(statuses[1].status.observation.needs_you());
        assert_eq!(
            statuses[1].status.observation.interactions[0].reason,
            WaitReason::Approval
        );
        assert_eq!(statuses[2].state, SessionActivityState::Busy);
        assert!(!statuses[2].status.observation.needs_you());
        assert_eq!(statuses[2].status.observation.outcome, None);
        assert_eq!(
            statuses[3].status.observation.interactions[0].reason,
            WaitReason::Answer
        );
        assert_eq!(
            statuses[4].status.observation.outcome,
            Some(TurnOutcome::Interrupted)
        );
        assert!(!statuses[4].status.observation.needs_you());
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
    let pool = pool_with_schema();
    let id = "native-generation";
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO sessions(id, status, agent_runtime) VALUES (?1, 'running', 'codex')",
            [id],
        )
        .unwrap();
    install_test_session_handle(&manager, id);
    let events = capture();
    let host_events: Arc<dyn SessionEvents> = events.clone();
    let terminal = manager
        .create_terminal(id, 80, 24, &pool, &host_events)
        .unwrap();
    manager.session_state(id).unwrap().lock().unwrap().terminal = Some(terminal);
    let older = Arc::new(AtomicBool::new(false));
    for report in [
        b"\x1b]7501;state=blocked:app=codex:kind=permission\x1b\\".as_slice(),
        b"\x1b]7501;state=idle:app=codex\x1b\\",
    ] {
        manager.ingest_output_for_launch(id, None, report, events.as_ref(), Some(&older));
        assert!(events.status.lock().unwrap().is_empty());
    }
    manager.ingest_output_chunk(
        id,
        None,
        b"\x1b]7501;state=blocked:app=codex:kind=permission\x1b\\",
        events.as_ref(),
    );
    manager.ingest_output_for_launch(
        id,
        None,
        b"\x1b]7501;state=idle:app=codex\x1b\\",
        events.as_ref(),
        Some(&older),
    );
    assert!(manager.agent_status(id).observation.needs_you());
    manager.session_state(id).unwrap().lock().unwrap().killed = true;
    manager.ingest_output_chunk(
        id,
        None,
        b"\x1b]7501;state=idle:app=codex\x1b\\",
        events.as_ref(),
    );
    assert!(manager.agent_status(id).observation.needs_you());
}

#[test]
fn codex_native_attention_preserves_real_hook_completion_in_both_clear_orderings() {
    use crate::session::state::agent::AgentEvent as A;
    for idle_first in [false, true] {
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
            let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
            let terminal = core
                .sessions
                .create_terminal(id, 80, 24, &core.db, &events)
                .unwrap();
            core.sessions
                .session_state(id)
                .unwrap()
                .lock()
                .unwrap()
                .terminal = Some(terminal);
            let native = |bytes: &[u8]| {
                core.sessions
                    .ingest_output_chunk(id, None, bytes, events.as_ref());
            };
            let writes = || {
                core.db.get().unwrap().query_row("SELECT count(*) FROM completion_writes WHERE session_id = 'native-completion'", [], |row| row.get::<_, i64>(0)).unwrap()
            };
            core.sessions.publish_agent_event(
                id,
                A::ProgramTurnConfirmed {
                    conversation_changed: false,
                    prompt: None,
                },
                events.as_ref(),
            );
            native(b"\x1b]7501;state=working:app=codex\x1b\\");
            native(b"\x1b]7501;state=blocked:app=codex:kind=permission\x1b\\");
            native(b"\x1b]7501;state=blocked:app=codex:kind=permission\x1b\\");
            assert_eq!(writes(), 0, "blocked wait must not record completion");
            native(b"\x1b]7501;state=working:app=codex\x1b\\");
            match outcome {
                TurnOutcome::Completed => {
                    native(b"\x1b]7501;state=done:app=codex\x1b\\");
                    assert_eq!(writes(), 1);
                    // Keypress acknowledgement (done -> idle) and duplicate done never double-write.
                    native(b"\x1b]7501;state=idle:app=codex\x1b\\");
                    native(b"\x1b]7501;state=done:app=codex\x1b\\");
                    assert_eq!(writes(), 1);
                }
                TurnOutcome::Failed => {
                    if idle_first {
                        native(b"\x1b]7501;state=idle:app=codex\x1b\\");
                        core.sessions.publish_agent_event(
                            id,
                            A::ProgramTurnOutcome {
                                outcome: TurnOutcome::Failed,
                                policy:
                                    crate::session::state::agent::ProgramOutcomePolicy::Immediate,
                            },
                            events.as_ref(),
                        );
                    } else {
                        core.sessions.publish_agent_event(
                            id,
                            A::ProgramTurnOutcome {
                                outcome: TurnOutcome::Failed,
                                policy:
                                    crate::session::state::agent::ProgramOutcomePolicy::Immediate,
                            },
                            events.as_ref(),
                        );
                        native(b"\x1b]7501;state=idle:app=codex\x1b\\");
                    }
                    assert_eq!(writes(), 0);
                    let status = core.sessions.agent_status(id);
                    assert_eq!(status.observation.outcome, Some(TurnOutcome::Failed));
                    assert!(status.failed_since.is_some());
                }
                TurnOutcome::Interrupted => {
                    if idle_first {
                        native(b"\x1b]7501;state=idle:app=codex\x1b\\");
                        core.sessions.publish_agent_event(
                            id,
                            A::ProgramTurnOutcome {
                                outcome: TurnOutcome::Interrupted,
                                policy:
                                    crate::session::state::agent::ProgramOutcomePolicy::Immediate,
                            },
                            events.as_ref(),
                        );
                    } else {
                        core.sessions.publish_agent_event(
                            id,
                            A::ProgramTurnOutcome {
                                outcome: TurnOutcome::Interrupted,
                                policy:
                                    crate::session::state::agent::ProgramOutcomePolicy::Immediate,
                            },
                            events.as_ref(),
                        );
                        native(b"\x1b]7501;state=idle:app=codex\x1b\\");
                    }
                    assert_eq!(writes(), 0);
                    assert_eq!(
                        core.sessions.agent_status(id).observation.outcome,
                        Some(TurnOutcome::Interrupted)
                    );
                }
            }
            let status = core.sessions.agent_status(id);
            assert_eq!(
                status.unread_since.is_some(),
                outcome == TurnOutcome::Completed
            );
            assert!(!status.observation.needs_you());
            assert_eq!(
                writes(),
                i64::from(outcome == TurnOutcome::Completed),
                "idle_first={idle_first} outcome={outcome:?}"
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
    use crate::session::state::agent::AgentEvent as A;
    use crate::session::status::WaitReason;

    for after_bridge_loss in [false, true] {
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
            let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
            let terminal = core
                .sessions
                .create_terminal(id, 80, 24, &core.db, &events)
                .unwrap();
            let state = core.sessions.session_state(id).unwrap();
            {
                let mut guard = state.lock().unwrap();
                guard.terminal = Some(terminal);
                // Exercise the Commit 1 hooked-overlay policy explicitly to ensure
                // overlay open/release preserves baseline activity and completion.
                guard.model.set_program_status_authority(false);
                guard.model.set_native_attention(true);
            }
            if after_bridge_loss {
                core.sessions.publish_agent_event(
                    id,
                    A::Batch {
                        runtime: Runtime::Codex,
                        events: vec![A::TurnStarted],
                    },
                    events.as_ref(),
                );
                core.sessions.publish_model_event(
                    id,
                    SessionEvent::BridgeFailed { live: true },
                    events.as_ref(),
                );
            }
            let start = Instant::now();
            let mut detector = IdleDetector::with_adapter_at(
                Duration::from_secs(2),
                crate::runtimes::adapter(Runtime::Codex).terminal_adapter(Some(false)),
                start,
            );
            detector.hooks_unavailable();
            let publish = |event| {
                core.sessions
                    .publish_direct_terminal_event(id, event, events.as_ref())
            };
            publish(
                detector
                    .tick_event_at(start + Duration::from_secs(3))
                    .unwrap(),
            );
            state.lock().unwrap().model.apply(
                SessionEvent::ArmCompletion,
                crate::session::clock::state_now(),
            );
            for (index, (control, expected_wait)) in [
                (
                    b"\x1b]7501;state=blocked:app=codex\x1b\\".as_slice(),
                    Some(WaitReason::Unknown),
                ),
                (
                    b"\x1b]7501;state=blocked:app=codex:kind=permission\x1b\\",
                    Some(WaitReason::Approval),
                ),
                (
                    b"\x1b]7501;state=blocked:app=codex:kind=question\x1b\\",
                    Some(WaitReason::Answer),
                ),
                (
                    b"\x1b]7501;state=blocked:app=codex:kind=auth\x07",
                    Some(WaitReason::Unknown),
                ),
                (b"\x1b]7501;state=clear\x1b\\", None),
                (b"\x1b]7501;state=working:app=codex\x07", None),
                (b"\x1b]7501;state=done:app=codex\x1b\\", None),
                (b"\x1b]7501;state=error:app=codex\x1b\\", None),
                (b"\x1b]7501;state=idle:app=codex\x1b\\", None),
                (b"\x1b]7501;state=clear\x07", None),
            ]
            .iter()
            .enumerate()
            {
                let time = start + Duration::from_secs(4 + index as u64 * 4);
                let chunk_size = if split_bytes { 1 } else { control.len() };
                for chunk in control.chunks(chunk_size) {
                    core.sessions
                        .ingest_output_chunk(id, None, chunk, events.as_ref());
                    if let Some(event) = detector.on_output_event_at(chunk, false, time) {
                        publish(event);
                    }
                }
                if let Some(event) = detector.tick_event_at(time + Duration::from_secs(3)) {
                    publish(event);
                }
                let status = core.sessions.agent_status(id);
                assert_eq!(
                    status.observation.interactions.first().map(|w| w.reason),
                    *expected_wait,
                    "after_bridge_loss={after_bridge_loss} split={split_bytes} idx={index}"
                );
                assert!(
                    status.observation.program_status.is_none(),
                    "Codex with status hooks on stays in overlay mode before hook takeover and after bridge loss"
                );
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
            let time = start + Duration::from_secs(60);
            let chunk_size = if split_bytes { 1 } else { mixed.len() };
            for chunk in mixed.chunks(chunk_size) {
                core.sessions
                    .ingest_output_chunk(id, None, chunk, events.as_ref());
                if let Some(event) = detector.on_output_event_at(chunk, false, time) {
                    publish(event);
                }
            }
            let working = core.sessions.agent_status(id);
            assert_eq!(working.observation.activity, Activity::Working);
            assert!(working.observation.program_status.is_none());
            publish(
                detector
                    .tick_event_at(time + Duration::from_secs(3))
                    .unwrap(),
            );
            assert!(core.sessions.agent_status(id).unread_since.is_some());
            assert!(crate::repo::session_attention::any_unread(
                &core.db.get().unwrap(),
                &[id.into()]
            )
            .unwrap());
            assert!(!state.lock().unwrap().model.completion_armed());
        }
    }
}

#[test]
fn codex_authoritative_program_status_ignores_foreign_root_and_child_reports_end_to_end() {
    use crate::session::status::WaitReason;

    let core = crate::test_support::test_core_with_runtime(fake_runtime());
    let id = "codex-foreign-isolation";
    core.db
        .get()
        .unwrap()
        .execute(
            "INSERT INTO sessions(id, status, agent_runtime) VALUES (?1, 'running', 'codex')",
            [id],
        )
        .unwrap();
    install_test_session_handle(&core.sessions, id);
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    // Real Codex launch semantics from create_terminal: program_status_authority = true,
    // program_status_root_app = Some("codex"), native_attention = false.
    let terminal = core
        .sessions
        .create_terminal(id, 80, 24, &core.db, &events)
        .unwrap();
    core.sessions
        .session_state(id)
        .unwrap()
        .lock()
        .unwrap()
        .terminal = Some(terminal);

    let ingest = |bytes: &[u8]| {
        core.sessions
            .ingest_output_chunk(id, None, bytes, events.as_ref())
    };
    let unread = || {
        crate::repo::session_attention::any_unread(&core.db.get().unwrap(), &[id.into()]).unwrap()
    };

    // Foreign root or child reports never open status, waits, or completion.
    for foreign in [
        b"\x1b]7501;state=blocked:app=other:kind=permission\x1b\\".as_slice(),
        b"\x1b]7501;state=blocked:kind=permission\x1b\\",
        b"\x1b]7501;state=blocked:id=sub:app=codex:kind=question\x1b\\",
        b"\x1b]7501;state=blocked:id=sub:app=other:kind=question\x1b\\",
        b"\x1b]7501;state=working:app=other\x1b\\",
        b"\x1b]7501;state=done:app=other\x1b\\",
    ] {
        ingest(foreign);
        let status = core.sessions.agent_status(id);
        assert!(status.observation.program_status.is_none());
        assert!(!status.observation.needs_you());
        assert!(!unread());
    }

    // Genuine Codex working arms completion; foreign done (root or child) while armed
    // must NOT finish the turn or record an unread completion badge.
    ingest(b"\x1b]7501;state=working:app=codex\x1b\\");
    assert_eq!(
        core.sessions.agent_status(id).observation.activity,
        Activity::Working
    );
    assert!(core
        .sessions
        .session_state(id)
        .unwrap()
        .lock()
        .unwrap()
        .model
        .completion_armed());
    for foreign in [
        b"\x1b]7501;state=done:app=other\x1b\\".as_slice(),
        b"\x1b]7501;state=done\x1b\\",
        b"\x1b]7501;state=done:id=sub:app=codex\x1b\\",
        b"\x1b]7501;state=idle:app=other\x1b\\",
    ] {
        ingest(foreign);
        let status = core.sessions.agent_status(id);
        assert_eq!(status.observation.activity, Activity::Working);
        assert_eq!(status.observation.outcome, None);
        assert!(!unread());
    }

    // Genuine Codex permission wait opens Approval needed and holds crew delivery.
    ingest(b"\x1b]7501;state=blocked:app=codex:kind=permission\x1b\\");
    let wait = core.sessions.agent_status(id).observation.interactions[0].clone();
    assert_eq!(wait.reason, WaitReason::Approval);
    assert_eq!(
        core.sessions.delivery_readiness(id),
        router::DeliveryReadiness::NeedsYou
    );

    // Interleaved foreign root idle/done/working/clear and foreign child blocked reports
    // cannot release or mutate Codex's wait.
    for foreign in [
        b"\x1b]7501;state=idle:app=other\x1b\\".as_slice(),
        b"\x1b]7501;state=done:app=other\x1b\\",
        b"\x1b]7501;state=working:app=claude\x07",
        b"\x1b]7501;state=clear:app=other\x1b\\",
        b"\x1b]7501;state=clear:id=sub\x1b\\",
        b"\x1b]7501;state=blocked:id=sub:app=other:kind=question\x1b\\",
        b"\x1b]7501;state=blocked:id=sub:app=codex:kind=question\x1b\\",
    ] {
        ingest(foreign);
        assert_eq!(
            core.sessions.agent_status(id).observation.interactions,
            vec![wait.clone()],
            "foreign report must not disturb Codex wait"
        );
        assert_eq!(
            core.sessions.delivery_readiness(id),
            router::DeliveryReadiness::NeedsYou
        );
        assert!(!unread());
    }

    // Codex's own root Done wins even while the foreign blocked child still occupies the
    // shared parser's aggregate record, recording Completed and one unread badge.
    ingest(b"\x1b]7501;state=done:app=codex\x1b\\");
    let completed = core.sessions.agent_status(id);
    assert!(!completed.observation.needs_you());
    assert_eq!(completed.observation.activity, Activity::Idle);
    assert_eq!(completed.observation.outcome, Some(TurnOutcome::Completed));
    assert!(completed.unread_since.is_some());
    assert!(unread());
    assert_eq!(
        core.sessions.delivery_readiness(id),
        router::DeliveryReadiness::Ready { idle: true }
    );

    // Unscoped root clear releases a live Codex wait and clears program_status, whereas
    // foreign scoped clear and child clear do not.
    ingest(b"\x1b]7501;state=working:app=codex\x1b\\");
    ingest(b"\x1b]7501;state=blocked:app=codex:kind=question\x1b\\");
    assert_eq!(
        core.sessions.agent_status(id).observation.interactions[0].reason,
        WaitReason::Answer
    );
    ingest(b"\x1b]7501;state=clear:app=other\x1b\\");
    assert!(core.sessions.agent_status(id).observation.needs_you());
    ingest(b"\x1b]7501;state=clear\x1b\\");
    let cleared = core.sessions.agent_status(id);
    assert!(!cleared.observation.needs_you());
    assert!(cleared.observation.program_status.is_none());
    assert!(!core
        .sessions
        .session_state(id)
        .unwrap()
        .lock()
        .unwrap()
        .model
        .completion_armed());
}

#[test]
fn codex_mixed_ready_or_title_idle_with_blocked_installs_wait_before_heuristic_through_forwarder() {
    use crate::session::pty_runtime::{emit_read_chunk, IdleDetector};
    use crate::session::status::WaitReason;

    type RecordedStatus = (SessionActivityState, StatusSource, bool, Option<WaitReason>);
    struct RecordingEvents {
        inner: crate::session::manager::CoreSessionEvents,
        recorded: Arc<Mutex<Vec<RecordedStatus>>>,
    }
    impl SessionEvents for RecordingEvents {
        fn exit(&self, ev: &ExitEvent) {
            self.inner.exit(ev);
        }
        fn spawned(&self, ev: &SessionSpawnedEvent) {
            self.inner.spawned(ev);
        }
        fn archived(&self, ev: &SessionUpdatedEvent) {
            self.inner.archived(ev);
        }
        fn updated(&self, ev: &SessionUpdatedEvent) {
            self.inner.updated(ev);
        }
        fn status(&self, ev: &SessionActivityEvent) {
            self.inner.status(ev);
            self.recorded.lock().unwrap().push((
                ev.state,
                ev.source,
                ev.status.observation.needs_you(),
                ev.status.observation.interactions.first().map(|w| w.reason),
            ));
        }
        fn role_activity(&self, ev: &RoleActivityEvent) {
            self.inner.role_activity(ev);
        }
        fn warning(&self, ev: &WarningEvent) {
            self.inner.warning(ev);
        }
    }

    for (label, title_mode, mixed, expected_reason) in [
        (
            "ready-idle-plus-blocked",
            false,
            b"\x1b[?2004h\x1b]7501;state=blocked:app=codex:kind=permission\x1b\\".to_vec(),
            WaitReason::Approval,
        ),
        (
            "title-idle-plus-blocked",
            true,
            b"\x1b]0;project\x07\x1b]7501;state=blocked:app=codex:kind=question\x1b\\".to_vec(),
            WaitReason::Answer,
        ),
    ] {
        let fake = fake_runtime();
        let core = crate::test_support::test_core_with_runtime(fake.clone());
        let id = format!("mixed-read-{label}");
        core.db
            .get()
            .unwrap()
            .execute(
                "INSERT INTO sessions(id, status, agent_runtime) VALUES (?1, 'running', 'codex')",
                [&id],
            )
            .unwrap();
        let (runtime, output) = fake
            .spawn(SpawnSpec {
                session_id: id.clone(),
                ..Default::default()
            })
            .unwrap();
        install_test_session_handle(&core.sessions, &id);
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let events: Arc<dyn SessionEvents> = Arc::new(RecordingEvents {
            inner: core.session_events(),
            recorded: Arc::clone(&recorded),
        });
        let terminal = core
            .sessions
            .create_terminal(&id, 80, 24, &core.db, &events)
            .unwrap();
        let state = core.sessions.session_state(&id).unwrap();
        {
            let mut guard = state.lock().unwrap();
            guard.handle.as_mut().unwrap().stop = output.stop_flag();
            guard.terminal = Some(terminal);
            guard.model.apply(
                SessionEvent::Transition {
                    state: SessionActivityState::Busy,
                    source: StatusSource::Forwarder,
                    live: true,
                },
                crate::session::clock::state_now(),
            );
            guard.model.apply(
                SessionEvent::ArmCompletion,
                crate::session::clock::state_now(),
            );
        }

        let start = Instant::now();
        let mut raw_detector = IdleDetector::with_adapter_at(
            Duration::from_secs(2),
            crate::runtimes::adapter(Runtime::Codex).terminal_adapter(Some(false)),
            start,
        );
        raw_detector.hooks_unavailable();
        if title_mode {
            raw_detector.on_output_event_at(b"\x1b]0;\xe2\xa0\x8b project\x07", false, start);
            raw_detector.on_output_event_at(b"\x1b]0;\xe2\xa0\x99 project\x07", false, start);
        }
        let detector = Mutex::new(raw_detector);

        let (fence_tx, fence_rx) = std::sync::mpsc::channel();
        {
            let spawns = fake.spawns.lock().unwrap();
            let tx = spawns[0].tx.as_ref().unwrap();
            assert!(emit_read_chunk(&mixed, false, &detector, tx));
            tx.send(RuntimeOutput::AgentEvent {
                event: crate::session::state::agent::AgentEvent::Batch {
                    runtime: Runtime::Codex,
                    events: Vec::new(),
                },
                feedback: Some(fence_tx),
            })
            .unwrap_or_else(|_| panic!("{label}"));
        }
        let root = tempfile::tempdir().unwrap();
        let forwarder = core.sessions.start_forwarder_thread(
            id.clone(),
            String::new(),
            None,
            runtime,
            output,
            Arc::clone(&core.db),
            Arc::clone(&events),
            role("codex", &[]),
            None,
            false,
            None,
            root.path().to_path_buf(),
            None,
        );
        fence_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|error| panic!("{label}: forwarder fence failed: {error}"));

        assert_eq!(
            core.sessions.delivery_readiness(&id),
            router::DeliveryReadiness::NeedsYou,
            "{label}"
        );
        let status = core.sessions.agent_status(&id);
        assert!(status.observation.needs_you(), "{label}");
        assert_eq!(status.unread_since, None, "{label}");
        assert!(state.lock().unwrap().model.completion_armed(), "{label}");
        let rows: i64 = core
            .db
            .get()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM session_attention WHERE session_id = ?1",
                [&id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rows, 0, "{label}: must write zero completion rows");
        let seq = recorded.lock().unwrap().clone();
        assert_eq!(
            seq,
            vec![(
                SessionActivityState::Busy,
                StatusSource::ProgramStatus,
                true,
                Some(expected_reason),
            )],
            "{label}: only the native wait must publish; heuristic Idle must never precede or follow it"
        );

        fake.close_spawn(0);
        forwarder.join().unwrap();
    }
}

#[test]
fn codex_resume_takes_back_a_replaced_conversation_or_starts_fresh() {
    let v7 = |at: std::time::SystemTime| {
        let millis = at
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let mut bytes = *uuid::Uuid::new_v4().as_bytes();
        bytes[..6].copy_from_slice(&millis.to_be_bytes()[2..]);
        bytes[6] = 0x70 | (bytes[6] & 0x0f);
        uuid::Uuid::from_bytes(bytes).to_string()
    };
    for recoverable in [false, true] {
        let pool = pool_with_schema();
        let data = tempfile::tempdir().unwrap();
        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, fake.clone());
        db::set_runtime_override(&pool, "codex", Some("custom-codex-wrapper")).unwrap();
        let home = data.path().join("codex-home");
        mgr.shell_env
            .write()
            .unwrap()
            .vars
            .insert("CODEX_HOME".into(), home.display().to_string());
        let id = ulid::Ulid::new();
        let created: chrono::DateTime<chrono::Local> =
            chrono::DateTime::<chrono::Utc>::from(id.datetime()).into();
        let day = home.join("sessions").join(created.format("%Y").to_string());
        let day = day
            .join(created.format("%m").to_string())
            .join(created.format("%d").to_string());
        std::fs::create_dir_all(&day).unwrap();
        let replaced = v7(std::time::SystemTime::now() - Duration::from_secs(60));
        let original = v7(std::time::SystemTime::now() - Duration::from_secs(3600));
        if recoverable {
            let marker = crate::runtimes::codex::capture::prompt_marker(&id.to_string());
            let lines = [
                serde_json::json!({"type": "session_meta", "payload": {"id": original, "source": "cli"}}),
                serde_json::json!({"type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": format!("goal\n\n{marker}")}]}}),
                serde_json::json!({"type": "response_item", "payload": {"type": "message", "role": "assistant", "content": []}}),
            ];
            let body: String = lines.iter().map(|line| format!("{line}\n")).collect();
            std::fs::write(
                day.join(format!("rollout-2026-11-03T09-22-56-{original}.jsonl")),
                body,
            )
            .unwrap();
        }
        let mut row = crate::repo::session::SessionRowDb::new_running(id.to_string());
        row.agent_runtime = Some("codex".into());
        row.agent_command = Some("custom-codex-wrapper".into());
        row.agent_session_key = Some(replaced.clone());
        row.status = crate::model::SessionStatus::Stopped;
        crate::test_support::insert_session_row(&pool.get().unwrap(), &row).unwrap();

        let launch =
            mgr.resume_on_launch(&row.id, None, None, data.path(), pool.clone(), capture());
        let spawned = if recoverable {
            launch.unwrap()
        } else {
            let error = launch.err().unwrap();
            assert!(error.to_string().contains("conversation is unavailable"));
            assert!(fake.spawns.lock().unwrap().is_empty());
            mgr.resume(&row.id, None, None, data.path(), pool.clone(), capture())
                .unwrap()
        };
        let args = fake.last_spawn_spec().unwrap().args;
        let saved = crate::repo::session::get_row(&pool.get().unwrap(), &row.id)
            .unwrap()
            .unwrap();
        mgr.kill(&spawned.id).unwrap();
        if recoverable {
            assert!(has_arg_pair(&args, "resume", &original), "{args:?}");
            assert_eq!(saved.agent_session_key.as_deref(), Some(original.as_str()));
        } else {
            assert!(!args.contains(&"resume".into()), "{args:?}");
            assert_eq!(saved.agent_session_key, None);
        }
    }
}

#[test]
fn codex_mission_delivery_uses_user_prompt_submit_and_osc7501_for_busy_blocked_and_done() {
    use crate::session::state::agent::AgentEvent as A;
    use crate::session::status::WaitReason;

    #[derive(Default)]
    struct DeliveryLog(Mutex<Vec<router::SessionDeliveryEvent>>);
    impl router::SessionDeliveryListener for DeliveryLog {
        fn session_delivery_event(&self, _: &str, event: router::SessionDeliveryEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let fake = fake_runtime();
    let core = crate::test_support::test_core_with_runtime(fake.clone());
    let mut codex_role = role("codex", &[]);
    codex_role.runtime = Runtime::Codex.key().into();
    let (mission, slot) = seed_mission_rows(&core.db, &codex_role);
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    let spawned = core
        .sessions
        .spawn(
            &mission,
            &codex_role,
            &slot,
            dir.path(),
            dir.path().join("events.ndjson"),
            Arc::clone(&core.db),
            Arc::clone(&events),
            Some("initial mission prompt".into()),
        )
        .unwrap();
    let id = &spawned.id;
    let terminal = core
        .sessions
        .create_terminal(id, 80, 24, &core.db, &events)
        .unwrap();
    core.sessions
        .session_state(id)
        .unwrap()
        .lock()
        .unwrap()
        .terminal = Some(terminal);
    let delivery = Arc::new(DeliveryLog::default());
    let listener: Arc<dyn router::SessionDeliveryListener> = delivery.clone();
    core.sessions
        .register_delivery_listener(id, Arc::downgrade(&listener));

    // Only SessionStart and UserPromptSubmit hooks are injected in mission spawns.
    let spawn_args = fake.last_spawn_spec().unwrap().args;
    let hooks: Vec<&String> = spawn_args
        .iter()
        .filter(|arg| arg.starts_with("hooks."))
        .collect();
    assert_eq!(hooks.len(), 2, "{hooks:?}");
    assert!(hooks[0].starts_with("hooks.SessionStart="));
    assert!(hooks[1].starts_with("hooks.UserPromptSubmit="));

    // First-turn startup holds mission delivery until OSC 7501 Done completes it.
    assert_eq!(
        core.sessions.delivery_readiness(id),
        router::DeliveryReadiness::Starting
    );
    core.sessions.publish_agent_event(
        id,
        A::ProgramSessionStarted {
            conversation_changed: false,
            compacted: false,
        },
        events.as_ref(),
    );
    core.sessions.publish_agent_event(
        id,
        A::ProgramTurnConfirmed {
            conversation_changed: false,
            prompt: None,
        },
        events.as_ref(),
    );
    core.sessions.ingest_output_chunk(
        id,
        Some(&mission.id),
        b"\x1b]7501;state=working:app=codex\x1b\\",
        events.as_ref(),
    );
    assert_eq!(
        core.sessions.delivery_readiness(id),
        router::DeliveryReadiness::Starting
    );
    core.sessions.ingest_output_chunk(
        id,
        Some(&mission.id),
        b"\x1b]7501;state=done:app=codex\x1b\\",
        events.as_ref(),
    );
    assert_eq!(
        core.sessions.delivery_readiness(id),
        router::DeliveryReadiness::Ready { idle: true }
    );

    // Next turn: UserPromptSubmit confirms start once, subsequent OSC 7501 Working does
    // not double-count, and an ordinary Working Codex slot stays deliverable mid-turn
    // (DeliveryReadiness::Working) while linked-chat reports wait for idle.
    let before = core.sessions.delivery_snapshot(id).unwrap().turn_started;
    core.sessions.publish_agent_event(
        id,
        A::ProgramTurnConfirmed {
            conversation_changed: false,
            prompt: None,
        },
        events.as_ref(),
    );
    assert_eq!(
        core.sessions.delivery_snapshot(id).unwrap().turn_started,
        before + 1
    );
    core.sessions.ingest_output_chunk(
        id,
        Some(&mission.id),
        b"\x1b]7501;state=working:app=codex\x1b\\",
        events.as_ref(),
    );
    assert_eq!(
        core.sessions.delivery_snapshot(id).unwrap().turn_started,
        before + 1
    );
    assert_eq!(
        core.sessions.delivery_readiness(id),
        router::DeliveryReadiness::Working
    );
    assert_eq!(
        core.sessions.report_readiness(id),
        router::DeliveryReadiness::Ready { idle: false }
    );

    // A steered mid-turn notice confirms turn_started precisely once.
    core.sessions.publish_agent_event(
        id,
        A::ProgramTurnConfirmed {
            conversation_changed: false,
            prompt: None,
        },
        events.as_ref(),
    );
    assert_eq!(
        core.sessions.delivery_snapshot(id).unwrap().turn_started,
        before + 2
    );

    // Blocked (permission/question) holds crew delivery as NeedsYou.
    core.sessions.ingest_output_chunk(
        id,
        Some(&mission.id),
        b"\x1b]7501;state=blocked:app=codex:kind=permission\x1b\\",
        events.as_ref(),
    );
    assert_eq!(
        core.sessions.delivery_readiness(id),
        router::DeliveryReadiness::NeedsYou
    );
    assert_eq!(
        core.sessions.agent_status(id).observation.interactions[0].reason,
        WaitReason::Approval
    );

    // Done resolves the wait and notifies delivery Idle.
    delivery.0.lock().unwrap().clear();
    core.sessions.ingest_output_chunk(
        id,
        Some(&mission.id),
        b"\x1b]7501;state=done:app=codex\x1b\\",
        events.as_ref(),
    );
    assert_eq!(
        core.sessions.delivery_readiness(id),
        router::DeliveryReadiness::Ready { idle: true }
    );
    assert_eq!(
        core.sessions.agent_status(id).observation.outcome,
        Some(TurnOutcome::Completed)
    );
    assert!(delivery
        .0
        .lock()
        .unwrap()
        .contains(&router::SessionDeliveryEvent::Idle));

    core.sessions.kill(id).unwrap();
}

#[test]
fn codex_conversation_key_change_clears_saved_transcript_interrupted_outcome() {
    use crate::session::state::agent::AgentEvent as A;

    let dir = tempfile::tempdir().unwrap();
    let pool = pool_with_schema();
    let fake = fake_runtime();
    let manager = mgr_with_fake(None, fake.clone());
    let events = capture();
    let mut codex_role = role("codex", &[]);
    codex_role.runtime = Runtime::Codex.key().into();
    insert_role_row(&pool.get().unwrap(), &codex_role);
    let spawned = manager
        .spawn_direct(
            &codex_role,
            DirectLaunch {
                cwd: Some(dir.path().to_str().unwrap().into()),
                ..Default::default()
            },
            dir.path(),
            pool.clone(),
            events.clone(),
        )
        .unwrap();
    let id = &spawned.id;
    let host_events: Arc<dyn SessionEvents> = events.clone();
    let terminal = manager
        .create_terminal(id, 80, 24, &pool, &host_events)
        .unwrap();
    manager.session_state(id).unwrap().lock().unwrap().terminal = Some(terminal);

    let old_key = uuid::Uuid::new_v4().to_string();
    let new_key = uuid::Uuid::new_v4().to_string();
    pool.get()
        .unwrap()
        .execute(
            "UPDATE sessions SET agent_session_key = ?1 WHERE id = ?2",
            rusqlite::params![old_key, id],
        )
        .unwrap();

    // Turn ends Interrupted from transcript and survives trailing idle.
    manager.ingest_output_chunk(
        id,
        None,
        b"\x1b]7501;state=working:app=codex\x1b\\",
        events.as_ref(),
    );
    manager.publish_agent_event(
        id,
        A::ProgramTurnOutcome {
            outcome: TurnOutcome::Interrupted,
            policy: crate::session::state::agent::ProgramOutcomePolicy::Immediate,
        },
        events.as_ref(),
    );
    manager.ingest_output_chunk(
        id,
        None,
        b"\x1b]7501;state=idle:app=codex\x1b\\",
        events.as_ref(),
    );
    assert_eq!(
        manager.agent_status(id).observation.outcome,
        Some(TurnOutcome::Interrupted)
    );

    // An actual conversation-key change (/clear) through output.rs resets the saved outcome
    // so the new conversation's initial idle stays plain Idle.
    events.status.lock().unwrap().clear();
    fake.spawns.lock().unwrap()[0]
        .tx
        .as_ref()
        .unwrap()
        .send(RuntimeOutput::ConversationStart(new_key))
        .unwrap();
    events.wait_for_status(id, |ev| {
        ev.status.observation.activity == Activity::Idle && ev.status.observation.outcome.is_none()
    });
    manager.ingest_output_chunk(
        id,
        None,
        b"\x1b]7501;state=idle:app=codex\x1b\\",
        events.as_ref(),
    );
    assert_eq!(
        manager.agent_status(id).observation.activity,
        Activity::Idle
    );
    assert_eq!(manager.agent_status(id).observation.outcome, None);
    manager.kill(id).unwrap();
}

#[cfg(unix)]
#[test]
fn codex_mid_turn_steer_sharing_turn_id_confirms_once_through_watcher_and_manager() {
    let app_data = tempfile::tempdir().unwrap();
    let role = codex_redraw_role(app_data.path());
    let pool = pool_with_schema();
    let (mission, slot) = seed_mission_rows(&pool, &role);
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
            Some("initial mission prompt".into()),
        )
        .unwrap();
    let id = &spawned.id;
    let host_events: Arc<dyn SessionEvents> = events.clone();
    let terminal = manager
        .create_terminal(id, 80, 24, &pool, &host_events)
        .unwrap();
    manager.session_state(id).unwrap().lock().unwrap().terminal = Some(terminal);
    std::fs::write(app_data.path().join("quiet"), "").unwrap();

    let transcript = app_data.path().join("steer-rollout.jsonl");
    std::fs::write(&transcript, "").unwrap();
    let thread = uuid::Uuid::new_v4().to_string();
    let generation = fixture_generation(app_data.path());
    let admit = |event: &str, turn: Option<&str>| {
        let mut payload = serde_json::json!({
            "hook_event_name": event,
            "session_id": thread,
            "transcript_path": transcript,
        });
        if let Some(turn) = turn {
            payload["turn_id"] = serde_json::json!(turn);
        }
        drop(
            manager
                .admit_hook(runner_core::protocol::hook::HookReport {
                    bridge_unavailable: false,
                    version: runner_core::protocol::hook::VERSION,
                    runtime: Runtime::Codex,
                    session_id: id.clone(),
                    generation: generation.clone(),
                    event: event.into(),
                    caller_thread_id: Some(thread.clone()),
                    payload,
                })
                .unwrap(),
        );
    };

    // First turn completes via OSC 7501 Done.
    let t0 = manager.delivery_snapshot(id).unwrap().turn_started;
    admit("SessionStart", None);
    admit("UserPromptSubmit", Some("turn-1"));
    let t1 = manager.wait_for_turn_started(id, t0);
    assert_eq!(t1, t0 + 1);
    manager.ingest_output_chunk(
        id,
        Some(&mission.id),
        b"\x1b]7501;state=working:app=codex\x1b\\",
        events.as_ref(),
    );
    assert_eq!(manager.delivery_snapshot(id).unwrap().turn_started, t1);
    manager.ingest_output_chunk(
        id,
        Some(&mission.id),
        b"\x1b]7501;state=done:app=codex\x1b\\",
        events.as_ref(),
    );
    assert_eq!(
        manager.delivery_readiness(id),
        router::DeliveryReadiness::Ready { idle: true }
    );

    // Turn 2 starts via raw UserPromptSubmit("turn-2") + OSC 7501 Working.
    admit("UserPromptSubmit", Some("turn-2"));
    let t2 = manager.wait_for_turn_started(id, t1);
    assert_eq!(t2, t1 + 1);
    manager.ingest_output_chunk(
        id,
        Some(&mission.id),
        b"\x1b]7501;state=working:app=codex\x1b\\",
        events.as_ref(),
    );
    assert_eq!(manager.delivery_snapshot(id).unwrap().turn_started, t2);
    assert_eq!(
        manager.delivery_readiness(id),
        router::DeliveryReadiness::Working
    );

    // Codex 0.170 mid-turn steer attaches to the active turn and reuses turn_id="turn-2"
    // while OSC 7501 remains Working (unchanged). Watcher -> manager must confirm once.
    admit("UserPromptSubmit", Some("turn-2"));
    let steered = manager.wait_for_turn_started(id, t2);
    assert_eq!(steered, t2 + 1);
    assert_eq!(
        manager.delivery_readiness(id),
        router::DeliveryReadiness::Working
    );

    // Finish turn-2 via transcript turn_aborted and wait for the status event so we know
    // deterministically that the live watcher marked turn_finished = true for "turn-2".
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&transcript)
            .unwrap();
        writeln!(
            file,
            "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"turn_aborted\",\"turn_id\":\"turn-2\",\"reason\":\"interrupted\"}}}}"
        )
        .unwrap();
        file.flush().unwrap();
    }
    events.wait_for_status(id, |ev| {
        ev.status.observation.outcome == Some(TurnOutcome::Interrupted)
    });

    // Now finished "turn-2" AND retired "turn-1" must both be dropped by the watcher;
    // admitting them ahead of fresh "turn-3" advances turn_started by precisely 1.
    admit("UserPromptSubmit", Some("turn-2"));
    admit("UserPromptSubmit", Some("turn-1"));
    admit("UserPromptSubmit", Some("turn-3"));
    let t3 = manager.wait_for_turn_started(id, steered);
    assert_eq!(
        t3,
        steered + 1,
        "finished turn-2 and retired turn-1 must not increment turn_started alongside turn-3"
    );
    manager.kill(id).unwrap();
}

#[cfg(unix)]
#[test]
fn codex_missed_session_start_user_prompt_submit_rekeys_before_confirming_once() {
    let app_data = tempfile::tempdir().unwrap();
    let role = codex_redraw_role(app_data.path());
    let pool = pool_with_schema();
    insert_crew_role(&pool, "missed-start", &role.id);
    let events = capture();
    let manager = manager_with_runtime(
        Default::default(),
        Arc::new(crate::session::pty_runtime::PtyRuntime::new()),
    );
    let spawned = manager
        .spawn_direct(
            &role,
            DirectLaunch {
                cwd: Some(app_data.path().to_str().unwrap().into()),
                ..Default::default()
            },
            app_data.path(),
            pool.clone(),
            events.clone(),
        )
        .unwrap();
    let id = &spawned.id;
    let host_events: Arc<dyn SessionEvents> = events.clone();
    let terminal = manager
        .create_terminal(id, 80, 24, &pool, &host_events)
        .unwrap();
    manager.session_state(id).unwrap().lock().unwrap().terminal = Some(terminal);
    std::fs::write(app_data.path().join("quiet"), "").unwrap();

    // Persist an older conversation key (e.g. resumed session) and an Interrupted outcome.
    let old_key = uuid::Uuid::new_v4().to_string();
    let new_key = uuid::Uuid::new_v4().to_string();
    pool.get()
        .unwrap()
        .execute(
            "UPDATE sessions SET agent_session_key = ?1 WHERE id = ?2",
            rusqlite::params![old_key, id],
        )
        .unwrap();
    manager.publish_agent_event(
        id,
        crate::session::state::agent::AgentEvent::ProgramTurnOutcome {
            outcome: TurnOutcome::Interrupted,
            policy: crate::session::state::agent::ProgramOutcomePolicy::Immediate,
        },
        events.as_ref(),
    );
    assert_eq!(
        manager.agent_status(id).observation.outcome,
        Some(TurnOutcome::Interrupted)
    );

    // Without any preceding SessionStart, UserPromptSubmit arrives for new_key.
    // ConversationStart(new_key) must precede ProgramTurnConfirmed on the forwarder channel
    // so the key-change reset happens before PendingConfirm is armed, and the following
    // OSC 7501 Working does NOT double-increment turn_started.
    let before = manager.delivery_snapshot(id).unwrap().turn_started;
    let generation = fixture_generation(app_data.path());
    drop(
        manager
            .admit_hook(runner_core::protocol::hook::HookReport {
                bridge_unavailable: false,
                version: runner_core::protocol::hook::VERSION,
                runtime: Runtime::Codex,
                session_id: id.clone(),
                generation,
                event: "UserPromptSubmit".into(),
                caller_thread_id: Some(new_key.clone()),
                payload: serde_json::json!({
                    "hook_event_name": "UserPromptSubmit",
                    "session_id": new_key,
                    "turn_id": "turn-1",
                    "transcript_path": null,
                }),
            })
            .unwrap(),
    );
    let after_hook = manager.wait_for_turn_started(id, before);
    events.wait_for_updated(id);
    assert_eq!(after_hook, before + 1);
    let stored_key = crate::repo::session::get_row(&pool.get().unwrap(), id)
        .unwrap()
        .unwrap()
        .agent_session_key;
    assert_eq!(stored_key.as_deref(), Some(new_key.as_str()));

    manager.ingest_output_chunk(
        id,
        None,
        b"\x1b]7501;state=working:app=codex\x1b\\",
        events.as_ref(),
    );
    assert_eq!(
        manager.delivery_snapshot(id).unwrap().turn_started,
        before + 1,
        "OSC Working after missed-SessionStart UserPromptSubmit must not double-count turn_started"
    );
    assert_eq!(manager.agent_status(id).observation.outcome, None);
    manager.kill(id).unwrap();

    // Inverse order (parser cold on old persisted key AND parser warm on a subsequent
    // missed-SessionStart switch): native OSC Working arrives BEFORE ConversationStart +
    // UserPromptSubmit. Total increments for the turn must stay 1, and a following real
    // mid-turn steer must increment once, checked behind a deterministic consumption fence.
    let app_data2 = tempfile::tempdir().unwrap();
    let role2 = codex_redraw_role(app_data2.path());
    let pool2 = pool_with_schema();
    insert_crew_role(&pool2, "missed-start-inverse", &role2.id);
    let events2 = capture();
    let manager2 = manager_with_runtime(
        Default::default(),
        Arc::new(crate::session::pty_runtime::PtyRuntime::new()),
    );
    let spawned2 = manager2
        .spawn_direct(
            &role2,
            DirectLaunch {
                cwd: Some(app_data2.path().to_str().unwrap().into()),
                ..Default::default()
            },
            app_data2.path(),
            pool2.clone(),
            events2.clone(),
        )
        .unwrap();
    let id2 = &spawned2.id;
    let host_events2: Arc<dyn SessionEvents> = events2.clone();
    let terminal2 = manager2
        .create_terminal(id2, 80, 24, &pool2, &host_events2)
        .unwrap();
    manager2
        .session_state(id2)
        .unwrap()
        .lock()
        .unwrap()
        .terminal = Some(terminal2);
    std::fs::write(app_data2.path().join("quiet"), "").unwrap();

    let persisted_old = uuid::Uuid::new_v4().to_string();
    let second_key = uuid::Uuid::new_v4().to_string();
    let third_key = uuid::Uuid::new_v4().to_string();
    let fourth_key = uuid::Uuid::new_v4().to_string();
    pool2
        .get()
        .unwrap()
        .execute(
            "UPDATE sessions SET agent_session_key = ?1 WHERE id = ?2",
            rusqlite::params![persisted_old, id2],
        )
        .unwrap();
    let generation2 = fixture_generation(app_data2.path());
    let admit_prompt = |session_key: &str, turn: &str| {
        drop(
            manager2
                .admit_hook(runner_core::protocol::hook::HookReport {
                    bridge_unavailable: false,
                    version: runner_core::protocol::hook::VERSION,
                    runtime: Runtime::Codex,
                    session_id: id2.clone(),
                    generation: generation2.clone(),
                    event: "UserPromptSubmit".into(),
                    caller_thread_id: Some(session_key.into()),
                    payload: serde_json::json!({
                        "hook_event_name": "UserPromptSubmit",
                        "session_id": session_key,
                        "turn_id": turn,
                        "transcript_path": null,
                    }),
                })
                .unwrap(),
        );
    };

    // Pass 1: parser cold (DB has persisted_old); Pass 2: parser warm (saw second_key, switches to third_key).
    for target_key in [second_key.as_str(), third_key.as_str()] {
        events2.updated.lock().unwrap().clear();
        let before_turn = manager2.delivery_snapshot(id2).unwrap().turn_started;
        let before_confirmed = manager2.program_turn_confirmed_count(id2);

        // 1. Native OSC Working arrives first and counts the turn once.
        manager2.ingest_output_chunk(
            id2,
            None,
            b"\x1b]7501;state=working:app=codex\x1b\\",
            events2.as_ref(),
        );
        assert_eq!(
            manager2.delivery_snapshot(id2).unwrap().turn_started,
            before_turn + 1
        );

        // 2. Missed-SessionStart UserPromptSubmit for target_key arrives after Working.
        // Wait for BOTH DB rekey (ConversationStart) AND ProgramTurnConfirmed consumption.
        admit_prompt(target_key, "turn-1");
        events2.wait_for_updated(id2);
        let after_first_confirm = manager2.wait_for_program_turn_confirmed(id2, before_confirmed);
        assert_eq!(after_first_confirm, before_confirmed + 1);
        assert_eq!(
            manager2.delivery_snapshot(id2).unwrap().turn_started,
            before_turn + 1,
            "target_key={target_key}: ConversationStart + UserPromptSubmit after Working must keep total increments at 1"
        );
        let stored = crate::repo::session::get_row(&pool2.get().unwrap(), id2)
            .unwrap()
            .unwrap()
            .agent_session_key;
        assert_eq!(stored.as_deref(), Some(target_key));

        // 3. Next real mid-turn steer (sharing turn-1) increments turn_started precisely once.
        admit_prompt(target_key, "turn-1");
        let after_steer = manager2.wait_for_program_turn_confirmed(id2, after_first_confirm);
        assert_eq!(after_steer, after_first_confirm + 1);
        assert_eq!(
            manager2.delivery_snapshot(id2).unwrap().turn_started,
            before_turn + 2,
            "target_key={target_key}: mid-turn steer after inverse-order rekey must increment once"
        );

        // Finish the turn cleanly before the warm-parser switch pass.
        manager2.ingest_output_chunk(
            id2,
            None,
            b"\x1b]7501;state=done:app=codex\x1b\\",
            events2.as_ref(),
        );
        assert_eq!(
            manager2.agent_status(id2).observation.outcome,
            Some(TurnOutcome::Completed)
        );
    }

    // Pass 3 (completed-turn inverse order): native OSC Working -> Done completes BEFORE
    // missed-SessionStart ConversationStart(fourth_key) + UserPromptSubmit arrives.
    // Total increments must stay 1, Completed must NOT be erased by the key-change event,
    // and the next real turn on fourth_key must increment turn_started precisely once.
    events2.updated.lock().unwrap().clear();
    let before_turn = manager2.delivery_snapshot(id2).unwrap().turn_started;
    let before_confirmed = manager2.program_turn_confirmed_count(id2);
    manager2.ingest_output_chunk(
        id2,
        None,
        b"\x1b]7501;state=working:app=codex\x1b\\",
        events2.as_ref(),
    );
    manager2.ingest_output_chunk(
        id2,
        None,
        b"\x1b]7501;state=done:app=codex\x1b\\",
        events2.as_ref(),
    );
    assert_eq!(
        manager2.delivery_snapshot(id2).unwrap().turn_started,
        before_turn + 1
    );
    assert_eq!(
        manager2.agent_status(id2).observation.outcome,
        Some(TurnOutcome::Completed)
    );
    admit_prompt(&fourth_key, "turn-1");
    events2.wait_for_updated(id2);
    let after_confirm = manager2.wait_for_program_turn_confirmed(id2, before_confirmed);
    assert_eq!(after_confirm, before_confirmed + 1);
    assert_eq!(
        manager2.delivery_snapshot(id2).unwrap().turn_started,
        before_turn + 1,
        "Working -> Done -> ConversationStart + UserPromptSubmit must keep total increments at 1"
    );
    assert_eq!(
        manager2.agent_status(id2).observation.outcome,
        Some(TurnOutcome::Completed),
        "key-change binding after Working -> Done must retain the new conversation's Completed outcome"
    );
    let stored = crate::repo::session::get_row(&pool2.get().unwrap(), id2)
        .unwrap()
        .unwrap()
        .agent_session_key;
    assert_eq!(stored.as_deref(), Some(fourth_key.as_str()));

    // Next real turn on fourth_key (hook first -> Working) increments turn_started precisely once.
    admit_prompt(&fourth_key, "turn-2");
    let after_next = manager2.wait_for_program_turn_confirmed(id2, after_confirm);
    assert_eq!(after_next, after_confirm + 1);
    assert_eq!(
        manager2.delivery_snapshot(id2).unwrap().turn_started,
        before_turn + 2
    );
    manager2.ingest_output_chunk(
        id2,
        None,
        b"\x1b]7501;state=working:app=codex\x1b\\",
        events2.as_ref(),
    );
    assert_eq!(
        manager2.delivery_snapshot(id2).unwrap().turn_started,
        before_turn + 2
    );

    manager2.kill(id2).unwrap();
}
