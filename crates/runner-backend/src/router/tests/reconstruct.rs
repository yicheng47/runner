use super::*;

#[test]
fn pending_ask_map_reconstructs_from_log_on_reopen() {
    // Mount router #1, dispatch ask_human (which appends human_question),
    // drop. Mount router #2, call reconstruct_from_log (the reopen entry
    // point), then route human_response. The answer must still reach the
    // original asker — no separate persistence layer.
    let dir = tempfile::tempdir().unwrap();
    let log = Arc::new(EventLog::open(dir.path()).unwrap());
    let roster = vec![slot_with_role("lead", true), slot_with_role("impl", false)];

    let ask = log
        .append(signal(
            "lead",
            "ask_human",
            serde_json::json!({
                "prompt": "Approve?",
                "choices": ["yes", "no"],
                "on_behalf_of": "impl",
            }),
        ))
        .unwrap();

    // First mount handles the ask live (appends human_question).
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
        router.handle_event(&ask);
    }
    // Capture the card id router #1 produced; we'll use it as
    // human_response.payload.question_id below.
    let card_id = read_signals(&log)
        .into_iter()
        .find(|s| {
            s.signal_type
                .as_ref()
                .map(|t| t.as_str() == "human_question")
                .unwrap_or(false)
        })
        .expect("router #1 must have appended human_question")
        .id;
    log.append(message("lead", Some("impl"), "historical mail"))
        .unwrap();

    // Reopen: build router #2, fold projection state from history. This
    // is the path mission_resume / mount-on-app-restart will follow.
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

    // Replay the historical events through handle_event the way the bus's
    // initial replay would. The watermark must short-circuit them so the
    // ask_human is NOT re-handled (no second human_question card in the
    // log) and the lead is NOT re-injected with anything.
    let card_count_before = read_signals(&log)
        .iter()
        .filter(|s| {
            s.signal_type
                .as_ref()
                .map(|t| t.as_str() == "human_question")
                .unwrap_or(false)
        })
        .count();
    for entry in log.read_from(0).unwrap() {
        router2.handle_event(&entry.event);
    }
    let card_count_after = read_signals(&log)
        .iter()
        .filter(|s| {
            s.signal_type
                .as_ref()
                .map(|t| t.as_str() == "human_question")
                .unwrap_or(false)
        })
        .count();
    assert_eq!(
        card_count_before, card_count_after,
        "replay must NOT re-emit human_question cards",
    );
    assert!(
        injector.all_pushes().is_empty(),
        "replay must NOT re-inject historical stdin; got {:?}",
        injector.all_pushes(),
    );

    // Now post a *new* response (id strictly greater than the watermark)
    // and assert it routes to the asker the reconstruct path recovered.
    let resp = log
        .append(signal(
            "human",
            "human_response",
            serde_json::json!({ "question_id": card_id, "choice": "yes" }),
        ))
        .unwrap();
    router2.handle_event(&resp);

    let lead_pushes = injector.pushes_for("S-LEAD");
    assert!(
        lead_pushes
            .iter()
            .any(|p| p.contains("[human_response] yes")),
        "after reopen + reconstruct, response must route to original asker; got {lead_pushes:?}",
    );
}

#[test]
fn reconstruct_and_dispatch_read_legacy_runner_status_rows() {
    // Mission logs written before #632 carry `runner_status` rows; the
    // replay fold and the live dispatch both read them as `session_status`.
    let dir = tempfile::tempdir().unwrap();
    let log = Arc::new(EventLog::open(dir.path()).unwrap());
    let roster = vec![slot_with_role("lead", true), slot_with_role("impl", false)];

    log.append(signal(
        "impl",
        "runner_status",
        serde_json::json!({ "state": "busy" }),
    ))
    .unwrap();

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
    router.reconstruct_from_log().unwrap();
    assert!(matches!(
        router.state.lock().unwrap().status.get("impl"),
        Some(super::SessionActivityState::Busy),
    ));

    let live_idle = log
        .append(signal(
            "impl",
            "runner_status",
            serde_json::json!({ "state": "idle" }),
        ))
        .unwrap();
    router.handle_event(&live_idle);
    assert!(matches!(
        router.state.lock().unwrap().status.get("impl"),
        Some(super::SessionActivityState::Idle),
    ));
    assert!(injector.all_pushes().is_empty());
}

