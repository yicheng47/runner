use super::*;
use crate::model::CodexSpeed;

#[test]
fn direct_spawn_records_effective_inputs_before_machine_settings() {
    let pool = pool_with_schema();
    let mut configured = role(
        "codex-configured",
        &["--search", "--sandbox", "workspace-write"],
    );
    configured.runtime = "codex".into();
    configured
        .env
        .insert("RUNNER_878_MARKER".into(), "before".into());
    configured.system_prompt = Some("original persona".into());
    configured.codex_speed = Some(CodexSpeed::Fast);
    insert_role_row(&pool.get().unwrap(), &configured);
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());
    let out = mgr
        .spawn_direct_with_speed(
            &configured,
            None,
            Some("selected-model"),
            Some("xhigh"),
            Some(CodexSpeed::Standard),
            None,
            Some(fixture_tmp_dir().to_str().unwrap()),
            None,
            None,
            fixture_tmp_dir(),
            pool.clone(),
            capture(),
            None,
        )
        .unwrap();
    let row = crate::repo::session::get_row(&pool.get().unwrap(), &out.id)
        .unwrap()
        .unwrap();
    let definition = row.launch_definition.unwrap();
    assert_eq!(row.agent_runtime.as_deref(), Some("codex"));
    assert_eq!(definition.command, "codex-configured");
    assert_eq!(
        definition.args,
        ["--search", "--sandbox", "workspace-write"]
    );
    assert_eq!(
        definition.env.get("RUNNER_878_MARKER").map(String::as_str),
        Some("before")
    );
    assert_eq!(definition.model.as_deref(), Some("selected-model"));
    assert_eq!(definition.effort.as_deref(), Some("xhigh"));
    assert_eq!(definition.codex_speed, Some(CodexSpeed::Standard));
    assert_eq!(definition.system_prompt, configured.system_prompt);
    assert_eq!(
        row.agent_command,
        Some(fake.last_spawn_spec().unwrap().command)
    );
    mgr.kill(&out.id).unwrap();
}

#[test]
fn direct_resume_keeps_every_recorded_input_and_agent_defaults_after_role_edit() {
    for runtime in ["codex", "pi", "antigravity"] {
        let pool = pool_with_schema();
        let home = tempfile::tempdir().unwrap();
        let mut configured = role("original-command", &["original-arg"]);
        configured.runtime = runtime.into();
        configured
            .env
            .insert("RUNNER_878_MARKER".into(), "before".into());
        configured.system_prompt = Some("ORIGINAL_PERSONA".into());
        insert_role_row(&pool.get().unwrap(), &configured);
        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, fake.clone());
        let out = mgr
            .spawn_direct(
                &configured,
                None,
                None,
                None,
                None,
                Some(home.path().to_str().unwrap()),
                None,
                None,
                home.path(),
                pool.clone(),
                capture(),
                None,
            )
            .unwrap();
        let original = crate::repo::session::get_row(&pool.get().unwrap(), &out.id)
            .unwrap()
            .unwrap()
            .launch_definition
            .unwrap();
        mgr.kill(&out.id).unwrap();
        configured.runtime = "claude-code".into();
        configured.command = "edited-command".into();
        configured.args = vec!["edited-arg".into()];
        configured
            .env
            .insert("RUNNER_878_MARKER".into(), "after".into());
        configured.model = Some("edited-model".into());
        configured.effort = Some("high".into());
        configured.codex_speed = Some(CodexSpeed::Fast);
        configured.system_prompt = Some("EDITED_PERSONA".into());
        update_role_row(&pool.get().unwrap(), &configured);
        // Keyless Codex and agy use the fresh-conversation fallback; pi composes a system prompt on every launch.
        crate::runtimes::with_conversation_home(home.path(), || {
            mgr.resume(&out.id, None, None, home.path(), pool.clone(), capture())
        })
        .unwrap();
        let spec = fake.last_spawn_spec().unwrap();
        assert_eq!(spec.command, "original-command", "{runtime}");
        assert_eq!(
            spec.env.get("RUNNER_878_MARKER").map(String::as_str),
            Some("before")
        );
        assert!(spec.args.contains(&"original-arg".into()));
        assert!(!spec.args.iter().any(|arg| arg.contains("edited")));
        if runtime == "antigravity" {
            assert!(spec.args.iter().any(|arg| arg.contains("ORIGINAL_PERSONA")));
        }
        if runtime == "pi" {
            let prompt_path = spec
                .args
                .windows(2)
                .find(|args| args[0] == "--append-system-prompt")
                .unwrap();
            let body = std::fs::read_to_string(&prompt_path[1]).unwrap();
            assert!(body.contains("ORIGINAL_PERSONA"));
            assert!(!body.contains("EDITED_PERSONA"));
        }
        let row = crate::repo::session::get_row(&pool.get().unwrap(), &out.id)
            .unwrap()
            .unwrap();
        assert_eq!(row.launch_definition.as_ref(), Some(&original));
        assert_eq!(row.agent_runtime.as_deref(), Some(runtime));
        assert_eq!(row.agent_command.as_deref(), Some("original-command"));
        mgr.kill(&out.id).unwrap();
    }
}

