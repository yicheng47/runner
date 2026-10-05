use super::*;

#[derive(Debug, PartialEq, Eq)]
enum ForwardedEvent {
    Output(u64, Vec<u8>),
    Status(SessionActivityState, String),
}

#[derive(Default)]
struct ForwarderCapture(Mutex<Vec<ForwardedEvent>>, Arc<terminal::FrameQueue>);
impl ForwarderCapture {
    fn drain(&self) {
        for frame in self.1.drain() {
            if let runner_core::protocol::terminal::TerminalFrame::Output { seq, bytes } = frame {
                self.0
                    .lock()
                    .unwrap()
                    .push(ForwardedEvent::Output(seq, bytes));
            }
        }
    }
    fn subscribe(&self, manager: &SessionManager, id: &str) {
        manager
            .session_state_or_insert(id)
            .lock()
            .unwrap()
            .subscribers
            .push((1, Arc::clone(&self.1)));
    }
}

impl SessionEvents for ForwarderCapture {
    fn status(&self, ev: &SessionActivityEvent) {
        self.drain();
        self.0
            .lock()
            .unwrap()
            .push(ForwardedEvent::Status(ev.state, ev.source.to_string()));
    }

    fn exit(&self, _: &ExitEvent) {}
}

fn forward_queued_output(items: Vec<RuntimeOutput>) -> Vec<ForwardedEvent> {
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let (rt_session, output) = fake
        .spawn(SpawnSpec {
            session_id: "burst-test".into(),
            ..Default::default()
        })
        .unwrap();
    for item in items {
        match item {
            RuntimeOutput::Stream(bytes) => fake.push_output(0, &bytes),
            RuntimeOutput::StatusTransition { state, .. } => fake.push_status(0, state),
            RuntimeOutput::AgentEvent { .. }
            | RuntimeOutput::ConversationStart(_)
            | RuntimeOutput::TerminalEvent(_)
            | RuntimeOutput::StatusBridgeFailed => {
                panic!("not a byte-batching fixture")
            }
        }
    }
    fake.close_spawn(0);
    let capture = Arc::new(ForwarderCapture::default());
    capture.subscribe(&mgr, &rt_session.session_id);
    let app_data = tempfile::tempdir().unwrap();
    mgr.start_forwarder_thread(
        rt_session.session_id.clone(),
        String::new(),
        None,
        rt_session,
        output,
        pool_with_schema(),
        capture.clone(),
        role("fake", &[]),
        false,
        false,
        None,
        app_data.path().to_path_buf(),
    )
    .join()
    .unwrap();
    capture.drain();
    let events = std::mem::take(&mut *capture.0.lock().unwrap());
    events
}

#[test]
fn codex_session_starts_rekey_current_running_row_for_direct_and_mission() {
    for mission in [false, true] {
        for keyed in [false, true] {
            let pool = pool_with_schema();
            let id = ulid::Ulid::new().to_string();
            let mission_id = mission.then(|| ulid::Ulid::new().to_string());
            let role_id = ulid::Ulid::new().to_string();
            if let Some(mission_id) = mission_id.as_deref() {
                insert_crew_role(&pool, mission_id, &role_id);
            }
            let old = uuid::Uuid::new_v4().to_string();
            let new = uuid::Uuid::new_v4().to_string();
            let mut row = crate::repo::session::SessionRowDb::new_running(id.clone());
            row.agent_session_key = keyed.then(|| old.clone());
            row.mission_id = mission_id.clone();
            row.started_at = Some(Utc::now());
            let started_at = row.started_at.unwrap().to_rfc3339();
            crate::repo::session::insert(&pool.get().unwrap(), &row).unwrap();

            let fake = fake_runtime();
            let mgr = mgr_with_fake(None, Arc::clone(&fake));
            let (rt_session, output) = fake
                .spawn(SpawnSpec {
                    session_id: id.clone(),
                    ..Default::default()
                })
                .unwrap();
            {
                let spawns = fake.spawns.lock().unwrap();
                let tx = spawns[0].tx.as_ref().unwrap();
                tx.send(RuntimeOutput::ConversationStart(new.clone()))
                    .unwrap();
                tx.send(RuntimeOutput::ConversationStart(new.clone()))
                    .unwrap();
            }
            fake.close_spawn(0);
            let events = capture();
            let app_data = tempfile::tempdir().unwrap();
            mgr.start_forwarder_thread(
                id.clone(),
                started_at.clone(),
                mission_id.clone(),
                rt_session,
                output,
                Arc::clone(&pool),
                events.clone(),
                role("fake", &[]),
                false,
                false,
                None,
                app_data.path().to_path_buf(),
            )
            .join()
            .unwrap();

            let conn = pool.get().unwrap();
            assert_eq!(
                crate::repo::session::get_row(&conn, &id)
                    .unwrap()
                    .unwrap()
                    .agent_session_key
                    .as_deref(),
                Some(new.as_str())
            );
            assert!(!crate::repo::session::capture_agent_session_key(
                &conn,
                &id,
                &old,
                &started_at
            )
            .unwrap());
            let updates = events.updated.lock().unwrap();
            assert_eq!(updates.len(), 1);
            assert_eq!(updates[0].session_id, id);
            assert_eq!(updates[0].mission_id, mission_id);
        }
    }
}

