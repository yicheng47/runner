use super::*;

#[test]
fn stream_status_publishes_without_panes_and_holds_delivery_for_chats_and_missions() {
    for mission in [false, true] {
        let manager = mgr_with_fake(None, fake_runtime());
        let pool = pool_with_schema();
        let id = "stream-status";
        pool.get()
            .unwrap()
            .execute(
                "INSERT INTO sessions(id, status, agent_runtime) VALUES (?1, 'running', 'shell')",
                [id],
            )
            .unwrap();
        install_test_session_handle(&manager, id);
        let root = tempfile::tempdir().unwrap();
        let log = Arc::new(EventLog::open(root.path()).unwrap());
        let events = capture();
        let host_events: Arc<dyn SessionEvents> = events.clone();
        let terminal = manager
            .create_terminal(id, 80, 24, &pool, &host_events)
            .unwrap();
        {
            let state = manager.session_state(id).unwrap();
            let mut state = state.lock().unwrap();
            state.terminal = Some(terminal);
            state.mission_status_sink = mission.then(|| ForwarderEmitCtx {
                crew_id: Some("crew".into()),
                mission_id: "mission".into(),
                handle: "agent".into(),
                event_log: log.clone(),
            });
            assert!(state.subscribers.is_empty());
        }
        let ingest = |bytes: &[u8]| {
            manager.ingest_output_chunk(id, mission.then_some("mission"), bytes, events.as_ref())
        };
        ingest(b"\x1b]7501;state=working:app=pi\x07");
        assert_eq!(
            manager.agent_status(id).observation.activity,
            Activity::Working
        );
        ingest(b"\x1b]7501;state=blocked:kind=question:msg=QW5zd2VyPw\x1b\\");
        let status = manager.agent_status(id);
        assert!(status.observation.needs_you());
        assert_eq!(
            status.observation.interactions[0].reason,
            crate::session::status::WaitReason::Answer
        );
        assert_eq!(
            status
                .observation
                .program_status
                .unwrap()
                .message
                .as_deref(),
            Some("Answer?")
        );
        assert_eq!(
            manager.delivery_readiness(id),
            router::DeliveryReadiness::NeedsYou
        );
        assert_eq!(
            manager.reserve_delivery(id).unwrap(),
            router::DeliveryReservation::HumanInteraction
        );
        let state = manager.session_state(id).unwrap();
        state.lock().unwrap().model.apply(
            SessionEvent::TestPending(true),
            crate::session::clock::state_now(),
        );
        ingest(b"\x1b]7501;state=idle\x07");
        assert!(!manager.agent_status(id).observation.needs_you());
        assert_eq!(
            manager.reserve_delivery(id).unwrap(),
            router::DeliveryReservation::LocalInputPending
        );
        state.lock().unwrap().model.apply(
            SessionEvent::TestPending(false),
            crate::session::clock::state_now(),
        );
        assert!(matches!(
            manager.reserve_delivery(id).unwrap(),
            router::DeliveryReservation::Ready(_)
        ));
        let statuses = events.status.lock().unwrap();
        assert_eq!(statuses.len(), 3);
        assert!(statuses
            .iter()
            .all(|status| status.source == StatusSource::ProgramStatus));
        if mission {
            let rows = log.read_from(0).unwrap();
            assert_eq!(rows.len(), statuses.len());
            for (row, status) in rows.iter().zip(statuses.iter()) {
                assert_eq!(
                    row.event.payload["status"],
                    serde_json::to_value(&status.status).unwrap()
                );
            }
        }
    }
}

#[test]
fn stream_reports_and_clear_never_run_the_completion_callback_even_when_armed() {
    let core = crate::test_support::test_core();
    let id = "stream-attention";
    core.db
        .get()
        .unwrap()
        .execute(
            "INSERT INTO sessions(id, status, agent_runtime) VALUES (?1, 'running', 'pi')",
            [id],
        )
        .unwrap();
    install_test_session_handle(&core.sessions, id);
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    let terminal = core
        .sessions
        .create_terminal(id, 80, 24, &core.db, &events)
        .unwrap();
    let state = core.sessions.session_state(id).unwrap();
    state.lock().unwrap().terminal = Some(terminal);
    core.sessions.publish_direct_activity(
        id,
        SessionActivityState::Idle,
        StatusSource::Forwarder,
        events.as_ref(),
    );
    state.lock().unwrap().model.apply(
        SessionEvent::ArmCompletion,
        crate::session::clock::state_now(),
    );
    for status in ["working", "blocked", "done", "error", "idle", "clear"] {
        core.sessions.ingest_output_chunk(
            id,
            None,
            format!("\x1b]7501;state={status}\x07").as_bytes(),
            events.as_ref(),
        );
        assert!(state.lock().unwrap().model.completion_armed());
        assert_eq!(core.sessions.agent_status(id).unread_since, None);
    }
    assert!(
        !crate::repo::session_attention::any_unread(&core.db.get().unwrap(), &[id.into()]).unwrap()
    );
}