#[test]
fn mission_resume_keeps_definition_and_restart_rebuilds_from_role_and_roster() {
    for lead in [false, true] {
        let (pool, home, id) = slot_respawn_fixture("codex", lead);
        let conn = pool.get().unwrap();
        conn.execute("UPDATE sessions SET agent_session_key = NULL", [])
            .unwrap();
        let mut row = crate::repo::session::get_row(&conn, &id).unwrap().unwrap();
        let mut original = row.launch_definition.take().unwrap();
        original.command = "original-command".into();
        original.args = vec!["original-arg".into()];
        original
            .env
            .insert("RUNNER_878_MARKER".into(), "before".into());
        original.codex_speed = Some(CodexSpeed::Standard);
        crate::repo::session::set_launch(&conn, &id, "codex", &original).unwrap();
        conn.execute("UPDATE slots SET model_override = 'slot-model', effort_override = 'xhigh', runtime_options_json = '{\"codex\":{\"speed\":\"standard\"}}' WHERE id = ?1", [row.slot_id.as_deref().unwrap()]).unwrap();
        let role_id = row.role_id.as_deref().unwrap();
        let mut edited = crate::ops::role::get(&conn, role_id).unwrap();
        edited.command = "edited-command".into();
        edited.args = vec!["edited-arg".into()];
        edited.model = Some("edited-model".into());
        edited.effort = Some("high".into());
        edited.codex_speed = Some(CodexSpeed::Fast);
        edited
            .env
            .insert("RUNNER_878_MARKER".into(), "after".into());
        edited.system_prompt = Some("EDITED_PERSONA".into());
        update_role_row(&conn, &edited);
        drop(conn);
        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, fake.clone());
        mgr.resume(&id, None, None, home.path(), pool.clone(), capture())
            .unwrap();
        let spec = fake.last_spawn_spec().unwrap();
        assert_eq!(spec.command, "original-command");
        assert_eq!(
            spec.env.get("RUNNER_878_MARKER").map(String::as_str),
            Some("before")
        );
        assert!(spec.args.last().unwrap().contains("SLOT_BRIEF"));
        assert!(!spec.args.last().unwrap().contains("EDITED_PERSONA"));
        assert!(!spec.args.iter().any(|arg| arg.contains("edited-model")));
        mgr.restart(&id, None, None, home.path(), pool.clone(), capture())
            .unwrap();
        let spec = fake.last_spawn_spec().unwrap();
        assert_eq!(spec.command, "edited-command");
        assert_eq!(
            spec.env.get("RUNNER_878_MARKER").map(String::as_str),
            Some("after")
        );
        assert!(spec.args.last().unwrap().contains("EDITED_PERSONA"));
        let restarted = crate::repo::session::get_row(&pool.get().unwrap(), &id)
            .unwrap()
            .unwrap();
        edited.model = Some("slot-model".into());
        edited.effort = Some("xhigh".into());
        edited.codex_speed = Some(CodexSpeed::Standard);
        assert_eq!(
            restarted.launch_definition,
            Some(crate::session::launch_definition::from_effective(&edited))
        );
        edited.runtime = "pi".into();
        edited.codex_speed = None;
        update_role_row(&pool.get().unwrap(), &edited);
        mgr.restart(&id, None, None, home.path(), pool.clone(), capture())
            .unwrap();
        let restarted = crate::repo::session::get_row(&pool.get().unwrap(), &id)
            .unwrap()
            .unwrap();
        assert_eq!(restarted.agent_runtime.as_deref(), Some("pi"));
        assert_eq!(restarted.launch_definition.unwrap().codex_speed, None);
        mgr.kill(&id).unwrap();
    }
}

