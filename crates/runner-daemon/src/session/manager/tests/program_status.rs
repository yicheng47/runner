use super::*;
use crate::session::hook_feed::HookWatcher;

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
                DirectLaunch {
                    cwd: Some(dir.path().to_str().unwrap().into()),
                    ..Default::default()
                },
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
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
                DirectLaunch {
                    cwd: Some(dir.path().to_str().unwrap().into()),
                    ..Default::default()
                },
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
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
        .spawn_direct(
            &shell_role,
            DirectLaunch {
                cwd: Some(dir.path().to_str().unwrap().into()),
                runtime_direct: true,
                ..Default::default()
            },
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
                        DirectLaunch {
                            cwd: Some(dir.path().to_str().unwrap().into()),
                            ..Default::default()
                        },
                        dir.path(),
                        Arc::clone(&core.db),
                        Arc::clone(&events),
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
                DirectLaunch {
                    cwd: Some(dir.path().to_str().unwrap().into()),
                    ..Default::default()
                },
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
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
                DirectLaunch {
                    cwd: Some(dir.path().to_str().unwrap().into()),
                    first_turn: Some("first turn".into()),
                    ..Default::default()
                },
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
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
                DirectLaunch {
                    cwd: Some(dir.path().to_str().unwrap().into()),
                    first_turn: Some("first turn".into()),
                    ..Default::default()
                },
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
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
                DirectLaunch {
                    cwd: Some(dir.path().to_str().unwrap().into()),
                    ..Default::default()
                },
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
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
                DirectLaunch {
                    cwd: Some(dir.path().to_str().unwrap().into()),
                    ..Default::default()
                },
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
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

#[test]
fn grok_and_opencode_osc7501_chats_and_missions_cover_waits_delivery_outcomes_cancel_children_legacy_switch_and_fallback(
) {
    use crate::session::state::agent::{AgentEvent as A, ProgramOutcomePolicy};
    use crate::session::status::WaitReason;
    use runner_core::protocol::runtime::RuntimeLaunchOption;

    for (runtime, app) in [(Runtime::Grok, "grok"), (Runtime::OpenCode, "opencode")] {
        for mission in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let core = crate::test_support::test_core_with_runtime(fake_runtime());
            let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());

            // Legacy status_hooks = false DB rows are ignored for both Grok and OpenCode.
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
                        DirectLaunch {
                            cwd: Some(dir.path().to_str().unwrap().into()),
                            ..Default::default()
                        },
                        dir.path(),
                        Arc::clone(&core.db),
                        Arc::clone(&events),
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
                "{runtime:?} mission={mission}: OSC 7501 authority must be active"
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
                    "2026-11-21T00:00:00Z",
                )
                .unwrap();
                state
                    .lock()
                    .unwrap()
                    .model
                    .apply(SessionEvent::Viewed, crate::session::clock::state_now());
            };

            // Release startup gate with OSC 7501 idle without fabricating Completed.
            ingest(&format!("state=idle:app={app}"));
            assert_eq!(
                core.sessions.delivery_readiness(&id),
                router::DeliveryReadiness::Ready { idle: true },
                "{runtime:?} mission={mission}"
            );
            assert_eq!(core.sessions.agent_status(&id).unread_since, None);

            // 1. Every blocked kind holds crew delivery and releases when cleared.
            ingest(&format!("state=working:app={app}"));
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
                assert!(core.sessions.agent_status(&id).observation.needs_you());
            }

            // 2. Child records never alter root status/delivery/badge. Grok exposes subagent
            // background_tasks detail; OpenCode suppresses per-session child counts.
            ingest(&format!("state=working:app={app}"));
            assert!(!core.sessions.agent_status(&id).observation.needs_you());
            ingest(&format!("id=child-1:app={app}:state=working"));
            ingest(&format!(
                "id=child-2:app={app}:state=blocked:kind=permission"
            ));
            let with_children = core.sessions.agent_status(&id);
            assert_eq!(with_children.observation.activity, Activity::Working);
            assert!(!with_children.observation.needs_you());
            let expected_bg = if runtime == Runtime::Grok { 2 } else { 0 };
            assert_eq!(
                with_children
                    .observation
                    .program_status
                    .as_ref()
                    .unwrap()
                    .background_tasks,
                expected_bg,
                "{runtime:?} mission={mission}"
            );

            // 3. Completion: Grok on `done`; OpenCode on root `working` -> `idle` (with children clearing).
            if runtime == Runtime::Grok {
                ingest("state=done:app=grok:msg=VHVybiBjb21wbGV0ZSBpbiAyLjVzLg");
            } else {
                ingest("state=idle:app=opencode");
                ingest("state=clear:id=child-1");
                ingest("state=clear:id=child-2");
            }
            let done = core.sessions.agent_status(&id);
            assert_eq!(
                done.observation.outcome,
                Some(TurnOutcome::Completed),
                "{runtime:?} mission={mission}"
            );
            assert!(
                done.unread_since.is_some(),
                "{runtime:?} mission={mission} must record unread badge"
            );
            assert!(
                crate::repo::session_attention::any_unread(
                    &core.db.get().unwrap(),
                    std::slice::from_ref(&id),
                )
                .unwrap(),
                "{runtime:?} mission={mission}"
            );
            assert_eq!(
                core.sessions.delivery_readiness(&id),
                router::DeliveryReadiness::Ready { idle: true }
            );
            if runtime == Runtime::Grok {
                ingest("state=clear:id=child-1");
                ingest("state=clear:id=child-2");
            }
            clear_unread();

            // 4. Cancel key mid-turn (Ctrl+C and Esc, including Grok Working("Cancelling") -> Idle)
            // settles Interrupted with no unread badge.
            for cancel_bytes in [b"\x03".as_slice(), b"\x1b".as_slice()] {
                ingest(&format!("state=working:app={app}"));
                core.sessions
                    .inject_direct_stdin(&id, cancel_bytes, events.as_ref())
                    .unwrap();
                if runtime == Runtime::Grok {
                    ingest("state=working:app=grok:msg=Q2FuY2VsbGluZw");
                }
                ingest(&format!("state=idle:app={app}"));
                let interrupted = core.sessions.agent_status(&id);
                assert_eq!(
                    interrupted.observation.outcome,
                    Some(TurnOutcome::Interrupted),
                    "{runtime:?} cancel={cancel_bytes:?}"
                );
                assert_eq!(interrupted.unread_since, None);
                assert_eq!(interrupted.failed_since, None);
                assert!(!crate::repo::session_attention::any_unread(
                    &core.db.get().unwrap(),
                    std::slice::from_ref(&id),
                )
                .unwrap());
            }

            // 5. Failed outcome:
            // - Grok uses OSC 7501 state=error (surviving trailing idle) AND updates.jsonl
            //   turn_completed failure when Grok exits working -> idle without OSC error.
            // - OpenCode uses opencode.db session_v2 idle_outcome="failed" both before and after
            //   OSC idle, reverting the optimistic unread badge in SQLite when arriving late.
            if runtime == Runtime::Grok {
                ingest("state=working:app=grok");
                ingest("state=error:app=grok");
                ingest("state=idle:app=grok");
                let failed = core.sessions.agent_status(&id);
                assert_eq!(failed.observation.outcome, Some(TurnOutcome::Failed));
                assert!(failed.failed_since.is_some());
                assert_eq!(failed.unread_since, None);

                // Grok working -> idle (no OSC error) + updates.jsonl failure before idle.
                ingest("state=working:app=grok");
                core.sessions.publish_agent_event(
                    &id,
                    A::ProgramTurnOutcome {
                        outcome: TurnOutcome::Failed,
                        policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                    },
                    events.as_ref(),
                );
                assert_eq!(
                    core.sessions.agent_status(&id).observation.activity,
                    Activity::Working
                );
                ingest("state=idle:app=grok");
                let grok_before_idle = core.sessions.agent_status(&id);
                assert_eq!(
                    grok_before_idle.observation.outcome,
                    Some(TurnOutcome::Failed)
                );
                assert!(grok_before_idle.failed_since.is_some());
                assert_eq!(grok_before_idle.unread_since, None);

                // Grok working -> idle (no OSC error) + updates.jsonl failure after idle.
                ingest("state=working:app=grok");
                ingest("state=idle:app=grok");
                assert_eq!(core.sessions.agent_status(&id).observation.outcome, None);
                core.sessions.publish_agent_event(
                    &id,
                    A::ProgramTurnOutcome {
                        outcome: TurnOutcome::Failed,
                        policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                    },
                    events.as_ref(),
                );
                let grok_after_idle = core.sessions.agent_status(&id);
                assert_eq!(
                    grok_after_idle.observation.outcome,
                    Some(TurnOutcome::Failed)
                );
                assert!(grok_after_idle.failed_since.is_some());
                assert_eq!(grok_after_idle.unread_since, None);
            } else {
                // (a) Before OSC idle: deferred while working, applied on idle with no badge.
                ingest("state=working:app=opencode");
                ingest("id=ses_main:app=opencode:state=working");
                let before_ms = crate::session::clock::state_now().wall_millis;
                core.sessions.publish_agent_event(
                    &id,
                    A::ProgramSessionOutcome {
                        outcome: TurnOutcome::Failed,
                        policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                        session_id: "ses_main".into(),
                        ended_at_ms: before_ms,
                        superseded: false,
                    },
                    events.as_ref(),
                );
                assert_eq!(
                    core.sessions.agent_status(&id).observation.activity,
                    Activity::Working
                );
                ingest("state=idle:app=opencode");
                ingest("state=clear:id=ses_main");
                let before_idle = core.sessions.agent_status(&id);
                assert_eq!(before_idle.observation.outcome, Some(TurnOutcome::Failed));
                assert!(before_idle.failed_since.is_some());
                assert_eq!(before_idle.unread_since, None);
                assert!(!crate::repo::session_attention::any_unread(
                    &core.db.get().unwrap(),
                    std::slice::from_ref(&id),
                )
                .unwrap());

                // (b) After OSC idle: idle-inferred Completed records unread, then late
                // opencode.db Failed overrides to Failed AND reverts SQLite unread attention.
                ingest("state=working:app=opencode");
                ingest("id=ses_main:app=opencode:state=working");
                ingest("state=idle:app=opencode");
                ingest("state=clear:id=ses_main");
                assert!(core.sessions.agent_status(&id).unread_since.is_some());
                assert!(crate::repo::session_attention::any_unread(
                    &core.db.get().unwrap(),
                    std::slice::from_ref(&id),
                )
                .unwrap());
                let after_ms = crate::session::clock::state_now().wall_millis;
                core.sessions.publish_agent_event(
                    &id,
                    A::ProgramSessionOutcome {
                        outcome: TurnOutcome::Failed,
                        policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                        session_id: "ses_main".into(),
                        ended_at_ms: after_ms,
                        superseded: false,
                    },
                    events.as_ref(),
                );
                let after_idle = core.sessions.agent_status(&id);
                assert_eq!(after_idle.observation.outcome, Some(TurnOutcome::Failed));
                assert!(after_idle.failed_since.is_some());
                assert_eq!(after_idle.unread_since, None);
                assert!(
                    !crate::repo::session_attention::any_unread(
                        &core.db.get().unwrap(),
                        std::slice::from_ref(&id),
                    )
                    .unwrap(),
                    "late OpenCode DB Failed must revert optimistic unread badge in DB"
                );

                // (c) Multi-session OpenCode TUI root messages.
                ingest("state=working:app=opencode:msg=MiBzZXNzaW9ucyB3b3JraW5n");
                ingest("id=ses_a:app=opencode:state=working");
                ingest("id=ses_b:app=opencode:state=working");
                let multi = core.sessions.agent_status(&id);
                assert_eq!(
                    multi
                        .observation
                        .program_status
                        .as_ref()
                        .unwrap()
                        .message
                        .as_deref(),
                    Some("2 sessions working")
                );
                assert_eq!(
                    multi
                        .observation
                        .program_status
                        .as_ref()
                        .unwrap()
                        .background_tasks,
                    0
                );
                ingest("state=blocked:app=opencode:msg=MiBzZXNzaW9ucyBuZWVkIGlucHV0");
                let multi_blocked = core.sessions.agent_status(&id);
                assert!(multi_blocked.observation.needs_you());
                assert_eq!(
                    multi_blocked.observation.interactions[0].reason,
                    WaitReason::Unknown
                );
                assert_eq!(
                    multi_blocked
                        .observation
                        .program_status
                        .as_ref()
                        .unwrap()
                        .message
                        .as_deref(),
                    Some("2 sessions need input")
                );
                ingest("state=idle:app=opencode");
                ingest("state=clear:id=ses_a");
                ingest("state=clear:id=ses_b");
                assert_eq!(
                    core.sessions.agent_status(&id).observation.outcome,
                    Some(TurnOutcome::Completed)
                );
                clear_unread();
            }

            core.sessions.kill(&id).unwrap();
        }

        // 6. No-OSC fallback (Grok program_status="off" / OpenCode unanswered probe):
        // direct chat with pending first turn releases on ProgramTurnFinished and tracks terminal
        // activity via Forwarder transitions.
        let dir = tempfile::tempdir().unwrap();
        let core = crate::test_support::test_core_with_runtime(fake_runtime());
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        let mut agent_role = role(runtime.key(), &[]);
        agent_role.runtime = runtime.key().into();
        agent_role.command = "fake".into();
        insert_role_row(&core.db.get().unwrap(), &agent_role);

        let spawned = core
            .sessions
            .spawn_direct(
                &agent_role,
                DirectLaunch {
                    cwd: Some(dir.path().to_str().unwrap().into()),
                    first_turn: Some("first turn".into()),
                    ..Default::default()
                },
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
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
        core.sessions.startup_output(&id, &stop, b"ready", now);
        core.sessions
            .release_settled_startup(&id, &stop, now + Duration::from_secs(60));
        assert_eq!(
            core.sessions.report_readiness(&id),
            router::DeliveryReadiness::Starting,
            "{runtime:?}: first turn without OSC stays Starting until turn finishes"
        );
        core.sessions
            .publish_agent_event(&id, A::ProgramTurnFinished, events.as_ref());
        assert_eq!(
            core.sessions.report_readiness(&id),
            router::DeliveryReadiness::Ready { idle: true },
            "{runtime:?}: no-OSC first turn releases on ProgramTurnFinished"
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
fn grok_without_transcript_and_opencode_without_db_release_no_osc_first_turn_while_holding_live_osc_waits(
) {
    use crate::session::hook_queue::TestHookRoute;

    for (runtime, app) in [(Runtime::Grok, "grok"), (Runtime::OpenCode, "opencode")] {
        for mission in [false, true] {
            for with_osc_wait in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let core = crate::test_support::test_core_with_runtime(fake_runtime());
                let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
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
                    let sid = pending.session_id.clone();
                    core.sessions
                        .complete_mission_session_spawn(
                            pending,
                            Arc::clone(&events),
                            Arc::new(AtomicBool::new(false)),
                        )
                        .unwrap();
                    let terminal = core
                        .sessions
                        .create_terminal(&sid, 80, 24, &core.db, &events)
                        .unwrap();
                    core.sessions
                        .session_state(&sid)
                        .unwrap()
                        .lock()
                        .unwrap()
                        .terminal = Some(terminal);
                    (sid, Some(m.id))
                } else {
                    insert_role_row(&core.db.get().unwrap(), &agent_role);
                    let spawned = core
                        .sessions
                        .spawn_direct(
                            &agent_role,
                            DirectLaunch {
                                cwd: Some(dir.path().to_str().unwrap().into()),
                                first_turn: Some("first turn".into()),
                                ..Default::default()
                            },
                            dir.path(),
                            Arc::clone(&core.db),
                            Arc::clone(&events),
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
                core.sessions.startup_output(&id, &stop, b"ready", now);

                // Drive the real runtime watcher with missing native outcome source:
                // - Grok host-server SessionStart without transcriptPath
                // - OpenCode snapshot with db_path = None (:memory:)
                if runtime == Runtime::Grok {
                    let (route, receiver) = TestHookRoute::new(Runtime::Grok, "current".into());
                    let mut watcher = crate::runtimes::grok::status::Watcher::new(
                        receiver,
                        &SpawnSpec::default(),
                    );
                    let grok_session_id = uuid::Uuid::new_v4().to_string();
                    drop(
                        route
                            .admit(serde_json::json!({
                                "hook_event_name": "SessionStart",
                                "sessionId": grok_session_id,
                            }))
                            .unwrap(),
                    );
                    watcher
                        .drain_events(
                            0,
                            &mut |ev| core.sessions.publish_agent_event(&id, ev, events.as_ref()),
                            &mut |_| {},
                        )
                        .unwrap();
                    route.retire();
                } else {
                    let (route, receiver) = TestHookRoute::new(Runtime::OpenCode, "current".into());
                    let mut watcher = crate::runtimes::opencode::status::Watcher::with_db(
                        receiver, None, None, 0,
                    );
                    drop(
                        route
                            .admit(serde_json::json!({
                                "hook_event_name": "snapshot",
                                "epoch": 1,
                                "sequence": 1,
                                "session_id": "ses_mem",
                            }))
                            .unwrap(),
                    );
                    watcher
                        .drain_events(
                            0,
                            &mut |ev| core.sessions.publish_agent_event(&id, ev, events.as_ref()),
                            &mut |_| {},
                        )
                        .unwrap();
                    route.retire();
                }

                if with_osc_wait {
                    core.sessions.ingest_output_chunk(
                        &id,
                        mission_id.as_deref(),
                        format!(
                            "\x1b]7501;state=working:app={app}\x1b\\\x1b]7501;state=blocked:app={app}:kind=permission\x1b\\"
                        )
                        .as_bytes(),
                        events.as_ref(),
                    );
                }

                core.sessions
                    .release_settled_startup(&id, &stop, now + Duration::from_secs(60));
                if with_osc_wait {
                    assert_eq!(
                        core.sessions.report_readiness(&id),
                        router::DeliveryReadiness::Starting,
                        "{runtime:?} mission={mission}: live OSC blocked wait must keep startup held"
                    );
                    assert!(
                        core.sessions.agent_status(&id).observation.needs_you(),
                        "{runtime:?} mission={mission}: live OSC blocked wait must surface NeedsYou"
                    );
                    assert_eq!(
                        core.sessions.reserve_delivery(&id).unwrap(),
                        if mission {
                            router::DeliveryReservation::Starting
                        } else {
                            router::DeliveryReservation::HumanInteraction
                        }
                    );
                    // Once the root OSC wait releases to idle, startup finishes cleanly.
                    core.sessions.ingest_output_chunk(
                        &id,
                        mission_id.as_deref(),
                        format!("\x1b]7501;state=idle:app={app}\x1b\\").as_bytes(),
                        events.as_ref(),
                    );
                    assert_eq!(
                        core.sessions.report_readiness(&id),
                        router::DeliveryReadiness::Ready { idle: true },
                        "{runtime:?} mission={mission}: releasing live OSC wait finishes startup"
                    );
                } else {
                    assert_eq!(
                        core.sessions.report_readiness(&id),
                        router::DeliveryReadiness::Ready { idle: true },
                        "{runtime:?} mission={mission}: missing outcome source + no OSC must release after settled output"
                    );
                }
                core.sessions.kill(&id).unwrap();
            }
        }
    }
}

#[test]
fn opencode_watcher_reports_background_failure_after_route_switch_and_isolates_delayed_outcome_across_next_turn(
) {
    use crate::session::hook_queue::TestHookRoute;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("opencode/opencode.db");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch(
        "CREATE TABLE session_v2(
            id TEXT PRIMARY KEY,
            time_created INTEGER NOT NULL,
            time_busy INTEGER,
            time_idle INTEGER,
            idle_outcome TEXT
        );",
    )
    .unwrap();

    let core = crate::test_support::test_core_with_runtime(fake_runtime());
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    let mut agent_role = role("opencode", &[]);
    agent_role.runtime = "opencode".into();
    agent_role.command = "fake".into();
    insert_role_row(&core.db.get().unwrap(), &agent_role);

    let spawned = core
        .sessions
        .spawn_direct(
            &agent_role,
            DirectLaunch {
                cwd: Some(dir.path().to_str().unwrap().into()),
                ..Default::default()
            },
            dir.path(),
            Arc::clone(&core.db),
            Arc::clone(&events),
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

    let launched_ms = crate::session::clock::state_now().wall_millis - 10_000;
    let (route, receiver) = TestHookRoute::new(Runtime::OpenCode, "current".into());
    let mut watcher = crate::runtimes::opencode::status::Watcher::with_db(
        receiver,
        None,
        Some(db_path.clone()),
        launched_ms,
    );
    let ingest = |seq: &str| {
        core.sessions.ingest_output_chunk(
            &id,
            None,
            format!("\x1b]7501;{seq}\x1b\\").as_bytes(),
            events.as_ref(),
        );
    };
    let mut drain = || {
        watcher
            .drain_events(
                0,
                &mut |ev| core.sessions.publish_agent_event(&id, ev, events.as_ref()),
                &mut |_| {},
            )
            .unwrap();
    };

    // 1. Start work in ses_a, navigate away to home ("") and idle ses_b; ses_a finishes as
    // failed in SQLite after root goes idle -> must still settle Failed and revert unread.
    ingest("state=idle:app=opencode");
    conn.execute(
        "INSERT INTO session_v2(id, time_created, time_busy) VALUES ('ses_a', ?1, ?1)",
        [launched_ms + 100],
    )
    .unwrap();
    drop(
        route
            .admit(serde_json::json!({
                "hook_event_name": "snapshot",
                "epoch": 1,
                "sequence": 1,
                "session_id": "ses_a",
            }))
            .unwrap(),
    );
    drain();
    ingest("state=working:app=opencode");
    ingest("id=ses_a:app=opencode:state=working");
    // Navigate away to home ("") and then idle ses_b while ses_a is still running.
    drop(
        route
            .admit(serde_json::json!({
                "hook_event_name": "snapshot",
                "epoch": 1,
                "sequence": 2,
                "session_id": "",
            }))
            .unwrap(),
    );
    drain();
    drop(
        route
            .admit(serde_json::json!({
                "hook_event_name": "snapshot",
                "epoch": 1,
                "sequence": 3,
                "session_id": "ses_b",
            }))
            .unwrap(),
    );
    drain();
    ingest("state=idle:app=opencode");
    ingest("state=clear:id=ses_a");
    assert!(core.sessions.agent_status(&id).unread_since.is_some());
    conn.execute(
        "UPDATE session_v2 SET time_idle = ?1, idle_outcome = 'failed' WHERE id = 'ses_a'",
        [launched_ms + 200],
    )
    .unwrap();
    drain();
    let bg_fail = core.sessions.agent_status(&id);
    assert_eq!(bg_fail.observation.outcome, Some(TurnOutcome::Failed));
    assert!(bg_fail.failed_since.is_some());
    assert_eq!(bg_fail.unread_since, None);
    assert!(!crate::repo::session_attention::any_unread(
        &core.db.get().unwrap(),
        std::slice::from_ref(&id),
    )
    .unwrap());

    // 2. Navigate back to ses_a; Turn 1 completes optimistically (unread recorded), Turn 2
    // starts immediately in ses_a (time_busy > time_idle), then watcher polls Turn 1's delayed
    // failure: Turn 1's unread is reverted in SQLite AND Turn 2 is NOT poisoned when it succeeds.
    drop(
        route
            .admit(serde_json::json!({
                "hook_event_name": "snapshot",
                "epoch": 1,
                "sequence": 4,
                "session_id": "ses_a",
            }))
            .unwrap(),
    );
    drain();
    ingest("state=working:app=opencode");
    ingest("id=ses_a:app=opencode:state=working");
    ingest("state=idle:app=opencode");
    ingest("state=clear:id=ses_a");
    let t1_idle = crate::session::clock::state_now().wall_millis;
    assert!(core.sessions.agent_status(&id).unread_since.is_some());
    assert!(crate::repo::session_attention::any_unread(
        &core.db.get().unwrap(),
        std::slice::from_ref(&id),
    )
    .unwrap());

    // Turn 2 starts before Turn 1's failure is polled; time_busy advances past Turn 1's time_idle.
    ingest("state=working:app=opencode");
    ingest("id=ses_a:app=opencode:state=working");
    let t2_busy = crate::session::clock::state_now().wall_millis.max(t1_idle) + 100;
    conn.execute(
        "UPDATE session_v2 SET time_idle = ?1, time_busy = ?2, idle_outcome = 'failed' WHERE id = 'ses_a'",
        rusqlite::params![t1_idle, t2_busy],
    )
    .unwrap();
    drain();
    assert_eq!(
        core.sessions.agent_status(&id).observation.activity,
        Activity::Working
    );
    assert_eq!(
        core.sessions.agent_status(&id).unread_since,
        None,
        "delayed Turn 1 failure must revert Turn 1's optimistic unread in memory"
    );
    assert!(
        !crate::repo::session_attention::any_unread(
            &core.db.get().unwrap(),
            std::slice::from_ref(&id),
        )
        .unwrap(),
        "delayed Turn 1 failure must revert Turn 1's optimistic unread in DB"
    );

    // Turn 2 finishes cleanly -> Completed + new unread badge!
    ingest("state=idle:app=opencode");
    ingest("state=clear:id=ses_a");
    conn.execute(
        "UPDATE session_v2 SET time_idle = ?1, idle_outcome = 'succeeded' WHERE id = 'ses_a'",
        [t2_busy + 100],
    )
    .unwrap();
    drain();
    let turn2 = core.sessions.agent_status(&id);
    assert_eq!(turn2.observation.outcome, Some(TurnOutcome::Completed));
    assert_eq!(turn2.failed_since, None);
    assert!(turn2.unread_since.is_some());

    route.retire();
    core.sessions.kill(&id).unwrap();
}

#[test]
fn opencode_late_failure_cas_undo_preserves_prior_durable_unread_error_ack_and_tab_viewed_state() {
    use crate::session::state::agent::{AgentEvent as A, ProgramOutcomePolicy};

    // Case A: Daemon restart / fresh SessionModel with pre-existing durable unread,
    // error_acknowledged_at, and a tab row whose last_viewed_at is NULL.
    {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::test_support::test_core_with_runtime(fake_runtime());
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        let mut agent_role = role("opencode", &[]);
        agent_role.runtime = "opencode".into();
        agent_role.command = "fake".into();
        insert_role_row(&core.db.get().unwrap(), &agent_role);

        let spawned = core
            .sessions
            .spawn_direct(
                &agent_role,
                DirectLaunch {
                    cwd: Some(dir.path().to_str().unwrap().into()),
                    ..Default::default()
                },
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
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

        let prior_unread_ms = 1_700_000_000_000i64;
        let prior_ack = "2026-11-20T00:00:00Z";
        let prior_tab_completed = "2026-11-20T00:05:00Z";
        let tab_id = {
            let conn = core.db.get().unwrap();
            conn.execute(
                "INSERT INTO session_attention(session_id, unread_since, error_acknowledged_at)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![&id, prior_unread_ms, prior_ack],
            )
            .unwrap();
            let layout = serde_json::json!({"session_id": id}).to_string();
            let tab =
                crate::repo::node::create_tab(&conn, None, "OpenCode tab", 1, &layout).unwrap();
            conn.execute(
                "UPDATE nodes SET last_completed_at = ?1, last_viewed_at = NULL WHERE id = ?2",
                rusqlite::params![prior_tab_completed, &tab.id],
            )
            .unwrap();
            tab.id
        };

        // Notice fresh SessionModel has unread_since == None in memory, just like after daemon restart.
        assert_eq!(core.sessions.agent_status(&id).unread_since, None);

        let ingest = |seq: &str| {
            core.sessions.ingest_output_chunk(
                &id,
                None,
                format!("\x1b]7501;{seq}\x1b\\").as_bytes(),
                events.as_ref(),
            );
        };
        ingest("state=idle:app=opencode");
        ingest("state=working:app=opencode");
        ingest("id=ses_restart:app=opencode:state=working");
        ingest("state=idle:app=opencode");
        ingest("state=clear:id=ses_restart");

        // Late Failed from opencode.db arrives after optimistic completion wrote Turn 2's unread.
        let now_ms = crate::session::clock::state_now().wall_millis;
        core.sessions.publish_agent_event(
            &id,
            A::ProgramSessionOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                session_id: "ses_restart".into(),
                ended_at_ms: now_ms + 50,
                superseded: false,
            },
            events.as_ref(),
        );

        let conn = core.db.get().unwrap();
        let (db_unread, db_ack): (Option<i64>, Option<String>) = conn
            .query_row(
                "SELECT unread_since, error_acknowledged_at FROM session_attention WHERE session_id = ?1",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            db_unread,
            Some(prior_unread_ms),
            "CAS rollback must restore Turn 1's durable unread_since instead of wiping it"
        );
        assert_eq!(
            db_ack.as_deref(),
            Some(prior_ack),
            "CAS rollback must never overwrite error_acknowledged_at"
        );
        let (tab_completed, tab_viewed): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT last_completed_at, last_viewed_at FROM nodes WHERE id = ?1",
                [&tab_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(tab_completed.as_deref(), Some(prior_tab_completed));
        assert_eq!(
            tab_viewed, None,
            "CAS rollback must never fabricate a last_viewed_at user view"
        );
        drop(conn);

        let snapshot = crate::ops::session::session_status_snapshot(&core).unwrap();
        let snap = &snapshot[&id];
        assert_eq!(snap.unread_since, Some(prior_unread_ms));
        assert_eq!(snap.observation.outcome, Some(TurnOutcome::Failed));
        assert!(snap.failed_since.is_some());

        core.sessions.kill(&id).unwrap();
    }

    // Case B: Fresh session with no prior unread, pre-existing error_acknowledged_at, and
    // tab last_viewed_at = NULL -> late Failed clears unread back to NULL without touching
    // error_acknowledged_at or fabricating last_viewed_at.
    {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::test_support::test_core_with_runtime(fake_runtime());
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        let mut agent_role = role("opencode", &[]);
        agent_role.runtime = "opencode".into();
        agent_role.command = "fake".into();
        insert_role_row(&core.db.get().unwrap(), &agent_role);

        let spawned = core
            .sessions
            .spawn_direct(
                &agent_role,
                DirectLaunch {
                    cwd: Some(dir.path().to_str().unwrap().into()),
                    ..Default::default()
                },
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
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

        let prior_ack = "2026-11-20T00:00:00Z";
        let tab_id = {
            let conn = core.db.get().unwrap();
            conn.execute(
                "INSERT INTO session_attention(session_id, unread_since, error_acknowledged_at)
                 VALUES (?1, NULL, ?2)",
                rusqlite::params![&id, prior_ack],
            )
            .unwrap();
            let layout = serde_json::json!({"session_id": id}).to_string();
            let tab = crate::repo::node::create_tab(&conn, None, "Fresh tab", 1, &layout).unwrap();
            conn.execute(
                "UPDATE nodes SET last_completed_at = NULL, last_viewed_at = NULL WHERE id = ?1",
                [&tab.id],
            )
            .unwrap();
            tab.id
        };

        let ingest = |seq: &str| {
            core.sessions.ingest_output_chunk(
                &id,
                None,
                format!("\x1b]7501;{seq}\x1b\\").as_bytes(),
                events.as_ref(),
            );
        };
        ingest("state=idle:app=opencode");
        ingest("state=working:app=opencode");
        ingest("id=ses_fresh:app=opencode:state=working");
        ingest("state=idle:app=opencode");
        ingest("state=clear:id=ses_fresh");

        let now_ms = crate::session::clock::state_now().wall_millis;
        core.sessions.publish_agent_event(
            &id,
            A::ProgramSessionOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                session_id: "ses_fresh".into(),
                ended_at_ms: now_ms + 50,
                superseded: false,
            },
            events.as_ref(),
        );

        let conn = core.db.get().unwrap();
        let (db_unread, db_ack): (Option<i64>, Option<String>) = conn
            .query_row(
                "SELECT unread_since, error_acknowledged_at FROM session_attention WHERE session_id = ?1",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(db_unread, None);
        assert_eq!(db_ack.as_deref(), Some(prior_ack));
        let (tab_completed, tab_viewed): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT last_completed_at, last_viewed_at FROM nodes WHERE id = ?1",
                [&tab_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(tab_completed, None);
        assert_eq!(tab_viewed, None);
        drop(conn);
        assert_eq!(core.sessions.agent_status(&id).unread_since, None);

        core.sessions.kill(&id).unwrap();
    }
}

#[test]
fn opencode_mixed_multi_session_turn_reverts_false_durable_unread_in_both_store_orderings() {
    use crate::session::hook_queue::TestHookRoute;

    // Cover both lexicographic orderings:
    // (1) ses_a=succeeded, ses_b=failed (BTreeSet polls succeeded BEFORE failed)
    // (2) ses_a=failed, ses_b=succeeded (BTreeSet polls failed BEFORE succeeded)
    for (outcome_a, outcome_b) in [("succeeded", "failed"), ("failed", "succeeded")] {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("opencode/opencode.db");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE session_v2(
                id TEXT PRIMARY KEY,
                time_created INTEGER NOT NULL,
                time_busy INTEGER,
                time_idle INTEGER,
                idle_outcome TEXT
            );",
        )
        .unwrap();

        let core = crate::test_support::test_core_with_runtime(fake_runtime());
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        let mut agent_role = role("opencode", &[]);
        agent_role.runtime = "opencode".into();
        agent_role.command = "fake".into();
        insert_role_row(&core.db.get().unwrap(), &agent_role);

        let spawned = core
            .sessions
            .spawn_direct(
                &agent_role,
                DirectLaunch {
                    cwd: Some(dir.path().to_str().unwrap().into()),
                    ..Default::default()
                },
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
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

        let launched_ms = crate::session::clock::state_now().wall_millis - 10_000;
        let (route, receiver) = TestHookRoute::new(Runtime::OpenCode, "mixed".into());
        let mut watcher = crate::runtimes::opencode::status::Watcher::with_db(
            receiver,
            None,
            Some(db_path.clone()),
            launched_ms,
        );
        let ingest = |seq: &str| {
            core.sessions.ingest_output_chunk(
                &id,
                None,
                format!("\x1b]7501;{seq}\x1b\\").as_bytes(),
                events.as_ref(),
            );
        };
        let mut drain = || {
            watcher
                .drain_events(
                    0,
                    &mut |ev| core.sessions.publish_agent_event(&id, ev, events.as_ref()),
                    &mut |_| {},
                )
                .unwrap();
        };

        ingest("state=idle:app=opencode");
        conn.execute(
            "INSERT INTO session_v2(id, time_created, time_busy) VALUES ('ses_a', ?1, ?1)",
            [launched_ms + 10],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_v2(id, time_created, time_busy) VALUES ('ses_b', ?1, ?1)",
            [launched_ms + 20],
        )
        .unwrap();
        // Visit ses_a (busy), then switch to ses_b (busy) so both run concurrently in one TUI.
        drop(
            route
                .admit(serde_json::json!({
                    "hook_event_name": "snapshot",
                    "epoch": 1,
                    "sequence": 1,
                    "session_id": "ses_a",
                }))
                .unwrap(),
        );
        drain();
        drop(
            route
                .admit(serde_json::json!({
                    "hook_event_name": "snapshot",
                    "epoch": 1,
                    "sequence": 2,
                    "session_id": "ses_b",
                }))
                .unwrap(),
        );
        drain();

        ingest("state=working:app=opencode:msg=MiBzZXNzaW9ucyB3b3JraW5n");
        ingest("id=ses_a:app=opencode:state=working");
        ingest("id=ses_b:app=opencode:state=working");
        // Root goes idle (unfocused) -> optimistic unread written to SQLite.
        ingest("state=idle:app=opencode");
        ingest("state=clear:id=ses_a");
        ingest("state=clear:id=ses_b");
        assert!(crate::repo::session_attention::any_unread(
            &core.db.get().unwrap(),
            std::slice::from_ref(&id),
        )
        .unwrap());

        // Both rows settle in SQLite and are read in the SAME watcher poll.
        let end_ms = crate::session::clock::state_now().wall_millis + 50;
        conn.execute(
            "UPDATE session_v2 SET time_idle = ?1, idle_outcome = ?2 WHERE id = 'ses_a'",
            rusqlite::params![end_ms, outcome_a],
        )
        .unwrap();
        conn.execute(
            "UPDATE session_v2 SET time_idle = ?1, idle_outcome = ?2 WHERE id = 'ses_b'",
            rusqlite::params![end_ms + 1, outcome_b],
        )
        .unwrap();
        drain();

        let snap = &crate::ops::session::session_status_snapshot(&core).unwrap()[&id];
        assert_eq!(
            snap.observation.outcome,
            Some(TurnOutcome::Failed),
            "order=({outcome_a}, {outcome_b}) must settle Failed"
        );
        assert!(snap.failed_since.is_some());
        assert_eq!(
            snap.unread_since, None,
            "order=({outcome_a}, {outcome_b}): fresh status snapshot must not restore false durable unread"
        );
        assert!(
            !crate::repo::session_attention::any_unread(
                &core.db.get().unwrap(),
                std::slice::from_ref(&id),
            )
            .unwrap(),
            "order=({outcome_a}, {outcome_b}): SQLite session_attention must have no durable unread"
        );

        route.retire();
        core.sessions.kill(&id).unwrap();
    }
}

#[test]
fn opencode_ignores_delayed_superseded_turn1_after_turn2_idle_and_foreign_db_change_on_idle_tui() {
    use crate::session::state::agent::{AgentEvent as A, ProgramOutcomePolicy};

    let dir = tempfile::tempdir().unwrap();
    let core = crate::test_support::test_core_with_runtime(fake_runtime());
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    let mut agent_role = role("opencode", &[]);
    agent_role.runtime = "opencode".into();
    agent_role.command = "fake".into();
    insert_role_row(&core.db.get().unwrap(), &agent_role);

    let spawned = core
        .sessions
        .spawn_direct(
            &agent_role,
            DirectLaunch {
                cwd: Some(dir.path().to_str().unwrap().into()),
                ..Default::default()
            },
            dir.path(),
            Arc::clone(&core.db),
            Arc::clone(&events),
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

    let ingest = |seq: &str| {
        core.sessions.ingest_output_chunk(
            &id,
            None,
            format!("\x1b]7501;{seq}\x1b\\").as_bytes(),
            events.as_ref(),
        );
    };

    // 1. TUI has only ever shown root idle: a foreign DB failure on a visited session must be ignored.
    ingest("state=idle:app=opencode");
    let now_ms = crate::session::clock::state_now().wall_millis;
    core.sessions.publish_agent_event(
        &id,
        A::ProgramSessionOutcome {
            outcome: TurnOutcome::Failed,
            policy: ProgramOutcomePolicy::DeferWhileNativeActive,
            session_id: "ses_visited".into(),
            ended_at_ms: now_ms + 10,
            superseded: false,
        },
        events.as_ref(),
    );
    let idle_snap = &crate::ops::session::session_status_snapshot(&core).unwrap()[&id];
    assert_eq!(idle_snap.observation.outcome, None);
    assert_eq!(idle_snap.failed_since, None);

    // 2. Turn 1 in ses_a finishes, Turn 2 in ses_a runs and finishes cleanly (working -> idle),
    // then delayed Turn 1 failure arrives with superseded=true: Turn 2 keeps Completed + durable unread.
    ingest("state=working:app=opencode");
    ingest("id=ses_a:app=opencode:state=working");
    ingest("state=idle:app=opencode");
    ingest("state=clear:id=ses_a");

    ingest("state=working:app=opencode");
    ingest("id=ses_a:app=opencode:state=working");
    ingest("state=idle:app=opencode");
    ingest("state=clear:id=ses_a");

    let turn2_unread = core.sessions.agent_status(&id).unread_since;
    assert!(turn2_unread.is_some());
    core.sessions.publish_agent_event(
        &id,
        A::ProgramSessionOutcome {
            outcome: TurnOutcome::Failed,
            policy: ProgramOutcomePolicy::DeferWhileNativeActive,
            session_id: "ses_a".into(),
            ended_at_ms: now_ms,
            superseded: true,
        },
        events.as_ref(),
    );
    let snap = &crate::ops::session::session_status_snapshot(&core).unwrap()[&id];
    assert_eq!(snap.observation.outcome, Some(TurnOutcome::Completed));
    assert_eq!(snap.failed_since, None);
    assert!(snap.unread_since.is_some());
    assert!(crate::repo::session_attention::any_unread(
        &core.db.get().unwrap(),
        std::slice::from_ref(&id),
    )
    .unwrap());

    core.sessions.kill(&id).unwrap();
}

#[test]
fn grok_unreadable_transcript_path_releases_no_osc_first_turn_while_holding_live_osc_waits() {
    use crate::session::hook_queue::TestHookRoute;

    for with_osc_wait in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let missing_path = dir.path().join("does-not-exist/updates.jsonl");
        let core = crate::test_support::test_core_with_runtime(fake_runtime());
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        let mut agent_role = role("grok", &[]);
        agent_role.runtime = "grok".into();
        agent_role.command = "fake".into();
        insert_role_row(&core.db.get().unwrap(), &agent_role);

        let spawned = core
            .sessions
            .spawn_direct(
                &agent_role,
                DirectLaunch {
                    cwd: Some(dir.path().to_str().unwrap().into()),
                    first_turn: Some("first turn".into()),
                    ..Default::default()
                },
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
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
        core.sessions.startup_output(&id, &stop, b"ready", now);

        let (route, receiver) = TestHookRoute::new(Runtime::Grok, "unreadable".into());
        let mut watcher =
            crate::runtimes::grok::status::Watcher::new(receiver, &SpawnSpec::default());
        drop(
            route
                .admit(serde_json::json!({
                    "hook_event_name": "SessionStart",
                    "sessionId": uuid::Uuid::new_v4().to_string(),
                    "transcriptPath": missing_path,
                }))
                .unwrap(),
        );
        watcher
            .drain_events(
                0,
                &mut |ev| core.sessions.publish_agent_event(&id, ev, events.as_ref()),
                &mut |_| {},
            )
            .unwrap();
        route.retire();

        if with_osc_wait {
            core.sessions.ingest_output_chunk(
                &id,
                None,
                b"\x1b]7501;state=working:app=grok\x1b\\\x1b]7501;state=blocked:app=grok:kind=permission\x1b\\",
                events.as_ref(),
            );
        }
        core.sessions
            .release_settled_startup(&id, &stop, now + Duration::from_secs(60));
        if with_osc_wait {
            assert_eq!(
                core.sessions.report_readiness(&id),
                router::DeliveryReadiness::Starting
            );
            assert!(core.sessions.agent_status(&id).observation.needs_you());
        } else {
            assert_eq!(
                core.sessions.report_readiness(&id),
                router::DeliveryReadiness::Ready { idle: true }
            );
        }
        core.sessions.kill(&id).unwrap();
    }
}

#[test]
fn opencode_viewed_idle_completion_and_db_leading_pty_reconcile_without_false_unread_or_foreign_bleed(
) {
    use crate::session::hook_queue::TestHookRoute;

    // 1. working -> idle -> Viewed -> matching DB Failed / Interrupted updates the displayed
    // outcome while keeping unread cleared in BOTH memory and SQLite.
    for (db_outcome, expected) in [
        ("failed", TurnOutcome::Failed),
        ("interrupted", TurnOutcome::Interrupted),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("opencode/opencode.db");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE session_v2(
                id TEXT PRIMARY KEY,
                time_created INTEGER NOT NULL,
                time_busy INTEGER,
                time_idle INTEGER,
                idle_outcome TEXT
            );",
        )
        .unwrap();

        let core = crate::test_support::test_core_with_runtime(fake_runtime());
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        let mut agent_role = role("opencode", &[]);
        agent_role.runtime = "opencode".into();
        agent_role.command = "fake".into();
        insert_role_row(&core.db.get().unwrap(), &agent_role);

        let spawned = core
            .sessions
            .spawn_direct(
                &agent_role,
                DirectLaunch {
                    cwd: Some(dir.path().to_str().unwrap().into()),
                    ..Default::default()
                },
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
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

        let launched_ms = crate::session::clock::state_now().wall_millis - 5_000;
        let (route, receiver) = TestHookRoute::new(Runtime::OpenCode, "viewed".into());
        let mut watcher = crate::runtimes::opencode::status::Watcher::with_db(
            receiver,
            None,
            Some(db_path.clone()),
            launched_ms,
        );
        let ingest = |seq: &str| {
            core.sessions.ingest_output_chunk(
                &id,
                None,
                format!("\x1b]7501;{seq}\x1b\\").as_bytes(),
                events.as_ref(),
            );
        };
        let mut drain = || {
            watcher
                .drain_events(
                    0,
                    &mut |ev| core.sessions.publish_agent_event(&id, ev, events.as_ref()),
                    &mut |_| {},
                )
                .unwrap();
        };

        ingest("state=idle:app=opencode");
        conn.execute(
            "INSERT INTO session_v2(id, time_created, time_busy) VALUES ('ses_view', ?1, ?1)",
            [launched_ms + 10],
        )
        .unwrap();
        drop(
            route
                .admit(serde_json::json!({
                    "hook_event_name": "snapshot",
                    "epoch": 1,
                    "sequence": 1,
                    "session_id": "ses_view",
                }))
                .unwrap(),
        );
        drain();

        ingest("state=working:app=opencode");
        ingest("id=ses_view:app=opencode:state=working");
        ingest("state=idle:app=opencode");
        ingest("state=clear:id=ses_view");
        assert!(crate::repo::session_attention::any_unread(
            &core.db.get().unwrap(),
            std::slice::from_ref(&id),
        )
        .unwrap());

        // User views the chat before opencode.db writes the outcome.
        crate::ops::node::mark_direct_sessions_viewed(&core, std::slice::from_ref(&id)).unwrap();
        assert_eq!(core.sessions.agent_status(&id).unread_since, None);
        assert!(!crate::repo::session_attention::any_unread(
            &core.db.get().unwrap(),
            std::slice::from_ref(&id),
        )
        .unwrap());

        let end_ms = crate::session::clock::state_now().wall_millis + 50;
        conn.execute(
            "UPDATE session_v2 SET time_idle = ?1, idle_outcome = ?2 WHERE id = 'ses_view'",
            rusqlite::params![end_ms, db_outcome],
        )
        .unwrap();
        drain();

        let snap = &crate::ops::session::session_status_snapshot(&core).unwrap()[&id];
        assert_eq!(
            snap.observation.outcome,
            Some(expected),
            "viewed turn must still reconcile late DB {db_outcome}"
        );
        assert_eq!(snap.failed_since.is_some(), expected == TurnOutcome::Failed);
        assert_eq!(snap.unread_since, None);
        assert!(!crate::repo::session_attention::any_unread(
            &core.db.get().unwrap(),
            std::slice::from_ref(&id),
        )
        .unwrap());

        route.retire();
        core.sessions.kill(&id).unwrap();
    }

    // 2. DB outcome polled ahead of PTY working/child/idle stays plain Idle until the matching
    // turn arrives, then settles Failed with no unread; foreign session failures never bleed
    // into another session's turn, and explicit completion stays protected.
    {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("opencode/opencode.db");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE session_v2(
                id TEXT PRIMARY KEY,
                time_created INTEGER NOT NULL,
                time_busy INTEGER,
                time_idle INTEGER,
                idle_outcome TEXT
            );",
        )
        .unwrap();

        let core = crate::test_support::test_core_with_runtime(fake_runtime());
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        let mut agent_role = role("opencode", &[]);
        agent_role.runtime = "opencode".into();
        agent_role.command = "fake".into();
        insert_role_row(&core.db.get().unwrap(), &agent_role);

        let spawned = core
            .sessions
            .spawn_direct(
                &agent_role,
                DirectLaunch {
                    cwd: Some(dir.path().to_str().unwrap().into()),
                    ..Default::default()
                },
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
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

        let launched_ms = crate::session::clock::state_now().wall_millis - 5_000;
        let (route, receiver) = TestHookRoute::new(Runtime::OpenCode, "lead".into());
        let mut watcher = crate::runtimes::opencode::status::Watcher::with_db(
            receiver,
            None,
            Some(db_path.clone()),
            launched_ms,
        );
        let ingest = |seq: &str| {
            core.sessions.ingest_output_chunk(
                &id,
                None,
                format!("\x1b]7501;{seq}\x1b\\").as_bytes(),
                events.as_ref(),
            );
        };
        let mut drain = || {
            watcher
                .drain_events(
                    0,
                    &mut |ev| core.sessions.publish_agent_event(&id, ev, events.as_ref()),
                    &mut |_| {},
                )
                .unwrap();
        };

        ingest("state=idle:app=opencode");
        let now_ms = crate::session::clock::state_now().wall_millis;
        conn.execute(
            "INSERT INTO session_v2(id, time_created, time_busy, time_idle, idle_outcome)
             VALUES ('ses_lead', ?1, ?1, ?2, 'failed')",
            rusqlite::params![now_ms, now_ms + 10],
        )
        .unwrap();
        drop(
            route
                .admit(serde_json::json!({
                    "hook_event_name": "snapshot",
                    "epoch": 1,
                    "sequence": 1,
                    "session_id": "ses_lead",
                }))
                .unwrap(),
        );
        drain();

        // DB record arrived while TUI is still at startup root idle -> remains plain Idle.
        let before_pty = &crate::ops::session::session_status_snapshot(&core).unwrap()[&id];
        assert_eq!(before_pty.observation.outcome, None);
        assert_eq!(before_pty.failed_since, None);

        // Matching PTY turn arrives -> reconciles with buffered DB failure, no unread written.
        ingest("state=working:app=opencode");
        ingest("id=ses_lead:app=opencode:state=working");
        ingest("state=idle:app=opencode");
        ingest("state=clear:id=ses_lead");
        let after_pty = &crate::ops::session::session_status_snapshot(&core).unwrap()[&id];
        assert_eq!(after_pty.observation.outcome, Some(TurnOutcome::Failed));
        assert!(after_pty.failed_since.is_some());
        assert_eq!(after_pty.unread_since, None);
        assert!(!crate::repo::session_attention::any_unread(
            &core.db.get().unwrap(),
            std::slice::from_ref(&id),
        )
        .unwrap());

        // Foreign session failure buffered while idle must NOT poison a turn on ses_clean,
        // and explicit ProgramSessionFinished keeps Completed protected.
        let t2_ms = crate::session::clock::state_now().wall_millis + 100;
        conn.execute(
            "INSERT INTO session_v2(id, time_created, time_busy, time_idle, idle_outcome)
             VALUES ('ses_foreign', ?1, ?1, ?2, 'failed')",
            rusqlite::params![t2_ms, t2_ms + 10],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_v2(id, time_created, time_busy)
             VALUES ('ses_clean', ?1, ?1)",
            [t2_ms + 20],
        )
        .unwrap();
        drop(
            route
                .admit(serde_json::json!({
                    "hook_event_name": "snapshot",
                    "epoch": 1,
                    "sequence": 2,
                    "session_id": "ses_foreign",
                }))
                .unwrap(),
        );
        drain();
        drop(
            route
                .admit(serde_json::json!({
                    "hook_event_name": "snapshot",
                    "epoch": 1,
                    "sequence": 3,
                    "session_id": "ses_clean",
                }))
                .unwrap(),
        );
        drain();
        ingest("state=working:app=opencode");
        ingest("id=ses_clean:app=opencode:state=working");
        ingest("state=idle:app=opencode");
        ingest("state=clear:id=ses_clean");
        let clean = &crate::ops::session::session_status_snapshot(&core).unwrap()[&id];
        assert_eq!(clean.observation.outcome, Some(TurnOutcome::Completed));
        assert!(clean.unread_since.is_some());

        // Explicit success confirms ses_clean; a later idle-only DB failure leaves Completed intact.
        let clean_end_ms = crate::session::clock::state_now().wall_millis.max(t2_ms) + 50;
        conn.execute(
            "UPDATE session_v2 SET time_idle = ?1, idle_outcome = 'succeeded' WHERE id = 'ses_clean'",
            [clean_end_ms],
        )
        .unwrap();
        drain();
        conn.execute(
            "UPDATE session_v2 SET time_idle = ?1, idle_outcome = 'failed' WHERE id = 'ses_clean'",
            [clean_end_ms + 30],
        )
        .unwrap();
        drain();
        let protected = &crate::ops::session::session_status_snapshot(&core).unwrap()[&id];
        assert_eq!(protected.observation.outcome, Some(TurnOutcome::Completed));
        assert!(protected.unread_since.is_some());

        route.retire();
        core.sessions.kill(&id).unwrap();
    }
}

#[test]
fn opencode_cancel_confirmation_and_between_root_and_child_db_outcomes_attribute_to_right_turn() {
    use crate::session::hook_queue::TestHookRoute;

    // 1. Ctrl+C settles Interrupted, DB confirms interrupted, and the following successful turn
    // on the same session still settles Completed with an unread badge before its own DB
    // succeeded record is polled.
    {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("opencode/opencode.db");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE session_v2(
                id TEXT PRIMARY KEY,
                time_created INTEGER NOT NULL,
                time_busy INTEGER,
                time_idle INTEGER,
                idle_outcome TEXT
            );",
        )
        .unwrap();

        let core = crate::test_support::test_core_with_runtime(fake_runtime());
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        let mut agent_role = role("opencode", &[]);
        agent_role.runtime = "opencode".into();
        agent_role.command = "fake".into();
        insert_role_row(&core.db.get().unwrap(), &agent_role);

        let spawned = core
            .sessions
            .spawn_direct(
                &agent_role,
                DirectLaunch {
                    cwd: Some(dir.path().to_str().unwrap().into()),
                    ..Default::default()
                },
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
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

        let launched_ms = crate::session::clock::state_now().wall_millis - 5_000;
        let (route, receiver) = TestHookRoute::new(Runtime::OpenCode, "cancel-next".into());
        let mut watcher = crate::runtimes::opencode::status::Watcher::with_db(
            receiver,
            None,
            Some(db_path.clone()),
            launched_ms,
        );
        let ingest = |seq: &str| {
            core.sessions.ingest_output_chunk(
                &id,
                None,
                format!("\x1b]7501;{seq}\x1b\\").as_bytes(),
                events.as_ref(),
            );
        };
        let mut drain = || {
            watcher
                .drain_events(
                    0,
                    &mut |ev| core.sessions.publish_agent_event(&id, ev, events.as_ref()),
                    &mut |_| {},
                )
                .unwrap();
        };

        ingest("state=idle:app=opencode");
        conn.execute(
            "INSERT INTO session_v2(id, time_created, time_busy) VALUES ('ses_c', ?1, ?1)",
            [launched_ms + 10],
        )
        .unwrap();
        drop(
            route
                .admit(serde_json::json!({
                    "hook_event_name": "snapshot",
                    "epoch": 1,
                    "sequence": 1,
                    "session_id": "ses_c",
                }))
                .unwrap(),
        );
        drain();

        // Turn 1: cancelled via Ctrl+C, then DB confirms interrupted after root idle.
        ingest("state=working:app=opencode");
        ingest("id=ses_c:app=opencode:state=working");
        core.sessions
            .inject_direct_stdin(&id, b"\x03", events.as_ref())
            .unwrap();
        ingest("state=idle:app=opencode");
        ingest("state=clear:id=ses_c");
        assert_eq!(
            core.sessions.agent_status(&id).observation.outcome,
            Some(TurnOutcome::Interrupted)
        );
        let cancel_db_ms = crate::session::clock::state_now().wall_millis + 25;
        conn.execute(
            "UPDATE session_v2 SET time_idle = ?1, idle_outcome = 'interrupted' WHERE id = 'ses_c'",
            [cancel_db_ms],
        )
        .unwrap();
        drain();

        // Turn 2: succeeds cleanly BEFORE its DB succeeded row is polled -> Completed + badge.
        ingest("state=working:app=opencode");
        ingest("id=ses_c:app=opencode:state=working");
        ingest("state=idle:app=opencode");
        ingest("state=clear:id=ses_c");
        let snap = &crate::ops::session::session_status_snapshot(&core).unwrap()[&id];
        assert_eq!(snap.observation.outcome, Some(TurnOutcome::Completed));
        assert!(snap.unread_since.is_some());
        assert!(crate::repo::session_attention::any_unread(
            &core.db.get().unwrap(),
            std::slice::from_ref(&id),
        )
        .unwrap());

        route.retire();
        core.sessions.kill(&id).unwrap();
    }

    // 2. DB failure polled strictly BETWEEN root working and child id=ses_b:state=working:
    // foreign ses_a must not poison ses_b (Completed + badge), while matching ses_b settles
    // Failed with no badge.
    for (failing_session, expected_outcome, expect_unread) in [
        ("ses_a", TurnOutcome::Completed, true),
        ("ses_b", TurnOutcome::Failed, false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("opencode/opencode.db");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE session_v2(
                id TEXT PRIMARY KEY,
                time_created INTEGER NOT NULL,
                time_busy INTEGER,
                time_idle INTEGER,
                idle_outcome TEXT
            );",
        )
        .unwrap();

        let core = crate::test_support::test_core_with_runtime(fake_runtime());
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        let mut agent_role = role("opencode", &[]);
        agent_role.runtime = "opencode".into();
        agent_role.command = "fake".into();
        insert_role_row(&core.db.get().unwrap(), &agent_role);

        let spawned = core
            .sessions
            .spawn_direct(
                &agent_role,
                DirectLaunch {
                    cwd: Some(dir.path().to_str().unwrap().into()),
                    ..Default::default()
                },
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
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

        let launched_ms = crate::session::clock::state_now().wall_millis - 5_000;
        let (route, receiver) = TestHookRoute::new(Runtime::OpenCode, "between".into());
        let mut watcher = crate::runtimes::opencode::status::Watcher::with_db(
            receiver,
            None,
            Some(db_path.clone()),
            launched_ms,
        );
        let ingest = |seq: &str| {
            core.sessions.ingest_output_chunk(
                &id,
                None,
                format!("\x1b]7501;{seq}\x1b\\").as_bytes(),
                events.as_ref(),
            );
        };
        let mut drain = || {
            watcher
                .drain_events(
                    0,
                    &mut |ev| core.sessions.publish_agent_event(&id, ev, events.as_ref()),
                    &mut |_| {},
                )
                .unwrap();
        };

        ingest("state=idle:app=opencode");
        conn.execute(
            "INSERT INTO session_v2(id, time_created, time_busy) VALUES ('ses_a', ?1, ?1)",
            [launched_ms + 10],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_v2(id, time_created, time_busy) VALUES ('ses_b', ?1, ?1)",
            [launched_ms + 20],
        )
        .unwrap();
        // Visit busy ses_a, then switch to ses_b so both rows are active in the watcher.
        drop(
            route
                .admit(serde_json::json!({
                    "hook_event_name": "snapshot",
                    "epoch": 1,
                    "sequence": 1,
                    "session_id": "ses_a",
                }))
                .unwrap(),
        );
        drain();
        drop(
            route
                .admit(serde_json::json!({
                    "hook_event_name": "snapshot",
                    "epoch": 1,
                    "sequence": 2,
                    "session_id": "ses_b",
                }))
                .unwrap(),
        );
        drain();

        // Root working arrives FIRST, BEFORE child id=ses_b:state=working.
        ingest("state=working:app=opencode");
        let fail_ms = crate::session::clock::state_now().wall_millis + 10;
        conn.execute(
            "UPDATE session_v2 SET time_idle = ?1, idle_outcome = 'failed' WHERE id = ?2",
            rusqlite::params![fail_ms, failing_session],
        )
        .unwrap();
        drain();

        // Now child id=ses_b identifies the active participant, then root goes idle.
        ingest("id=ses_b:app=opencode:state=working");
        ingest("state=idle:app=opencode");
        ingest("state=clear:id=ses_b");

        let snap = &crate::ops::session::session_status_snapshot(&core).unwrap()[&id];
        assert_eq!(
            snap.observation.outcome,
            Some(expected_outcome),
            "failing_session={failing_session}"
        );
        assert_eq!(
            snap.unread_since.is_some(),
            expect_unread,
            "failing_session={failing_session}"
        );
        assert_eq!(
            crate::repo::session_attention::any_unread(
                &core.db.get().unwrap(),
                std::slice::from_ref(&id),
            )
            .unwrap(),
            expect_unread,
            "failing_session={failing_session}"
        );

        route.retire();
        core.sessions.kill(&id).unwrap();
    }

    // 3. Turn 1 succeeds on ses_a (with its DB succeeded poll preceding root idle) and leaves
    // durable unread U1 standing; Turn 2 root working arrives, non-superseded DB Failed for
    // ses_a lands BEFORE child ses_a working, then child ses_a working + root idle:
    // Turn 1's exact unread timestamp U1 remains in memory and SQLite, Turn 2 settles Failed,
    // and no second completion overwrites U1.
    {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("opencode/opencode.db");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE session_v2(
                id TEXT PRIMARY KEY,
                time_created INTEGER NOT NULL,
                time_busy INTEGER,
                time_idle INTEGER,
                idle_outcome TEXT
            );",
        )
        .unwrap();

        let core = crate::test_support::test_core_with_runtime(fake_runtime());
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        let mut agent_role = role("opencode", &[]);
        agent_role.runtime = "opencode".into();
        agent_role.command = "fake".into();
        insert_role_row(&core.db.get().unwrap(), &agent_role);

        let spawned = core
            .sessions
            .spawn_direct(
                &agent_role,
                DirectLaunch {
                    cwd: Some(dir.path().to_str().unwrap().into()),
                    ..Default::default()
                },
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
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

        let launched_ms = crate::session::clock::state_now().wall_millis - 5_000;
        let (route, receiver) = TestHookRoute::new(Runtime::OpenCode, "turn2-between".into());
        let mut watcher = crate::runtimes::opencode::status::Watcher::with_db(
            receiver,
            None,
            Some(db_path.clone()),
            launched_ms,
        );
        let ingest = |seq: &str| {
            core.sessions.ingest_output_chunk(
                &id,
                None,
                format!("\x1b]7501;{seq}\x1b\\").as_bytes(),
                events.as_ref(),
            );
        };
        let mut drain = || {
            watcher
                .drain_events(
                    0,
                    &mut |ev| core.sessions.publish_agent_event(&id, ev, events.as_ref()),
                    &mut |_| {},
                )
                .unwrap();
        };

        ingest("state=idle:app=opencode");
        conn.execute(
            "INSERT INTO session_v2(id, time_created, time_busy) VALUES ('ses_a', ?1, ?1)",
            [launched_ms + 10],
        )
        .unwrap();
        drop(
            route
                .admit(serde_json::json!({
                    "hook_event_name": "snapshot",
                    "epoch": 1,
                    "sequence": 1,
                    "session_id": "ses_a",
                }))
                .unwrap(),
        );
        drain();

        // Turn 1: DB succeeded poll precedes root idle; root idle records durable unread U1.
        ingest("state=working:app=opencode");
        ingest("id=ses_a:app=opencode:state=working");
        let t1_db_ms = crate::session::clock::state_now().wall_millis;
        conn.execute(
            "UPDATE session_v2 SET time_idle = ?1, idle_outcome = 'succeeded' WHERE id = 'ses_a'",
            [t1_db_ms],
        )
        .unwrap();
        drain();
        ingest("state=idle:app=opencode");
        ingest("state=clear:id=ses_a");
        let turn1_unread_mem = core.sessions.agent_status(&id).unread_since.unwrap();
        let turn1_unread_db =
            crate::repo::session_attention::unread_since(&core.db.get().unwrap(), &id)
                .unwrap()
                .unwrap();

        // Turn 2: root working arrives first; non-superseded DB Failed lands after Turn 2 start
        // and BEFORE child id=ses_a:state=working arrives.
        ingest("state=working:app=opencode");
        let t2_fail_ms = crate::session::clock::state_now().wall_millis.max(t1_db_ms) + 20;
        conn.execute(
            "UPDATE session_v2 SET time_busy = ?1, time_idle = ?2, idle_outcome = 'failed' WHERE id = 'ses_a'",
            rusqlite::params![t2_fail_ms - 5, t2_fail_ms],
        )
        .unwrap();
        drain();

        ingest("id=ses_a:app=opencode:state=working");
        ingest("state=idle:app=opencode");
        ingest("state=clear:id=ses_a");

        let snap = &crate::ops::session::session_status_snapshot(&core).unwrap()[&id];
        assert_eq!(snap.observation.outcome, Some(TurnOutcome::Failed));
        assert!(snap.failed_since.is_some());
        assert_eq!(
            core.sessions.agent_status(&id).unread_since,
            Some(turn1_unread_mem),
            "Turn 1's in-memory unread timestamp must stay intact"
        );
        assert_eq!(
            crate::repo::session_attention::unread_since(&core.db.get().unwrap(), &id).unwrap(),
            Some(turn1_unread_db),
            "Turn 1's durable SQLite unread timestamp must stay intact without a second completion"
        );
        assert_eq!(snap.unread_since, Some(turn1_unread_db));

        route.retire();
        core.sessions.kill(&id).unwrap();
    }
}

#[test]
fn opencode_permission_reject_via_session_message_overrides_completed_focused_and_unfocused() {
    use crate::session::hook_queue::TestHookRoute;

    for focused in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("opencode/opencode.db");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE session_v2(
                id TEXT PRIMARY KEY,
                time_created INTEGER NOT NULL,
                time_busy INTEGER,
                time_idle INTEGER,
                idle_outcome TEXT
            );
            CREATE TABLE session_message(
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                type TEXT NOT NULL,
                seq INTEGER NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL,
                data TEXT NOT NULL
            );",
        )
        .unwrap();

        let core = crate::test_support::test_core_with_runtime(fake_runtime());
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        let mut agent_role = role("opencode", &[]);
        agent_role.runtime = "opencode".into();
        agent_role.command = "fake".into();
        insert_role_row(&core.db.get().unwrap(), &agent_role);

        let spawned = core
            .sessions
            .spawn_direct(
                &agent_role,
                DirectLaunch {
                    cwd: Some(dir.path().to_str().unwrap().into()),
                    ..Default::default()
                },
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
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

        if focused {
            core.windows.register("main");
            core.windows.set_subjects(
                "main",
                vec![crate::windows::Subject::DirectChat(id.clone())],
            );
            core.windows.set_viewed_session("main", Some(&id));
            core.windows.mark_focused("main");
        }

        let launched_ms = crate::session::clock::state_now().wall_millis - 5_000;
        let (route, receiver) = TestHookRoute::new(Runtime::OpenCode, format!("reject-{focused}"));
        let mut watcher = crate::runtimes::opencode::status::Watcher::with_db(
            receiver,
            None,
            Some(db_path.clone()),
            launched_ms,
        );
        let ingest = |seq: &str| {
            core.sessions.ingest_output_chunk(
                &id,
                None,
                format!("\x1b]7501;{seq}\x1b\\").as_bytes(),
                events.as_ref(),
            );
        };
        let mut drain = || {
            watcher
                .drain_events(
                    0,
                    &mut |ev| core.sessions.publish_agent_event(&id, ev, events.as_ref()),
                    &mut |_| {},
                )
                .unwrap();
        };

        ingest("state=idle:app=opencode");
        let t1_busy = crate::session::clock::state_now().wall_millis;
        conn.execute(
            "INSERT INTO session_v2(id, time_created, time_busy) VALUES ('ses_r', ?1, ?1)",
            [t1_busy],
        )
        .unwrap();
        drop(
            route
                .admit(serde_json::json!({
                    "hook_event_name": "snapshot",
                    "epoch": 1,
                    "sequence": 1,
                    "session_id": "ses_r",
                }))
                .unwrap(),
        );
        drain();

        // Turn 1: permission approval appears, user clicks Reject (no Escape cancel key).
        // OpenCode 2.3.2 leaves session_v2.time_idle NULL and writes an assistant error row.
        ingest("state=working:app=opencode");
        ingest("id=ses_r:app=opencode:state=working");
        ingest("state=blocked:app=opencode:kind=permission:msg=UnVuIGJhc2g");
        ingest("state=idle:app=opencode");
        ingest("state=clear:id=ses_r");

        let t1_reject = crate::session::clock::state_now().wall_millis.max(t1_busy) + 10;
        let reject_data = serde_json::json!({
            "time": { "created": t1_busy, "completed": t1_reject },
            "finish": "error",
            "error": { "type": "aborted", "message": "Step interrupted" },
        })
        .to_string();
        conn.execute(
            "INSERT INTO session_message(id, session_id, type, seq, time_created, time_updated, data)
             VALUES ('msg_1', 'ses_r', 'assistant', 1, ?1, ?2, ?3)",
            rusqlite::params![t1_busy, t1_reject, reject_data],
        )
        .unwrap();
        drain();

        let snap = &crate::ops::session::session_status_snapshot(&core).unwrap()[&id];
        assert_eq!(
            snap.observation.outcome,
            Some(TurnOutcome::Interrupted),
            "focused={focused}: OpenCode Reject must reconcile to Interrupted"
        );
        assert_eq!(
            snap.unread_since, None,
            "focused={focused}: OpenCode Reject must leave no unread badge"
        );
        assert!(
            !crate::repo::session_attention::any_unread(
                &core.db.get().unwrap(),
                std::slice::from_ref(&id),
            )
            .unwrap(),
            "focused={focused}: SQLite attention must roll back optimistic unread on Reject"
        );

        // Turn 2 on the same session succeeds cleanly and (when unfocused) records unread.
        let t2_busy = crate::session::clock::state_now()
            .wall_millis
            .max(t1_reject)
            + 20;
        conn.execute(
            "UPDATE session_v2 SET time_busy = ?1 WHERE id = 'ses_r'",
            [t2_busy],
        )
        .unwrap();
        ingest("state=working:app=opencode");
        ingest("id=ses_r:app=opencode:state=working");
        drain();

        let t2_idle = crate::session::clock::state_now().wall_millis.max(t2_busy) + 10;
        conn.execute(
            "UPDATE session_v2 SET time_idle = ?1, idle_outcome = 'succeeded' WHERE id = 'ses_r'",
            [t2_idle],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_message(id, session_id, type, seq, time_created, time_updated, data)
             VALUES ('msg_2', 'ses_r', 'idle', 2, ?1, ?1, '{\"outcome\":\"succeeded\"}')",
            [t2_idle],
        )
        .unwrap();
        ingest("state=idle:app=opencode");
        ingest("state=clear:id=ses_r");
        drain();

        let snap = &crate::ops::session::session_status_snapshot(&core).unwrap()[&id];
        assert_eq!(
            snap.observation.outcome,
            Some(TurnOutcome::Completed),
            "focused={focused}: subsequent turn must complete normally"
        );
        assert_eq!(
            snap.unread_since.is_some(),
            !focused,
            "focused={focused}: unfocused subsequent completion must surface unread"
        );
        assert_eq!(
            crate::repo::session_attention::any_unread(
                &core.db.get().unwrap(),
                std::slice::from_ref(&id),
            )
            .unwrap(),
            !focused,
            "focused={focused}: SQLite unread must match focus state on subsequent completion"
        );

        drop(conn);
        route.retire();
        core.sessions.kill(&id).unwrap();
    }
}

#[test]
fn opencode_turn1_failed_and_turn2_rejected_same_poll_reverts_turn1_unread() {
    use crate::session::hook_queue::TestHookRoute;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("opencode/opencode.db");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch(
        "CREATE TABLE session_v2(
            id TEXT PRIMARY KEY,
            time_created INTEGER NOT NULL,
            time_busy INTEGER,
            time_idle INTEGER,
            idle_outcome TEXT
        );
        CREATE TABLE session_message(
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            type TEXT NOT NULL,
            seq INTEGER NOT NULL,
            time_created INTEGER NOT NULL,
            time_updated INTEGER NOT NULL,
            data TEXT NOT NULL
        );",
    )
    .unwrap();

    let core = crate::test_support::test_core_with_runtime(fake_runtime());
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    let mut agent_role = role("opencode", &[]);
    agent_role.runtime = "opencode".into();
    agent_role.command = "fake".into();
    insert_role_row(&core.db.get().unwrap(), &agent_role);

    let spawned = core
        .sessions
        .spawn_direct(
            &agent_role,
            DirectLaunch {
                cwd: Some(dir.path().to_str().unwrap().into()),
                ..Default::default()
            },
            dir.path(),
            Arc::clone(&core.db),
            Arc::clone(&events),
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

    let launched_ms = crate::session::clock::state_now().wall_millis - 5_000;
    let (route, receiver) = TestHookRoute::new(Runtime::OpenCode, "both-same-poll".into());
    let mut watcher = crate::runtimes::opencode::status::Watcher::with_db(
        receiver,
        None,
        Some(db_path.clone()),
        launched_ms,
    );
    let ingest = |seq: &str| {
        core.sessions.ingest_output_chunk(
            &id,
            None,
            format!("\x1b]7501;{seq}\x1b\\").as_bytes(),
            events.as_ref(),
        );
    };
    let mut drain = || {
        watcher
            .drain_events(
                0,
                &mut |ev| core.sessions.publish_agent_event(&id, ev, events.as_ref()),
                &mut |_| {},
            )
            .unwrap();
    };

    ingest("state=idle:app=opencode");
    let t1_busy = crate::session::clock::state_now().wall_millis;
    conn.execute(
        "INSERT INTO session_v2(id, time_created, time_busy) VALUES ('ses_r', ?1, ?1)",
        [t1_busy],
    )
    .unwrap();
    drop(
        route
            .admit(serde_json::json!({
                "hook_event_name": "snapshot",
                "epoch": 1,
                "sequence": 1,
                "session_id": "ses_r",
            }))
            .unwrap(),
    );
    drain();

    // Turn 1 settles idle via OSC (recording optimistic unread) before opencode.db is polled.
    ingest("state=working:app=opencode");
    ingest("id=ses_r:app=opencode:state=working");
    ingest("state=idle:app=opencode");
    ingest("state=clear:id=ses_r");
    assert!(
        crate::repo::session_attention::any_unread(
            &core.db.get().unwrap(),
            std::slice::from_ref(&id),
        )
        .unwrap(),
        "Turn 1 optimistic idle must record unread before DB reconciliation"
    );
    let t1_fail = crate::session::clock::state_now().wall_millis.max(t1_busy) + 10;

    // Turn 2 starts and hits a permission Reject without advancing session_v2.time_idle
    // BEFORE the watcher polls Turn 1's failure.
    ingest("state=working:app=opencode");
    ingest("id=ses_r:app=opencode:state=working");
    ingest("state=blocked:app=opencode:kind=permission:msg=UnVuIGJhc2g");

    let t2_busy = crate::session::clock::state_now().wall_millis.max(t1_fail) + 20;
    let t2_reject = t2_busy + 10;
    conn.execute(
        "UPDATE session_v2 SET time_idle = ?1, idle_outcome = 'failed', time_busy = ?2 WHERE id = 'ses_r'",
        rusqlite::params![t1_fail, t2_busy],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO session_message(id, session_id, type, seq, time_created, time_updated, data)
         VALUES ('m_1', 'ses_r', 'idle', 1, ?1, ?1, '{\"outcome\":\"failed\"}')",
        [t1_fail],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO session_message(id, session_id, type, seq, time_created, time_updated, data)
         VALUES ('m_2', 'ses_r', 'assistant', 2, ?1, ?2, ?3)",
        rusqlite::params![
            t2_busy,
            t2_reject,
            serde_json::json!({
                "time": { "created": t2_busy, "completed": t2_reject },
                "finish": "error",
                "error": { "type": "aborted", "message": "Step interrupted" },
            })
            .to_string(),
        ],
    )
    .unwrap();

    // Single poll sees BOTH Turn 1 Failed (rolling back Turn 1's optimistic unread) AND
    // Turn 2 Interrupted (attaching to the active Turn 2).
    drain();
    ingest("state=idle:app=opencode");
    ingest("state=clear:id=ses_r");
    drain();

    let snap = &crate::ops::session::session_status_snapshot(&core).unwrap()[&id];
    assert_eq!(snap.observation.outcome, Some(TurnOutcome::Interrupted));
    assert_eq!(
        snap.unread_since, None,
        "Turn 1's optimistic unread must be rolled back when Turn 1 Failed and Turn 2 Rejected arrive on the same poll"
    );
    assert!(
        !crate::repo::session_attention::any_unread(
            &core.db.get().unwrap(),
            std::slice::from_ref(&id),
        )
        .unwrap(),
        "SQLite attention must have no surviving false Turn 1 unread"
    );

    drop(conn);
    route.retire();
    core.sessions.kill(&id).unwrap();
}

#[test]
fn grok_permission_reject_without_transcript_path_discovers_updates_jsonl_and_settles_interrupted()
{
    use crate::session::hook_queue::TestHookRoute;
    use std::io::Write;

    for (focused, transcript_before_idle) in [(true, false), (false, false), (false, true)] {
        let dir = tempfile::tempdir().unwrap();
        let grok_home = dir.path().join(".grok");
        let core = crate::test_support::test_core_with_runtime(fake_runtime());
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        let mut agent_role = role("grok", &[]);
        agent_role.runtime = "grok".into();
        agent_role.command = "fake".into();
        insert_role_row(&core.db.get().unwrap(), &agent_role);

        let spawned = core
            .sessions
            .spawn_direct(
                &agent_role,
                DirectLaunch {
                    cwd: Some(dir.path().to_str().unwrap().into()),
                    ..Default::default()
                },
                dir.path(),
                Arc::clone(&core.db),
                Arc::clone(&events),
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

        if focused {
            core.windows.register("main");
            core.windows.set_subjects(
                "main",
                vec![crate::windows::Subject::DirectChat(id.clone())],
            );
            core.windows.set_viewed_session("main", Some(&id));
            core.windows.mark_focused("main");
        }

        let grok_session_id = uuid::Uuid::new_v4().to_string();
        let launched_ms = crate::session::clock::state_now().wall_millis - 5_000;
        let (route, receiver) = TestHookRoute::new(
            Runtime::Grok,
            format!("grok-reject-{focused}-{transcript_before_idle}"),
        );
        let mut watcher = crate::runtimes::grok::status::Watcher::with_home(
            receiver,
            Some(grok_home.clone()),
            launched_ms,
        );
        let ingest = |seq: &str| {
            core.sessions.ingest_output_chunk(
                &id,
                None,
                format!("\x1b]7501;{seq}\x1b\\").as_bytes(),
                events.as_ref(),
            );
        };
        let mut drain = || {
            watcher
                .drain_events(
                    0,
                    &mut |ev| core.sessions.publish_agent_event(&id, ev, events.as_ref()),
                    &mut |_| {},
                )
                .unwrap();
        };

        // Grok 1.0.89 SessionStart omits transcriptPath and updates.jsonl is not yet created.
        drop(
            route
                .admit(serde_json::json!({
                    "hook_event_name": "SessionStart",
                    "sessionId": grok_session_id,
                    "cwd": dir.path(),
                }))
                .unwrap(),
        );
        drain();
        ingest("state=idle:app=grok");

        // Turn 1: UserPromptSubmit omits transcriptPath; Grok creates updates.jsonl under
        // <grok_home>/sessions/<encoded-cwd>/<session_id>/updates.jsonl.
        drop(
            route
                .admit(serde_json::json!({
                    "hook_event_name": "UserPromptSubmit",
                    "sessionId": grok_session_id,
                    "promptId": "p_reject",
                    "cwd": dir.path(),
                    "prompt": "run bash",
                }))
                .unwrap(),
        );
        drain();

        let canonical_cwd = dir
            .path()
            .canonicalize()
            .unwrap_or_else(|_| dir.path().to_path_buf());
        let encoded_cwd: String = canonical_cwd
            .to_string_lossy()
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect();
        let session_dir = grok_home
            .join("sessions")
            .join(&encoded_cwd)
            .join(&grok_session_id);
        std::fs::create_dir_all(&session_dir).unwrap();
        let updates_path = session_dir.join("updates.jsonl");
        std::fs::write(&updates_path, "").unwrap();

        ingest("state=working:app=grok:msg=VGhpbmtpbmc");
        ingest("state=blocked:app=grok:kind=permission:msg=UnVuIENvbW1hbmQ");

        // User presses '4' (No, reject) — NOT Ctrl+C or Escape.
        core.sessions
            .inject_direct_stdin(&id, b"4", events.as_ref())
            .unwrap();
        ingest("state=working:app=grok");

        let now_ms = crate::session::clock::state_now().wall_millis;
        let write_reject_record = || {
            writeln!(
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(&updates_path)
                    .unwrap(),
                "{}",
                serde_json::json!({
                    "timestamp": now_ms / 1_000,
                    "timestamp_us": now_ms * 1_000,
                    "method": "_x.ai/session/update",
                    "params": {
                        "sessionId": grok_session_id,
                        "update": {
                            "sessionUpdate": "turn_completed",
                            "prompt_id": "p_reject",
                            "stop_reason": "cancelled",
                        },
                        "_meta": {
                            "agentTimestampMs": now_ms,
                            "cancellationCategory": "PermissionRejected",
                        }
                    }
                })
            )
            .unwrap();
        };

        if transcript_before_idle {
            write_reject_record();
            drain();
            ingest("state=idle:app=grok");
        } else {
            ingest("state=idle:app=grok");
            write_reject_record();
            drain();
        }

        let snap = &crate::ops::session::session_status_snapshot(&core).unwrap()[&id];
        assert_eq!(
            snap.observation.outcome,
            Some(TurnOutcome::Interrupted),
            "focused={focused} transcript_before_idle={transcript_before_idle}: Grok Reject must settle Interrupted"
        );
        assert_eq!(snap.unread_since, None);
        assert!(!crate::repo::session_attention::any_unread(
            &core.db.get().unwrap(),
            std::slice::from_ref(&id),
        )
        .unwrap());

        // Turn 2 completes cleanly via OSC done and records unread when unfocused.
        drop(
            route
                .admit(serde_json::json!({
                    "hook_event_name": "UserPromptSubmit",
                    "sessionId": grok_session_id,
                    "promptId": "p_ok",
                    "cwd": dir.path(),
                }))
                .unwrap(),
        );
        drain();
        ingest("state=working:app=grok");
        ingest("state=done:app=grok");

        let snap = &crate::ops::session::session_status_snapshot(&core).unwrap()[&id];
        assert_eq!(snap.observation.outcome, Some(TurnOutcome::Completed));
        assert_eq!(snap.unread_since.is_some(), !focused);

        route.retire();
        core.sessions.kill(&id).unwrap();
    }
}
