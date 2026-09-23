use super::*;

fn proxied_manager(fake: &Arc<FakeRuntime>) -> Arc<SessionManager> {
    let mut vars = BTreeMap::new();
    vars.insert("HTTPS_PROXY".into(), "http://login-shell:7890".into());
    vars.insert("NO_PROXY".into(), "localhost".into());
    manager_with_runtime(
        crate::shell_path::LoginShellEnv {
            path: Some("/login/bin".into()),
            vars,
        },
        Arc::clone(fake) as Arc<dyn SessionRuntime>,
    )
}

#[test]
fn update_spec_runs_the_update_in_home_without_runner_layers() {
    let fake = fake_runtime();
    let mgr = proxied_manager(&fake);
    let home = PathBuf::from("/Users/tester");
    let spec = mgr.update_spawn_spec(
        "/Users/tester/.local/bin/claude".into(),
        vec!["update".into()],
        Some(home.clone()),
        (100, 30),
    );
    assert_eq!(spec.command, "/Users/tester/.local/bin/claude");
    assert_eq!(spec.args, ["update"]);
    assert_eq!(spec.cwd, Some(home));
    assert_eq!(spec.initial_size, Some((100, 30)));
    assert_eq!(spec.shell_path.as_deref(), Some("/login/bin"));
    assert!(!spec.mission);
    assert!(spec.shim_dir.is_none());
    assert!(spec.bundled_bin_dir.is_none());
    assert!(ulid::Ulid::from_string(&spec.session_id).is_ok());
    assert_eq!(
        spec.env.get("HTTPS_PROXY").map(String::as_str),
        Some("http://login-shell:7890")
    );
    assert_eq!(
        spec.env.get("NO_PROXY").map(String::as_str),
        Some("localhost")
    );
    assert_eq!(
        spec.env.get("TERM").map(String::as_str),
        Some("xterm-256color")
    );
    for runner_layer in [
        "DISABLE_INSTALLATION_CHECKS",
        "CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY",
        "PI_SKIP_VERSION_CHECK",
        crate::session::claude_status::PATH_ENV,
        crate::session::codex_status::PATH_ENV,
    ] {
        assert!(!spec.env.contains_key(runner_layer), "{runner_layer}");
    }
    assert!(!spec.env.keys().any(|key| key.starts_with("RUNNER_")));
}

#[test]
fn sessions_keep_the_quiet_flags_the_update_drops() {
    let claude = spawn::agent_env(
        BTreeMap::new(),
        &HashMap::new(),
        BTreeMap::new(),
        Some(Runtime::ClaudeCode),
    );
    assert_eq!(
        claude
            .get("DISABLE_INSTALLATION_CHECKS")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        claude
            .get("CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY")
            .map(String::as_str),
        Some("1")
    );
    let pi = spawn::agent_env(
        BTreeMap::new(),
        &HashMap::new(),
        BTreeMap::new(),
        Some(Runtime::Pi),
    );
    assert_eq!(
        pi.get("PI_SKIP_VERSION_CHECK").map(String::as_str),
        Some("1")
    );
}

#[test]
fn unlisted_pty_reports_only_to_its_listener_and_leaves_no_trace() {
    let fake = fake_runtime();
    let mgr = proxied_manager(&fake);
    let pool = pool_with_schema();
    let spec = mgr.update_spawn_spec("/bin/codex".into(), vec!["update".into()], None, (80, 24));
    let session_id = spec.session_id.clone();
    let listener = capture();
    mgr.spawn_unlisted(spec, &pool, Arc::clone(&listener) as Arc<dyn SessionEvents>)
        .unwrap();

    assert_eq!(fake.spawn_count(), 1);
    assert_eq!(mgr.live_session_ids(), std::slice::from_ref(&session_id));
    mgr.inject_direct_stdin(&session_id, b"y", listener.as_ref())
        .unwrap();
    assert!(fake.inputs.lock().unwrap().contains(&FakeInput::Bytes {
        session_id: session_id.clone(),
        bytes: b"y".to_vec(),
    }));

    fake.push_output(0, b"added 1 package\r\n");
    fake.push_status(0, SessionActivityState::Idle);
    wait_for_output_event(&listener, &session_id);
    fake.set_status_exit_code(Some(243));
    fake.close_spawn(0);
    let deadline = Instant::now() + Duration::from_secs(10);
    while listener.exit.lock().unwrap().is_empty() {
        assert!(
            Instant::now() < deadline,
            "unlisted PTY never reported exit"
        );
        thread::sleep(Duration::from_millis(10));
    }

    let exit = listener.exit.lock().unwrap()[0].clone();
    assert_eq!(exit.session_id, session_id);
    assert_eq!(exit.exit_code, Some(243));
    assert!(!exit.success);
    assert!(listener.status.lock().unwrap().is_empty());
    assert!(mgr.live_session_ids().is_empty());
    assert!(!mgr.status_snapshot().contains_key(&session_id));
    assert!(!mgr.activity_snapshot().contains_key(&session_id));
    let conn = pool.get().unwrap();
    assert!(crate::repo::session::get_row(&conn, &session_id)
        .unwrap()
        .is_none());
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0);
}
