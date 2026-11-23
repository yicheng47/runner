use super::*;

#[test]
#[cfg(unix)]
fn grok_new_chats_spawn_with_native_permissions_and_assigned_identity() {
    let pool = pool_with_schema();
    let data = tempfile::tempdir().unwrap();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());
    let mut role =
        runtime_direct_role("grok", Some("custom-grok-wrapper"), Some("grok-4.7"), None).unwrap();
    role.env.insert(
        "GROK_HOME".into(),
        data.path().join("grok").display().to_string(),
    );
    role.args = vec![
        "--always-approve".into(),
        "--sandbox=safe".into(),
        "--trust".into(),
    ];
    insert_role_row(&pool.get().unwrap(), &role);
    for role_backed in [false, true] {
        let spawned = if role_backed {
            mgr.spawn_direct(
                &role,
                DirectLaunch {
                    first_turn: Some("persona".into()),
                    ..Default::default()
                },
                data.path(),
                pool.clone(),
                capture(),
            )
        } else {
            mgr.spawn_direct(
                &role,
                DirectLaunch {
                    runtime_direct: true,
                    ..Default::default()
                },
                data.path(),
                pool.clone(),
                capture(),
            )
        }
        .unwrap();
        let spec = fake.last_spawn_spec().unwrap();
        assert_eq!(spec.command, "custom-grok-wrapper");
        assert!(!spec.mission);
        assert!(!spec.env.contains_key("RUNNER_MISSION_ID"));
        let mut expected = vec!["--trust", "-m", "grok-4.7"];
        if role_backed {
            expected.extend(["--", "persona"]);
        }
        assert_eq!(&spec.args[2..], expected);
        assert_eq!(spec.args[0], "-s");
        assert!(uuid::Uuid::parse_str(&spec.args[1]).is_ok());
        let row = crate::repo::session::get_row(&pool.get().unwrap(), &spawned.id)
            .unwrap()
            .unwrap();
        assert!(uuid::Uuid::parse_str(row.agent_session_key.as_deref().unwrap()).is_ok());
        mgr.kill(&spawned.id).unwrap();
    }
    assert_eq!(fake.spawns.lock().unwrap().len(), 2);
}

