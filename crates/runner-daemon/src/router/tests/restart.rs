use super::*;

#[test]
fn registry_register_get_unregister() {
    let (router, _i, _l, _d) = fixture(vec![slot_with_role("lead", true)], &[("lead", "S-LEAD")]);
    let reg = RouterRegistry::new();
    reg.register("mission-1".into(), router.clone());
    assert!(reg.get("mission-1").is_some());
    reg.unregister("mission-1");
    assert!(reg.get("mission-1").is_none());
}

#[test]
fn slot_restart_events_are_ordered_and_nudge_the_lead() {
    let (router, injector, log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    router
        .record_slot_restart("impl", "S-IMPL", Some("prior-key"))
        .unwrap();
    let (entries, _) = log.read_from_lossy(0).unwrap();
    assert_eq!(entries.len(), 2);
    let signal = &entries[0].event;
    assert_eq!(signal.kind, EventKind::Signal);
    assert_eq!(signal.from, "human");
    assert_eq!(
        signal.signal_type.as_ref().unwrap().as_str(),
        "slot_restarted"
    );
    assert_eq!(
        signal.payload,
        serde_json::json!({"handle": "impl", "session_id": "S-IMPL", "prior_agent_session_key": "prior-key"})
    );
    let note = &entries[1].event;
    assert_eq!(note.kind, EventKind::Message);
    assert_eq!(note.from, "runner");
    assert_eq!(note.to.as_deref(), Some("lead"));
    assert_eq!(note.payload["text"], "@impl was restarted by the human and starts over with only its brief. Nothing it was told in this mission survives; re-send the task, branch, and anything else it needs.");
    for entry in entries {
        router.handle_event(&entry.event);
    }
    assert!(injector.pushes_for("S-LEAD")[0].contains("runner msg read"));
    assert!(injector.pushes_for("S-IMPL").is_empty());
}

#[test]
fn lead_restart_notifies_each_other_slot() {
    let (router, injector, log, _dir) = fixture(
        vec![
            slot_with_role("lead", true),
            slot_with_role("impl", false),
            slot_with_role("review", false),
        ],
        &[
            ("lead", "S-LEAD"),
            ("impl", "S-IMPL"),
            ("review", "S-REVIEW"),
        ],
    );
    router.record_slot_restart("lead", "S-LEAD", None).unwrap();
    let (entries, _) = log.read_from_lossy(0).unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[1].event.to.as_deref(), Some("impl"));
    assert_eq!(entries[2].event.to.as_deref(), Some("review"));
    for entry in entries {
        router.handle_event(&entry.event);
    }
    assert!(injector.pushes_for("S-LEAD").is_empty());
    for id in ["S-IMPL", "S-REVIEW"] {
        assert!(injector.pushes_for(id)[0].contains("runner msg read"));
    }
}
