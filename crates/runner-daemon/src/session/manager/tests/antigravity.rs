use super::*;

fn created_line(key: &str) -> String {
    format!(
        "I0923 10:00:00.000000     580 conversation_manager.go:887] Streaming conversation {key}\n"
    )
}

fn append_log(app_data: &Path, session_id: &str, text: &str) {
    use std::io::Write as _;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(crate::runtimes::antigravity::agy_capture::log_path(
            app_data, session_id,
        ))
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
    let settings = crate::runtimes::antigravity::agy_trust::settings_path(home.path());
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
    let spawned = crate::runtimes::with_conversation_home(home.path(), || {
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

    let log = crate::runtimes::antigravity::agy_capture::log_path(app_data.path(), &spawned.id);
    let settings = crate::runtimes::antigravity::antigravity_settings_args(&[], app_data.path());
    let fresh = fake.last_spawn_spec().unwrap();
    let expected = vec![
        "--role-flag",
        "--model",
        "gemini-3.1-pro",
        "--effort",
        "high",
        "--log-file",
        log.to_str().unwrap(),
        settings[0].as_str(),
        settings[1].as_str(),
        "-i",
        "persona first turn",
    ];
    assert_eq!(fresh.args, expected);
    assert!(!fresh
        .env
        .contains_key("RUNNER_ANTIGRAVITY_WORKSPACE_CONTEXT"));
    assert!(!fresh.env.contains_key("RUNNER_ANTIGRAVITY_STATUS_PATH"));
    assert!(!fresh
        .env
        .contains_key(runner_core::protocol::hook::SESSION_ENV));
    assert!(!fresh
        .env
        .contains_key(runner_core::protocol::hook::GENERATION_ENV));
    assert!(log.parent().unwrap().is_dir());

    let key = uuid::Uuid::new_v4().to_string();
    append_log(
        app_data.path(),
        &spawned.id,
        "I0923 10:00:00.000000       1 hooks_manager.go:53] loaded 2 named hooks from 2 hooks.json file(s)\n",
    );
    append_log(app_data.path(), &spawned.id, &created_line(&key));
    wait_for_key(&pool, &spawned.id, &key);
    let cleared = uuid::Uuid::new_v4().to_string();
    append_log(app_data.path(), &spawned.id, &created_line(&cleared));
    wait_for_key(&pool, &spawned.id, &cleared);
    let forked = uuid::Uuid::new_v4().to_string();
    append_log(app_data.path(), &spawned.id, &created_line(&forked));
    wait_for_key(&pool, &spawned.id, &forked);
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
    crate::runtimes::with_conversation_home(home.path(), || {
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
    let resumed_spec = fake.last_spawn_spec().unwrap();
    assert!(!resumed_spec
        .env
        .contains_key("RUNNER_ANTIGRAVITY_WORKSPACE_CONTEXT"));
    let resumed = resumed_spec.args;
    assert!(!resumed.iter().any(|arg| arg == "--add-dir"));
    assert!(has_arg_pair(&resumed, "--conversation", &key));
    assert!(has_arg_pair(&resumed, "--log-file", log.to_str().unwrap()));
    assert!(has_arg_pair(&resumed, "--settings", &settings[1]));
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
    crate::runtimes::with_conversation_home(home.path(), || {
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
fn antigravity_cwd_context_covers_chats_leads_workers_and_resumes_without_changing_workspaces() {
    let data = tempfile::tempdir().unwrap();
    let cwd = data.path().join("project space 工作");
    let user_dir = data.path().join("user workspace");
    let bus = data.path().join("mission-bus");
    for dir in [&cwd, &user_dir, &bus] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let mut role = role("agy", &["--role-flag", "--add-dir"]);
    role.runtime = "antigravity".into();
    role.args.push(user_dir.to_string_lossy().into_owned());
    let key = "019fa1b9-a133-7841-b4dd-730d376ab1d1";
    for (shape, mission, prior, first_turn) in [
        ("chat", false, None, None),
        ("lead", true, None, Some("MISSION_GOAL")),
        ("worker", true, None, Some("WORKER_CONTEXT")),
        ("chat-resume", false, Some(key), None),
        ("mission-resume", true, Some(key), None),
    ] {
        let mut spec = SpawnSpec {
            session_id: shape.into(),
            cwd: Some(cwd.clone()),
            command: role.command.clone(),
            args: role.args.clone(),
            mission,
            ..Default::default()
        };
        let plan = crate::runtimes::for_key(&role.runtime).resume_plan(prior);
        mgr_with_fake(None, fake_runtime()).apply_runtime_args(
            &mut spec,
            &role,
            &plan,
            data.path(),
            None,
            first_turn,
            mission.then_some(bus.as_path()),
            None,
        );
        assert_eq!(spec.cwd.as_deref(), Some(cwd.as_path()));
        assert_eq!(&spec.args[..role.args.len()], &role.args, "{shape}");
        assert!(
            !has_arg_pair(&spec.args, "--add-dir", bus.to_str().unwrap()),
            "{shape}"
        );
        assert_eq!(
            spec.args.iter().filter(|arg| *arg == "--add-dir").count(),
            1,
            "{shape}"
        );
        assert_eq!(
            has_arg_pair(&spec.args, "--conversation", key),
            prior.is_some(),
            "{shape}"
        );
        assert_eq!(
            spec.args.iter().any(|arg| arg == "-i"),
            first_turn.is_some()
        );
        assert!(
            spec.args.windows(2).any(|pair| pair[0] == "--settings"),
            "{shape}"
        );
        assert!(!has_arg_pair(
            &spec.args,
            "--add-dir",
            data.path().join("antigravity-hooks").to_str().unwrap()
        ));
        assert!(!spec
            .env
            .contains_key("RUNNER_ANTIGRAVITY_WORKSPACE_CONTEXT"));
    }
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
        runner_core::event_log::path::mission_dir(app_data.path(), Some(&crew_id), &mission_id);
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());

    std::fs::write(conversation_db(home.path(), &key), "").unwrap();
    crate::runtimes::with_conversation_home(home.path(), || {
        mgr.resume(&id, None, None, app_data.path(), pool.clone(), capture())
    })
    .unwrap();
    let args = fake.last_spawn_spec().unwrap().args;
    assert!(has_arg_pair(&args, "--conversation", &key));
    assert!(!has_arg_pair(
        &args,
        "--add-dir",
        mission_dir.to_str().unwrap()
    ));
    assert!(!args.iter().any(|arg| arg == "--add-dir"));
    assert!(args
        .iter()
        .any(|arg| arg == "--dangerously-skip-permissions"));
    assert!(!args.iter().any(|arg| arg == "-i"));
    mgr.kill(&id).unwrap();

    std::fs::remove_file(conversation_db(home.path(), &key)).unwrap();
    crate::runtimes::with_conversation_home(home.path(), || {
        mgr.resume(&id, None, None, app_data.path(), pool.clone(), capture())
    })
    .unwrap();
    let args = fake.last_spawn_spec().unwrap().args;
    assert!(!args.iter().any(|arg| arg == "--conversation"));
    assert!(!has_arg_pair(
        &args,
        "--add-dir",
        mission_dir.to_str().unwrap()
    ));
    assert!(!args.iter().any(|arg| arg == "--add-dir"));
    assert_eq!(
        &args[args.len() - 2..],
        [
            "-i".to_owned(),
            router::prompt::compose_member_first_turn(
                Some("SLOT_BRIEF"),
                Some("TEAM_RULES"),
                Some("LATEST_GOAL")
            ),
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

#[test]
fn antigravity_capture_syncs_turn_one_summary_title_and_honors_settings_opt_out_and_resume_generation(
) {
    fn session_live_title(pool: &DbPool, session_id: &str) -> Option<String> {
        crate::repo::session::get_row(&pool.get().unwrap(), session_id)
            .unwrap()
            .unwrap()
            .live_title
    }

    fn wait_for_live_title(pool: &DbPool, session_id: &str, expected: &str) {
        let deadline = Instant::now() + ci_scaled_budget(Duration::from_secs(5));
        while session_live_title(pool, session_id).as_deref() != Some(expected) {
            assert!(
                Instant::now() < deadline,
                "live_title {expected:?} never persisted for {session_id}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    let pool = pool_with_schema();
    let app_data = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let agy_home = home.path().join(".gemini/antigravity-cli");
    std::fs::create_dir_all(&agy_home).unwrap();
    let summaries_db = agy_home.join("conversation_summaries.db");
    let writer = rusqlite::Connection::open(&summaries_db).unwrap();
    writer.pragma_update(None, "journal_mode", "WAL").unwrap();
    writer
        .execute_batch(
            "CREATE TABLE conversation_summaries (
                conversation_id TEXT PRIMARY KEY,
                title TEXT NOT NULL DEFAULT ''
            );",
        )
        .unwrap();

    let project = app_data.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let direct_role = runtime_direct_role("antigravity", Some("agy"), None, None).unwrap();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));

    let spawned = crate::runtimes::with_conversation_home(home.path(), || {
        mgr.spawn_runtime_direct(
            &direct_role,
            None,
            Some(project.to_str().unwrap()),
            None,
            None,
            app_data.path(),
            Arc::clone(&pool),
            capture(),
        )
    })
    .unwrap();

    let key = uuid::Uuid::new_v4().to_string();
    writer
        .execute(
            "INSERT INTO conversation_summaries (conversation_id, title) VALUES (?1, ?2)",
            [&key, ""],
        )
        .unwrap();
    append_log(app_data.path(), &spawned.id, &created_line(&key));
    wait_for_key(&pool, &spawned.id, &key);
    assert_eq!(session_live_title(&pool, &spawned.id), None);

    writer
        .execute(
            "UPDATE conversation_summaries SET title = ?2 WHERE conversation_id = ?1",
            [&key, "Rainbows Form Through Light Refraction"],
        )
        .unwrap();
    wait_for_live_title(&pool, &spawned.id, "Rainbows Form Through Light Refraction");

    // Stop and resume the same conversation (`initial_key = Some(key)`), then verify
    // summary updates land under the resumed `started_at` generation.
    mgr.kill(&spawned.id).unwrap();
    std::fs::write(conversation_db(home.path(), &key), "").unwrap();
    crate::runtimes::with_conversation_home(home.path(), || {
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
    writer
        .execute(
            "UPDATE conversation_summaries SET title = ?2 WHERE conversation_id = ?1",
            [&key, "Resumed Summary Topic"],
        )
        .unwrap();
    wait_for_live_title(&pool, &spawned.id, "Resumed Summary Topic");
    mgr.kill(&spawned.id).unwrap();

    // Custom role passing `--settings` opts out of both `--settings` injection and
    // `conversation_summaries.db` polling while preserving conversation-key capture.
    let mut opt_out_role = role(
        "agy-optout",
        &["--settings", r#"{"toolPermission":"request-review"}"#],
    );
    opt_out_role.runtime = "antigravity".into();
    insert_role_row(&pool.get().unwrap(), &opt_out_role);
    let opt_out = crate::runtimes::with_conversation_home(home.path(), || {
        mgr.spawn_direct(
            &opt_out_role,
            None,
            None,
            None,
            None,
            Some(project.to_str().unwrap()),
            None,
            None,
            app_data.path(),
            Arc::clone(&pool),
            capture(),
            None,
        )
    })
    .unwrap();
    let opt_out_key = uuid::Uuid::new_v4().to_string();
    writer
        .execute(
            "INSERT INTO conversation_summaries (conversation_id, title) VALUES (?1, ?2)",
            [&opt_out_key, "Opt-Out Summary Topic"],
        )
        .unwrap();
    append_log(app_data.path(), &opt_out.id, &created_line(&opt_out_key));
    wait_for_key(&pool, &opt_out.id, &opt_out_key);
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(session_live_title(&pool, &opt_out.id), None);
    mgr.kill(&opt_out.id).unwrap();
}

#[test]
fn antigravity_summary_title_updates_attached_terminal_metadata_and_preferred_title_after_osc_title(
) {
    let app_data = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let agy_home = home.path().join(".gemini/antigravity-cli");
    std::fs::create_dir_all(&agy_home).unwrap();
    let summaries_db = agy_home.join("conversation_summaries.db");
    let writer = rusqlite::Connection::open(&summaries_db).unwrap();
    writer.pragma_update(None, "journal_mode", "WAL").unwrap();
    writer
        .execute_batch(
            "CREATE TABLE conversation_summaries (
                conversation_id TEXT PRIMARY KEY,
                title TEXT NOT NULL DEFAULT ''
            );",
        )
        .unwrap();

    let project = app_data.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let fake = fake_runtime();
    let mut core = crate::test_support::test_core_in(app_data.path().to_owned());
    core.db = Arc::new(db::open_pool(&app_data.path().join("runner.db")).unwrap());
    core.sessions = manager_with_runtime(
        crate::shell_path::LoginShellEnv::default(),
        Arc::clone(&fake) as Arc<dyn SessionRuntime>,
    );
    core.db
        .get()
        .unwrap()
        .execute_batch(
            "CREATE TABLE title_writes (title TEXT);
             CREATE TRIGGER record_title AFTER UPDATE OF live_title ON sessions
             BEGIN INSERT INTO title_writes VALUES (NEW.live_title); END;",
        )
        .unwrap();

    let registry = runner_terminal::terminal::TerminalBridge::new(
        crate::daemon::InProcessTransport::client(core.clone()),
        Arc::new(|| {}),
    )
    .unwrap();
    let direct_role = runtime_direct_role("antigravity", Some("agy"), None, None).unwrap();
    let spawned = crate::runtimes::with_conversation_home(home.path(), || {
        core.sessions.spawn_runtime_direct(
            &direct_role,
            None,
            Some(project.to_str().unwrap()),
            Some(80),
            Some(24),
            &core.app_data_dir,
            core.db.clone(),
            Arc::new(core.session_events()),
        )
    })
    .unwrap();
    let id = spawned.id;

    let deadline = Instant::now() + ci_scaled_budget(Duration::from_secs(5));
    let mirror = loop {
        if let Some(mirror) = registry.session(&id) {
            break mirror;
        }
        assert!(Instant::now() < deadline, "mirror never attached for {id}");
        std::thread::sleep(Duration::from_millis(10));
    };

    let selected_ui_title =
        |core: &crate::AppCore, mirror: &runner_terminal::terminal::TerminalMirror| {
            let entry = crate::ops::session::session_list_recent_direct(core)
                .unwrap()
                .into_iter()
                .find(|entry| entry.session_id == id)
                .unwrap();
            let live = mirror.title();
            (entry.live_title.clone(), entry.preferred_title(Some(&live)))
        };

    let push_and_flush_osc = |bytes: &[u8], expected_mirror_title: &str| {
        let prev_seq = core.sessions.terminal_output_sequence(&id);
        fake.push_output(0, bytes);
        let wait_deadline = Instant::now() + ci_scaled_budget(Duration::from_secs(5));
        while core.sessions.terminal_output_sequence(&id) <= prev_seq
            || mirror.title() != expected_mirror_title
        {
            assert!(
                Instant::now() < wait_deadline,
                "timed out waiting for output seq > {prev_seq} and mirror title {expected_mirror_title:?} (got {:?})",
                mirror.title()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        core.sessions.terminal_model(&id).unwrap().flush_events();
    };

    // 1. Conversation 1 emits OSC 2 title A (e.g. `/rename` or `--conversation` resume).
    let c1 = uuid::Uuid::new_v4().to_string();
    append_log(app_data.path(), &id, &created_line(&c1));
    wait_for_key(&core.db, &id, &c1);
    push_and_flush_osc(b"\x1b]2;Initial OSC Title A\x07", "Initial OSC Title A");
    assert_eq!(
        crate::session::manager::terminal::metadata(&core, &id)
            .unwrap()
            .title,
        "Initial OSC Title A"
    );
    assert_eq!(
        selected_ui_title(&core, &mirror),
        (
            Some("Initial OSC Title A".into()),
            Some("Initial OSC Title A".into())
        )
    );

    // 2. Switch to fresh conversation 2, which writes native summary title B into
    // `conversation_summaries.db` without emitting any OSC 2 output.
    let c2 = uuid::Uuid::new_v4().to_string();
    writer
        .execute(
            "INSERT INTO conversation_summaries (conversation_id, title) VALUES (?1, ?2)",
            [&c2, "Switched Summary Title B"],
        )
        .unwrap();
    append_log(app_data.path(), &id, &created_line(&c2));
    let switch_deadline = Instant::now() + ci_scaled_budget(Duration::from_secs(5));
    while mirror.title() != "Switched Summary Title B" {
        assert!(
            Instant::now() < switch_deadline,
            "attached mirror kept {:?} instead of updating to Switched Summary Title B",
            mirror.title()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    core.sessions.terminal_model(&id).unwrap().flush_events();
    assert_eq!(
        crate::session::manager::terminal::metadata(&core, &id)
            .unwrap()
            .title,
        "Switched Summary Title B"
    );
    assert_eq!(
        selected_ui_title(&core, &mirror),
        (
            Some("Switched Summary Title B".into()),
            Some("Switched Summary Title B".into())
        )
    );

    // 3. Identical OSC 2 is deduplicated (`0` extra DB writes), while a later `/rename`
    // back to Title A updates both SQLite and attached metadata without being reverted
    // by the unchanged summary row.
    let writes_before: usize = core
        .db
        .get()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM title_writes", [], |row| row.get(0))
        .unwrap();
    push_and_flush_osc(
        b"\x1b]2;Switched Summary Title B\x07",
        "Switched Summary Title B",
    );
    let writes_after_same: usize = core
        .db
        .get()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM title_writes", [], |row| row.get(0))
        .unwrap();
    assert_eq!(writes_after_same, writes_before);

    push_and_flush_osc(b"\x1b]2;Initial OSC Title A\x07", "Initial OSC Title A");
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(
        selected_ui_title(&core, &mirror),
        (
            Some("Initial OSC Title A".into()),
            Some("Initial OSC Title A".into())
        )
    );

    crate::ops::session::session_stop(&core, &id).unwrap();
    core.sessions.drain_terminal_workers();
}

#[test]
fn antigravity_daemon_startup_removes_legacy_hooks_folder() {
    let pool = pool_with_schema();
    let app_data = tempfile::tempdir().unwrap();
    let hooks_root = app_data.path().join("antigravity-hooks");
    let agents_dir = hooks_root.join(".agents");
    std::fs::create_dir_all(&agents_dir).unwrap();
    std::fs::write(agents_dir.join("hooks.json"), "{}").unwrap();
    std::fs::write(hooks_root.join("report.sh"), "#!/bin/sh\n").unwrap();

    let mgr = mgr_with_fake(None, fake_runtime());
    mgr.start_runtime_watchers(app_data.path(), pool, capture())
        .unwrap();

    assert!(!hooks_root.exists());
}