#[test]
fn failed_runtime_changing_restart_keeps_definition_and_conversation_key_consistent() {
    let (pool, home, id) = slot_respawn_fixture("codex", false);
    let conn = pool.get().unwrap();
    let original = crate::repo::session::get_row(&conn, &id).unwrap().unwrap();
    conn.execute("UPDATE sessions SET cwd = '/fixture' WHERE id = ?1", [&id])
        .unwrap();
    let role_id = original.role_id.as_deref().unwrap();
    let mut edited = crate::ops::role::get(&conn, role_id).unwrap();
    edited.runtime = "claude-code".into();
    edited.command = "edited-command".into();
    edited.args = vec!["--debug".into()];
    edited.system_prompt = Some("EDITED_PERSONA".into());
    update_role_row(&conn, &edited);
    drop(conn);
    let expected = crate::session::launch_definition::from_effective(&edited);
    let fake = fake_runtime();
    *fake.spawn_failure_handle.lock().unwrap() = Some("slot".into());
    let mgr = mgr_with_fake(None, fake.clone());
    let error = mgr
        .restart(&id, None, None, home.path(), pool.clone(), capture())
        .unwrap_err();
    assert!(error.to_string().contains("injected spawn failure"));
    assert_eq!(fake.spawn_count(), 0);
    let failed = crate::repo::session::get_row(&pool.get().unwrap(), &id)
        .unwrap()
        .unwrap();
    assert_eq!(failed.status, crate::model::SessionStatus::Stopped);
    assert_eq!(failed.agent_runtime.as_deref(), Some("claude-code"));
    assert_eq!(failed.launch_definition.as_ref(), Some(&expected));
    assert_ne!(failed.agent_session_key, original.agent_session_key);
    let key = failed.agent_session_key.as_deref().unwrap();
    uuid::Uuid::parse_str(key).unwrap();

    let transcripts = home.path().join(".claude/projects/-fixture");
    std::fs::create_dir_all(&transcripts).unwrap();
    std::fs::write(transcripts.join(format!("{key}.jsonl")), "{}\n").unwrap();
    *fake.spawn_failure_handle.lock().unwrap() = None;
    edited.runtime = "pi".into();
    edited.command = "later-command".into();
    edited.args = vec!["later-arg".into()];
    update_role_row(&pool.get().unwrap(), &edited);
    crate::runtimes::with_conversation_home(home.path(), || {
        mgr.resume(&id, None, None, home.path(), pool.clone(), capture())
    })
    .unwrap();
    let spec = fake.last_spawn_spec().unwrap();
    assert_eq!(spec.command, expected.command);
    assert!(spec.args.contains(&"--debug".into()));
    assert!(has_arg_pair(&spec.args, "--resume", key));
    assert!(!spec
        .args
        .contains(original.agent_session_key.as_ref().unwrap()));
    assert!(!spec.args.contains(&"later-arg".into()));
    let resumed = crate::repo::session::get_row(&pool.get().unwrap(), &id)
        .unwrap()
        .unwrap();
    assert_eq!(resumed.agent_runtime, failed.agent_runtime);
    assert_eq!(resumed.agent_session_key, failed.agent_session_key);
    assert_eq!(resumed.launch_definition, failed.launch_definition);
    mgr.kill(&id).unwrap();
}

#[test]
fn resume_without_a_definition_is_refused_by_name() {
    let pool = pool_with_schema();
    let mut row = crate::test_support::test_session_row(
        "missing-definition",
        crate::model::SessionStatus::Stopped,
    );
    row.agent_runtime = Some("codex".into());
    row.agent_command = Some("codex".into());
    crate::repo::session::insert(&pool.get().unwrap(), &row).unwrap();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());
    let error = mgr
        .resume(&row.id, None, None, fixture_tmp_dir(), pool, capture())
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "session missing-definition has no launch definition"
    );
    assert!(fake.last_spawn_spec().is_none());
}

