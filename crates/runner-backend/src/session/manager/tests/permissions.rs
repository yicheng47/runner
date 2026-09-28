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
        !body.contains("in a crew coordinated by the bundled"),
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
        !trailing.contains("in a crew coordinated by the bundled"),
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
    use crate::router::prompt::compose_worker_first_turn;

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

    let body = compose_worker_first_turn(role.system_prompt.as_deref(), None);
    // Composer ships the on-bus preamble + the brief.
    assert!(body.contains("in a crew coordinated by the bundled"));
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
        trailing.contains("in a crew coordinated by the bundled"),
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
        crew_id: "c".into(),
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
    let mission_dir =
        runner_core::event_log::path::mission_dir(app_data.path(), &mission.crew_id, &mission.id);
    let events_log_path =
        runner_core::event_log::path::events_path(app_data.path(), &mission.crew_id, &mission.id);

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
    let marker = crate::session::codex_capture::prompt_marker(&spawned.id);
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

fn mission_spawn_args(role: &Role, mode: MissionPermissionMode) -> Vec<String> {
    let pool = pool_with_schema();
    let (mission, slot) = seed_mission_rows(&pool, role);
    let app_data = tempfile::tempdir().unwrap();
    let events_log_path =
        runner_core::event_log::path::events_path(app_data.path(), &mission.crew_id, &mission.id);
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    mgr.set_mission_permission_mode(mode);
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
fn mission_spawn_converges_claude_row_to_the_app_wide_permission_mode() {
    let mut role = role("/bin/sh", &["--permission-mode", "plan", "--model", "opus"]);
    role.runtime = "claude-code".into();
    role.handle = "perm-claude".into();

    let bypass = mission_spawn_args(&role, MissionPermissionMode::Bypass);
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

    let auto = mission_spawn_args(&role, MissionPermissionMode::Auto);
    assert_eq!(&auto[..4], ["--model", "opus", "--permission-mode", "auto"]);
    assert!(has_arg_pair(&auto, "--permission-mode", "auto"), "{auto:?}");
    assert!(
        !has_arg_pair(&auto, "--permission-mode", "plan"),
        "{auto:?}"
    );
    assert!(has_arg_pair(&auto, "--model", "opus"), "{auto:?}");

    let role_default = mission_spawn_args(&role, MissionPermissionMode::RoleDefault);
    assert_eq!(&role_default[..role.args.len()], role.args);
    assert!(
        has_arg_pair(&role_default, "--permission-mode", "plan"),
        "{role_default:?}"
    );
    assert!(
        !role_default.iter().any(|arg| arg == "bypassPermissions"),
        "{role_default:?}"
    );
    assert!(
        has_arg_pair(&role_default, "--model", "opus"),
        "{role_default:?}"
    );
}

#[test]
fn mission_spawn_converges_codex_row_to_the_app_wide_permission_mode() {
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

    let bypass = mission_spawn_args(&role, MissionPermissionMode::Bypass);
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

    let auto = mission_spawn_args(&role, MissionPermissionMode::Auto);
    assert_eq!(
        &auto[..4],
        [
            "--ask-for-approval",
            "on-request",
            "--sandbox",
            "workspace-write"
        ]
    );
    assert!(
        has_arg_pair(&auto, "--ask-for-approval", "on-request"),
        "{auto:?}"
    );
    assert!(
        has_arg_pair(&auto, "--sandbox", "workspace-write"),
        "{auto:?}"
    );

    let role_default = mission_spawn_args(&role, MissionPermissionMode::RoleDefault);
    assert_eq!(&role_default[..role.args.len()], role.args);
    assert!(
        has_arg_pair(&role_default, "--ask-for-approval", "on-request"),
        "{role_default:?}"
    );
    assert!(
        has_arg_pair(&role_default, "--sandbox", "workspace-write"),
        "{role_default:?}"
    );
}

#[test]
fn mission_spawn_converges_trae_row_to_the_app_wide_permission_mode() {
    let mut role = role("trae-custom", &["--permission-mode", "auto", "--debug"]);
    role.runtime = "trae".into();

    let bypass = mission_spawn_args(&role, MissionPermissionMode::Bypass);
    assert_eq!(
        &bypass[..3],
        ["--debug", "--permission-mode", "bypass_permissions"]
    );
    let auto = mission_spawn_args(&role, MissionPermissionMode::Auto);
    assert_chat_has_no_permission_flags(&auto);
    assert_eq!(auto[0], "--debug");
    let role_default = mission_spawn_args(&role, MissionPermissionMode::RoleDefault);
    assert_eq!(&role_default[..role.args.len()], role.args);
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
    let events_log_path =
        runner_core::event_log::path::events_path(app_data.path(), &mission.crew_id, &mission.id);
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
fn mission_spawn_with_shell_runtime_ignores_the_permission_mode() {
    let role = role("/bin/sh", &["-c", "cat"]);
    let baseline = mission_spawn_args(&role, MissionPermissionMode::RoleDefault);
    for mode in [MissionPermissionMode::Bypass, MissionPermissionMode::Auto] {
        assert_eq!(mission_spawn_args(&role, mode), baseline, "{mode:?}");
    }
}

#[test]
fn mission_resume_reads_the_current_permission_mode() {
    let pool = pool_with_schema();
    let mut role = role("/bin/sh", &["--permission-mode", "plan"]);
    role.runtime = "claude-code".into();
    role.handle = "perm-resume".into();
    let (mission, slot) = seed_mission_rows(&pool, &role);
    let app_data = tempfile::tempdir().unwrap();
    let events_log_path =
        runner_core::event_log::path::events_path(app_data.path(), &mission.crew_id, &mission.id);

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture();
    mgr.set_mission_permission_mode(MissionPermissionMode::Bypass);
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

    mgr.set_mission_permission_mode(MissionPermissionMode::Auto);
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
        has_arg_pair(&second, "--permission-mode", "auto"),
        "{second:?}"
    );
    assert!(
        !second
            .iter()
            .any(|arg| arg == "bypassPermissions" || arg == "plan"),
        "{second:?}"
    );

    fake.close_spawn(1);
    wait_for_session_exit(&mgr, &pool, &spawned.id);

    mgr.set_mission_permission_mode(MissionPermissionMode::RoleDefault);
    mgr.resume(
        &spawned.id,
        None,
        None,
        app_data.path(),
        Arc::clone(&pool),
        Arc::clone(&cap) as Arc<dyn SessionEvents>,
    )
    .unwrap();
    let third = fake.last_spawn_spec().unwrap().args;
    assert!(
        has_arg_pair(&third, "--permission-mode", "plan"),
        "{third:?}"
    );
    assert!(
        !third
            .iter()
            .any(|arg| arg == "auto" || arg == "bypassPermissions"),
        "{third:?}"
    );

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn direct_spawn_ignores_the_mission_permission_mode() {
    // `--session-id <uuid>` and the `--settings` hook JSON embed the
    // fresh session id; everything else must match byte for byte.
    fn without_per_session_args(args: &[String]) -> Vec<String> {
        let mut out = Vec::with_capacity(args.len());
        let mut skip_next = false;
        for arg in args {
            if skip_next {
                skip_next = false;
                continue;
            }
            if arg == "--session-id" || arg == "--settings" {
                skip_next = true;
                continue;
            }
            out.push(arg.clone());
        }
        out
    }

    let pool = pool_with_schema();
    let mut role = role("/bin/sh", &["--permission-mode", "plan", "--model", "opus"]);
    role.runtime = "claude-code".into();
    role.handle = "perm-direct".into();
    insert_role_row(&pool.get().unwrap(), &role);

    let mut seen: Vec<Vec<String>> = Vec::new();
    for mode in MissionPermissionMode::ALL {
        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        mgr.set_mission_permission_mode(mode);
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
            "{mode:?}: {args:?}"
        );
        assert!(
            !args
                .iter()
                .any(|arg| arg == "bypassPermissions" || arg == "auto" || arg == "plan"),
            "{mode:?}: {args:?}"
        );
        seen.push(without_per_session_args(&args));
        mgr.kill(&spawned.id).unwrap();
    }
    assert!(seen.windows(2).all(|pair| pair[0] == pair[1]), "{seen:?}");
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
        // agy takes `--effort` only with a catalog model that lists the level.
        let effort = match runtime {
            "claude-code" | "copilot" => Some(("--effort", "high")),
            "pi" => Some(("--thinking", "high")),
            "antigravity" => None,
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
    let (first_turn, marker) = SessionManager::codex_capture_prompt_marker(
        Some(Runtime::Trae),
        "session-id",
        Some("first turn".to_string()),
    );
    let marker = marker.expect("trae must use the codex-lineage capture marker");
    assert_eq!(
        marker,
        crate::session::codex_capture::prompt_marker("session-id")
    );
    let expected = format!("first turn\n\n{marker}");
    assert_eq!(first_turn.as_deref(), Some(expected.as_str()));
}
