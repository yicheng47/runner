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
        "AGY_CLI_DISABLE_AUTO_UPDATE",
        "DISABLE_INSTALLATION_CHECKS",
        "CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY",
        "PI_SKIP_VERSION_CHECK",
        "RUNNER_CLAUDE_STATUS_PATH",
        "RUNNER_CODEX_STATUS_PATH",
    ] {
        assert!(!spec.env.contains_key(runner_layer), "{runner_layer}");
    }
    assert!(!spec.env.keys().any(|key| key.starts_with("RUNNER_")));
}

#[test]
fn runtime_update_spec_allows_live_sessions_without_stopping_or_restarting_them() {
    let bin = tempfile::tempdir().unwrap();
    let executable = bin.path().join("agent.exe");
    std::fs::write(&executable, "").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let command = executable.to_str().unwrap();

    let mut saw_antigravity = false;
    for definition in crate::runtimes::catalogs()
        .iter()
        .filter(|definition| !definition.update_args.is_empty())
    {
        saw_antigravity |= definition.name == Runtime::Antigravity;
        let mut state = crate::test_support::test_core();
        crate::test_support::isolate_runtime_path(&state, None);
        let fake = fake_runtime();
        state.sessions = SessionManager::new(
            Arc::clone(&state.runtime_shell_env),
            Arc::clone(&state.runtime_discovery),
            Arc::clone(&fake) as Arc<dyn SessionRuntime>,
        );
        crate::db::set_runtime_override(&state.db, definition.name.key(), Some(command)).unwrap();
        for running in 0..=2 {
            if running > 0 {
                let id = format!("running-{running}");
                let mut row = crate::test_support::test_session_row(
                    &id,
                    crate::model::SessionStatus::Running,
                );
                row.agent_runtime = Some(definition.name.to_string());
                crate::test_support::insert_session_row(&state.db.get().unwrap(), &row).unwrap();
                install_test_session_handle(&state.sessions, &id);
            }
            let live = state.sessions.live_session_ids();
            assert_eq!(
                crate::ops::session::live_session_counts(&state)
                    .unwrap()
                    .get(&definition.name)
                    .copied()
                    .unwrap_or(0),
                running
            );
            let spec =
                crate::ops::runtime::runtime_update_spawn_spec(&state, definition.name, (90, 28))
                    .unwrap();
            assert_eq!(spec.command, command);
            assert_eq!(spec.args, definition.update_args);
            assert_eq!(spec.cwd, runner_core::app_paths::home_dir());
            assert_eq!(spec.initial_size, Some((90, 28)));
            assert!(!spec
                .env
                .keys()
                .any(|key| key.eq_ignore_ascii_case("AGY_CLI_DISABLE_AUTO_UPDATE")));
            if definition.name == Runtime::Antigravity {
                assert_eq!(spec.env_remove, ["AGY_CLI_DISABLE_AUTO_UPDATE"]);
            } else {
                assert!(spec.env_remove.is_empty());
            }
            assert_eq!(state.sessions.live_session_ids(), live);
            assert!(fake.stops.lock().unwrap().is_empty());
            assert_eq!(fake.spawn_count(), 0);
        }
    }
    assert!(saw_antigravity);
}

#[test]
fn empty_mission_env_cannot_inherit_a_crew_from_the_shell_or_role() {
    let env = spawn::agent_env(
        BTreeMap::from([("RUNNER_CREW_ID".into(), "shell-crew".into())]),
        &HashMap::from([("RUNNER_CREW_ID".into(), "role-crew".into())]),
        BTreeMap::from([("RUNNER_MISSION_ID".into(), "empty-mission".into())]),
        "shell",
    );
    assert!(!env.contains_key("RUNNER_CREW_ID"));
    assert_eq!(env["RUNNER_MISSION_ID"], "empty-mission");
}

#[test]
fn sessions_keep_the_quiet_flags_the_update_drops() {
    let claude = spawn::agent_env(
        BTreeMap::new(),
        &HashMap::new(),
        BTreeMap::new(),
        Runtime::ClaudeCode.key(),
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
        Runtime::Pi.key(),
    );
    assert_eq!(
        pi.get("PI_SKIP_VERSION_CHECK").map(String::as_str),
        Some("1")
    );
    let antigravity = spawn::agent_env(
        BTreeMap::new(),
        &HashMap::new(),
        BTreeMap::new(),
        Runtime::Antigravity.key(),
    );
    assert_eq!(
        antigravity
            .get("AGY_CLI_DISABLE_AUTO_UPDATE")
            .map(String::as_str),
        cfg!(windows).then_some("true")
    );
}

