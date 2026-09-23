use super::*;

#[test]
fn pi_direct_spawn_and_relocated_agent_dir_resume_use_live_prompt_file_and_never_approve() {
    let pool = pool_with_schema();
    let app_data = tempfile::tempdir().unwrap();
    crate::session::pi_status::install_extension(app_data.path()).unwrap();
    let cwd = app_data.path().to_string_lossy().into_owned();
    let pi_agent_dir = app_data.path().join("pi-agent");
    let mut role = role("pi-custom", &["--role-flag"]);
    role.runtime = "pi".into();
    role.system_prompt = Some("PERSONA_V1".into());
    role.model = Some("deepseek/deepseek-v4-pro".into());
    role.effort = Some("High".into());
    role.env.insert(
        "PI_CODING_AGENT_DIR".into(),
        pi_agent_dir.to_string_lossy().into_owned(),
    );
    insert_role_row(&pool.get().unwrap(), &role);

    let fake = fake_runtime();
    let spawn_pool = Arc::clone(&pool);
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
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .spawn_direct(
            &role,
            None,
            None,
            None,
            None,
            Some(&cwd),
            None,
            None,
            app_data.path(),
            Arc::clone(&pool),
            capture(),
            Some("PERSONA_V1".into()),
        )
        .unwrap();
    let row = crate::repo::session::get_row(&pool.get().unwrap(), &spawned.id)
        .unwrap()
        .unwrap();
    let key = row.agent_session_key.clone().unwrap();
    let started_at = row.started_at.unwrap().to_rfc3339();
    assert_eq!(
        crate::repo::session::set_live_title(
            &pool.get().unwrap(),
            &spawned.id,
            Some("PI_RESUME_TITLE"),
            Some(&started_at),
        )
        .unwrap(),
        1
    );
    let prompt_path = crate::session::system_prompt::path(app_data.path(), &spawned.id);
    let fresh = fake.last_spawn_spec().unwrap();
    let extension_path = crate::session::pi_status::extension_path(app_data.path());
    assert_eq!(
        fresh.args,
        [
            "--session-id",
            key.as_str(),
            "--role-flag",
            "--model",
            "deepseek/deepseek-v4-pro",
            "--thinking",
            "high",
            "-e",
            extension_path.to_str().unwrap(),
            "--append-system-prompt",
            prompt_path.to_str().unwrap(),
        ]
    );
    assert_eq!(
        fresh.env.get("PI_SKIP_VERSION_CHECK").map(String::as_str),
        Some("1")
    );
    assert_eq!(
        fresh.env[crate::session::pi_status::PATH_ENV],
        crate::session::hook_feed::hook_path(&crate::session::hook_feed::status_path(
            app_data.path(),
            &spawned.id
        ))
    );
    assert!(uuid::Uuid::parse_str(&fresh.env[crate::session::pi_status::GENERATION_ENV]).is_ok());
    assert_eq!(fresh.env[crate::session::pi_status::SESSION_KEY_ENV], key);
    assert_eq!(
        fresh.env[crate::session::pi_status::REKEY_PATH_ENV],
        crate::session::hook_feed::hook_path(&crate::session::claude_rekey::drop_path(
            app_data.path(),
            &spawned.id
        ))
    );
    let fresh_generation = fresh.env[crate::session::pi_status::GENERATION_ENV].clone();
    assert!(!fresh.args.iter().any(|arg| arg == "--approve"));
    assert_eq!(std::fs::read_to_string(&prompt_path).unwrap(), "PERSONA_V1");

    mgr.kill(&spawned.id).unwrap();
    wait_for_session_exit(&mgr, &pool, &spawned.id);
    assert!(!prompt_path.exists());

    role.system_prompt = Some("PERSONA_V2".into());
    update_role_row(&pool.get().unwrap(), &role);
    let slug = router::runtime::pi_project_slug(&cwd);
    let pi_sessions = pi_agent_dir.join("sessions").join(slug);
    std::fs::create_dir_all(&pi_sessions).unwrap();
    std::fs::write(pi_sessions.join(format!("resume_{key}.jsonl")), "").unwrap();
    let stale_rekey = crate::session::claude_rekey::drop_path(app_data.path(), &spawned.id);
    std::fs::create_dir_all(stale_rekey.parent().unwrap()).unwrap();
    std::fs::write(&stale_rekey, "stale report").unwrap();

    router::runtime::with_conversation_home(app_data.path(), || {
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
    assert!(has_arg_pair(&resumed.args, "--session-id", &key));
    assert!(has_arg_pair(
        &resumed.args,
        "--append-system-prompt",
        prompt_path.to_str().unwrap()
    ));
    assert!(has_arg_pair(
        &resumed.args,
        "-e",
        extension_path.to_str().unwrap()
    ));
    assert_eq!(resumed.env[crate::session::pi_status::SESSION_KEY_ENV], key);
    assert_eq!(
        resumed.env[crate::session::pi_status::PATH_ENV],
        fresh.env[crate::session::pi_status::PATH_ENV]
    );
    assert_eq!(
        resumed.env[crate::session::pi_status::REKEY_PATH_ENV],
        fresh.env[crate::session::pi_status::REKEY_PATH_ENV]
    );
    assert_ne!(
        resumed.env[crate::session::pi_status::GENERATION_ENV],
        fresh_generation
    );
    assert!(!stale_rekey.exists());
    assert!(!resumed.args.iter().any(|arg| arg == "--"));
    assert!(!resumed.args.iter().any(|arg| arg == "--approve"));
    assert_eq!(std::fs::read_to_string(&prompt_path).unwrap(), "PERSONA_V2");
    assert_eq!(
        crate::repo::session::get_row(&pool.get().unwrap(), &spawned.id)
            .unwrap()
            .unwrap()
            .live_title
            .as_deref(),
        Some("PI_RESUME_TITLE"),
        "the relocated conversation must resolve as a genuine resume",
    );
    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn pi_lead_and_worker_split_prompt_channels_and_approve_only_the_slot() {
    let worker_body =
        router::prompt::compose_worker_first_turn(Some("WORKER_BRIEF"), Some("TEAM_RULES"));

    for lead in [true, false] {
        let pool = pool_with_schema();
        let app_data = tempfile::tempdir().unwrap();
        crate::session::pi_status::install_extension(app_data.path()).unwrap();
        let mut role = role("pi-custom", &["--role-flag"]);
        role.runtime = "pi".into();
        role.system_prompt = Some(if lead { "LEAD_BRIEF" } else { "WORKER_BRIEF" }.into());
        let (mission, mut slot) = seed_mission_rows(&pool, &role);
        slot.lead = lead;
        pool.get()
            .unwrap()
            .execute(
                "UPDATE slots SET lead = ?1 WHERE id = ?2",
                params![lead, slot.id],
            )
            .unwrap();
        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        let prompt_channels = if lead {
            let roster = [
                router::prompt::RosterEntry {
                    handle: "lead",
                    display_name: "Lead",
                    lead: true,
                },
                router::prompt::RosterEntry {
                    handle: "worker",
                    display_name: "Worker",
                    lead: false,
                },
            ];
            router::prompt::compose_lead_prompt_channels(
                Some(Runtime::Pi),
                &router::prompt::LaunchPromptInput {
                    lead: router::prompt::LeadView {
                        handle: "lead",
                        display_name: "Lead",
                        system_prompt: Some("LEAD_BRIEF"),
                    },
                    crew_name: "crew",
                    mission_goal: "@ship - safely",
                    roster: &roster,
                    allowed_signals: &[],
                    crew_addendum: None,
                },
            )
        } else {
            router::prompt::split_session_prompt(
                Some(Runtime::Pi),
                router::prompt::SessionPromptKind::Worker,
                Some(worker_body.clone()),
            )
        };
        let spawned = mgr
            .spawn_with_prompt_channels(
                &mission,
                &role,
                &slot,
                app_data.path(),
                runner_core::event_log::path::events_path(
                    app_data.path(),
                    &mission.crew_id,
                    &mission.id,
                ),
                Arc::clone(&pool),
                capture(),
                prompt_channels.0.clone(),
                prompt_channels.1.clone(),
            )
            .unwrap();
        let spec = fake.last_spawn_spec().unwrap();
        let key = crate::repo::session::get_row(&pool.get().unwrap(), &spawned.id)
            .unwrap()
            .unwrap()
            .agent_session_key
            .unwrap();
        assert_eq!(
            &spec.args[..3],
            ["--session-id", key.as_str(), "--role-flag"]
        );
        assert!(spec.args.iter().any(|arg| arg == "--approve"));
        assert!(has_arg_pair(
            &spec.args,
            "-e",
            crate::session::pi_status::extension_path(app_data.path())
                .to_str()
                .unwrap()
        ));
        assert_eq!(spec.env[crate::session::pi_status::SESSION_KEY_ENV], key);
        assert!(
            uuid::Uuid::parse_str(&spec.env[crate::session::pi_status::GENERATION_ENV]).is_ok()
        );
        assert_eq!(
            spec.env[crate::session::pi_status::PATH_ENV],
            crate::session::hook_feed::hook_path(&crate::session::hook_feed::status_path(
                app_data.path(),
                &spawned.id
            ))
        );
        assert_eq!(
            spec.env[crate::session::pi_status::REKEY_PATH_ENV],
            crate::session::hook_feed::hook_path(&crate::session::claude_rekey::drop_path(
                app_data.path(),
                &spawned.id
            ))
        );
        let prompt_path = crate::session::system_prompt::path(app_data.path(), &spawned.id);
        assert!(has_arg_pair(
            &spec.args,
            "--append-system-prompt",
            prompt_path.to_str().unwrap()
        ));
        if lead {
            assert_eq!(
                std::fs::read_to_string(&prompt_path).unwrap(),
                prompt_channels.0.unwrap()
            );
            assert_eq!(
                &spec.args[spec.args.len() - 2..],
                ["--", prompt_channels.1.as_deref().unwrap()]
            );
        } else {
            assert_eq!(std::fs::read_to_string(&prompt_path).unwrap(), worker_body);
            assert!(!spec.args.iter().any(|arg| arg == "--"));
            assert!(!spec.args.iter().any(|arg| arg == "WORKER_BRIEF"));
        }
        mgr.kill(&spawned.id).unwrap();
    }
}

#[test]
fn pi_missing_lead_conversation_keeps_id_and_resends_only_the_goal_turn() {
    let (pool, app_data, id) = slot_respawn_fixture("pi", true);
    let key = crate::repo::session::get_row(&pool.get().unwrap(), &id)
        .unwrap()
        .unwrap()
        .agent_session_key
        .unwrap();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    router::runtime::with_conversation_home(app_data.path(), || {
        mgr.resume(
            &id,
            None,
            None,
            app_data.path(),
            Arc::clone(&pool),
            capture(),
        )
    })
    .unwrap();
    let spec = fake.last_spawn_spec().unwrap();
    assert!(has_arg_pair(&spec.args, "--session-id", &key));
    assert!(spec.args.iter().any(|arg| arg == "--approve"));
    assert_eq!(spec.args[spec.args.len() - 2], "--");
    assert_eq!(
        spec.args.last().map(String::as_str),
        Some("== Mission ==\nGoal: LATEST_GOAL\n\n")
    );
    let persisted = crate::repo::session::get_row(&pool.get().unwrap(), &id)
        .unwrap()
        .unwrap();
    assert_eq!(persisted.agent_session_key.as_deref(), Some(key.as_str()));
    let prompt =
        std::fs::read_to_string(crate::session::system_prompt::path(app_data.path(), &id)).unwrap();
    assert!(prompt.contains("SLOT_BRIEF"));
    assert!(prompt.contains("TEAM_RULES"));
    assert!(prompt.contains("== Coordination =="));
    assert!(!prompt.contains("== Mission =="));
    mgr.kill(&id).unwrap();
}

#[test]
fn pi_launch_resume_recreates_an_untouched_worker_with_the_same_id() {
    let (pool, app_data, id) = slot_respawn_fixture("pi", false);
    let key = crate::repo::session::get_row(&pool.get().unwrap(), &id)
        .unwrap()
        .unwrap()
        .agent_session_key
        .unwrap();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));

    router::runtime::with_conversation_home(app_data.path(), || {
        mgr.resume_on_launch(
            &id,
            None,
            None,
            app_data.path(),
            Arc::clone(&pool),
            capture(),
        )
    })
    .unwrap();

    let spec = fake.last_spawn_spec().unwrap();
    assert!(has_arg_pair(&spec.args, "--session-id", &key));
    assert!(!spec.args.iter().any(|arg| arg == "--"));
    assert_eq!(
        crate::repo::session::get_row(&pool.get().unwrap(), &id)
            .unwrap()
            .unwrap()
            .agent_session_key
            .as_deref(),
        Some(key.as_str())
    );
    mgr.kill(&id).unwrap();
}

#[test]
fn pi_launch_resume_refuses_a_row_without_an_assigned_key() {
    let (pool, app_data, id) = slot_respawn_fixture("pi", false);
    pool.get()
        .unwrap()
        .execute(
            "UPDATE sessions SET agent_session_key = NULL WHERE id = ?1",
            params![id],
        )
        .unwrap();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));

    let error = mgr
        .resume_on_launch(
            &id,
            None,
            None,
            app_data.path(),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap_err();

    assert!(error.to_string().contains("cannot resume"));
    assert_eq!(fake.spawn_count(), 0);
}

