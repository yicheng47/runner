use super::*;

#[test]
fn session_status_updates_state_map_without_injecting_to_lead() {
    // Contract: session_status is observability, not coordination. It must
    // never inject into any session — including the lead's — under any
    // state transition. The in-memory status map is the only side effect.
    let (router, injector, log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );

    // busy from a worker — silent (no push to lead).
    let busy = log
        .append(signal(
            "impl",
            "session_status",
            serde_json::json!({ "state": "busy" }),
        ))
        .unwrap();
    router.handle_event(&busy);
    assert!(injector.pushes_for("S-LEAD").is_empty());

    // idle from a worker — also silent. (Was the regression in #125: the
    // idle notice injected into the lead's TUI as a fake user message.)
    let idle = log
        .append(signal(
            "impl",
            "session_status",
            serde_json::json!({ "state": "idle" }),
        ))
        .unwrap();
    router.handle_event(&idle);
    assert!(
        injector.pushes_for("S-LEAD").is_empty(),
        "session_status idle must not inject into the lead",
    );

    // Observability path still works: the state map reflects the latest
    // status from the worker.
    assert!(matches!(
        router.state.lock().unwrap().status.get("impl"),
        Some(super::SessionActivityState::Idle),
    ));

    // idle from the lead itself — defense-in-depth, still no push.
    let lead_idle = log
        .append(signal(
            "lead",
            "session_status",
            serde_json::json!({ "state": "idle" }),
        ))
        .unwrap();
    router.handle_event(&lead_idle);
    assert!(
        injector.pushes_for("S-LEAD").is_empty(),
        "lead going idle must not push to lead",
    );
}

#[test]
fn session_status_latest_wins_across_forwarder_and_agent_sources() {
    // The router doesn't branch on `payload.source`. Forwarder-emitted
    // (`source: "forwarder"`) and historical or hook-emitted
    // (`source: "agent"`) events both feed the same per-handle map under
    // a strict latest-wins policy.
    let (router, _injector, log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );

    // Interleave: forwarder busy → agent idle → forwarder idle →
    // agent busy → forwarder idle. Each event lands one after the
    // other so the latest wins.
    let series = [
        ("forwarder", "busy"),
        ("agent", "idle"),
        ("forwarder", "idle"),
        ("agent", "busy"),
        ("forwarder", "idle"),
    ];
    for (source, state) in series {
        let ev = log
            .append(signal(
                "impl",
                "session_status",
                serde_json::json!({ "state": state, "source": source }),
            ))
            .unwrap();
        router.handle_event(&ev);
    }

    // Last event was a forwarder Idle — that's what the state map
    // should reflect.
    assert!(matches!(
        router.state.lock().unwrap().status.get("impl"),
        Some(super::SessionActivityState::Idle),
    ));

    // One more flip — agent says busy, that wins.
    let agent_busy = log
        .append(signal(
            "impl",
            "session_status",
            serde_json::json!({ "state": "busy", "source": "agent" }),
        ))
        .unwrap();
    router.handle_event(&agent_busy);
    assert!(matches!(
        router.state.lock().unwrap().status.get("impl"),
        Some(super::SessionActivityState::Busy),
    ));
}

