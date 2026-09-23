use super::*;

#[test]
fn ordered_interrupt_is_idle_only_for_a_busy_hook_session() {
    let manager = mgr_with_fake(None, fake_runtime());
    for session_id in ["hooks", "baseline"] {
        install_test_session_handle(&manager, session_id);
        manager.arm_completion(session_id);
    }
    assert!(manager.note_forwarder_transition("baseline", SessionActivityState::Busy, "forwarder"));
    assert!(!manager.note_forwarder_transition(
        "baseline",
        SessionActivityState::Idle,
        "input-interrupt"
    ));
    assert!(manager.take_completion_armed(&["baseline".into()]));
    assert!(manager.note_forwarder_transition("hooks", SessionActivityState::Busy, "hook"));
    assert!(manager.note_forwarder_transition(
        "hooks",
        SessionActivityState::Idle,
        "input-interrupt"
    ));
    assert!(!manager.take_completion_armed(&["hooks".into()]));
    assert!(!manager.note_forwarder_transition(
        "hooks",
        SessionActivityState::Idle,
        "input-interrupt"
    ));
    assert!(!manager.note_forwarder_transition("hooks", SessionActivityState::Busy, "forwarder"));
    assert_eq!(
        manager.activity_snapshot()["hooks"],
        SessionActivityState::Idle
    );
    assert!(manager.note_forwarder_transition("hooks", SessionActivityState::Busy, "hook"));
}

#[test]
fn escape_preserves_input_but_only_continued_work_can_complete() {
    for continues in [false, true] {
        let manager = mgr_with_fake(None, fake_runtime());
        for id in ["hooks", "other"] {
            install_test_session_handle(&manager, id);
            manager.arm_completion(id);
        }
        assert!(manager.note_forwarder_transition(
            "other",
            SessionActivityState::Idle,
            "forwarder"
        ));
        assert!(manager.note_forwarder_transition("hooks", SessionActivityState::Busy, "hook"));
        let since = Instant::now();
        {
            let session = manager.session_state("hooks").unwrap();
            let mut session = session.lock().unwrap();
            session.local_input_pending = true;
            session.observed_input = Some(ObservedInput {
                state: InputState::Drafting,
                since,
            });
        }
        manager
            .inject_direct_stdin("hooks", b"\x1b", capture().as_ref())
            .unwrap();
        assert!(manager.note_forwarder_transition(
            "hooks",
            SessionActivityState::Idle,
            "input-escape"
        ));
        assert_eq!(
            manager.activity_snapshot()["hooks"],
            SessionActivityState::Idle
        );
        assert!(!manager.take_completion_armed(&["hooks".into()]));
        assert!(manager.take_completion_armed(&["hooks".into(), "other".into()]));
        {
            let session = manager.session_state("hooks").unwrap();
            let session = session.lock().unwrap();
            assert!(session.local_input_pending);
            assert_eq!(session.observed_input.unwrap().state, InputState::Drafting);
            assert_eq!(session.observed_input.unwrap().since, since);
            assert!(!session.completion_armed);
        }
        if continues {
            assert!(manager.note_forwarder_transition("hooks", SessionActivityState::Busy, "hook"));
            manager.arm_completion("hooks");
        }
        assert!(manager.note_forwarder_transition("hooks", SessionActivityState::Idle, "hook"));
        assert_eq!(manager.take_completion_armed(&["hooks".into()]), continues);
        assert!(!manager.take_completion_armed(&["other".into()]));
        assert!(!manager.take_completion_armed(&["hooks".into(), "other".into()]));
    }
}

#[test]
fn ctrl_c_after_provisional_escape_cancels_completion() {
    let manager = mgr_with_fake(None, fake_runtime());
    install_test_session_handle(&manager, "hooks");
    manager.arm_completion("hooks");
    manager.note_forwarder_transition("hooks", SessionActivityState::Busy, "hook");
    manager.note_forwarder_transition("hooks", SessionActivityState::Idle, "input-escape");
    assert!(!manager.note_forwarder_transition(
        "hooks",
        SessionActivityState::Idle,
        "input-interrupt"
    ));
    assert!(!manager.take_completion_armed(&["hooks".into()]));
    assert!(
        !manager
            .session_state("hooks")
            .unwrap()
            .lock()
            .unwrap()
            .provisional_idle
    );
}