#[test]
fn replaced_launch_output_is_rejected_before_terminal_parsing() {
    let core = crate::test_support::test_core();
    let id = "old-program-launch";
    core.db
        .get()
        .unwrap()
        .execute(
            "INSERT INTO sessions(id, status, agent_runtime) VALUES (?1, 'running', 'shell')",
            [id],
        )
        .unwrap();
    install_test_session_handle(&core.sessions, id);
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    let terminal = core
        .sessions
        .create_terminal(id, 80, 24, &core.db, &events)
        .unwrap();
    let state = core.sessions.session_state(id).unwrap();
    state.lock().unwrap().terminal = Some(terminal);
    let previous = state.lock().unwrap().handle.as_ref().unwrap().stop.clone();
    install_test_session_handle(&core.sessions, id);
    core.sessions.ingest_output_for_launch(
        id,
        None,
        b"\x1b]7501;state=blocked\x07",
        events.as_ref(),
        Some(&previous),
    );
    assert!(core
        .sessions
        .agent_status(id)
        .observation
        .program_status
        .is_none());
    assert_eq!(state.lock().unwrap().output_seq, 0);
}

#[test]
fn process_exit_drops_child_records_and_partial_reports_before_resume() {
    let core = crate::test_support::test_core();
    let id = "program-resume";
    core.db
        .get()
        .unwrap()
        .execute(
            "INSERT INTO sessions(id, status, agent_runtime) VALUES (?1, 'running', 'shell')",
            [id],
        )
        .unwrap();
    install_test_session_handle(&core.sessions, id);
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    let terminal = core
        .sessions
        .create_terminal(id, 80, 24, &core.db, &events)
        .unwrap();
    let state = core.sessions.session_state(id).unwrap();
    state.lock().unwrap().terminal = Some(terminal);
    core.sessions.ingest_output_chunk(
        id,
        None,
        b"\x1b]7501;state=blocked:id=child\x07\x1b]7501;state=blocked",
        events.as_ref(),
    );
    core.sessions.record_exit_status(id, Some(0), false);
    assert!(core
        .sessions
        .agent_status(id)
        .observation
        .program_status
        .is_none());
    state
        .lock()
        .unwrap()
        .model
        .apply(SessionEvent::Attached, crate::session::clock::state_now());
    core.sessions
        .ingest_output_chunk(id, None, b"\x07\x1b]7501;state=idle\x07", events.as_ref());
    assert!(!core.sessions.agent_status(id).observation.needs_you());
    assert_eq!(
        core.sessions.agent_status(id).observation.activity,
        Activity::Idle
    );
}

#[test]
fn bridge_loss_does_not_replay_cached_or_partial_child_records() {
    use crate::session::state::agent::AgentEvent;
    for recovery in [
        b"\x1b]7501;state=idle\x07".as_slice(),
        b"\x1b]7501;state=clear:id=unrelated\x07",
    ] {
        for partial in [false, true] {
            let core = crate::test_support::test_core();
            let id = "program-bridge-loss";
            core.db.get().unwrap().execute("INSERT INTO sessions(id, status, agent_runtime) VALUES (?1, 'running', 'claude-code')", [id]).unwrap();
            install_test_session_handle(&core.sessions, id);
            let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
            let terminal = core
                .sessions
                .create_terminal(id, 80, 24, &core.db, &events)
                .unwrap();
            core.sessions
                .session_state(id)
                .unwrap()
                .lock()
                .unwrap()
                .terminal = Some(terminal);
            core.sessions
                .publish_agent_event(id, AgentEvent::TurnStarted, events.as_ref());
            let ingest = |bytes: &[u8]| {
                core.sessions
                    .ingest_output_chunk(id, None, bytes, events.as_ref())
            };
            ingest(b"\x1b]7501;state=blocked:id=old\x07");
            assert_eq!(
                core.sessions.agent_status(id).observation.source,
                ObservationSource::Hook
            );
            if partial {
                ingest(b"\x1b]7501;state=blocked:id=partial");
            }
            core.sessions.publish_model_event(
                id,
                SessionEvent::BridgeFailed { live: true },
                events.as_ref(),
            );
            if partial {
                ingest(b"\x07");
                assert!(!core.sessions.agent_status(id).observation.needs_you());
            }
            ingest(recovery);
            assert!(!core.sessions.agent_status(id).observation.needs_you());
            assert!(matches!(
                core.sessions.delivery_readiness(id),
                router::DeliveryReadiness::Ready { .. }
            ));
            ingest(b"\x1b]7501;state=blocked:id=new\x07");
            assert!(core.sessions.agent_status(id).observation.needs_you());
        }
    }
}