#[test]
fn claude_accounts_follow_active_selection_and_resume_and_fork_from_origin() {
    use runner_core::protocol::ClaudeAccount;
    let pool = pool_with_schema();
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().to_str().unwrap();
    let native = ClaudeAccount {
        id: "native".into(),
        name: "Personal".into(),
        folder: temp.path().join("native").to_string_lossy().into_owned(),
        native: true,
    };
    let work = ClaudeAccount {
        id: "work".into(),
        name: "Work".into(),
        folder: temp
            .path()
            .join("work folder")
            .to_string_lossy()
            .into_owned(),
        native: false,
    };
    std::fs::create_dir_all(&native.folder).unwrap();
    std::fs::create_dir_all(&work.folder).unwrap();
    std::fs::write(
        Path::new(&native.folder).join("settings.json"),
        r#"{"env":{"SHARED_MAIN":"enabled"},"enabledPlugins":{"fixture@main":true}}"#,
    )
    .unwrap();
    std::fs::write(
        Path::new(&native.folder)
            .parent()
            .unwrap()
            .join(".claude.json"),
        r#"{"identity":"not-in-definition","mcpServers":{"fixture":{"command":"fixture-tool"}}}"#,
    )
    .unwrap();
    let plugin = Path::new(&native.folder).join("plugins/cache/fixture plugin");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(
        Path::new(&native.folder).join("plugins/installed_plugins.json"),
        serde_json::json!({"version":2,"plugins":{"fixture@main":[{"scope":"user","installPath":plugin}]}}).to_string(),
    ).unwrap();
    let check_shared = |spec: &SpawnSpec| {
        assert_eq!(
            spec.args.iter().filter(|arg| *arg == "--settings").count(),
            1
        );
        assert!(
            std::fs::read_to_string(Path::new(&work.folder).join("settings.json"))
                .unwrap()
                .contains("SHARED_MAIN")
        );
        assert!(spec.args.iter().any(|arg| arg == "--mcp-config"));
        assert!(spec.args.windows(2).any(|pair| pair[0] == "--plugin-dir"
            && std::fs::canonicalize(&pair[1]).unwrap()
                == std::fs::canonicalize(&plugin).unwrap()));
    };
    let save = |accounts: &[ClaudeAccount], active: &str| {
        crate::db::app_state_set(
            &pool.get().unwrap(),
            "claude_accounts",
            &serde_json::to_string(&crate::claude_accounts::AccountsState {
                accounts: accounts.to_vec(),
                main_id: native.id.clone(),
                main_change: None,
                active_id: active.into(),
                sharing: if accounts.len()>1 {serde_json::from_value(serde_json::json!({"work":{"main_folder":native.folder,"folder":work.folder,"links":[]}})).unwrap()} else {Default::default()},
            })
            .unwrap(),
        )
        .unwrap();
    };
    save(&[native.clone(), work.clone()], &native.id);
    let mut configured = role("claude", &[]);
    configured.runtime = "claude-code".into();
    insert_role_row(&pool.get().unwrap(), &configured);
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());
    let source = mgr
        .spawn_direct(
            &configured,
            None,
            None,
            None,
            None,
            Some(cwd),
            None,
            None,
            temp.path(),
            pool.clone(),
            capture(),
            None,
        )
        .unwrap();
    let spec = fake.last_spawn_spec().unwrap();
    assert!(!spec.env.contains_key("CLAUDE_CONFIG_DIR"));
    let source_row = crate::repo::session::get_row(&pool.get().unwrap(), &source.id)
        .unwrap()
        .unwrap();
    let key = source_row.agent_session_key.clone().unwrap();
    let transcript = crate::claude_accounts::transcript(&native.folder, Some(cwd), &key).unwrap();
    std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    std::fs::write(&transcript, "{\"permissionMode\":\"acceptEdits\"}\n").unwrap();
    mgr.kill(&source.id).unwrap();
    save(&[native.clone(), work.clone()], &work.id);
    mgr.resume_on_launch(&source.id, None, None, temp.path(), pool.clone(), capture())
        .unwrap();
    let spec = fake.last_spawn_spec().unwrap();
    assert_eq!(spec.env.get("CLAUDE_CONFIG_DIR"), Some(&work.folder));
    check_shared(&spec);
    assert!(spec
        .args
        .windows(2)
        .any(|pair| pair[0] == "--resume" && pair[1] == transcript.to_string_lossy()));
    assert!(spec
        .args
        .windows(2)
        .any(|pair| pair == ["--permission-mode", "acceptEdits"]));
    assert_eq!(
        spec.args
            .iter()
            .filter(|arg| *arg == "--permission-mode")
            .count(),
        1
    );
    let resumed = crate::repo::session::get_row(&pool.get().unwrap(), &source.id)
        .unwrap()
        .unwrap();
    assert_eq!(resumed.agent_session_key.as_ref(), Some(&key));
    let account = resumed
        .launch_definition
        .as_ref()
        .unwrap()
        .claude_account
        .as_ref()
        .unwrap();
    assert_eq!(account.folder, work.folder);
    assert_eq!(account.origin_folder, native.folder);
    assert!(!resumed
        .launch_definition
        .as_ref()
        .unwrap()
        .env
        .contains_key("CLAUDE_CONFIG_DIR"));
    assert!(!resumed
        .launch_definition
        .as_ref()
        .unwrap()
        .args
        .iter()
        .any(|arg| arg == "--plugin-dir"));
    mgr.kill(&source.id).unwrap();
    let work_config = Path::new(&work.folder).join(".claude.json");
    let before: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&work_config).unwrap_or_else(|_| "{}".into()),
    )
    .unwrap();
    assert!(before.get("projects").is_none());
    let fork = crate::runtimes::with_conversation_home(&temp.path().join("isolated-home"), || {
        mgr.spawn_fork(
            &source.id,
            None,
            None,
            None,
            temp.path(),
            pool.clone(),
            capture(),
        )
    })
    .unwrap();
    let mut trusted: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&work_config).unwrap()).unwrap();
    let root = std::fs::canonicalize(cwd).unwrap();
    #[cfg(windows)]
    let root = crate::runtimes::ordinary_windows_path(&root);
    let root = root.to_string_lossy().replace('\\', "/");
    assert_eq!(trusted["projects"][&root]["hasTrustDialogAccepted"], true);
    trusted.as_object_mut().unwrap().remove("projects");
    assert_eq!(trusted, before);
    let fork_row = crate::repo::session::get_row(&pool.get().unwrap(), &fork.id)
        .unwrap()
        .unwrap();
    assert_ne!(fork_row.agent_session_key, Some(key.clone()));
    assert_eq!(
        fork_row
            .launch_definition
            .as_ref()
            .unwrap()
            .claude_account
            .as_ref()
            .unwrap()
            .origin_folder,
        work.folder
    );
    assert!(fake
        .last_spawn_spec()
        .unwrap()
        .args
        .iter()
        .any(|arg| arg == "--fork-session"));
    check_shared(&fake.last_spawn_spec().unwrap());
    assert!(!fork_row
        .launch_definition
        .as_ref()
        .unwrap()
        .args
        .join(" ")
        .contains("SHARED_MAIN"));
    assert!(!fork_row
        .launch_definition
        .as_ref()
        .unwrap()
        .args
        .iter()
        .any(|arg| arg == "--plugin-dir"));
    mgr.kill(&fork.id).unwrap();
    // Forgetting Work leaves existing histories usable on native Personal.
    save(std::slice::from_ref(&native), &native.id);
    mgr.resume(&source.id, None, None, temp.path(), pool.clone(), capture())
        .unwrap();
    assert!(fake
        .last_spawn_spec()
        .unwrap()
        .args
        .windows(2)
        .any(|pair| pair[0] == "--resume" && pair[1] == key));
    mgr.kill(&source.id).unwrap();
}

