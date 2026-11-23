use super::*;

#[test]
#[cfg(unix)]
fn opencode_direct_role_and_concurrent_runtime_chats_persist_exact_keys() {
    let pool = pool_with_schema();
    let data = tempfile::tempdir().unwrap();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());
    let mut role =
        runtime_direct_role("opencode", Some("custom opencode"), Some("p/model"), None).unwrap();
    role.env.insert(
        "XDG_CONFIG_HOME".into(),
        data.path().join("config").display().to_string(),
    );
    role.env.insert(
        "XDG_DATA_HOME".into(),
        data.path().join("data").display().to_string(),
    );
    role.env.insert(
        "OPENCODE_CONFIG_CONTENT".into(),
        r#"{"permissions":[{"action":"edit","resource":"secret","effect":"deny"}]}"#.into(),
    );
    role.args = vec!["--auto".into()];
    role.effort = Some("high".into());
    role.working_dir = Some(data.path().display().to_string());
    insert_role_row(&pool.get().unwrap(), &role);
    let first = mgr
        .spawn_direct(
            &role,
            DirectLaunch {
                first_turn: Some("中文 persona\nmission-free".into()),
                ..Default::default()
            },
            data.path(),
            pool.clone(),
            capture(),
        )
        .unwrap();
    let spec = fake.last_spawn_spec().unwrap();
    assert_eq!(spec.command, "custom opencode");
    assert!(spec.args.iter().any(|arg| arg == "--standalone"));
    assert!(!spec.args.iter().any(|arg| arg == "--session"));
    assert!(!spec.args.iter().any(|arg| arg == "--auto"));
    assert!(!spec.env.contains_key("RUNNER_MISSION_ID"));
    assert_eq!(spec.args.last().unwrap(), "中文 persona\nmission-free");
    let config: serde_json::Value =
        serde_json::from_str(&spec.env["OPENCODE_CONFIG_CONTENT"]).unwrap();
    assert_eq!(config["model"], "p/model");
    assert_eq!(config["permissions"][0]["effect"], "deny");
    assert_eq!(config["plugins"][0]["options"]["model"]["variant"], "high");
    let second = mgr
        .spawn_direct(
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
        .unwrap();
    assert!(!fake
        .last_spawn_spec()
        .unwrap()
        .args
        .contains(&"--session".into()));
    let key = |id: &str| {
        crate::repo::session::get_row(&pool.get().unwrap(), id)
            .unwrap()
            .unwrap()
            .agent_session_key
    };
    assert!(key(&first.id).is_none());
    assert!(key(&second.id).is_none());
    let native_keys = [
        "ses_ecc290331ffeqzayUqWErOjpRo",
        "ses_ecc290331ffeqzayUqWErOjpRp",
    ];
    {
        let spawns = fake.spawns.lock().unwrap();
        for (index, native_key) in native_keys.iter().enumerate() {
            spawns[index]
                .tx
                .as_ref()
                .unwrap()
                .send(RuntimeOutput::ConversationStart((*native_key).into()))
                .unwrap();
        }
    }
    fake.close_spawn(0);
    fake.close_spawn(1);
    wait_for_session_exit(&mgr, &pool, &first.id);
    wait_for_session_exit(&mgr, &pool, &second.id);
    assert_eq!(key(&first.id).as_deref(), Some(native_keys[0]));
    assert_eq!(key(&second.id).as_deref(), Some(native_keys[1]));
    assert_ne!(key(&first.id), key(&second.id));
    mgr.kill(&first.id).unwrap();
    mgr.kill(&second.id).unwrap();
}