#[test]
fn tui_transcript_markers_preserve_native_work_and_wait_until_shell_return() {
    let core = crate::test_support::test_core();
    let id = "pi-transcript-zones";
    core.db
        .get()
        .unwrap()
        .execute(
            "INSERT INTO sessions(id, status, agent_runtime) VALUES (?1, 'running', 'shell')",
            [id],
        )
        .unwrap();
    install_test_session_handle(&core.sessions, id);
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    let terminal = core
        .sessions
        .create_terminal(id, 80, 24, &core.db, &events)
        .unwrap();
    core.sessions
        .session_state(id)
        .unwrap()
        .lock()
        .unwrap()
        .terminal = Some(terminal);
    let ingest = |bytes: &[u8]| {
        core.sessions
            .ingest_output_chunk(id, None, bytes, events.as_ref())
    };
    let redraw = b"\x1b]133;A\x07user\x1b]133;B\x07\x1b]133;C\x07\x1b]133;A\x07assistant\x1b]133;B\x07\x1b]133;C\x07";
    ingest(b"\x1b]7501;state=working:app=pi\x07");
    ingest(redraw);
    let working = core.sessions.agent_status(id);
    assert_eq!(working.observation.activity, Activity::Working);
    assert!(working.observation.program_status.is_some());
    ingest(b"\x1b]7501;state=blocked:kind=question:app=pi:msg=QW5zd2VyPw\x07");
    ingest(redraw);
    assert!(core
        .sessions
        .agent_status(id)
        .observation
        .program_status
        .is_some());
    assert_eq!(
        core.sessions.delivery_readiness(id),
        router::DeliveryReadiness::NeedsYou
    );
    ingest(b"\x1b]133;D;0\x07\x1b]133;A\x07");
    let shell = core.sessions.agent_status(id);
    assert!(shell.observation.program_status.is_none());
    assert!(!shell.observation.needs_you());
    assert_eq!(shell.unread_since, None);
    assert!(matches!(
        core.sessions.delivery_readiness(id),
        router::DeliveryReadiness::Ready { .. }
    ));
}

#[test]
fn antigravity_program_status_records_completion_only_on_done() {
    let core = crate::test_support::test_core();
    let id = "agy-program-attention";
    core.db
        .get()
        .unwrap()
        .execute(
            "INSERT INTO sessions(id, status, agent_runtime) VALUES (?1, 'running', 'antigravity')",
            [id],
        )
        .unwrap();
    install_test_session_handle(&core.sessions, id);
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    let terminal = core
        .sessions
        .create_terminal(id, 80, 24, &core.db, &events)
        .unwrap();
    let state = core.sessions.session_state(id).unwrap();
    state.lock().unwrap().terminal = Some(terminal);
    let ingest = |bytes: &[u8]| {
        core.sessions
            .ingest_output_chunk(id, None, bytes, events.as_ref())
    };
    let clear_unread = || {
        crate::repo::session_attention::mark_viewed(
            &core.db.get().unwrap(),
            id,
            "2026-11-13T00:00:00Z",
        )
        .unwrap();
        state
            .lock()
            .unwrap()
            .model
            .apply(SessionEvent::Viewed, crate::session::clock::state_now());
    };

    // Working -> Blocked -> Working -> Done records unread completion once.
    ingest(b"\x1b]7501;state=working:app=agy\x07");
    assert_eq!(core.sessions.agent_status(id).unread_since, None);
    ingest(b"\x1b]7501;state=blocked:kind=permission:msg=QWxsb3c/\x07");
    assert!(core.sessions.agent_status(id).observation.needs_you());
    assert_eq!(core.sessions.agent_status(id).unread_since, None);
    ingest(b"\x1b]7501;state=working:app=agy\x07");
    ingest(b"\x1b]7501;state=done:app=agy\x07");
    assert!(core.sessions.agent_status(id).unread_since.is_some());
    assert!(
        crate::repo::session_attention::any_unread(&core.db.get().unwrap(), &[id.into()]).unwrap()
    );

    clear_unread();

    // Keypress / TUI redraw emitting state=idle after Done does not re-trigger completion.
    ingest(b"\x1b]7501;state=idle:app=agy\x07");
    assert_eq!(core.sessions.agent_status(id).unread_since, None);

    // Working -> Idle (Esc interrupt) does NOT record unread completion.
    ingest(b"\x1b]7501;state=working:app=agy\x07");
    ingest(b"\x1b]7501;state=idle:app=agy\x07");
    assert_eq!(core.sessions.agent_status(id).unread_since, None);
    assert!(
        !crate::repo::session_attention::any_unread(&core.db.get().unwrap(), &[id.into()]).unwrap()
    );

    // Working -> Error does NOT record unread completion, and sets failed_since.
    ingest(b"\x1b]7501;state=working:app=agy\x07");
    ingest(b"\x1b]7501;state=error:app=agy\x07");
    assert_eq!(core.sessions.agent_status(id).unread_since, None);
    assert!(core.sessions.agent_status(id).failed_since.is_some());
    clear_unread();

    // Working -> clear followed by Forwarder Idle does NOT record unread completion
    // under either an Idle or Busy output baseline.
    for baseline in [SessionActivityState::Idle, SessionActivityState::Busy] {
        core.sessions.publish_direct_activity(
            id,
            baseline,
            StatusSource::Forwarder,
            events.as_ref(),
        );
        ingest(b"\x1b]7501;state=working:app=agy\x07");
        ingest(b"\x1b]7501;state=clear\x07");
        core.sessions.publish_direct_activity(
            id,
            SessionActivityState::Idle,
            StatusSource::Forwarder,
            events.as_ref(),
        );
        assert_eq!(
            core.sessions.agent_status(id).unread_since,
            None,
            "clear under {baseline:?} baseline must not record unread completion"
        );
        assert!(
            !crate::repo::session_attention::any_unread(&core.db.get().unwrap(), &[id.into()])
                .unwrap()
        );
    }
}