#[test]
fn codex_session_starts_cannot_rekey_older_or_stopped_rows() {
    for stale in [false, true] {
        let pool = pool_with_schema();
        let id = ulid::Ulid::new().to_string();
        let old = uuid::Uuid::new_v4().to_string();
        let new = uuid::Uuid::new_v4().to_string();
        let mut row = crate::repo::session::SessionRowDb::new_running(id.clone());
        row.agent_session_key = Some(old.clone());
        row.started_at = Some(Utc::now());
        let started_at = row.started_at.unwrap().to_rfc3339();
        crate::repo::session::insert(&pool.get().unwrap(), &row).unwrap();
        {
            let conn = pool.get().unwrap();
            if stale {
                conn.execute(
                    "UPDATE sessions SET started_at = ?2 WHERE id = ?1",
                    params![id, (Utc::now() + chrono::Duration::seconds(1)).to_rfc3339()],
                )
                .unwrap();
            } else {
                conn.execute(
                    "UPDATE sessions SET status = 'stopped' WHERE id = ?1",
                    params![id],
                )
                .unwrap();
            }
        }
        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        let (rt_session, output) = fake
            .spawn(SpawnSpec {
                session_id: id.clone(),
                ..Default::default()
            })
            .unwrap();
        fake.spawns.lock().unwrap()[0]
            .tx
            .as_ref()
            .unwrap()
            .send(RuntimeOutput::ConversationStart(new))
            .unwrap();
        fake.close_spawn(0);
        let events = capture();
        let app_data = tempfile::tempdir().unwrap();
        mgr.start_forwarder_thread(
            id.clone(),
            started_at,
            None,
            rt_session,
            output,
            Arc::clone(&pool),
            events.clone(),
            role("fake", &[]),
            false,
            false,
            None,
            app_data.path().to_path_buf(),
        )
        .join()
        .unwrap();
        assert_eq!(
            crate::repo::session::get_row(&pool.get().unwrap(), &id)
                .unwrap()
                .unwrap()
                .agent_session_key
                .as_deref(),
            Some(old.as_str())
        );
        assert!(events.updated.lock().unwrap().is_empty());
    }
}

#[test]
fn forwarder_coalesces_queued_stream_chunks_into_one_output_event() {
    let chunks = [
        vec![b'a'; 8 * 1024],
        vec![b'b'; 8 * 1024],
        vec![b'c'; 8 * 1024],
    ];
    let events = forward_queued_output(
        chunks
            .iter()
            .map(|bytes| RuntimeOutput::Stream(bytes.to_vec()))
            .collect(),
    );
    assert_eq!(events, vec![ForwardedEvent::Output(1, chunks.concat())]);
    eprintln!(
        "forwarder burst: {} queued 8 KiB chunks -> {} output event",
        chunks.len(),
        events.len()
    );
}

#[cfg(windows)]
#[test]
fn forwarder_delivers_a_cursor_burst_without_waiting_for_eof() {
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let (rt_session, output) = fake
        .spawn(SpawnSpec {
            session_id: "cursor-burst-test".into(),
            ..Default::default()
        })
        .unwrap();
    let redraw = b"\x1b[?2026h\x1b[?25l\x1b[38;1H\x1b[?25h\x1b[0 q\x1b[?2026l";
    let restore = b"\x1b[?25l \x1b[42;3H\x1b[?25h";
    fake.push_output(0, redraw);
    fake.push_output(0, restore);
    let capture = Arc::new(ForwarderCapture::default());
    capture.subscribe(&mgr, &rt_session.session_id);
    let app_data = tempfile::tempdir().unwrap();
    let forwarder = mgr.start_forwarder_thread(
        rt_session.session_id.clone(),
        String::new(),
        None,
        rt_session,
        output,
        pool_with_schema(),
        capture.clone(),
        role("fake", &[]),
        false,
        false,
        None,
        app_data.path().to_path_buf(),
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while {
        capture.drain();
        capture.0.lock().unwrap().is_empty()
    } && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(5));
    }
    capture.drain();
    let events = std::mem::take(&mut *capture.0.lock().unwrap());
    fake.close_spawn(0);
    forwarder.join().unwrap();
    assert_eq!(
        events,
        vec![ForwardedEvent::Output(
            1,
            [redraw.as_slice(), restore.as_slice()].concat()
        )],
    );
}

