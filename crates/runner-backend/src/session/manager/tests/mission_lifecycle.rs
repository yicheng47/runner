use super::*;

// `compose_path` moved to `session::launch::compose_path` as
// part of the Step 9 cutover; equivalent coverage lives in
// `session::launch::tests::compose_path_*`.

#[test]
fn concurrent_missions_on_same_crew_keep_session_state_isolated() {
    // Per #55 the per-crew "at most one live mission" guard was
    // lifted. The contract that makes that safe is mission-id
    // namespacing: `sessions.mission_id` is a foreign key,
    // `kill_all_for_mission` filters on `mission_id`, the role
    // CLI shim path is keyed by mission_id, etc. This test pins
    // the session-isolation half of that contract: spawn one
    // session per mission against the same crew + same role
    // template, assert both alive concurrently, then assert
    // `kill_all_for_mission(A)` reaps A's session and leaves B's
    // alone.
    let pool = pool_with_schema();
    let role_id = ulid::Ulid::new().to_string();
    let crew_id = "c-concurrent".to_string();
    let slot_id = ulid::Ulid::new().to_string();
    let mission_a = ulid::Ulid::new().to_string();
    let mission_b = ulid::Ulid::new().to_string();
    let now = Utc::now().to_rfc3339();
    {
        let conn = pool.get().unwrap();
        conn.execute(
            "INSERT INTO crews (id, name, created_at, updated_at)
                 VALUES (?1, 'c', ?2, ?2)",
            params![crew_id, now],
        )
        .unwrap();
        crate::test_support::insert_test_role(&conn, &role_id, "concurrent", "shell", "/bin/cat");
        crate::test_support::insert_test_slot(
            &conn,
            &slot_id,
            &crew_id,
            &role_id,
            "concurrent",
            0,
            true,
        );
        for mid in [&mission_a, &mission_b] {
            conn.execute(
                "INSERT INTO missions (id, crew_id, title, status, started_at)
                     VALUES (?1, ?2, 't', 'running', ?3)",
                params![mid, crew_id, now],
            )
            .unwrap();
        }
    }

    let mut role = role("/bin/cat", &[]);
    role.id = role_id.clone();
    role.handle = "concurrent".into();
    let mut slot = slot_for(&role);
    slot.id = slot_id.clone();
    slot.crew_id = crew_id.clone();

    let mission_row_a = Mission {
        id: mission_a.clone(),
        crew_id: crew_id.clone(),
        ..mission()
    };
    let mission_row_b = Mission {
        id: mission_b.clone(),
        crew_id: crew_id.clone(),
        ..mission()
    };

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned_a = mgr
        .spawn(
            &mission_row_a,
            &role,
            &slot,
            fixture_tmp_dir(),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();
    let spawned_b = mgr
        .spawn(
            &mission_row_b,
            &role,
            &slot,
            fixture_tmp_dir(),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();
    assert_ne!(
        spawned_a.id, spawned_b.id,
        "two missions on the same crew must produce distinct session ids",
    );

    // Both sessions live in the SessionManager's map at this point
    // — /bin/cat reads stdin until EOF, so neither has exited yet.
    {
        assert!(
            mgr.session_state(&spawned_a.id).is_some_and(|state| state
                .lock()
                .unwrap()
                .handle
                .is_some()),
            "session A must be live"
        );
        assert!(
            mgr.session_state(&spawned_b.id).is_some_and(|state| state
                .lock()
                .unwrap()
                .handle
                .is_some()),
            "session B must be live"
        );
    }

    // Reap mission A's sessions only. The filter on mission_id must
    // leave B untouched.
    mgr.kill_all_for_mission(&mission_a).unwrap();

    // After kill_all_for_mission, A's reader thread joins via
    // SessionManager::kill (which awaits the join), so A's row is
    // already terminal in the DB. B is still running.
    let status_a: String = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT status FROM sessions WHERE id = ?1",
            params![spawned_a.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(status_a, "running", "mission A's session must be reaped");

    {
        assert!(
            mgr.session_state(&spawned_a.id).is_none_or(|state| state
                .lock()
                .unwrap()
                .handle
                .is_none()),
            "mission A's live handle must be cleared",
        );
        assert!(
            mgr.session_state(&spawned_b.id).is_some_and(|state| state
                .lock()
                .unwrap()
                .handle
                .is_some()),
            "mission B's session must survive kill_all_for_mission(A)",
        );
    }
    let status_b: String = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT status FROM sessions WHERE id = ?1",
            params![spawned_b.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        status_b, "running",
        "mission B's session row must still be running",
    );

    // Cleanup so the test's PTY child doesn't outlive the test.
    mgr.kill(&spawned_b.id).unwrap();
}

#[test]
fn mission_slot_exit_reaps_live_siblings_and_keeps_mission_running() {
    let pool = pool_with_schema();
    let mission_id = ulid::Ulid::new().to_string();
    let role_id = ulid::Ulid::new().to_string();
    let slot_id = insert_crew_role(&pool, &mission_id, &role_id);
    let mission = Mission {
        id: mission_id.clone(),
        crew_id: "c".into(),
        ..mission()
    };
    let mut role = role("/bin/cat", &[]);
    role.id = role_id;
    let mut slot = slot_for(&role);
    slot.id = slot_id;
    slot.crew_id = "c".into();

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let first = mgr
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
    let sibling = mgr
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

    fake.close_spawn(0);

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let conn = pool.get().unwrap();
        let live_sessions: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions
                  WHERE mission_id = ?1 AND status = 'running'",
                params![mission_id],
                |row| row.get(0),
            )
            .unwrap();
        if live_sessions == 0 {
            break;
        }
        if Instant::now() > deadline {
            panic!("mission siblings were not reaped");
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    assert!(
        fake.stops.lock().unwrap().contains(&sibling.id),
        "the surviving sibling must be stopped",
    );
    for session_id in [&first.id, &sibling.id] {
        assert!(
            mgr.session_state(session_id).is_none_or(|state| state
                .lock()
                .unwrap()
                .handle
                .is_none()),
            "session {session_id} must not retain a live handle",
        );
    }
    let mission_status: String = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT status FROM missions WHERE id = ?1",
            params![mission_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(mission_status, "running");
}

#[test]
fn mission_slot_exit_cancels_pending_sibling_spawns() {
    let pool = pool_with_schema();
    let mission_id = ulid::Ulid::new().to_string();
    let role_id = ulid::Ulid::new().to_string();
    let slot_id = insert_crew_role(&pool, &mission_id, &role_id);
    let mission = Mission {
        id: mission_id.clone(),
        crew_id: "c".into(),
        ..mission()
    };
    let mut role = role("/bin/cat", &[]);
    role.id = role_id;
    let mut slot = slot_for(&role);
    slot.id = slot_id;
    slot.crew_id = "c".into();

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    mgr.spawn(
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
    let cancel = mgr.register_pending_mission_cancel(&mission_id);

    fake.close_spawn(0);

    let deadline = Instant::now() + Duration::from_secs(2);
    while !cancel.load(std::sync::atomic::Ordering::Acquire) {
        if Instant::now() > deadline {
            panic!("pending sibling spawns were not cancelled");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    mgr.drop_pending_mission_cancel(&mission_id, &cancel);
}

#[test]
fn intentional_mission_kill_does_not_reap_siblings_from_exit_epilogue() {
    let pool = pool_with_schema();
    let mission_id = ulid::Ulid::new().to_string();
    let role_id = ulid::Ulid::new().to_string();
    let slot_id = insert_crew_role(&pool, &mission_id, &role_id);
    let mission = Mission {
        id: mission_id.clone(),
        crew_id: "c".into(),
        ..mission()
    };
    let mut role = role("/bin/cat", &[]);
    role.id = role_id;
    let mut slot = slot_for(&role);
    slot.id = slot_id;
    slot.crew_id = "c".into();

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let first = mgr
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
    let sibling = mgr
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

    // mission_stop kills sessions one at a time through this path. The
    // first intentional exit must not recursively start another sweep.
    mgr.kill(&first.id).unwrap();

    assert!(
        mgr.session_state(&sibling.id)
            .is_some_and(|state| state.lock().unwrap().handle.is_some()),
        "the sibling must stay live until mission_stop reaches it",
    );
    assert!(
        !fake.stops.lock().unwrap().contains(&sibling.id),
        "the first intentional exit must not stop its sibling",
    );
    let sibling_status: String = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT status FROM sessions WHERE id = ?1",
            params![sibling.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(sibling_status, "running");
    let first_status: String = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT status FROM sessions WHERE id = ?1",
            params![first.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(first_status, "stopped");

    mgr.kill_all_for_mission(&mission_id).unwrap();
}

#[test]
fn spawn_marks_session_stopped_after_runtime_channel_closes() {
    // Spawn a mission session through FakeRuntime, then close
    // the runtime's output channel to simulate a clean pane exit.
    // The forwarder thread should query status (FakeRuntime
    // returns exit_code=0 by default), flip the DB row to
    // 'stopped', and emit ExitEvent with success=true.
    let pool = pool_with_schema();
    let mission = mission();
    let role = role("/bin/sh", &["-c", "echo hi"]);
    insert_crew_role(&pool, &mission.id, &role.id);
    let project = {
        let conn = pool.get().unwrap();
        crate::repo::project::create(&conn, "Runner", "/tmp/runner").unwrap()
    };
    let mission = Mission {
        project_id: Some(project.id.clone()),
        ..mission
    };

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture();
    let slot = slot_for(&role);
    let spawned = mgr
        .spawn(
            &mission,
            &role,
            &slot,
            fixture_tmp_dir(),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            Arc::clone(&cap) as Arc<dyn SessionEvents>,
            None,
        )
        .unwrap();
    // pid is no longer pre-known on spawn return — the runtime
    // surfaces it lazily via status() once the manager needs it.
    assert!(spawned.pid.is_none());
    assert_eq!(fake.spawn_count(), 1);

    // Simulate a clean pane exit.
    fake.close_spawn(0);

    // Wait for the whole exit sequence: the row flips first, the exit event
    // and handle release follow, and the assertions below need all of it.
    wait_for_session_exit(&mgr, &pool, &spawned.id);
    let final_status: String = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT status FROM sessions WHERE id = ?1",
            params![spawned.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(final_status, "stopped");
    let stored_project: Option<String> = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT project_id FROM sessions WHERE id = ?1",
            params![spawned.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored_project, Some(project.id));

    // Exit event should have fired with success=true.
    let exits = cap.exit.lock().unwrap();
    assert_eq!(exits.len(), 1, "expected 1 exit event, got {}", exits.len());
    assert!(exits[0].success);
    drop(exits);

    let mission_status: String = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT status FROM missions WHERE id = ?1",
            params![mission.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(mission_status, "running");
    assert!(
        fake.stops
            .lock()
            .unwrap()
            .iter()
            .all(|session_id| session_id == &spawned.id),
        "a single-slot exit must not start a mission sweep",
    );
}

#[test]
fn spawn_failure_after_spawn_command_reaps_the_child() {
    // Force the `sessions` INSERT to fail by dropping the table after the
    // pool is built. Without the post-spawn cleanup, the child would keep
    // running after `spawn` returns Err because nothing knows about it.
    let pool = pool_with_schema();
    let mission = mission();
    let role = role("/bin/cat", &[]);
    insert_crew_role(&pool, &mission.id, &role.id);

    // Break the schema so the next INSERT fails.
    pool.get()
        .unwrap()
        .execute("DROP TABLE sessions", [])
        .unwrap();

    let mgr = manager_with_runtime(crate::shell_path::LoginShellEnv::default(), inert_runtime());
    let slot = slot_for(&role);
    let err = mgr
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
        .unwrap_err();
    // The error must surface the DB failure, not a spawn failure.
    assert!(
        format!("{err}").contains("sessions") || format!("{err}").contains("no such table"),
        "unexpected error: {err}"
    );
    // No live session left behind.
    assert!(mgr.sessions.lock().unwrap().values().all(|state| state
        .lock()
        .unwrap()
        .handle
        .is_none()));
}

#[test]
fn kill_blocks_until_session_row_is_terminal() {
    // mission_stop relies on this contract: kill must return only
    // after the forwarder thread has updated the DB row. With
    // FakeRuntime, `runtime.stop` drops the mpsc Sender so the
    // forwarder sees Disconnected and reconciles immediately;
    // `kill` joins on it before returning.
    let pool = pool_with_schema();
    let mission = mission();
    let role = role("/bin/cat", &[]);
    insert_crew_role(&pool, &mission.id, &role.id);

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let slot = slot_for(&role);
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

    mgr.kill(&spawned.id).unwrap();

    let conn = pool.get().unwrap();
    let status: String = conn
        .query_row(
            "SELECT status FROM sessions WHERE id = ?1",
            params![spawned.id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        status != "running",
        "kill returned while session still running: {status}"
    );
    // The killed flag caused the forwarder to classify as `stopped`
    // even though FakeRuntime returns exit_code=0.
    assert_eq!(status, "stopped");
    // The runtime should have observed at least one stop call
    // — two is normal (kill calls stop directly; the
    // forwarder also calls stop on its way out as
    // belt-and-suspenders cleanup once the channel closes).
    assert!(!fake.stops.lock().unwrap().is_empty());
}

#[test]
fn kill_many_stops_sessions_concurrently() {
    let pool = pool_with_schema();
    let mission_id = ulid::Ulid::new().to_string();
    let role_id = ulid::Ulid::new().to_string();
    let slot_id = insert_crew_role(&pool, &mission_id, &role_id);
    let mission = Mission {
        id: mission_id,
        crew_id: "c".into(),
        ..mission()
    };
    let mut role = role("/bin/cat", &[]);
    role.id = role_id;
    let mut slot = slot_for(&role);
    slot.id = slot_id;
    slot.crew_id = "c".into();

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let ids: Vec<_> = (0..3)
        .map(|_| {
            mgr.spawn(
                &mission,
                &role,
                &slot,
                fixture_tmp_dir(),
                PathBuf::from("/dev/null"),
                Arc::clone(&pool),
                capture(),
                None,
            )
            .unwrap()
            .id
        })
        .collect();
    *fake.stop_barrier.lock().unwrap() = Some(Arc::new(Barrier::new(ids.len())));

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let kill_ids = ids.clone();
    let kill = std::thread::spawn(move || {
        let _ = done_tx.send(mgr.kill_many(&kill_ids));
    });
    done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("kill_many must reach every stop barrier concurrently")
        .unwrap();
    kill.join().unwrap();

    let stopped: HashSet<_> = fake.stops.lock().unwrap().iter().cloned().collect();
    assert_eq!(stopped, ids.into_iter().collect());
}

#[test]
fn kill_all_for_mission_attempts_every_session_and_aggregates_failures() {
    let pool = pool_with_schema();
    let mission_id = ulid::Ulid::new().to_string();
    let role_id = ulid::Ulid::new().to_string();
    let slot_id = insert_crew_role(&pool, &mission_id, &role_id);
    let mission = Mission {
        id: mission_id.clone(),
        crew_id: "c".into(),
        ..mission()
    };
    let mut role = role("/bin/cat", &[]);
    role.id = role_id;
    let mut slot = slot_for(&role);
    slot.id = slot_id;
    slot.crew_id = "c".into();

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let first = mgr
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
    let second = mgr
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
    fake.fail_stop_for(&first.id);

    let error = mgr.kill_all_for_mission(&mission_id).unwrap_err();
    let message = error.to_string();
    assert!(message.contains(&first.id), "unexpected error: {message}");
    assert!(
        fake.stops.lock().unwrap().contains(&second.id),
        "sweep stopped before attempting the second session"
    );
    assert!(
        mgr.session_state(&second.id)
            .is_none_or(|state| state.lock().unwrap().handle.is_none()),
        "successful sessions must still be torn down"
    );
    assert!(
        mgr.session_state(&first.id)
            .is_some_and(|state| state.lock().unwrap().handle.is_some()),
        "failed session must remain retryable"
    );

    fake.allow_stop_for(&first.id);
    mgr.kill(&first.id).unwrap();
}

#[test]
fn spawn_direct_writes_session_with_null_mission_id_and_emits_activity() {
    // C8.5: a "Chat now" session lives outside any mission. Verify the
    // sessions row has mission_id IS NULL, the session lands in the
    // live state, and the role_activity emission fires on spawn.
    let pool = pool_with_schema();
    // We don't go through `insert_crew_role` here because direct
    // chat doesn't need a crew or mission — only a role row.
    let role_id = ulid::Ulid::new().to_string();
    let mut role = role("/bin/sh", &["-c", "echo direct"]);
    role.id = role_id.clone();
    role.handle = "directrunner".into();
    insert_role_row(&pool.get().unwrap(), &role);
    let project = {
        let conn = pool.get().unwrap();
        crate::repo::project::create(&conn, "Runner", fixture_tmp_dir().to_str().unwrap()).unwrap()
    };

    let cap = capture();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .spawn_direct(
            &role,
            None,
            None,
            None,
            Some(&project.id),
            Some(&project.cwd),
            None,
            None,
            fixture_tmp_dir(),
            Arc::clone(&pool),
            cap.clone(),
            None,
        )
        .unwrap();
    assert_eq!(spawned.mission_id, None);
    assert_eq!(spawned.role_id, Some(role_id.clone()));
    let (stored_project_id, stored_cwd): (Option<String>, Option<String>) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT project_id, cwd FROM sessions WHERE id = ?1",
            params![&spawned.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(stored_project_id.as_deref(), Some(project.id.as_str()));
    assert_eq!(stored_cwd.as_deref(), Some(project.cwd.as_str()));

    // Direct chat stays off-bus, but gets the spawning app's CLI.
    let spec = fake.last_spawn_spec().expect("spawn was called");
    assert!(!spec.mission, "spawn_direct must spawn with mission=false");
    assert!(spec.shim_dir.is_none(), "direct chat must not have a shim");
    assert_eq!(
        spec.bundled_bin_dir.as_deref(),
        Some(fixture_tmp_dir().join("bin").as_path()),
    );

    // Simulate clean exit so the activity emission cycle
    // completes (spawn-time emit then reap-time emit). The exit event is
    // emitted after the reap-time activity, so it is the signal to wait on;
    // the stopped row lands before either.
    fake.close_spawn(0);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let conn = pool.get().unwrap();
        let mission_id: Option<String> = conn
            .query_row(
                "SELECT mission_id FROM sessions WHERE id = ?1",
                params![&spawned.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            mission_id, None,
            "direct session must persist with NULL mission_id"
        );
        if !cap.exit.lock().unwrap().is_empty() {
            break;
        }
        if Instant::now() > deadline {
            panic!("direct session never exited");
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    // Last activity emission after reap should show zero
    // active sessions for this role.
    let activity = cap.activity.lock().unwrap();
    assert!(!activity.is_empty(), "role_activity must fire");
    let last = activity.last().unwrap();
    assert_eq!(last.role_id, role_id);
    assert_eq!(
        last.active_sessions, 0,
        "after reap, active_sessions for this role must be 0"
    );
}

#[test]
fn role_activity_event_direct_session_id_ignores_slot_bound_orphans() {
    let pool = pool_with_schema();
    let now = Utc::now();
    let role_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        crate::test_support::insert_test_role(&conn, &role_id, "directevent", "shell", "/bin/cat");
        let mut orphan = crate::test_support::test_session_row(
            "slot-orphan-newer",
            crate::model::SessionStatus::Running,
        );
        orphan.role_id = Some(role_id.clone());
        orphan.slot_id = Some("slot-old".into());
        orphan.started_at = Some(now + chrono::Duration::seconds(10));
        crate::repo::session::insert(&conn, &orphan).unwrap();
        let mut direct = crate::test_support::test_session_row(
            "direct-valid-older",
            crate::model::SessionStatus::Running,
        );
        direct.role_id = Some(role_id.clone());
        direct.started_at = Some(now);
        crate::repo::session::insert(&conn, &direct).unwrap();
    }

    let mut r = role("/bin/cat", &[]);
    r.id = role_id;
    r.handle = "directevent".into();
    let cap = capture();
    emit_role_activity(&pool, &r, cap.as_ref());

    let activity = cap.activity.lock().unwrap();
    let ev = activity.last().expect("role/activity event emitted");
    assert_eq!(
        ev.direct_session_id.as_deref(),
        Some("direct-valid-older"),
        "slot-bound orphan must not be emitted as a direct chat"
    );
}
