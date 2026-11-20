use super::*;
use crate::session::hook_queue::TestHookRoute;
use crate::session::state::agent::{AgentEvent, ProgramOutcomePolicy};
use crate::session::status::TurnOutcome;

#[test]
fn installer_preserves_foreign_hooks_and_writes_identity_events_only() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join(".grok");
    install(&home).unwrap();
    assert!(!home.exists());
    std::fs::create_dir_all(home.join("hooks")).unwrap();
    let foreign = home.join("hooks/orca-status.json");
    std::fs::write(&foreign, "foreign").unwrap();
    install(&home).unwrap();
    let path = home.join("hooks/runner-status.json");
    let bytes = std::fs::read(&path).unwrap();
    install(&home).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(std::fs::read_to_string(foreign).unwrap(), "foreign");
    let document: Value = serde_json::from_slice(&bytes).unwrap();
    let mut keys: Vec<_> = document["hooks"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["SessionStart", "UserPromptSubmit"]);
    for event in EVENTS {
        let command = document["hooks"][event][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert_eq!(document["hooks"][event][0]["hooks"][0]["timeout"], 2);
        assert!(
            !command.contains("RUNNER_STATUS_HOOKS"),
            "identity hook must not gate on RUNNER_STATUS_HOOKS: {command}"
        );
    }
}

