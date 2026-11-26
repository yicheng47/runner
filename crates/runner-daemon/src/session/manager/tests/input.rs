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
fn report_draft_hold_shares_the_observed_cap_while_native_drafts_stay_uncapped() {
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
    crate::session::clock::at(start + Duration::from_secs(599), 600_000, || {
        assert_eq!(
            manager.report_readiness(id),
            router::DeliveryReadiness::Drafting,
            "held before the 10-minute fallback cap"
        );
        assert_eq!(manager.reserve_report(id).unwrap(), None);
    });
    crate::session::clock::at(start + Duration::from_secs(600), 601_000, || {
        assert_eq!(
            manager.report_readiness(id),
            router::DeliveryReadiness::Ready { idle: true },
            "fallback cap releases a stale screen-observed draft for reports"
        );
        assert_eq!(
            manager.delivery_readiness(id),
            router::DeliveryReadiness::Ready { idle: true },
            "inbox delivery shares the same cap"
        );
        let token = manager
            .reserve_report(id)
            .unwrap()
            .expect("report reserves once the fallback cap expires");
        assert_eq!(
            manager.report_readiness(id),
            router::DeliveryReadiness::InFlight
        );
        manager.finish_delivery(id, token);
    });

    // Native editor drafts (e.g. pi) still hold reports past the 10-minute cap.
    crate::session::clock::at(start + Duration::from_secs(24 * 3600), 86_401_000, || {
        let state = manager.session_state(id).unwrap();
        state.lock().unwrap().model.apply(
            SessionEvent::Agent {
                event: crate::session::state::agent::AgentEvent::EditorDraft { drafting: true },
                live: true,
            },
            crate::session::clock::state_now(),
        );
        assert_eq!(
            manager.report_readiness(id),
            router::DeliveryReadiness::Drafting
        );
        assert_eq!(manager.reserve_report(id).unwrap(), None);

        state.lock().unwrap().model.apply(
            SessionEvent::Agent {
                event: crate::session::state::agent::AgentEvent::EditorDraft { drafting: false },
                live: true,
            },
            crate::session::clock::state_now(),
        );
        assert_eq!(
            manager.report_readiness(id),
            router::DeliveryReadiness::Ready { idle: true }
        );
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
                .browser_resize(id, 1, "Browser", 80, 24, 0, 0, true, false, &core.db)
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
            .take_back_terminal(id, 80, 24, 0, 0, &core.db)
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

fn direct_chat(
    mgr: &Arc<SessionManager>,
    pool: &Arc<DbPool>,
    cap: &Arc<Capture>,
    role: &Role,
) -> String {
    mgr.spawn_direct(
        role,
        DirectLaunch {
            cwd: Some(fixture_tmp_dir().to_str().unwrap().into()),
            ..Default::default()
        },
        fixture_tmp_dir(),
        Arc::clone(pool),
        Arc::clone(cap) as Arc<dyn SessionEvents>,
    )
    .unwrap()
    .id
}

fn input_role(pool: &DbPool) -> Role {
    let role = role("/bin/cat", &[]);
    insert_role_row(&pool.get().unwrap(), &role);
    role
}

fn written(fake: &FakeRuntime, session_id: &str) -> Vec<Vec<u8>> {
    fake.bytes_writes()
        .into_iter()
        .filter(|(id, _)| id == session_id)
        .map(|(_, bytes)| bytes)
        .collect()
}

#[test]
fn every_input_path_writes_through_one_ordered_stream() {
    let pool = pool_with_schema();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture_for(&mgr);
    let role = input_role(&pool);
    let id = direct_chat(&mgr, &pool, &cap, &role);
    fake.push_status(0, SessionActivityState::Idle);
    wait_for_session_status_event(&cap, &id, SessionActivityState::Idle);
    let before = fake.inputs.lock().unwrap().len();

    let token = match mgr.reserve_delivery(&id).unwrap() {
        router::DeliveryReservation::Ready(token) => token,
        other => panic!("expected a delivery reservation, got {other:?}"),
    };
    assert!(mgr.inject_reserved(&id, token, b"[inbox]").unwrap());
    assert!(mgr.inject_reserved(&id, token, b"\r").unwrap());
    mgr.finish_delivery(&id, token);

    // Typing blocks in the PTY; a reply and a paste queue behind it from their own threads.
    let (started, release) = fake.block_next_write();
    let typing = {
        let (mgr, cap, id) = (Arc::clone(&mgr), Arc::clone(&cap), id.clone());
        std::thread::spawn(move || mgr.inject_direct_stdin(&id, b"typed", cap.as_ref()))
    };
    started.recv_timeout(Duration::from_secs(30)).unwrap();
    let reply = {
        let (mgr, id) = (Arc::clone(&mgr), id.clone());
        std::thread::spawn(move || mgr.inject_stdin(&id, b"\x1b[?1u"))
    };
    mgr.wait_for_queued_input(&id, 1);
    let paste = {
        let (mgr, id) = (Arc::clone(&mgr), id.clone());
        std::thread::spawn(move || mgr.inject_paste(&id, b"pasted"))
    };
    mgr.wait_for_queued_input(&id, 2);
    release.send(()).unwrap();
    typing.join().unwrap().unwrap();
    reply.join().unwrap().unwrap();
    paste.join().unwrap().unwrap();

    let bytes = |bytes: &[u8]| FakeInput::Bytes {
        session_id: id.clone(),
        bytes: bytes.to_vec(),
    };
    let enter = || FakeInput::Key {
        session_id: id.clone(),
        key: "Enter".into(),
    };
    assert_eq!(
        fake.inputs.lock().unwrap()[before..],
        [
            bytes(b"[inbox]"),
            enter(),
            bytes(b"typed"),
            bytes(b"\x1b[?1u"),
            bytes(b"pasted"),
            enter(),
        ]
    );
    mgr.kill(&id).unwrap();
}

#[test]
fn a_session_that_stops_reading_fails_its_typing_promptly_while_others_keep_working() {
    let pool = pool_with_schema();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture_for(&mgr);
    let role = input_role(&pool);
    let stuck = direct_chat(&mgr, &pool, &cap, &role);
    let healthy = direct_chat(&mgr, &pool, &cap, &role);
    mgr.desktop_input(&healthy, b"first").unwrap();
    eventually("the healthy session's first input is written", || {
        written(&fake, &healthy) == [b"first"]
    });

    // Only the stuck session's writer, started after this, gets the short deadline.
    mgr.set_input_deadline(Duration::from_millis(200));
    let (started, release) = fake.block_next_write();
    mgr.desktop_input(&stuck, b"blocked").unwrap();
    started.recv_timeout(Duration::from_secs(30)).unwrap();

    let _ = mgr.agent_status(&stuck);
    assert!(mgr.status_snapshot().contains_key(&stuck));
    assert!(mgr.delivery_snapshot(&stuck).is_some());
    mgr.desktop_input(&healthy, b"second").unwrap();
    eventually("the healthy session keeps taking input", || {
        written(&fake, &healthy) == [&b"first"[..], b"second"]
    });

    let stalls = || {
        cap.input_errors
            .lock()
            .unwrap()
            .iter()
            .filter(|(id, message)| id == &stuck && message.contains("stopped reading its input"))
            .count()
    };
    eventually("the blocked write fails at its deadline", || stalls() == 1);
    mgr.desktop_input(&stuck, b"refused").unwrap();
    eventually("input behind an overdue write is refused", || stalls() == 2);

    release.send(()).unwrap();
    mgr.wait_for_idle_input(&stuck);
    mgr.desktop_input(&stuck, b"recovered").unwrap();
    eventually("the session takes input once it reads again", || {
        written(&fake, &stuck) == [&b"blocked"[..], b"recovered"]
    });
    mgr.kill(&stuck).unwrap();
    mgr.kill(&healthy).unwrap();
}

#[test]
fn only_the_sessions_own_input_worker_and_terminal_write_to_it() {
    let pool = pool_with_schema();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture_for(&mgr);
    let role = input_role(&pool);
    let id = direct_chat(&mgr, &pool, &cap, &role);

    let stray = mgr
        .inject_terminal_input(&id, b"stray", cap.as_ref())
        .unwrap_err();
    assert!(
        stray.to_string().contains("terminal input closed"),
        "{stray}"
    );
    mgr.desktop_input(&id, b"typed").unwrap();
    eventually("the input worker's input is written", || {
        written(&fake, &id) == [b"typed"]
    });

    let events = Arc::clone(&cap) as Arc<dyn SessionEvents>;
    let released = mgr.create_terminal(&id, 80, 24, &pool, &events).unwrap();
    let stale = mgr.inject_reply(&id, &released, b"stale").unwrap_err();
    assert!(stale.to_string().contains("terminal released"), "{stale}");
    mgr.inject_reply(&id, &mgr.terminal_model(&id).unwrap(), b"reply")
        .unwrap();
    assert_eq!(written(&fake, &id), [&b"typed"[..], b"reply"]);
    for worker in released.stop_workers() {
        worker.join().unwrap();
    }
    mgr.kill(&id).unwrap();
}

fn enters(fake: &FakeRuntime, session_id: &str) -> usize {
    fake.keys()
        .iter()
        .filter(|(id, key)| id == session_id && key == "Enter")
        .count()
}

#[test]
fn an_overdue_write_stands_instead_of_being_undone_or_delivered_again() {
    let pool = pool_with_schema();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    mgr.set_input_deadline(Duration::from_millis(200));
    let events = crate::events::EventChannel::new();
    let mut seen = events.subscribe();
    mgr.set_runtime_events(events);
    let cap = capture_for(&mgr);
    let role = input_role(&pool);
    let delivered = direct_chat(&mgr, &pool, &cap, &role);
    let typed = direct_chat(&mgr, &pool, &cap, &role);
    for (index, id) in [&delivered, &typed].into_iter().enumerate() {
        fake.push_status(index, SessionActivityState::Idle);
        wait_for_session_status_event(&cap, id, SessionActivityState::Idle);
    }

    // A delivery body whose write outlives its deadline reports as typed, so the router does
    // not finish the delivery as failed and type the notice again.
    let token = match mgr.reserve_delivery(&delivered).unwrap() {
        router::DeliveryReservation::Ready(token) => token,
        other => panic!("expected a delivery reservation, got {other:?}"),
    };
    let (started, release) = fake.block_next_write();
    assert!(mgr.inject_reserved(&delivered, token, b"[inbox]").unwrap());
    started.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(reported_overdue(&mut seen, &delivered));
    mgr.finish_delivery(&delivered, token);
    release.send(()).unwrap();
    mgr.wait_for_idle_input(&delivered);
    assert_eq!(written(&fake, &delivered), [b"[inbox]"]);

    // A submitted Enter whose write outlives its deadline keeps the turn it started, says it is
    // still waiting, and lands once when the agent reads again.
    let (started, release) = fake.block_next_write();
    mgr.desktop_input(&typed, b"\r").unwrap();
    started.recv_timeout(Duration::from_secs(30)).unwrap();
    eventually("the overdue Enter is reported as still waiting", || {
        cap.input_errors
            .lock()
            .unwrap()
            .iter()
            .any(|(id, message)| {
                id == &typed && message.contains("arrives if the session reads again")
            })
    });
    assert_eq!(
        mgr.activity_snapshot().get(&typed),
        Some(&SessionActivityState::Busy)
    );
    assert!(mgr.take_completion_armed(std::slice::from_ref(&typed)));
    release.send(()).unwrap();
    mgr.wait_for_idle_input(&typed);
    assert_eq!(enters(&fake, &typed), 1);
    mgr.kill(&delivered).unwrap();
    mgr.kill(&typed).unwrap();
}

#[test]
fn a_resubmit_enter_is_not_queued_behind_earlier_input() {
    let pool = pool_with_schema();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture_for(&mgr);
    let role = input_role(&pool);
    let id = direct_chat(&mgr, &pool, &cap, &role);
    let before = mgr.delivery_snapshot(&id).unwrap().rows;
    let body = "[inbox] 1 new messages — run `runner msg read` to view.";
    fake.push_output(0, format!("> {body}").as_bytes());
    eventually("the notice shows in the composer", || {
        mgr.delivery_snapshot(&id)
            .unwrap()
            .rows
            .iter()
            .any(|row| row.contains(body))
    });
    let snapshot = mgr.delivery_snapshot(&id).unwrap();
    let mut probe = runner_terminal::delivery_probe::InputProbe::new(before, body);
    probe.locate(&snapshot.rows);
    assert_eq!(
        probe.state(&snapshot.rows),
        runner_terminal::delivery_probe::ProbeState::Present
    );

    // Behind a write the agent is not draining, the checked composer could change before the
    // Enter lands, so the resubmit is left to a later delivery turn.
    let (started, release) = fake.block_next_write();
    let reply = {
        let (mgr, id) = (Arc::clone(&mgr), id.clone());
        std::thread::spawn(move || mgr.inject_stdin(&id, b"\x1b[?1u"))
    };
    started.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(!mgr.resubmit(&id, &snapshot, &probe).unwrap());
    release.send(()).unwrap();
    reply.join().unwrap().unwrap();
    mgr.wait_for_idle_input(&id);
    assert_eq!(enters(&fake, &id), 0);

    assert!(mgr.resubmit(&id, &snapshot, &probe).unwrap());
    assert_eq!(enters(&fake, &id), 1);
    mgr.kill(&id).unwrap();
}

#[test]
fn typing_beyond_the_input_limit_is_refused_at_once_from_desktop_and_browser() {
    let pool = pool_with_schema();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture_for(&mgr);
    let role = input_role(&pool);
    let id = direct_chat(&mgr, &pool, &cap, &role);
    let large = vec![b'x'; super::input_writer::INPUT_QUEUE_LIMIT];

    let (started, release) = fake.block_next_write();
    mgr.desktop_input(&id, b"blocked").unwrap();
    started.recv_timeout(Duration::from_secs(30)).unwrap();
    let desktop = mgr.desktop_input(&id, &large).unwrap_err();
    assert!(desktop.to_string().contains("is full"), "{desktop}");
    let pool_arc = Arc::clone(&pool);
    mgr.browser_resize(&id, 7, "Phone", 80, 24, 0, 0, true, false, &pool_arc)
        .unwrap();
    let browser = mgr.browser_input(&id, 7, &large).unwrap_err();
    assert!(browser.to_string().contains("is full"), "{browser}");

    release.send(()).unwrap();
    mgr.wait_for_idle_input(&id);
    mgr.browser_input(&id, 7, b"after").unwrap();
    eventually("input is taken again once the write returns", || {
        written(&fake, &id) == [&b"blocked"[..], b"after"]
    });
    mgr.kill(&id).unwrap();
}

#[test]
fn a_dropped_input_leaves_a_stopped_session_as_the_stop_left_it() {
    let pool = pool_with_schema();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let cap = capture_for(&mgr);
    let role = input_role(&pool);
    let id = direct_chat(&mgr, &pool, &cap, &role);
    // A stopped chat's state leaves the map, so read the one the input worker rolls back into.
    let state = mgr.session_state(&id).unwrap();
    let pending = || state.lock().unwrap().model.local_input_pending();
    mgr.desktop_input(&id, b"draft").unwrap();
    eventually("the draft is written", || written(&fake, &id) == [b"draft"]);
    assert!(pending());

    // The Enter that clears the draft waits behind a reply the agent is not draining.
    let (started, release) = fake.block_next_write();
    let reply = {
        let (mgr, id) = (Arc::clone(&mgr), id.clone());
        std::thread::spawn(move || mgr.inject_stdin(&id, b"\x1b[?1u"))
    };
    started.recv_timeout(Duration::from_secs(30)).unwrap();
    mgr.desktop_input(&id, b"\r").unwrap();
    mgr.wait_for_queued_input(&id, 1);
    mgr.kill(&id).unwrap();
    eventually("the dropped Enter reports", || {
        cap.input_errors
            .lock()
            .unwrap()
            .iter()
            .any(|(session, message)| {
                session == &id && message.contains("stopped before its input was written")
            })
    });
    assert!(
        !pending(),
        "the stop cleared the draft and the dropped Enter leaves that"
    );
    release.send(()).unwrap();
    assert!(reply.join().unwrap().is_err());
    assert_eq!(enters(&fake, &id), 0);
}