#[test]
fn reconstruct_recovers_latest_session_status_only() {
    // Reopen-path test for arch §5.5.1: latest reported state per handle.
    // busy → idle → busy must leave status[impl] = Busy after reconstruct,
    // and replay of the historical sequence must never inject into the
    // lead (session_status is observability-only since #125).
    let dir = tempfile::tempdir().unwrap();
    let log = Arc::new(EventLog::open(dir.path()).unwrap());
    let roster = vec![slot_with_role("lead", true), slot_with_role("impl", false)];

    log.append(signal(
        "impl",
        "session_status",
        serde_json::json!({ "state": "busy" }),
    ))
    .unwrap();
    log.append(signal(
        "impl",
        "session_status",
        serde_json::json!({ "state": "idle", "note": "first idle" }),
    ))
    .unwrap();
    log.append(signal(
        "impl",
        "session_status",
        serde_json::json!({ "state": "busy" }),
    ))
    .unwrap();

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
    router.reconstruct_from_log().unwrap();

    // After reconstruct, the state map reflects the latest reported state.
    assert!(matches!(
        router.state.lock().unwrap().status.get("impl"),
        Some(super::SessionActivityState::Busy),
    ));

    // Bus replay of the historical events must not inject into the lead —
    // session_status is observability-only.
    for entry in log.read_from(0).unwrap() {
        router.handle_event(&entry.event);
    }
    assert!(
        injector.all_pushes().is_empty(),
        "historical session_status replay must not push to lead; got {:?}",
        injector.all_pushes(),
    );

    // A *new* idle event after reconstruct also must not push — same
    // observability-only contract applies to live events.
    let live_idle = log
        .append(signal(
            "impl",
            "session_status",
            serde_json::json!({ "state": "idle" }),
        ))
        .unwrap();
    router.handle_event(&live_idle);
    assert!(
        injector.pushes_for("S-LEAD").is_empty(),
        "live session_status idle must not push to lead",
    );
    assert!(matches!(
        router.state.lock().unwrap().status.get("impl"),
        Some(super::SessionActivityState::Idle),
    ));
}

#[test]
fn fresh_mission_start_does_not_call_reconstruct() {
    // Regression on the reviewer's caveat: if a fresh-start mount called
    // reconstruct_from_log() over the just-written opening events, the
    // watermark would cover mission_goal and downstream handlers (UI
    // toasts, future signal-routing extensions) wouldn't see them on
    // replay. mission_start must skip reconstruct entirely.
    //
    // Plan 0007 update: the lead's launch prompt is now delivered at
    // spawn time via the positional `[PROMPT]` argv, not by the
    // `mission_goal` handler — see
    // `ops::mission::mission_start` and
    // `router::runtime::first_turn_argv`. This test still guards the
    // "no watermark over opening events" invariant by replaying the
    // log without reconstruct; we just no longer assert the handler
    // injects to the lead, since that side effect moved upstream.
    let dir = tempfile::tempdir().unwrap();
    let log = Arc::new(EventLog::open(dir.path()).unwrap());
    let roster = vec![slot_with_role("lead", true)];

    log.append(signal(
        "system",
        "mission_start",
        serde_json::json!({ "title": "fresh" }),
    ))
    .unwrap();
    log.append(signal(
        "human",
        "mission_goal",
        serde_json::json!({ "text": "go" }),
    ))
    .unwrap();

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
    router.register_sessions(&[("lead".into(), "S-LEAD".into())]);
    // NB: no reconstruct call. The bus's initial replay drives the
    // bootstrap; the router exercises every handler without
    // suppressing the opening events.
    for entry in log.read_from(0).unwrap() {
        router.handle_event(&entry.event);
    }
    // Spawn-time argv owns launch-prompt delivery now, so the
    // handler-side recorder should be silent for the lead.
    assert!(
        injector.pushes_for("S-LEAD").is_empty(),
        "mission_goal handler must not inject the launch prompt (#88)"
    );
}

