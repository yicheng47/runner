use super::*;

#[cfg(unix)]
fn fork_materializer(stdout: &str, exit_code: i32) -> (tempfile::TempDir, String, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let command = dir.path().join("fork-materializer");
    let capture_path = dir.path().join("capture.txt");
    std::fs::write(
        &command,
        format!(
            "#!/bin/sh\n{{\n  printf 'cwd=%s\\n' \"$PWD\"\n  printf 'env=%s\\n' \"$FORK_TEST_ENV\"\n  for arg in \"$@\"; do printf 'arg=%s\\n' \"$arg\"; done\n}} > \"{}\"\nprintf '%s\\n' '{}'\nexit {exit_code}\n",
            capture_path.display(),
            stdout,
        ),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&command).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&command, permissions).unwrap();
    (dir, command.to_string_lossy().into_owned(), capture_path)
}

#[cfg(unix)]
fn codex_fork_materializer(
    source_key: &str,
    fork_key: &str,
    create_rollout: bool,
) -> (tempfile::TempDir, String, PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let command = dir.path().join("codex-fork-materializer");
    let capture_path = dir.path().join("capture.txt");
    let codex_home = dir.path().join("codex-home");
    let now = chrono::Local::now();
    let rollout_dir = codex_home
        .join("sessions")
        .join(now.format("%Y").to_string())
        .join(now.format("%m").to_string())
        .join(now.format("%d").to_string());
    std::fs::create_dir_all(&rollout_dir).unwrap();
    let rollout_path = rollout_dir.join(format!("rollout-test-{fork_key}.jsonl"));
    let event = format!(r#"{{"type":"thread.started","thread_id":"{fork_key}"}}"#);
    let session_meta = serde_json::json!({
        "type": "session_meta",
        "payload": {"id": fork_key, "forked_from_id": source_key},
    });
    let create_rollout = if create_rollout {
        format!(
            "printf '%s\\n' '{}' > \"{}\"\n",
            session_meta,
            rollout_path.display()
        )
    } else {
        String::new()
    };
    std::fs::write(
        &command,
        format!(
            "#!/bin/sh\n{{\n  printf 'cwd=%s\\n' \"$PWD\"\n  printf 'env=%s\\n' \"$FORK_TEST_ENV\"\n  for arg in \"$@\"; do printf 'arg=%s\\n' \"$arg\"; done\n}} > \"{}\"\nprintf '%s\\n' '{}'\n{create_rollout}sleep 60\n",
            capture_path.display(),
            event,
        ),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&command).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&command, permissions).unwrap();
    (
        dir,
        command.to_string_lossy().into_owned(),
        capture_path,
        codex_home,
    )
}

#[cfg(unix)]
struct RepairingCapture {
    pool: Arc<DbPool>,
    updated: Mutex<Vec<SessionUpdatedEvent>>,
}

#[cfg(unix)]
impl SessionEvents for RepairingCapture {
    fn output(&self, _ev: &OutputEvent) {}

    fn exit(&self, _ev: &ExitEvent) {}