#[test]
fn claude_rekey_after_account_switch_resumes_and_forks_the_new_conversation() {
    use crate::session::state::{KeyOrigin, PersistKey};
    use runner_core::protocol::ClaudeAccount;

    for transcript_before_report in [false, true] {
        let pool = pool_with_schema();
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().to_str().unwrap();
        let native = ClaudeAccount {
            id: "native".into(),
            name: "Personal".into(),
            folder: temp.path().join("native").to_string_lossy().into_owned(),
            native: true,
        };
        let work = ClaudeAccount {
            id: "work".into(),
            name: "Work".into(),
            folder: temp.path().join("work").to_string_lossy().into_owned(),
            native: false,
        };
        let select = |active: &str| {
            crate::db::app_state_set(
                &pool.get().unwrap(),
                "claude_accounts",
                &serde_json::json!({
                    "accounts": [native, work],
                    "main_id": native.id,
                    "active_id": active,
                })
                .to_string(),
            )
            .unwrap();
        };
        select(&native.id);
        let mut configured = role("claude", &[]);
        configured.runtime = "claude-code".into();
        insert_role_row(&pool.get().unwrap(), &configured);
        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, fake.clone());
        let source = mgr
            .spawn_direct(
                &configured,
                None,
                None,
                None,
                None,
                Some(cwd),
                None,
                None,
                temp.path(),
                pool.clone(),
                capture(),
                None,
            )
            .unwrap();
        let row = || {
            crate::repo::session::get_row(&pool.get().unwrap(), &source.id)
                .unwrap()
                .unwrap()
        };
        let old_key = row().agent_session_key.unwrap();
        let original =
            crate::claude_accounts::transcript(&native.folder, Some(cwd), &old_key).unwrap();
        std::fs::create_dir_all(original.parent().unwrap()).unwrap();
        std::fs::write(&original, "{\"permissionMode\":\"acceptEdits\"}\n").unwrap();
        mgr.kill(&source.id).unwrap();
        select(&work.id);
        mgr.resume(&source.id, None, None, temp.path(), pool.clone(), capture())
            .unwrap();
        let report = |key: &str| {
            mgr.persist_reported_key(
                &source.id,
                PersistKey {
                    key: Some(key.into()),
                    generation: row().started_at.unwrap().to_rfc3339(),
                    origin: KeyOrigin::Rekeyed,
                },
                &pool,
            )
            .unwrap();
        };
        report(&old_key);
        assert_eq!(
            row()
                .launch_definition
                .unwrap()
                .claude_account
                .unwrap()
                .origin_folder,
            native.folder
        );

        // Selection changes do not change the account of this running process.
        select(&native.id);
        let new_key = uuid::Uuid::new_v4().to_string();
        let new_transcript =
            crate::claude_accounts::transcript(&work.folder, Some(cwd), &new_key).unwrap();
        std::fs::create_dir_all(new_transcript.parent().unwrap()).unwrap();
        let contents = "{\"permissionMode\":\"default\",\"marker\":\"NEW_WORK\"}\n";
        if transcript_before_report {
            std::fs::write(&new_transcript, contents).unwrap();
        }
        report(&new_key);
        assert_eq!(
            row()
                .launch_definition
                .unwrap()
                .claude_account
                .unwrap()
                .origin_folder,
            work.folder
        );
        std::fs::write(&new_transcript, contents).unwrap();
        mgr.kill(&source.id).unwrap();

        // A new manager represents a daemon reload of the persisted origin.
        let mgr = mgr_with_fake(None, fake.clone());
        mgr.resume_on_launch(&source.id, None, None, temp.path(), pool.clone(), capture())
            .unwrap();
        let spec = fake.last_spawn_spec().unwrap();
        assert!(!spec.env.contains_key("CLAUDE_CONFIG_DIR"));
        assert!(spec
            .args
            .windows(2)
            .any(|pair| pair[0] == "--resume" && pair[1] == new_transcript.to_string_lossy()));
        assert!(spec
            .args
            .windows(2)
            .any(|pair| pair == ["--permission-mode", "default"]));
        assert_eq!(row().agent_session_key.as_deref(), Some(new_key.as_str()));
        mgr.kill(&source.id).unwrap();

        select(&work.id);
        mgr.resume(&source.id, None, None, temp.path(), pool.clone(), capture())
            .unwrap();
        assert!(fake
            .last_spawn_spec()
            .unwrap()
            .args
            .windows(2)
            .any(|pair| pair[0] == "--resume" && pair[1] == new_key));
        mgr.kill(&source.id).unwrap();
        select(&native.id);
        let fork = mgr
            .spawn_fork(
                &source.id,
                None,
                None,
                None,
                temp.path(),
                pool.clone(),
                capture(),
            )
            .unwrap();
        let spec = fake.last_spawn_spec().unwrap();
        assert!(spec.args.iter().any(|arg| arg == "--fork-session"));
        assert!(spec
            .args
            .windows(2)
            .any(|pair| pair[0] == "--resume" && pair[1] == new_transcript.to_string_lossy()));
        mgr.kill(&fork.id).unwrap();
        assert_eq!(
            std::fs::read_to_string(original).unwrap(),
            "{\"permissionMode\":\"acceptEdits\"}\n"
        );
        assert_eq!(std::fs::read_to_string(new_transcript).unwrap(), contents);
    }
}

