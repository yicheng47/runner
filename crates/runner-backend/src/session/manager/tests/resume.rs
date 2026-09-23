use super::*;

#[test]
fn mission_spawn_cwd_prefers_mission_over_role_working_dir() {
    // Regression guard for #101: the per-mission cwd typed into the
    // Start-mission modal must beat the role template's
    // `working_dir` default. Before the fix the role override
    // silently won, so StartMissionModal's helper text ("Each
    // role's PTY starts in this directory") was a lie.
    //
    // Exercises the resolver at the spawn site by inspecting the
    // SpawnSpec FakeRuntime captures. The contended both-set case
    // is the load-bearing one; the others lock in the fallback
    // chain so a future refactor can't quietly drop a branch.
    fn resolved_spawn_cwd(mission_cwd: Option<&str>, role_cwd: Option<&str>) -> Option<PathBuf> {
        let pool = pool_with_schema();
        let mission_base = mission();
        let mut role = role("/bin/sh", &["-c", "cat"]);
        role.working_dir = role_cwd.map(|s| s.to_string());
        let slot_id = insert_crew_role(&pool, &mission_base.id, &role.id);
        let mission = Mission {
            cwd: mission_cwd.map(|s| s.to_string()),
            ..mission_base
        };
        let mut slot = slot_for(&role);
        slot.id = slot_id;

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
                None,
            )
            .unwrap();
        let cwd = fake.last_spawn_spec().expect("spawn was called").cwd;
        mgr.kill(&spawned.id).unwrap();
        cwd
    }

    // The contended case: both set, mission wins. This is the bug.
    assert_eq!(
        resolved_spawn_cwd(Some("/mission-dir"), Some("/role-dir")),
        Some(PathBuf::from("/mission-dir")),
        "mission.cwd must beat role.working_dir when both are set",
    );
    // Mission only: mission flows through.
    assert_eq!(
        resolved_spawn_cwd(Some("/mission-only"), None),
        Some(PathBuf::from("/mission-only")),
    );
    // Role only: role is the fallback.
    assert_eq!(
        resolved_spawn_cwd(None, Some("/role-only")),
        Some(PathBuf::from("/role-only")),
    );
    assert_eq!(
        resolved_spawn_cwd(None, None),
        runner_core::app_paths::home_dir()
    );
    assert_eq!(
        resolved_spawn_cwd(Some(""), Some("/role-only")),
        Some(PathBuf::from("/role-only")),
    );
}

// Pre-#88 `mission_spawn_injects_preamble_for_non_lead_worker`
// is superseded by
// `mission_spawn_worker_preamble_lands_as_trailing_positional_argv_with_brief`
// above; the on-bus invariant from #45 is now exercised over
// the argv delivery path, and persistence-layer validation
// (`MAX_SYSTEM_PROMPT_BYTES` / `MAX_MISSION_GOAL_BYTES`)
// prevents the body from exceeding the runtime's argv slot.