    fn updated(&self, ev: &SessionUpdatedEvent) {
        self.updated.lock().unwrap().push(ev.clone());
        let mut conn = self.pool.get().unwrap();
        crate::repo::node::list_with_repair(&mut conn).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn claude_direct_chat_fork_spawns_tui_directly_with_copied_row() {
    let pool = pool_with_schema();
    let role_id = ulid::Ulid::new().to_string();
    let source_id = ulid::Ulid::new().to_string();
    let source_key = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    let command = "claude-custom".to_string();
    let project_id = {
        let conn = pool.get().unwrap();
        let project = crate::repo::project::create(&conn, "Runner", "/tmp").unwrap();
        let mut persisted_role = role(
            &command,
            &[
                "--permission-mode",
                "auto",
                "--role-flag",
                "--dangerously-skip-permissions",
            ],
        );
        persisted_role.id = role_id.clone();
        persisted_role.handle = "forker".into();
        persisted_role.runtime = "claude-code".into();
        persisted_role
            .env
            .insert("FORK_TEST_ENV".into(), "same-env".into());
        persisted_role.working_dir = Some("/tmp".into());
        persisted_role.system_prompt = Some("Forker persona".into());
        insert_role_row(&conn, &persisted_role);
        let mut row = crate::repo::session::SessionRowDb::new_running(source_id.clone());
        row.project_id = Some(project.id.clone());
        row.role_id = Some(role_id.clone());
        row.cwd = Some("/tmp".into());
        row.started_at = Some(now);
        row.agent_session_key = Some(source_key.clone());
        row.agent_model = Some("opus".into());
        row.agent_effort = Some("max".into());
        row.title = Some("Source".into());
        row.last_cols = Some(90);
        row.last_rows = Some(30);
        crate::repo::session::insert(&conn, &row).unwrap();
        project.id
    };
    let source_before = crate::repo::session::get_row(&pool.get().unwrap(), &source_id)
        .unwrap()
        .unwrap();

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let events = capture();
    let spawned = mgr
        .spawn_fork(
            &source_id,
            Some("Source (fork)".into()),
            Some(120),
            Some(40),
            Path::new("/tmp"),
            Arc::clone(&pool),
            Arc::clone(&events) as Arc<dyn SessionEvents>,
        )
        .unwrap();
    assert_eq!(
        *events.fork_started.lock().unwrap(),
        [SessionForkStartedEvent {
            source_session_id: source_id.clone(),
            session_id: spawned.id.clone(),
        }]
    );

    let fork = crate::repo::session::get_row(&pool.get().unwrap(), &spawned.id)
        .unwrap()
        .unwrap();
    assert_ne!(fork.id, source_id);
    assert_eq!(fork.project_id.as_deref(), Some(project_id.as_str()));
    assert_eq!(fork.role_id.as_deref(), Some(role_id.as_str()));
    assert_eq!(fork.cwd.as_deref(), Some("/tmp"));
    assert_eq!(fork.agent_runtime, source_before.agent_runtime);
    assert_eq!(fork.agent_command, source_before.agent_command);
    assert_eq!(fork.agent_model, source_before.agent_model);
    assert_eq!(fork.agent_effort, source_before.agent_effort);
    assert_eq!(fork.title.as_deref(), Some("Source (fork)"));
    assert_eq!(fork.last_cols, Some(120));
    assert_eq!(fork.last_rows, Some(40));
    assert!(fork.agent_session_key.is_some());
    assert_ne!(fork.agent_session_key, source_before.agent_session_key);

    let assigned_key = fork.agent_session_key.as_deref().unwrap();
    let spec = fake.last_spawn_spec().expect("fork should spawn the TUI");
    assert!(spec.shim_dir.is_none());
    assert_eq!(spec.bundled_bin_dir.as_deref(), Some(Path::new("/tmp/bin")));
    assert_chat_has_no_permission_flags(&spec.args);
    assert_eq!(spec.command, command);
    assert_eq!(
        &spec.args[..11],
        [
            "--role-flag",
            "--resume",
            source_key.as_str(),
            "--fork-session",
            "--session-id",
            assigned_key,
            "--model",
            "opus",
            "--effort",
            "max",
            "--settings",
        ]
    );
    let settings: serde_json::Value = serde_json::from_str(&spec.args[11]).unwrap();
    assert_eq!(settings["tui"], "fullscreen");
    assert_eq!(
        settings["hooks"]["SessionStart"][0]["hooks"][0]["type"],
        "command"
    );
    assert_eq!(spec.cwd.as_deref(), Some(Path::new("/tmp")));
    assert_eq!(
        spec.env.get("FORK_TEST_ENV").map(String::as_str),
        Some("same-env")
    );
    assert!(!spec.args.contains(&"-p".to_string()));
    assert!(!spec.args.iter().any(|arg| arg.contains("forked from")));
    assert!(!spec.args.iter().any(|arg| arg.contains("Forker persona")));
    assert_eq!(spec.initial_size, Some((120, 40)));
    assert_eq!(fake.spawn_count(), 1);

    let source_after = crate::repo::session::get_row(&pool.get().unwrap(), &source_id)
        .unwrap()
        .unwrap();
    assert_eq!(source_after, source_before);
    mgr.kill(&spawned.id).unwrap();
}

#[test]
#[cfg(unix)]
fn codex_direct_chat_fork_captures_headless_key_then_resumes_without_watcher() {
    let pool = pool_with_schema();
    let role_id = ulid::Ulid::new().to_string();
    let source_id = ulid::Ulid::new().to_string();
    let source_key = uuid::Uuid::new_v4().to_string();
    let fork_key = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    let (_materializer, command, capture_path, codex_home) =
        codex_fork_materializer(&source_key, &fork_key, true);
    {
        let conn = pool.get().unwrap();
        let mut persisted_role = role(
            &command,
            &[
                "--ask-for-approval",
                "never",
                "--sandbox",
                "workspace-write",
            ],
        );
        persisted_role.id = role_id.clone();
        persisted_role.handle = "codex-forker".into();
        persisted_role.runtime = "codex".into();
        persisted_role.env.insert(
            "CODEX_HOME".into(),
            codex_home.to_string_lossy().into_owned(),
        );
        persisted_role
            .env
            .insert("FORK_TEST_ENV".into(), "same-env".into());
        persisted_role.working_dir = Some("/tmp".into());
        persisted_role.system_prompt = Some("Codex persona".into());
        insert_role_row(&conn, &persisted_role);
        let mut row = crate::repo::session::SessionRowDb::new_running(source_id.clone());
        row.status = crate::model::SessionStatus::Stopped;
        row.role_id = Some(role_id);
        row.cwd = Some("/tmp".into());
        row.started_at = Some(now);
        row.agent_session_key = Some(source_key.clone());
        row.agent_runtime = Some("codex".into());
        row.agent_command = Some(command.clone());
        row.agent_model = Some("gpt-5.6-sol".into());
        row.agent_effort = Some("xhigh".into());
        row.title = Some("Codex".into());
        crate::repo::session::insert(&conn, &row).unwrap();
    }

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let started = Instant::now();
    let spawned = mgr
        .spawn_fork(
            &source_id,
            Some("Codex (fork)".into()),
            None,
            None,
            Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap();
    // The materializer lives for a minute after writing the rollout, so
    // returning well inside that shows the fork took the rollout file without
    // waiting on the process, however slowly the fixture started.
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "Codex fork waited for the materializer reply instead of the rollout file"
    );

    let materialize = std::fs::read_to_string(capture_path).unwrap();
    let expected_cwd = std::fs::canonicalize("/tmp").unwrap();
    assert!(materialize.contains(&format!("cwd={}", expected_cwd.display())));
    assert!(materialize.contains(&format!("arg=exec\narg=fork\narg={source_key}\narg=--json")));
    assert!(materialize.contains("arg=--json\narg=--skip-git-repo-check"));
    assert!(!materialize.contains("arg=-c"));
    assert!(!materialize.contains("gpt-5.6-luna"));
    assert!(materialize.contains("env=same-env"));
    assert!(materialize.contains("arg=This chat was forked from 'Codex'."));
    assert!(!materialize.contains("Codex persona"));
    assert!(!materialize.contains("arg=--ask-for-approval"));
    assert!(!materialize.contains("arg=--sandbox"));

    let spec = fake.last_spawn_spec().expect("resume should spawn the TUI");
    assert_eq!(&spec.args[..2], &["resume", fork_key.as_str()]);
    assert_chat_has_no_permission_flags(&spec.args);
    assert!(!spec.args.contains(&"fork".to_string()));
    assert!(mgr.codex_capture_context(&spawned.id).is_none());
    let fork = crate::repo::session::get_row(&pool.get().unwrap(), &spawned.id)
        .unwrap()
        .unwrap();
    assert_eq!(fork.agent_runtime.as_deref(), Some("codex"));
    assert_eq!(fork.agent_command.as_deref(), Some(command.as_str()));
    assert_eq!(fork.agent_model.as_deref(), Some("gpt-5.6-sol"));
    assert_eq!(fork.agent_effort.as_deref(), Some("xhigh"));
    assert_eq!(fork.agent_session_key.as_deref(), Some(fork_key.as_str()));
    mgr.kill(&spawned.id).unwrap();
}

#[test]
#[cfg(unix)]
fn fork_materialization_missing_thread_event_removes_row_and_tab() {
    let pool = pool_with_schema();
    let source_id = ulid::Ulid::new().to_string();
    let source_key = uuid::Uuid::new_v4().to_string();
    let (_materializer, command, _capture_path) =
        fork_materializer(r#"{"type":"turn.started"}"#, 0);
    let source_before = {
        let conn = pool.get().unwrap();
        let mut row = crate::repo::session::SessionRowDb::new_running(source_id.clone());
        row.cwd = Some("/tmp".into());
        row.started_at = Some(Utc::now());
        row.agent_session_key = Some(source_key);
        row.agent_runtime = Some("codex".into());
        row.agent_command = Some(command);
        crate::repo::session::insert(&conn, &row).unwrap();
        row
    };
    let events = Arc::new(RepairingCapture {
        pool: Arc::clone(&pool),
        updated: Mutex::new(Vec::new()),
    });
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));

    let error = mgr
        .spawn_fork(
            &source_id,
            Some("Bad fork".into()),
            None,
            None,
            Path::new("/tmp"),
            Arc::clone(&pool),
            events.clone(),
        )
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("first event is not thread.started"));
    assert_eq!(fake.spawn_count(), 0);

