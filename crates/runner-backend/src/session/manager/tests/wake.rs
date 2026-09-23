use super::*;

#[test]
fn synthetic_wake_busy_updates_activity_and_allows_final_idle() {
    let pool = pool_with_schema();
    let mission = Mission {
        crew_id: "c".into(),
        ..mission()
    };
    let role = role("/bin/cat", &[]);
    let slot_id = insert_crew_role(&pool, &mission.id, &role.id);
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
    mgr.synthesize_wake_busy(
        &spawned.id,
        EventDraft::signal(
            mission.crew_id.clone(),
            mission.id.clone(),
            role.handle.clone(),
            SignalType::new("session_status"),
            serde_json::json!({ "state": "busy" }),
        ),
    )
    .unwrap();

    assert_eq!(
        mgr.activity_snapshot().get(&spawned.id),
        Some(&SessionActivityState::Busy),
        "synthetic busy must update the session-side dedup key",
    );
    let after_busy: Vec<_> = log
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
    assert_eq!(after_busy.len(), 3);
    assert_eq!(after_busy[2].payload["state"], "busy");

    fake.push_status(0, SessionActivityState::Idle);
    fake.push_output(0, b"final-idle-drained");
    wait_for_output_event(&cap, &spawned.id);

    let statuses: Vec<_> = log
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
    assert_eq!(statuses.len(), 4);
    assert_eq!(statuses[3].payload["state"], "idle");
    assert_eq!(statuses[3].payload["source"], "forwarder");
    assert_eq!(
        mgr.activity_snapshot().get(&spawned.id),
        Some(&SessionActivityState::Idle),
    );

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn suppressed_busy_then_agent_output_and_quiet_appends_final_idle() {
    let pool = pool_with_schema();
    let mission = Mission {
        crew_id: "c".into(),
        ..mission()
    };
    let role = role("/bin/cat", &[]);
    let slot_id = insert_crew_role(&pool, &mission.id, &role.id);
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

    mgr.inject_direct_stdin(&spawned.id, b"router nudge", cap.as_ref())
        .unwrap();
    assert!(
        mgr.session_state(&spawned.id)
            .unwrap()
            .lock()
            .unwrap()
            .suppress_local_input_busy,
        "unsubmitted nudge body must open the suppressed-busy window",
    );

    let log = EventLog::open(&mission_dir).unwrap();
    mgr.synthesize_wake_busy(
        &spawned.id,
        EventDraft::signal(
            mission.crew_id.clone(),
            mission.id.clone(),
            role.handle.clone(),
            SignalType::new("session_status"),
            serde_json::json!({ "state": "busy" }),
        ),
    )
    .unwrap();

    fake.push_status(0, SessionActivityState::Busy);
    fake.push_output(0, b"agent output");
    fake.push_status(0, SessionActivityState::Idle);
    fake.push_output(0, b"quiet-transition-drained");

    let deadline = Instant::now() + Duration::from_secs(2);
    let statuses = loop {
        let statuses: Vec<_> = log
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
        if statuses.last().is_some_and(|event| {
            event.payload.get("state").and_then(|state| state.as_str()) == Some("idle")
        }) && statuses.len() >= 4
        {
            break statuses;
        }
        assert!(
            Instant::now() < deadline,
            "final idle was not appended after the suppressed busy"
        );
        std::thread::sleep(Duration::from_millis(10));
    };

    assert_eq!(
        statuses
            .iter()
            .map(|event| event.payload["state"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["busy", "idle", "busy", "idle"],
        "the suppressed forwarder busy must not swallow the paired idle",
    );
    assert_eq!(statuses[3].payload["source"], "forwarder");
    assert_eq!(
        mgr.activity_snapshot().get(&spawned.id),
        Some(&SessionActivityState::Idle),
    );

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn forwarder_status_emit_stays_bounded_under_event_log_contention() {
    // Issue #124 / @reviewer P1: the forwarder consumer drains
    // terminal output, exit-event reap, AND `session_status`
    // emission through the same thread. If `try_append_session_status`
    // ever blocked on the event-log flock, a stuck mission log
    // would freeze terminal output too — the user would see a
    // hang the moment a second CLI writer took the lock.
    // Construct a real ForwarderEmitCtx against a tempdir,
    // steal the flock from another "process" (a parallel fd
    // holding LOCK_EX), and assert that
    // `try_append_session_status` exhausts its bounded retries and
    // returns `Contended` within a hard 100ms bound.
    use fs2::FileExt;
    use std::fs::OpenOptions;
    let dir = tempfile::tempdir().unwrap();
    let event_log = Arc::new(EventLog::open(dir.path()).unwrap());
    let blocker = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(event_log.path())
        .unwrap();
    blocker.lock_exclusive().unwrap();

    let ctx = ForwarderEmitCtx {
        crew_id: "test-crew".into(),
        mission_id: "test-mission".into(),
        handle: "tester".into(),
        event_log: Arc::clone(&event_log),
    };

    let start = Instant::now();
    let outcome = ctx.try_append_session_status(
        SessionActivityState::Idle,
        "forwarder",
        &AgentStatus::default(),
    );
    let elapsed = start.elapsed();

    assert!(
        elapsed < ci_scaled_budget(Duration::from_millis(100)),
        "try_append_session_status must not block; took {elapsed:?}",
    );
    assert!(
        matches!(outcome, AppendOutcome::Contended),
        "expected Contended outcome under lock contention",
    );

    // Streak-threshold table: the consumer logs at 1 / 10 / 100 /
    // 1000 / 10_000 / 20_000 / … Anything between those values
    // should be silent so a steady failure doesn't spam stderr.
    assert!(drop_streak_is_loggable(1));
    assert!(drop_streak_is_loggable(10));
    assert!(drop_streak_is_loggable(100));
    assert!(drop_streak_is_loggable(1000));
    assert!(drop_streak_is_loggable(10_000));
    assert!(drop_streak_is_loggable(20_000));
    assert!(!drop_streak_is_loggable(2));
    assert!(!drop_streak_is_loggable(50));
    assert!(!drop_streak_is_loggable(999));
    assert!(!drop_streak_is_loggable(10_001));
    assert!(!drop_streak_is_loggable(15_000));

    // Release the blocker and confirm the same call now succeeds.
    // Proves the test setup isn't accidentally getting Contended
    // for the wrong reason.
    blocker.unlock().unwrap();
    let outcome = ctx.try_append_session_status(
        SessionActivityState::Busy,
        "forwarder",
        &AgentStatus::default(),
    );
    assert!(matches!(outcome, AppendOutcome::Ok));
}

#[test]
fn forwarder_status_emit_retries_brief_event_log_contention() {
    use fs2::FileExt;
    use std::fs::OpenOptions;

    let dir = tempfile::tempdir().unwrap();
    let event_log = Arc::new(EventLog::open(dir.path()).unwrap());
    let blocker = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(event_log.path())
        .unwrap();
    blocker.lock_exclusive().unwrap();

    let ctx = ForwarderEmitCtx {
        crew_id: "test-crew".into(),
        mission_id: "test-mission".into(),
        handle: "tester".into(),
        event_log: Arc::clone(&event_log),
    };
    assert!(matches!(
        event_log.try_append(ctx.session_status_draft(
            SessionActivityState::Idle,
            "forwarder",
            &AgentStatus::default()
        )),
        Err(TryAppendError::Contended),
    ));

    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let retry_ctx = ctx.clone();
    let append = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        retry_ctx.try_append_session_status(
            SessionActivityState::Idle,
            "forwarder",
            &AgentStatus::default(),
        )
    });
    started_rx.recv().unwrap();
    // Keep the unlock well inside the ~35ms retry budget so a loaded CI
    // machine can't slip it past the last attempt.
    std::thread::sleep(Duration::from_millis(5));
    blocker.unlock().unwrap();

    assert!(matches!(append.join().unwrap(), AppendOutcome::Ok));
    let statuses: Vec<_> = event_log
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
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].payload["state"], "idle");
}