#[test]
fn apply_launch_env_strips_case_insensitive_aliases_when_folding_case() {
    let mut windows_env = BTreeMap::from([
        ("agy_cli_disable_auto_update".into(), String::new()),
        ("Agy_Cli_Disable_Auto_Update".into(), "false".into()),
        ("OTHER".into(), "keep".into()),
    ]);
    spawn::apply_launch_env(
        &mut windows_env,
        &[("AGY_CLI_DISABLE_AUTO_UPDATE", "true")],
        true,
    );
    let matches: Vec<_> = windows_env
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("AGY_CLI_DISABLE_AUTO_UPDATE"))
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(matches, [("AGY_CLI_DISABLE_AUTO_UPDATE", "true")]);
    assert_eq!(windows_env.get("OTHER").map(String::as_str), Some("keep"));

    let mut unix_env = BTreeMap::from([("agy_cli_disable_auto_update".into(), String::new())]);
    spawn::apply_launch_env(
        &mut unix_env,
        &[("AGY_CLI_DISABLE_AUTO_UPDATE", "true")],
        false,
    );
    assert_eq!(
        unix_env
            .get("AGY_CLI_DISABLE_AUTO_UPDATE")
            .map(String::as_str),
        Some("true")
    );
    assert_eq!(
        unix_env
            .get("agy_cli_disable_auto_update")
            .map(String::as_str),
        Some("")
    );
}

#[test]
fn spawn_spec_remove_env_strips_case_aliases_when_folding_case() {
    let mut win_spec = SpawnSpec {
        env: BTreeMap::from([
            ("AGY_CLI_DISABLE_AUTO_UPDATE".into(), "true".into()),
            ("agy_cli_disable_auto_update".into(), "true".into()),
            ("Agy_Cli_Disable_Auto_Update".into(), "false".into()),
            ("OTHER".into(), "keep".into()),
        ]),
        ..Default::default()
    };
    win_spec.remove_env("AGY_CLI_DISABLE_AUTO_UPDATE", true);
    win_spec.remove_env("agy_cli_disable_auto_update", true);
    assert!(!win_spec
        .env
        .keys()
        .any(|k| k.eq_ignore_ascii_case("AGY_CLI_DISABLE_AUTO_UPDATE")));
    assert_eq!(win_spec.env.get("OTHER").map(String::as_str), Some("keep"));
    assert_eq!(win_spec.env_remove, ["AGY_CLI_DISABLE_AUTO_UPDATE"]);

    let mut unix_spec = SpawnSpec {
        env: BTreeMap::from([
            ("AGY_CLI_DISABLE_AUTO_UPDATE".into(), "true".into()),
            ("agy_cli_disable_auto_update".into(), "keep".into()),
        ]),
        ..Default::default()
    };
    unix_spec.remove_env("AGY_CLI_DISABLE_AUTO_UPDATE", false);
    assert!(!unix_spec.env.contains_key("AGY_CLI_DISABLE_AUTO_UPDATE"));
    assert_eq!(
        unix_spec
            .env
            .get("agy_cli_disable_auto_update")
            .map(String::as_str),
        Some("keep")
    );
    assert_eq!(unix_spec.env_remove, ["AGY_CLI_DISABLE_AUTO_UPDATE"]);
}

#[cfg(windows)]
#[test]
fn windows_antigravity_agent_env_replaces_case_insensitive_auto_update_aliases() {
    let env = spawn::agent_env(
        BTreeMap::from([("Agy_Cli_Disable_Auto_Update".into(), "false".into())]),
        &HashMap::from([("agy_cli_disable_auto_update".into(), String::new())]),
        BTreeMap::new(),
        Runtime::Antigravity.key(),
    );
    let matches: Vec<_> = env
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("AGY_CLI_DISABLE_AUTO_UPDATE"))
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(matches, [("AGY_CLI_DISABLE_AUTO_UPDATE", "true")]);
}

#[test]
fn unlisted_pty_reports_only_to_its_listener_and_leaves_no_trace() {
    let fake = fake_runtime();
    let mgr = proxied_manager(&fake);
    let pool = pool_with_schema();
    let spec = mgr.update_spawn_spec("/bin/codex".into(), vec!["update".into()], None, (80, 24));
    let session_id = spec.session_id.clone();
    let listener = capture_for(&mgr);
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