    let updates = events.updated.lock().unwrap();
    assert_eq!(updates.len(), 2);
    let fork_id = &updates[0].session_id;
    assert_eq!(updates[1].session_id, *fork_id);
    assert!(crate::repo::session::get_row(&pool.get().unwrap(), fork_id)
        .unwrap()
        .is_none());
    let nodes = crate::repo::node::list(&pool.get().unwrap()).unwrap();
    assert!(!nodes.iter().any(|node| node
        .layout
        .as_deref()
        .is_some_and(|layout| layout.contains(fork_id))));
    let source_after = crate::repo::session::get_row(&pool.get().unwrap(), &source_id)
        .unwrap()
        .unwrap();
    assert_eq!(source_after, source_before);
}

#[test]
#[cfg(unix)]
fn headless_fork_rejects_nonzero_exit_and_kills_timed_out_process_group() {
    use std::os::unix::fs::PermissionsExt;

    let source_key = uuid::Uuid::new_v4().to_string();
    let fork_key = uuid::Uuid::new_v4().to_string();
    let event = format!(r#"{{"type":"thread.started","thread_id":"{fork_key}"}}"#);
    let plan = router::runtime::fork_plan(Some(Runtime::Codex), &source_key, "fork note").unwrap();
    let (_materializer, command, _capture_path) = fork_materializer(&event, 7);
    let codex_home = tempfile::tempdir().unwrap();
    let mut env = std::collections::BTreeMap::new();
    env.insert(
        "CODEX_HOME".to_string(),
        codex_home.path().to_string_lossy().into_owned(),
    );
    let spec = SpawnSpec {
        codex_pending_turn: None,
        session_id: ulid::Ulid::new().to_string(),
        cwd: Some(PathBuf::from("/tmp")),
        command,
        args: Vec::new(),
        env,
        mission: false,
        shim_dir: None,
        bundled_bin_dir: None,
        shell_path: None,
        initial_size: None,
    };
    let error = super::spawn::run_headless_fork(&spec, &plan, Duration::from_secs(10)).unwrap_err();
    assert!(error.to_string().contains("exited with"), "{error}");

    let fork_key = uuid::Uuid::new_v4().to_string();
    let (_materializer, command, _capture_path, codex_home) =
        codex_fork_materializer(&source_key, &fork_key, false);
    let mut env = std::collections::BTreeMap::new();
    env.insert(
        "CODEX_HOME".to_string(),
        codex_home.to_string_lossy().into_owned(),
    );
    let missing_rollout_spec = SpawnSpec {
        codex_pending_turn: None,
        session_id: ulid::Ulid::new().to_string(),
        cwd: Some(PathBuf::from("/tmp")),
        command,
        args: Vec::new(),
        env,
        mission: false,
        shim_dir: None,
        bundled_bin_dir: None,
        shell_path: None,
        initial_size: None,
    };
    let codex_plan =
        router::runtime::fork_plan(Some(Runtime::Codex), &source_key, "Source").unwrap();
    // The rollout deadline, five seconds after thread.started, must be what
    // ends this wait. A short overall timeout races the materializer's start
    // on a loaded machine and reports a timeout before thread.started arrives.
    let timeout = Duration::from_secs(30);
    let started = Instant::now();
    let error =
        super::spawn::run_headless_fork(&missing_rollout_spec, &codex_plan, timeout).unwrap_err();
    assert!(error.to_string().contains("rollout for thread"), "{error}");
    assert!(started.elapsed() < timeout);

    let dir = tempfile::tempdir().unwrap();
    let command = dir.path().join("slow-materializer");
    std::fs::write(&command, "#!/bin/sh\nsleep 10\n").unwrap();
    let mut permissions = std::fs::metadata(&command).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&command, permissions).unwrap();
    let mut timed_spec = spec;
    timed_spec.command = command.to_string_lossy().into_owned();
    let started = Instant::now();
    let error = super::spawn::run_headless_fork(&timed_spec, &plan, Duration::from_millis(250))
        .unwrap_err();
    assert!(error.to_string().contains("timed out"));
    assert!(started.elapsed() < ci_scaled_budget(Duration::from_secs(2)));
}