#[cfg(unix)]
#[test]
fn codex_resume_skips_first_prompt_injection() {
    // On a codex resume the agent already has its system context
    // — replaying the brief would either be a no-op (codex
    // resume doesn't replay first turns) or, worse, push a fresh
    // user turn against the existing conversation. Verify the
    // resume path leaves stdin untouched: spawn /bin/cat with
    // codex runtime + a populated `agent_session_key` (so
    // `resume_plan` chooses the resuming branch), wait briefly,
    // and assert no echo arrived. Pairs with
    // `codex_fresh_spawn_injects_brief_via_stdin` — same setup,
    // opposite expectation, locking in the resume guard.
    let pool = pool_with_schema();
    let role_id = ulid::Ulid::new().to_string();
    let session_id = ulid::Ulid::new().to_string();
    let sibling_session_id = ulid::Ulid::new().to_string();
    let prior_key = uuid::Uuid::new_v4().to_string();
    let sibling_key = uuid::Uuid::new_v4().to_string();
    {
        let conn = pool.get().unwrap();
        let mut persisted_role = role("/bin/cat", &[]);
        persisted_role.id = role_id.clone();
        persisted_role.handle = "codex-resumer".into();
        persisted_role.runtime = "codex".into();
        persisted_role.system_prompt = Some("CODEX_BRIEF_TOKEN_RESUME".into());
        crate::repo::role::insert(&conn, &crate::repo::role::RoleRow::from(&persisted_role))
            .unwrap();
        for (id, key) in [
            (&session_id, &prior_key),
            (&sibling_session_id, &sibling_key),
        ] {
            let mut row =
                crate::test_support::test_session_row(id, crate::model::SessionStatus::Stopped);
            row.role_id = Some(role_id.clone());
            row.cwd = Some("/tmp".into());
            row.agent_session_key = Some(key.clone());
            crate::repo::session::insert(&conn, &row).unwrap();
        }
    }

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let resumed = mgr
        .resume(
            &session_id,
            None,
            None,
            std::path::Path::new("/tmp"),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap();

    let spec = fake
        .last_spawn_spec()
        .expect("codex resume should spawn through FakeRuntime");
    let mut expected = vec![
        "resume".into(),
        prior_key.clone(),
        "-c".into(),
        "check_for_update_on_startup=false".into(),
    ];
    expected.extend(router::runtime::codex_status_args(
        Some(Runtime::Codex),
        &[],
        Path::new("/tmp"),
        &session_id,
    ));
    assert_eq!(
        spec.args, expected,
        "resume must bind to its own native session key and status feed"
    );
    assert!(
        !spec.args.contains(&sibling_key),
        "codex resume must not use a sibling row's agent_session_key"
    );

    // FIRST_PROMPT_DELAY = ZERO under cfg(test); a would-be
    // injection would already be visible in fake.bytes_writes() by
    // the time resume() returns. The contract: codex resume
    // MUST NOT write anything containing the brief.
    let written: String = fake
        .bytes_writes()
        .iter()
        .map(|(_, p)| String::from_utf8_lossy(p).to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !written.contains("CODEX_BRIEF_TOKEN_RESUME"),
        "codex resume must NOT write the brief; got = {written:?}"
    );

    mgr.kill(&resumed.id).unwrap();
}

#[test]
fn resume_reuses_row_and_preserves_agent_session_key() {
    // Multi-chat-per-role contract: a direct chat IS a
    // sessions row. spawn_direct creates the row and the
    // claude-code adapter persists a UUID under
    // `agent_session_key`. After exit, resume respawns the
    // *same* row (same id, same agent_session_key column
    // populated) and flips status back to running. See
    // docs/impls/archive/0003-direct-chats.md.
    let pool = pool_with_schema();
    let role_id = ulid::Ulid::new().to_string();
    let mut role = role("/bin/sh", &["-c", "echo first"]);
    role.id = role_id.clone();
    role.handle = "resumer".into();
    role.runtime = "claude-code".into();
    insert_role_row(&pool.get().unwrap(), &role);

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture();
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
            Arc::clone(&cap) as Arc<dyn SessionEvents>,
            None,
        )
        .unwrap();
    let session_id = spawned.id.clone();

    // Force the spawn to "exit" so the forwarder marks the
    // row stopped; resume() refuses a row that's still
    // running, and the activity snapshot clears only once the
    // handle is released after the row flip.
    fake.close_spawn(0);
    wait_for_session_exit(&mgr, &pool, &session_id);

    // The claude-code adapter persisted a UUID — capture it.
    let key_before: Option<String> = {
        let conn = pool.get().unwrap();
        conn.query_row(
            "SELECT agent_session_key FROM sessions WHERE id = ?1",
            params![&session_id],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert!(
        key_before.is_some(),
        "claude-code spawn must persist an agent_session_key for later resume",
    );

    assert!(!mgr.activity_snapshot().contains_key(&session_id));
    cap.status.lock().unwrap().clear();

    // Resume: same id, same row.
    let resumed = mgr
        .resume(
            &session_id,
            None,
            None,
            fixture_tmp_dir(),
            Arc::clone(&pool),
            Arc::clone(&cap) as Arc<dyn SessionEvents>,
        )
        .unwrap();
    assert_eq!(resumed.id, session_id, "resume must reuse the row id");
    let seeded = wait_for_session_status_event(&cap, &session_id, SessionActivityState::Busy);
    assert_eq!(seeded.source, "resume");
    assert_eq!(
        mgr.activity_snapshot().get(&session_id),
        Some(&SessionActivityState::Busy)
    );
    assert!(
        !mgr.take_completion_armed(std::slice::from_ref(&session_id)),
        "resume busy seeding must not arm completion",
    );

    // After resume the status is running again with the
    // agent_session_key still populated. We don't pin the
    // UUID value — the resume_plan logic + missing-
    // conversation-file fallback can rotate it; the
    // manager-level invariant is "row id is preserved and
    // the key column stays populated."
    let key_after: Option<String> = {
        let conn = pool.get().unwrap();
        conn.query_row(
            "SELECT agent_session_key FROM sessions WHERE id = ?1",
            params![&session_id],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert!(
        key_after.is_some(),
        "resume must keep agent_session_key populated; got NULL",
    );

    // Only one row survives: resume must not have INSERTed a
    // duplicate.
    let count = crate::repo::role::session_count(&pool.get().unwrap(), &role_id).unwrap();
    assert_eq!(count, 1, "resume must update in place, not insert");

    mgr.kill(&session_id).unwrap();
}

#[test]
fn resume_refuses_running_and_archived_rows() {
    // Mission rows are no longer rejected — see
    // resume_mission_session_stamps_slot_handle_env. This test
    // covers the gates that remain.
    let pool = pool_with_schema();
    let role_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        crate::test_support::insert_test_role(&conn, &role_id, "r", "shell", "/bin/sh");
        // Already-running direct session.
        let mut running = crate::test_support::test_session_row(
            "running-sid",
            crate::model::SessionStatus::Running,
        );
        running.role_id = Some(role_id.clone());
        crate::repo::session::insert(&conn, &running).unwrap();
        // Archived direct session.
        let mut archived = crate::test_support::test_session_row(
            "archived-sid",
            crate::model::SessionStatus::Stopped,
        );
        archived.role_id = Some(role_id.clone());
        archived.archived_at = archived.started_at;
        crate::repo::session::insert(&conn, &archived).unwrap();
    }
    let mgr = manager_with_runtime(crate::shell_path::LoginShellEnv::default(), inert_runtime());
    for (sid, needle) in [
        ("running-sid", "already running"),
        ("archived-sid", "archived"),
    ] {
        let err = mgr
            .resume(
                sid,
                None,
                None,
                fixture_tmp_dir(),
                Arc::clone(&pool),
                capture(),
            )
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains(needle),
            "resume({sid}) should reject with `{needle}`, got `{msg}`"
        );
    }
}

#[test]
fn launch_resume_never_falls_back_to_a_fresh_chat_spawn() {
    let pool = pool_with_schema();
    let role_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        crate::test_support::insert_test_role(&conn, &role_id, "codex-role", "codex", "/bin/sh");
        let mut row = crate::test_support::test_session_row(
            "launch-sid",
            crate::model::SessionStatus::Stopped,
        );
        row.role_id = Some(role_id.clone());
        crate::repo::session::insert(&conn, &row).unwrap();
    }
    let mgr = manager_with_runtime(crate::shell_path::LoginShellEnv::default(), inert_runtime());

    let error = mgr
        .resume_on_launch(
            "launch-sid",
            None,
            None,
            fixture_tmp_dir(),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap_err();

    assert!(error.to_string().contains("cannot resume"));
    let status: String = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT status FROM sessions WHERE id = 'launch-sid'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "stopped");
}

#[test]
fn launch_resume_keeps_missing_cwd_as_a_chat_error() {
    let pool = pool_with_schema();
    let root = tempfile::tempdir().unwrap();
    let missing_cwd = root.path().join("deleted-chat-cwd");
    let role_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        crate::test_support::insert_test_role(&conn, &role_id, "codex-role", "codex", "/bin/sh");
        let mut row = crate::test_support::test_session_row(
            "chat-missing-cwd",
            crate::model::SessionStatus::Stopped,
        );
        row.role_id = Some(role_id.clone());
        row.cwd = Some(missing_cwd.to_string_lossy().into_owned());
        row.agent_session_key = Some("00000000-0000-0000-0000-000000000001".into());
        crate::repo::session::insert(&conn, &row).unwrap();
    }
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));

    let error = mgr
        .resume_on_launch(
            "chat-missing-cwd",
            None,
            None,
            fixture_tmp_dir(),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        format!(
            "working directory does not exist: {}",
            missing_cwd.to_string_lossy()
        )
    );
    assert_eq!(fake.spawn_count(), 0);
}