#[cfg(unix)]
#[test]
fn reporter_command_is_silent_with_unset_env_and_a_path_with_spaces() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let executable = root.path().join("Application Support/reporter");
    std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
    std::fs::write(&executable, "#!/bin/sh\nprintf noise\nexit 1\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    for reporter in [None, Some(executable.as_os_str())] {
        let mut command = std::process::Command::new("/bin/sh");
        command.args([
            "-c",
            hooks()["hooks"]["SessionStart"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap(),
        ]);
        command.env_remove("RUNNER_HOOK_EXECUTABLE");
        if let Some(reporter) = reporter {
            command.env("RUNNER_HOOK_EXECUTABLE", reporter);
        }
        let output = command.output().unwrap();
        assert!(output.status.success());
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn identity_hooks_emit_program_events_rekey_on_dashboard_switch_and_ignore_subagents() {
    let (route, receiver) = TestHookRoute::new(Runtime::Grok, "generation".into());
    let key = uuid::Uuid::new_v4().to_string();
    let next = uuid::Uuid::new_v4().to_string();
    let dashboard = uuid::Uuid::new_v4().to_string();
    let mut watcher = Watcher::new(
        receiver,
        &SpawnSpec {
            args: strings(&["-s", &key]),
            ..Default::default()
        },
    );
    let mut keys = Vec::new();
    let mut drain = |watcher: &mut Watcher| {
        let mut events = Vec::new();
        watcher
            .drain_events(
                0,
                &mut |event| {
                    events.push(event);
                    Default::default()
                },
                &mut |key| keys.push(key),
            )
            .unwrap();
        events
    };
    let report = |event: &str, id: &str| json!({"hook_event_name":event,"sessionId":id});

    drop(route.admit(report("SessionStart", &key)).unwrap());
    assert_eq!(
        drain(&mut watcher),
        vec![
            AgentEvent::ProgramSessionStarted {
                conversation_changed: false,
                compacted: false,
            },
            AgentEvent::ProgramOutcomeUnavailable,
        ]
    );

    drop(
        route
            .admit(json!({
                "hook_event_name": "UserPromptSubmit",
                "sessionId": key,
                "promptId": "p1",
                "prompt": "hello"
            }))
            .unwrap(),
    );
    assert_eq!(
        drain(&mut watcher),
        vec![AgentEvent::ProgramTurnConfirmed {
            conversation_changed: false,
            prompt: Some("hello".into()),
        }]
    );

    // Legacy status hooks (Stop, Notification, PostToolUse, etc.) are ignored and never arm hooks.
    for ignored in [
        json!({"hook_event_name":"Notification","sessionId":key,"notificationType":"permission_prompt"}),
        json!({"hook_event_name":"PostToolUse","sessionId":key}),
        json!({"hook_event_name":"Stop","sessionId":key,"reason":"end_turn"}),
        json!({"hook_event_name":"StopFailure","sessionId":key}),
        json!({"hook_event_name":"StopCancelled","sessionId":key}),
    ] {
        drop(route.admit(ignored).unwrap());
        assert!(drain(&mut watcher).is_empty());
    }

    // Subagent hooks are ignored.
    for field in ["subagentType", "subagent_type", "agentId", "agent_id"] {
        for event in ["SessionStart", "UserPromptSubmit"] {
            let mut payload = report(event, &next);
            payload[field] = json!("child");
            drop(route.admit(payload).unwrap());
            assert!(drain(&mut watcher).is_empty());
        }
    }

    // Stale generations are rejected at admission.
    let mut stale = report("SessionStart", &next);
    stale["generation"] = json!("old");
    assert!(route.admit(stale).is_err());

    // SessionStart rekeys to a new session.
    drop(route.admit(report("SessionStart", &next)).unwrap());
    assert_eq!(
        drain(&mut watcher),
        vec![AgentEvent::ProgramSessionStarted {
            conversation_changed: true,
            compacted: false,
        }]
    );

    // Dashboard switch via UserPromptSubmit rekeys without needing SessionStart.
    drop(route.admit(report("UserPromptSubmit", &dashboard)).unwrap());
    assert_eq!(
        drain(&mut watcher),
        vec![AgentEvent::ProgramTurnConfirmed {
            conversation_changed: true,
            prompt: None,
        }]
    );
    assert_eq!(keys, vec![next, dashboard]);
}

#[test]
fn transcript_tail_emits_live_outcomes_and_skips_preexisting_history() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("updates.jsonl");
    let key = uuid::Uuid::new_v4().to_string();
    let other_key = uuid::Uuid::new_v4().to_string();

    // Pre-existing turn_completed from before startup/resume must be skipped.
    std::fs::write(
        &transcript,
        format!(
            "{}\n",
            json!({
                "method": "_x.ai/session/update",
                "params": {
                    "sessionId": key,
                    "update": {
                        "sessionUpdate": "turn_completed",
                        "prompt_id": "old",
                        "stop_reason": "cancelled"
                    }
                }
            })
        ),
    )
    .unwrap();

    let (route, receiver) = TestHookRoute::new(Runtime::Grok, "generation".into());
    let mut watcher = Watcher::new(
        receiver,
        &SpawnSpec {
            args: strings(&["-s", &key]),
            ..Default::default()
        },
    );
    let drain = |watcher: &mut Watcher| {
        let mut events = Vec::new();
        watcher
            .drain_events(
                0,
                &mut |event| {
                    events.push(event);
                    Default::default()
                },
                &mut |_| {},
            )
            .unwrap();
        events
    };
    let append = |value: Value| {
        writeln!(
            std::fs::OpenOptions::new()
                .append(true)
                .open(&transcript)
                .unwrap(),
            "{value}"
        )
        .unwrap();
    };

    drop(
        route
            .admit(json!({
                "hook_event_name": "SessionStart",
                "sessionId": key,
                "transcriptPath": transcript,
            }))
            .unwrap(),
    );
    assert_eq!(
        drain(&mut watcher),
        vec![AgentEvent::ProgramSessionStarted {
            conversation_changed: false,
            compacted: false,
        }]
    );

    // Turn 1: mismatched prompt_id and foreign sessionId are ignored; matching cancelled emits Interrupted.
    drop(
        route
            .admit(json!({
                "hook_event_name": "UserPromptSubmit",
                "sessionId": key,
                "promptId": "p1",
                "transcriptPath": transcript,
            }))
            .unwrap(),
    );
    assert_eq!(
        drain(&mut watcher),
        vec![AgentEvent::ProgramTurnConfirmed {
            conversation_changed: false,
            prompt: None,
        }]
    );
    append(json!({
        "params": {
            "sessionId": other_key,
            "update": {"sessionUpdate": "turn_completed", "prompt_id": "p1", "stop_reason": "cancelled"}
        }
    }));
    append(json!({
        "params": {
            "sessionId": key,
            "update": {"sessionUpdate": "turn_completed", "prompt_id": "old", "stop_reason": "cancelled"}
        }
    }));
    assert!(drain(&mut watcher).is_empty());

    append(json!({
        "params": {
            "sessionId": key,
            "update": {"sessionUpdate": "turn_completed", "prompt_id": "p1", "stop_reason": "cancelled"}
        }
    }));
    assert_eq!(
        drain(&mut watcher),
        vec![AgentEvent::ProgramTurnOutcome {
            outcome: TurnOutcome::Interrupted,
            policy: ProgramOutcomePolicy::DeferWhileNativeActive,
        }]
    );

    // Turn 2: error_kind emits Failed (DeferWhileNativeActive so working -> idle without OSC error reconciles).
    drop(
        route
            .admit(json!({
                "hook_event_name": "UserPromptSubmit",
                "sessionId": key,
                "promptId": "p2",
            }))
            .unwrap(),
    );
    drain(&mut watcher);
    append(json!({
        "params": {
            "sessionId": key,
            "update": {"sessionUpdate": "turn_completed", "prompt_id": "p2", "error_kind": "rate_limit"}
        }
    }));
    assert_eq!(
        drain(&mut watcher),
        vec![AgentEvent::ProgramTurnOutcome {
            outcome: TurnOutcome::Failed,
            policy: ProgramOutcomePolicy::DeferWhileNativeActive,
        }]
    );

    // Turn 3: end_turn emits ProgramTurnFinished.
    drop(
        route
            .admit(json!({
                "hook_event_name": "UserPromptSubmit",
                "sessionId": key,
                "promptId": "p3",
            }))
            .unwrap(),
    );
    drain(&mut watcher);
    append(json!({
        "params": {
            "sessionId": key,
            "update": {"sessionUpdate": "turn_completed", "prompt_id": "p3", "stop_reason": "end_turn"}
        }
    }));
    assert_eq!(drain(&mut watcher), vec![AgentEvent::ProgramTurnFinished]);

    // Missing transcriptPath (e.g. grok host-server) emits ProgramOutcomeUnavailable once.
    let (hs_route, hs_receiver) = TestHookRoute::new(Runtime::Grok, "hs".into());
    let mut hs_watcher = Watcher::from_receiver(hs_receiver);
    drop(
        hs_route
            .admit(json!({"hook_event_name": "SessionStart", "sessionId": key}))
            .unwrap(),
    );
    assert_eq!(
        drain(&mut hs_watcher),
        vec![
            AgentEvent::ProgramSessionStarted {
                conversation_changed: false,
                compacted: false,
            },
            AgentEvent::ProgramOutcomeUnavailable,
        ]
    );
    // Repeated hook without transcriptPath does not spam ProgramOutcomeUnavailable.
    drop(
        hs_route
            .admit(
                json!({"hook_event_name": "UserPromptSubmit", "sessionId": key, "promptId": "p4"}),
            )
            .unwrap(),
    );
    assert_eq!(
        drain(&mut hs_watcher),
        vec![AgentEvent::ProgramTurnConfirmed {
            conversation_changed: false,
            prompt: None,
        }]
    );

    // Reported transcriptPath that cannot be opened yet emits ProgramOutcomeUnavailable once
    // while retaining retries so a file appearing later still tails live outcomes cleanly.
    let missing_transcript = dir.path().join("delayed-updates.jsonl");
    let (miss_route, miss_receiver) = TestHookRoute::new(Runtime::Grok, "miss".into());
    let mut miss_watcher = Watcher::from_receiver(miss_receiver);
    drop(
        miss_route
            .admit(json!({
                "hook_event_name": "SessionStart",
                "sessionId": key,
                "transcriptPath": missing_transcript,
            }))
            .unwrap(),
    );
    assert_eq!(
        drain(&mut miss_watcher),
        vec![
            AgentEvent::ProgramSessionStarted {
                conversation_changed: false,
                compacted: false,
            },
            AgentEvent::ProgramOutcomeUnavailable,
        ]
    );
    // Repeated polls while still unreadable must not spam ProgramOutcomeUnavailable.
    assert!(drain(&mut miss_watcher).is_empty());
    drop(
        miss_route
            .admit(json!({
                "hook_event_name": "UserPromptSubmit",
                "sessionId": key,
                "promptId": "p_late",
                "transcriptPath": missing_transcript,
            }))
            .unwrap(),
    );
    assert_eq!(
        drain(&mut miss_watcher),
        vec![AgentEvent::ProgramTurnConfirmed {
            conversation_changed: false,
            prompt: None,
        }]
    );
    // Once the transcript file appears later, retries open it and deliver live records.
    std::fs::write(
        &missing_transcript,
        format!(
            "{}\n",
            json!({
                "params": {
                    "sessionId": key,
                    "update": {"sessionUpdate": "turn_completed", "prompt_id": "p_late", "error_kind": "rate_limit"}
                }
            })
        ),
    )
    .unwrap();
    assert_eq!(
        drain(&mut miss_watcher),
        vec![AgentEvent::ProgramTurnOutcome {
            outcome: TurnOutcome::Failed,
            policy: ProgramOutcomePolicy::DeferWhileNativeActive,
        }]
    );
}

#[test]
fn grok_watcher_discovers_session_updates_jsonl_without_transcript_path() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let grok_home = dir.path().join(".grok");
    let key1 = uuid::Uuid::new_v4().to_string();
    let key2 = uuid::Uuid::new_v4().to_string();
    let launch_ms = 2_000_000i64;

    let (route, receiver) = TestHookRoute::new(Runtime::Grok, "discover".into());
    let mut watcher = Watcher::with_home(receiver, Some(grok_home.clone()), launch_ms);
    let drain = |watcher: &mut Watcher| {
        let mut events = Vec::new();
        watcher
            .drain_events(
                0,
                &mut |event| {
                    events.push(event);
                    Default::default()
                },
                &mut |_| {},
            )
            .unwrap();
        events
    };

    // 1. Fresh Grok 1.0.89 SessionStart omits transcriptPath while updates.jsonl does not exist yet.
    drop(
        route
            .admit(json!({
                "hook_event_name": "SessionStart",
                "sessionId": key1,
                "cwd": "/private/tmp/qa-proj",
            }))
            .unwrap(),
    );
    assert_eq!(
        drain(&mut watcher),
        vec![
            AgentEvent::ProgramSessionStarted {
                conversation_changed: false,
                compacted: false,
            },
            AgentEvent::ProgramOutcomeUnavailable,
        ]
    );

    // Grok creates sessions/<encoded-cwd>/<session_id>/updates.jsonl and runs a permission Reject.
    let s1_dir = grok_home
        .join("sessions")
        .join("%2Fprivate%2Ftmp%2Fqa-proj")
        .join(&key1);
    std::fs::create_dir_all(&s1_dir).unwrap();
    let s1_updates = s1_dir.join("updates.jsonl");
    // Pre-launch record must be filtered out even when opened from byte 0.
    std::fs::write(
        &s1_updates,
        format!(
            "{}\n{}\n",
            json!({
                "timestamp": (launch_ms - 10_000) / 1_000,
                "method": "_x.ai/session/update",
                "params": {
                    "sessionId": key1,
                    "update": {"sessionUpdate": "turn_completed", "prompt_id": "pre", "stop_reason": "cancelled"},
                    "_meta": {"agentTimestampMs": launch_ms - 10_000}
                }
            }),
            json!({
                "timestamp": (launch_ms + 1_000) / 1_000,
                "method": "_x.ai/session/update",
                "params": {
                    "sessionId": key1,
                    "update": {"sessionUpdate": "hook_execution", "event_name": "session_start"},
                    "_meta": {"agentTimestampMs": launch_ms + 1_000}
                }
            }),
        ),
    )
    .unwrap();
    assert!(drain(&mut watcher).is_empty());

    drop(
        route
            .admit(json!({
                "hook_event_name": "UserPromptSubmit",
                "sessionId": key1,
                "promptId": "p_reject",
                "cwd": "/private/tmp/qa-proj",
            }))
            .unwrap(),
    );
    assert_eq!(
        drain(&mut watcher),
        vec![AgentEvent::ProgramTurnConfirmed {
            conversation_changed: false,
            prompt: None,
        }]
    );
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(&s1_updates)
            .unwrap(),
        "{}",
        json!({
            "timestamp": (launch_ms + 5_000) / 1_000,
            "timestamp_us": (launch_ms + 5_000) * 1_000,
            "method": "_x.ai/session/update",
            "params": {
                "sessionId": key1,
                "update": {
                    "sessionUpdate": "turn_completed",
                    "prompt_id": "p_reject",
                    "stop_reason": "cancelled"
                },
                "_meta": {
                    "agentTimestampMs": launch_ms + 5_000,
                    "cancellationCategory": "PermissionRejected"
                }
            }
        })
    )
    .unwrap();
    assert_eq!(
        drain(&mut watcher),
        vec![AgentEvent::ProgramTurnOutcome {
            outcome: TurnOutcome::Interrupted,
            policy: ProgramOutcomePolicy::DeferWhileNativeActive,
        }]
    );

    // 2. Switching to an existing session in a different encoded-cwd folder without transcriptPath
    // skips its pre-existing history and tails live records from its updates.jsonl.
    let s2_dir = grok_home
        .join("sessions")
        .join("%2Fprivate%2Ftmp%2Fother-proj")
        .join(&key2);
    std::fs::create_dir_all(&s2_dir).unwrap();
    let s2_updates = s2_dir.join("updates.jsonl");
    std::fs::write(
        &s2_updates,
        format!(
            "{}\n",
            json!({
                "method": "_x.ai/session/update",
                "params": {
                    "sessionId": key2,
                    "update": {"sessionUpdate": "turn_completed", "prompt_id": "old2", "stop_reason": "cancelled"}
                }
            })
        ),
    )
    .unwrap();
    drop(
        route
            .admit(json!({
                "hook_event_name": "SessionStart",
                "sessionId": key2,
                "cwd": "/private/tmp/other-proj",
            }))
            .unwrap(),
    );
    assert_eq!(
        drain(&mut watcher),
        vec![AgentEvent::ProgramSessionStarted {
            conversation_changed: true,
            compacted: false,
        }]
    );
    drop(
        route
            .admit(json!({
                "hook_event_name": "UserPromptSubmit",
                "sessionId": key2,
                "promptId": "p2_live",
                "cwd": "/private/tmp/other-proj",
            }))
            .unwrap(),
    );
    let _ = drain(&mut watcher);
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(&s2_updates)
            .unwrap(),
        "{}",
        json!({
            "method": "_x.ai/session/update",
            "params": {
                "sessionId": key2,
                "update": {"sessionUpdate": "turn_completed", "prompt_id": "p2_live", "stop_reason": "cancelled"}
            }
        })
    )
    .unwrap();
    assert_eq!(
        drain(&mut watcher),
        vec![AgentEvent::ProgramTurnOutcome {
            outcome: TurnOutcome::Interrupted,
            policy: ProgramOutcomePolicy::DeferWhileNativeActive,
        }]
    );

    // 3. Resumed SpawnSpec (--resume <key2> with GROK_HOME) opens at end so pre-existing history
    // never replays while new records emit.
    let (res_route, res_receiver) = TestHookRoute::new(Runtime::Grok, "resume".into());
    let mut env = std::collections::BTreeMap::new();
    env.insert("GROK_HOME".into(), grok_home.to_string_lossy().into_owned());
    let mut res_watcher = Watcher::new(
        res_receiver,
        &SpawnSpec {
            args: strings(&["--resume", &key2]),
            env,
            ..Default::default()
        },
    );
    drop(
        res_route
            .admit(json!({
                "hook_event_name": "SessionStart",
                "sessionId": key2,
                "cwd": "/private/tmp/other-proj",
            }))
            .unwrap(),
    );
    assert_eq!(
        drain(&mut res_watcher),
        vec![AgentEvent::ProgramSessionStarted {
            conversation_changed: false,
            compacted: false,
        }]
    );
    drop(
        res_route
            .admit(json!({
                "hook_event_name": "UserPromptSubmit",
                "sessionId": key2,
                "promptId": "p2_after_resume",
                "cwd": "/private/tmp/other-proj",
            }))
            .unwrap(),
    );
    let _ = drain(&mut res_watcher);
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(&s2_updates)
            .unwrap(),
        "{}",
        json!({
            "method": "_x.ai/session/update",
            "params": {
                "sessionId": key2,
                "update": {"sessionUpdate": "turn_completed", "prompt_id": "p2_after_resume", "error_kind": "rate_limit"}
            }
        })
    )
    .unwrap();
    assert_eq!(
        drain(&mut res_watcher),
        vec![AgentEvent::ProgramTurnOutcome {
            outcome: TurnOutcome::Failed,
            policy: ProgramOutcomePolicy::DeferWhileNativeActive,
        }]
    );
    // Direct encoded-cwd probe hit on resumed SessionStart without any full bucket scan.
    assert_eq!(res_watcher.full_scan_count(), 0);
}