#[test]
fn fork_refuses_ineligible_source_rows() {
    let pool = pool_with_schema();
    let now = Utc::now();
    let key = uuid::Uuid::new_v4().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
             VALUES ('fork-crew', 'Fork Crew', ?1, ?1)",
            params![now.to_rfc3339()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO missions (id, crew_id, title, status, started_at)
             VALUES ('fork-mission', 'fork-crew', 'Fork mission', 'running', ?1)",
            params![now.to_rfc3339()],
        )
        .unwrap();
        for (id, runtime, agent_key) in [
            ("fork-mission-source", "claude-code", Some(key.as_str())),
            ("fork-archived-source", "claude-code", Some(key.as_str())),
            ("fork-unkeyed-source", "claude-code", None),
            ("fork-trae-source", "trae", Some(key.as_str())),
            ("fork-shell-source", "shell", Some(key.as_str())),
        ] {
            let mut row = crate::repo::session::SessionRowDb::new_running(id.into());
            row.cwd = Some("/tmp".into());
            row.started_at = Some(now);
            row.agent_runtime = Some(runtime.into());
            row.agent_command =
                (!matches!(runtime, "trae" | "shell")).then(|| format!("{runtime}-custom"));
            row.agent_session_key = agent_key.map(str::to_owned);
            if id == "fork-mission-source" {
                row.mission_id = Some("fork-mission".into());
            }
            if id == "fork-archived-source" {
                row.archived_at = Some(now);
            }
            crate::repo::session::insert(&conn, &row).unwrap();
        }
    }

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    for (source, message) in [
        ("missing-fork-source", "session not found"),
        ("fork-mission-source", "only direct chats"),
        ("fork-archived-source", "is archived"),
        ("fork-unkeyed-source", "no captured agent session key"),
        ("fork-trae-source", "does not support native fork"),
        ("fork-shell-source", "does not support native fork"),
    ] {
        let error = mgr
            .spawn_fork(
                source,
                None,
                None,
                None,
                fixture_tmp_dir(),
                Arc::clone(&pool),
                capture(),
            )
            .unwrap_err();
        assert!(
            error.to_string().contains(message),
            "unexpected error for {source}: {error}"
        );
    }
    assert_eq!(fake.spawn_count(), 0);
}