fn hold_event_log_lock(
    event_log: &EventLog,
) -> (std::sync::mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    use fs2::FileExt;
    use std::fs::OpenOptions;

    let path = event_log.path().to_path_buf();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let blocker = std::thread::spawn(move || {
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(path)
            .unwrap();
        file.lock_exclusive().unwrap();
        ready_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        file.unlock().unwrap();
    });
    ready_rx.recv().unwrap();
    (release_tx, blocker)
}

fn manager_with_contended_wake_sink(event_log: Arc<EventLog>) -> Arc<SessionManager> {
    let mgr = manager_with_runtime(crate::shell_path::LoginShellEnv::default(), inert_runtime());
    let state = mgr.session_state_or_insert("session");
    state.lock().unwrap().mission_status_sink = Some(ForwarderEmitCtx {
        crew_id: "crew".into(),
        mission_id: "mission".into(),
        handle: "runner".into(),
        event_log,
    });
    mgr
}

/// The wake append retries for a bounded budget and then reports the
/// contention instead of blocking, so under a lock held longer than that
/// budget "event log busy" is the designed outcome, not a failure. The tests
/// below hold the lock for as long as it takes their other path to prove it
/// was not blocked, which on a loaded runner can outlast the budget.
fn assert_wake_completed_or_gave_up(result: crate::error::Result<()>) {
    if let Err(error) = result {
        assert_eq!(error.to_string(), "event log busy", "{error}");
    }
}

