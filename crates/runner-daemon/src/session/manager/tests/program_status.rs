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
            "INSERT INTO sessions(id, status, agent_runtime) VALUES (?1, 'running', 'copilot')",
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
            core.db.get().unwrap().execute("INSERT INTO sessions(id, status, agent_runtime) VALUES (?1, 'running', 'copilot')", [id]).unwrap();
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

    // Working -> Error does NOT record unread completion, sets failed_since, and
    // Antigravity Error -> Idle dismisses back to plain Idle (#1295).
    ingest(b"\x1b]7501;state=working:app=agy\x07");
    ingest(b"\x1b]7501;state=error:app=agy\x07");
    assert_eq!(core.sessions.agent_status(id).unread_since, None);
    assert!(core.sessions.agent_status(id).failed_since.is_some());
    ingest(b"\x1b]7501;state=idle:app=agy\x07");
    assert_eq!(core.sessions.agent_status(id).observation.outcome, None);
    assert_eq!(core.sessions.agent_status(id).failed_since, None);
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

#[test]
fn hooks_off_chats_promote_osc7501_across_start_resume_and_fork_while_shells_and_hooked_chats_stay_bounded(
) {
    use crate::session::state::agent::AgentEvent;
    use crate::session::status::WaitReason;
    use runner_core::protocol::runtime::RuntimeLaunchOption;

    for (runtime, app) in [
        (Runtime::Codex, "codex"),
        (Runtime::ClaudeCode, "claude-code"),
        (Runtime::Pi, "pi"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let fake = fake_runtime();
        let core = crate::test_support::test_core_with_runtime(fake.clone());
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        db::set_runtime_launch_option(&core.db, runtime, RuntimeLaunchOption::StatusHooks, false)
            .unwrap();

        let mut agent_role = role(runtime.key(), &[]);
        agent_role.runtime = runtime.key().into();
        agent_role.command = "fake".into();
        insert_role_row(&core.db.get().unwrap(), &agent_role);

        let spawned = core
            .sessions
            .spawn_direct(
                &agent_role,
                None,
                None,
                None,
                None,
                Some(dir.path().to_str().unwrap()),
                None,
                None,
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
                None,
            )
            .unwrap();
        let id = spawned.id;
        let state = core.sessions.session_state(&id).unwrap();
        assert!(
            state.lock().unwrap().model.program_status_authority(),
            "{runtime:?} start with hooks off must grant program-status authority"
        );
        // Late terminal creation after handle install must keep hooks-off authority.
        let _recreated = core
            .sessions
            .create_terminal(&id, 80, 24, &core.db, &events)
            .unwrap();
        assert!(
            state.lock().unwrap().model.program_status_authority(),
            "{runtime:?} late create_terminal must preserve hooks-off authority"
        );

        let verify_turn = |session_id: &str, label: &str| {
            let state = core.sessions.session_state(session_id).unwrap();
            let clear_unread = || {
                crate::repo::session_attention::mark_viewed(
                    &core.db.get().unwrap(),
                    session_id,
                    "2026-11-19T00:00:00Z",
                )
                .unwrap();
                state
                    .lock()
                    .unwrap()
                    .model
                    .apply(SessionEvent::Viewed, crate::session::clock::state_now());
            };
            let ingest = |payload: &str| {
                core.sessions.ingest_output_chunk(
                    session_id,
                    None,
                    format!("\x1b]7501;{payload}\x1b\\").as_bytes(),
                    events.as_ref(),
                );
            };

            ingest(&format!("state=working:app={app}"));
            let working = core.sessions.agent_status(session_id);
            assert_eq!(
                working.observation.activity,
                Activity::Working,
                "{runtime:?} {label}"
            );
            assert!(
                working.observation.program_status.is_some(),
                "{runtime:?} {label}"
            );
            assert_eq!(working.unread_since, None, "{runtime:?} {label}");

            for (kind, expected) in [
                ("permission", WaitReason::Approval),
                ("question", WaitReason::Answer),
                ("auth", WaitReason::Unknown),
            ] {
                ingest(&format!("state=blocked:app={app}:kind={kind}"));
                let blocked = core.sessions.agent_status(session_id);
                assert!(
                    blocked.observation.needs_you(),
                    "{runtime:?} {label} {kind}"
                );
                assert_eq!(
                    blocked.observation.interactions[0].reason, expected,
                    "{runtime:?} {label} {kind}"
                );
            }

            ingest(&format!("state=working:app={app}"));
            assert!(
                !core
                    .sessions
                    .agent_status(session_id)
                    .observation
                    .needs_you(),
                "{runtime:?} {label}"
            );

            ingest(&format!("state=done:app={app}"));
            let done = core.sessions.agent_status(session_id);
            assert_eq!(
                done.observation.activity,
                Activity::Idle,
                "{runtime:?} {label}"
            );
            assert_eq!(
                done.observation.outcome,
                Some(TurnOutcome::Completed),
                "{runtime:?} {label}"
            );
            assert!(
                done.unread_since.is_some(),
                "{runtime:?} {label} done must record unread completion"
            );
            assert!(
                crate::repo::session_attention::any_unread(
                    &core.db.get().unwrap(),
                    &[session_id.to_owned()]
                )
                .unwrap(),
                "{runtime:?} {label}"
            );
            clear_unread();

            // Keypress acknowledgement (done -> idle) must not double-record completion.
            ingest(&format!("state=idle:app={app}"));
            assert_eq!(
                core.sessions.agent_status(session_id).unread_since,
                None,
                "{runtime:?} {label}"
            );

            // Working -> error marks Failed without unread completion; Codex, Claude Code,
            // and pi reconcile transcript/OSC errors and retain Failed until the next turn.
            ingest(&format!("state=working:app={app}"));
            ingest(&format!("state=error:app={app}"));
            let failed = core.sessions.agent_status(session_id);
            assert_eq!(
                failed.observation.outcome,
                Some(TurnOutcome::Failed),
                "{runtime:?} {label}"
            );
            assert!(failed.failed_since.is_some(), "{runtime:?} {label}");
            assert_eq!(failed.unread_since, None, "{runtime:?} {label}");
            ingest(&format!("state=idle:app={app}"));
            let after_idle = core.sessions.agent_status(session_id);
            assert_eq!(
                after_idle.observation.outcome,
                Some(TurnOutcome::Failed),
                "{runtime:?} {label} error -> idle"
            );
            assert!(
                after_idle.failed_since.is_some(),
                "{runtime:?} {label} error -> idle failed_since"
            );
            clear_unread();

            // Working -> idle (interrupt) disarms without unread completion.
            ingest(&format!("state=working:app={app}"));
            ingest(&format!("state=idle:app={app}"));
            assert_eq!(
                core.sessions.agent_status(session_id).unread_since,
                None,
                "{runtime:?} {label}"
            );
        };

        verify_turn(&id, "start");

        // Resume with hooks off preserves program-status authority.
        core.sessions.kill(&id).unwrap();
        core.sessions
            .resume(
                &id,
                None,
                None,
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
            )
            .unwrap();
        assert!(
            core.sessions
                .session_state(&id)
                .unwrap()
                .lock()
                .unwrap()
                .model
                .program_status_authority(),
            "{runtime:?} resume with hooks off must grant program-status authority"
        );
        verify_turn(&id, "resume");

        // Fork with hooks off grants program-status authority for direct-fork runtimes
        // (Claude Code and pi) and for Codex's headless-fork launch path (`resume_on_launch`).
        if matches!(runtime, Runtime::ClaudeCode | Runtime::Pi) {
            let key = uuid::Uuid::new_v4().to_string();
            core.db
                .get()
                .unwrap()
                .execute(
                    "UPDATE sessions SET agent_session_key = ?1 WHERE id = ?2",
                    rusqlite::params![key, id],
                )
                .unwrap();
            let forked = core
                .sessions
                .spawn_fork(
                    &id,
                    None,
                    None,
                    None,
                    dir.path(),
                    Arc::clone(&core.db),
                    Arc::clone(&events),
                )
                .unwrap();
            assert!(
                core.sessions
                    .session_state(&forked.id)
                    .unwrap()
                    .lock()
                    .unwrap()
                    .model
                    .program_status_authority(),
                "{runtime:?} fork with hooks off must grant program-status authority"
            );
            verify_turn(&forked.id, "fork");
            core.sessions.kill(&forked.id).unwrap();
        } else {
            let key = uuid::Uuid::new_v4().to_string();
            core.sessions.kill(&id).unwrap();
            core.db
                .get()
                .unwrap()
                .execute(
                    "UPDATE sessions SET agent_session_key = ?1 WHERE id = ?2",
                    rusqlite::params![key, id],
                )
                .unwrap();
            core.sessions
                .resume_on_launch(
                    &id,
                    None,
                    None,
                    dir.path(),
                    Arc::clone(&core.db),
                    Arc::clone(&events),
                )
                .unwrap();
            assert!(
                core.sessions
                    .session_state(&id)
                    .unwrap()
                    .lock()
                    .unwrap()
                    .model
                    .program_status_authority(),
                "{runtime:?} resume_on_launch (fork launch) with hooks off must grant program-status authority"
            );
            verify_turn(&id, "fork-resume-on-launch");
        }

        // Codex, Claude Code, and pi always keep OSC 7501 authority regardless of legacy
        // status_hooks DB rows (true or false).
        core.sessions.kill(&id).unwrap();
        assert!(db::set_runtime_launch_option(
            &core.db,
            runtime,
            RuntimeLaunchOption::StatusHooks,
            true,
        )
        .is_ok());
        core.sessions
            .resume(
                &id,
                None,
                None,
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
            )
            .unwrap();
        assert!(
            core.sessions
                .session_state(&id)
                .unwrap()
                .lock()
                .unwrap()
                .model
                .program_status_authority(),
            "{runtime:?} keeps program-status authority regardless of legacy status_hooks DB rows"
        );
        core.sessions.kill(&id).unwrap();
    }

    // Hooked runtimes that still use status hooks (e.g. Copilot) keep hook precedence over OSC 7501.
    {
        let dir = tempfile::tempdir().unwrap();
        let fake = fake_runtime();
        let core = crate::test_support::test_core_with_runtime(fake);
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        let mut copilot_role = role(Runtime::Copilot.key(), &[]);
        copilot_role.runtime = Runtime::Copilot.key().into();
        copilot_role.command = "fake".into();
        insert_role_row(&core.db.get().unwrap(), &copilot_role);
        let spawned = core
            .sessions
            .spawn_direct(
                &copilot_role,
                None,
                None,
                None,
                None,
                Some(dir.path().to_str().unwrap()),
                None,
                None,
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
                None,
            )
            .unwrap();
        assert!(!core
            .sessions
            .session_state(&spawned.id)
            .unwrap()
            .lock()
            .unwrap()
            .model
            .program_status_authority());
        core.sessions.publish_agent_event(
            &spawned.id,
            AgentEvent::Batch {
                runtime: Runtime::Copilot,
                events: vec![AgentEvent::TurnStarted],
            },
            events.as_ref(),
        );
        core.sessions.ingest_output_chunk(
            &spawned.id,
            None,
            b"\x1b]7501;state=done:app=copilot\x1b\\",
            events.as_ref(),
        );
        let hooked = core.sessions.agent_status(&spawned.id);
        assert_eq!(hooked.observation.source, ObservationSource::Hook);
        assert_eq!(hooked.observation.activity, Activity::Working);
        assert_eq!(hooked.unread_since, None);
        core.sessions.ingest_output_chunk(
            &spawned.id,
            None,
            b"\x1b]7501;state=blocked:app=copilot:kind=permission\x1b\\",
            events.as_ref(),
        );
        assert!(!core
            .sessions
            .agent_status(&spawned.id)
            .observation
            .needs_you());
        core.sessions.kill(&spawned.id).unwrap();
    }

    // Bare shell tabs stay passive: OSC 7501 shows status, never unread completion badges (RT-STATUS-14).
    let dir = tempfile::tempdir().unwrap();
    let fake = fake_runtime();
    let core = crate::test_support::test_core_with_runtime(fake);
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    let mut shell_role = role("shell", &[]);
    shell_role.runtime = Runtime::Shell.key().into();
    shell_role.command = "fake".into();
    let shell = core
        .sessions
        .spawn_runtime_direct(
            &shell_role,
            None,
            Some(dir.path().to_str().unwrap()),
            None,
            None,
            dir.path(),
            Arc::clone(&core.db),
            Arc::clone(&events),
        )
        .unwrap();
    assert!(!core
        .sessions
        .session_state(&shell.id)
        .unwrap()
        .lock()
        .unwrap()
        .model
        .program_status_authority());
    core.sessions.ingest_output_chunk(
        &shell.id,
        None,
        b"\x1b]7501;state=working:app=claude\x07\x1b]7501;state=done:app=claude\x07",
        events.as_ref(),
    );
    let status = core.sessions.agent_status(&shell.id);
    assert_eq!(status.observation.outcome, Some(TurnOutcome::Completed));
    assert_eq!(status.unread_since, None);
    assert!(!crate::repo::session_attention::any_unread(
        &core.db.get().unwrap(),
        std::slice::from_ref(&shell.id),
    )
    .unwrap());
    core.sessions.kill(&shell.id).unwrap();
}

#[test]
fn claude_and_pi_osc7501_chats_and_missions_cover_waits_outcomes_cancel_children_and_no_osc_fallback(
) {
    use crate::session::state::agent::AgentEvent as A;
    use crate::session::status::WaitReason;
    use runner_core::protocol::runtime::RuntimeLaunchOption;

    for (runtime, app) in [(Runtime::ClaudeCode, "claude-code"), (Runtime::Pi, "pi")] {
        for mission in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let fake = fake_runtime();
            let core = crate::test_support::test_core_with_runtime(fake);
            let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());

            // Legacy status_hooks=false DB rows must be ignored in both chats and missions.
            db::set_runtime_launch_option(
                &core.db,
                runtime,
                RuntimeLaunchOption::StatusHooks,
                false,
            )
            .unwrap();

            let mut agent_role = role(runtime.key(), &[]);
            agent_role.runtime = runtime.key().into();
            agent_role.command = "fake".into();

            let (id, mission_id) = if mission {
                let mut m = crate::session::manager::tests::mission();
                m.crew_id = Some("c".into());
                let slot_id = insert_crew_role(&core.db, &m.id, &agent_role.id);
                let mut slot = crate::session::manager::tests::slot_for(&agent_role);
                slot.id = slot_id;
                slot.slot_handle = runtime.key().into();
                let pending = core
                    .sessions
                    .register_mission_session(
                        &m,
                        &agent_role,
                        &slot,
                        dir.path(),
                        dir.path().join("events.ndjson"),
                        Arc::clone(&core.db),
                        None,
                        None,
                        None,
                        "goal",
                    )
                    .unwrap();
                let session_id = pending.session_id.clone();
                core.sessions
                    .complete_mission_session_spawn(
                        pending,
                        Arc::clone(&events),
                        Arc::new(AtomicBool::new(false)),
                    )
                    .unwrap();
                let terminal = core
                    .sessions
                    .create_terminal(&session_id, 80, 24, &core.db, &events)
                    .unwrap();
                core.sessions
                    .session_state(&session_id)
                    .unwrap()
                    .lock()
                    .unwrap()
                    .terminal = Some(terminal);
                (session_id, Some(m.id))
            } else {
                insert_role_row(&core.db.get().unwrap(), &agent_role);
                let spawned = core
                    .sessions
                    .spawn_direct(
                        &agent_role,
                        None,
                        None,
                        None,
                        None,
                        Some(dir.path().to_str().unwrap()),
                        None,
                        None,
                        dir.path(),
                        Arc::clone(&core.db),
                        Arc::clone(&events),
                        None,
                    )
                    .unwrap();
                let terminal = core
                    .sessions
                    .create_terminal(&spawned.id, 80, 24, &core.db, &events)
                    .unwrap();
                core.sessions
                    .session_state(&spawned.id)
                    .unwrap()
                    .lock()
                    .unwrap()
                    .terminal = Some(terminal);
                (spawned.id, None)
            };

            let state = core.sessions.session_state(&id).unwrap();
            assert!(
                state.lock().unwrap().model.program_status_authority(),
                "{runtime:?} mission={mission} must grant OSC 7501 authority"
            );

            let ingest = |seq: &str| {
                core.sessions.ingest_output_chunk(
                    &id,
                    mission_id.as_deref(),
                    format!("\x1b]7501;{seq}\x1b\\").as_bytes(),
                    events.as_ref(),
                );
            };
            let clear_unread = || {
                crate::repo::session_attention::mark_viewed(
                    &core.db.get().unwrap(),
                    &id,
                    "2026-11-20T00:00:00Z",
                )
                .unwrap();
                state
                    .lock()
                    .unwrap()
                    .model
                    .apply(SessionEvent::Viewed, crate::session::clock::state_now());
            };

            // Release startup gate with OSC 7501 idle.
            ingest(&format!("state=idle:app={app}"));
            assert_eq!(
                core.sessions.delivery_readiness(&id),
                router::DeliveryReadiness::Ready { idle: true },
                "{runtime:?} mission={mission}"
            );

            // 1. ProgramTurnConfirmed pairs with Working without double-counting, and a
            // mid-turn ProgramTurnConfirmed (mission steer) counts as a second turn start.
            let before_turns = core.sessions.delivery_snapshot(&id).unwrap().turn_started;
            core.sessions.publish_agent_event(
                &id,
                A::ProgramTurnConfirmed {
                    conversation_changed: false,
                    prompt: Some("task".into()),
                },
                events.as_ref(),
            );
            assert_eq!(
                core.sessions.delivery_snapshot(&id).unwrap().turn_started,
                before_turns + 1,
                "{runtime:?} mission={mission}"
            );
            ingest(&format!("state=working:app={app}"));
            assert_eq!(
                core.sessions.delivery_snapshot(&id).unwrap().turn_started,
                before_turns + 1,
                "{runtime:?} mission={mission}"
            );
            core.sessions.publish_agent_event(
                &id,
                A::ProgramTurnConfirmed {
                    conversation_changed: false,
                    prompt: Some("steer".into()),
                },
                events.as_ref(),
            );
            assert_eq!(
                core.sessions.delivery_snapshot(&id).unwrap().turn_started,
                before_turns + 2,
                "{runtime:?} mission={mission}"
            );

            // 2. Every blocked kind (permission, question, auth, absent) holds delivery
            // with the right WaitReason, and foreign root reports cannot release it.
            for (kind, expected) in [
                (Some("permission"), WaitReason::Approval),
                (Some("question"), WaitReason::Answer),
                (Some("auth"), WaitReason::Unknown),
                (None, WaitReason::Unknown),
            ] {
                let seq = match kind {
                    Some(k) => format!("state=blocked:app={app}:kind={k}"),
                    None => format!("state=blocked:app={app}"),
                };
                ingest(&seq);
                let status = core.sessions.agent_status(&id);
                assert!(status.observation.needs_you(), "{runtime:?} {kind:?}");
                assert_eq!(
                    status.observation.interactions[0].reason, expected,
                    "{runtime:?} {kind:?}"
                );
                assert_eq!(
                    core.sessions.delivery_readiness(&id),
                    router::DeliveryReadiness::NeedsYou,
                    "{runtime:?} {kind:?}"
                );
                assert_eq!(
                    core.sessions.reserve_delivery(&id).unwrap(),
                    router::DeliveryReservation::HumanInteraction,
                    "{runtime:?} {kind:?}"
                );
                // Foreign root idle cannot release this runtime's wait.
                ingest("state=idle:app=other");
                assert!(
                    core.sessions.agent_status(&id).observation.needs_you(),
                    "{runtime:?} foreign idle must not release wait"
                );
            }

            // 2. Child records (Claude Code id= background tasks/subagents) only add detail text
            // and never change root Working, Done, unread badges or crew delivery.
            ingest(&format!("state=working:app={app}"));
            assert!(!core.sessions.agent_status(&id).observation.needs_you());
            ingest(&format!("id=bg-1:app={app}:state=working"));
            ingest(&format!("id=bg-2:app={app}:state=blocked:kind=permission"));
            let with_children = core.sessions.agent_status(&id);
            assert_eq!(with_children.observation.activity, Activity::Working);
            assert!(!with_children.observation.needs_you());
            assert_eq!(
                with_children
                    .observation
                    .program_status
                    .as_ref()
                    .unwrap()
                    .background_tasks,
                2,
                "{runtime:?} mission={mission}"
            );

            // Root done records Completed + unread badge even with active children, and delivery opens.
            ingest(&format!("state=done:app={app}"));
            let done = core.sessions.agent_status(&id);
            assert_eq!(done.observation.outcome, Some(TurnOutcome::Completed));
            assert!(done.unread_since.is_some(), "{runtime:?} mission={mission}");
            assert_eq!(
                core.sessions.delivery_readiness(&id),
                router::DeliveryReadiness::Ready { idle: true }
            );
            ingest("state=clear:id=bg-1");
            ingest("state=clear:id=bg-2");
            assert_eq!(
                core.sessions
                    .agent_status(&id)
                    .observation
                    .program_status
                    .as_ref()
                    .unwrap()
                    .background_tasks,
                0
            );
            clear_unread();

            // 3. Esc mid-turn produces Interrupted with no unread badge; Kitty Esc works too.
            for esc in [b"\x1b".as_slice(), b"\x1b[27u"] {
                ingest(&format!("state=working:app={app}"));
                core.sessions
                    .inject_direct_stdin(&id, esc, events.as_ref())
                    .unwrap();
                ingest(&format!("state=idle:app={app}"));
                let interrupted = core.sessions.agent_status(&id);
                assert_eq!(
                    interrupted.observation.outcome,
                    Some(TurnOutcome::Interrupted),
                    "{runtime:?} esc={esc:?}"
                );
                assert_eq!(interrupted.unread_since, None, "{runtime:?} esc={esc:?}");
                assert_eq!(interrupted.failed_since, None, "{runtime:?} esc={esc:?}");
            }

            // 4. Error produces Failed with failed_since and no unread badge, surviving idle.
            ingest(&format!("state=working:app={app}"));
            ingest(&format!("state=error:app={app}"));
            ingest(&format!("state=idle:app={app}"));
            let failed = core.sessions.agent_status(&id);
            assert_eq!(failed.observation.outcome, Some(TurnOutcome::Failed));
            assert!(failed.failed_since.is_some());
            assert_eq!(failed.unread_since, None);

            core.sessions.kill(&id).unwrap();
        }
    }

    // 6. No-OSC fallback (CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1 / PI_PROGRAM_STATUS=0):
    // untrusted direct chat holds quiet trust prompt until SessionStart/session_start, then
    // settles via terminal activity; first turn releases on ProgramTurnFinished.
    for runtime in [Runtime::ClaudeCode, Runtime::Pi] {
        let dir = tempfile::tempdir().unwrap();
        let fake = fake_runtime();
        let core = crate::test_support::test_core_with_runtime(fake);
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        let mut agent_role = role(runtime.key(), &[]);
        agent_role.runtime = runtime.key().into();
        agent_role.command = "fake".into();
        insert_role_row(&core.db.get().unwrap(), &agent_role);

        let spawned = core
            .sessions
            .spawn_direct(
                &agent_role,
                None,
                None,
                None,
                None,
                Some(dir.path().to_str().unwrap()),
                None,
                None,
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
                None,
            )
            .unwrap();
        let id = spawned.id;
        let stop = core
            .sessions
            .session_state(&id)
            .unwrap()
            .lock()
            .unwrap()
            .handle
            .as_ref()
            .unwrap()
            .stop
            .clone();
        let now = Instant::now();
        core.sessions.note_forwarder_transition(
            &id,
            SessionActivityState::Idle,
            StatusSource::Forwarder,
        );
        core.sessions
            .startup_output(&id, &stop, b"Trust this folder?", now);
        core.sessions
            .release_settled_startup(&id, &stop, now + Duration::from_secs(60));
        assert_eq!(
            core.sessions.report_readiness(&id),
            router::DeliveryReadiness::Starting,
            "{runtime:?}: no-OSC quiet startup must wait for SessionStart/session_start"
        );
        core.sessions.publish_agent_event(
            &id,
            A::ProgramSessionStarted {
                conversation_changed: false,
                compacted: false,
            },
            events.as_ref(),
        );
        core.sessions
            .startup_output(&id, &stop, b"> prompt", now + Duration::from_secs(61));
        core.sessions
            .release_settled_startup(&id, &stop, now + Duration::from_secs(75));
        assert_eq!(
            core.sessions.report_readiness(&id),
            router::DeliveryReadiness::Ready { idle: true },
            "{runtime:?}: no-OSC chat falls back to terminal output settle after identity hook"
        );
        core.sessions.note_forwarder_transition(
            &id,
            SessionActivityState::Busy,
            StatusSource::Forwarder,
        );
        assert_eq!(
            core.sessions.agent_status(&id).observation.activity,
            Activity::Working
        );
        core.sessions.note_forwarder_transition(
            &id,
            SessionActivityState::Idle,
            StatusSource::Forwarder,
        );
        assert_eq!(
            core.sessions.agent_status(&id).observation.activity,
            Activity::Idle
        );
        core.sessions.kill(&id).unwrap();
    }
}