#[test]
fn failed_interrupt_write_preserves_busy_and_completion_state() {
    let manager =
        manager_with_runtime(crate::shell_path::LoginShellEnv::default(), inert_runtime());
    let events = capture();
    install_test_session_handle(&manager, "failed-interrupt");
    assert!(manager.note_forwarder_transition(
        "failed-interrupt",
        SessionActivityState::Busy,
        "hook"
    ));
    manager.arm_completion("failed-interrupt");
    assert!(manager
        .inject_direct_stdin("failed-interrupt", b"\x03", events.as_ref())
        .is_err());
    assert_eq!(
        manager.activity_snapshot()["failed-interrupt"],
        SessionActivityState::Busy
    );
    assert!(manager.take_completion_armed(&["failed-interrupt".into()]));
    assert!(events.status.lock().unwrap().is_empty());
}

#[test]
fn hook_status_owns_activity_until_teardown_without_changing_submit_or_wake() {
    let manager =
        manager_with_runtime(crate::shell_path::LoginShellEnv::default(), inert_runtime());
    install_test_session_handle(&manager, "hooks");
    install_test_session_handle(&manager, "baseline");
    assert!(manager.note_forwarder_transition("hooks", SessionActivityState::Busy, "forwarder"));
    // Source changes must reach attached views even when activity is unchanged.
    assert!(manager.note_forwarder_transition("hooks", SessionActivityState::Busy, "hook"));
    assert!(!manager.note_forwarder_transition("hooks", SessionActivityState::Idle, "forwarder"));
    assert_eq!(
        manager.activity_snapshot()["hooks"],
        SessionActivityState::Busy
    );
    assert!(manager.note_forwarder_transition("hooks", SessionActivityState::Idle, "hook"));
    assert!(!manager.note_forwarder_transition("hooks", SessionActivityState::Busy, "forwarder"));
    assert!(manager.note_forwarder_transition("hooks", SessionActivityState::Busy, "input-submit"));
    assert!(manager.note_forwarder_transition("hooks", SessionActivityState::Idle, "hook"));
    assert!(manager.note_forwarder_transition("hooks", SessionActivityState::Busy, "wake"));
    assert!(manager.note_forwarder_transition("baseline", SessionActivityState::Idle, "forwarder"));
    assert!(manager.note_forwarder_transition("baseline", SessionActivityState::Busy, "forwarder"));

    assert!(manager.note_forwarder_transition("hooks", SessionActivityState::Idle, "input-escape"));
    let runtime_session = manager.live_runtime_session("hooks").unwrap();
    manager
        .forget_runtime_handle("hooks", &runtime_session)
        .unwrap();
    assert!(!manager.note_forwarder_transition("hooks", SessionActivityState::Idle, "hook"));
    assert!(!manager.activity_snapshot().contains_key("hooks"));
    assert!(!manager
        .session_state("hooks")
        .is_some_and(|session| session.lock().unwrap().provisional_idle));
    install_test_session_handle(&manager, "hooks");
    assert!(manager.note_forwarder_transition("hooks", SessionActivityState::Busy, "forwarder"));
    assert!(manager.note_forwarder_transition("hooks", SessionActivityState::Idle, "forwarder"));
}

#[test]
fn healthy_hook_owned_work_ignores_title_fallback_transitions() {
    let manager = mgr_with_fake(None, fake_runtime());
    install_test_session_handle(&manager, "hooks");
    assert!(manager.note_forwarder_transition("hooks", SessionActivityState::Busy, "hook"));
    assert!(!manager.note_forwarder_transition("hooks", SessionActivityState::Idle, "forwarder"));
    assert_eq!(
        manager.agent_status("hooks").observation.activity,
        Activity::Working
    );
    assert_eq!(
        manager.agent_status("hooks").observation.source,
        ObservationSource::Hook
    );
}

#[test]
fn hook_status_uses_existing_direct_and_mission_consumers() {
    assert_status_uses_existing_direct_and_mission_consumers("hook");
}

#[test]
fn ordered_interrupt_uses_existing_direct_and_mission_consumers() {
    assert_status_uses_existing_direct_and_mission_consumers("input-interrupt");
}

#[test]
fn provisional_escape_uses_existing_direct_and_mission_consumers() {
    assert_status_uses_existing_direct_and_mission_consumers("input-escape");
}

