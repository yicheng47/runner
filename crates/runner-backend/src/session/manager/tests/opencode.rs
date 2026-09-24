use super::*;

use crate::session::opencode::{CONFIG_CONTENT_ENV, REKEY_PATH_ENV};

fn session_key(pool: &DbPool, session_id: &str) -> Option<String> {
    crate::repo::session::get_row(&pool.get().unwrap(), session_id)
        .unwrap()
        .unwrap()
        .agent_session_key
}

fn wait_for_key(pool: &DbPool, session_id: &str, expected: &str) {
    let deadline = Instant::now() + ci_scaled_budget(Duration::from_secs(5));
    while session_key(pool, session_id).as_deref() != Some(expected) {
        assert!(
            Instant::now() < deadline,
            "key {expected} never captured for {session_id}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// What Runner's plugin does inside OpenCode on `session.created`.
fn report(spec: &SpawnSpec, key: &str) {
    let path = PathBuf::from(&spec.env[REKEY_PATH_ENV]);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let temporary = path.with_extension("json.tmp");
    std::fs::write(
        &temporary,
        serde_json::json!({ "session_id": key }).to_string(),
    )
    .unwrap();
    std::fs::rename(temporary, path).unwrap();
}

fn opencode_store(home: &Path) -> rusqlite::Connection {
    let db = home.join(".local/share/opencode/opencode.db");
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    let conn = rusqlite::Connection::open(db).unwrap();
    conn.execute_batch("CREATE TABLE IF NOT EXISTS session (id text PRIMARY KEY)")
        .unwrap();
    conn
}

fn assert_plugin_env(spec: &SpawnSpec, app_data: &Path) {
    assert_eq!(spec.env["OPENCODE_DISABLE_AUTOUPDATE"], "1");
    let plugin = crate::session::opencode::plugin_path(app_data)
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&spec.env[CONFIG_CONTENT_ENV]).unwrap(),
        serde_json::json!({ "plugin": [plugin] })
    );
    assert_eq!(
        spec.env[REKEY_PATH_ENV],
        crate::session::hook_feed::hook_path(&crate::session::claude_rekey::drop_path(
            app_data,
            &spec.session_id
        ))
    );
}

#[test]
fn opencode_direct_chat_takes_the_reported_key_and_resumes_by_session() {
    let pool = pool_with_schema();
    let app_data = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    crate::session::opencode::install_plugin(app_data.path()).unwrap();
    let mut role = role("opencode", &["--role-flag"]);
    role.runtime = "opencode".into();
    role.model = Some("anthropic/claude-sonnet-4-5".into());
    role.effort = Some("high".into());
    role.system_prompt = Some("PERSONA".into());
    insert_role_row(&pool.get().unwrap(), &role);
    let events = capture();
    let _watcher = crate::session::claude_rekey::ClaudeSessionKeyWatcher::start(
        app_data.path(),
        Arc::clone(&pool),
        events.clone(),
    )
    .unwrap();

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = router::runtime::with_conversation_home(home.path(), || {
        mgr.spawn_direct(
            &role,
            None,
            None,
            None,
            None,
            Some(app_data.path().to_str().unwrap()),
            None,
            None,
            app_data.path(),
            Arc::clone(&pool),
            events.clone(),
            Some("persona first turn".into()),
        )
    })
    .unwrap();
    let fresh = fake.last_spawn_spec().unwrap();
    assert_eq!(
        fresh.args,
        [
            "--role-flag",
            "--model",
            "anthropic/claude-sonnet-4-5",
            "--prompt",
            "persona first turn",
        ]
    );
    assert_plugin_env(&fresh, app_data.path());
    assert_eq!(session_key(&pool, &spawned.id), None);

    let key = "ses_f31048251ffepsv6qvMgfycvy1";
    report(&fresh, key);
    wait_for_key(&pool, &spawned.id, key);
    assert!(events
        .updated
        .lock()
        .unwrap()
        .iter()
        .any(|event| event.session_id == spawned.id));
    mgr.kill(&spawned.id).unwrap();

    let store = opencode_store(home.path());
    store
        .execute("INSERT INTO session VALUES (?1)", [key])
        .unwrap();
    router::runtime::with_conversation_home(home.path(), || {
        mgr.resume(
            &spawned.id,
            None,
            None,
            app_data.path(),
            Arc::clone(&pool),
            capture(),
        )
    })
    .unwrap();
    let resumed = fake.last_spawn_spec().unwrap();
    assert!(has_arg_pair(&resumed.args, "--session", key));
    assert!(!resumed.args.iter().any(|arg| arg == "--prompt"));
    assert_plugin_env(&resumed, app_data.path());
    assert_eq!(session_key(&pool, &spawned.id).as_deref(), Some(key));
    mgr.kill(&spawned.id).unwrap();

    // A session deleted in OpenCode starts fresh with the persona, once, and
    // the new session's report replaces the key.
    store.execute("DELETE FROM session", []).unwrap();
    router::runtime::with_conversation_home(home.path(), || {
        mgr.resume(
            &spawned.id,
            None,
            None,
            app_data.path(),
            Arc::clone(&pool),
            capture(),
        )
    })
    .unwrap();
    let fresh_again = fake.last_spawn_spec().unwrap();
    assert!(!fresh_again.args.iter().any(|arg| arg == "--session"));
    assert_eq!(
        fresh_again
            .args
            .iter()
            .filter(|arg| *arg == "--prompt")
            .count(),
        1
    );
    assert_eq!(
        &fresh_again.args[fresh_again.args.len() - 2..],
        [
            "--prompt".to_owned(),
            router::prompt::compose_direct_first_turn(Some("PERSONA")).unwrap(),
        ]
    );
    assert_eq!(session_key(&pool, &spawned.id), None);
    let recreated = "ses_f2f554195ffevYe8HhXhQv7Pfh";
    report(&fresh_again, recreated);
    wait_for_key(&pool, &spawned.id, recreated);
    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn opencode_blank_chat_gets_its_key_from_the_plugin_and_keeps_the_users_config() {
    let pool = pool_with_schema();
    let app_data = tempfile::tempdir().unwrap();
    crate::session::opencode::install_plugin(app_data.path()).unwrap();
    let _watcher = crate::session::claude_rekey::ClaudeSessionKeyWatcher::start(
        app_data.path(),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();
    let mut role = runtime_direct_role("opencode", Some("opencode"), None, None).unwrap();
    role.env.insert(
        CONFIG_CONTENT_ENV.into(),
        r#"{"permission": {"edit": "ask", "*": "allow"}, "plugin": ["mine"]}"#.into(),
    );
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .spawn_runtime_direct(
            &role,
            None,
            Some(app_data.path().to_str().unwrap()),
            None,
            None,
            app_data.path(),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap();
    let spec = fake.last_spawn_spec().unwrap();
    assert!(spec.args.is_empty(), "{:?}", spec.args);
    let plugin = crate::session::opencode::plugin_path(app_data.path())
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        spec.env[CONFIG_CONTENT_ENV],
        serde_json::json!({
            "permission": {"edit": "ask", "*": "allow"},
            "plugin": ["mine", plugin],
        })
        .to_string()
    );
    assert_eq!(session_key(&pool, &spawned.id), None);

    // The first typed message creates the session; a later `/new` moves the key.
    report(&spec, "ses_first0000000000000000000000");
    wait_for_key(&pool, &spawned.id, "ses_first0000000000000000000000");
    report(&spec, "ses_second000000000000000000000");
    wait_for_key(&pool, &spawned.id, "ses_second000000000000000000000");
    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn opencode_direct_fork_spawns_a_native_fork_that_reports_its_own_key() {
    let pool = pool_with_schema();
    let app_data = tempfile::tempdir().unwrap();
    crate::session::opencode::install_plugin(app_data.path()).unwrap();
    let _watcher = crate::session::claude_rekey::ClaudeSessionKeyWatcher::start(
        app_data.path(),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();
    let role_id = ulid::Ulid::new().to_string();
    let source_id = ulid::Ulid::new().to_string();
    let source_key = "ses_f31048251ffepsv6qvMgfycvy1";
    let mut persisted_role = role("opencode-custom", &["--role-flag", "--auto"]);
    persisted_role.id = role_id.clone();
    persisted_role.runtime = "opencode".into();
    insert_role_row(&pool.get().unwrap(), &persisted_role);
    let mut source = crate::repo::session::SessionRowDb::new_running(source_id.clone());
    source.role_id = Some(role_id);
    source.cwd = Some(app_data.path().to_string_lossy().into_owned());
    source.agent_session_key = Some(source_key.into());
    source.status = crate::model::SessionStatus::Stopped;
    crate::repo::session::insert(&pool.get().unwrap(), &source).unwrap();

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let forked = mgr
        .spawn_fork(
            &source_id,
            None,
            None,
            None,
            app_data.path(),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap();
    let spec = fake.last_spawn_spec().unwrap();
    assert_eq!(
        spec.args,
        ["--role-flag", "--session", source_key, "--fork"]
    );
    assert_plugin_env(&spec, app_data.path());
    assert_eq!(session_key(&pool, &forked.id), None);

    let fork_key = "ses_f2f551714ffeRYWmHy0q9aD9Tb";
    report(&spec, fork_key);
    wait_for_key(&pool, &forked.id, fork_key);
    assert_eq!(session_key(&pool, &source_id).as_deref(), Some(source_key));
    mgr.kill(&forked.id).unwrap();
}

#[test]
fn opencode_slot_resumes_by_session_under_mission_bypass() {
    let (pool, app_data, id) = slot_respawn_fixture("opencode", false);
    let key = "ses_f31048251ffepsv6qvMgfycvy1";
    pool.get()
        .unwrap()
        .execute(
            "UPDATE sessions SET agent_session_key = ?2 WHERE id = ?1",
            rusqlite::params![id, key],
        )
        .unwrap();
    let home = tempfile::tempdir().unwrap();
    opencode_store(home.path())
        .execute("INSERT INTO session VALUES (?1)", [key])
        .unwrap();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());
    router::runtime::with_conversation_home(home.path(), || {
        mgr.resume(&id, None, None, app_data.path(), pool.clone(), capture())
    })
    .unwrap();
    let args = fake.last_spawn_spec().unwrap().args;
    assert!(has_arg_pair(&args, "--session", key));
    assert!(args.iter().any(|arg| arg == "--auto"));
    assert!(!args
        .iter()
        .any(|arg| arg == "--prompt" || arg == "--add-dir"));
    mgr.kill(&id).unwrap();
}

#[test]
fn opencode_plugin_env_is_skipped_without_the_plugin_or_for_other_runtimes() {
    let pool = pool_with_schema();
    let app_data = tempfile::tempdir().unwrap();
    let spawn = |runtime: &str| {
        let role = runtime_direct_role(runtime, Some(runtime), None, None).unwrap();
        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        let spawned = mgr
            .spawn_runtime_direct(
                &role,
                None,
                Some(app_data.path().to_str().unwrap()),
                None,
                None,
                app_data.path(),
                Arc::clone(&pool),
                capture(),
            )
            .unwrap();
        let spec = fake.last_spawn_spec().unwrap();
        mgr.kill(&spawned.id).unwrap();
        spec
    };
    let without_plugin = spawn("opencode");
    assert!(!without_plugin.env.contains_key(CONFIG_CONTENT_ENV));
    assert!(!without_plugin.env.contains_key(REKEY_PATH_ENV));
    assert_eq!(without_plugin.env["OPENCODE_DISABLE_AUTOUPDATE"], "1");

    crate::session::opencode::install_plugin(app_data.path()).unwrap();
    for runtime in ["claude-code", "codex", "pi", "copilot", "antigravity"] {
        let spec = spawn(runtime);
        for name in [
            CONFIG_CONTENT_ENV,
            REKEY_PATH_ENV,
            "OPENCODE_DISABLE_AUTOUPDATE",
        ] {
            assert!(!spec.env.contains_key(name), "{runtime} {name}");
        }
    }
}