#[test]
fn antigravity_coalesced_and_split_program_status_chunks_preserve_completion_and_turns() {
    for coalesced in [false, true] {
        let core = crate::test_support::test_core();
        let id = if coalesced {
            "agy-coalesced"
        } else {
            "agy-split"
        };
        core.db
            .get()
            .unwrap()
            .execute(
                "INSERT INTO sessions(id, status, agent_runtime) VALUES (?1, 'running', 'antigravity')",
                [id],
            )
            .unwrap();
        install_test_session_handle(&core.sessions, id);
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        let terminal = core
            .sessions
            .create_terminal(id, 80, 24, &core.db, &events)
            .unwrap();
        let state = core.sessions.session_state(id).unwrap();
        state.lock().unwrap().terminal = Some(terminal);

        let emit_sequence = |chunks: &[&[u8]]| {
            if coalesced {
                let joined = chunks.concat();
                core.sessions
                    .ingest_output_chunk(id, None, &joined, events.as_ref());
            } else {
                for chunk in chunks {
                    core.sessions
                        .ingest_output_chunk(id, None, chunk, events.as_ref());
                }
            }
        };
        let clear_unread = || {
            crate::repo::session_attention::mark_viewed(
                &core.db.get().unwrap(),
                id,
                "2026-11-13T00:00:00Z",
            )
            .unwrap();
            state
                .lock()
                .unwrap()
                .model
                .apply(SessionEvent::Viewed, crate::session::clock::state_now());
        };

        // 1. Working -> Done in one chunk vs split chunks increments turn_started and records completion.
        emit_sequence(&[
            b"\x1b]7501;state=working:app=agy\x07",
            b"\x1b]7501;state=done:app=agy\x07",
        ]);
        assert_eq!(
            core.sessions.delivery_snapshot(id).unwrap().turn_started,
            1,
            "coalesced={coalesced}"
        );
        assert!(
            core.sessions.agent_status(id).unread_since.is_some(),
            "Working -> Done must record completion (coalesced={coalesced})"
        );
        assert_eq!(
            core.sessions.agent_status(id).observation.outcome,
            Some(TurnOutcome::Completed)
        );
        clear_unread();

        // 2. Working -> Done -> Idle in one chunk vs split chunks still records completion on Done
        // and settles cleanly to plain Idle.
        emit_sequence(&[
            b"\x1b]7501;state=working:app=agy\x07",
            b"\x1b]7501;state=done:app=agy\x07",
            b"\x1b]7501;state=idle:app=agy\x07",
        ]);
        assert_eq!(
            core.sessions.delivery_snapshot(id).unwrap().turn_started,
            2,
            "coalesced={coalesced}"
        );
        assert!(
            core.sessions.agent_status(id).unread_since.is_some(),
            "Done -> Idle in same chunk must preserve unread completion (coalesced={coalesced})"
        );
        assert_eq!(core.sessions.agent_status(id).observation.outcome, None);
        clear_unread();

        // 3. Working -> Idle in one chunk vs split chunks increments turn_started once and never records completion.
        emit_sequence(&[
            b"\x1b]7501;state=working:app=agy\x07",
            b"\x1b]7501;state=idle:app=agy\x07",
        ]);
        assert_eq!(
            core.sessions.delivery_snapshot(id).unwrap().turn_started,
            3,
            "coalesced={coalesced}"
        );
        assert_eq!(
            core.sessions.agent_status(id).unread_since,
            None,
            "Working -> Idle must not record completion (coalesced={coalesced})"
        );
    }
}
