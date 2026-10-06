use super::*;

#[test]
fn mission_registration_preserves_initial_terminal_size() {
    let pool = pool_with_schema();
    let mission_base = Mission {
        crew_id: "c".into(),
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
        crew_id: "c".into(),
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
        crew_id: "c".into(),
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
        crew_id: "c".into(),
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