#[test]
fn directed_wake_synthesizes_busy_and_idle_clears_it() {
    // Issue #32: the rail badge stayed `idle` because nothing flipped
    // a worker to `busy` on dispatch — only the worker's own end-of-task
    // `idle` was emitted. The router now synthesizes `session_status busy`
    // (with `from = recipient`) for any wake nudge, and the existing
    // worker-emitted `idle` clears it.
    let (router, injector, log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );

    let busy_for_impl = |log: &EventLog| -> usize {
        read_signals(log)
            .into_iter()
            .filter(|s| {
                s.signal_type
                    .as_ref()
                    .map(|t| t.as_str() == "session_status")
                    .unwrap_or(false)
                    && s.from == "impl"
                    && s.payload.get("state").and_then(|v| v.as_str()) == Some("busy")
            })
            .count()
    };

    // (a) directed `runner msg post --to impl` → recipient flips to busy
    // and a synthetic session_status busy event lands in the log.
    let direct = log.append(message("lead", Some("impl"), "go")).unwrap();
    router.handle_event(&direct);
    assert_eq!(
        busy_for_impl(&log),
        1,
        "wake nudge must append one busy event"
    );
    assert!(matches!(
        router.state.lock().unwrap().status.get("impl"),
        Some(super::SessionActivityState::Busy),
    ));
    assert_eq!(
        injector.activity_for("S-IMPL"),
        Some(super::SessionActivityState::Busy),
        "synthetic busy must update the session-side activity store",
    );

    // A second directed wake while still busy must not churn another
    // busy event into the log — the dedupe guard suppresses it.
    let direct_again = log
        .append(message("lead", Some("impl"), "still going"))
        .unwrap();
    router.handle_event(&direct_again);
    assert_eq!(
        busy_for_impl(&log),
        1,
        "back-to-back wake while busy must not append a second busy event",
    );

    // (b) worker emits session_status idle → state flips back to Idle.
    let idle = log
        .append(signal(
            "impl",
            "session_status",
            serde_json::json!({ "state": "idle" }),
        ))
        .unwrap();
    router.handle_event(&idle);
    assert!(matches!(
        router.state.lock().unwrap().status.get("impl"),
        Some(super::SessionActivityState::Idle),
    ));

    // (c) follow-up directed message → flips back to busy. This is the
    // exact regression issue #32 calls out.
    let direct_followup = log.append(message("lead", Some("impl"), "next")).unwrap();
    router.handle_event(&direct_followup);
    wait_until(Duration::from_secs(1), || busy_for_impl(&log) == 2);
    assert_eq!(
        busy_for_impl(&log),
        2,
        "wake after idle must re-synthesize busy",
    );
    assert!(matches!(
        router.state.lock().unwrap().status.get("impl"),
        Some(super::SessionActivityState::Busy),
    ));
}

#[test]
fn synthetic_busy_replays_through_existing_session_status_projection() {
    // The reconstruct_from_log path at router/mod.rs handles
    // session_status events generically — synthetic ones written by
    // inject_and_submit must replay correctly without any special
    // handling. This pins that contract: a busy event from a prior
    // session is recovered into router state on reopen.
    let dir = tempfile::tempdir().unwrap();
    let log = Arc::new(EventLog::open(dir.path()).unwrap());
    let roster = vec![slot_with_role("lead", true), slot_with_role("impl", false)];

    // First mount: drive a directed message to synthesize busy.
    {
        let injector = Arc::new(RecordingInjector::new(Arc::clone(&log)));
        let injector_dyn: Arc<dyn StdinInjector> = injector.clone();
        let router = Router::new(
            "mission-1".into(),
            "crew-1".into(),
            "Crew One".into(),
            &roster,
            vec![],
            None,
            log.clone(),
            injector_dyn,
            injector.clone(),
        )
        .unwrap();
        router.register_sessions(&[
            ("lead".into(), "S-LEAD".into()),
            ("impl".into(), "S-IMPL".into()),
        ]);
        let direct = log.append(message("lead", Some("impl"), "go")).unwrap();
        router.handle_event(&direct);
    }

    // Reopen + reconstruct.
    let injector = Arc::new(RecordingInjector::new(Arc::clone(&log)));
    let injector_dyn: Arc<dyn StdinInjector> = injector.clone();
    let router2 = Router::new(
        "mission-1".into(),
        "crew-1".into(),
        "Crew One".into(),
        &roster,
        vec![],
        None,
        log.clone(),
        injector_dyn,
        injector.clone(),
    )
    .unwrap();
    router2.register_sessions(&[
        ("lead".into(), "S-LEAD".into()),
        ("impl".into(), "S-IMPL".into()),
    ]);
    router2.reconstruct_from_log().unwrap();

    assert!(matches!(
        router2.state.lock().unwrap().status.get("impl"),
        Some(super::SessionActivityState::Busy),
    ));
}
