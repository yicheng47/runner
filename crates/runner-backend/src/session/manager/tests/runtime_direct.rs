use super::*;

#[test]
fn runtime_direct_role_applies_model_and_effort() {
    let configured =
        runtime_direct_role("codex", None, Some(" gpt-5.6-sol "), Some(" max ")).unwrap();
    assert_eq!(configured.model.as_deref(), Some("gpt-5.6-sol"));
    assert_eq!(configured.effort.as_deref(), Some("max"));

    let defaults = runtime_direct_role("codex", None, Some(" "), Some("")).unwrap();
    assert_eq!(defaults.model, None);
    assert_eq!(defaults.effort, None);
}

#[cfg(unix)]
#[test]
fn shell_runtime_spawns_and_resumes_as_plain_login_shell() {
    let pool = pool_with_schema();
    let project = crate::repo::project::create(&pool.get().unwrap(), "Project", "/tmp").unwrap();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(
        Some("/usr/local/bin:/usr/bin:/bin".into()),
        Arc::clone(&fake),
    );
    let shell = runtime_direct_role("shell", Some("/bin/zsh"), None, None).unwrap();

    assert_eq!(shell.args, ["-l"]);
    assert!(shell.system_prompt.is_none());
    assert!(shell.env.is_empty());
    assert!(shell.model.is_none());
    assert!(shell.effort.is_none());

    let spawned = mgr
        .spawn_runtime_direct(
            &shell,
            Some(&project.id),
            Some("/tmp"),
            Some(132),
            Some(41),
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap();
    let spec = fake.last_spawn_spec().expect("shell should spawn");
    assert_eq!(spec.command, "/bin/zsh");
    assert_eq!(spec.args, ["-l"]);
    assert_eq!(spec.cwd.as_deref(), Some(std::path::Path::new("/tmp")));
    assert_eq!(spec.initial_size, Some((132, 41)));
    assert_eq!(
        spec.shell_path.as_deref(),
        Some("/usr/local/bin:/usr/bin:/bin")
    );
    assert!(spec.shim_dir.is_none());
    assert_eq!(
        spec.bundled_bin_dir.as_deref(),
        Some(std::path::Path::new("/tmp/bin"))
    );
    assert!(spec.env.keys().all(|key| !key.starts_with("RUNNER_")));

    let stored = crate::repo::session::get_row(&pool.get().unwrap(), &spawned.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.project_id.as_deref(), Some(project.id.as_str()));
    assert_eq!(stored.cwd.as_deref(), Some("/tmp"));
    assert_eq!(stored.agent_runtime.as_deref(), Some("shell"));
    assert_eq!(stored.agent_command.as_deref(), Some("/bin/zsh"));
    assert!(stored.agent_session_key.is_none());

    mgr.kill(&spawned.id).unwrap();
    mgr.resume_on_launch(
        &spawned.id,
        None,
        None,
        std::path::Path::new("/tmp"),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();
    let resumed = fake.last_spawn_spec().expect("shell should resume");
    assert_eq!(resumed.command, "/bin/zsh");
    assert_eq!(resumed.args, ["-l"]);
    assert_eq!(resumed.cwd.as_deref(), Some(std::path::Path::new("/tmp")));
    assert!(resumed.env.keys().all(|key| !key.starts_with("RUNNER_")));

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn shell_resume_uses_nearest_existing_cwd_and_feeds_notice_first() {
    let pool = pool_with_schema();
    let root = tempfile::tempdir().unwrap();
    let project_cwd = root.path().join("project");
    let existing_ancestor = root.path().join("worktrees").join("feature");
    std::fs::create_dir_all(&project_cwd).unwrap();
    std::fs::create_dir_all(&existing_ancestor).unwrap();
    let missing_cwd = existing_ancestor.join("deleted").join("nested");
    let project = crate::repo::project::create(
        &pool.get().unwrap(),
        "Project",
        &project_cwd.to_string_lossy(),
    )
    .unwrap();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO sessions
                (id, project_id, status, started_at, cwd,
                 agent_runtime, agent_command, resume_on_launch)
             VALUES ('shell-missing-cwd', ?1, 'stopped', ?2, ?3,
                     'shell', '/bin/zsh', 1)",
            params![
                project.id,
                Utc::now().to_rfc3339(),
                missing_cwd.to_string_lossy()
            ],
        )
        .unwrap();
    }

    let fake = fake_runtime();
    let fake_for_hook = Arc::clone(&fake);
    *fake.spawn_hook.lock().unwrap() = Some(Box::new(move || {
        fake_for_hook.push_output(0, b"shell startup\r\n");
    }));
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let events = capture();
    mgr.resume_on_launch(
        "shell-missing-cwd",
        None,
        None,
        fixture_tmp_dir(),
        Arc::clone(&pool),
        events.clone(),
    )
    .unwrap();

    let spawned = fake.last_spawn_spec().expect("shell should relaunch");
    assert_eq!(spawned.cwd.as_deref(), Some(existing_ancestor.as_path()));
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if events.output.lock().unwrap().len() >= 2 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "shell startup output was not forwarded"
        );
        std::thread::yield_now();
    }
    let output = events.output.lock().unwrap();
    assert_eq!(output[0].seq, 1);
    let home = runner_core::app_paths::home_dir();
    let displayed_paths = [&missing_cwd, &existing_ancestor].map(|path| {
        match home.as_ref().and_then(|home| path.strip_prefix(home).ok()) {
            Some(relative) => format!("~/{}", relative.display()),
            None => path.to_string_lossy().into_owned(),
        }
    });
    assert_eq!(
        output[0].bytes,
        format!(
            "\x1b[33mrunner: {} no longer exists\r\n        opened {} instead\x1b[0m\r\n",
            displayed_paths[0], displayed_paths[1],
        )
        .into_bytes()
    );
    assert_eq!(output[1].bytes, b"shell startup\r\n");
    drop(output);

    mgr.kill("shell-missing-cwd").unwrap();
}

