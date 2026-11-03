use super::*;

#[test]
fn inject_stdin_roundtrip_routes_through_runtime() {
    // After the Step 9 cutover, inject_stdin no longer writes to
    // a master PTY — it routes through `runtime.send_bytes`
    // (literal byte stream) or `runtime.send_key("Enter")` (the
    // bare `\r` carve-out). FakeRuntime captures both; assert
    // the byte payload landed in `bytes_writes`, then bare `\r`
    // routed as a key press, then kill flips the row.
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
    mgr.inject_stdin(&spawned.id, b"hello\n").unwrap();
    mgr.inject_stdin(&spawned.id, b"\r").unwrap();

    let writes = fake.bytes_writes();
    assert!(
        writes.iter().any(|(_, bytes)| bytes == b"hello\n"),
        "send_bytes should have captured hello\\n; got = {writes:?}",
    );
    let keys = fake.keys();
    assert!(
        keys.iter().any(|(_, k)| k == "Enter"),
        "bare \\r should route as send_key(Enter); got = {keys:?}",
    );

    mgr.kill(&spawned.id).unwrap();

    // After kill, forwarder thread exits and flips the row.
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let conn = pool.get().unwrap();
        let status: String = conn
            .query_row(
                "SELECT status FROM sessions WHERE id = ?1",
                params![spawned.id],
                |r| r.get(0),
            )
            .unwrap();
        if status != "running" {
            break;
        }
        if Instant::now() > deadline {
            panic!("session never exited after kill");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn inject_stdin_on_unknown_session_errors_cleanly() {
    let mgr = manager_with_runtime(crate::shell_path::LoginShellEnv::default(), inert_runtime());
    let err = mgr.inject_stdin("nope", b"x").unwrap_err();
    assert!(format!("{err}").contains("session not found"));
}

#[test]
fn local_input_byte_classes_remain_the_unobserved_fallback() {
    use super::output::{classify_local_input, update_local_input_state, LocalInputClass};

    assert_eq!(
        classify_local_input(b"x"),
        Some(LocalInputClass::SetPending)
    );
    assert_eq!(
        classify_local_input("界".as_bytes()),
        Some(LocalInputClass::SetPending)
    );
    for protocol in [
        b"\x1b[A".as_slice(),
        b"\x1b",
        b"\x1b[27u",
        b"\x1bd",
        b"\x1b[100;3u",
        b"\x1b[13;2u",
        b"\x1b[27u\x1b[27u",
        b"\x1b]10;rgb:dcdc/dcdc/e0e0\x1b\\",
        b"\x1b]11;rgb:1515/1616/1b1b\x1b\\",
    ] {
        assert_eq!(
            classify_local_input(protocol),
            Some(LocalInputClass::ActivityOnly),
            "terminal protocol traffic must not mark local input pending"
        );
    }
    for paste in [
        b"\x1b[200~pasted text\x1b[201~".as_slice(),
        b"\x1b[200~\x03\x1b\x1b[27u\x1b[99;5u\x1b[201~",
    ] {
        assert_eq!(
            classify_local_input(paste),
            Some(LocalInputClass::SetPending)
        );
    }
    assert_eq!(
        classify_local_input(b"\x16"),
        Some(LocalInputClass::SetPending)
    );
    assert_eq!(
        classify_local_input(b"\r"),
        Some(LocalInputClass::ClearPending)
    );
    for interrupt in [b"\x03".as_slice(), b"\x1b[99;5u"] {
        assert_eq!(
            classify_local_input(interrupt),
            Some(LocalInputClass::ClearPending)
        );
    }

    let now = Instant::now();
    let mut state = SessionState::default();
    assert!(state.model.observed_input().is_none());
    update_local_input_state(&mut state, classify_local_input(b"draft"), now);
    assert!(state.model.local_input_pending());
    assert_eq!(state.model.last_local_input_at(), Some(now));

    update_local_input_state(&mut state, classify_local_input(b"\r"), now);
    assert!(!state.model.local_input_pending());
    assert!(state.model.last_local_input_at().is_none());

    update_local_input_state(&mut state, classify_local_input(b"\x1b[D"), now);
    assert!(!state.model.local_input_pending());
    assert_eq!(state.model.last_local_input_at(), Some(now));

    state.model.apply(
        SessionEvent::TestPending(true),
        crate::session::clock::state_now(),
    );
    for escape in [b"\x1b".as_slice(), b"\x1b[27u"] {
        update_local_input_state(&mut state, classify_local_input(escape), now);
        assert!(state.model.local_input_pending());
        assert_eq!(state.model.last_local_input_at(), Some(now));
    }
    for interrupt in [b"\x03".as_slice(), b"\x1b[99;5u"] {
        state.model.apply(
            SessionEvent::TestPending(true),
            crate::session::clock::state_now(),
        );
        update_local_input_state(&mut state, classify_local_input(interrupt), now);
        assert!(!state.model.local_input_pending());
        assert!(state.model.last_local_input_at().is_none());
    }
}

#[test]
fn enter_while_observed_idle_is_activity_only() {
    use super::output::{classify_local_input, update_local_input_state};

    let now = Instant::now();
    let mut state = SessionState::default();
    state.model.apply(
        SessionEvent::TestObserved(ObservedInput {
            state: InputState::Idle,
            since: now,
            composer_visible: true,
        }),
        crate::session::clock::state_now(),
    );
    update_local_input_state(&mut state, classify_local_input(b"\r"), now);
    assert!(!state.model.local_input_pending());
    assert_eq!(state.model.last_local_input_at(), Some(now));
}

#[test]
fn observed_input_tier_precedes_the_byte_latch_and_hidden_drafts_park() {
    let manager =
        manager_with_runtime(crate::shell_path::LoginShellEnv::default(), inert_runtime());
    let session_id = "observed-input";
    install_test_session_handle(&manager, session_id);
    manager.note_forwarder_transition(
        session_id,
        SessionActivityState::Idle,
        StatusSource::Forwarder,
    );
    {
        let state = manager.session_state(session_id).unwrap();
        let mut state = state.lock().unwrap();
        state.model.apply(
            SessionEvent::TestPending(true),
            crate::session::clock::state_now(),
        );
        state.model.apply(
            SessionEvent::TestInputAt(None),
            crate::session::clock::state_now(),
        );
    }
    assert_eq!(
        manager.reserve_delivery(session_id).unwrap(),
        router::DeliveryReservation::LocalInputPending,
        "None must retain the byte latch exactly as the fallback tier"
    );

    let now = Instant::now();
    manager.report_input_state(
        session_id,
        InputObservation {
            state: InputState::Idle,
            since: now,
            composing: false,
            composer_visible: true,
        },
    );
    let token = match manager.reserve_delivery(session_id).unwrap() {
        router::DeliveryReservation::Ready(token) => token,
        other => panic!("observed Idle must override a stale byte latch, got {other:?}"),
    };
    manager.finish_delivery(session_id, token);

    manager
        .session_state(session_id)
        .unwrap()
        .lock()
        .unwrap()
        .model
        .apply(
            SessionEvent::TestInputAt(Some(now)),
            crate::session::clock::state_now(),
        );

    manager.report_input_state(
        session_id,
        InputObservation {
            state: InputState::Drafting,
            since: now,
            composing: false,
            composer_visible: false,
        },
    );
    assert_eq!(
        manager.reserve_delivery(session_id).unwrap(),
        router::DeliveryReservation::Drafting {
            composer_visible: false
        }
    );
    manager.report_input_state(
        session_id,
        InputObservation {
            state: InputState::Submitted,
            since: Instant::now(),
            composing: false,
            composer_visible: true,
        },
    );
    assert!(matches!(
        manager.reserve_delivery(session_id).unwrap(),
        router::DeliveryReservation::RecentlyTyping(_)
    ));

    manager.report_input_state(
        session_id,
        InputObservation {
            state: InputState::Submitted,
            since: Instant::now() - RECENT_LOCAL_INPUT_WINDOW,
            composing: false,
            composer_visible: true,
        },
    );
    manager
        .session_state(session_id)
        .unwrap()
        .lock()
        .unwrap()
        .model
        .apply(
            SessionEvent::TestInputAt(Some(Instant::now())),
            crate::session::clock::state_now(),
        );
    assert!(matches!(
        manager.reserve_delivery(session_id).unwrap(),
        router::DeliveryReservation::RecentlyTyping(_)
    ));
}

#[derive(Default)]
struct DeliveryEventCapture(Mutex<Vec<router::SessionDeliveryEvent>>);

impl router::SessionDeliveryListener for DeliveryEventCapture {
    fn session_delivery_event(&self, _session_id: &str, event: router::SessionDeliveryEvent) {
        self.0.lock().unwrap().push(event);
    }
}

#[test]
fn a_draft_holds_a_report_past_the_inbox_cap_until_it_is_cleared() {
    let manager =
        manager_with_runtime(crate::shell_path::LoginShellEnv::default(), inert_runtime());
    let id = "report-draft";
    install_test_session_handle(&manager, id);
    let start = Instant::now();
    crate::session::clock::at(start, 1000, || {
        let state = manager.session_state(id).unwrap();
        let mut state = state.lock().unwrap();
        state.model.apply(
            SessionEvent::Transition {
                state: SessionActivityState::Idle,
                source: StatusSource::Forwarder,
                live: true,
            },
            crate::session::clock::state_now(),
        );
        state.model.apply(
            SessionEvent::TestInputAt(Some(start)),
            crate::session::clock::state_now(),
        );
    });
    let observe = |state| {
        manager.report_input_state(
            id,
            InputObservation {
                state,
                since: start,
                composing: false,
                composer_visible: true,
            },
        )
    };
    observe(InputState::Drafting);
    for seconds in [599, 600, 24 * 3600] {
        crate::session::clock::at(
            start + Duration::from_secs(seconds),
            1000 + seconds as i64 * 1000,
            || {
                assert_eq!(
                    manager.report_readiness(id),
                    router::DeliveryReadiness::Drafting,
                    "{seconds}s"
                );
                assert_eq!(manager.reserve_report(id).unwrap(), None, "{seconds}s");
            },
        );
    }
    crate::session::clock::at(start + Duration::from_secs(600), 601000, || {
        assert_eq!(
            manager.delivery_readiness(id),
            router::DeliveryReadiness::Ready { idle: true },
            "an inbox notice still stops waiting at the cap"
        );
    });
    observe(InputState::Idle);
    crate::session::clock::at(start + Duration::from_secs(24 * 3600), 86_401_000, || {
        assert_eq!(
            manager.report_readiness(id),
            router::DeliveryReadiness::Ready { idle: true }
        );
        let token = manager
            .reserve_report(id)
            .unwrap()
            .expect("an idle chat takes a report");
        assert_eq!(
            manager.report_readiness(id),
            router::DeliveryReadiness::InFlight
        );
        manager.finish_delivery(id, token);
    });
}

#[test]
fn observed_draft_cap_releases_readiness_and_reservation_only_while_idle() {
    let manager =
        manager_with_runtime(crate::shell_path::LoginShellEnv::default(), inert_runtime());
    let id = "capped-draft";
    install_test_session_handle(&manager, id);
    let start = Instant::now();
    crate::session::clock::at(start, 1000, || {
        let state = manager.session_state(id).unwrap();
        let mut state = state.lock().unwrap();
        state.model.apply(
            SessionEvent::Transition {
                state: SessionActivityState::Idle,
                source: StatusSource::Forwarder,
                live: true,
            },
            crate::session::clock::state_now(),
        );
        state.model.apply(
            SessionEvent::TestInputAt(Some(start)),
            crate::session::clock::state_now(),
        );
    });
    manager.report_input_state(
        id,
        InputObservation {
            state: InputState::Drafting,
            since: start,
            composing: false,
            composer_visible: true,
        },
    );
    crate::session::clock::at(start + Duration::from_secs(599), 600000, || {
        assert_eq!(
            manager.delivery_readiness(id),
            router::DeliveryReadiness::Drafting
        );
        assert!(matches!(
            manager.reserve_delivery(id).unwrap(),
            router::DeliveryReservation::Drafting { .. }
        ));
    });
    crate::session::clock::at(start + Duration::from_secs(600), 601000, || {
        assert_eq!(
            manager.delivery_readiness(id),
            router::DeliveryReadiness::Ready { idle: true }
        );
        let router::DeliveryReservation::Ready(token) = manager.reserve_delivery(id).unwrap()
        else {
            panic!("cap must permit the next attempt");
        };
        manager.finish_delivery(id, token);
        manager
            .session_state(id)
            .unwrap()
            .lock()
            .unwrap()
            .model
            .apply(
                SessionEvent::Transition {
                    state: SessionActivityState::Busy,
                    source: StatusSource::Forwarder,
                    live: true,
                },
                crate::session::clock::state_now(),
            );
        assert_eq!(
            manager.delivery_readiness(id),
            router::DeliveryReadiness::Ready { idle: false }
        );
        assert_eq!(
            manager.reserve_delivery(id).unwrap(),
            router::DeliveryReservation::Unavailable
        );
    });
}

#[test]
fn connection_compositions_clear_on_disconnect_and_terminal_handover() {
    use runner_core::protocol::terminal::InputEvent;
    for browser in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut core = crate::test_support::test_core_in(temp.path().to_owned());
        core.sessions = mgr_with_fake(None, fake_runtime());
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        let id = "orphaned-composition";
        core.sessions
            .prepare_unlisted_terminal(id, (80, 24), &core.db, &events)
            .unwrap();
        install_test_session_handle(&core.sessions, id);
        let capture = Arc::new(DeliveryEventCapture::default());
        let listener: Arc<dyn router::SessionDeliveryListener> = capture.clone();
        core.sessions
            .register_delivery_listener(id, Arc::downgrade(&listener));
        core.sessions
            .observe_terminal_input(id, 1, InputEvent::Composing { composing: true })
            .unwrap();
        assert_eq!(
            core.sessions.delivery_readiness(id),
            router::DeliveryReadiness::Drafting
        );
        core.sessions.clear_connection_compositions(2);
        assert_eq!(
            core.sessions.delivery_readiness(id),
            router::DeliveryReadiness::Drafting
        );
        if browser {
            core.sessions
                .browser_resize(id, 1, "Browser", 80, 24, true, false, &core.db)
                .unwrap();
        } else {
            core.sessions.clear_connection_compositions(1);
        }
        assert!(capture
            .0
            .lock()
            .unwrap()
            .contains(&router::SessionDeliveryEvent::InputCleared));
        assert!(!matches!(
            core.sessions.delivery_readiness(id),
            router::DeliveryReadiness::Drafting
        ));
        core.sessions
            .take_back_terminal(id, 80, 24, &core.db)
            .unwrap();
        core.sessions
            .observe_terminal_input(id, 2, InputEvent::Composing { composing: true })
            .unwrap();
        core.sessions
            .observe_terminal_input(id, 1, InputEvent::Composing { composing: false })
            .unwrap();
        assert_eq!(
            core.sessions.delivery_readiness(id),
            router::DeliveryReadiness::Drafting,
            "a stale clear cannot cancel another client's preedit"
        );
        core.sessions
            .take_terminal_input_control(id, 3, &core.db)
            .unwrap();
        assert!(!matches!(
            core.sessions.delivery_readiness(id),
            router::DeliveryReadiness::Drafting
        ));
        core.sessions.drain_terminal_workers();
    }
}

#[test]
fn observed_drafting_to_idle_emits_input_cleared() {
    let manager =
        manager_with_runtime(crate::shell_path::LoginShellEnv::default(), inert_runtime());
    let session_id = "observed-clear";
    install_test_session_handle(&manager, session_id);
    let capture = Arc::new(DeliveryEventCapture::default());
    let listener: Arc<dyn router::SessionDeliveryListener> = capture.clone();
    manager.register_delivery_listener(session_id, Arc::downgrade(&listener));
    let observation = |state| InputObservation {
        state,
        since: Instant::now(),
        composing: false,
        composer_visible: true,
    };
    manager.report_input_state(session_id, observation(InputState::Drafting));
    manager
        .session_state(session_id)
        .unwrap()
        .lock()
        .unwrap()
        .model
        .apply(
            SessionEvent::TestInputAt(Some(Instant::now())),
            crate::session::clock::state_now(),
        );
    manager.report_input_state(session_id, observation(InputState::Idle));
    assert_eq!(
        capture.0.lock().unwrap().as_slice(),
        &[router::SessionDeliveryEvent::InputCleared]
    );
    assert!(manager
        .session_state(session_id)
        .unwrap()
        .lock()
        .unwrap()
        .model
        .last_local_input_at()
        .is_none());

    capture.0.lock().unwrap().clear();
    manager.report_input_state(session_id, observation(InputState::Drafting));
    manager.report_input_state(session_id, observation(InputState::Submitted));
    manager.report_input_state(session_id, observation(InputState::Idle));
    assert_eq!(
        capture.0.lock().unwrap().as_slice(),
        &[router::SessionDeliveryEvent::InputCleared],
        "a submitted draft must release the same parked outbox once the grid is empty"
    );
}

// `await_pty_output` was deleted in the Step 9 cutover. Tests
// that previously observed echoed bytes from /bin/cat through
// a portable-pty master now assert on FakeRuntime's captured
// pastes / keys / bytes_writes directly — faster and free of
// shell-timing flakes.