#[test]
fn grok_watcher_preserves_history_boundary_on_recovery_and_bounds_idle_rescans() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let grok_home = dir.path().join(".grok");
    // Create 40 unrelated workspace buckets to verify idle polling does not rescan all buckets.
    for i in 0..40 {
        std::fs::create_dir_all(
            grok_home
                .join("sessions")
                .join(format!("%2Fwork%2Fb{i:02}")),
        )
        .unwrap();
    }

    let t0 = Instant::now();
    let launch_ms = 2_000_000i64;
    let key1 = uuid::Uuid::new_v4().to_string();
    let key2 = uuid::Uuid::new_v4().to_string();
    let key3 = uuid::Uuid::new_v4().to_string();

    let drain = |watcher: &mut Watcher| {
        let mut events = Vec::new();
        watcher
            .drain_events(
                0,
                &mut |event| {
                    events.push(event);
                    Default::default()
                },
                &mut |_| {},
            )
            .unwrap();
        events
    };

    // 1. Bounded idle rescanning + O(1) direct probe on late creation:
    // Start a chat whose updates.jsonl does not exist yet; repeated 50ms idle drains must not
    // rescan every bucket on each tick, and creating the file at the encoded-cwd path is
    // picked up immediately via direct probe without triggering a full scan.
    let (route, receiver) = TestHookRoute::new(Runtime::Grok, "backoff".into());
    let mut watcher = clock::at(t0, launch_ms, || {
        Watcher::with_home(receiver, Some(grok_home.clone()), launch_ms)
    });
    clock::at(t0, launch_ms, || {
        drop(
            route
                .admit(json!({
                    "hook_event_name": "SessionStart",
                    "sessionId": key1,
                    "cwd": "/private/tmp/late-proj",
                }))
                .unwrap(),
        );
        assert_eq!(
            drain(&mut watcher),
            vec![
                AgentEvent::ProgramSessionStarted {
                    conversation_changed: false,
                    compacted: false,
                },
                AgentEvent::ProgramOutcomeUnavailable,
            ]
        );
    });
    assert_eq!(
        watcher.full_scan_count(),
        1,
        "initial missing-file SessionStart scans once"
    );
    // 4 idle drains at 50ms intervals inside the initial 250ms backoff window do ZERO rescans.
    for step in 1..=4 {
        clock::at(
            t0 + Duration::from_millis(step * 50),
            launch_ms + (step * 50) as i64,
            || {
                assert!(drain(&mut watcher).is_empty());
            },
        );
    }
    assert_eq!(
        watcher.full_scan_count(),
        1,
        "idle polls within backoff window must not rescan every workspace bucket"
    );
    // Creating updates.jsonl at the expected encoded-cwd path is found via O(1) direct probe
    // even while the full-scan backoff is still active (still at +210ms < 250ms).
    let s1_updates = grok_home
        .join("sessions")
        .join("%2Fprivate%2Ftmp%2Flate-proj")
        .join(&key1)
        .join("updates.jsonl");
    std::fs::create_dir_all(s1_updates.parent().unwrap()).unwrap();
    std::fs::write(
        &s1_updates,
        format!(
            "{}\n",
            json!({
                "timestamp_us": (launch_ms + 210) * 1_000,
                "method": "_x.ai/session/update",
                "params": {
                    "sessionId": key1,
                    "update": {"sessionUpdate": "turn_completed", "prompt_id": "p1", "stop_reason": "cancelled"},
                    "_meta": {"agentTimestampMs": launch_ms + 210, "cancellationCategory": "PermissionRejected"}
                }
            })
        ),
    )
    .unwrap();
    clock::at(t0 + Duration::from_millis(210), launch_ms + 210, || {
        assert_eq!(
            drain(&mut watcher),
            vec![AgentEvent::ProgramTurnOutcome {
                outcome: TurnOutcome::Interrupted,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
            }]
        );
    });
    assert_eq!(
        watcher.full_scan_count(),
        1,
        "late-created file in expected encoded-cwd bucket must resolve via O(1) direct probe"
    );

    // 2. Conversation switch whose transcript is initially unavailable and then recovers with
    // an in-process pre-switch cancelled turn (written AFTER process launch, BEFORE switch!)
    // plus a post-handoff rejection: pre-switch record must NEVER replay while idle or after
    // prompt handoff, and the post-handoff rejection must emit once.
    let switch_at = t0 + Duration::from_secs(5);
    let switch_ms = launch_ms + 5_000;
    clock::at(switch_at, switch_ms, || {
        drop(
            route
                .admit(json!({
                    "hook_event_name": "SessionStart",
                    "sessionId": key2,
                    "cwd": "/private/tmp/other-switch",
                }))
                .unwrap(),
        );
        assert_eq!(
            drain(&mut watcher),
            vec![
                AgentEvent::ProgramSessionStarted {
                    conversation_changed: true,
                    compacted: false,
                },
                AgentEvent::ProgramOutcomeUnavailable,
            ]
        );
    });
    let scans_after_switch = watcher.full_scan_count();
    // File appears in a slug-hash bucket (direct probe cannot hit) with an old in-process
    // cancelled record from launch_ms + 2_000 (< switch_ms) while the switched chat is idle.
    let s2_updates = grok_home
        .join("sessions")
        .join("slug-hash-bucket")
        .join(&key2)
        .join("updates.jsonl");
    std::fs::create_dir_all(s2_updates.parent().unwrap()).unwrap();
    std::fs::write(
        &s2_updates,
        format!(
            "{}\n",
            json!({
                "timestamp_us": (launch_ms + 2_000) * 1_000,
                "method": "_x.ai/session/update",
                "params": {
                    "sessionId": key2,
                    "update": {"sessionUpdate": "turn_completed", "prompt_id": "pre_switch", "stop_reason": "cancelled"},
                    "_meta": {"agentTimestampMs": launch_ms + 2_000}
                }
            })
        ),
    )
    .unwrap();
    // Backoff expires at +250ms; recovery opens at EOF while idle so the pre-switch record is skipped.
    clock::at(
        switch_at + Duration::from_millis(300),
        switch_ms + 300,
        || {
            assert!(
                drain(&mut watcher).is_empty(),
                "recovering an unavailable switched transcript while idle must not replay pre-switch history"
            );
        },
    );
    assert_eq!(watcher.full_scan_count(), scans_after_switch + 1);
    // Now run a real post-handoff rejection on key2 and verify it emits once.
    clock::at(
        switch_at + Duration::from_secs(1),
        switch_ms + 1_000,
        || {
            drop(
                route
                    .admit(json!({
                        "hook_event_name": "UserPromptSubmit",
                        "sessionId": key2,
                        "promptId": "p_post_switch",
                        "cwd": "/private/tmp/other-switch",
                    }))
                    .unwrap(),
            );
            assert_eq!(
                drain(&mut watcher),
                vec![AgentEvent::ProgramTurnConfirmed {
                    conversation_changed: false,
                    prompt: None,
                }]
            );
        },
    );
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(&s2_updates)
            .unwrap(),
        "{}",
        json!({
            "timestamp_us": (switch_ms + 1_500) * 1_000,
            "method": "_x.ai/session/update",
            "params": {
                "sessionId": key2,
                "update": {"sessionUpdate": "turn_completed", "prompt_id": "p_post_switch", "stop_reason": "cancelled"},
                "_meta": {"agentTimestampMs": switch_ms + 1_500, "cancellationCategory": "PermissionRejected"}
            }
        })
    )
    .unwrap();
    clock::at(
        switch_at + Duration::from_millis(1_500),
        switch_ms + 1_500,
        || {
            assert_eq!(
                drain(&mut watcher),
                vec![AgentEvent::ProgramTurnOutcome {
                    outcome: TurnOutcome::Interrupted,
                    policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                }]
            );
        },
    );

    // 3. Conversation switch where a post-switch prompt arrives BEFORE the transcript file
    // becomes available, and the recovered file contains BOTH an in-process pre-switch
    // cancelled record (< switch3_ms) AND the new post-switch rejection (>= switch3_ms).
    let switch3_at = t0 + Duration::from_secs(10);
    let switch3_ms = launch_ms + 10_000;
    clock::at(switch3_at, switch3_ms, || {
        drop(
            route
                .admit(json!({
                    "hook_event_name": "SessionStart",
                    "sessionId": key3,
                    "cwd": "/private/tmp/switch3",
                }))
                .unwrap(),
        );
        assert_eq!(
            drain(&mut watcher),
            vec![
                AgentEvent::ProgramSessionStarted {
                    conversation_changed: true,
                    compacted: false,
                },
                AgentEvent::ProgramOutcomeUnavailable,
            ]
        );
    });
    // Prompt arrives while file is still missing; hook resets scan backoff immediately.
    clock::at(
        switch3_at + Duration::from_millis(100),
        switch3_ms + 100,
        || {
            drop(
                route
                    .admit(json!({
                        "hook_event_name": "UserPromptSubmit",
                        "sessionId": key3,
                        "promptId": "p3_new",
                        "cwd": "/private/tmp/switch3",
                    }))
                    .unwrap(),
            );
        },
    );
    let s3_updates = grok_home
        .join("sessions")
        .join("slug-hash-bucket-3")
        .join(&key3)
        .join("updates.jsonl");
    std::fs::create_dir_all(s3_updates.parent().unwrap()).unwrap();
    std::fs::write(
        &s3_updates,
        format!(
            "{}\n{}\n",
            json!({
                "timestamp_us": (launch_ms + 8_000) * 1_000,
                "method": "_x.ai/session/update",
                "params": {
                    "sessionId": key3,
                    "update": {"sessionUpdate": "turn_completed", "stop_reason": "cancelled"},
                    "_meta": {"agentTimestampMs": launch_ms + 8_000}
                }
            }),
            json!({
                "timestamp_us": (switch3_ms + 500) * 1_000,
                "method": "_x.ai/session/update",
                "params": {
                    "sessionId": key3,
                    "update": {"sessionUpdate": "turn_completed", "prompt_id": "p3_new", "stop_reason": "cancelled"},
                    "_meta": {"agentTimestampMs": switch3_ms + 500, "cancellationCategory": "PermissionRejected"}
                }
            }),
        ),
    )
    .unwrap();
    clock::at(
        switch3_at + Duration::from_millis(500),
        switch3_ms + 500,
        || {
            assert_eq!(
                drain(&mut watcher),
                vec![
                    AgentEvent::ProgramTurnConfirmed {
                        conversation_changed: false,
                        prompt: None,
                    },
                    AgentEvent::ProgramTurnOutcome {
                        outcome: TurnOutcome::Interrupted,
                        policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                    },
                ],
                "post-handoff recovery must skip pre-switch records and admit the new rejection"
            );
        },
    );

    // 4. Unix permission-denied recovery on --resume: file exists at SessionStart so path is
    // found, open_at_end fails with PermissionDenied, and once readable again while idle,
    // recovery opens at EOF instead of replaying the old cancelled turn.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let res_key = uuid::Uuid::new_v4().to_string();
        let res_updates = grok_home
            .join("sessions")
            .join("%2Fprivate%2Ftmp%2Fres-perm")
            .join(&res_key)
            .join("updates.jsonl");
        std::fs::create_dir_all(res_updates.parent().unwrap()).unwrap();
        std::fs::write(
            &res_updates,
            format!(
                "{}\n",
                json!({
                    "method": "_x.ai/session/update",
                    "params": {
                        "sessionId": res_key,
                        "update": {"sessionUpdate": "turn_completed", "stop_reason": "cancelled"}
                    }
                })
            ),
        )
        .unwrap();
        std::fs::set_permissions(&res_updates, std::fs::Permissions::from_mode(0o000)).unwrap();

        let (res_route, res_receiver) = TestHookRoute::new(Runtime::Grok, "res-perm".into());
        let mut env = std::collections::BTreeMap::new();
        env.insert("GROK_HOME".into(), grok_home.to_string_lossy().into_owned());
        let mut res_watcher = clock::at(t0, launch_ms, || {
            Watcher::new(
                res_receiver,
                &SpawnSpec {
                    args: strings(&["--resume", &res_key]),
                    env,
                    ..Default::default()
                },
            )
        });
        clock::at(t0, launch_ms, || {
            drop(
                res_route
                    .admit(json!({
                        "hook_event_name": "SessionStart",
                        "sessionId": res_key,
                        "cwd": "/private/tmp/res-perm",
                    }))
                    .unwrap(),
            );
            assert_eq!(
                drain(&mut res_watcher),
                vec![
                    AgentEvent::ProgramSessionStarted {
                        conversation_changed: false,
                        compacted: false,
                    },
                    AgentEvent::ProgramOutcomeUnavailable,
                ]
            );
        });
        // Recover permissions while idle: old timestamp-less cancelled turn must NOT replay.
        std::fs::set_permissions(&res_updates, std::fs::Permissions::from_mode(0o644)).unwrap();
        clock::at(t0 + Duration::from_millis(100), launch_ms + 100, || {
            assert!(
                drain(&mut res_watcher).is_empty(),
                "recovering an initially-unreadable resumed transcript must open at EOF and skip old history"
            );
        });
        // Subsequent live rejection on the recovered resumed chat emits once.
        clock::at(t0 + Duration::from_millis(200), launch_ms + 200, || {
            drop(
                res_route
                    .admit(json!({
                        "hook_event_name": "UserPromptSubmit",
                        "sessionId": res_key,
                        "promptId": "p_res_new",
                        "cwd": "/private/tmp/res-perm",
                    }))
                    .unwrap(),
            );
            let _ = drain(&mut res_watcher);
        });
        writeln!(
            std::fs::OpenOptions::new()
                .append(true)
                .open(&res_updates)
                .unwrap(),
            "{}",
            json!({
                "method": "_x.ai/session/update",
                "params": {
                    "sessionId": res_key,
                    "update": {"sessionUpdate": "turn_completed", "prompt_id": "p_res_new", "stop_reason": "cancelled"}
                }
            })
        )
        .unwrap();
        clock::at(t0 + Duration::from_millis(300), launch_ms + 300, || {
            assert_eq!(
                drain(&mut res_watcher),
                vec![AgentEvent::ProgramTurnOutcome {
                    outcome: TurnOutcome::Interrupted,
                    policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                }]
            );
        });
    }

    // 5. Prompt-led dashboard switch (UserPromptSubmit without SessionStart) where the
    // matching turn_completed lands AFTER hook admission and BEFORE drain: admission
    // timestamp (not drain time) is the boundary, so the older record is skipped and the
    // post-admission pre-drain outcome emits once.
    let key4 = uuid::Uuid::new_v4().to_string();
    let s4_updates = grok_home
        .join("sessions")
        .join("%2Fprivate%2Ftmp%2Fdash-switch")
        .join(&key4)
        .join("updates.jsonl");
    std::fs::create_dir_all(s4_updates.parent().unwrap()).unwrap();
    let dash_at = t0 + Duration::from_secs(20);
    let dash_ms = launch_ms + 20_000;
    std::fs::write(
        &s4_updates,
        format!(
            "{}\n",
            json!({
                "timestamp_us": (dash_ms - 50) * 1_000,
                "method": "_x.ai/session/update",
                "params": {
                    "sessionId": key4,
                    "update": {"sessionUpdate": "turn_completed", "stop_reason": "cancelled"},
                    "_meta": {"agentTimestampMs": dash_ms - 50}
                }
            })
        ),
    )
    .unwrap();
    // Admit UserPromptSubmit at t=dash_ms, append its turn_completed at t=dash_ms+10, drain at t=dash_ms+50.
    clock::at(dash_at, dash_ms, || {
        drop(
            route
                .admit(json!({
                    "hook_event_name": "UserPromptSubmit",
                    "sessionId": key4,
                    "promptId": "p_dash",
                    "cwd": "/private/tmp/dash-switch",
                }))
                .unwrap(),
        );
    });
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(&s4_updates)
            .unwrap(),
        "{}",
        json!({
            "timestamp_us": (dash_ms + 10) * 1_000,
            "method": "_x.ai/session/update",
            "params": {
                "sessionId": key4,
                "update": {"sessionUpdate": "turn_completed", "prompt_id": "p_dash", "stop_reason": "cancelled"},
                "_meta": {"agentTimestampMs": dash_ms + 10, "cancellationCategory": "PermissionRejected"}
            }
        })
    )
    .unwrap();
    clock::at(dash_at + Duration::from_millis(50), dash_ms + 50, || {
        assert_eq!(
            drain(&mut watcher),
            vec![
                AgentEvent::ProgramTurnConfirmed {
                    conversation_changed: true,
                    prompt: None,
                },
                AgentEvent::ProgramTurnOutcome {
                    outcome: TurnOutcome::Interrupted,
                    policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                },
            ],
            "prompt-led dashboard switch must admit post-admission pre-drain outcome once"
        );
        assert!(drain(&mut watcher).is_empty());
    });

    // 6. Same-drain SessionStart + UserPromptSubmit on a switched session with an older
    // history record AND a post-admission pre-drain outcome: SessionStart must not seek past
    // the following prompt's live outcome.
    let key5 = uuid::Uuid::new_v4().to_string();
    let s5_updates = grok_home
        .join("sessions")
        .join("%2Fprivate%2Ftmp%2Fsame-drain")
        .join(&key5)
        .join("updates.jsonl");
    std::fs::create_dir_all(s5_updates.parent().unwrap()).unwrap();
    let same_at = t0 + Duration::from_secs(30);
    let same_ms = launch_ms + 30_000;
    std::fs::write(
        &s5_updates,
        format!(
            "{}\n",
            json!({
                "timestamp_us": (same_ms - 50) * 1_000,
                "method": "_x.ai/session/update",
                "params": {
                    "sessionId": key5,
                    "update": {"sessionUpdate": "turn_completed", "stop_reason": "cancelled"},
                    "_meta": {"agentTimestampMs": same_ms - 50}
                }
            })
        ),
    )
    .unwrap();
    clock::at(same_at, same_ms, || {
        drop(
            route
                .admit(json!({
                    "hook_event_name": "SessionStart",
                    "sessionId": key5,
                    "cwd": "/private/tmp/same-drain",
                }))
                .unwrap(),
        );
    });
    clock::at(same_at + Duration::from_millis(10), same_ms + 10, || {
        drop(
            route
                .admit(json!({
                    "hook_event_name": "UserPromptSubmit",
                    "sessionId": key5,
                    "promptId": "p_same",
                    "cwd": "/private/tmp/same-drain",
                }))
                .unwrap(),
        );
    });
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(&s5_updates)
            .unwrap(),
        "{}",
        json!({
            "timestamp_us": (same_ms + 20) * 1_000,
            "method": "_x.ai/session/update",
            "params": {
                "sessionId": key5,
                "update": {"sessionUpdate": "turn_completed", "prompt_id": "p_same", "error_kind": "rate_limit"},
                "_meta": {"agentTimestampMs": same_ms + 20}
            }
        })
    )
    .unwrap();
    clock::at(same_at + Duration::from_millis(60), same_ms + 60, || {
        assert_eq!(
            drain(&mut watcher),
            vec![
                AgentEvent::ProgramSessionStarted {
                    conversation_changed: true,
                    compacted: false,
                },
                AgentEvent::ProgramTurnConfirmed {
                    conversation_changed: false,
                    prompt: None,
                },
                AgentEvent::ProgramTurnOutcome {
                    outcome: TurnOutcome::Failed,
                    policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                },
            ],
            "same-drain SessionStart + UserPromptSubmit must skip pre-switch history and emit the prompt's outcome once"
        );
        assert!(drain(&mut watcher).is_empty());
    });
}