fn assert_status_uses_existing_direct_and_mission_consumers(source: &'static str) {
    for is_mission in [false, true] {
        let pool = pool_with_schema();
        let (mission, role, slot) = single_slot_mission(&pool);
        let app_data = tempfile::tempdir().unwrap();
        let events_path = runner_core::event_log::path::events_path(
            app_data.path(),
            &mission.crew_id,
            &mission.id,
        );
        let mission_dir = runner_core::event_log::path::mission_dir(
            app_data.path(),
            &mission.crew_id,
            &mission.id,
        );
        let fake = fake_runtime();
        let mgr = mgr_with_fake(None, Arc::clone(&fake));
        let cap = capture();
        let spawned = if is_mission {
            mgr.spawn(
                &mission,
                &role,
                &slot,
                app_data.path(),
                events_path,
                Arc::clone(&pool),
                cap.clone(),
                None,
            )
            .unwrap()
        } else {
            mgr.spawn_direct(
                &role,
                None,
                None,
                None,
                None,
                Some(fixture_tmp_dir().to_str().unwrap()),
                None,
                None,
                app_data.path(),
                Arc::clone(&pool),
                cap.clone(),
                None,
            )
            .unwrap()
        };
        for state in [
            SessionActivityState::Busy,
            SessionActivityState::Idle,
            SessionActivityState::Busy,
            SessionActivityState::Idle,
        ] {
            for _ in 0..2 {
                let event_source = if matches!(source, "input-interrupt" | "input-escape")
                    && state == SessionActivityState::Idle
                {
                    source
                } else {
                    "hook"
                };
                fake.push_status_from(0, state, event_source);
            }
            fake.push_status(0, SessionActivityState::Busy);
            fake.push_status(0, SessionActivityState::Idle);
        }
        fake.close_spawn(0);
        join_forwarder_for_test(&mgr, &spawned.id);

        if is_mission {
            let statuses: Vec<_> = EventLog::open(&mission_dir)
                .unwrap()
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
                .map(|event| {
                    (
                        event.payload["state"].as_str().unwrap().to_owned(),
                        event.payload["source"].as_str().unwrap().to_owned(),
                    )
                })
                .collect();
            assert_eq!(
                statuses,
                std::iter::once(("busy".to_owned(), "spawn".to_owned()))
                    .chain(["busy", "idle", "busy", "idle"].map(|state| {
                        let event_source = if matches!(source, "input-interrupt" | "input-escape")
                            && state == "busy"
                        {
                            "hook"
                        } else {
                            source
                        };
                        (state.to_owned(), event_source.to_owned())
                    }))
                    .collect::<Vec<_>>()
            );
            let incremental: Vec<_> = cap
                .status
                .lock()
                .unwrap()
                .iter()
                .map(|event| {
                    (
                        (if event.state == SessionActivityState::Busy {
                            "busy"
                        } else {
                            "idle"
                        })
                        .to_owned(),
                        event.source.clone(),
                    )
                })
                .collect();
            assert_eq!(incremental, statuses);
        } else {
            let statuses: Vec<_> = cap
                .status
                .lock()
                .unwrap()
                .iter()
                .map(|event| (event.state, event.source.clone()))
                .collect();
            let mut expected = vec![
                (SessionActivityState::Busy, "spawn".to_owned()),
                (SessionActivityState::Idle, source.to_owned()),
                (
                    SessionActivityState::Busy,
                    if matches!(source, "input-interrupt" | "input-escape") {
                        "hook"
                    } else {
                        source
                    }
                    .to_owned(),
                ),
                (SessionActivityState::Idle, source.to_owned()),
            ];
            if matches!(source, "hook" | "input-interrupt" | "input-escape") {
                expected.insert(1, (SessionActivityState::Busy, "hook".to_owned()));
            }
            assert_eq!(statuses, expected);
        }
        assert!(!mgr.activity_snapshot().contains_key(&spawned.id));
    }
}

#[test]
fn mission_spawn_seeds_status_and_allows_hook_takeover() {
    let pool = pool_with_schema();
    let (mission, role, slot) = single_slot_mission(&pool);
    let app_data = tempfile::tempdir().unwrap();
    let events_path =
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
            events_path,
            Arc::clone(&pool),
            Arc::clone(&cap) as Arc<dyn SessionEvents>,
            None,
        )
        .unwrap();

    let seeded = wait_for_session_status_event(&cap, &spawned.id, SessionActivityState::Busy);
    assert_eq!(seeded.source, "spawn");
    assert_eq!(seeded.status.observation.activity, Activity::Working);
    assert_eq!(
        seeded.status.observation.source,
        ObservationSource::Baseline
    );

    let statuses: Vec<_> = EventLog::open(&mission_dir)
        .unwrap()
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
    assert_eq!(statuses[0].from, slot.slot_handle);
    assert_eq!(statuses[0].payload["state"], "busy");
    assert_eq!(statuses[0].payload["source"], "spawn");

    cap.status.lock().unwrap().clear();
    fake.push_status_from(0, SessionActivityState::Busy, "hook");
    let hook = wait_for_session_status_event(&cap, &spawned.id, SessionActivityState::Busy);
    assert_eq!(hook.source, "hook");

    mgr.kill(&spawned.id).unwrap();
}