#[test]
fn claude_legacy_shell_pin_keeps_its_origin_when_a_second_account_is_added() {
    let pool = pool_with_schema();
    let temp = tempfile::tempdir().unwrap();
    let folder = temp
        .path()
        .join("shell pinned account")
        .to_string_lossy()
        .into_owned();
    let mut configured = role("claude", &[]);
    configured.runtime = "claude-code".into();
    insert_role_row(&pool.get().unwrap(), &configured);
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, fake.clone());
    mgr.shell_env
        .write()
        .unwrap()
        .vars
        .insert("CLAUDE_CONFIG_DIR".into(), folder.clone());
    let source = mgr
        .spawn_direct(
            &configured,
            None,
            None,
            None,
            None,
            temp.path().to_str(),
            None,
            None,
            temp.path(),
            pool.clone(),
            capture(),
            None,
        )
        .unwrap();
    let row = crate::repo::session::get_row(&pool.get().unwrap(), &source.id)
        .unwrap()
        .unwrap();
    let definition = row.launch_definition.unwrap();
    assert!(definition.claude_account.is_none());
    assert!(!definition.env.contains_key("CLAUDE_CONFIG_DIR"));
    assert_eq!(
        fake.last_spawn_spec().unwrap().env.get("CLAUDE_CONFIG_DIR"),
        Some(&folder)
    );
    let key = row.agent_session_key.unwrap();
    let transcript =
        crate::claude_accounts::transcript(&folder, temp.path().to_str(), &key).unwrap();
    std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    std::fs::write(&transcript, "{\"permissionMode\":\"acceptEdits\"}\n").unwrap();
    mgr.kill(&source.id).unwrap();
    let core = crate::test_support::test_core_in(temp.path().join("core"));
    crate::claude_accounts::action(
        &crate::AppCore {
            db: pool.clone(),
            ..core
        },
        runner_core::protocol::ClaudeAccountAction::Add {
            name: "Work".into(),
            folder: Some(temp.path().join("work").to_string_lossy().into_owned()),
            use_main_settings: false,
        },
    )
    .unwrap();
    let fork = mgr
        .spawn_fork(
            &source.id,
            None,
            None,
            None,
            temp.path(),
            pool.clone(),
            capture(),
        )
        .unwrap();
    let spec = fake.last_spawn_spec().unwrap();
    assert_eq!(spec.env.get("CLAUDE_CONFIG_DIR"), Some(&folder));
    assert!(spec
        .args
        .windows(2)
        .any(|pair| pair[0] == "--resume" && pair[1] == key));
    assert!(!spec.args.iter().any(|arg| arg.ends_with(".jsonl")));
    let row = crate::repo::session::get_row(&pool.get().unwrap(), &fork.id)
        .unwrap()
        .unwrap();
    let account = row.launch_definition.unwrap().claude_account.unwrap();
    assert_eq!(account.origin_folder, folder);
    assert_eq!(account.permission_mode, "acceptEdits");
    assert!(account.pinned);
    assert_ne!(row.agent_session_key.as_ref(), Some(&key));
    mgr.kill(&fork.id).unwrap();
}

