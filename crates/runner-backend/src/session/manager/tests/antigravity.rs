use super::*;

fn created_line(key: &str) -> String {
    format!("I0923 10:00:00.000000     580 server.go:1224] Created conversation {key}\n")
}

fn append_log(app_data: &Path, session_id: &str, text: &str) {
    use std::io::Write as _;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(crate::session::agy_capture::log_path(app_data, session_id))
        .unwrap()
        .write_all(text.as_bytes())
        .unwrap();
}

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

fn conversation_db(home: &Path, key: &str) -> PathBuf {
    let path = home
        .join(".gemini/antigravity-cli/conversations")
        .join(format!("{key}.db"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    path
}

#[test]
fn antigravity_direct_chat_seeds_trust_captures_its_key_and_resumes_by_conversation() {
    let pool = pool_with_schema();
    let app_data = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    crate::session::agy_status::install_hooks(app_data.path()).unwrap();
    let project = app_data.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let canonical = std::fs::canonicalize(&project).unwrap();
    let mut role = role("agy", &["--role-flag"]);
    role.runtime = "antigravity".into();
    role.model = Some("gemini-3.1-pro".into());
    role.effort = Some("High".into());
    role.system_prompt = Some("PERSONA".into());
    insert_role_row(&pool.get().unwrap(), &role);

    let fake = fake_runtime();
    let settings = crate::session::agy_trust::settings_path(home.path());
    let settings_for_hook = settings.clone();
    let expected_trust = canonical.to_string_lossy().into_owned();
    *fake.spawn_hook.lock().unwrap() = Some(Box::new(move || {
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings_for_hook).unwrap()).unwrap();
        assert!(value["trustedWorkspaces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry.as_str() == Some(expected_trust.as_str())));
    }));
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let events = capture();
    let spawned = router::runtime::with_conversation_home(home.path(), || {
        mgr.spawn_direct(
            &role,
            None,
            None,
            None,
            None,
            Some(project.to_str().unwrap()),
            None,
            None,
            app_data.path(),
            Arc::clone(&pool),
            events.clone(),
            Some("persona first turn".into()),
        )
    })
    .unwrap();
    assert_eq!(session_key(&pool, &spawned.id), None);

    let log = crate::session::agy_capture::log_path(app_data.path(), &spawned.id);
    let hooks = crate::session::agy_status::hooks_dir(app_data.path());
    let fresh = fake.last_spawn_spec().unwrap();
    let mut expected = vec![
        "--role-flag",
        "--model",
        "gemini-3.1-pro",
        "--effort",
        "high",
        "--log-file",
        log.to_str().unwrap(),
    ];
    if cfg!(not(windows)) {
        expected.extend(["--add-dir", hooks.to_str().unwrap()]);
        assert_eq!(
            fresh.env[crate::session::agy_status::PATH_ENV],
            crate::session::hook_feed::hook_path(&crate::session::hook_feed::status_path(
                app_data.path(),
                &spawned.id
            ))
        );
        assert!(
            uuid::Uuid::parse_str(&fresh.env[crate::session::agy_status::GENERATION_ENV]).is_ok()
        );
    } else {
        assert!(!fresh.env.contains_key(crate::session::agy_status::PATH_ENV));
    }
    expected.extend(["-i", "persona first turn"]);
    assert_eq!(fresh.args, expected);
    assert!(log.parent().unwrap().is_dir());

    let key = uuid::Uuid::new_v4().to_string();
    append_log(
        app_data.path(),
        &spawned.id,
        "I0923 10:00:00.000000       1 hooks_manager.go:53] loaded 2 named hooks from 2 hooks.json file(s)\n",
    );
    append_log(app_data.path(), &spawned.id, &created_line(&key));
    wait_for_key(&pool, &spawned.id, &key);
    assert!(events
        .updated
        .lock()
        .unwrap()
        .iter()
        .any(|event| event.session_id == spawned.id));

    mgr.kill(&spawned.id).unwrap();
    std::fs::write(conversation_db(home.path(), &key), "").unwrap();
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
    let resumed = fake.last_spawn_spec().unwrap().args;
    assert!(has_arg_pair(&resumed, "--conversation", &key));
    assert!(has_arg_pair(&resumed, "--log-file", log.to_str().unwrap()));
    assert!(!resumed.iter().any(|arg| arg == "-i"));
    assert!(
        !log.exists(),
        "a resume must not reread the last spawn's log"
    );
    assert_eq!(
        session_key(&pool, &spawned.id).as_deref(),
        Some(key.as_str())
    );

    // agy's own "not found" fallback starts a new conversation; its line wins.
    let replacement = uuid::Uuid::new_v4().to_string();
    append_log(app_data.path(), &spawned.id, &created_line(&replacement));
    wait_for_key(&pool, &spawned.id, &replacement);
    mgr.kill(&spawned.id).unwrap();

    // The replacement has no conversation file, so the next resume starts
    // fresh and resends the persona exactly once.
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
    let fresh_again = fake.last_spawn_spec().unwrap().args;
    assert!(!fresh_again.iter().any(|arg| arg == "--conversation"));
    assert_eq!(fresh_again.iter().filter(|arg| *arg == "-i").count(), 1);
    assert_eq!(
        &fresh_again[fresh_again.len() - 2..],
        [
            "-i".to_owned(),
            router::prompt::compose_direct_first_turn(Some("PERSONA")).unwrap(),
        ]
    );
    assert_eq!(session_key(&pool, &spawned.id), None);
    let recreated = uuid::Uuid::new_v4().to_string();
    append_log(app_data.path(), &spawned.id, &created_line(&recreated));
    wait_for_key(&pool, &spawned.id, &recreated);
    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn antigravity_blank_chat_has_no_key_until_its_first_message() {
    let pool = pool_with_schema();
    let app_data = tempfile::tempdir().unwrap();
    let role = runtime_direct_role("antigravity", Some("agy"), None, None).unwrap();
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
    let args = fake.last_spawn_spec().unwrap().args;
    assert!(!args
        .iter()
        .any(|arg| arg == "-i" || arg == "--conversation"));
    assert!(!args.iter().any(|arg| arg == "--model" || arg == "--effort"));

    append_log(
        app_data.path(),
        &spawned.id,
        "I0923 10:00:00.000000     494 manager.go:934] Full redraw completed (rerenderAll) for conversation  (epoch 0, items 1)\n",
    );
    std::thread::sleep(Duration::from_millis(900));
    assert_eq!(session_key(&pool, &spawned.id), None);

    let key = uuid::Uuid::new_v4().to_string();
    append_log(app_data.path(), &spawned.id, &created_line(&key));
    wait_for_key(&pool, &spawned.id, &key);
    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn antigravity_slot_resume_uses_the_conversation_or_starts_fresh_with_its_turn() {
    let (pool, app_data, id) = slot_respawn_fixture("antigravity", false);
    let key = session_key(&pool, &id).unwrap();
    let home = tempfile::tempdir().unwrap();
    let (crew_id, mission_id): (String, String) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT m.crew_id, m.id FROM sessions s JOIN missions m ON m.id = s.mission_id
              WHERE s.id = ?1",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let mission_dir =
        runner_core::event_log::path::mission_dir(app_data.path(), &crew_id, &mission_id);
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());

    std::fs::write(conversation_db(home.path(), &key), "").unwrap();
    router::runtime::with_conversation_home(home.path(), || {
        mgr.resume(&id, None, None, app_data.path(), pool.clone(), capture())
    })
    .unwrap();
    let args = fake.last_spawn_spec().unwrap().args;
    assert!(has_arg_pair(&args, "--conversation", &key));
    assert!(has_arg_pair(
        &args,
        "--add-dir",
        mission_dir.to_str().unwrap()
    ));
    assert!(args
        .iter()
        .any(|arg| arg == "--dangerously-skip-permissions"));
    assert!(!args.iter().any(|arg| arg == "-i"));
    mgr.kill(&id).unwrap();

    std::fs::remove_file(conversation_db(home.path(), &key)).unwrap();
    router::runtime::with_conversation_home(home.path(), || {
        mgr.resume(&id, None, None, app_data.path(), pool.clone(), capture())
    })
    .unwrap();
    let args = fake.last_spawn_spec().unwrap().args;
    assert!(!args.iter().any(|arg| arg == "--conversation"));
    assert!(has_arg_pair(
        &args,
        "--add-dir",
        mission_dir.to_str().unwrap()
    ));
    assert_eq!(
        &args[args.len() - 2..],
        [
            "-i".to_owned(),
            router::prompt::compose_worker_first_turn(Some("SLOT_BRIEF"), Some("TEAM_RULES")),
        ]
    );
    assert_eq!(session_key(&pool, &id), None);
    let fresh_key = uuid::Uuid::new_v4().to_string();
    append_log(app_data.path(), &id, &created_line(&fresh_key));
    wait_for_key(&pool, &id, &fresh_key);
    mgr.kill(&id).unwrap();
}

#[test]
fn antigravity_capture_writes_nothing_after_its_session_stops() {
    let pool = pool_with_schema();
    let app_data = tempfile::tempdir().unwrap();
    let role = runtime_direct_role("antigravity", Some("agy"), None, None).unwrap();
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
    mgr.kill(&spawned.id).unwrap();
    std::thread::sleep(Duration::from_millis(900));
    append_log(
        app_data.path(),
        &spawned.id,
        &created_line(&uuid::Uuid::new_v4().to_string()),
    );
    std::thread::sleep(Duration::from_millis(900));
    assert_eq!(session_key(&pool, &spawned.id), None);
}
