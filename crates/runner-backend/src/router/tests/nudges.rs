use super::*;

#[test]
fn directed_message_nudges_target_only() {
    // Pull-based inbox routing strands the worker without a stdin
    // poke. A directed message must wake the target with a one-line
    // notification; the sender must not be echoed back to themselves.
    let (router, injector, log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    let direct = log.append(message("lead", Some("impl"), "go")).unwrap();
    router.handle_event(&direct);
    let impl_pushes = injector.pushes_for("S-IMPL");
    assert_eq!(impl_pushes.len(), 1);
    assert!(impl_pushes[0].contains("[inbox]"));
    assert!(impl_pushes[0].contains("from @lead"));
    assert!(impl_pushes[0].contains("runner msg read"));
    // Sender is not nudged.
    assert!(injector.pushes_for("S-LEAD").is_empty());
}

#[test]
fn input_clear_flush_reparks_when_typing_resumes_during_grace() {
    let (router, injector, log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    injector.set_pending("S-IMPL");

    let direct = log.append(message("lead", Some("impl"), "go")).unwrap();
    router.handle_event(&direct);
    assert!(injector.pushes_for("S-IMPL").is_empty());
    assert!(!matches!(
        router.state.lock().unwrap().status.get("impl"),
        Some(super::SessionActivityState::Busy)
    ));

    injector.clear_pending("S-IMPL");
    assert!(
        injector.pushes_for("S-IMPL").is_empty(),
        "flush must leave a grace period before writing the deferred body"
    );
    injector.set_pending("S-IMPL");
    std::thread::sleep(Duration::from_millis(550));
    assert!(
        injector.pushes_for("S-IMPL").is_empty(),
        "typing a new draft during the grace period must keep the delivery parked"
    );
    injector.exit("S-IMPL");
}

#[test]
fn input_clear_flushes_after_quiet_500ms_grace() {
    let (router, injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    injector.set_pending("S-IMPL");
    router.inject_and_submit("impl", b"deferred relay").unwrap();

    let cleared_at = Instant::now();
    injector.clear_pending("S-IMPL");
    std::thread::sleep(Duration::from_millis(250));
    // A loaded runner can oversleep past the grace; only assert the
    // "still parked" half while we are provably inside it.
    if cleared_at.elapsed() < super::INPUT_CLEAR_FLUSH_GRACE {
        assert!(injector.submitted_bodies_for("S-IMPL").is_empty());
    }
    wait_until(Duration::from_millis(400), || {
        injector.submitted_bodies_for("S-IMPL") == ["deferred relay"]
    });
    assert!(cleared_at.elapsed() >= Duration::from_millis(500));
}

#[test]
fn observed_hello_delete_to_empty_flushes_a_parked_nudge() {
    let (router, injector, log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    injector.set_observed_input("S-IMPL", InputState::Drafting, false);
    let direct = log
        .append(message(
            "lead",
            Some("impl"),
            "after hello and five backspaces",
        ))
        .unwrap();
    router.handle_event(&direct);
    assert!(injector.pushes_for("S-IMPL").is_empty());

    let cleared_at = Instant::now();
    injector.set_observed_input("S-IMPL", InputState::Idle, true);
    wait_until(
        super::INPUT_CLEAR_FLUSH_GRACE + Duration::from_millis(300),
        || injector.submitted_bodies_for("S-IMPL").len() == 1,
    );
    assert!(cleared_at.elapsed() >= super::INPUT_CLEAR_FLUSH_GRACE);
}

#[test]
fn recent_typing_retries_after_quiet_window() {
    let (router, injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    injector.set_recent_typing("S-IMPL");
    router.inject_and_submit("impl", b"after quiet").unwrap();
    assert!(injector.pushes_for("S-IMPL").is_empty());

    wait_until(Duration::from_millis(300), || {
        injector.submitted_bodies_for("S-IMPL") == ["after quiet"]
    });
}

#[test]
fn deferred_nudges_coalesce_while_relays_preserve_order() {
    let (router, injector, log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    injector.set_pending("S-IMPL");

    for event in [
        log.append(message("lead", Some("impl"), "first")).unwrap(),
        log.append(signal(
            "human",
            "human_said",
            serde_json::json!({ "target": "impl", "text": "relay one" }),
        ))
        .unwrap(),
        log.append(message("lead", Some("impl"), "second")).unwrap(),
        log.append(signal(
            "human",
            "human_said",
            serde_json::json!({ "target": "impl", "text": "relay two" }),
        ))
        .unwrap(),
    ] {
        router.handle_event(&event);
    }
    assert!(injector.pushes_for("S-IMPL").is_empty());

    injector.clear_pending("S-IMPL");
    wait_until(Duration::from_secs(1), || {
        injector.submitted_bodies_for("S-IMPL").len() == 3
    });
    let bodies = injector.submitted_bodies_for("S-IMPL");
    assert!(bodies[0].contains("2 new messages"));
    assert_eq!(bodies[1], "relay one");
    assert_eq!(bodies[2], "relay two");
}

#[test]
fn deferred_delivery_flushes_on_respawn() {
    let (router, injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    injector.set_pending("S-IMPL");
    router
        .inject_and_submit("impl", b"relay after respawn")
        .unwrap();
    assert!(injector.pushes_for("S-IMPL").is_empty());

    injector.respawn("S-IMPL");
    wait_until(Duration::from_millis(250), || {
        injector.submitted_bodies_for("S-IMPL") == ["relay after respawn"]
    });
}

#[test]
fn session_exit_drops_deferred_delivery() {
    let (router, injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    injector.set_pending("S-IMPL");
    router
        .inject_and_submit("impl", b"must be dropped")
        .unwrap();

    injector.exit("S-IMPL");
    injector.respawn("S-IMPL");
    std::thread::sleep(Duration::from_millis(200));
    assert!(injector.pushes_for("S-IMPL").is_empty());
}

#[test]
fn blocked_empty_body_does_not_flush_a_stray_enter() {
    let (router, injector, _log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    injector.set_pending("S-IMPL");
    router.inject_and_submit("impl", b"").unwrap();
    injector.clear_pending("S-IMPL");
    std::thread::sleep(Duration::from_millis(200));
    assert!(injector.pushes_for("S-IMPL").is_empty());

    router.inject_and_submit("impl", b"").unwrap();
    wait_until(Duration::from_millis(250), || {
        injector.pushes_for("S-IMPL") == ["\r"]
    });
}

#[test]
fn broadcast_message_nudges_every_slot_except_sender() {
    let (router, injector, log, _dir) = fixture(
        vec![
            slot_with_role("lead", true),
            slot_with_role("impl", false),
            slot_with_role("reviewer", false),
        ],
        &[
            ("lead", "S-LEAD"),
            ("impl", "S-IMPL"),
            ("reviewer", "S-REV"),
        ],
    );
    let bcast = log.append(message("lead", None, "heads up")).unwrap();
    router.handle_event(&bcast);
    assert_eq!(injector.submitted_bodies_for("S-IMPL").len(), 1);
    assert_eq!(injector.submitted_bodies_for("S-REV").len(), 1);
    assert!(injector.pushes_for("S-LEAD").is_empty());
}

#[test]
fn human_messages_nudge_the_broadcast_roster_or_target_only() {
    let roster = || {
        vec![
            slot_with_role("lead", true),
            slot_with_role("impl", false),
            slot_with_role("reviewer", false),
        ]
    };
    let sessions = &[
        ("lead", "S-LEAD"),
        ("impl", "S-IMPL"),
        ("reviewer", "S-REV"),
    ];

    {
        let (router, injector, log, _dir) = fixture(roster(), sessions);
        let broadcast = log
            .append(message("human", None, "Message the crew"))
            .unwrap();
        router.handle_event(&broadcast);
        assert_eq!(injector.submitted_bodies_for("S-LEAD").len(), 1);
        assert_eq!(injector.submitted_bodies_for("S-IMPL").len(), 1);
        assert_eq!(injector.submitted_bodies_for("S-REV").len(), 1);
    }

    let (router, injector, log, _dir) = fixture(roster(), sessions);
    let targeted = log
        .append(message("human", Some("reviewer"), "Please review"))
        .unwrap();
    router.handle_event(&targeted);
    assert!(injector.pushes_for("S-LEAD").is_empty());
    assert!(injector.pushes_for("S-IMPL").is_empty());
    assert_eq!(injector.submitted_bodies_for("S-REV").len(), 1);
}

#[test]
fn human_broadcast_waits_for_sessions_still_starting() {
    let sessions = &[
        ("lead", "S-LEAD"),
        ("impl", "S-IMPL"),
        ("reviewer", "S-REV"),
    ];
    let (router, injector, log, _dir) = fixture(
        vec![
            slot_with_role("lead", true),
            slot_with_role("impl", false),
            slot_with_role("reviewer", false),
        ],
        sessions,
    );
    let pending: Vec<(String, String)> = sessions
        .iter()
        .map(|(handle, session)| (handle.to_string(), session.to_string()))
        .collect();
    router.register_pending_sessions(&pending);

    let broadcast = log
        .append(message("human", None, "Message the crew"))
        .unwrap();
    router.handle_event(&broadcast);
    assert!(injector.all_pushes().is_empty());
    assert!(!read_signals(&log).iter().any(|event| {
        event
            .signal_type
            .as_ref()
            .is_some_and(|signal| signal.as_str() == "mission_warning")
    }));

    for (_, session_id) in sessions {
        injector.respawn(session_id);
        wait_until(Duration::from_millis(100), || {
            injector.submitted_bodies_for(session_id).len() == 1
        });
    }
}

#[test]
fn message_self_directed_is_not_nudged() {
    // Edge case: a runner posting `runner msg post --to @self`. We
    // never echo a message back to its sender — that would create a
    // tight loop where reading the nudge prompts another post.
    let (router, injector, log, _dir) =
        fixture(vec![slot_with_role("lead", true)], &[("lead", "S-LEAD")]);
    let ev = log.append(message("lead", Some("lead"), "self")).unwrap();
    router.handle_event(&ev);
    assert!(injector.pushes_for("S-LEAD").is_empty());
}

#[test]
fn mission_goal_handler_no_longer_injects_launch_prompt() {
    // Plan 0007: lead launch prompt is delivered at spawn time via
    // the positional `[PROMPT]` argv (see
    // `router::runtime::first_turn_argv`). The bus's `mission_goal`
    // handler stays subscribed for UI consumers but no longer
    // injects the body — the prior post-spawn paste path raced
    // claude-code's trust-folder dialog / boot banner.
    let (router, injector, log, _dir) = fixture(
        vec![slot_with_role("lead", true), slot_with_role("impl", false)],
        &[("lead", "S-LEAD"), ("impl", "S-IMPL")],
    );
    let ev = log
        .append(signal(
            "human",
            "mission_goal",
            serde_json::json!({ "text": "ship v0" }),
        ))
        .unwrap();
    router.handle_event(&ev);

    // No injection (raw or paste) fires for the lead from
    // `mission_goal`. Spawn-time delivery is exercised end-to-end
    // by the `ops::mission` tests.
    assert!(
        injector.pushes_for("S-LEAD").is_empty(),
        "mission_goal handler must not inject the launch prompt — \
         that path moved to spawn-time argv (#88)"
    );
    // Workers were never targeted by `mission_goal` and still aren't.
    assert!(injector.pushes_for("S-IMPL").is_empty());
}