#[test]
fn resume_mission_session_stamps_slot_handle_env() {
    // Mission resume must look up the slot for the session and
    // use slot.slot_handle as RUNNER_HANDLE, not role.handle.
    // After the Step 9 cutover the manager hands env to the
    // runtime via SpawnSpec.env; FakeRuntime captures the spec
    // and we assert RUNNER_HANDLE == slot_handle directly.
    let pool = pool_with_schema();
    let now = Utc::now().to_rfc3339();
    let role_id = ulid::Ulid::new().to_string();
    let mission_id = ulid::Ulid::new().to_string();
    let slot_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
                 VALUES ('c-mr', 'c', ?1, ?1)",
            params![now],
        )
        .unwrap();
        let mut persisted_role = role("/bin/sh", &["-c", "echo HANDLE=$RUNNER_HANDLE && exit"]);
        persisted_role.id = role_id.clone();
        persisted_role.handle = "template-handle".into();
        crate::repo::role::insert(&conn, &crate::repo::role::RoleRow::from(&persisted_role))
            .unwrap();
        crate::test_support::insert_test_slot(
            &conn,
            &slot_id,
            "c-mr",
            &role_id,
            "architect-slot",
            0,
            true,
        );
        conn.execute(
            "INSERT INTO missions
                    (id, crew_id, title, status, started_at)
                 VALUES (?1, 'c-mr', 't', 'running', ?2)",
            params![mission_id, now],
        )
        .unwrap();
        let mut row =
            crate::test_support::test_session_row("mr-sid", crate::model::SessionStatus::Stopped);
        row.mission_id = Some(mission_id.clone());
        row.role_id = Some(role_id.clone());
        row.slot_id = Some(slot_id.clone());
        crate::repo::session::insert(&conn, &row).unwrap();
    }

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .resume(
            "mr-sid",
            None,
            None,
            fixture_tmp_dir(),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap();
    // Returned identity is the slot's, not the template's.
    assert_eq!(spawned.handle, "architect-slot");
    assert_eq!(spawned.mission_id.as_deref(), Some(mission_id.as_str()));

    // The SpawnSpec the manager built for the runtime must
    // carry RUNNER_HANDLE = slot_handle (not the template
    // handle), plus the other mission-bus env vars.
    let spec = fake
        .last_spawn_spec()
        .expect("resume should have called spawn");
    assert_eq!(
        spec.env.get("RUNNER_HANDLE").map(String::as_str),
        Some("architect-slot"),
        "RUNNER_HANDLE must be the slot_handle, got env = {:?}",
        spec.env,
    );
    assert_eq!(
        spec.env.get("RUNNER_CREW_ID").map(String::as_str),
        Some("c-mr"),
    );
    assert_eq!(
        spec.env.get("RUNNER_MISSION_ID").map(String::as_str),
        Some(mission_id.as_str()),
    );
    assert!(
        spec.shim_dir.is_some(),
        "mission resume must install the per-slot shim",
    );
    assert!(
        spec.bundled_bin_dir.is_some(),
        "mission resume must put the bundled CLI on PATH",
    );

    mgr.kill("mr-sid").unwrap();
}

