use super::*;

#[test]
fn delivery_blocked_transition_dedupes_repeated_parks_and_reemits_count_changes() {
    let (router, injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    set_unread(&router, "impl", 1);
    injector.set_pending("S-IMPL");

    router.inject_inbox_nudge("impl", b"[inbox] first").unwrap();
    router
        .inject_inbox_nudge("impl", b"[inbox] second")
        .unwrap();
    assert_eq!(
        injector.blocked_events(),
        [DeliveryBlockedEvent {
            mission_id: "mission-1".into(),
            session_id: "S-IMPL".into(),
            handle: "impl".into(),
            unread_count: 1,
            blocked: true,
        }]
    );

    set_unread(&router, "impl", 2);
    set_unread(&router, "impl", 2);
    assert_eq!(
        injector.blocked_events().last(),
        Some(&DeliveryBlockedEvent {
            mission_id: "mission-1".into(),
            session_id: "S-IMPL".into(),
            handle: "impl".into(),
            unread_count: 2,
            blocked: true,
        })
    );
    assert_eq!(injector.blocked_events().len(), 2);
    injector.exit("S-IMPL");
}

#[test]
fn transient_delivery_reservations_do_not_emit_blocked() {
    let (router, injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    set_unread(&router, "impl", 1);
    injector.set_recent_typing("S-IMPL");
    router
        .inject_inbox_nudge("impl", b"[inbox] recent")
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(injector.blocked_events().is_empty());
    injector.exit("S-IMPL");

    let (router, injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    set_unread(&router, "impl", 1);
    injector.set_in_flight("S-IMPL");
    router
        .inject_inbox_nudge("impl", b"[inbox] in flight")
        .unwrap();
    assert!(injector.blocked_events().is_empty());
    injector.exit("S-IMPL");
}

#[test]
fn delivery_blocked_clears_when_parked_delivery_flushes() {
    let (router, injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    set_unread(&router, "impl", 1);
    injector.set_pending("S-IMPL");
    router
        .inject_inbox_nudge("impl", b"[inbox] waiting")
        .unwrap();

    injector.clear_pending("S-IMPL");
    wait_until(Duration::from_secs(1), || {
        injector
            .blocked_events()
            .last()
            .is_some_and(|event| !event.blocked)
    });
    assert_eq!(injector.blocked_events().len(), 2);
}

#[test]
fn delivery_blocked_clears_when_watermark_reaches_zero() {
    let (router, injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    set_unread(&router, "impl", 1);
    injector.set_pending("S-IMPL");
    router
        .inject_inbox_nudge("impl", b"[inbox] waiting")
        .unwrap();

    set_unread(&router, "impl", 0);
    assert_eq!(injector.blocked_events().len(), 2);
    assert_eq!(
        injector.blocked_events().last(),
        Some(&DeliveryBlockedEvent {
            mission_id: "mission-1".into(),
            session_id: "S-IMPL".into(),
            handle: "impl".into(),
            unread_count: 0,
            blocked: false,
        })
    );
    injector.exit("S-IMPL");
}

#[test]
fn delivery_blocked_clears_on_session_exit_and_router_unmount() {
    let (router, injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    set_unread(&router, "impl", 1);
    injector.set_pending("S-IMPL");
    router
        .inject_inbox_nudge("impl", b"[inbox] waiting")
        .unwrap();
    injector.exit("S-IMPL");
    assert_eq!(injector.blocked_events().len(), 2);
    assert!(!injector.blocked_events().last().unwrap().blocked);

    let (router, injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    set_unread(&router, "impl", 1);
    injector.set_pending("S-IMPL");
    router
        .inject_inbox_nudge("impl", b"[inbox] waiting")
        .unwrap();
    let registry = RouterRegistry::new();
    registry.register("mission-1".into(), router);
    registry.unregister("mission-1");
    assert_eq!(injector.blocked_events().len(), 2);
    assert!(!injector.blocked_events().last().unwrap().blocked);
}

#[test]
fn concurrent_parks_emit_one_delivery_blocked_transition() {
    use std::sync::Barrier;

    let (router, injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    set_unread(&router, "impl", 1);
    injector.set_pending("S-IMPL");
    let barrier = Arc::new(Barrier::new(5));
    let mut threads = Vec::new();
    for _ in 0..4 {
        let router = Arc::clone(&router);
        let barrier = Arc::clone(&barrier);
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            router
                .inject_inbox_nudge("impl", b"[inbox] concurrent")
                .unwrap();
        }));
    }
    barrier.wait();
    for thread in threads {
        thread.join().unwrap();
    }

    assert_eq!(injector.blocked_events().len(), 1);
    injector.exit("S-IMPL");
}
