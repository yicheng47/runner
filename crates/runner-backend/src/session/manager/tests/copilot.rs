use super::*;

#[test]
fn copilot_direct_spawn_persists_key_before_spawn_and_resume_never_replays_first_turn() {
    let pool = pool_with_schema();
    let app_data = tempfile::tempdir().unwrap();
    crate::session::copilot_status::install_plugin(app_data.path()).unwrap();
    let mut role = role(
        "copilot",
        &[
            "--user-flag",
            "kept",
            "--plugin-dir",
            "/user/plugin",
            "--yolo",
        ],
    );
    role.runtime = "copilot".into();
    crate::repo::role::insert(&pool.get().unwrap(), &(&role).into()).unwrap();
    let fake = fake_runtime();
    let spawn_pool = pool.clone();
    *fake.spawn_hook.lock().unwrap() = Some(Box::new(move || {
        let key: String = spawn_pool
            .get()
            .unwrap()
            .query_row("SELECT agent_session_key FROM sessions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(uuid::Uuid::parse_str(&key).is_ok());
    }));
    let mgr = mgr_with_fake(None, fake.clone());
    let spawned = mgr
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
            capture(),
            Some("persona first turn".into()),
        )
        .unwrap();
    let key: String = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT agent_session_key FROM sessions WHERE id=?1",
            [&spawned.id],
            |row| row.get(0),
        )
        .unwrap();
    let fresh_spec = fake.last_spawn_spec().unwrap();
    let args = fresh_spec.args;
    let fresh_generation = fresh_spec
        .env
        .get(crate::session::copilot_status::GENERATION_ENV)
        .cloned();
    let plugin_dir = crate::session::copilot_status::plugin_dir(app_data.path())
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        fresh_spec.env[crate::session::copilot_status::PATH_ENV],
        crate::session::hook_feed::hook_path(&crate::session::hook_feed::status_path(
            app_data.path(),
            &spawned.id
        ))
    );
    assert!(
        uuid::Uuid::parse_str(&fresh_spec.env[crate::session::copilot_status::GENERATION_ENV])
            .is_ok()
    );
    let expected = vec![
        "--user-flag".to_owned(),
        "kept".to_owned(),
        "--plugin-dir".to_owned(),
        "/user/plugin".to_owned(),
        "--session-id".to_owned(),
        key.clone(),
        "--no-auto-update".to_owned(),
        "--plugin-dir".to_owned(),
        plugin_dir.clone(),
        "-i".to_owned(),
        "persona first turn".to_owned(),
    ];
    assert_eq!(args, expected);
    assert!(fake.inputs.lock().unwrap().is_empty());
    mgr.kill(&spawned.id).unwrap();
    mgr.resume(
        &spawned.id,
        None,
        None,
        app_data.path(),
        pool.clone(),
        capture(),
    )
    .unwrap();
    let resumed_spec = fake.last_spawn_spec().unwrap();
    let args = resumed_spec.args;
    let resumed_generation = &resumed_spec.env[crate::session::copilot_status::GENERATION_ENV];
    assert!(uuid::Uuid::parse_str(resumed_generation).is_ok());
    assert_ne!(
        fresh_generation.as_deref(),
        Some(resumed_generation.as_str())
    );
    let expected = vec![
        "--user-flag".to_owned(),
        "kept".to_owned(),
        "--plugin-dir".to_owned(),
        "/user/plugin".to_owned(),
        "--session-id".to_owned(),
        key,
        "--no-auto-update".to_owned(),
        "--plugin-dir".to_owned(),
        plugin_dir,
    ];
    assert_eq!(args, expected);
    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn copilot_missing_worker_conversation_keeps_id_and_delivers_first_turn() {
    let (pool, app_data, id) = slot_respawn_fixture("copilot", false);
    let key: String = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT agent_session_key FROM sessions WHERE id=?1",
            [&id],
            |row| row.get(0),
        )
        .unwrap();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());
    router::runtime::with_conversation_home(app_data.path(), || {
        mgr.resume(&id, None, None, app_data.path(), pool.clone(), capture())
    })
    .unwrap();
    let spec = fake.last_spawn_spec().unwrap();
    assert!(has_arg_pair(&spec.args, "--session-id", &key));
    assert_eq!(spec.args[spec.args.len() - 2], "-i");
    assert_eq!(
        spec.args.last().unwrap(),
        &router::prompt::compose_worker_first_turn(Some("SLOT_BRIEF"), Some("TEAM_RULES"))
    );
    mgr.kill(&id).unwrap();
}