#[test]
fn codex_mission_resume_grants_event_log_dir_to_sandbox() {
    let pool = pool_with_schema();
    let missing_cwd_root = tempfile::tempdir().unwrap();
    let missing_cwd = missing_cwd_root.path().join("deleted-mission-cwd");
    let now = Utc::now().to_rfc3339();
    let role_id = ulid::Ulid::new().to_string();
    let mission_id = ulid::Ulid::new().to_string();
    let slot_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
                 VALUES ('c-codex-resume', 'c', ?1, ?1)",
            params![now],
        )
        .unwrap();
        let mut persisted_role = role(
            "codex",
            &[
                "--ask-for-approval",
                "on-request",
                "--sandbox",
                "workspace-write",
            ],
        );
        persisted_role.id = role_id.clone();
        persisted_role.handle = "codex-template".into();
        persisted_role.runtime = "codex".into();
        crate::repo::role::insert(&conn, &crate::repo::role::RoleRow::from(&persisted_role))
            .unwrap();
        crate::test_support::insert_test_slot(
            &conn,
            &slot_id,
            "c-codex-resume",
            &role_id,
            "impl",
            0,
            true,
        );
        conn.execute(
            "INSERT INTO missions
                    (id, crew_id, title, status, started_at)
                 VALUES (?1, 'c-codex-resume', 't', 'running', ?2)",
            params![mission_id, now],
        )
        .unwrap();
        let mut row = crate::test_support::test_session_row(
            "codex-resume-sid",
            crate::model::SessionStatus::Stopped,
        );
        row.mission_id = Some(mission_id.clone());
        row.role_id = Some(role_id.clone());
        row.slot_id = Some(slot_id.clone());
        row.cwd = Some(missing_cwd.to_string_lossy().into_owned());
        crate::repo::session::insert(&conn, &row).unwrap();
    }

    let app_data = tempfile::tempdir().unwrap();
    let mission_dir =
        runner_core::event_log::path::mission_dir(app_data.path(), "c-codex-resume", &mission_id);
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .resume(
            "codex-resume-sid",
            None,
            None,
            app_data.path(),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap();

    let spec = fake
        .last_spawn_spec()
        .expect("resume should have called spawn");
    assert_eq!(spec.cwd.as_deref(), Some(missing_cwd.as_path()));
    let mission_dir_arg = mission_dir.to_string_lossy().to_string();
    assert!(
        has_arg_pair(&spec.args, "--add-dir", &mission_dir_arg),
        "codex mission resume must grant mission dir with --add-dir; args = {:?}",
        spec.args,
    );

    mgr.kill(&spawned.id).unwrap();
}

// The verify-and-retry first-prompt readback tests
// (`first_prompt_landed_first_try`, `*_after_retry`,
// `*_gives_up_after_max_attempts`,
// `continue_resume_rejects_stale_placeholder`) lived here
// before docs/impls/archive/0011 retired the readback verify path. The
// post-spawn "continue" auto-paste on resume that
// also lived here has been removed — Resume just respawns the
// PTY with no stdin injection, so the helper that synthesized
// a FakeRuntime SessionHandle for those tests is gone too.