#[cfg(unix)]
#[test]
fn opencode_missing_history_automatic_resume_is_safe_and_manual_resume_is_fresh() {
    let pool = pool_with_schema();
    let data = tempfile::tempdir().unwrap();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());
    db::set_runtime_override(&pool, "opencode", Some("custom-opencode-wrapper")).unwrap();
    let mut row = crate::repo::session::SessionRowDb::new_running("opencode-saved".into());
    row.agent_runtime = Some("opencode".into());
    row.agent_command = Some("custom-opencode-wrapper".into());
    row.agent_session_key = Some("ses_missing".into());
    row.status = crate::model::SessionStatus::Stopped;
    crate::test_support::insert_session_row(&pool.get().unwrap(), &row).unwrap();
    mgr.shell_env.write().unwrap().vars.insert(
        "XDG_DATA_HOME".into(),
        data.path().join("data").display().to_string(),
    );
    mgr.shell_env.write().unwrap().vars.insert(
        "XDG_CONFIG_HOME".into(),
        data.path().join("config").display().to_string(),
    );
    let error = mgr
        .resume_on_launch(&row.id, None, None, data.path(), pool.clone(), capture())
        .err()
        .unwrap();
    assert!(error.to_string().contains("conversation is unavailable"));
    assert!(fake.spawns.lock().unwrap().is_empty());
    assert_eq!(
        crate::repo::session::get_row(&pool.get().unwrap(), &row.id)
            .unwrap()
            .unwrap()
            .agent_session_key,
        row.agent_session_key
    );
    let resumed = mgr
        .resume(&row.id, None, None, data.path(), pool.clone(), capture())
        .unwrap();
    let saved = crate::repo::session::get_row(&pool.get().unwrap(), &row.id)
        .unwrap()
        .unwrap();
    assert_ne!(saved.agent_session_key, row.agent_session_key);
    assert!(saved.agent_session_key.is_none());
    assert!(!fake
        .last_spawn_spec()
        .unwrap()
        .args
        .contains(&"--session".into()));
    mgr.kill(&resumed.id).unwrap();
}

#[test]
fn opencode_mission_exact_resume_keeps_bus_permissions_and_omits_first_turn() {
    let data = tempfile::tempdir().unwrap();
    let pool = pool_with_schema();
    let fake = fake_runtime();
    let manager = mgr_with_fake(None, fake.clone());
    let mut configured = role("fixture-opencode", &[]);
    configured.runtime = "opencode".into();
    configured.model = Some("p/model".into());
    configured.effort = Some("high".into());
    configured.env.insert(
        "XDG_DATA_HOME".into(),
        data.path().join("native-data").display().to_string(),
    );
    configured.env.insert(
        "XDG_CONFIG_HOME".into(),
        data.path().join("native-config").display().to_string(),
    );
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
            Some("中文 goal\ncoordination".into()),
        )
        .unwrap();
    assert!(
        crate::repo::session::get_row(&pool.get().unwrap(), &spawned.id)
            .unwrap()
            .unwrap()
            .agent_session_key
            .is_none()
    );
    let key = "ses_ecc290331ffeqzayUqWErOjpRo".to_owned();
    fake.spawns.lock().unwrap()[0]
        .tx
        .as_ref()
        .unwrap()
        .send(RuntimeOutput::ConversationStart(key.clone()))
        .unwrap();
    let spec = fake.last_spawn_spec().unwrap();
    assert!(!spec.args.contains(&"--session".into()));
    assert!(spec.args.contains(&"--auto".into()));
    assert_eq!(spec.env["RUNNER_MISSION_ID"], mission.id);
    let config: serde_json::Value =
        serde_json::from_str(&spec.env["OPENCODE_CONFIG_CONTENT"]).unwrap();
    assert_eq!(config["plugins"][0]["options"]["model"]["variant"], "high");
    assert!(has_arg_pair(
        &spec.args,
        "--prompt",
        "中文 goal\ncoordination"
    ));
    let path = data.path().join("native-data/opencode/opencode.db");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let native = rusqlite::Connection::open(path).unwrap();
    native
        .execute_batch("CREATE TABLE session_v2(id TEXT PRIMARY KEY)")
        .unwrap();
    native
        .execute("INSERT INTO session_v2 VALUES (?1)", [&key])
        .unwrap();
    fake.close_spawn(0);
    wait_for_session_exit(&manager, &pool, &spawned.id);
    assert_eq!(
        crate::repo::session::get_row(&pool.get().unwrap(), &spawned.id)
            .unwrap()
            .unwrap()
            .agent_session_key
            .as_deref(),
        Some(key.as_str())
    );
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
    let spec = fake.last_spawn_spec().unwrap();
    assert!(has_arg_pair(&spec.args, "--session", &key));
    assert!(spec.args.contains(&"--auto".into()));
    assert!(!spec.args.contains(&"--prompt".into()));
    assert_eq!(spec.env["RUNNER_MISSION_ID"], mission.id);
    let config: serde_json::Value =
        serde_json::from_str(&spec.env["OPENCODE_CONFIG_CONTENT"]).unwrap();
    assert!(config.get("plugins").is_none());
    manager.kill(&resumed.id).unwrap();
}