#[test]
fn runtime_direct_spawn_defaults_to_home_and_preserves_explicit_directories() {
    let home = runner_core::app_paths::home_dir().expect("home directory");
    let selected = tempfile::tempdir().unwrap();
    for (cwd, role_cwd, expected) in [
        (None, None, home.as_path()),
        (Some(""), None, home.as_path()),
        (Some(" \t"), Some(""), home.as_path()),
        (None, selected.path().to_str(), selected.path()),
        (selected.path().to_str(), home.to_str(), selected.path()),
    ] {
        let pool = pool_with_schema();
        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        let mut configured = role("/bin/sh", &[]);
        configured.working_dir = role_cwd.map(str::to_owned);
        let spawned = mgr
            .spawn_runtime_direct(
                &configured,
                None,
                cwd,
                None,
                None,
                fixture_tmp_dir(),
                Arc::clone(&pool),
                capture(),
            )
            .unwrap();
        let stored_cwd: String = pool
            .get()
            .unwrap()
            .query_row(
                "SELECT cwd FROM sessions WHERE id = ?1",
                params![spawned.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(Path::new(&stored_cwd), expected);
        assert_eq!(
            fake.last_spawn_spec().unwrap().cwd.as_deref(),
            Some(expected)
        );
        mgr.kill(&spawned.id).unwrap();
    }
}

#[test]
fn runtime_direct_spawn_persists_model_and_effort() {
    let pool = pool_with_schema();
    let configured =
        runtime_direct_role("codex", Some("/bin/sh"), Some("gpt-5.6-sol"), Some("max")).unwrap();
    let mgr = mgr_with_fake(None, fake_runtime());
    let spawned = mgr
        .spawn_runtime_direct(
            &configured,
            None,
            Some(fixture_tmp_dir().to_str().unwrap()),
            None,
            None,
            fixture_tmp_dir(),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap();

    let stored: (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT agent_runtime, agent_command, agent_model, agent_effort
               FROM sessions WHERE id = ?1",
            params![spawned.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(stored.0.as_deref(), Some("codex"));
    assert_eq!(stored.1.as_deref(), Some("/bin/sh"));
    assert_eq!(stored.2.as_deref(), Some("gpt-5.6-sol"));
    assert_eq!(stored.3.as_deref(), Some("max"));

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn pinned_direct_spawn_records_override_model_and_effort() {
    let pool = pool_with_schema();
    let role_id = ulid::Ulid::new().to_string();
    crate::test_support::insert_test_role(
        &pool.get().unwrap(),
        &role_id,
        "pin-me",
        "codex",
        "/bin/sh",
    );
    let mut r = role("/bin/sh", &[]);
    r.id = role_id;
    r.runtime = "codex".into();
    r.model = Some("role-model".into());
    r.effort = Some("role-effort".into());
    let mgr = mgr_with_fake(None, fake_runtime());
    let spawned = mgr
        .spawn_direct(
            &r,
            Some("codex"),
            Some("gpt-5.6-sol"),
            Some("ultra"),
            None,
            Some(fixture_tmp_dir().to_str().unwrap()),
            None,
            None,
            fixture_tmp_dir(),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();

    let stored: (Option<String>, Option<String>, Option<String>) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT agent_runtime, agent_model, agent_effort
               FROM sessions WHERE id = ?1",
            params![spawned.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(stored.0.as_deref(), Some("codex"));
    assert_eq!(stored.1.as_deref(), Some("gpt-5.6-sol"));
    assert_eq!(stored.2.as_deref(), Some("ultra"));

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn unpinned_direct_spawn_persists_options_without_pinning_runtime() {
    let pool = pool_with_schema();
    let role_id = ulid::Ulid::new().to_string();
    crate::test_support::insert_test_role(
        &pool.get().unwrap(),
        &role_id,
        "options-only",
        "codex",
        "/bin/sh",
    );
    let mut r = role("/bin/sh", &[]);
    r.id = role_id;
    r.runtime = "codex".into();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .spawn_direct(
            &r,
            None,
            Some("gpt-5.6-sol"),
            Some("ultra"),
            None,
            Some(fixture_tmp_dir().to_str().unwrap()),
            None,
            None,
            fixture_tmp_dir(),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();

    let stored: (Option<String>, Option<String>, Option<String>) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT agent_runtime, agent_model, agent_effort
               FROM sessions WHERE id = ?1",
            params![spawned.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(stored.0, None, "options-only direct chats must not pin");
    assert_eq!(stored.1.as_deref(), Some("gpt-5.6-sol"));
    assert_eq!(stored.2.as_deref(), Some("ultra"));

    mgr.kill(&spawned.id).unwrap();
    mgr.resume(
        &spawned.id,
        None,
        None,
        fixture_tmp_dir(),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();
    let resumed = fake.last_spawn_spec().expect("resume should spawn");
    assert!(resumed
        .args
        .windows(2)
        .any(|w| w[0] == "--model" && w[1] == "gpt-5.6-sol"));
    assert!(resumed
        .args
        .windows(2)
        .any(|w| w[0] == "-c" && w[1] == "model_reasoning_effort=ultra"));
    mgr.kill(&spawned.id).unwrap();
}

#[test]
#[cfg(unix)]
fn catalog_default_role_uses_detected_command_while_custom_command_stays_untouched() {
    use std::os::unix::fs::PermissionsExt;

    let pool = pool_with_schema();
    let bin = tempfile::tempdir().unwrap();
    let detected = bin.path().join("codex");
    std::fs::write(&detected, "#!/bin/sh\n").unwrap();
    let mut permissions = std::fs::metadata(&detected).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&detected, permissions).unwrap();

    let default_id = ulid::Ulid::new().to_string();
    let custom_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        for (id, handle, command) in [
            (&default_id, "default-runtime", "codex"),
            (&custom_id, "custom-runtime", "codex-wrapper"),
        ] {
            crate::test_support::insert_test_role(&conn, id, handle, "codex", command);
        }
    }

    let fake = fake_runtime();
    let mgr = mgr_with_fake(Some(bin.path().display().to_string()), Arc::clone(&fake));
    let mut default_role = role("codex", &[]);
    default_role.id = default_id;
    default_role.handle = "default-runtime".into();
    default_role.runtime = "codex".into();
    let default_session = mgr
        .spawn_direct(
            &default_role,
            None,
            None,
            None,
            None,
            Some("/tmp"),
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();
    assert_eq!(
        fake.last_spawn_spec().unwrap().command,
        detected.display().to_string()
    );

    let mut custom_role = role("codex-wrapper", &[]);
    custom_role.id = custom_id;
    custom_role.handle = "custom-runtime".into();
    custom_role.runtime = "codex".into();
    let custom_session = mgr
        .spawn_direct(
            &custom_role,
            None,
            None,
            None,
            None,
            Some("/tmp"),
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();
    assert_eq!(fake.last_spawn_spec().unwrap().command, "codex-wrapper");

    mgr.shell_env.write().unwrap().path = Some("/swapped/bin".into());
    let swapped_session = mgr
        .spawn_direct(
            &custom_role,
            None,
            None,
            None,
            None,
            Some("/tmp"),
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();
    assert_eq!(
        fake.last_spawn_spec().unwrap().shell_path.as_deref(),
        Some("/swapped/bin")
    );

    mgr.kill(&default_session.id).unwrap();
    mgr.kill(&custom_session.id).unwrap();
    mgr.kill(&swapped_session.id).unwrap();
}

#[test]
#[cfg(unix)]
fn runtime_only_resume_keeps_live_recorded_path_and_reresolves_dead_path() {
    use std::os::unix::fs::PermissionsExt;

    let pool = pool_with_schema();
    let recorded_dir = tempfile::tempdir().unwrap();
    let detected_dir = tempfile::tempdir().unwrap();
    let make_executable = |path: &Path| {
        std::fs::write(path, "#!/bin/sh\n").unwrap();
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).unwrap();
    };
    let recorded = recorded_dir.path().join("codex-recorded");
    let detected = detected_dir.path().join("codex");
    make_executable(&recorded);
    make_executable(&detected);

    let now = Utc::now().to_rfc3339();
    {
        let conn = pool.get().unwrap();
        for (id, command) in [
            ("runtime-live-path", recorded.display().to_string()),
            ("runtime-dead-path", "/definitely/missing/codex".to_string()),
        ] {
            conn.execute(
                "INSERT INTO sessions
                    (id, status, started_at, agent_runtime, agent_command,
                     agent_model, agent_effort)
                 VALUES (?1, 'stopped', ?2, 'codex', ?3,
                         'gpt-5.6-sol', 'max')",
                params![id, now, command],
            )
            .unwrap();
        }
    }

    let fake = fake_runtime();
    let mgr = mgr_with_fake(
        Some(detected_dir.path().display().to_string()),
        Arc::clone(&fake),
    );
    mgr.resume(
        "runtime-live-path",
        None,
        None,
        std::path::Path::new("/tmp"),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();
    assert_eq!(
        fake.last_spawn_spec().unwrap().command,
        recorded.display().to_string()
    );
    assert!(fake
        .last_spawn_spec()
        .unwrap()
        .args
        .windows(2)
        .any(|args| args[0] == "--model" && args[1] == "gpt-5.6-sol"));
    assert!(fake
        .last_spawn_spec()
        .unwrap()
        .args
        .windows(2)
        .any(|args| args[0] == "-c" && args[1] == "model_reasoning_effort=max"));

    mgr.resume(
        "runtime-dead-path",
        None,
        None,
        std::path::Path::new("/tmp"),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();
    assert_eq!(
        fake.last_spawn_spec().unwrap().command,
        detected.display().to_string()
    );

    mgr.kill("runtime-live-path").unwrap();
    mgr.kill("runtime-dead-path").unwrap();
}