#[cfg(windows)]
#[test]
fn headless_fork_timeout_kills_batch_descendant_and_closes_pipes() {
    if std::env::var_os("RUNNER_FORK_TEST_PID").is_some() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let descendant_pid = dir.path().join("descendant.pid");
    let command = dir.path().join("materializer.cmd");
    std::fs::write(
        &command,
        "@echo off\r\n\"%RUNNER_FORK_TEST_EXE%\" --exact session::manager::tests::headless_fork_descendant --nocapture >nul\r\n",
    ).unwrap();
    let spec = SpawnSpec {
        codex_pending_turn: None,
        session_id: ulid::Ulid::new().to_string(),
        command: command.to_string_lossy().into_owned(),
        args: Vec::new(),
        env: BTreeMap::from([
            (
                "CODEX_HOME".into(),
                dir.path().to_string_lossy().into_owned(),
            ),
            (
                "RUNNER_FORK_TEST_PID".into(),
                descendant_pid.to_string_lossy().into_owned(),
            ),
            (
                "RUNNER_FORK_TEST_EXE".into(),
                std::env::current_exe()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            ),
        ]),
        cwd: Some(dir.path().to_path_buf()),
        mission: false,
        shim_dir: None,
        bundled_bin_dir: None,
        shell_path: None,
        initial_size: None,
    };
    let plan = router::runtime::ForkPlan::Headless {
        args: Vec::new(),
        source_key: uuid::Uuid::new_v4().to_string(),
    };
    let started = Instant::now();
    let error = super::spawn::run_headless_fork(&spec, &plan, Duration::from_secs(5)).unwrap_err();
    assert!(error.to_string().contains("timed out"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(15));
    let pid: i32 = std::fs::read_to_string(descendant_pid)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        crate::session::process::wait_for_process_exit_until(
            pid,
            Instant::now() + Duration::from_secs(2),
        ),
        "headless descendant {pid} survived the kill grace"
    );
}