#[test]
#[cfg(unix)]
fn concurrent_grok_new_chats_have_distinct_rows_without_shared_resume_identity() {
    let pool = pool_with_schema();
    let data = tempfile::tempdir().unwrap();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());
    let role = runtime_direct_role("grok", Some("custom-grok-wrapper"), None, None).unwrap();
    let ids = std::thread::scope(|scope| {
        let spawn = || {
            mgr.spawn_direct(
                &role,
                DirectLaunch {
                    cwd: Some(data.path().to_str().unwrap().into()),
                    runtime_direct: true,
                    ..Default::default()
                },
                data.path(),
                pool.clone(),
                capture(),
            )
        };
        let a = scope.spawn(spawn);
        let b = scope.spawn(spawn);
        vec![a.join().unwrap().unwrap().id, b.join().unwrap().unwrap().id]
    });
    assert_ne!(ids[0], ids[1]);
    assert_eq!(fake.spawns.lock().unwrap().len(), 2);
    let first_key = crate::repo::session::get_row(&pool.get().unwrap(), &ids[0])
        .unwrap()
        .unwrap()
        .agent_session_key;
    let second_key = crate::repo::session::get_row(&pool.get().unwrap(), &ids[1])
        .unwrap()
        .unwrap()
        .agent_session_key;
    assert_ne!(first_key, second_key);
    for id in ids {
        let row = crate::repo::session::get_row(&pool.get().unwrap(), &id)
            .unwrap()
            .unwrap();
        assert!(uuid::Uuid::parse_str(row.agent_session_key.as_deref().unwrap()).is_ok());
        mgr.kill(&id).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn grok_automatic_resume_rejects_missing_history_and_manual_resume_starts_fresh() {
    let pool = pool_with_schema();
    let data = tempfile::tempdir().unwrap();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());
    db::set_runtime_override(&pool, "grok", Some("custom-grok-wrapper")).unwrap();
    let mut row = crate::repo::session::SessionRowDb::new_running("grok-saved".into());
    row.agent_runtime = Some("grok".into());
    row.agent_command = Some("custom-grok-wrapper".into());
    row.agent_session_key = Some(uuid::Uuid::new_v4().to_string());
    row.status = crate::model::SessionStatus::Stopped;
    crate::test_support::insert_session_row(&pool.get().unwrap(), &row).unwrap();
    mgr.shell_env.write().unwrap().vars.insert(
        "GROK_HOME".into(),
        data.path().join("grok").display().to_string(),
    );
    let error = mgr
        .resume_on_launch(&row.id, None, None, data.path(), pool.clone(), capture())
        .err()
        .unwrap();
    assert!(error.to_string().contains("conversation is unavailable"));
    let saved = crate::repo::session::get_row(&pool.get().unwrap(), &row.id)
        .unwrap()
        .unwrap();
    assert_eq!(saved.agent_runtime, row.agent_runtime);
    assert_eq!(saved.agent_command, row.agent_command);
    assert_eq!(saved.agent_session_key, row.agent_session_key);
    assert_eq!(saved.status, row.status);
    assert!(fake.spawns.lock().unwrap().is_empty());
    let spawned = mgr
        .resume(&row.id, None, None, data.path(), pool.clone(), capture())
        .unwrap();
    let fresh = crate::repo::session::get_row(&pool.get().unwrap(), &row.id)
        .unwrap()
        .unwrap();
    assert_ne!(fresh.agent_session_key, row.agent_session_key);
    assert!(uuid::Uuid::parse_str(fresh.agent_session_key.as_deref().unwrap()).is_ok());
    assert_eq!(fake.last_spawn_spec().unwrap().args[0], "-s");
    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn grok_resume_and_mission_use_role_home_before_captured_shell_home() {
    let data = tempfile::tempdir().unwrap();
    let pool = pool_with_schema();
    let fake = fake_runtime();
    let manager = mgr_with_fake(None, fake.clone());
    let config = data.path().join("grok");
    std::fs::create_dir(&config).unwrap();
    manager.shell_env.write().unwrap().vars.insert(
        "GROK_HOME".into(),
        data.path().join("missing").display().to_string(),
    );
    let mut configured = role("/bin/sh", &[]);
    configured.runtime = "grok".into();
    configured
        .env
        .insert("GROK_HOME".into(), config.display().to_string());
    let mut mission = mission();
    mission.cwd = Some(data.path().display().to_string());
    let slot_id = insert_crew_role(&pool, &mission.id, &configured.id);
    let mut slot = slot_for(&configured);
    slot.id = slot_id;
    crate::repo::role::update(
        &pool.get().unwrap(),
        &crate::repo::role::RoleRow::from(&configured),
    )
    .unwrap();
    let spawned = manager
        .spawn(
            &mission,
            &configured,
            &slot,
            data.path(),
            data.path().join("events.ndjson"),
            pool.clone(),
            capture(),
            Some("goal".into()),
        )
        .unwrap();
    let row = crate::repo::session::get_row(&pool.get().unwrap(), &spawned.id)
        .unwrap()
        .unwrap();
    let key = row.agent_session_key.unwrap();
    let spec = fake.last_spawn_spec().unwrap();
    assert!(has_arg_pair(
        &spec.args,
        "--permission-mode",
        "bypassPermissions"
    ));
    assert!(spec.args.contains(&"--trust".into()));
    let path = config
        .join("sessions/opaque")
        .join(&key)
        .join("summary.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        serde_json::json!({"info":{"id":key,"cwd":data.path().canonicalize().unwrap()}})
            .to_string(),
    )
    .unwrap();
    fake.close_spawn(0);
    wait_for_session_exit(&manager, &pool, &spawned.id);
    let resumed = manager
        .resume(
            &spawned.id,
            None,
            None,
            data.path(),
            pool.clone(),
            capture(),
        )
        .unwrap();
    let args = fake.last_spawn_spec().unwrap().args;
    manager.kill(&resumed.id).unwrap();
    assert!(has_arg_pair(&args, "--resume", &key));
    assert!(has_arg_pair(
        &args,
        "--permission-mode",
        "bypassPermissions"
    ));
    assert!(!args.contains(&"--verbatim".into()));
    assert_eq!(
        config.join("hooks/runner-status.json").is_file(),
        !cfg!(windows)
    );
    assert!(!data.path().join("missing").exists());
}