#[test]
fn forwarder_preserves_status_transition_between_stream_chunks() {
    let events = forward_queued_output(vec![
        RuntimeOutput::Stream(b"before".to_vec()),
        RuntimeOutput::StatusTransition {
            state: SessionActivityState::Idle,
            source: crate::session::state::StatusSource::Forwarder,
        },
        RuntimeOutput::Stream(b"after".to_vec()),
    ]);
    assert_eq!(
        events,
        vec![
            ForwardedEvent::Output(1, b"before".to_vec()),
            ForwardedEvent::Status(SessionActivityState::Idle, "forwarder".into()),
            ForwardedEvent::Output(2, b"after".to_vec()),
        ]
    );
}

#[test]
fn forwarder_caps_bursts_without_losing_the_next_chunk() {
    for first_len in [1024 * 1024, 1024 * 1024 - 1] {
        let first = vec![b'a'; first_len];
        let events = forward_queued_output(vec![
            RuntimeOutput::Stream(first.clone()),
            RuntimeOutput::Stream(b"bc".to_vec()),
            RuntimeOutput::Stream(b"de".to_vec()),
        ]);
        assert_eq!(
            events,
            vec![
                ForwardedEvent::Output(1, first),
                ForwardedEvent::Output(2, b"bcde".to_vec()),
            ]
        );
    }
}

#[test]
fn pre_attachment_key_effects_do_not_add_status_snapshot_rows() {
    let manager = mgr_with_fake(None, fake_runtime());
    let report = || PersistKey {
        key: Some("assigned-key".into()),
        generation: "spawn-start".into(),
        origin: KeyOrigin::Assigned,
    };
    assert!(manager
        .report_key("pending", report(), |_| Ok(true))
        .unwrap());
    assert!(manager.status_snapshot().is_empty());
    assert!(manager.session_state("pending").is_none());
    assert!(manager
        .inject_direct_stdin("pending", b"draft", capture().as_ref())
        .is_err());
    manager.note_forwarder_transition("pending", SessionActivityState::Busy, StatusSource::Spawn);
    assert_eq!(
        manager.status_snapshot()["pending"].lifecycle,
        Lifecycle::Running
    );
    manager.session_state_or_insert("measured-before-spawn");
    manager
        .report_key("measured-before-spawn", report(), |_| Ok(true))
        .unwrap();
    assert!(manager
        .status_snapshot()
        .contains_key("measured-before-spawn"));
}

#[test]
fn rejected_key_reports_and_explicit_forget_remove_pre_attachment_entries() {
    let manager = mgr_with_fake(None, fake_runtime());
    let report = || PersistKey {
        key: Some("assigned-key".into()),
        generation: "spawn-start".into(),
        origin: KeyOrigin::Assigned,
    };
    assert!(!manager
        .report_key("rejected", report(), |_| Ok(false))
        .unwrap());
    assert!(manager
        .report_key("failed", report(), |_| Err(Error::msg("write failed")))
        .is_err());
    assert!(manager.sessions.lock().unwrap().is_empty());

    manager
        .report_key("forgotten", report(), |_| Ok(true))
        .unwrap();
    assert!(manager.raw_session_state("forgotten").is_some());
    manager.forget_session_state("forgotten");
    assert!(manager.sessions.lock().unwrap().is_empty());
}

#[test]
fn rejected_key_cleanup_preserves_attachment_and_measurement_during_persistence() {
    let manager = mgr_with_fake(None, fake_runtime());
    let report = || PersistKey {
        key: Some("assigned-key".into()),
        generation: "spawn-start".into(),
        origin: KeyOrigin::Assigned,
    };
    manager
        .report_key("attached", report(), |_| {
            install_test_session_handle(&manager, "attached");
            Ok(false)
        })
        .unwrap();
    assert!(manager
        .session_state("attached")
        .unwrap()
        .lock()
        .unwrap()
        .handle
        .is_some());

    assert!(manager
        .report_key("measured", report(), |_| {
            manager
                .session_state_or_insert("measured")
                .lock()
                .unwrap()
                .last_requested_size = Some((120, 40));
            Err(Error::msg("write failed"))
        })
        .is_err());
    assert_eq!(manager.latest_requested_size("measured"), Some((120, 40)));
    assert_eq!(manager.sessions.lock().unwrap().len(), 2);
}