fn wake_busy_draft() -> EventDraft {
    EventDraft::signal(
        "crew",
        "mission",
        "runner",
        SignalType::new("session_status"),
        serde_json::json!({ "state": "busy", "source": "router-wake" }),
    )
}

#[test]
fn contended_wake_append_does_not_block_output_ingestion() {
    let dir = tempfile::tempdir().unwrap();
    let event_log = Arc::new(EventLog::open(dir.path()).unwrap());
    let mgr = manager_with_contended_wake_sink(Arc::clone(&event_log));
    let (release, blocker) = hold_event_log_lock(&event_log);

    let session = mgr.session_state("session").unwrap();
    let session_guard = session.lock().unwrap();
    let (wake_started_tx, wake_started_rx) = std::sync::mpsc::channel();
    let wake_mgr = Arc::clone(&mgr);
    let wake = std::thread::spawn(move || {
        wake_started_tx.send(()).unwrap();
        wake_mgr.synthesize_wake_busy("session", wake_busy_draft())
    });
    wake_started_rx.recv().unwrap();
    std::thread::sleep(Duration::from_millis(1));
    drop(session_guard);
    std::thread::sleep(Duration::from_millis(2));

    let (output_started_tx, output_started_rx) = std::sync::mpsc::channel();
    let (output_done_tx, output_done_rx) = std::sync::mpsc::channel();
    let output_mgr = Arc::clone(&mgr);
    let output = std::thread::spawn(move || {
        output_started_tx.send(()).unwrap();
        let start = Instant::now();
        let mut event = None;
        for _ in 0..300 {
            event = Some(output_mgr.record_output("session", Some("mission"), b"output"));
        }
        output_done_tx.send((start.elapsed(), event)).unwrap();
    });
    output_started_rx.recv().unwrap();
    let observed = output_done_rx.recv_timeout(Duration::from_millis(20));

    release.send(()).unwrap();
    blocker.join().unwrap();
    let wake_result = wake.join().unwrap();
    output.join().unwrap();

    let (elapsed, event) = observed.expect(
        "record_output must finish while the wake append is still parked on the event-log lock",
    );
    assert!(
        elapsed < Duration::from_millis(20),
        "record_output must stay well under the wake retry budget; took {elapsed:?}",
    );
    assert_eq!(event.unwrap().seq, 300);
    assert_wake_completed_or_gave_up(wake_result);
}

