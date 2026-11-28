use super::*;

#[cfg(windows)]
#[test]
fn windows_batch_first_turn_waits_for_split_tui_readiness() {
    assert_windows_batch_first_turn(Runtime::Cursor.key(), "ready", None);
}

#[cfg(windows)]
#[test]
fn windows_batch_first_turn_falls_back_without_tui_output() {
    assert_windows_batch_first_turn(Runtime::ClaudeCode.key(), "fallback", None);
}

#[cfg(windows)]
#[test]
fn windows_batch_codex_first_turn_retries_swallowed_enter() {
    assert_windows_batch_first_turn(
        Runtime::Codex.key(),
        "retry",
        Some(crate::session::manager::first_turn::FirstTurnTimings {
            submit_delay: Duration::from_millis(400),
            confirm_window: Duration::from_millis(400),
            submit_timeout: Duration::from_secs(10),
        }),
    );
}

#[cfg(windows)]
fn assert_windows_batch_first_turn(
    runtime_key: &str,
    mode: &str,
    timings: Option<crate::session::manager::first_turn::FirstTurnTimings>,
) {
    let dir = tempfile::tempdir().unwrap();
    let batch = dir.path().join("prompt reader.cmd");
    std::fs::write(&batch,
        "@echo off\r\n\"%RUNNER_BATCH_PROMPT_EXE%\" --exact session::manager::tests::windows_batch_prompt_probe --nocapture\r\n").unwrap();
    let mut role = role(batch.to_str().unwrap(), &[]);
    role.runtime = runtime_key.into();
    role.env.insert(
        "RUNNER_BATCH_PROMPT_EXE".into(),
        std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    );
    role.env
        .insert("RUNNER_BATCH_PROMPT_MODE".into(), mode.into());
    let pool = pool_with_schema();
    insert_crew_role(&pool, "batch-prompt", &role.id);
    let mgr = manager_with_runtime(
        Default::default(),
        Arc::new(crate::session::pty_runtime::PtyRuntime::new()),
    );
    if let Some(timings) = timings {
        mgr.set_first_turn_timings(timings);
    }
    let events = capture_for(&mgr);
    let spawned = mgr
        .spawn_direct(
            &role,
            DirectLaunch {
                cwd: Some(dir.path().to_str().unwrap().into()),
                first_turn: Some("first line\nsecond line".into()),
                ..Default::default()
            },
            dir.path(),
            Arc::clone(&pool),
            events.clone(),
        )
        .unwrap();
    let expected_marker = (mode == "retry").then(|| {
        crate::runtimes::codex::capture::mark_prompt(
            &spawned.id,
            Some("first line\nsecond line".into()),
        )
        .1
        .expect("codex first turn carries capture marker")
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let output = loop {
        let bytes = events
            .output
            .lock()
            .unwrap()
            .iter()
            .flat_map(|event| event.bytes.iter().copied())
            .collect::<Vec<_>>();
        let output = String::from_utf8_lossy(&bytes).into_owned();
        if output.contains("BATCH_INPUT_FIRST=first line")
            && output.contains("BATCH_INPUT_SECOND=second line")
            && expected_marker.as_deref().is_none_or(|marker| {
                output.contains(&format!("BATCH_INPUT_MARKER={marker}"))
                    && output.contains("BATCH_RETRY_ENTERS=2")
            })
            || Instant::now() >= deadline
        {
            break output;
        }
        thread::sleep(Duration::from_millis(20));
    };
    mgr.kill(&spawned.id).unwrap();
    assert!(output.contains("BATCH_INPUT_FIRST=first line"), "{output}");
    assert!(
        output.contains("BATCH_INPUT_SECOND=second line"),
        "{output}"
    );
    if let Some(marker) = expected_marker {
        assert!(
            output.contains(&format!("BATCH_INPUT_MARKER={marker}")),
            "{output}"
        );
        assert!(output.contains("BATCH_RETRY_ENTERS=2"), "{output}");
    }
}

#[cfg(windows)]
#[test]
fn windows_batch_slot_restart_queues_first_turn_without_argv() {
    let (pool, app_data, id) = slot_respawn_fixture("claude-code", false);
    let conn = pool.get().unwrap();
    let mut role = crate::repo::role::list(&conn).unwrap().pop().unwrap();
    role.command = "agent.cmd".into();
    update_role_row(&conn, &role);
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());
    mgr.restart(&id, None, None, app_data.path(), pool.clone(), capture())
        .unwrap();
    assert!(!fake
        .last_spawn_spec()
        .unwrap()
        .args
        .iter()
        .any(|arg| arg.contains("SLOT_BRIEF")));
    let state = mgr.session_state(&id).unwrap();
    let body = state
        .lock()
        .unwrap()
        .handle
        .as_ref()
        .unwrap()
        .pending_first_turn
        .as_ref()
        .unwrap()
        .body
        .clone();
    assert_eq!(
        body,
        router::prompt::compose_member_first_turn(
            Some("SLOT_BRIEF"),
            Some("TEAM_RULES"),
            Some("LATEST_GOAL")
        )
    );
    mgr.kill(&id).unwrap();
}
