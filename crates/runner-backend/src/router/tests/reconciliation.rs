use super::*;

#[test]
fn reconciliation_tick_does_not_churn_blocked_notifications() {
    let (router, injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    set_unread(&router, "impl", 1);
    injector.set_pending("S-IMPL");
    router
        .inject_inbox_nudge("impl", b"[inbox] waiting")
        .unwrap();
    router.set_status("impl".into(), super::SessionActivityState::Idle);

    assert_eq!(
        router.reconcile_inbox_at(Instant::now(), Duration::from_secs(120)),
        0
    );
    assert_eq!(injector.blocked_events().len(), 1);
    injector.exit("S-IMPL");
}

#[test]
fn reconciliation_tick_is_silent_for_empty_inbox() {
    let (router, injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    router.set_status("impl".into(), super::SessionActivityState::Idle);

    assert_eq!(
        router.reconcile_inbox_at(Instant::now(), Duration::from_secs(120)),
        0
    );
    assert!(injector.pushes_for("S-IMPL").is_empty());
}

#[test]
fn reconciliation_tick_is_silent_for_busy_session() {
    let (router, injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    router.update_inbox(&crate::event_bus::InboxUpdate {
        mission_id: "mission-1".into(),
        role_handle: "impl".into(),
        last_id: None,
        watermark: None,
        unread_count: 1,
    });
    router.set_status("impl".into(), super::SessionActivityState::Busy);

    assert_eq!(
        router.reconcile_inbox_at(Instant::now(), Duration::from_secs(120)),
        0
    );
    assert!(injector.pushes_for("S-IMPL").is_empty());
}

#[test]
fn reconciliation_tick_renudges_idle_session_with_unread_mail() {
    let (router, injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    router.update_inbox(&crate::event_bus::InboxUpdate {
        mission_id: "mission-1".into(),
        role_handle: "impl".into(),
        last_id: None,
        watermark: None,
        unread_count: 1,
    });
    router.set_status("impl".into(), super::SessionActivityState::Idle);

    assert_eq!(
        router.reconcile_inbox_at(Instant::now(), Duration::from_secs(120)),
        1
    );
    assert_eq!(
        injector.submitted_bodies_for("S-IMPL"),
        ["[inbox] unread messages — run `runner msg read` to view."]
    );
    assert!(matches!(
        router.state.lock().unwrap().status.get("impl"),
        Some(super::SessionActivityState::Busy)
    ));
}

#[test]
fn reconciliation_tick_does_not_park_when_input_is_pending() {
    let (router, injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    injector.set_pending("S-IMPL");
    router.update_inbox(&crate::event_bus::InboxUpdate {
        mission_id: "mission-1".into(),
        role_handle: "impl".into(),
        last_id: None,
        watermark: None,
        unread_count: 1,
    });
    router.set_status("impl".into(), super::SessionActivityState::Idle);

    assert_eq!(
        router.reconcile_inbox_at(Instant::now(), Duration::from_secs(120)),
        0
    );
    {
        let state = router.state.lock().unwrap();
        assert!(!state.outbox_by_session.contains_key("S-IMPL"));
        assert!(!state.last_reconciliation_nudge.contains_key("impl"));
    }

    router.update_inbox(&crate::event_bus::InboxUpdate {
        mission_id: "mission-1".into(),
        role_handle: "impl".into(),
        last_id: None,
        watermark: Some("watermark".into()),
        unread_count: 0,
    });
    router.set_status("impl".into(), super::SessionActivityState::Busy);
    injector.clear_pending("S-IMPL");
    std::thread::sleep(Duration::from_millis(550));
    assert!(
        injector.pushes_for("S-IMPL").is_empty(),
        "a deferred clock nudge must not escape the busy and unread gates"
    );
}

#[test]
fn reconciliation_tick_does_not_duplicate_a_parked_nudge() {
    let (router, injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    injector.set_pending("S-IMPL");
    router
        .inject_inbox_nudge("impl", b"[inbox] original nudge")
        .unwrap();
    router.update_inbox(&crate::event_bus::InboxUpdate {
        mission_id: "mission-1".into(),
        role_handle: "impl".into(),
        last_id: None,
        watermark: None,
        unread_count: 1,
    });
    router.set_status("impl".into(), super::SessionActivityState::Idle);

    assert_eq!(
        router.reconcile_inbox_at(Instant::now(), Duration::from_secs(120)),
        0
    );
    let state = router.state.lock().unwrap();
    let outbox = state.outbox_by_session.get("S-IMPL").unwrap();
    assert_eq!(outbox.deliveries.len(), 1);
    assert_eq!(outbox.deliveries.front().unwrap().count, 1);
    assert!(!state.last_reconciliation_nudge.contains_key("impl"));
    drop(state);
    injector.exit("S-IMPL");
}

#[test]
fn reconciliation_reservation_error_does_not_start_backoff() {
    let (router, injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    router.update_inbox(&crate::event_bus::InboxUpdate {
        mission_id: "mission-1".into(),
        role_handle: "impl".into(),
        last_id: None,
        watermark: None,
        unread_count: 1,
    });
    router.set_status("impl".into(), super::SessionActivityState::Idle);
    injector.mark_dead("S-IMPL");

    assert_eq!(
        router.reconcile_inbox_at(Instant::now(), Duration::from_secs(120)),
        0
    );
    assert!(!router
        .state
        .lock()
        .unwrap()
        .last_reconciliation_nudge
        .contains_key("impl"));
}

#[test]
fn reconciliation_tick_honors_per_handle_backoff() {
    let (router, _injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    router.update_inbox(&crate::event_bus::InboxUpdate {
        mission_id: "mission-1".into(),
        role_handle: "impl".into(),
        last_id: None,
        watermark: None,
        unread_count: 1,
    });
    router.set_status("impl".into(), super::SessionActivityState::Idle);
    let now = Instant::now();
    let backoff = Duration::from_secs(120);

    assert_eq!(router.reconcile_inbox_at(now, backoff), 1);
    wait_until(Duration::from_millis(300), || {
        !router
            .state
            .lock()
            .unwrap()
            .outbox_by_session
            .contains_key("S-IMPL")
    });
    router.set_status("impl".into(), super::SessionActivityState::Idle);
    assert_eq!(
        router.reconcile_inbox_at(now + Duration::from_secs(119), backoff),
        0
    );
    assert_eq!(
        router.reconcile_inbox_at(now + Duration::from_secs(120), backoff),
        1
    );
}

#[test]
fn reconciliation_tick_quiesces_after_watermark_advance() {
    let (router, injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    let now = Instant::now();
    router.update_inbox(&crate::event_bus::InboxUpdate {
        mission_id: "mission-1".into(),
        role_handle: "impl".into(),
        last_id: None,
        watermark: None,
        unread_count: 1,
    });
    router.set_status("impl".into(), super::SessionActivityState::Idle);
    assert_eq!(router.reconcile_inbox_at(now, Duration::from_secs(120)), 1);
    wait_until(Duration::from_millis(300), || {
        !router
            .state
            .lock()
            .unwrap()
            .outbox_by_session
            .contains_key("S-IMPL")
    });
    injector.clear_pushes();
    router.update_inbox(&crate::event_bus::InboxUpdate {
        mission_id: "mission-1".into(),
        role_handle: "impl".into(),
        last_id: None,
        watermark: Some("watermark".into()),
        unread_count: 0,
    });
    router.set_status("impl".into(), super::SessionActivityState::Idle);

    assert_eq!(
        router.reconcile_inbox_at(now + Duration::from_secs(120), Duration::from_secs(120)),
        0
    );
    assert!(injector.pushes_for("S-IMPL").is_empty());
}

#[test]
fn reconciliation_clock_stops_with_mission_and_skips_stopped_sessions() {
    let (router, injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    router.update_inbox(&crate::event_bus::InboxUpdate {
        mission_id: "mission-1".into(),
        role_handle: "impl".into(),
        last_id: None,
        watermark: None,
        unread_count: 1,
    });
    router.set_status("impl".into(), super::SessionActivityState::Idle);
    let registry = RouterRegistry::new();
    registry.register_with_timings(
        "mission-1".into(),
        Arc::clone(&router),
        Duration::from_millis(10),
        Duration::ZERO,
    );
    wait_until(Duration::from_millis(100), || {
        injector.submitted_bodies_for("S-IMPL").len() == 1
    });
    wait_until(Duration::from_millis(300), || {
        !router
            .state
            .lock()
            .unwrap()
            .outbox_by_session
            .contains_key("S-IMPL")
    });

    injector.clear_pushes();
    router.set_status("impl".into(), super::SessionActivityState::Idle);
    injector.exit("S-IMPL");
    std::thread::sleep(Duration::from_millis(40));
    assert!(
        injector.pushes_for("S-IMPL").is_empty(),
        "clock must not nudge an exited session"
    );

    injector.respawn("S-IMPL");
    router.set_status("impl".into(), super::SessionActivityState::Idle);
    wait_until(Duration::from_millis(100), || {
        injector.submitted_bodies_for("S-IMPL").len() == 1
    });
    wait_until(Duration::from_millis(300), || {
        !router
            .state
            .lock()
            .unwrap()
            .outbox_by_session
            .contains_key("S-IMPL")
    });
    registry.unregister("mission-1");
    injector.clear_pushes();
    router.set_status("impl".into(), super::SessionActivityState::Idle);
    std::thread::sleep(Duration::from_millis(40));
    assert!(
        injector.pushes_for("S-IMPL").is_empty(),
        "clock must halt when the mission router unmounts"
    );
}