#[test]
fn grok_status_report_accepts_simultaneous_camelcase_and_snakecase_aliases() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("session");
    std::fs::create_dir_all(&session_dir).unwrap();
    let updates_path = session_dir.join("updates.jsonl");
    let chat_history_path = session_dir.join("chat_history.jsonl");
    std::fs::write(&updates_path, "").unwrap();
    std::fs::write(&chat_history_path, "").unwrap();

    let key = uuid::Uuid::new_v4().to_string();
    let sub_key = uuid::Uuid::new_v4().to_string();
    let (route, receiver) = TestHookRoute::new(Runtime::Grok, "dual-keys".into());
    let mut watcher = Watcher::new(
        receiver,
        &SpawnSpec {
            args: strings(&["-s", &key]),
            ..Default::default()
        },
    );
    let mut keys = Vec::new();
    let mut drain = |watcher: &mut Watcher| {
        let mut events = Vec::new();
        watcher
            .drain_events(
                0,
                &mut |event| {
                    events.push(event);
                    Default::default()
                },
                &mut |k| keys.push(k),
            )
            .unwrap();
        events
    };

    // Real Grok 1.0.89 emits both camelCase and snake_case aliases in the same object.
    // Also verify chat_history.jsonl maps to sibling updates.jsonl.
    drop(
        route
            .admit(json!({
                "hookEventName": "session_start",
                "hook_event_name": "SessionStart",
                "sessionId": key,
                "session_id": key,
                "transcriptPath": chat_history_path,
                "transcript_path": chat_history_path,
                "cwd": dir.path(),
                "workspaceRoot": dir.path(),
                "permissionMode": "default",
                "permission_mode": "default",
            }))
            .unwrap(),
    );
    assert_eq!(
        drain(&mut watcher),
        vec![AgentEvent::ProgramSessionStarted {
            conversation_changed: false,
            compacted: false,
        }]
    );

    // Subagent payload carrying both camelCase and snake_case keys must still be ignored.
    drop(
        route
            .admit(json!({
                "hookEventName": "user_prompt_submit",
                "hook_event_name": "UserPromptSubmit",
                "sessionId": sub_key,
                "session_id": sub_key,
                "subagentType": "explore",
                "subagent_type": "explore",
                "agentId": "sub-1",
                "agent_id": "sub-1",
            }))
            .unwrap(),
    );
    assert!(drain(&mut watcher).is_empty());

    // Dual-key UserPromptSubmit followed by real PermissionRejected turn_completed in updates.jsonl.
    drop(
        route
            .admit(json!({
                "hookEventName": "user_prompt_submit",
                "hook_event_name": "UserPromptSubmit",
                "sessionId": key,
                "session_id": key,
                "promptId": "p_dual",
                "prompt_id": "p_dual",
                "prompt": "run bash",
                "transcriptPath": updates_path,
                "transcript_path": updates_path,
                "cwd": dir.path(),
                "permissionMode": "default",
                "permission_mode": "default",
            }))
            .unwrap(),
    );
    assert_eq!(
        drain(&mut watcher),
        vec![AgentEvent::ProgramTurnConfirmed {
            conversation_changed: false,
            prompt: Some("run bash".into()),
        }]
    );
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(&updates_path)
            .unwrap(),
        "{}",
        json!({
            "method": "_x.ai/session/update",
            "params": {
                "sessionId": key,
                "update": {
                    "sessionUpdate": "turn_completed",
                    "prompt_id": "p_dual",
                    "stop_reason": "cancelled"
                },
                "_meta": {"cancellationCategory": "PermissionRejected"}
            }
        })
    )
    .unwrap();
    assert_eq!(
        drain(&mut watcher),
        vec![AgentEvent::ProgramTurnOutcome {
            outcome: TurnOutcome::Interrupted,
            policy: ProgramOutcomePolicy::DeferWhileNativeActive,
        }]
    );
}