#[test]
fn pi_restart_running_worker_rewrites_prompt_after_old_forwarder_exits() {
    let (pool, app_data, id) = slot_respawn_fixture("pi", false);
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    mgr.restart(
        &id,
        None,
        None,
        app_data.path(),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();

    let mut role = crate::repo::role::list(&pool.get().unwrap())
        .unwrap()
        .pop()
        .unwrap();
    role.system_prompt = Some("UPDATED_BRIEF".into());
    update_role_row(&pool.get().unwrap(), &role);
    mgr.restart(
        &id,
        None,
        None,
        app_data.path(),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();

    let prompt_path = crate::session::system_prompt::path(app_data.path(), &id);
    assert_eq!(
        std::fs::read_to_string(&prompt_path).unwrap(),
        router::prompt::compose_worker_first_turn(Some("UPDATED_BRIEF"), Some("TEAM_RULES"))
    );
    assert!(has_arg_pair(
        &fake.last_spawn_spec().unwrap().args,
        "--append-system-prompt",
        prompt_path.to_str().unwrap()
    ));
    assert_eq!(fake.spawn_count(), 2);
    mgr.kill(&id).unwrap();
}

#[test]
fn pi_direct_fork_uses_native_args_and_gets_its_own_prompt_file() {
    let pool = pool_with_schema();
    let app_data = tempfile::tempdir().unwrap();
    let role_id = ulid::Ulid::new().to_string();
    let source_id = ulid::Ulid::new().to_string();
    let source_key = uuid::Uuid::new_v4().to_string();
    let mut persisted_role = role("pi-custom", &["--role-flag"]);
    persisted_role.id = role_id.clone();
    persisted_role.runtime = "pi".into();
    persisted_role.system_prompt = Some("FORK_PERSONA".into());
    insert_role_row(&pool.get().unwrap(), &persisted_role);
    let mut source = crate::repo::session::SessionRowDb::new_running(source_id.clone());
    source.role_id = Some(role_id);
    source.cwd = Some(app_data.path().to_string_lossy().into_owned());
    source.agent_session_key = Some(source_key.clone());
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
    let row = crate::repo::session::get_row(&pool.get().unwrap(), &forked.id)
        .unwrap()
        .unwrap();
    let fork_key = row.agent_session_key.unwrap();
    let prompt_path = crate::session::system_prompt::path(app_data.path(), &forked.id);
    let spec = fake.last_spawn_spec().unwrap();
    assert_eq!(
        &spec.args[..5],
        [
            "--fork",
            source_key.as_str(),
            "--session-id",
            fork_key.as_str(),
            "--role-flag",
        ]
    );
    assert!(has_arg_pair(
        &spec.args,
        "--append-system-prompt",
        prompt_path.to_str().unwrap()
    ));
    assert_eq!(
        std::fs::read_to_string(&prompt_path).unwrap(),
        "FORK_PERSONA"
    );
    assert_ne!(forked.id, source_id);
    assert!(!crate::session::system_prompt::path(app_data.path(), &source_id).exists());
    mgr.kill(&forked.id).unwrap();
}