#[test]
fn claude_single_account_and_explicit_pins_keep_their_launch_inputs() {
    let pool = pool_with_schema();
    let mut configured = role("claude", &["--verbose"]);
    configured.runtime = "claude-code".into();
    let before = configured.clone();
    assert_eq!(
        crate::claude_accounts::launch_account(
            &pool,
            &mut configured,
            &BTreeMap::new(),
            None,
            true
        )
        .unwrap(),
        None
    );
    assert_eq!(configured.args, before.args);
    assert_eq!(configured.env, before.env);
    let home = runner_core::app_paths::home_dir()
        .unwrap()
        .join(".claude")
        .to_string_lossy()
        .into_owned();
    let accounts = serde_json::json!({"accounts":[{"id":"native","name":"Personal","folder":home,"native":true},{"id":"work","name":"Work","folder":"/tmp/work folder","native":false}],"main_id":"work","active_id":"work"});
    crate::db::app_state_set(
        &pool.get().unwrap(),
        "claude_accounts",
        &accounts.to_string(),
    )
    .unwrap();
    configured
        .env
        .insert("CLAUDE_CONFIG_DIR".into(), home.clone());
    let account = crate::claude_accounts::launch_account(
        &pool,
        &mut configured,
        &BTreeMap::new(),
        None,
        true,
    )
    .unwrap()
    .unwrap();
    assert!(!account.native);
    assert_eq!(account.account_id, None);
    assert_eq!(configured.env["CLAUDE_CONFIG_DIR"], home);
    configured.env.clear();
    let shell = BTreeMap::from([("CLAUDE_CONFIG_DIR".into(), "/tmp/explicit".into())]);
    let account =
        crate::claude_accounts::launch_account(&pool, &mut configured, &shell, None, true)
            .unwrap()
            .unwrap();
    assert_eq!(account.folder, "/tmp/explicit");
    assert_eq!(account.account_id, None);
    let mut accounts = accounts;
    accounts["active_id"] = serde_json::json!("native");
    crate::db::app_state_set(
        &pool.get().unwrap(),
        "claude_accounts",
        &accounts.to_string(),
    )
    .unwrap();
    let account = crate::claude_accounts::launch_account(
        &pool,
        &mut configured,
        &BTreeMap::new(),
        None,
        true,
    )
    .unwrap()
    .unwrap();
    assert!(account.native);
    assert!(!configured.env.contains_key("CLAUDE_CONFIG_DIR"));
}

