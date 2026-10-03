use super::*;

static DELIVERY_LOG: DeliveryLog = DeliveryLog(Mutex::new(Vec::new()));

struct DeliveryLog(Mutex<Vec<String>>);

impl log::Log for DeliveryLog {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        let text = record.args().to_string();
        if text.starts_with("router delivery ") || text.starts_with("router stale inbox ") {
            self.0.lock().unwrap().push(text);
        }
    }
    fn flush(&self) {}
}

fn delivery_logs(session_id: &str) -> Vec<String> {
    DELIVERY_LOG
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|line| line.contains(&format!("session={session_id} ")))
        .cloned()
        .collect()
}

#[test]
fn hold_logs_dedupe_retries_and_name_reasons_and_release_trigger() {
    log::set_logger(&DELIVERY_LOG).unwrap();
    log::set_max_level(log::LevelFilter::Info);
    let (router, injector, _log, _dir) =
        fixture(vec![slot_with_role("lead", true)], &[("lead", "S-LOG-766")]);
    injector.set_observed_input("S-LOG-766", InputState::Drafting, true);
    router.inject_and_submit("lead", b"held").unwrap();
    router.flush_outbox("S-LOG-766", "InputQueueDrained");
    assert_eq!(delivery_logs("S-LOG-766").len(), 1);
    assert!(delivery_logs("S-LOG-766")[0].contains("Drafting { composer_visible: true }"));
    injector.set_observed_input("S-LOG-766", InputState::Drafting, false);
    router.flush_outbox("S-LOG-766", "InputQueueDrained");
    assert_eq!(delivery_logs("S-LOG-766").len(), 2);
    assert!(delivery_logs("S-LOG-766")[1].contains("composer_visible: false"));
    injector.clear_pending("S-LOG-766");
    wait_until(Duration::from_secs(2), || {
        !injector.submitted_bodies_for("S-LOG-766").is_empty()
    });
    assert_eq!(delivery_logs("S-LOG-766").len(), 3);
    assert!(delivery_logs("S-LOG-766")[2].contains("trigger=InputCleared"));
    injector.exit("S-LOG-766");

    let mut outbox = super::super::SessionOutbox {
        handle: "lead".into(),
        ..Default::default()
    };
    for reason in [
        DeliveryReservation::LocalInputPending,
        DeliveryReservation::HumanInteraction,
        DeliveryReservation::InFlight,
        DeliveryReservation::RecentlyTyping(Duration::from_millis(20)),
        DeliveryReservation::RecentlyTyping(Duration::from_millis(10)),
    ] {
        outbox.hold("S-LOG-REASONS-766", reason);
    }
    outbox.release("S-LOG-REASONS-766", "RecentlyTypingElapsed");
    outbox.release("S-LOG-REASONS-766", "RecentlyTypingElapsed");
    let logs = delivery_logs("S-LOG-REASONS-766");
    assert_eq!(logs.len(), 5);
    for (line, reason) in logs.iter().zip([
        "LocalInputPending",
        "HumanInteraction",
        "InFlight",
        "RecentlyTyping",
        "trigger=RecentlyTypingElapsed",
    ]) {
        assert!(line.contains(reason), "{line}");
    }
}

#[test]
fn delivery_blocked_transition_dedupes_repeated_parks_and_reemits_count_changes() {
    let (router, injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    set_unread(&router, "impl", 1);
    injector.set_pending("S-IMPL");

    router
        .inject_inbox_nudge("impl", b"[inbox] first", None)
        .unwrap();
    router
        .inject_inbox_nudge("impl", b"[inbox] second", None)
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
        .inject_inbox_nudge("impl", b"[inbox] recent", None)
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
        .inject_inbox_nudge("impl", b"[inbox] in flight", None)
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
        .inject_inbox_nudge("impl", b"[inbox] waiting", None)
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
        .inject_inbox_nudge("impl", b"[inbox] waiting", None)
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
        .inject_inbox_nudge("impl", b"[inbox] waiting", None)
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
        .inject_inbox_nudge("impl", b"[inbox] waiting", None)
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
                .inject_inbox_nudge("impl", b"[inbox] concurrent", None)
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
