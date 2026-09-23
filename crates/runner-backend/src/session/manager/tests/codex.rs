use super::*;

#[test]
fn codex_spawn_composes_hooks_without_changing_user_home_and_respects_overrides() {
    use crate::session::codex_status::{GENERATION_ENV, PATH_ENV};
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
                codex_pending_turn: None,
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
            SessionManager::apply_runtime_args(
                &mut spec,
                &role,
                &router::runtime::resume_plan(Some(Runtime::Codex), key),
                root.path(),
                None,
                Some("first turn"),
                None,
            );
            assert_eq!(spec.env["CODEX_HOME"], "user home");
            assert_eq!(spec.codex_pending_turn, Some(key.is_none()));
            let injected = args.is_empty();
            assert_eq!(spec.env.contains_key(PATH_ENV), injected);
            assert_eq!(spec.env.contains_key(GENERATION_ENV), injected);
            assert_eq!(
                spec.args
                    .iter()
                    .any(|arg| arg.starts_with("hooks.UserPromptSubmit=")),
                injected
            );
            assert!(!spec
                .env
                .contains_key(crate::session::claude_status::PATH_ENV));
            if injected {
                let generation = spec.env[GENERATION_ENV].clone();
                assert!(uuid::Uuid::parse_str(&generation).is_ok());
                generations.push(generation);
                assert_eq!(
                    spec.env[PATH_ENV],
                    crate::session::hook_feed::hook_path(&crate::session::hook_feed::status_path(
                        root.path(),
                        "codex-spawn"
                    ))
                );
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
    use crate::session::codex_status::CodexStatusWatcher;
    use std::io::Write;
    let core = crate::test_support::test_core();
    core.db.get().unwrap().execute("INSERT INTO sessions(id, status, agent_runtime) VALUES ('codex-status', 'running', 'codex')", []).unwrap();
    install_test_session_handle(&core.sessions, "codex-status");
    core.sessions.note_forwarder_transition(
        "codex-status",
        SessionActivityState::Busy,
        "forwarder",
    );
    let path = core.app_data_dir.join("codex-delivery.ndjson");
    let mut watcher = CodexStatusWatcher::start(&path, "current".into()).unwrap();
    for event in [
        "UserPromptSubmit",
        "PreToolUse",
        "PermissionRequest",
        "Interrupt",
        "PostToolUse",
        "Stop",
    ] {
        writeln!(std::fs::OpenOptions::new().append(true).open(&path).unwrap(), "{}", serde_json::json!({"generation":"current","hook_event_name":event,"session_id":"main","turn_id":"one","tool_name":"request_user_input","tool_use_id":"question"})).unwrap();
    }
    let events = core.session_events();
    let mut count = 0;
    watcher
        .drain_observations(|value, _| {
            count += 1;
            core.sessions
                .publish_observation("codex-status", value.clone(), &events);
            let token = match core.sessions.reserve_delivery("codex-status").unwrap() {
                router::DeliveryReservation::Ready(token) => token,
                other => panic!("{other:?}"),
            };
            core.sessions.finish_delivery("codex-status", token);
            let state = core.sessions.session_state("codex-status").unwrap();
            state.lock().unwrap().local_input_pending = true;
            core.sessions
                .publish_observation("codex-status", value, &events);
            assert_eq!(
                core.sessions.reserve_delivery("codex-status").unwrap(),
                router::DeliveryReservation::PendingInput
            );
            assert!(state.lock().unwrap().local_input_pending);
            state.lock().unwrap().local_input_pending = false;
        })
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
printf '%s' "$RUNNER_CODEX_STATUS_GENERATION" > "$FIXTURE_ROOT/generation"
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

#[cfg(unix)]
fn append_codex_fixture_hook(root: &Path, id: &str, event: &str, turn: &str) {
    use std::io::Write;
    let generation = std::fs::read_to_string(root.join("generation")).unwrap();
    let path = crate::session::hook_feed::status_path(root, id);
    writeln!(
        std::fs::OpenOptions::new().append(true).open(path).unwrap(),
        "{}",
        serde_json::json!({
            "generation": generation, "session_id": "fixture-conversation", "turn_id": turn,
            "hook_event_name": event, "source": "startup"
        })
    )
    .unwrap();
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
        let events = capture();
        let manager = manager_with_runtime(
            Default::default(),
            Arc::new(crate::session::pty_runtime::PtyRuntime::new()),
        );
        let spawned = if launch == "keyed-mission" {
            let (mission, slot) = seed_mission_rows(&pool, &role);
            let mut row = crate::repo::session::SessionRowDb::new_running("resumed-slot".into());
            row.status = crate::model::SessionStatus::Stopped;
            row.mission_id = Some(mission.id);
            row.slot_id = Some(slot.id);
            row.role_id = Some(role.id.clone());
            row.cwd = Some(app_data.path().to_string_lossy().into());
            row.agent_session_key = Some(uuid::Uuid::new_v4().to_string());
            crate::repo::session::insert(&pool.get().unwrap(), &row).unwrap();
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
        let hook_bytes = std::fs::metadata(crate::session::hook_feed::status_path(
            app_data.path(),
            &spawned.id,
        ))
        .unwrap()
        .len();
        let armed = manager
            .session_state(&spawned.id)
            .unwrap()
            .lock()
            .unwrap()
            .hook_status_armed;
        let args = std::fs::read_to_string(app_data.path().join("args")).unwrap();
        manager.kill(&spawned.id).unwrap();
        assert_eq!(hook_bytes, 0, "{launch}");
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
                assert_eq!(
                    manager.reserve_delivery(id).unwrap(),
                    router::DeliveryReservation::PendingInput
                );
                manager
                    .inject_direct_stdin(id, b"\r", events.as_ref())
                    .unwrap();
            }
            "router" => {
                let token = match manager.reserve_delivery(id).unwrap() {
                    router::DeliveryReservation::Ready(token) => token,
                    other => panic!("{other:?}"),
                };
                assert!(manager.inject_reserved(id, token, b"routed work").unwrap());
                assert!(manager.inject_reserved(id, token, b"\r").unwrap());
                manager.finish_delivery(id, token);
            }
            _ => manager.inject_paste(id, b"automatic paste").unwrap(),
        }
        wait_for_observation(&manager, id, Activity::Working, ObservationSource::Baseline);
        append_codex_fixture_hook(app_data.path(), id, "SessionStart", "one");
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
        append_codex_fixture_hook(app_data.path(), id, "UserPromptSubmit", "one");
        wait_for_observation(&manager, id, Activity::Working, ObservationSource::Hook);
        thread::sleep(Duration::from_millis(2300));
        assert_eq!(
            manager.agent_status(id).observation.activity,
            Activity::Working
        );
        std::fs::remove_file(app_data.path().join("quiet")).unwrap();
        append_codex_fixture_hook(app_data.path(), id, "Stop", "one");
        wait_for_observation(&manager, id, Activity::Ready, ObservationSource::Hook);
        assert_eq!(
            manager.agent_status(id).observation.outcome,
            Some(TurnOutcome::Completed)
        );
        manager
            .inject_direct_stdin(id, b"\r", events.as_ref())
            .unwrap();
        append_codex_fixture_hook(app_data.path(), id, "UserPromptSubmit", "two");
        wait_for_observation(&manager, id, Activity::Working, ObservationSource::Hook);
        append_codex_fixture_hook(app_data.path(), id, "Interrupt", "two");
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
    use std::io::Write;
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
    let feed = crate::session::hook_feed::status_path(app_data.path(), id);
    assert_eq!(std::fs::metadata(&feed).unwrap().len(), 0);
    let generation = std::fs::read_to_string(app_data.path().join("generation")).unwrap();
    writeln!(
        std::fs::OpenOptions::new().append(true).open(feed).unwrap(),
        "{}",
        serde_json::json!({
            "generation": generation, "session_id": "fixture-conversation", "turn_id": "one",
            "hook_event_name": "Interrupt", "transcript_path": transcript,
        })
    )
    .unwrap();
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
    append_codex_fixture_hook(app_data.path(), id, "UserPromptSubmit", "two");
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
        let events = capture();
        let manager = manager_with_runtime(
            Default::default(),
            Arc::new(crate::session::pty_runtime::PtyRuntime::new()),
        );
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
        append_codex_fixture_hook(app_data.path(), &spawned.id, "SessionStart", "one");
        thread::sleep(Duration::from_millis(2300));
        let observation = manager.agent_status(&spawned.id).observation;
        let args = std::fs::read_to_string(app_data.path().join("args")).unwrap();
        append_codex_fixture_hook(app_data.path(), &spawned.id, "UserPromptSubmit", "one");
        wait_for_observation(
            &manager,
            &spawned.id,
            Activity::Working,
            ObservationSource::Hook,
        );
        append_codex_fixture_hook(app_data.path(), &spawned.id, "Stop", "one");
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
            std::fs::remove_file(crate::session::hook_feed::status_path(app_data.path(), id))
                .unwrap();
        }
        std::fs::write(app_data.path().join("quiet"), "").unwrap();
        wait_for_observation(&manager, id, Activity::Idle, ObservationSource::Baseline);
        let observation = manager.agent_status(id).observation;
        let armed = manager
            .session_state(id)
            .unwrap()
            .lock()
            .unwrap()
            .hook_status_armed;
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
    SessionManager::apply_runtime_args(
        &mut spec,
        &role,
        &router::runtime::resume_plan(Some(Runtime::Codex), None),
        root.path(),
        None,
        Some("automatic prompt"),
        None,
    );
    assert_eq!(spec.codex_pending_turn, Some(true));
    assert!(!spec.args.iter().any(|arg| arg == "automatic prompt"));
}