#[test]
fn pi_retry_claude_no_osc_first_turn_failure_and_pi_untrusted_bridge_unavailable() {
    use crate::session::state::agent::{AgentEvent as A, ProgramOutcomePolicy};

    // 1. pi pending first turn: transient 429 retry transcript error during Working must not
    // release first-turn delivery early or disarm the unread Completed badge on retry success.
    {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::test_support::test_core_with_runtime(fake_runtime());
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        let mut pi_role = role("pi", &[]);
        pi_role.runtime = "pi".into();
        pi_role.command = "fake".into();
        insert_role_row(&core.db.get().unwrap(), &pi_role);

        let spawned = core
            .sessions
            .spawn_direct(
                &pi_role,
                None,
                None,
                None,
                None,
                Some(dir.path().to_str().unwrap()),
                None,
                None,
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
                Some("first turn".into()),
            )
            .unwrap();
        let id = spawned.id;
        let terminal = core
            .sessions
            .create_terminal(&id, 80, 24, &core.db, &events)
            .unwrap();
        core.sessions
            .session_state(&id)
            .unwrap()
            .lock()
            .unwrap()
            .terminal = Some(terminal);

        core.sessions.ingest_output_chunk(
            &id,
            None,
            b"\x1b]7501;state=working:app=pi\x1b\\",
            events.as_ref(),
        );
        assert_eq!(
            core.sessions.report_readiness(&id),
            router::DeliveryReadiness::Starting
        );
        core.sessions.publish_agent_event(
            &id,
            A::ProgramTurnOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::NativeFallbackOnly,
            },
            events.as_ref(),
        );
        assert_eq!(
            core.sessions.agent_status(&id).observation.activity,
            Activity::Working,
            "transient retry error must not override Working root"
        );
        assert_eq!(
            core.sessions.report_readiness(&id),
            router::DeliveryReadiness::Starting,
            "transient retry error must not release first-turn delivery"
        );
        // Retry succeeds straight from Working to Done with no intervening Working.
        core.sessions.ingest_output_chunk(
            &id,
            None,
            b"\x1b]7501;state=done:app=pi\x1b\\",
            events.as_ref(),
        );
        let done = core.sessions.agent_status(&id);
        assert_eq!(done.observation.outcome, Some(TurnOutcome::Completed));
        assert!(
            done.unread_since.is_some(),
            "successful retry must still award the unread Completed badge"
        );
        assert_eq!(
            core.sessions.report_readiness(&id),
            router::DeliveryReadiness::Ready { idle: true }
        );
        core.sessions.kill(&id).unwrap();
    }

    // 2. Claude Code pending first turn with CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1 (no OSC,
    // no turn_duration): isApiErrorMessage releases startup delivery as Failed, while native
    // blocked:auth stays authoritative when OSC 7501 is active.
    {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::test_support::test_core_with_runtime(fake_runtime());
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        let mut cc_role = role("claude-code", &[]);
        cc_role.runtime = "claude-code".into();
        cc_role.command = "fake".into();
        insert_role_row(&core.db.get().unwrap(), &cc_role);

        let spawned = core
            .sessions
            .spawn_direct(
                &cc_role,
                None,
                None,
                None,
                None,
                Some(dir.path().to_str().unwrap()),
                None,
                None,
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
                Some("first turn".into()),
            )
            .unwrap();
        let id = spawned.id;
        let stop = core
            .sessions
            .session_state(&id)
            .unwrap()
            .lock()
            .unwrap()
            .handle
            .as_ref()
            .unwrap()
            .stop
            .clone();
        let now = Instant::now();
        core.sessions.publish_agent_event(
            &id,
            A::ProgramSessionStarted {
                conversation_changed: false,
                compacted: false,
            },
            events.as_ref(),
        );
        core.sessions.startup_output(&id, &stop, b"API Error", now);
        core.sessions
            .release_settled_startup(&id, &stop, now + Duration::from_secs(60));
        assert_eq!(
            core.sessions.report_readiness(&id),
            router::DeliveryReadiness::Starting,
            "pending first turn without OSC stays Starting until terminal error or finish"
        );
        core.sessions.publish_agent_event(
            &id,
            A::ProgramTurnOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::NativeFallbackOnly,
            },
            events.as_ref(),
        );
        let failed = core.sessions.agent_status(&id);
        assert_eq!(failed.observation.outcome, Some(TurnOutcome::Failed));
        assert!(failed.failed_since.is_some());
        assert!(failed.observation.program_status.is_none());
        assert_eq!(
            core.sessions.report_readiness(&id),
            router::DeliveryReadiness::Ready { idle: true },
            "terminal isApiErrorMessage without OSC or turn_duration must release first-turn delivery"
        );
        core.sessions.kill(&id).unwrap();

        // With live OSC 7501 blocked:auth, isApiErrorMessage keeps the auth wait authoritative.
        let authed = core
            .sessions
            .spawn_direct(
                &cc_role,
                None,
                None,
                None,
                None,
                Some(dir.path().to_str().unwrap()),
                None,
                None,
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
                None,
            )
            .unwrap();
        let auth_id = authed.id;
        let terminal = core
            .sessions
            .create_terminal(&auth_id, 80, 24, &core.db, &events)
            .unwrap();
        core.sessions
            .session_state(&auth_id)
            .unwrap()
            .lock()
            .unwrap()
            .terminal = Some(terminal);
        core.sessions.ingest_output_chunk(
            &auth_id,
            None,
            b"\x1b]7501;state=blocked:app=claude-code:kind=auth\x1b\\",
            events.as_ref(),
        );
        core.sessions.publish_agent_event(
            &auth_id,
            A::ProgramTurnOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::NativeFallbackOnly,
            },
            events.as_ref(),
        );
        assert!(core.sessions.agent_status(&auth_id).observation.needs_you());
        assert_eq!(
            core.sessions.delivery_readiness(&auth_id),
            router::DeliveryReadiness::NeedsYou
        );
        core.sessions.kill(&auth_id).unwrap();
    }

    // 3. Attended pi without real trust flags defaults to untrusted: ProgramIdentityUnavailable
    // before session_start and before OSC 7501 must NOT allow quiet output to release a
    // report into pi's trust prompt, including when `--approve` or `-a` appears after `--` or as
    // an option value; explicit parsed trust flags allow fallback.
    for (args, approved) in [
        (&[][..], false),
        (&["--", "--approve"][..], false),
        (&["--name", "-a"][..], false),
        (&["--approve"][..], true),
        (&["-a"][..], true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::test_support::test_core_with_runtime(fake_runtime());
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        let mut pi_role = role("pi", args);
        pi_role.runtime = "pi".into();
        pi_role.command = "fake".into();
        insert_role_row(&core.db.get().unwrap(), &pi_role);

        let spawned = core
            .sessions
            .spawn_direct(
                &pi_role,
                None,
                None,
                None,
                None,
                Some(dir.path().to_str().unwrap()),
                None,
                None,
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
                None,
            )
            .unwrap();
        let id = spawned.id;
        let stop = core
            .sessions
            .session_state(&id)
            .unwrap()
            .lock()
            .unwrap()
            .handle
            .as_ref()
            .unwrap()
            .stop
            .clone();
        let now = Instant::now();
        core.sessions.note_forwarder_transition(
            &id,
            SessionActivityState::Idle,
            StatusSource::Forwarder,
        );
        core.sessions
            .publish_agent_event(&id, A::ProgramIdentityUnavailable, events.as_ref());
        core.sessions
            .startup_output(&id, &stop, b"Trust project?", now);
        core.sessions
            .release_settled_startup(&id, &stop, now + Duration::from_secs(60));
        let expected = if approved {
            router::DeliveryReadiness::Ready { idle: true }
        } else {
            router::DeliveryReadiness::Starting
        };
        assert_eq!(
            core.sessions.report_readiness(&id),
            expected,
            "pi args={args:?} approved={approved}: bridge unavailable before session_start"
        );
        core.sessions.kill(&id).unwrap();
    }
}

