use super::*;

// Pre-#88 `codex_direct_chat_injects_persona_without_preamble`
// and `claude_code_direct_chat_injects_persona_without_preamble`
// asserted the off-bus invariant from #51 over the post-spawn
// paste path. Plan 0007 moved first-turn delivery to spawn-time
// positional argv; the same invariant is now exercised by
// `direct_chat_persona_lands_as_trailing_positional_argv_without_worker_preamble`
// below, and `compose_direct_first_turn` is unit-tested in
// `router::prompt`.

#[cfg(unix)]
#[test]
fn direct_chat_persona_lands_as_trailing_positional_argv_without_worker_preamble() {
    // Plan 0007: when `spawn_direct` receives a non-empty
    // `first_turn`, the body must (a) land as the trailing
    // positional argv on the SpawnSpec, (b) suppress the
    // post-spawn paste fallback so the agent doesn't receive
    // the persona twice, and (c) preserve the off-bus
    // invariant from #51 — direct chats must NOT carry the
    // worker coordination preamble (the bundled `role` CLI
    // isn't on PATH for direct chats; the preamble's verbs
    // would mislead the agent).
    let pool = pool_with_schema();
    let role_id = ulid::Ulid::new().to_string();
    let mut role = role("/bin/sh", &["-c", "cat"]);
    role.id = role_id;
    role.handle = "cc-argv".into();
    role.runtime = "claude-code".into();
    role.system_prompt = Some("DIRECT_PERSONA".into());
    insert_role_row(&pool.get().unwrap(), &role);

    // Compose via the same helper `session_start_direct` uses.
    let body = crate::router::prompt::compose_direct_first_turn(role.system_prompt.as_deref())
        .expect("non-empty persona");
    assert!(
        !body.contains("of a team coordinated by the bundled"),
        "compose_direct_first_turn must NOT include the worker preamble (off-bus invariant)",
    );

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .spawn_direct(
            &role,
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
            Some(body.clone()),
        )
        .unwrap();

    let spec = fake.last_spawn_spec().expect("spawn was called");
    let trailing = spec.args.last().map(String::as_str).unwrap_or("");
    assert!(
        trailing.contains("DIRECT_PERSONA"),
        "first_turn body must land as the trailing positional argv; got args = {:?}",
        spec.args
    );
    assert!(
        !trailing.contains("of a team coordinated by the bundled"),
        "direct chat must NOT ship the worker coordination preamble in argv: {trailing:?}",
    );
    assert!(
        fake.bytes_writes().is_empty(),
        "argv delivery must suppress the post-spawn byte injection fallback; got writes = {:?}",
        fake.bytes_writes()
    );
    assert!(
        mgr.take_completion_armed(std::slice::from_ref(&spawned.id)),
        "argv first-turn delivery must arm the initial busy episode",
    );
    assert!(
        !mgr.take_completion_armed(std::slice::from_ref(&spawned.id)),
        "taking the argv completion arm must consume it",
    );

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn mission_spawn_worker_preamble_lands_as_trailing_positional_argv_with_brief() {
    // Regression guard for #45 + #88 combined: a non-lead worker
    // must still receive the WORKER_COORDINATION_PREAMBLE plus
    // its brief as the first user turn, but now via the
    // spawn-time positional argv path rather than post-spawn
    // paste. Argv delivery must also suppress the paste
    // fallback so the worker doesn't get double-delivered.
    use crate::router::prompt::compose_member_first_turn;

    let pool = pool_with_schema();
    let mission = mission();
    let mut role = role("/bin/sh", &["-c", "cat"]);
    role.runtime = "claude-code".into();
    role.handle = "worker-argv".into();
    role.system_prompt = Some("WORKER_BRIEF".into());

    let slot_id = insert_crew_role(&pool, &mission.id, &role.id);
    {
        let conn = pool.get().unwrap();
        conn.execute("UPDATE slots SET lead = 0 WHERE id = ?1", params![slot_id])
            .unwrap();
        update_role_row(&conn, &role);
    }
    let fresh_mission_id: String = {
        let conn = pool.get().unwrap();
        conn.query_row("SELECT id FROM missions LIMIT 1", [], |r| r.get(0))
            .unwrap()
    };
    let mission = Mission {
        id: fresh_mission_id,
        ..mission
    };
    let mut slot = slot_for(&role);
    slot.id = slot_id;
    slot.lead = false;

    let body = compose_member_first_turn(role.system_prompt.as_deref(), None, None);
    // Composer ships the on-bus preamble + the brief.
    assert!(body.contains("of a team coordinated by the bundled"));
    assert!(body.contains("WORKER_BRIEF"));

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .spawn(
            &mission,
            &role,
            &slot,
            fixture_tmp_dir(),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            capture(),
            Some(body.clone()),
        )
        .unwrap();

    let spec = fake.last_spawn_spec().expect("spawn was called");
    let trailing = spec.args.last().map(String::as_str).unwrap_or("");
    assert_eq!(
            trailing, body,
            "worker first-turn body must land as the trailing positional argv; got args.last() = {trailing:?}"
        );
    assert!(
        trailing.contains("of a team coordinated by the bundled"),
        "worker argv must ship the coordination preamble (on-bus invariant)"
    );
    assert!(
        trailing.contains("WORKER_BRIEF"),
        "worker argv must ship the brief"
    );
    assert!(
        fake.bytes_writes().is_empty(),
        "argv delivery must suppress the post-spawn byte injection fallback; got = {:?}",
        fake.bytes_writes()
    );

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn codex_mission_spawn_grants_event_log_dir_to_sandbox() {
    // Codex's workspace-write sandbox cannot append to Role's
    // app-data mission log unless we grant the mission directory.
    let pool = pool_with_schema();
    let mission_base = Mission {
        crew_id: Some("c".into()),
        ..mission()
    };
    let mut role = role(
        "codex",
        &[
            "--ask-for-approval",
            "on-request",
            "--sandbox",
            "workspace-write",
        ],
    );
    role.runtime = "codex".into();
    role.handle = "codex-worker".into();
    let slot_id = insert_crew_role(&pool, &mission_base.id, &role.id);
    let fresh_mission_id: String = {
        let conn = pool.get().unwrap();
        conn.query_row("SELECT id FROM missions LIMIT 1", [], |r| r.get(0))
            .unwrap()
    };
    let mission = Mission {
        id: fresh_mission_id,
        ..mission_base
    };
    let mut slot = slot_for(&role);
    slot.id = slot_id;

    let app_data = tempfile::tempdir().unwrap();
    let mission_dir = runner_core::event_log::path::mission_dir(
        app_data.path(),
        mission.crew_id.as_deref(),
        &mission.id,
    );
    let events_log_path = runner_core::event_log::path::events_path(
        app_data.path(),
        mission.crew_id.as_deref(),
        &mission.id,
    );

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let first_turn = "mission first turn".to_string();
    let spawned = mgr
        .spawn(
            &mission,
            &role,
            &slot,
            app_data.path(),
            events_log_path,
            Arc::clone(&pool),
            capture(),
            Some(first_turn.clone()),
        )
        .unwrap();

    let spec = fake.last_spawn_spec().expect("spawn was called");
    let mission_dir_arg = mission_dir.to_string_lossy().to_string();
    let marker = crate::runtimes::codex::capture::prompt_marker(&spawned.id);
    assert!(
        has_arg_pair(&spec.args, "--add-dir", &mission_dir_arg),
        "codex mission spawn must grant mission dir with --add-dir; args = {:?}",
        spec.args,
    );
    assert!(
        spec.args
            .iter()
            .any(|arg| arg.contains(&first_turn) && arg.contains(&marker)),
        "codex mission first turn and capture marker must ride argv; args = {:?}",
        spec.args,
    );
    assert!(
        fake.bytes_writes().is_empty(),
        "argv delivery must not schedule byte injection; got {:?}",
        fake.bytes_writes(),
    );
    assert!(
        fake.keys().is_empty(),
        "argv delivery must not schedule submit key injection; got {:?}",
        fake.keys(),
    );

    mgr.kill(&spawned.id).unwrap();
}

fn mission_spawn_args(role: &Role) -> Vec<String> {
    let pool = pool_with_schema();
    let (mission, slot) = seed_mission_rows(&pool, role);
    let app_data = tempfile::tempdir().unwrap();
    let events_log_path = runner_core::event_log::path::events_path(
        app_data.path(),
        mission.crew_id.as_deref(),
        &mission.id,
    );
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .spawn(
            &mission,
            role,
            &slot,
            app_data.path(),
            events_log_path,
            Arc::clone(&pool),
            capture(),
            Some("first turn".into()),
        )
        .unwrap();
    let args = fake.last_spawn_spec().expect("spawn was called").args;
    mgr.kill(&spawned.id).unwrap();
    args
}

#[test]
fn mission_spawn_converges_claude_row_to_bypass() {
    let mut role = role("/bin/sh", &["--permission-mode", "plan", "--model", "opus"]);
    role.runtime = "claude-code".into();
    role.handle = "perm-claude".into();

    let bypass = mission_spawn_args(&role);
    assert_eq!(
        &bypass[..4],
        ["--model", "opus", "--permission-mode", "bypassPermissions"]
    );
    assert!(
        has_arg_pair(&bypass, "--permission-mode", "bypassPermissions"),
        "{bypass:?}"
    );
    assert!(
        !has_arg_pair(&bypass, "--permission-mode", "plan"),
        "{bypass:?}"
    );
    assert!(has_arg_pair(&bypass, "--model", "opus"), "{bypass:?}");
}

#[test]
fn mission_spawn_converges_codex_row_to_bypass() {
    let mut role = role(
        "/bin/sh",
        &[
            "--ask-for-approval",
            "on-request",
            "--sandbox",
            "workspace-write",
        ],
    );
    role.runtime = "codex".into();
    role.handle = "perm-codex".into();

    let bypass = mission_spawn_args(&role);
    assert_eq!(
        &bypass[..4],
        [
            "--ask-for-approval",
            "never",
            "--sandbox",
            "danger-full-access"
        ]
    );
    assert!(
        has_arg_pair(&bypass, "--ask-for-approval", "never"),
        "{bypass:?}"
    );
    assert!(
        has_arg_pair(&bypass, "--sandbox", "danger-full-access"),
        "{bypass:?}"
    );
    assert!(
        !bypass.iter().any(|arg| arg == "workspace-write"),
        "{bypass:?}"
    );
    assert_eq!(
        bypass.iter().filter(|arg| *arg == "--sandbox").count(),
        1,
        "{bypass:?}"
    );
}

#[test]
fn mission_spawn_converges_trae_row_to_bypass() {
    let mut role = role("trae-custom", &["--permission-mode", "auto", "--debug"]);
    role.runtime = "trae".into();

    let bypass = mission_spawn_args(&role);
    assert_eq!(
        &bypass[..3],
        ["--debug", "--permission-mode", "bypass_permissions"]
    );
}

#[test]
fn trae_mission_resume_strips_conflicting_permission_mode() {
    let pool = pool_with_schema();
    let mut role = role(
        "trae-custom",
        &["--permission-mode", "bypass_permissions", "--debug"],
    );
    role.runtime = "trae".into();
    role.handle = "trae-resume".into();
    let (mission, slot) = seed_mission_rows(&pool, &role);
    let app_data = tempfile::tempdir().unwrap();
    let events_log_path = runner_core::event_log::path::events_path(
        app_data.path(),
        mission.crew_id.as_deref(),
        &mission.id,
    );
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .spawn(
            &mission,
            &role,
            &slot,
            app_data.path(),
            events_log_path,
            Arc::clone(&pool),
            capture(),
            Some("first turn".into()),
        )
        .unwrap();
    fake.close_spawn(0);
    wait_for_session_exit(&mgr, &pool, &spawned.id);

    let agent_session_key = "019fa1b9-a133-7841-b4dd-730d376ab1d1";
    pool.get()
        .unwrap()
        .execute(
            "UPDATE sessions SET agent_session_key = ?2 WHERE id = ?1",
            params![spawned.id, agent_session_key],
        )
        .unwrap();

    mgr.resume(
        &spawned.id,
        None,
        None,
        app_data.path(),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();
    assert_eq!(
        fake.last_spawn_spec().unwrap().args,
        ["resume", agent_session_key, "--debug"]
    );
    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn mission_spawn_with_shell_runtime_preserves_args() {
    let role = role("/bin/sh", &["-c", "cat"]);
    assert_eq!(mission_spawn_args(&role), role.args);
}

#[test]
fn mission_resume_applies_bypass_permissions() {
    let pool = pool_with_schema();
    let mut role = role("/bin/sh", &["--permission-mode", "plan"]);
    role.runtime = "claude-code".into();
    role.handle = "perm-resume".into();
    let (mission, slot) = seed_mission_rows(&pool, &role);
    let app_data = tempfile::tempdir().unwrap();
    let events_log_path = runner_core::event_log::path::events_path(
        app_data.path(),
        mission.crew_id.as_deref(),
        &mission.id,
    );

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture();
    let spawned = mgr
        .spawn(
            &mission,
            &role,
            &slot,
            app_data.path(),
            events_log_path,
            Arc::clone(&pool),
            Arc::clone(&cap) as Arc<dyn SessionEvents>,
            Some("first turn".into()),
        )
        .unwrap();
    let first = fake.last_spawn_spec().unwrap().args;
    assert!(
        has_arg_pair(&first, "--permission-mode", "bypassPermissions"),
        "{first:?}"
    );

    fake.close_spawn(0);
    wait_for_session_exit(&mgr, &pool, &spawned.id);

    let resumed = mgr
        .resume(
            &spawned.id,
            None,
            None,
            app_data.path(),
            Arc::clone(&pool),
            Arc::clone(&cap) as Arc<dyn SessionEvents>,
        )
        .unwrap();
    assert_eq!(resumed.id, spawned.id);
    let second = fake.last_spawn_spec().unwrap().args;
    assert!(
        has_arg_pair(&second, "--permission-mode", "bypassPermissions"),
        "{second:?}"
    );
    assert!(
        !second.iter().any(|arg| arg == "auto" || arg == "plan"),
        "{second:?}"
    );

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn direct_spawn_strips_permission_flags() {
    let pool = pool_with_schema();
    let mut role = role("/bin/sh", &["--permission-mode", "plan", "--model", "opus"]);
    role.runtime = "claude-code".into();
    role.handle = "perm-direct".into();
    insert_role_row(&pool.get().unwrap(), &role);

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .spawn_direct(
            &role,
            None,
            None,
            None,
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
    let args = fake.last_spawn_spec().unwrap().args;
    assert!(
        !args.iter().any(|arg| arg == "--permission-mode"),
        "{args:?}"
    );
    assert!(
        !args
            .iter()
            .any(|arg| arg == "bypassPermissions" || arg == "auto" || arg == "plan"),
        "{args:?}"
    );
    assert!(has_arg_pair(&args, "--model", "opus"), "{args:?}");
    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn direct_chat_spawn_and_resume_strip_permission_flags_and_preserve_row_args() {
    for (runtime, permission_args, effort_args) in [
        (
            "claude-code",
            vec![
                "--permission-mode",
                "auto",
                "--permission-mode=plan",
                "--dangerously-skip-permissions",
            ],
            vec!["--effort", "high"],
        ),
        (
            "codex",
            vec![
                "--ask-for-approval",
                "never",
                "--sandbox",
                "workspace-write",
                "--ask-for-approval=on-request",
                "--sandbox=read-only",
            ],
            vec!["-c", "model_reasoning_effort=high"],
        ),
        (
            "trae",
            vec![
                "--permission-mode",
                "auto",
                "--permission-mode=bypass_permissions",
            ],
            vec!["-c", "model_reasoning_effort=high"],
        ),
        (
            "copilot",
            vec![
                "--allow-tool=write",
                "--allow-tool",
                "shell",
                "--yolo",
                "--allow-all",
                "--allow-all-tools",
                "--allow-all-paths",
                "--allow-all-urls",
            ],
            vec!["--effort", "high"],
        ),
        (
            "antigravity",
            vec![
                "--mode",
                "plan",
                "-mode=accept-edits",
                "--dangerously-skip-permissions",
                "-dangerously-skip-permissions=true",
            ],
            vec!["--effort", "high"],
        ),
    ] {
        let pool = pool_with_schema();
        let app_data = tempfile::tempdir().unwrap();
        let mut kept = vec!["--debug", "--model", "test-model"];
        kept.extend(effort_args);
        kept.extend(["--user-flag", "custom-value"]);
        let mut args = kept.clone();
        args.splice(1..1, permission_args);
        let mut role = role("agent-custom", &args);
        role.runtime = runtime.into();
        let stored_args = role.args.clone();
        insert_role_row(&pool.get().unwrap(), &role);
        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
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
                Arc::clone(&pool),
                capture(),
                None,
            )
            .unwrap();
        let args = fake.last_spawn_spec().unwrap().args;
        assert_chat_has_no_permission_flags(&args);
        assert_eq!(&args[..kept.len()], kept);

        fake.close_spawn(0);
        wait_for_session_exit(&mgr, &pool, &spawned.id);
        mgr.resume(
            &spawned.id,
            None,
            None,
            app_data.path(),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap();
        let resumed_spec = fake.last_spawn_spec().unwrap();
        assert_eq!(
            resumed_spec.bundled_bin_dir.as_deref(),
            Some(app_data.path().join("bin").as_path())
        );
        let args = resumed_spec.args;
        assert_chat_has_no_permission_flags(&args);
        assert_eq!(&args[..kept.len()], kept);
        let persisted_args = crate::repo::role::get(&pool.get().unwrap(), &role.id)
            .unwrap()
            .unwrap()
            .args;
        assert_eq!(persisted_args, stored_args);
        mgr.kill(&spawned.id).unwrap();
    }
}

#[test]
fn runtime_only_chat_spawn_and_resume_assert_no_permission_posture() {
    for runtime in [
        "claude-code",
        "codex",
        "trae",
        "copilot",
        "pi",
        "antigravity",
    ] {
        let pool = pool_with_schema();
        let app_data = tempfile::tempdir().unwrap();
        let role = runtime_direct_role(
            runtime,
            Some("agent-custom"),
            Some("test-model"),
            Some("high"),
        )
        .unwrap();
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
        assert_chat_has_no_permission_flags(&args);
        assert!(has_arg_pair(&args, "--model", "test-model"));
        let effort = match runtime {
            "claude-code" | "copilot" | "antigravity" => Some(("--effort", "high")),
            "pi" => Some(("--thinking", "high")),
            _ => Some(("-c", "model_reasoning_effort=high")),
        };
        let has_effort = |args: &[String]| match effort {
            Some((flag, value)) => has_arg_pair(args, flag, value),
            None => !args.iter().any(|arg| arg == "--effort"),
        };
        assert!(has_effort(&args), "{args:?}");

        fake.close_spawn(0);
        wait_for_session_exit(&mgr, &pool, &spawned.id);
        mgr.resume(
            &spawned.id,
            None,
            None,
            app_data.path(),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap();
        let args = fake.last_spawn_spec().unwrap().args;
        assert_chat_has_no_permission_flags(&args);
        assert!(has_arg_pair(&args, "--model", "test-model"));
        assert!(has_effort(&args), "{args:?}");
        mgr.kill(&spawned.id).unwrap();
    }
}

#[test]
fn trae_first_turn_gets_capture_prompt_marker() {
    let (first_turn, marker) = SessionManager::capture_prompt_marker(
        Runtime::Trae.key(),
        "session-id",
        Some("first turn".to_string()),
    );
    let marker = marker.expect("trae must use the codex-lineage capture marker");
    assert_eq!(
        marker,
        crate::runtimes::codex::capture::prompt_marker("session-id")
    );
    let expected = format!("first turn\n\n{marker}");
    assert_eq!(first_turn.as_deref(), Some(expected.as_str()));
}

#[test]
fn launch_switches_cover_chat_resume_mission_and_role_permissions() {
    use runner_core::protocol::runtime::RuntimeLaunchOption as OptionKey;
    let root = tempfile::tempdir().unwrap();
    crate::runtimes::pi::pi_status::install_extension(root.path()).unwrap();
    crate::runtimes::copilot::copilot_status::install_plugin(root.path()).unwrap();
    let pool = pool_with_schema();
    let manager = mgr_with_fake(None, fake_runtime());
    for runtime in Runtime::ALL
        .into_iter()
        .filter(|runtime| !runtime.is_shell())
    {
        for resuming in [false, true] {
            for mission in [false, true] {
                let plan = crate::runtimes::adapter(runtime)
                    .resume_plan(resuming.then_some("eec53725-8f5d-428a-8afb-46510ddfb000"));
                let compose = |bypass, catalog, hooks| {
                    crate::db::set_runtime_launch_option(&pool, runtime, OptionKey::Bypass, bypass)
                        .unwrap();
                    crate::db::set_runtime_launch_option(
                        &pool,
                        runtime,
                        OptionKey::RemotePluginCatalog,
                        catalog,
                    )
                    .unwrap();
                    crate::db::set_runtime_launch_option(
                        &pool,
                        runtime,
                        OptionKey::StatusHooks,
                        hooks,
                    )
                    .unwrap();
                    let mut role = role(runtime.command().unwrap(), &[]);
                    role.runtime = runtime.key().into();
                    if mission {
                        role.args = crate::runtimes::adapter(runtime)
                            .permissions()
                            .mission_args();
                    }
                    let mut spec = SpawnSpec {
                        session_id: "launch-options".into(),
                        command: role.command.clone(),
                        args: role.args.clone(),
                        mission,
                        ..Default::default()
                    };
                    manager.apply_runtime_args(
                        &mut spec,
                        &role,
                        &plan,
                        root.path(),
                        None,
                        Some("hello"),
                        mission.then_some(Path::new("/mission")),
                        Some(&pool),
                    );
                    spec
                };
                let baseline = compose(false, true, true);
                let bypass = compose(true, true, true);
                if mission || !matches!(runtime, Runtime::Codex | Runtime::ClaudeCode) {
                    assert_eq!(baseline.args, bypass.args, "{runtime} mission={mission}");
                } else {
                    assert!(crate::runtimes::adapter(runtime)
                        .permissions()
                        .mission_args()
                        .iter()
                        .all(|arg| bypass.args.contains(arg)));
                    assert_ne!(baseline.args, bypass.args);
                    if runtime == Runtime::ClaudeCode {
                        assert!(bypass
                            .args
                            .iter()
                            .any(|arg| arg.contains("\"skipDangerousModePermissionPrompt\":true")));
                    }
                }
                let catalog = compose(false, false, true);
                assert_eq!(
                    catalog
                        .args
                        .iter()
                        .any(|arg| arg == "features.remote_plugin=false"),
                    runtime == Runtime::Codex
                );
                let no_hooks = compose(false, true, false);
                if runtime == Runtime::Antigravity {
                    assert!(!baseline
                        .args
                        .iter()
                        .any(|arg| arg == "--plugin-dir" || arg == "--add-dir"));
                }
                if mission {
                    assert_eq!(baseline.args, no_hooks.args, "mission hooks for {runtime}");
                } else {
                    match runtime {
                        Runtime::Codex => {
                            assert!(no_hooks
                                .args
                                .iter()
                                .any(|arg| arg.starts_with("hooks.SessionStart=")));
                            assert!(no_hooks
                                .args
                                .iter()
                                .any(|arg| arg.starts_with("hooks.UserPromptSubmit=")));
                            assert!(!no_hooks
                                .args
                                .iter()
                                .any(|arg| arg.starts_with("hooks.Stop=")));
                        }
                        Runtime::ClaudeCode => {
                            let value = no_hooks
                                .args
                                .windows(2)
                                .find(|pair| pair[0] == "--settings")
                                .unwrap();
                            let settings: serde_json::Value =
                                serde_json::from_str(&value[1]).unwrap();
                            assert_eq!(
                                settings["hooks"]
                                    .as_object()
                                    .unwrap()
                                    .keys()
                                    .collect::<Vec<_>>(),
                                ["SessionStart"]
                            );
                        }
                        Runtime::Pi | Runtime::OpenCode | Runtime::Copilot => {
                            assert!(
                                no_hooks
                                    .env
                                    .contains_key(runner_core::protocol::hook::GENERATION_ENV),
                                "identity for {runtime}"
                            );
                        }
                        Runtime::Grok => {
                            assert_eq!(
                                no_hooks
                                    .env
                                    .contains_key(runner_core::protocol::hook::GENERATION_ENV),
                                !cfg!(windows)
                            );
                        }
                        Runtime::Antigravity => {
                            assert_eq!(baseline.args, no_hooks.args);
                            assert!(!baseline
                                .env
                                .contains_key(runner_core::protocol::hook::GENERATION_ENV));
                            assert!(!baseline
                                .args
                                .iter()
                                .any(|arg| arg == "--plugin-dir" || arg == "--add-dir"));
                        }
                        _ => assert_eq!(baseline.args, no_hooks.args),
                    }
                }
            }
        }
    }
    for runtime in [Runtime::Codex, Runtime::ClaudeCode] {
        let adapter = crate::runtimes::adapter(runtime);
        for mode in adapter
            .permissions()
            .offered
            .iter()
            .copied()
            .filter(|mode| *mode != router::runtime::PermissionMode::Default)
        {
            let mut role = role(runtime.command().unwrap(), &[]);
            role.runtime = runtime.key().into();
            role.args = adapter.permissions().mode_args(mode);
            let effective = crate::runtimes::apply_launch_options(
                &role,
                false,
                runner_core::protocol::runtime::RuntimeLaunchOptions {
                    bypass: true,
                    ..Default::default()
                },
            );
            assert_eq!(effective.args, adapter.permissions().mission_args());
        }
    }
}

#[test]
fn launch_options_persist_under_one_key_and_publish_in_status() {
    use runner_core::protocol::runtime::RuntimeLaunchOption as OptionKey;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("launch.db");
    {
        let pool = crate::db::open_pool(&path).unwrap();
        assert!(crate::db::runtime_launch_options(&pool).unwrap().is_empty());
        crate::db::set_runtime_launch_option(&pool, Runtime::Codex, OptionKey::Bypass, true)
            .unwrap();
        crate::db::set_runtime_launch_option(
            &pool,
            Runtime::ClaudeCode,
            OptionKey::StatusHooks,
            false,
        )
        .unwrap();
        crate::db::set_runtime_launch_option(
            &pool,
            Runtime::Codex,
            OptionKey::RemotePluginCatalog,
            false,
        )
        .unwrap();
    }
    let pool = crate::db::open_pool(&path).unwrap();
    let options = crate::db::runtime_launch_options(&pool).unwrap();
    assert!(options["codex"].bypass);
    assert!(!options["codex"].remote_plugin_catalog);
    assert!(options["codex"].status_hooks);
    assert!(!options["claude-code"].status_hooks);
    let count: i64 = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM _app_state WHERE key = 'runtime_launch_options'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    let core = crate::test_support::test_core();
    let status = crate::ops::runtime::runtime_set_launch_option(
        &core,
        Runtime::Codex,
        OptionKey::Bypass,
        true,
    )
    .unwrap();
    assert!(
        status
            .runtimes
            .iter()
            .find(|row| row.name == Runtime::Codex)
            .unwrap()
            .launch_options
            .bypass
    );
    assert!(crate::ops::runtime::runtime_set_launch_option(
        &core,
        Runtime::Cursor,
        OptionKey::StatusHooks,
        false
    )
    .is_err());
}

#[test]
fn claude_account_metadata_records_effective_chat_bypass() {
    use runner_core::protocol::runtime::RuntimeLaunchOption;
    let pool = pool_with_schema();
    let root = tempfile::tempdir().unwrap();
    let mut accounts = crate::claude_accounts::load(&pool).unwrap();
    accounts
        .accounts
        .push(runner_core::protocol::ClaudeAccount {
            id: "work".into(),
            name: "Work".into(),
            folder: root.path().join("work").display().to_string(),
            native: false,
        });
    accounts.active_id = "work".into();
    crate::db::app_state_set(
        &pool.get().unwrap(),
        "claude_accounts",
        &serde_json::to_string(&accounts).unwrap(),
    )
    .unwrap();
    crate::db::set_runtime_launch_option(
        &pool,
        Runtime::ClaudeCode,
        RuntimeLaunchOption::Bypass,
        true,
    )
    .unwrap();
    let mut configured = role("claude-custom", &[]);
    configured.runtime = "claude-code".into();
    insert_role_row(&pool.get().unwrap(), &configured);
    let manager = mgr_with_fake(None, fake_runtime());
    let session = manager
        .spawn_direct(
            &configured,
            None,
            None,
            None,
            None,
            root.path().to_str(),
            None,
            None,
            root.path(),
            pool.clone(),
            capture(),
            None,
        )
        .unwrap();
    let saved = crate::repo::session::get_row(&pool.get().unwrap(), &session.id)
        .unwrap()
        .unwrap();
    assert_eq!(
        saved
            .launch_definition
            .unwrap()
            .claude_account
            .unwrap()
            .permission_mode,
        "bypassPermissions"
    );
    manager.kill(&session.id).unwrap();
}

#[test]
fn chat_bypass_switch_applies_to_roles_with_stored_permission_modes() {
    use runner_core::protocol::runtime::RuntimeLaunchOption;
    for runtime in [Runtime::Codex, Runtime::ClaudeCode] {
        for mode in [
            router::runtime::PermissionMode::Auto,
            router::runtime::PermissionMode::Bypass,
        ] {
            let pool = pool_with_schema();
            let root = tempfile::tempdir().unwrap();
            let mut configured = role("agent-custom", &[]);
            configured.runtime = runtime.key().into();
            configured.args = crate::runtimes::adapter(runtime)
                .permissions()
                .mode_args(mode);
            let stored_args = configured.args.clone();
            insert_role_row(&pool.get().unwrap(), &configured);
            crate::db::set_runtime_launch_option(&pool, runtime, RuntimeLaunchOption::Bypass, true)
                .unwrap();
            let fake = fake_runtime();
            let manager = mgr_with_fake(None, fake.clone());
            let session = manager
                .spawn_direct(
                    &configured,
                    None,
                    None,
                    None,
                    None,
                    root.path().to_str(),
                    None,
                    None,
                    root.path(),
                    pool.clone(),
                    capture(),
                    None,
                )
                .unwrap();
            let check = |args: &[String]| {
                for pair in crate::runtimes::adapter(runtime)
                    .permissions()
                    .mission_args()
                    .chunks_exact(2)
                {
                    assert!(
                        has_arg_pair(args, &pair[0], &pair[1]),
                        "{runtime}: {args:?}"
                    );
                }
                assert!(!has_arg_pair(args, "--permission-mode", "auto"));
                assert!(!has_arg_pair(args, "--sandbox", "workspace-write"));
            };
            check(&fake.last_spawn_spec().unwrap().args);
            manager.kill(&session.id).unwrap();
            pool.get()
                .unwrap()
                .execute(
                    "UPDATE sessions SET agent_session_key = ?2 WHERE id = ?1",
                    params![session.id, "11111111-1111-4111-8111-111111111111"],
                )
                .unwrap();
            manager
                .resume(
                    &session.id,
                    None,
                    None,
                    root.path(),
                    pool.clone(),
                    capture(),
                )
                .unwrap();
            check(&fake.last_spawn_spec().unwrap().args);
            assert_eq!(
                crate::repo::role::get(&pool.get().unwrap(), &configured.id)
                    .unwrap()
                    .unwrap()
                    .args,
                stored_args
            );
            manager.kill(&session.id).unwrap();
        }
    }
}
