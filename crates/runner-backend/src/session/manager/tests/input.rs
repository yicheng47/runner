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
        b"\x1b]10;rgb:dcdc/dcdc/e0e0\x1b\\",
        b"\x1b]11;rgb:1515/1616/1b1b\x1b\\",
    ] {
        assert_eq!(
            classify_local_input(protocol),
            Some(LocalInputClass::ActivityOnly),
            "terminal protocol traffic must not mark local input pending"
        );
    }
    assert_eq!(
        classify_local_input(b"\x1b[200~pasted text\x1b[201~"),
        Some(LocalInputClass::SetPending)
    );
    assert_eq!(
        classify_local_input(b"\x16"),
        Some(LocalInputClass::SetPending)
    );
    assert_eq!(
        classify_local_input(b"\r"),
        Some(LocalInputClass::ClearPending)
    );
    assert_eq!(
        classify_local_input(b"\x03"),
        Some(LocalInputClass::ClearPending)
    );

    let now = Instant::now();
    let mut state = SessionState::default();
    assert!(state.observed_input.is_none());
    update_local_input_state(&mut state, classify_local_input(b"draft"), now);
    assert!(state.local_input_pending);
    assert_eq!(state.last_local_input_at, Some(now));

    update_local_input_state(&mut state, classify_local_input(b"\r"), now);
    assert!(!state.local_input_pending);
    assert!(state.last_local_input_at.is_none());

    update_local_input_state(&mut state, classify_local_input(b"\x1b[D"), now);
    assert!(!state.local_input_pending);
    assert_eq!(state.last_local_input_at, Some(now));

    state.local_input_pending = true;
    update_local_input_state(&mut state, classify_local_input(b"\x03"), now);
    assert!(!state.local_input_pending);
    assert!(state.last_local_input_at.is_none());
}

#[test]
fn enter_while_observed_idle_is_activity_only() {
    use super::output::{classify_local_input, update_local_input_state};

    let now = Instant::now();
    let mut state = SessionState {
        observed_input: Some(ObservedInput {
            state: InputState::Idle,
            since: now,
        }),
        ..SessionState::default()
    };
    update_local_input_state(&mut state, classify_local_input(b"\r"), now);
    assert!(!state.local_input_pending);
    assert_eq!(state.last_local_input_at, Some(now));
}

#[test]
fn observed_input_tier_precedes_the_byte_latch_and_hidden_drafts_park() {
    let manager =
        manager_with_runtime(crate::shell_path::LoginShellEnv::default(), inert_runtime());
    let session_id = "observed-input";
    install_test_session_handle(&manager, session_id);
    {
        let state = manager.session_state(session_id).unwrap();
        let mut state = state.lock().unwrap();
        state.local_input_pending = true;
        state.last_local_input_at = None;
    }
    assert_eq!(
        manager.reserve_delivery(session_id).unwrap(),
        router::DeliveryReservation::PendingInput,
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
        router::DeliveryReservation::PendingInput
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
        .last_local_input_at = Some(Instant::now());
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
        .last_local_input_at = Some(Instant::now());
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
        .last_local_input_at
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