#[test]
fn pi_custom_prompt_overlay_holds_and_releases_mission_delivery_with_dedup_and_shutdown() {
    use crate::session::state::agent::AgentEvent as A;
    use crate::session::status::WaitReason;

    let dir = tempfile::tempdir().unwrap();
    let core = crate::test_support::test_core_with_runtime(fake_runtime());
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    let mut pi_role = role("pi", &[]);
    pi_role.runtime = "pi".into();
    pi_role.command = "fake".into();

    let mut m = crate::session::manager::tests::mission();
    m.crew_id = Some("c".into());
    let slot_id = insert_crew_role(&core.db, &m.id, &pi_role.id);
    let mut slot = crate::session::manager::tests::slot_for(&pi_role);
    slot.id = slot_id;
    slot.slot_handle = "pi".into();

    let pending = core
        .sessions
        .register_mission_session(
            &m,
            &pi_role,
            &slot,
            dir.path(),
            dir.path().join("events.ndjson"),
            Arc::clone(&core.db),
            None,
            None,
            None,
            "goal",
        )
        .unwrap();
    let id = pending.session_id.clone();
    core.sessions
        .complete_mission_session_spawn(
            pending,
            Arc::clone(&events),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    let terminal = core
        .sessions
        .create_terminal(&id, 80, 24, &core.db, &events)
        .unwrap();
    core.sessions
        .session_state(&id)
        .unwrap()
        .lock()
        .unwrap()
        .terminal = Some(terminal);

    let ingest = |seq: &str| {
        core.sessions.ingest_output_chunk(
            &id,
            Some(&m.id),
            format!("\x1b]7501;{seq}\x1b\\").as_bytes(),
            events.as_ref(),
        );
    };

    // Release startup gate with OSC 7501 idle.
    ingest("state=idle:app=pi");
    assert_eq!(
        core.sessions.delivery_readiness(&id),
        router::DeliveryReadiness::Ready { idle: true }
    );

    // 1. Unannotated custom prompt shows Needs you (Unknown) and holds crew delivery
    // until ui_prompt_end releases it once.
    core.sessions
        .publish_agent_event(&id, A::ProgramWaitOverlay { active: true }, events.as_ref());
    let waiting = core.sessions.agent_status(&id);
    assert_eq!(waiting.observation.interactions.len(), 1);
    assert_eq!(
        waiting.observation.interactions[0].reason,
        WaitReason::Unknown
    );
    assert_eq!(
        core.sessions.delivery_readiness(&id),
        router::DeliveryReadiness::NeedsYou
    );
    assert_eq!(
        core.sessions.reserve_delivery(&id).unwrap(),
        router::DeliveryReservation::HumanInteraction
    );

    core.sessions.publish_agent_event(
        &id,
        A::ProgramWaitOverlay { active: false },
        events.as_ref(),
    );
    assert!(!core.sessions.agent_status(&id).observation.needs_you());
    assert_eq!(
        core.sessions.delivery_readiness(&id),
        router::DeliveryReadiness::Ready { idle: true }
    );
    let router::DeliveryReservation::Ready(token) = core.sessions.reserve_delivery(&id).unwrap()
    else {
        panic!("expected Ready after unannotated custom prompt closes");
    };
    core.sessions.finish_delivery(&id, token);

    // 2. session_shutdown (routed through PiStatusWatcher) and conversation switch also clear
    // an active custom prompt overlay.
    let (hooks, receiver) =
        crate::session::hook_queue::TestHookRoute::new(Runtime::Pi, "current".into());
    let mut watcher = crate::runtimes::pi::pi_status::PiStatusWatcher::from_receiver(receiver);
    hooks
        .admit(serde_json::json!({
            "generation": "current",
            "hook_event_name": "ui_prompt_start",
            "kind": "custom",
        }))
        .unwrap();
    watcher
        .drain_events(
            0,
            |event| {
                core.sessions
                    .publish_agent_event(&id, event, events.as_ref())
            },
            |_| {},
        )
        .unwrap();
    assert_eq!(
        core.sessions.delivery_readiness(&id),
        router::DeliveryReadiness::NeedsYou
    );
    hooks
        .admit(serde_json::json!({
            "generation": "current",
            "hook_event_name": "session_shutdown",
            "reason": "quit",
        }))
        .unwrap();
    watcher
        .drain_events(
            0,
            |event| {
                core.sessions
                    .publish_agent_event(&id, event, events.as_ref())
            },
            |_| {},
        )
        .unwrap();
    assert_eq!(
        core.sessions.delivery_readiness(&id),
        router::DeliveryReadiness::Ready { idle: true }
    );

    // 3. Annotated custom prompt deduplicates native OSC blocked + overlay to one wait
    // and keeps OSC authoritative for Working -> Done + unread badge.
    ingest("state=working:app=pi");
    ingest("state=blocked:app=pi:kind=question");
    core.sessions
        .publish_agent_event(&id, A::ProgramWaitOverlay { active: true }, events.as_ref());
    let deduped = core.sessions.agent_status(&id);
    assert_eq!(deduped.observation.interactions.len(), 1);
    assert_eq!(
        deduped.observation.interactions[0].reason,
        WaitReason::Answer
    );
    assert_eq!(
        core.sessions.delivery_readiness(&id),
        router::DeliveryReadiness::NeedsYou
    );
    ingest("state=working:app=pi");
    // Still held until ui_prompt_end arrives; then releases cleanly while Working continues.
    assert_eq!(
        core.sessions.delivery_readiness(&id),
        router::DeliveryReadiness::NeedsYou
    );
    core.sessions.publish_agent_event(
        &id,
        A::ProgramWaitOverlay { active: false },
        events.as_ref(),
    );
    assert!(!core.sessions.agent_status(&id).observation.needs_you());
    assert_eq!(
        core.sessions.agent_status(&id).observation.activity,
        Activity::Working
    );
    ingest("state=done:app=pi");
    let done = core.sessions.agent_status(&id);
    assert_eq!(done.observation.outcome, Some(TurnOutcome::Completed));
    assert!(done.unread_since.is_some());
    assert_eq!(
        core.sessions.delivery_readiness(&id),
        router::DeliveryReadiness::Ready { idle: true }
    );

    // 4. Stacked confirm -> unannotated custom: confirm emits ui_prompt_start(confirm) +
    // OSC blocked(permission); custom drops OSC to Working while both stay open; delivery
    // stays held until both close and releases once.
    hooks
        .admit(serde_json::json!({
            "generation": "current",
            "hook_event_name": "ui_prompt_start",
            "kind": "confirm",
        }))
        .unwrap();
    watcher
        .drain_events(
            0,
            |event| {
                core.sessions
                    .publish_agent_event(&id, event, events.as_ref())
            },
            |_| {},
        )
        .unwrap();
    ingest("state=blocked:app=pi:kind=permission");
    assert_eq!(
        core.sessions
            .agent_status(&id)
            .observation
            .interactions
            .len(),
        1
    );
    assert_eq!(
        core.sessions.agent_status(&id).observation.interactions[0].reason,
        WaitReason::Approval
    );
    // Unannotated custom opens on top -> OSC drops blocked back to Working.
    ingest("state=working:app=pi");
    assert_eq!(
        core.sessions.delivery_readiness(&id),
        router::DeliveryReadiness::NeedsYou,
        "stacked unannotated custom dialog must retain delivery hold"
    );
    assert_eq!(
        core.sessions.agent_status(&id).observation.interactions[0].reason,
        WaitReason::Unknown
    );
    // Custom closes -> confirm returns, then confirm closes -> ui_prompt_end releases once.
    ingest("state=blocked:app=pi:kind=permission");
    ingest("state=idle:app=pi");
    assert_eq!(
        core.sessions.delivery_readiness(&id),
        router::DeliveryReadiness::NeedsYou
    );
    hooks
        .admit(serde_json::json!({
            "generation": "current",
            "hook_event_name": "ui_prompt_end",
            "kind": "confirm",
        }))
        .unwrap();
    watcher
        .drain_events(
            0,
            |event| {
                core.sessions
                    .publish_agent_event(&id, event, events.as_ref())
            },
            |_| {},
        )
        .unwrap();
    assert_eq!(
        core.sessions.delivery_readiness(&id),
        router::DeliveryReadiness::Ready { idle: true }
    );

    // 5. Completion attention with overlay:
    // (a) After a viewed Done, opening an unannotated custom dialog, dismissing with Enter
    // (which re-arms completion), and closing it must NOT record a second unread completion.
    ingest("state=working:app=pi");
    ingest("state=done:app=pi");
    assert!(core.sessions.agent_status(&id).unread_since.is_some());
    crate::repo::session_attention::mark_viewed(
        &core.db.get().unwrap(),
        &id,
        "2026-11-20T00:00:00Z",
    )
    .unwrap();
    core.sessions
        .session_state(&id)
        .unwrap()
        .lock()
        .unwrap()
        .model
        .apply(SessionEvent::Viewed, crate::session::clock::state_now());
    assert_eq!(core.sessions.agent_status(&id).unread_since, None);
    assert!(!crate::repo::session_attention::any_unread(
        &core.db.get().unwrap(),
        std::slice::from_ref(&id),
    )
    .unwrap());

    core.sessions
        .publish_agent_event(&id, A::ProgramWaitOverlay { active: true }, events.as_ref());
    core.sessions
        .inject_direct_stdin(&id, b"\r", events.as_ref())
        .unwrap();
    core.sessions.publish_agent_event(
        &id,
        A::ProgramWaitOverlay { active: false },
        events.as_ref(),
    );
    assert_eq!(
        core.sessions.agent_status(&id).unread_since,
        None,
        "dismissing custom dialog with Enter over a viewed Done must not record a second badge"
    );
    assert!(!crate::repo::session_attention::any_unread(
        &core.db.get().unwrap(),
        std::slice::from_ref(&id),
    )
    .unwrap());
    // Clear the synthetic Enter timestamp so section 6 asserts wait release without a 2s typing window.
    core.sessions
        .session_state(&id)
        .unwrap()
        .lock()
        .unwrap()
        .model
        .apply(
            SessionEvent::TestInputAt(None),
            crate::session::clock::state_now(),
        );

    // (b) A real root Done arriving while a custom wait is open must record the unread badge
    // immediately, even if OSC transitions to Idle before the custom dialog closes.
    ingest("state=working:app=pi");
    core.sessions
        .publish_agent_event(&id, A::ProgramWaitOverlay { active: true }, events.as_ref());
    ingest("state=done:app=pi");
    assert!(
        core.sessions.agent_status(&id).unread_since.is_some(),
        "real root Done during custom wait must record unread badge immediately"
    );
    assert!(crate::repo::session_attention::any_unread(
        &core.db.get().unwrap(),
        std::slice::from_ref(&id),
    )
    .unwrap());
    ingest("state=idle:app=pi");
    core.sessions.publish_agent_event(
        &id,
        A::ProgramWaitOverlay { active: false },
        events.as_ref(),
    );
    assert!(
        core.sessions.agent_status(&id).unread_since.is_some(),
        "unread badge must survive Idle -> overlay close"
    );

    // 6. OSC state=clear during an active custom wait preserves the wait across suspend/resume
    // until ui_prompt_end arrives.
    core.sessions
        .publish_agent_event(&id, A::ProgramWaitOverlay { active: true }, events.as_ref());
    ingest("state=clear");
    assert!(core
        .sessions
        .agent_status(&id)
        .observation
        .program_status
        .is_none());
    assert_eq!(
        core.sessions.delivery_readiness(&id),
        router::DeliveryReadiness::NeedsYou
    );
    ingest("state=idle:app=pi");
    assert_eq!(
        core.sessions.delivery_readiness(&id),
        router::DeliveryReadiness::NeedsYou
    );
    core.sessions.publish_agent_event(
        &id,
        A::ProgramWaitOverlay { active: false },
        events.as_ref(),
    );
    assert_eq!(
        core.sessions.delivery_readiness(&id),
        router::DeliveryReadiness::Ready { idle: true }
    );

    core.sessions.kill(&id).unwrap();
}
