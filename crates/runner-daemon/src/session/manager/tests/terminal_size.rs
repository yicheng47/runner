use super::*;

#[test]
fn mission_registration_preserves_initial_terminal_size() {
    let pool = pool_with_schema();
    let mission_base = Mission {
        crew_id: Some("c".into()),
        ..mission()
    };
    let role = role("/bin/cat", &[]);
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

    let mgr = mgr_with_fake(None, fake_runtime());
    let pending = mgr
        .register_mission_session(
            &mission,
            &role,
            &slot,
            fixture_tmp_dir(),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            None,
            None,
            Some((132, 41)),
            "caller-supplied",
        )
        .unwrap();

    assert_eq!(pending.spec.initial_size, Some((132, 41)));
}

#[test]
fn hinted_mission_start_forks_slots_at_the_hint() {
    // #367 across the whole seam: the size the resolver derives from the
    // frontend grid hint (mission_fork_size — exactly what mission_start
    // feeds register when the caller passes no size) must reach the PTY
    // fork itself. FakeRuntime records the SpawnSpec actually forked.
    let (size, source) = crate::ops::mission::mission_fork_size(None, Some((161, 45)));
    let pool = pool_with_schema();
    let mission_base = Mission {
        crew_id: Some("c".into()),
        ..mission()
    };
    let role = role("/bin/cat", &[]);
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

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture();
    let pending = mgr
        .register_mission_session(
            &mission,
            &role,
            &slot,
            fixture_tmp_dir(),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            None,
            None,
            size,
            source,
        )
        .unwrap();
    let session_id = pending.session_id.clone();
    let outcome = mgr
        .complete_mission_session_spawn(
            pending,
            Arc::clone(&cap) as Arc<dyn SessionEvents>,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .unwrap();

    assert!(matches!(outcome, CompleteSpawnOutcome::Spawned));
    assert_eq!(
        fake.last_spawn_spec().unwrap().initial_size,
        Some((161, 45))
    );
    mgr.kill(&session_id).unwrap();
}

#[test]
fn mission_fork_uses_a_size_pushed_before_the_pty_existed() {
    // The two-phase spawn leaves the row visible for the whole gate wait
    // before any PTY exists. A terminal that measures itself in that
    // window pushes through `resize`, which can only persist the size;
    // the fork must honor it over the hint captured at registration, or
    // the PTY comes up wider than the grid the terminal already moved to
    // and every full-width row wraps by a cell.
    let pool = pool_with_schema();
    let (mission, role, slot) = single_slot_mission(&pool);
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture();
    let pending = mgr
        .register_mission_session(
            &mission,
            &role,
            &slot,
            fixture_tmp_dir(),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            None,
            None,
            Some((113, 38)),
            "mission-hint",
        )
        .unwrap();
    let session_id = pending.session_id.clone();

    mgr.resize(&session_id, 112, 38, &pool).unwrap();
    assert!(
        fake.resizes.lock().unwrap().is_empty(),
        "no PTY yet: the push can only be persisted"
    );

    let outcome = mgr
        .complete_mission_session_spawn(
            pending,
            Arc::clone(&cap) as Arc<dyn SessionEvents>,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .unwrap();

    assert!(matches!(outcome, CompleteSpawnOutcome::Spawned));
    assert_eq!(
        fake.last_spawn_spec().unwrap().initial_size,
        Some((112, 38))
    );
    assert!(fake.resizes.lock().unwrap().is_empty());
    mgr.kill(&session_id).unwrap();
}

#[test]
fn mission_fork_applies_a_size_pushed_mid_fork() {
    // Narrower window, same drop: a push between the fork and the handle
    // install. The post-install re-read applies it to the new PTY.
    let pool = pool_with_schema();
    let (mission, role, slot) = single_slot_mission(&pool);
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture();
    let pending = mgr
        .register_mission_session(
            &mission,
            &role,
            &slot,
            fixture_tmp_dir(),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            None,
            None,
            Some((113, 38)),
            "mission-hint",
        )
        .unwrap();
    let session_id = pending.session_id.clone();
    {
        let mgr = Arc::clone(&mgr);
        let pool = Arc::clone(&pool);
        let session_id = session_id.clone();
        *fake.spawn_hook.lock().unwrap() = Some(Box::new(move || {
            mgr.resize(&session_id, 112, 38, &pool).unwrap();
        }));
    }

    let outcome = mgr
        .complete_mission_session_spawn(
            pending,
            Arc::clone(&cap) as Arc<dyn SessionEvents>,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .unwrap();

    assert!(matches!(outcome, CompleteSpawnOutcome::Spawned));
    assert_eq!(
        fake.last_spawn_spec().unwrap().initial_size,
        Some((113, 38)),
        "the push came after the fork"
    );
    assert_eq!(
        fake.resizes.lock().unwrap().as_slice(),
        &[(session_id.clone(), 112, 38)]
    );
    *fake.spawn_hook.lock().unwrap() = None;
    mgr.kill(&session_id).unwrap();
}

#[test]
fn unhinted_mission_start_still_forks_at_default() {
    // Same seam with no caller size and no recorded hint: the fork still
    // happens, at DEFAULT_PTY_SIZE — the pre-#367 behavior.
    let (size, source) = crate::ops::mission::mission_fork_size(None, None);
    let pool = pool_with_schema();
    let mission_base = Mission {
        crew_id: Some("c".into()),
        ..mission()
    };
    let role = role("/bin/cat", &[]);
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

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture();
    let pending = mgr
        .register_mission_session(
            &mission,
            &role,
            &slot,
            fixture_tmp_dir(),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            None,
            None,
            size,
            source,
        )
        .unwrap();
    let session_id = pending.session_id.clone();
    let outcome = mgr
        .complete_mission_session_spawn(
            pending,
            Arc::clone(&cap) as Arc<dyn SessionEvents>,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .unwrap();

    assert!(matches!(outcome, CompleteSpawnOutcome::Spawned));
    assert_eq!(
        fake.last_spawn_spec().unwrap().initial_size,
        Some(DEFAULT_PTY_SIZE)
    );
    mgr.kill(&session_id).unwrap();
}

#[test]
fn mission_registration_defaults_to_80x24_when_unsized() {
    // The last rung of the #367 chain: no caller size and no recorded
    // grid hint must still fork — at DEFAULT_PTY_SIZE, as before.
    let pool = pool_with_schema();
    let mission_base = Mission {
        crew_id: Some("c".into()),
        ..mission()
    };
    let role = role("/bin/cat", &[]);
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

    let mgr = mgr_with_fake(None, fake_runtime());
    let pending = mgr
        .register_mission_session(
            &mission,
            &role,
            &slot,
            fixture_tmp_dir(),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            None,
            None,
            None,
            "DEFAULT_PTY_SIZE",
        )
        .unwrap();

    assert_eq!(pending.spec.initial_size, Some(DEFAULT_PTY_SIZE));
}

#[test]
fn resume_applies_a_size_pushed_mid_fork() {
    // The resume window has the same shape as the mission fork: the row
    // exists, `resuming` is set, and there is no handle until the new
    // PTY is installed. A push in that window is persisted only; the
    // post-install re-read must apply it.
    let pool = pool_with_schema();
    let role_id = ulid::Ulid::new().to_string();
    let mut role = role("/bin/sh", &[]);
    role.id = role_id;
    role.handle = "midfork".into();
    role.runtime = "codex".into();
    insert_role_row(&pool.get().unwrap(), &role);

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let events = capture();
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
            events.clone(),
            None,
        )
        .unwrap();
    let session_id = spawned.id.clone();
    fake.close_spawn(0);
    wait_for_session_exit(&mgr, &pool, &session_id);
    {
        let mgr = Arc::clone(&mgr);
        let pool = Arc::clone(&pool);
        let session_id = session_id.clone();
        *fake.spawn_hook.lock().unwrap() = Some(Box::new(move || {
            mgr.resize(&session_id, 112, 38, &pool).unwrap();
        }));
    }

    mgr.resume(
        &session_id,
        Some(113),
        Some(38),
        fixture_tmp_dir(),
        Arc::clone(&pool),
        events.clone(),
    )
    .unwrap();

    assert_eq!(
        fake.last_spawn_spec().unwrap().initial_size,
        Some((113, 38)),
        "the caller's size still forks the PTY"
    );
    assert_eq!(
        fake.resizes.lock().unwrap().as_slice(),
        &[(session_id.clone(), 112, 38)]
    );
    *fake.spawn_hook.lock().unwrap() = None;
    mgr.kill(&session_id).unwrap();
}

#[test]
fn first_spawn_without_dims_uses_and_persists_default_size() {
    let pool = pool_with_schema();
    let role_id = ulid::Ulid::new().to_string();
    let mut role = role("/bin/sh", &[]);
    role.id = role_id;
    role.handle = "defaultsize".into();
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

    assert_eq!(
        fake.last_spawn_spec().unwrap().initial_size,
        Some(DEFAULT_PTY_SIZE)
    );
    let persisted: (u16, u16) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT last_cols, last_rows FROM sessions WHERE id = ?1",
            params![spawned.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(persisted, DEFAULT_PTY_SIZE);

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn resume_size_resolution_prefers_explicit_then_persisted_after_manager_restart() {
    let pool = pool_with_schema();
    let role_id = ulid::Ulid::new().to_string();
    let mut role = role("/bin/sh", &[]);
    role.runtime = "codex".into();
    role.id = role_id;
    role.handle = "persistedsize".into();
    insert_role_row(&pool.get().unwrap(), &role);

    let first_fake = fake_runtime();
    let first_mgr = mgr_with_fake(None, Arc::clone(&first_fake));
    let spawned = first_mgr
        .spawn_direct(
            &role,
            None,
            None,
            None,
            None,
            Some(fixture_tmp_dir().to_str().unwrap()),
            Some(120),
            Some(30),
            fixture_tmp_dir(),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();
    first_fake.close_spawn(0);
    wait_for_session_exit(&first_mgr, &pool, &spawned.id);
    first_mgr.resize(&spawned.id, 132, 41, &pool).unwrap();
    first_mgr.settle_pending_resize_now(&spawned.id);
    let persisted: (u16, u16) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT last_cols, last_rows FROM sessions WHERE id = ?1",
            params![spawned.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        persisted,
        (132, 41),
        "a stopped pane must persist its measured dimensions"
    );
    assert!(
        first_fake.resizes.lock().unwrap().is_empty(),
        "a stopped pane must not call the runtime resize path"
    );
    drop(first_mgr);
    drop(first_fake);

    let resumed_fake = fake_runtime();
    let resumed_mgr = mgr_with_fake(None, Arc::clone(&resumed_fake));
    resumed_mgr
        .resume(
            &spawned.id,
            None,
            None,
            fixture_tmp_dir(),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap();

    assert_eq!(
        resumed_fake.last_spawn_spec().unwrap().initial_size,
        Some((132, 41)),
        "unsized resume must use the DB size without manager memory"
    );
    resumed_mgr.kill(&spawned.id).unwrap();
    wait_for_session_exit(&resumed_mgr, &pool, &spawned.id);

    resumed_mgr
        .resume(
            &spawned.id,
            Some(144),
            Some(50),
            fixture_tmp_dir(),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap();
    assert_eq!(
        resumed_fake.last_spawn_spec().unwrap().initial_size,
        Some((144, 50)),
        "explicit resume size must win over the persisted size"
    );
    resumed_mgr.kill(&spawned.id).unwrap();
}

#[test]
fn resize_unknown_session_defers_validation_off_the_caller_thread() {
    let pool = pool_with_schema();
    let mgr = manager_with_runtime(crate::shell_path::LoginShellEnv::default(), inert_runtime());

    mgr.set_resize_settle_ms(3_600_000);
    mgr.resize("missing-session", 120, 30, &pool).unwrap();

    assert!(
        mgr.session_state("missing-session").is_some(),
        "the caller only records the measurement in manager memory"
    );
    mgr.settle_pending_resize_now("missing-session");
}

fn spawn_claude_for_resize(
    handle: &str,
) -> (
    Arc<db::DbPool>,
    Arc<FakeRuntime>,
    Arc<SessionManager>,
    String,
) {
    let pool = pool_with_schema();
    let role_id = ulid::Ulid::new().to_string();
    let mut role = role("/bin/cat", &[]);
    role.id = role_id;
    role.handle = handle.into();
    role.runtime = "claude-code".into();
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
            Some(120),
            Some(30),
            fixture_tmp_dir(),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();
    (pool, fake, mgr, spawned.id)
}

#[test]
fn resize_storm_ioctls_every_push_and_persists_once() {
    let (pool, fake, mgr, id) = spawn_claude_for_resize("storm");
    mgr.set_resize_settle_ms(3_600_000);
    {
        let conn = pool.get().unwrap();
        conn.execute_batch(
            "CREATE TABLE resize_writes (count INTEGER NOT NULL);
             INSERT INTO resize_writes VALUES (0);
             CREATE TRIGGER count_resize_writes
             AFTER UPDATE OF last_cols, last_rows ON sessions
             BEGIN UPDATE resize_writes SET count = count + 1; END;",
        )
        .unwrap();
    }

    for cols in [78u16, 76, 209, 76, 210] {
        mgr.resize(&id, cols, 30, &pool).unwrap();
    }
    let expected = vec![
        (id.clone(), 78, 30),
        (id.clone(), 76, 30),
        (id.clone(), 209, 30),
        (id.clone(), 76, 30),
        (id.clone(), 210, 30),
    ];
    assert_eq!(fake.resizes.lock().unwrap().clone(), expected);
    let before: (u16, u16, i64) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT s.last_cols, s.last_rows, w.count
               FROM sessions s CROSS JOIN resize_writes w
              WHERE s.id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(before, (120, 30, 0));

    mgr.settle_pending_resize_now(&id);
    assert_eq!(fake.resizes.lock().unwrap().clone(), expected);
    let settled: (u16, u16, i64) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT s.last_cols, s.last_rows, w.count
               FROM sessions s CROSS JOIN resize_writes w
              WHERE s.id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(settled, (210, 30, 1));

    mgr.kill(&id).unwrap();
}

#[test]
fn round_trip_resize_storm_preserves_the_immediate_width_chain() {
    let (pool, fake, mgr, id) = spawn_claude_for_resize("roundtrip");
    mgr.set_resize_settle_ms(3_600_000);

    for cols in [150u16, 209, 150, 120] {
        mgr.resize(&id, cols, 30, &pool).unwrap();
    }
    let expected = vec![
        (id.clone(), 150, 30),
        (id.clone(), 209, 30),
        (id.clone(), 150, 30),
        (id.clone(), 120, 30),
    ];
    assert_eq!(fake.resizes.lock().unwrap().clone(), expected);
    mgr.settle_pending_resize_now(&id);
    assert_eq!(fake.resizes.lock().unwrap().clone(), expected);

    mgr.kill(&id).unwrap();
}

#[test]
fn stale_resize_settle_does_not_persist_after_respawn() {
    let (pool, _fake, mgr, id) = spawn_claude_for_resize("stalegeneration");
    mgr.set_resize_settle_ms(3_600_000);

    mgr.resize(&id, 100, 30, &pool).unwrap();
    let stale_generation = mgr
        .session_state(&id)
        .unwrap()
        .lock()
        .unwrap()
        .pending_resize
        .as_ref()
        .unwrap()
        .generation;
    mgr.kill(&id).unwrap();
    mgr.resume(
        &id,
        None,
        None,
        fixture_tmp_dir(),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();

    mgr.resize(&id, 110, 30, &pool).unwrap();
    let state = mgr.session_state(&id).unwrap();
    let current_generation = state
        .lock()
        .unwrap()
        .pending_resize
        .as_ref()
        .unwrap()
        .generation;
    assert_ne!(stale_generation, current_generation);

    mgr.settle_pending_resize_generation_now(&id, stale_generation);
    assert_eq!(
        state
            .lock()
            .unwrap()
            .pending_resize
            .as_ref()
            .map(|pending| pending.generation),
        Some(current_generation)
    );
    let before: (u16, u16) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT last_cols, last_rows FROM sessions WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(before, (120, 30));

    mgr.settle_pending_resize_generation_now(&id, current_generation);
    let settled: (u16, u16) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT last_cols, last_rows FROM sessions WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(settled, (110, 30));

    mgr.kill(&id).unwrap();
}

#[test]
fn settle_during_inflight_kill_abandons_persistence() {
    let (pool, fake, mgr, id) = spawn_claude_for_resize("killwindow");
    mgr.set_resize_settle_ms(3_600_000);

    mgr.resize(&id, 100, 30, &pool).unwrap();
    let (stop_entered, stop_release) = fake.arm_stop_gate();
    let kill = {
        let mgr = Arc::clone(&mgr);
        let id = id.clone();
        std::thread::spawn(move || mgr.kill(&id))
    };
    stop_entered
        .recv_timeout(Duration::from_secs(2))
        .expect("kill never reached runtime.stop");
    mgr.settle_pending_resize_now(&id);
    let persisted: (u16, u16) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT last_cols, last_rows FROM sessions WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(persisted, (120, 30));
    assert_eq!(
        fake.resizes.lock().unwrap().clone(),
        vec![(id.clone(), 100, 30)]
    );

    stop_release.send(()).unwrap();
    kill.join().unwrap().unwrap();
}

#[test]
fn resize_settle_thread_persists_without_extra_ioctls() {
    let (pool, fake, mgr, id) = spawn_claude_for_resize("stormthread");
    mgr.set_resize_settle_ms(25);

    for cols in [78u16, 209, 210] {
        mgr.resize(&id, cols, 30, &pool).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let persisted: (u16, u16) = pool
            .get()
            .unwrap()
            .query_row(
                "SELECT last_cols, last_rows FROM sessions WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        if persisted == (210, 30) {
            break;
        }
        if Instant::now() > deadline {
            panic!("settle thread never persisted the storm");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        fake.resizes.lock().unwrap().clone(),
        vec![
            (id.clone(), 78, 30),
            (id.clone(), 209, 30),
            (id.clone(), 210, 30),
        ]
    );

    mgr.kill(&id).unwrap();
}

fn resize_terminal_fixture(
    temp: &tempfile::TempDir,
    id: &str,
) -> (
    crate::AppCore,
    Arc<FakeRuntime>,
    runner_core::protocol::terminal::TerminalAttachment,
) {
    let mut core = crate::test_support::test_core_in(temp.path().to_owned());
    let fake = fake_runtime();
    core.sessions = mgr_with_fake(None, Arc::clone(&fake));
    core.sessions.set_resize_settle_ms(3_600_000);
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    core.sessions
        .prepare_unlisted_terminal(id, (80, 24), &core.db, &events)
        .unwrap();
    install_test_session_handle(&core.sessions, id);
    let attachment = core.sessions.attach_terminal(id).unwrap();
    (core, fake, attachment)
}

fn resized_frames_until_output(
    core: &crate::AppCore,
    id: &str,
    attachment: &mut runner_core::protocol::terminal::TerminalAttachment,
) -> Vec<(u16, u16)> {
    let events = core.session_events();
    core.sessions
        .ingest_output_chunk(id, None, b"marker", &events);
    let mut sizes = Vec::new();
    loop {
        match attachment.frames.recv().unwrap() {
            runner_core::protocol::terminal::TerminalFrame::Resized { cols, rows, .. } => {
                sizes.push((cols, rows))
            }
            runner_core::protocol::terminal::TerminalFrame::Output { .. } => return sizes,
            runner_core::protocol::terminal::TerminalFrame::Resync => {
                panic!("resize frames overflowed the subscriber")
            }
        }
    }
}

#[test]
fn unchanged_resize_reaches_neither_the_runtime_nor_the_mirrors() {
    let temp = tempfile::tempdir().unwrap();
    let id = "unchanged-resize";
    let (core, fake, mut attachment) = resize_terminal_fixture(&temp, id);

    core.sessions.resize(id, 80, 24, &core.db).unwrap();
    core.sessions.resize(id, 100, 30, &core.db).unwrap();
    core.sessions.resize(id, 100, 30, &core.db).unwrap();
    core.sessions
        .take_back_terminal(id, 100, 30, &core.db)
        .unwrap();

    assert_eq!(
        resized_frames_until_output(&core, id, &mut attachment),
        vec![(100, 30)]
    );
    assert_eq!(
        fake.resizes.lock().unwrap().clone(),
        vec![(id.to_owned(), 100, 30)]
    );
    assert_eq!(core.sessions.terminal_model(id).unwrap().size(), (100, 30));
}

#[test]
fn alternating_remote_takes_resize_only_on_a_real_change() {
    let temp = tempfile::tempdir().unwrap();
    let id = "alternating-takes";
    let (core, fake, mut attachment) = resize_terminal_fixture(&temp, id);
    let request = |channel: u64, (cols, rows): (u16, u16), take: bool, viewport: bool| {
        let name = if channel == 1 { "First" } else { "Second" };
        core.sessions
            .browser_resize(id, channel, name, cols, rows, take, viewport, &core.db)
            .unwrap();
    };

    request(1, (100, 30), true, false);
    request(1, (100, 30), true, false);
    request(1, (100, 30), false, true);
    request(2, (120, 40), true, false);
    request(1, (90, 30), false, true);
    request(2, (120, 40), true, false);
    request(2, (120, 40), false, true);
    request(1, (100, 30), true, false);
    request(2, (100, 30), true, false);

    let expected = vec![(100, 30), (120, 40), (100, 30), (100, 30)];
    assert_eq!(
        resized_frames_until_output(&core, id, &mut attachment),
        expected
    );
    assert_eq!(
        fake.resizes
            .lock()
            .unwrap()
            .iter()
            .map(|(_, cols, rows)| (*cols, *rows))
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        core.sessions.browser_controller(id, 2),
        runner_core::companion::Controller::You
    );
}

#[test]
fn settle_and_install_handle_drop_locks_before_db_and_reject_stale_resize_writes_1377() {
    let (pool, _fake, mgr, id) = spawn_claude_for_resize("monotonic-settle");
    mgr.set_resize_settle_ms(3_600_000);

    // 1. Hold every DbPool connection so settle_pending_resize blocks at pool.get(); verify
    // SessionState + terminal_control were dropped by running a newer resize immediately, and
    // verify the older settle's write is rejected once DbPool frees.
    mgr.resize(&id, 100, 30, &pool).unwrap();
    let held_conns: Vec<_> = (0..8).map(|_| pool.get().unwrap()).collect();
    assert!(pool.try_get().is_none());

    let settle_mgr = Arc::clone(&mgr);
    let settle_id = id.clone();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let stale_settle = std::thread::spawn(move || {
        let _ = entered_tx.send(());
        settle_mgr.settle_pending_resize_now(&settle_id);
    });
    entered_rx.recv().unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    let state = mgr.session_state(&id).unwrap();
    loop {
        if state.lock().unwrap().pending_resize.is_none() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "settle_pending_resize did not take pending_resize and release SessionState"
        );
        std::thread::yield_now();
    }

    // While all 8 DbPool connections are still held, a newer resize must succeed immediately.
    mgr.resize(&id, 185, 42, &pool).unwrap();
    assert!(state.lock().unwrap().last_requested_size_dirty);

    drop(held_conns);
    stale_settle.join().unwrap();

    let after_stale: (u16, u16) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT last_cols, last_rows FROM sessions WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        after_stale,
        (120, 30),
        "stale settle must not overwrite SQLite after newer resize stamp"
    );
    assert!(
        state.lock().unwrap().last_requested_size_dirty,
        "stale settle must not clear dirty flag for newer resize"
    );

    mgr.settle_pending_resize_now(&id);
    let final_size: (u16, u16) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT last_cols, last_rows FROM sessions WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(final_size, (185, 42));
    assert!(!state.lock().unwrap().last_requested_size_dirty);
    mgr.kill(&id).unwrap();

    // 2. install_handle_with_size_persistence drops terminal_control, gate.state, and SessionState
    // before persist_size, and preserves dirty=true if a newer resize arrives mid-persistence.
    mgr.resize(&id, 150, 36, &pool).unwrap();
    let gate = state.lock().unwrap().delivery_gate.clone();
    let handle = crate::session::manager::SessionHandle {
        startup: None,
        #[cfg(windows)]
        pending_first_turn: None,
        id: id.clone(),
        mission_id: None,
        role_id: None,
        runtime_session: crate::session::runtime::RuntimeSession {
            runtime: "fake".into(),
            session_id: id.clone(),
        },
        rollout_capture: None,
        forwarder: None,
        input: Default::default(),
        stop: Default::default(),
    };
    let check_mgr = Arc::clone(&mgr);
    let check_state = Arc::clone(&state);
    let check_pool = Arc::clone(&pool);
    let check_id = id.clone();
    mgr.install_handle_with_size_persistence(
        &id,
        handle,
        None,
        Some((120, 30)),
        |cols, rows| {
            assert_eq!((cols, rows), (150, 36));
            let probe_mgr = Arc::clone(&check_mgr);
            let probe_gate = Arc::clone(&gate);
            let probe_state = Arc::clone(&check_state);
            let (probe_tx, probe_rx) = std::sync::mpsc::channel();
            let probe = std::thread::spawn(move || {
                drop(probe_mgr.terminal_control.lock().unwrap());
                drop(probe_gate.state.lock().unwrap());
                drop(probe_state.lock().unwrap());
                let _ = probe_tx.send(());
            });
            probe_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("terminal_control, delivery_gate.state, and SessionState must be dropped before persist_size");
            probe.join().unwrap();
            // Concurrent newer resize arrives while 150x36 is persisting.
            check_mgr.resize(&check_id, 205, 48, &check_pool).unwrap();
            crate::repo::session::update_last_size(
                &check_pool.get().unwrap(),
                &check_id,
                cols,
                rows,
            )
            .is_ok()
        },
        capture().as_ref(),
    );
    assert_eq!(state.lock().unwrap().last_requested_size, Some((205, 48)));
    assert!(
        state.lock().unwrap().last_requested_size_dirty,
        "install_handle persistence of 150x36 must not clear dirty flag for concurrent 205x48"
    );
    mgr.settle_pending_resize_now(&id);
    let installed_final: (u16, u16) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT last_cols, last_rows FROM sessions WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(installed_final, (205, 48));
    assert!(!state.lock().unwrap().last_requested_size_dirty);
    mgr.kill(&id).unwrap();
}

#[test]
fn install_handle_stalled_pool_checkout_keeps_terminal_control_free_1377() {
    let (pool, _fake, mgr, id) = spawn_claude_for_resize("install-handle-pool");
    mgr.kill(&id).unwrap();
    mgr.resize(&id, 144, 34, &pool).unwrap();

    let events: Arc<dyn SessionEvents> = capture();
    mgr.prepare_unlisted_terminal("healthy-pane", (80, 24), &pool, &events)
        .unwrap();
    let input_rx = mgr.record_terminal_input("healthy-pane");

    let (checkout_tx, checkout_rx) = std::sync::mpsc::channel();
    mgr.set_install_checkout_hook(&id, move || {
        checkout_tx.send(()).unwrap();
    });
    let held_conns: Vec<_> = (0..8).map(|_| pool.get().unwrap()).collect();
    assert!(pool.try_get().is_none());

    let handle = crate::session::manager::SessionHandle {
        startup: None,
        #[cfg(windows)]
        pending_first_turn: None,
        id: id.clone(),
        mission_id: None,
        role_id: None,
        runtime_session: crate::session::runtime::RuntimeSession {
            runtime: "fake".into(),
            session_id: id.clone(),
        },
        rollout_capture: None,
        forwarder: None,
        input: Default::default(),
        stop: Default::default(),
    };
    let install_mgr = Arc::clone(&mgr);
    let install_pool = Arc::clone(&pool);
    let install_events = Arc::clone(&events);
    let install_id = id.clone();
    let install_worker = std::thread::spawn(move || {
        install_mgr.install_handle(
            &install_id,
            handle,
            None,
            Some((120, 30)),
            &install_pool,
            &install_events,
        );
    });

    checkout_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("install_handle worker did not reach DbPool checkout stage");
    assert!(
        !install_worker.is_finished(),
        "install_handle must still be waiting on exhausted DbPool"
    );

    // Unrelated terminal_control operations must complete within a bounded window while
    // install_handle waits on DbPool checkout.
    let probe_mgr = Arc::clone(&mgr);
    let (probe_tx, probe_rx) = std::sync::mpsc::channel();
    let probe_worker = std::thread::spawn(move || {
        probe_mgr.desktop_input("healthy-pane", b"ping").unwrap();
        probe_mgr.release_browser_channel(99);
        let _ = probe_tx.send(());
    });
    let probe_ok = probe_rx.recv_timeout(Duration::from_millis(500)).is_ok();
    if !probe_ok {
        drop(held_conns);
        let _ = probe_worker.join();
        let _ = install_worker.join();
        panic!("terminal_control blocked while install_handle waited on DbPool checkout");
    }
    probe_worker.join().unwrap();
    assert_eq!(
        input_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        b"ping"
    );

    drop(held_conns);
    install_worker.join().unwrap();

    let persisted: (u16, u16) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT last_cols, last_rows FROM sessions WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(persisted, (144, 34));
    mgr.kill(&id).unwrap();
    mgr.drain_terminal_workers();
}

#[test]
fn failed_resume_and_failed_stop_preserve_pending_resize_settlement_1377() {
    let temp = tempfile::tempdir().unwrap();
    let (pool, fake, mgr, id) = spawn_claude_for_resize("failed-lifecycle");
    mgr.set_resize_settle_ms(3_600_000);
    let state = mgr.session_state(&id).unwrap();

    // 1a. Failed resume on an already-running session preserves deferred pending_resize.
    mgr.resize(&id, 150, 36, &pool).unwrap();
    assert!(mgr
        .resume(&id, None, None, temp.path(), Arc::clone(&pool), capture())
        .is_err());
    mgr.settle_pending_resize_now(&id);
    let after_failed_resume: (u16, u16) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT last_cols, last_rows FROM sessions WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(after_failed_resume, (150, 36));
    assert!(!state.lock().unwrap().last_requested_size_dirty);

    // 1b. Settle firing mid-resume (while resuming=true) defers and rearms on resume rollback.
    mgr.resize(&id, 162, 38, &pool).unwrap();
    let claim = mgr.try_claim_resume(&id).unwrap();
    mgr.settle_pending_resize_now(&id);
    assert!(state.lock().unwrap().last_requested_size_dirty);
    state
        .lock()
        .unwrap()
        .pending_resize
        .as_mut()
        .unwrap()
        .deadline = Instant::now();
    drop(claim);

    let deadline = Instant::now() + Duration::from_secs(5);
    while state.lock().unwrap().last_requested_size_dirty {
        assert!(
            Instant::now() < deadline,
            "deferred resize did not rearm and settle after failed resume rollback"
        );
        std::thread::yield_now();
    }
    let after_mid_resume: (u16, u16) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT last_cols, last_rows FROM sessions WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(after_mid_resume, (162, 38));

    // 2a. Failed runtime.stop preserves deferred pending_resize.
    fake.stop_failures.lock().unwrap().insert(id.clone());
    mgr.resize(&id, 174, 40, &pool).unwrap();
    assert!(mgr.kill(&id).is_err());
    mgr.settle_pending_resize_now(&id);
    let after_failed_stop: (u16, u16) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT last_cols, last_rows FROM sessions WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(after_failed_stop, (174, 40));
    assert!(!state.lock().unwrap().last_requested_size_dirty);

    // 2b. Settle firing mid-stop (while killed=true) defers and rearms when runtime.stop fails.
    let (stop_entered_tx, stop_entered_rx) = std::sync::mpsc::channel();
    let (stop_release_tx, stop_release_rx) = std::sync::mpsc::channel();
    *fake.stop_gate.lock().unwrap() = Some(super::RuntimeGate {
        entered: stop_entered_tx,
        release: stop_release_rx,
    });
    mgr.resize(&id, 188, 44, &pool).unwrap();
    let kill_mgr = Arc::clone(&mgr);
    let kill_id = id.clone();
    let kill_worker = std::thread::spawn(move || kill_mgr.kill(&kill_id));
    stop_entered_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap();

    mgr.settle_pending_resize_now(&id);
    assert!(state.lock().unwrap().last_requested_size_dirty);
    state
        .lock()
        .unwrap()
        .pending_resize
        .as_mut()
        .unwrap()
        .deadline = Instant::now();
    stop_release_tx.send(()).unwrap();
    assert!(kill_worker.join().unwrap().is_err());

    let deadline = Instant::now() + Duration::from_secs(5);
    while state.lock().unwrap().last_requested_size_dirty {
        assert!(
            Instant::now() < deadline,
            "deferred resize did not rearm and settle after failed stop rollback"
        );
        std::thread::yield_now();
    }
    let after_mid_stop: (u16, u16) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT last_cols, last_rows FROM sessions WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(after_mid_stop, (188, 44));

    fake.stop_failures.lock().unwrap().remove(&id);
    mgr.kill(&id).unwrap();
}

#[test]
fn install_handle_and_prepare_unlisted_revalidate_control_group_after_tab_move_1377() {
    use crate::ops::node::{node_tab_upsert, NodeTabUpsertInput};
    use runner_core::companion::Controller as WireController;
    use std::sync::Barrier;

    let state = crate::test_support::test_core();
    let events: Arc<dyn SessionEvents> = Arc::new(state.session_events());
    {
        let conn = state.db.get().unwrap();
        for id in ["anchor-a", "anchor-b", "moving-install", "moving-unlisted"] {
            conn.execute(
                "INSERT INTO sessions (id, status, agent_runtime, agent_command)
                 VALUES (?1, 'running', 'codex', 'codex')",
                params![id],
            )
            .unwrap();
        }
    }
    let tab_a = node_tab_upsert(
        &state,
        NodeTabUpsertInput {
            id: ulid::Ulid::new().to_string(),
            parent_id: None,
            name: "A".into(),
            layout: serde_json::json!({"slots": ["anchor-a", "moving-install", "moving-unlisted"]})
                .to_string(),
        },
    )
    .unwrap();
    let tab_b = node_tab_upsert(
        &state,
        NodeTabUpsertInput {
            id: ulid::Ulid::new().to_string(),
            parent_id: None,
            name: "B".into(),
            layout: serde_json::json!({"slots": ["anchor-b"]}).to_string(),
        },
    )
    .unwrap();

    state
        .sessions
        .prepare_unlisted_terminal("anchor-a", (80, 24), &state.db, &events)
        .unwrap();
    state
        .sessions
        .prepare_unlisted_terminal("anchor-b", (80, 24), &state.db, &events)
        .unwrap();
    state
        .sessions
        .browser_resize("anchor-a", 1, "First", 80, 24, true, false, &state.db)
        .unwrap();
    state
        .sessions
        .browser_resize("anchor-b", 2, "Second", 80, 24, true, false, &state.db)
        .unwrap();

    // 1. install_handle snapshots moving-install in tab A (channel 1), pauses at barrier while
    // node_tab_upsert moves moving-install into tab B (channel 2) and completes reconcile_tab_control.
    let snapped_install = Arc::new(Barrier::new(2));
    let release_install = Arc::new(Barrier::new(2));
    {
        let snapped = Arc::clone(&snapped_install);
        let release = Arc::clone(&release_install);
        state
            .sessions
            .set_after_install_snapshot_hook("moving-install", move || {
                snapped.wait();
                release.wait();
            });
    }
    let install_mgr = Arc::clone(&state.sessions);
    let install_pool = Arc::clone(&state.db);
    let install_events = Arc::clone(&events);
    let install_worker = std::thread::spawn(move || {
        let handle = crate::session::manager::SessionHandle {
            startup: None,
            #[cfg(windows)]
            pending_first_turn: None,
            id: "moving-install".into(),
            mission_id: None,
            role_id: None,
            runtime_session: crate::session::runtime::RuntimeSession {
                runtime: "fake".into(),
                session_id: "moving-install".into(),
            },
            rollout_capture: None,
            forwarder: None,
            input: Default::default(),
            stop: Default::default(),
        };
        install_mgr.install_handle(
            "moving-install",
            handle,
            None,
            Some((80, 24)),
            &install_pool,
            &install_events,
        );
    });

    snapped_install.wait();
    node_tab_upsert(
        &state,
        NodeTabUpsertInput {
            id: tab_b.id.clone(),
            parent_id: None,
            name: "B".into(),
            layout: serde_json::json!({"slots": ["anchor-b", "moving-install"]}).to_string(),
        },
    )
    .unwrap();
    release_install.wait();
    install_worker.join().unwrap();

    assert_eq!(
        state.sessions.browser_controller("moving-install", 2),
        WireController::You
    );
    assert!(state
        .sessions
        .browser_input("moving-install", 1, b"stale")
        .is_err());
    let rx_install = state.sessions.record_terminal_input("moving-install");
    state
        .sessions
        .browser_input("moving-install", 2, b"dest-b")
        .unwrap();
    assert_eq!(
        rx_install.recv_timeout(Duration::from_secs(2)).unwrap(),
        b"dest-b"
    );

    // 2. prepare_unlisted_terminal snapshots moving-unlisted in tab A (channel 1), pauses at
    // barrier while node_tab_upsert moves moving-unlisted into tab B (channel 2).
    let snapped_unlisted = Arc::new(Barrier::new(2));
    let release_unlisted = Arc::new(Barrier::new(2));
    {
        let snapped = Arc::clone(&snapped_unlisted);
        let release = Arc::clone(&release_unlisted);
        state
            .sessions
            .set_after_install_snapshot_hook("moving-unlisted", move || {
                snapped.wait();
                release.wait();
            });
    }
    let unlisted_mgr = Arc::clone(&state.sessions);
    let unlisted_pool = Arc::clone(&state.db);
    let unlisted_events = Arc::clone(&events);
    let unlisted_worker = std::thread::spawn(move || {
        unlisted_mgr
            .prepare_unlisted_terminal(
                "moving-unlisted",
                (80, 24),
                &unlisted_pool,
                &unlisted_events,
            )
            .unwrap();
    });

    snapped_unlisted.wait();
    node_tab_upsert(
        &state,
        NodeTabUpsertInput {
            id: tab_b.id,
            parent_id: None,
            name: "B".into(),
            layout: serde_json::json!({"slots": ["anchor-b", "moving-install", "moving-unlisted"]})
                .to_string(),
        },
    )
    .unwrap();
    let _ = tab_a;
    release_unlisted.wait();
    unlisted_worker.join().unwrap();

    assert_eq!(
        state.sessions.browser_controller("moving-unlisted", 2),
        WireController::You
    );
    assert!(state
        .sessions
        .browser_input("moving-unlisted", 1, b"stale")
        .is_err());
    let rx_unlisted = state.sessions.record_terminal_input("moving-unlisted");
    state
        .sessions
        .browser_input("moving-unlisted", 2, b"dest-b-unlisted")
        .unwrap();
    assert_eq!(
        rx_unlisted.recv_timeout(Duration::from_secs(2)).unwrap(),
        b"dest-b-unlisted"
    );

    state.sessions.forget_session_state("anchor-a");
    state.sessions.forget_session_state("anchor-b");
    state.sessions.forget_session_state("moving-unlisted");
    state.sessions.drain_terminal_workers();
}
