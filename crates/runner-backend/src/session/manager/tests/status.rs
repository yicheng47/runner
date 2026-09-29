use super::*;

#[test]
fn direct_chat_status_transition_emits_session_status_busy() {
    let pool = pool_with_schema();
    let role_id = ulid::Ulid::new().to_string();
    let mut role = role("/bin/cat", &[]);
    role.id = role_id;
    role.handle = "directbusy".into();
    insert_role_row(&pool.get().unwrap(), &role);

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture();
    assert!(mgr.activity_snapshot().is_empty());
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

    let seeded = wait_for_session_status_event(&cap, &spawned.id, SessionActivityState::Busy);
    assert_eq!(seeded.source, "spawn");
    assert_eq!(
        mgr.activity_snapshot().get(&spawned.id),
        Some(&SessionActivityState::Busy)
    );
    cap.status.lock().unwrap().clear();

    fake.push_status(0, SessionActivityState::Idle);
    wait_for_session_status_event(&cap, &spawned.id, SessionActivityState::Idle);
    cap.status.lock().unwrap().clear();
    fake.push_status(0, SessionActivityState::Busy);
    let ev = wait_for_session_status_event(&cap, &spawned.id, SessionActivityState::Busy);

    assert_eq!(ev.session_id, spawned.id);
    assert_eq!(ev.state, SessionActivityState::Busy);
    assert_eq!(ev.source, "forwarder");

    mgr.kill(&spawned.id).unwrap();
    assert!(!mgr.activity_snapshot().contains_key(&spawned.id));
}