#[test]
fn stopped_session_delivery_waits_for_respawn() {
    let (router, injector, log, _dir) =
        fixture(vec![slot_with_role("lead", true)], &[("lead", "S-LEAD")]);
    let ask = log
        .append(signal(
            "lead",
            "ask_human",
            serde_json::json!({
                "prompt": "Continue?",
                "choices": ["Resume", "Cancel mission"],
            }),
        ))
        .unwrap();
    router.handle_event(&ask);
    let card_id = read_signals(&log)
        .into_iter()
        .find(|event| {
            event
                .signal_type
                .as_ref()
                .is_some_and(|signal| signal.as_str() == "human_question")
        })
        .expect("router must append human_question")
        .id;

    injector.mark_dead("S-LEAD");
    let ev = log
        .append(signal(
            "human",
            "human_response",
            serde_json::json!({
                "question_id": card_id,
                "choice": "Cancel mission",
            }),
        ))
        .unwrap();
    router.handle_event(&ev);

    assert!(injector.pushes_for("S-LEAD").is_empty());
    let warnings = read_signals(&log)
        .into_iter()
        .filter(|event| {
            event
                .signal_type
                .as_ref()
                .is_some_and(|signal| signal.as_str() == "mission_warning")
        })
        .collect::<Vec<_>>();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].payload["message"]
        .as_str()
        .unwrap()
        .contains("queued until the session resumes"));

    router
        .inject_and_submit("lead", b"second deferred delivery")
        .unwrap();
    assert_eq!(
        read_signals(&log)
            .into_iter()
            .filter(|event| {
                event
                    .signal_type
                    .as_ref()
                    .is_some_and(|signal| signal.as_str() == "mission_warning")
            })
            .count(),
        1,
        "only the first queued delivery should add a feed warning"
    );

    injector.respawn("S-LEAD");
    wait_until(Duration::from_millis(100), || {
        injector
            .submitted_bodies_for("S-LEAD")
            .iter()
            .any(|body| body.contains("[human_response] Cancel mission"))
    });
}

#[test]
fn reconstruct_tolerates_malformed_lines_like_the_bus() {
    // Regression: the bus uses `read_from_lossy` so a single bad NDJSON
    // line doesn't poison projection. `reconstruct_from_log` must do the
    // same — otherwise reopen would fail on a log the bus is otherwise
    // happily tailing.
    use std::io::Write;

    let dir = tempfile::tempdir().unwrap();
    let log = Arc::new(EventLog::open(dir.path()).unwrap());

    // Pre-seed an ask_human, then a hand-written malformed line, then a
    // matching human_question via the live router. Reconstruct must
    // recover the pending-ask mapping despite the bad line in between.
    let ask = log
        .append(signal(
            "lead",
            "ask_human",
            serde_json::json!({
                "prompt": "ok?",
                "choices": ["yes", "no"],
            }),
        ))
        .unwrap();
    {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.path().join("events.ndjson"))
            .unwrap();
        f.write_all(b"this is not json\n").unwrap();
    }

    let roster = vec![slot_with_role("lead", true)];
    // First mount handles the ask live — appends human_question.
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
        router.register_sessions(&[("lead".into(), "S-LEAD".into())]);
        router.handle_event(&ask);
    }
    let card_id = read_signals(&log)
        .into_iter()
        .find(|s| {
            s.signal_type
                .as_ref()
                .map(|t| t.as_str() == "human_question")
                .unwrap_or(false)
        })
        .expect("router must append human_question")
        .id;

    // Reopen + reconstruct: must not fail despite the malformed middle line.
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
    router2.register_sessions(&[("lead".into(), "S-LEAD".into())]);
    router2
        .reconstruct_from_log()
        .expect("reconstruct must tolerate malformed lines");

    // The pending-ask map should still resolve; route a human_response and
    // assert the answer reaches the lead.
    let resp = log
        .append(signal(
            "human",
            "human_response",
            serde_json::json!({ "question_id": card_id, "choice": "yes" }),
        ))
        .unwrap();
    router2.handle_event(&resp);
    let lead_pushes = injector.pushes_for("S-LEAD");
    assert!(
        lead_pushes
            .iter()
            .any(|p| p.contains("[human_response] yes")),
        "reconstruct must have recovered the pending ask; got {lead_pushes:?}",
    );
}