#[test]
fn synthetic_wake_does_not_overwrite_a_newer_forwarder_transition() {
    let dir = tempfile::tempdir().unwrap();
    let event_log = Arc::new(EventLog::open(dir.path()).unwrap());
    let mgr = manager_with_contended_wake_sink(Arc::clone(&event_log));
    mgr.note_forwarder_transition("session", SessionActivityState::Idle, "forwarder");
    let (release, blocker) = hold_event_log_lock(&event_log);

    let session = mgr.session_state("session").unwrap();
    let session_guard = session.lock().unwrap();
    let (wake_started_tx, wake_started_rx) = std::sync::mpsc::channel();
    let wake_mgr = Arc::clone(&mgr);
    let wake = std::thread::spawn(move || {
        wake_started_tx.send(()).unwrap();
        wake_mgr.synthesize_wake_busy("session", wake_busy_draft())
    });
    wake_started_rx.recv().unwrap();
    std::thread::sleep(Duration::from_millis(1));
    drop(session_guard);
    std::thread::sleep(Duration::from_millis(2));

    let (transition_started_tx, transition_started_rx) = std::sync::mpsc::channel();
    let (transition_done_tx, transition_done_rx) = std::sync::mpsc::channel();
    let transition_mgr = Arc::clone(&mgr);
    let transition = std::thread::spawn(move || {
        transition_started_tx.send(()).unwrap();
        let busy_changed = transition_mgr.note_forwarder_transition(
            "session",
            SessionActivityState::Busy,
            "forwarder",
        );
        let idle_changed = transition_mgr.note_forwarder_transition(
            "session",
            SessionActivityState::Idle,
            "forwarder",
        );
        transition_done_tx
            .send((busy_changed, idle_changed))
            .unwrap();
    });
    transition_started_rx.recv().unwrap();
    let transition_before_unlock = transition_done_rx.recv_timeout(Duration::from_millis(20));

    release.send(()).unwrap();
    blocker.join().unwrap();
    let wake_result = wake.join().unwrap();
    transition.join().unwrap();

    assert_eq!(
        transition_before_unlock.expect(
            "newer forwarder transitions must acquire the session lock while append is parked",
        ),
        (true, true),
    );
    assert_wake_completed_or_gave_up(wake_result);
    assert_eq!(
        mgr.activity_snapshot().get("session"),
        Some(&SessionActivityState::Idle),
        "the completed wake append must not replace the newer idle transition",
    );
}

#[test]
fn failed_synthetic_wake_append_preserves_activity_and_error_mapping() {
    let dir = tempfile::tempdir().unwrap();
    let event_log = Arc::new(EventLog::open(dir.path()).unwrap());
    let mgr = manager_with_contended_wake_sink(Arc::clone(&event_log));
    mgr.note_forwarder_transition("session", SessionActivityState::Idle, "forwarder");
    let (release, blocker) = hold_event_log_lock(&event_log);

    let error = mgr
        .synthesize_wake_busy("session", wake_busy_draft())
        .unwrap_err();
    assert_eq!(error.to_string(), "event log busy");
    assert_eq!(
        mgr.activity_snapshot().get("session"),
        Some(&SessionActivityState::Idle),
    );

    release.send(()).unwrap();
    blocker.join().unwrap();

    let missing_dir = tempfile::tempdir().unwrap();
    let missing_log = Arc::new(EventLog::open(missing_dir.path()).unwrap());
    let missing_mgr = manager_with_contended_wake_sink(missing_log);
    missing_mgr.note_forwarder_transition("session", SessionActivityState::Idle, "forwarder");
    missing_dir.close().unwrap();

    let error = missing_mgr
        .synthesize_wake_busy("session", wake_busy_draft())
        .unwrap_err();
    assert_ne!(error.to_string(), "event log busy");
    assert_eq!(
        missing_mgr.activity_snapshot().get("session"),
        Some(&SessionActivityState::Idle),
    );
}
