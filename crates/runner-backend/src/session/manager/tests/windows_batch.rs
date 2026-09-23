use super::*;

#[cfg(windows)]
#[test]
fn windows_batch_first_turn_waits_for_split_tui_readiness() {
    assert_windows_batch_first_turn("ready");
}

#[cfg(windows)]
#[test]
fn windows_batch_first_turn_falls_back_without_tui_output() {
    assert_windows_batch_first_turn("fallback");
}

#[cfg(windows)]
fn assert_windows_batch_first_turn(mode: &str) {
    let dir = tempfile::tempdir().unwrap();
    let batch = dir.path().join("prompt reader.cmd");
    std::fs::write(&batch,
        "@echo off\r\n\"%RUNNER_BATCH_PROMPT_EXE%\" --exact session::manager::tests::windows_batch_prompt_probe --nocapture\r\n").unwrap();
    let mut role = role(batch.to_str().unwrap(), &[]);
    role.runtime = "claude-code".into();
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
    let events = capture();
    let mgr = manager_with_runtime(
        Default::default(),
        Arc::new(crate::session::pty_runtime::PtyRuntime::new()),
    );
    let spawned = mgr
        .spawn_direct(
            &role,
            None,
            None,
            None,
            None,
            Some(dir.path().to_str().unwrap()),
            None,
            None,
            dir.path(),
            Arc::clone(&pool),
            events.clone(),
            Some("first line\nsecond line".into()),
        )
        .unwrap();
    // The ConPTY host asks the terminal for the cursor position and
    // device attributes as soon as it starts and holds the client for
    // seconds when nothing answers. The app's terminal answers these;
    // this test has no terminal, so it stands in for one (#524).
    let mut host_handshake_answered = false;
    let deadline = Instant::now() + Duration::from_secs(15);
    let output = loop {
        let bytes = events
            .output
            .lock()
            .unwrap()
            .iter()
            .flat_map(|event| event.bytes.iter().copied())
            .collect::<Vec<_>>();
        if !host_handshake_answered && bytes.windows(4).any(|w| w == b"\x1b[6n") {
            mgr.inject_stdin(&spawned.id, b"\x1b[1;1R\x1b[?6c").unwrap();
            host_handshake_answered = true;
        }
        let output = String::from_utf8_lossy(&bytes).into_owned();
        if output.contains("BATCH_INPUT_FIRST=first line")
            && output.contains("BATCH_INPUT_SECOND=second line")
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
        router::prompt::compose_worker_first_turn(Some("SLOT_BRIEF"), Some("TEAM_RULES"))
    );
    mgr.kill(&id).unwrap();
}