#[test]
fn direct_chat_status_transition_emits_session_status_idle() {
    let pool = pool_with_schema();
    let role_id = ulid::Ulid::new().to_string();
    let mut role = role("/bin/cat", &[]);
    role.id = role_id;
    role.handle = "directidle".into();
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
    assert!(
        mgr.session_state(&spawned.id)
            .unwrap()
            .lock()
            .unwrap()
            .mission_status_sink
            .is_none(),
        "direct chats must not carry a mission status sink",
    );

    fake.push_status(0, SessionActivityState::Idle);
    let ev = wait_for_session_status_event(&cap, &spawned.id, SessionActivityState::Idle);

    assert_eq!(ev.session_id, spawned.id);
    assert_eq!(ev.state, SessionActivityState::Idle);
    assert_eq!(ev.source, "forwarder");

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn direct_chat_typing_stays_idle_until_submit() {
    let pool = pool_with_schema();
    let role_id = ulid::Ulid::new().to_string();
    let mut role = role("/bin/cat", &[]);
    role.id = role_id;
    role.handle = "directtyping".into();
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

    fake.push_status(0, SessionActivityState::Idle);
    wait_for_session_status_event(&cap, &spawned.id, SessionActivityState::Idle);
    cap.status.lock().unwrap().clear();

    let token = match mgr.reserve_delivery(&spawned.id).unwrap() {
        router::DeliveryReservation::Ready(token) => token,
        other => panic!("expected delivery reservation, got {other:?}"),
    };
    let first_mgr = Arc::clone(&mgr);
    let first_cap = Arc::clone(&cap);
    let first_session_id = spawned.id.clone();
    let first = std::thread::spawn(move || {
        first_mgr
            .inject_direct_stdin(&first_session_id, b"h", first_cap.as_ref())
            .unwrap();
    });
    let wait_for_tickets = |expected| {
        // Generous on purpose: a loaded Windows runner once took over a second to schedule
        // the injecting thread. The loop leaves as soon as the ticket is issued.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let gate = mgr
                .session_state(&spawned.id)
                .unwrap()
                .lock()
                .unwrap()
                .delivery_gate
                .clone();
            if gate.state.lock().unwrap().next_ticket == expected {
                break;
            }
            assert!(Instant::now() < deadline, "input ticket was not issued");
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    wait_for_tickets(1);

    let second_mgr = Arc::clone(&mgr);
    let second_cap = Arc::clone(&cap);
    let second_session_id = spawned.id.clone();
    let second = std::thread::spawn(move || {
        second_mgr
            .inject_direct_stdin(&second_session_id, b"e", second_cap.as_ref())
            .unwrap();
    });
    wait_for_tickets(2);
    assert!(
        !first.is_finished() && !second.is_finished(),
        "local input must wait behind the reserved body/Enter chord"
    );
    assert!(mgr.inject_reserved(&spawned.id, token, b"[inbox]").unwrap());
    assert!(mgr.inject_reserved(&spawned.id, token, b"\r").unwrap());
    mgr.finish_delivery(&spawned.id, token);
    first.join().unwrap();
    second.join().unwrap();
    let writes = fake.bytes_writes();
    assert!(writes.ends_with(&[
        (spawned.id.clone(), b"[inbox]".to_vec()),
        (spawned.id.clone(), b"h".to_vec()),
        (spawned.id.clone(), b"e".to_vec()),
    ]));
    assert!(!mgr.input_quiescent(&spawned.id));
    assert!(
        !mgr.take_completion_armed(std::slice::from_ref(&spawned.id)),
        "typing without submit must not arm completion",
    );
    fake.push_status(0, SessionActivityState::Busy);
    fake.push_status(0, SessionActivityState::Idle);
    fake.push_output(0, b"typing-echo-drained");
    wait_for_output_event(&cap, &spawned.id);

    assert!(cap.status.lock().unwrap().is_empty());
    assert_eq!(
        mgr.activity_snapshot().get(&spawned.id),
        Some(&SessionActivityState::Idle)
    );

    mgr.inject_direct_stdin(&spawned.id, b"\r", cap.as_ref())
        .unwrap();
    let submitted = wait_for_session_status_event(&cap, &spawned.id, SessionActivityState::Busy);
    assert_eq!(submitted.source, "input-submit");
    assert_eq!(
        mgr.activity_snapshot().get(&spawned.id),
        Some(&SessionActivityState::Busy)
    );
    assert!(
        mgr.take_completion_armed(std::slice::from_ref(&spawned.id)),
        "xterm Enter submit must arm completion",
    );
    assert!(
        !mgr.take_completion_armed(std::slice::from_ref(&spawned.id)),
        "taking the submit completion arm must consume it",
    );

    mgr.inject_paste(&spawned.id, b"pasted prompt").unwrap();
    assert!(
        mgr.take_completion_armed(std::slice::from_ref(&spawned.id)),
        "paste-then-Enter delivery must arm completion",
    );

    let stale_token = match mgr.reserve_delivery(&spawned.id).unwrap() {
        router::DeliveryReservation::Ready(token) => token,
        other => panic!("expected delivery reservation, got {other:?}"),
    };
    mgr.kill(&spawned.id).unwrap();
    assert!(!mgr
        .inject_reserved(&spawned.id, stale_token, b"must not reach respawn")
        .unwrap());
    mgr.finish_delivery(&spawned.id, stale_token);
}

#[test]
fn direct_input_gate_timeout_is_bounded_and_does_not_pin_the_queue() {
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture();
    let session_id = "direct-timeout";
    let state = mgr.session_state_or_insert(session_id);
    let gate = state.lock().unwrap().delivery_gate.clone();
    state.lock().unwrap().handle = Some(SessionHandle {
        #[cfg(windows)]
        pending_first_turn: None,
        id: session_id.into(),
        mission_id: None,
        role_id: None,
        runtime_session: RuntimeSession {
            runtime: "fake".into(),
            session_id: session_id.into(),
        },
        codex_capture: None,
        forwarder: None,
        stop: Arc::new(AtomicBool::new(false)),
    });
    gate.state.lock().unwrap().in_flight = true;

    let budget = Duration::from_millis(20);
    let started = Instant::now();
    let error = mgr
        .inject_direct_stdin_with_wait_timeout(session_id, b"x", cap.as_ref(), budget)
        .unwrap_err();
    assert!(matches!(
        error,
        Error::DirectInputTimeout {
            ref session_id,
            timeout_ms: 20,
        } if session_id == "direct-timeout"
    ));
    assert!(
        started.elapsed() <= ci_scaled_budget(Duration::from_secs(1)),
        "gate timeout exceeded its bounded test budget"
    );

    mgr.finish_delivery(session_id, 0);
    mgr.inject_direct_stdin_with_wait_timeout(session_id, b"after-timeout", cap.as_ref(), budget)
        .unwrap();
    assert_eq!(
        fake.bytes_writes(),
        vec![(session_id.to_string(), b"after-timeout".to_vec())]
    );
}

#[test]
fn mission_status_transition_appends_once_and_matches_incremental_status() {
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
    slot.crew_id = mission.crew_id.clone();

    let app_data = tempfile::tempdir().unwrap();
    let events_log_path =
        runner_core::event_log::path::events_path(app_data.path(), &mission.crew_id, &mission.id);
    let mission_dir =
        runner_core::event_log::path::mission_dir(app_data.path(), &mission.crew_id, &mission.id);

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
            None,
        )
        .unwrap();

    fake.push_status(0, SessionActivityState::Busy);
    fake.push_status(0, SessionActivityState::Busy);
    fake.close_spawn(0);
    join_forwarder_for_test(&mgr, &spawned.id);

    let log = EventLog::open(&mission_dir).unwrap();
    let events: Vec<_> = log
        .read_from(0)
        .unwrap()
        .into_iter()
        .map(|entry| entry.event)
        .filter(|event| {
            event
                .signal_type
                .as_ref()
                .is_some_and(|ty| ty.as_str() == "session_status")
        })
        .collect();
    assert_eq!(
        events.len(),
        1,
        "unchanged mission states must not append duplicate rows",
    );
    let event = &events[0];

    assert_eq!(event.from, role.handle);
    assert_eq!(event.payload["state"], "busy");
    assert_eq!(event.payload["source"], "spawn");
    let incremental = cap.status.lock().unwrap();
    assert_eq!(incremental.len(), 1);
    assert_eq!(
        event.payload["status"],
        serde_json::to_value(&incremental[0].status).unwrap()
    );
}

#[test]
fn mission_typing_stays_idle_until_submit() {
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
    slot.crew_id = mission.crew_id.clone();

    let app_data = tempfile::tempdir().unwrap();
    let events_log_path =
        runner_core::event_log::path::events_path(app_data.path(), &mission.crew_id, &mission.id);
    let mission_dir =
        runner_core::event_log::path::mission_dir(app_data.path(), &mission.crew_id, &mission.id);

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
            None,
        )
        .unwrap();

    fake.push_status(0, SessionActivityState::Idle);
    fake.push_output(0, b"initial-idle-synced");
    wait_for_output_event(&cap, &spawned.id);
    cap.output.lock().unwrap().clear();

    let log = EventLog::open(&mission_dir).unwrap();
    let read_statuses = || {
        log.read_from(0)
            .unwrap()
            .into_iter()
            .map(|entry| entry.event)
            .filter(|event| {
                event
                    .signal_type
                    .as_ref()
                    .is_some_and(|ty| ty.as_str() == "session_status")
            })
            .collect::<Vec<_>>()
    };
    let initial_statuses = read_statuses();
    assert_eq!(initial_statuses.len(), 2);
    assert_eq!(initial_statuses[0].payload["state"], "busy");
    assert_eq!(initial_statuses[0].payload["source"], "spawn");
    assert_eq!(initial_statuses[1].payload["state"], "idle");
    assert_eq!(initial_statuses[1].payload["source"], "forwarder");

    mgr.inject_direct_stdin(&spawned.id, b"x", cap.as_ref())
        .unwrap();
    fake.push_status(0, SessionActivityState::Busy);
    fake.push_status(0, SessionActivityState::Idle);
    fake.push_status(0, SessionActivityState::Idle);
    fake.push_output(0, b"typing-echo-drained");
    wait_for_output_event(&cap, &spawned.id);

    assert_eq!(
        read_statuses().len(),
        2,
        "suppressed echo-busy and the unchanged idle transition must append nothing",
    );
    assert_eq!(
        mgr.activity_snapshot().get(&spawned.id),
        Some(&SessionActivityState::Idle),
    );
    assert!(
        !mgr.session_state(&spawned.id)
            .unwrap()
            .lock()
            .unwrap()
            .suppress_local_input_busy,
        "the idle transition must clear local-input suppression",
    );

    use fs2::FileExt;
    use std::fs::OpenOptions;
    let blocker = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(log.path())
        .unwrap();
    blocker.lock_exclusive().unwrap();

    let submit_mgr = Arc::clone(&mgr);
    let submit_cap = Arc::clone(&cap);
    let submit_session_id = spawned.id.clone();
    let (submit_done_tx, submit_done_rx) = std::sync::mpsc::channel();
    let submit = std::thread::spawn(move || {
        let result = submit_mgr
            .inject_direct_stdin(&submit_session_id, b"\r", submit_cap.as_ref())
            .map_err(|error| error.to_string());
        submit_done_tx.send(result).unwrap();
    });

    let deadline = Instant::now() + Duration::from_secs(2);
    while !fake
        .keys()
        .iter()
        .any(|(session_id, key)| session_id == &spawned.id && key == "Enter")
    {
        assert!(
            Instant::now() <= deadline,
            "submit never reached the PTY while the event log was contended",
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(matches!(
        submit_done_rx.recv_timeout(Duration::from_millis(50)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));

    blocker.unlock().unwrap();
    submit_done_rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    submit.join().unwrap();

    let statuses = read_statuses();
    assert_eq!(statuses.len(), 3);
    assert_eq!(statuses[2].payload["state"], "busy");
    assert_eq!(statuses[2].payload["source"], "input-submit");
    assert_eq!(
        statuses
            .iter()
            .filter(|event| event.payload["source"] == "input-submit")
            .count(),
        1,
    );
    assert_eq!(
        mgr.activity_snapshot().get(&spawned.id),
        Some(&SessionActivityState::Busy),
    );
    let incremental = cap.status.lock().unwrap();
    assert_eq!(incremental.len(), 3);
    assert_eq!(
        statuses[2].payload["status"],
        serde_json::to_value(&incremental[2].status).unwrap()
    );
    drop(incremental);

    mgr.kill(&spawned.id).unwrap();
}
