use super::*;

#[test]
#[cfg(unix)]
fn cursor_new_chats_spawn_with_native_permissions_and_assigned_identity() {
    let pool = pool_with_schema();
    let data = tempfile::tempdir().unwrap();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());
    let mut role = runtime_direct_role(
        "cursor",
        Some("custom-cursor-wrapper"),
        Some("grok-4.7-high-fast"),
        None,
    )
    .unwrap();
    role.args = vec![
        "--force".into(),
        "--sandbox".into(),
        "disabled".into(),
        "--trust".into(),
        "--approve-mcps".into(),
    ];
    insert_role_row(&pool.get().unwrap(), &role);
    for role_backed in [false, true] {
        let spawned = if role_backed {
            mgr.spawn_direct(
                &role,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                data.path(),
                pool.clone(),
                capture(),
                Some("persona".into()),
            )
        } else {
            mgr.spawn_runtime_direct(
                &role,
                None,
                None,
                None,
                None,
                data.path(),
                pool.clone(),
                capture(),
            )
        }
        .unwrap();
        let spec = fake.last_spawn_spec().unwrap();
        assert_eq!(spec.command, "custom-cursor-wrapper");
        assert!(!spec.mission);
        assert!(!spec.env.contains_key("RUNNER_MISSION_ID"));
        let mut expected = vec!["--trust", "--model", "grok-4.7-high-fast"];
        if role_backed {
            expected.push("persona");
        }
        assert_eq!(&spec.args[2..], expected);
        assert_eq!(spec.args[0], "--new-session-id");
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
fn concurrent_cursor_new_chats_have_distinct_rows_without_shared_resume_identity() {
    let pool = pool_with_schema();
    let data = tempfile::tempdir().unwrap();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());
    let role = runtime_direct_role("cursor", Some("custom-cursor-wrapper"), None, None).unwrap();
    let ids = std::thread::scope(|scope| {
        let spawn = || {
            mgr.spawn_runtime_direct(
                &role,
                None,
                Some(data.path().to_str().unwrap()),
                None,
                None,
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
fn cursor_automatic_resume_rejects_missing_history_and_manual_resume_starts_fresh() {
    let pool = pool_with_schema();
    let data = tempfile::tempdir().unwrap();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());
    db::set_runtime_override(&pool, "cursor", Some("custom-cursor-wrapper")).unwrap();
    let mut row = crate::repo::session::SessionRowDb::new_running("cursor-saved".into());
    row.agent_runtime = Some("cursor".into());
    row.agent_command = Some("custom-cursor-wrapper".into());
    row.agent_session_key = Some(uuid::Uuid::new_v4().to_string());
    row.status = crate::model::SessionStatus::Stopped;
    crate::repo::session::insert(&pool.get().unwrap(), &row).unwrap();
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
    assert_eq!(fake.last_spawn_spec().unwrap().args[0], "--new-session-id");
    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn cursor_closed_conversation_clears_key_without_stopping_output_forwarder() {
    let pool = pool_with_schema();
    let mut row = crate::repo::session::SessionRowDb::new_running("cursor-rekey".into());
    row.started_at = Some(Utc::now());
    row.agent_session_key = Some(uuid::Uuid::new_v4().to_string());
    crate::repo::session::insert(&pool.get().unwrap(), &row).unwrap();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());
    let (rt_session, output) = fake
        .spawn(SpawnSpec {
            session_id: row.id.clone(),
            ..Default::default()
        })
        .unwrap();
    let new = uuid::Uuid::new_v4().to_string();
    {
        let spawns = fake.spawns.lock().unwrap();
        let tx = spawns[0].tx.as_ref().unwrap();
        tx.send(RuntimeOutput::ConversationStart(String::new()))
            .unwrap();
        tx.send(RuntimeOutput::ConversationStart(new.clone()))
            .unwrap();
    }
    fake.close_spawn(0);
    let data = tempfile::tempdir().unwrap();
    let events = capture();
    mgr.start_forwarder_thread(
        row.id.clone(),
        row.started_at.unwrap().to_rfc3339(),
        None,
        rt_session,
        output,
        pool.clone(),
        events.clone(),
        role("cursor", &[]),
        false,
        false,
        None,
        data.path().to_path_buf(),
    )
    .join()
    .unwrap();
    let saved = crate::repo::session::get_row(&pool.get().unwrap(), &row.id)
        .unwrap()
        .unwrap();
    assert_eq!(saved.agent_session_key.as_deref(), Some(new.as_str()));
    assert_eq!(events.updated.lock().unwrap().len(), 2);
}

#[cfg(unix)]
#[test]
fn cursor_resume_uses_the_same_login_shell_config_root_as_spawn() {
    use md5::{Digest, Md5};
    for variable in ["CURSOR_CONFIG_DIR", "XDG_CONFIG_HOME"] {
        let pool = pool_with_schema();
        let data = tempfile::tempdir().unwrap();
        let cwd = data.path().canonicalize().unwrap();
        let config = cwd.join("config");
        let fake = fake_runtime();
        let mgr = manager_with_runtime(
            crate::shell_path::LoginShellEnv {
                vars: BTreeMap::from([(variable.into(), config.display().to_string())]),
                ..Default::default()
            },
            fake.clone(),
        );
        db::set_runtime_override(&pool, "cursor", Some("custom-cursor-wrapper")).unwrap();
        let mut row = crate::repo::session::SessionRowDb::new_running("cursor-saved".into());
        row.agent_runtime = Some("cursor".into());
        row.agent_command = Some("custom-cursor-wrapper".into());
        row.agent_session_key = Some(uuid::Uuid::new_v4().to_string());
        row.cwd = Some(cwd.display().to_string());
        row.status = crate::model::SessionStatus::Stopped;
        crate::repo::session::insert(&pool.get().unwrap(), &row).unwrap();
        let root = if variable == "XDG_CONFIG_HOME" {
            config.join("cursor")
        } else {
            config.clone()
        };
        let store = root
            .join("chats")
            .join(format!(
                "{:x}",
                Md5::digest(cwd.to_string_lossy().as_bytes())
            ))
            .join(row.agent_session_key.as_ref().unwrap())
            .join("store.db");
        std::fs::create_dir_all(store.parent().unwrap()).unwrap();
        std::fs::write(store, []).unwrap();
        crate::golden::with_config_env(BTreeMap::new(), || {
            let resumed = mgr
                .resume_on_launch(&row.id, None, None, data.path(), pool.clone(), capture())
                .unwrap();
            let spec = fake.last_spawn_spec().unwrap();
            assert_eq!(spec.args[0], "--resume");
            assert_eq!(spec.args[1], *row.agent_session_key.as_ref().unwrap());
            assert_eq!(spec.env.get(variable), Some(&config.display().to_string()));
            mgr.kill(&resumed.id).unwrap();
        });
    }
}

#[test]
fn cursor_mission_exact_resume_keeps_bypass_permissions() {
    let data = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
    let pool = pool_with_schema();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());
    mgr.shell_env.write().unwrap().vars.insert(
        "CURSOR_CONFIG_DIR".into(),
        data.path().join("shell-config").display().to_string(),
    );
    mgr.set_mission_permission_mode(crate::router::runtime::MissionPermissionMode::Bypass);
    let mut configured = role("/bin/sh", &[]);
    configured.runtime = "cursor".into();
    configured.env.insert(
        "CURSOR_CONFIG_DIR".into(),
        data.path().join("config").display().to_string(),
    );
    let mut run = mission();
    run.cwd = Some(data.path().display().to_string());
    let slot_id = insert_crew_role(&pool, &run.id, &configured.id);
    let mut slot = slot_for(&configured);
    slot.id = slot_id;
    crate::repo::role::update(
        &pool.get().unwrap(),
        &crate::repo::role::RoleRow::from(&configured),
    )
    .unwrap();
    let spawned = mgr
        .spawn(
            &run,
            &configured,
            &slot,
            data.path(),
            data.path().join("events.ndjson"),
            pool.clone(),
            capture(),
            None,
        )
        .unwrap();
    assert!(fake
        .last_spawn_spec()
        .unwrap()
        .args
        .iter()
        .any(|arg| arg == "--force"));
    let key = crate::repo::session::get_row(&pool.get().unwrap(), &spawned.id)
        .unwrap()
        .unwrap()
        .agent_session_key
        .unwrap();
    use md5::{Digest, Md5};
    let cwd = data.path().canonicalize().unwrap();
    let store = data
        .path()
        .join("config/chats")
        .join(format!(
            "{:x}",
            Md5::digest(cwd.to_string_lossy().as_bytes())
        ))
        .join(&key)
        .join("store.db");
    std::fs::create_dir_all(store.parent().unwrap()).unwrap();
    std::fs::write(&store, []).unwrap();
    fake.close_spawn(0);
    wait_for_session_exit(&mgr, &pool, &spawned.id);
    let resumed = mgr
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
    mgr.kill(&resumed.id).unwrap();
    assert!(args.iter().any(|arg| arg == "--resume"));
    for expected in [
        "--force",
        "--sandbox",
        "disabled",
        "--approve-mcps",
        "--trust",
    ] {
        assert!(
            args.iter().any(|arg| arg == expected),
            "missing {expected} after resume: {args:?}"
        );
    }
}