#[test]
fn claude_trust_uses_the_effective_launch_account_and_environment() {
    for source in ["native", "active-account", "shell", "role"] {
        let data = tempfile::tempdir().unwrap();
        let home = data.path().join("home");
        let cwd = data.path().join("project");
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir(&cwd).unwrap();
        let active = home.join("active-account");
        let shell = home.join("shell-account");
        let role_dir = home.join("role-account");
        let pool = pool_with_schema();
        if source != "native" {
            let state = serde_json::json!({
                "accounts": [
                    {"id":"native", "name":"Personal", "folder":home.join(".claude"), "native":true},
                    {"id":"work", "name":"Work", "folder":active, "native":false}
                ],
                "main_id":"work", "active_id":"work"
            });
            crate::db::app_state_set(&pool.get().unwrap(), "claude_accounts", &state.to_string())
                .unwrap();
        }
        let mut configured = role("claude", &[]);
        configured.runtime = "claude-code".into();
        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, fake.clone());
        if matches!(source, "shell" | "role") {
            mgr.shell_env
                .write()
                .unwrap()
                .vars
                .insert("CLAUDE_CONFIG_DIR".into(), shell.display().to_string());
        }
        if source == "role" {
            configured
                .env
                .insert("CLAUDE_CONFIG_DIR".into(), role_dir.display().to_string());
        }
        insert_role_row(&pool.get().unwrap(), &configured);
        let out = crate::runtimes::with_conversation_home(&home, || {
            mgr.spawn_direct(
                &configured,
                None,
                None,
                None,
                None,
                cwd.to_str(),
                None,
                None,
                data.path(),
                pool,
                capture(),
                None,
            )
        })
        .unwrap();
        let expected = match source {
            "active-account" => active.join(".claude.json"),
            "shell" => shell.join(".claude.json"),
            "role" => role_dir.join(".claude.json"),
            _ => home.join(".claude.json"),
        };
        let config: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&expected).unwrap()).unwrap();
        let root = std::fs::canonicalize(&cwd).unwrap();
        #[cfg(windows)]
        let root = crate::runtimes::ordinary_windows_path(&root);
        let root = root.to_string_lossy().replace('\\', "/");
        assert_eq!(
            config["projects"][&root]["hasTrustDialogAccepted"], true,
            "{source}"
        );
        for path in [
            home.join(".claude.json"),
            active.join(".claude.json"),
            shell.join(".claude.json"),
            role_dir.join(".claude.json"),
        ] {
            assert_eq!(
                path.exists(),
                path == expected,
                "{source}: {}",
                path.display()
            );
        }
        mgr.kill(&out.id).unwrap();
    }
}
