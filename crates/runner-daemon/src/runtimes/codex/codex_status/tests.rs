use super::*;
use crate::model::Runtime;
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::Write;

struct TestWatcher {
    inner: CodexStatusWatcher,
    hooks: crate::session::hook_queue::TestHookRoute,
}

impl TestWatcher {
    fn new(generation: &str) -> Self {
        let (hooks, receiver) =
            crate::session::hook_queue::TestHookRoute::new(Runtime::Codex, generation.into());
        Self {
            inner: CodexStatusWatcher::from_receiver(receiver),
            hooks,
        }
    }

    fn drain(&mut self) -> (Vec<AgentEvent>, Vec<String>) {
        let mut events = Vec::new();
        let mut starts = Vec::new();
        self.inner
            .drain_events(
                0,
                |event| {
                    events.push(event);
                    Default::default()
                },
                |id| starts.push(id),
            )
            .unwrap();
        (events, starts)
    }
}

fn abort_record(turn: &str, reason: &str) -> Value {
    json!({"type":"event_msg","payload":{"type":"turn_aborted","turn_id":turn,"reason":reason}})
}

fn task_complete_record(turn: &str, error: Option<Value>) -> Value {
    json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":turn,"last_agent_message":null,"error":error}})
}

#[test]
fn only_identity_hook_events_are_registered_and_removed_status_hooks_are_ignored() {
    assert_eq!(EVENTS, &["SessionStart", "UserPromptSubmit"]);

    let mut watcher = TestWatcher::new("current");
    let current = uuid::Uuid::new_v4().to_string();
    let other = uuid::Uuid::new_v4().to_string();
    for removed in [
        "PreToolUse",
        "PostToolUse",
        "PreCompact",
        "PostCompact",
        "Stop",
        "Interrupt",
        "SessionEnd",
    ] {
        watcher
            .hooks
            .admit(json!({
                "hook_event_name": removed,
                "session_id": other,
                "turn_id": "t0",
                "transcript_path": "/tmp/should-be-ignored.jsonl",
            }))
            .unwrap();
    }
    let (events, starts) = watcher.drain();
    assert!(events.is_empty());
    assert!(starts.is_empty());
    assert!(watcher.inner.parser.session_id.is_none());
    assert!(watcher.inner.parser.transcript_path.is_none());

    watcher
        .hooks
        .admit(json!({
            "hook_event_name": "SessionStart",
            "session_id": current,
            "source": "startup",
            "transcript_path": "/tmp/current.jsonl",
        }))
        .unwrap();
    let (events, starts) = watcher.drain();
    assert!(matches!(
        events.as_slice(),
        [AgentEvent::ProgramSessionStarted {
            conversation_changed: false,
            compacted: false,
        }]
    ));
    assert_eq!(starts, vec![current.clone()]);
    assert!(!watcher.inner.parser.reads_transcript());

    watcher
        .hooks
        .admit(json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": current,
            "turn_id": "turn-1",
            "transcript_path": "/tmp/current.jsonl",
        }))
        .unwrap();
    let (events, starts) = watcher.drain();
    assert!(matches!(
        events.as_slice(),
        [AgentEvent::ProgramTurnConfirmed {
            conversation_changed: false,
            ..
        }]
    ));
    assert!(starts.is_empty());
    assert!(watcher.inner.parser.reads_transcript());

    // Removed hooks arriving mid-turn never alter identity or emit status events.
    for removed in [
        "PreToolUse",
        "PostToolUse",
        "PreCompact",
        "PostCompact",
        "Stop",
        "Interrupt",
        "SessionEnd",
    ] {
        watcher
            .hooks
            .admit(json!({
                "hook_event_name": removed,
                "session_id": current,
                "turn_id": "turn-1",
            }))
            .unwrap();
    }
    let (events, starts) = watcher.drain();
    assert!(events.is_empty());
    assert!(starts.is_empty());
    assert_eq!(
        watcher.inner.parser.session_id.as_deref(),
        Some(current.as_str())
    );
    assert_eq!(watcher.inner.parser.turn_id.as_deref(), Some("turn-1"));
    assert!(watcher.inner.parser.reads_transcript());
}

#[test]
fn route_and_parser_isolate_generations_caller_threads_subagents_and_retired_sessions() {
    use runner_core::protocol::hook::{HookReport, VERSION};
    use std::sync::Arc;

    let routes = Arc::new(crate::session::hook_queue::HookRoutes::default());
    let receiver = routes.register(Runtime::Codex, "runner".into(), "launch".into());
    let mut watcher = CodexStatusWatcher::from_receiver(receiver);
    let first = uuid::Uuid::new_v4().to_string();
    let second = uuid::Uuid::new_v4().to_string();

    let make = |generation: &str, event: &str, caller: Option<&str>, payload: Value| HookReport {
        bridge_unavailable: false,
        version: VERSION,
        runtime: Runtime::Codex,
        session_id: "runner".into(),
        generation: generation.into(),
        event: event.into(),
        payload,
        caller_thread_id: caller.map(str::to_owned),
    };

    // Wrong generation and mismatched/missing caller_thread_id are rejected at admission.
    assert!(routes
        .admit(make(
            "stale",
            "SessionStart",
            Some(&first),
            json!({"hook_event_name":"SessionStart","session_id":first,"source":"startup"}),
        ))
        .is_err());
    for caller in [None, Some(""), Some("child-thread")] {
        assert!(routes
            .admit(make(
                "launch",
                "UserPromptSubmit",
                caller,
                json!({"hook_event_name":"UserPromptSubmit","session_id":first,"turn_id":"t1"}),
            ))
            .is_err());
    }

    // Subagent reports and non-UUID session_ids never emit conversation keys; subagents never confirm turns.
    drop(
        routes
            .admit(make(
                "launch",
                "SessionStart",
                Some(&first),
                json!({"hook_event_name":"SessionStart","session_id":first,"agent_id":"sub"}),
            ))
            .unwrap(),
    );
    drop(
        routes
            .admit(make(
                "launch",
                "UserPromptSubmit",
                Some(&first),
                json!({"hook_event_name":"UserPromptSubmit","session_id":first,"turn_id":"sub-turn","agent_id":"sub"}),
            ))
            .unwrap(),
    );
    drop(
        routes
            .admit(make(
                "launch",
                "SessionStart",
                Some("not-a-uuid"),
                json!({"hook_event_name":"SessionStart","session_id":"not-a-uuid","source":"startup"}),
            ))
            .unwrap(),
    );
    let mut events = Vec::new();
    let mut starts = Vec::new();
    watcher
        .drain_events(
            0,
            |e| {
                events.push(e);
                Default::default()
            },
            |id| starts.push(id),
        )
        .unwrap();
    assert!(matches!(
        events.as_slice(),
        [AgentEvent::ProgramSessionStarted {
            conversation_changed: false,
            compacted: false,
        }]
    ));
    assert!(starts.is_empty());
    events.clear();

    // Valid SessionStart and UserPromptSubmit on session 1, then switch to session 2 via SessionStart.
    for (event, id, payload) in [
        (
            "SessionStart",
            first.as_str(),
            json!({"hook_event_name":"SessionStart","session_id":first,"source":"startup"}),
        ),
        (
            "UserPromptSubmit",
            first.as_str(),
            json!({"hook_event_name":"UserPromptSubmit","session_id":first,"turn_id":"t1"}),
        ),
        // Active-turn steer reuses the active turn_id and emits another confirmation.
        (
            "UserPromptSubmit",
            first.as_str(),
            json!({"hook_event_name":"UserPromptSubmit","session_id":first,"turn_id":"t1"}),
        ),
        // Switch to second session.
        (
            "SessionStart",
            second.as_str(),
            json!({"hook_event_name":"SessionStart","session_id":second,"source":"clear"}),
        ),
        // Late prompt from retired first session is ignored.
        (
            "UserPromptSubmit",
            first.as_str(),
            json!({"hook_event_name":"UserPromptSubmit","session_id":first,"turn_id":"t2"}),
        ),
        // Prompt on second session confirms turn.
        (
            "UserPromptSubmit",
            second.as_str(),
            json!({"hook_event_name":"UserPromptSubmit","session_id":second,"turn_id":"t1"}),
        ),
    ] {
        drop(
            routes
                .admit(make("launch", event, Some(id), payload))
                .unwrap(),
        );
    }
    let order = std::cell::RefCell::new(Vec::new());
    watcher
        .drain_events(
            0,
            |e| {
                order.borrow_mut().push("event");
                events.push(e);
                Default::default()
            },
            |id| {
                order.borrow_mut().push("session_start");
                starts.push(id);
            },
        )
        .unwrap();
    assert_eq!(starts, vec![first, second.clone()]);
    assert_eq!(
        *order.borrow(),
        vec![
            "session_start",
            "event",
            "event",
            "event",
            "session_start",
            "event",
            "event"
        ],
        "session_start must precede the triggering hook's AgentEvent"
    );
    assert!(matches!(
        events.as_slice(),
        [
            AgentEvent::ProgramSessionStarted {
                conversation_changed: true,
                compacted: false,
            },
            AgentEvent::ProgramTurnConfirmed {
                conversation_changed: false,
                ..
            },
            AgentEvent::ProgramTurnConfirmed {
                conversation_changed: false,
                ..
            },
            AgentEvent::ProgramSessionStarted {
                conversation_changed: true,
                compacted: false,
            },
            AgentEvent::ProgramTurnConfirmed {
                conversation_changed: false,
                ..
            }
        ]
    ));
    assert_eq!(watcher.parser.session_id.as_deref(), Some(second.as_str()));
    assert_eq!(watcher.parser.turn_id.as_deref(), Some("t1"));
}

#[test]
fn active_turn_steer_reuses_turn_id_while_finished_and_retired_turns_are_rejected() {
    let mut parser = CodexParser::default();
    let session = uuid::Uuid::new_v4().to_string();
    let prompt = |turn: &str| StatusReport {
        hook_event_name: "UserPromptSubmit".into(),
        source: None,
        session_id: Some(session.clone()),
        turn_id: Some(turn.into()),
        transcript_path: None,
        agent_id: None,
    };

    assert!(matches!(
        parser.hook(prompt("turn-1")),
        Some(AgentEvent::ProgramTurnConfirmed {
            conversation_changed: false,
            ..
        })
    ));
    // A mid-turn steer reuses the active turn_id while turn_finished is false without retiring it.
    assert!(matches!(
        parser.hook(prompt("turn-1")),
        Some(AgentEvent::ProgramTurnConfirmed {
            conversation_changed: false,
            ..
        })
    ));
    assert!(parser.reads_transcript());
    assert!(matches!(
        parser.record(&abort_record("turn-1", "interrupted")),
        Some(AgentEvent::ProgramTurnOutcome {
            outcome: TurnOutcome::Interrupted,
            ..
        })
    ));
    assert!(!parser.reads_transcript());
    // Once finished (by abort or task_complete), the same turn_id cannot re-confirm or reopen polling.
    assert!(parser.hook(prompt("turn-1")).is_none());
    assert!(!parser.reads_transcript());

    assert!(matches!(
        parser.hook(prompt("turn-2")),
        Some(AgentEvent::ProgramTurnConfirmed {
            conversation_changed: false,
            ..
        })
    ));
    assert!(parser.reads_transcript());
    // Retired turn-1 cannot take back turn tracking.
    assert!(parser.hook(prompt("turn-1")).is_none());
    assert_eq!(parser.turn_id.as_deref(), Some("turn-2"));
    assert!(matches!(
        parser.record(&task_complete_record("turn-2", None)),
        Some(AgentEvent::ProgramTurnFinished)
    ));
    assert!(!parser.reads_transcript());
    assert!(parser.hook(prompt("turn-2")).is_none());
    assert!(!parser.reads_transcript());
}

#[test]
fn transcript_reports_interrupted_and_api_error_for_matching_turn_only() {
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("rollout.jsonl");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&transcript)
        .unwrap();

    // Historical records written before this turn must never fire.
    writeln!(file, "{}", abort_record("old-turn", "interrupted")).unwrap();
    writeln!(
        file,
        "{}",
        task_complete_record("old-turn", Some(json!({"message":"stale"})))
    )
    .unwrap();
    file.flush().unwrap();

    let mut watcher = TestWatcher::new("current");
    let session = uuid::Uuid::new_v4().to_string();
    watcher
        .hooks
        .admit(json!({
            "hook_event_name": "SessionStart",
            "session_id": session,
            "source": "startup",
            "transcript_path": null,
        }))
        .unwrap();
    let (events, starts) = watcher.drain();
    assert!(matches!(
        events.as_slice(),
        [AgentEvent::ProgramSessionStarted {
            conversation_changed: false,
            compacted: false,
        }]
    ));
    assert_eq!(starts, vec![session.clone()]);

    // Turn 1: UserPromptSubmit provides transcript_path and confirms the turn.
    watcher
        .hooks
        .admit(json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": session,
            "turn_id": "turn-1",
            "transcript_path": transcript,
        }))
        .unwrap();
    let (events, _) = watcher.drain();
    assert!(matches!(
        events.as_slice(),
        [AgentEvent::ProgramTurnConfirmed {
            conversation_changed: false,
            ..
        }]
    ));
    assert!(watcher.inner.parser.reads_transcript());

    // Non-interrupted abort, wrong turn_id, and malformed JSON are ignored.
    writeln!(file, "{{not valid json").unwrap();
    writeln!(file, "{}", abort_record("turn-1", "replaced")).unwrap();
    writeln!(file, "{}", abort_record("other-turn", "interrupted")).unwrap();
    file.flush().unwrap();
    let (events, _) = watcher.drain();
    assert!(events.is_empty());
    assert!(watcher.inner.parser.reads_transcript());

    // Partial line waits for its newline, then emits Interrupted once.
    let line = format!("{}\n", abort_record("turn-1", "interrupted"));
    let (head, tail) = line.split_at(line.len() / 2);
    file.write_all(head.as_bytes()).unwrap();
    file.flush().unwrap();
    assert!(watcher.drain().0.is_empty());

    file.write_all(tail.as_bytes()).unwrap();
    // A duplicate abort for the same turn is ignored once turn_finished is set.
    writeln!(file, "{}", abort_record("turn-1", "interrupted")).unwrap();
    file.flush().unwrap();
    let (events, _) = watcher.drain();
    assert!(matches!(
        events.as_slice(),
        [AgentEvent::ProgramTurnOutcome {
            outcome: TurnOutcome::Interrupted,
            ..
        }]
    ));
    assert!(!watcher.inner.parser.reads_transcript());

    // Turn 2: clean task_complete (error: null) emits ProgramTurnFinished and stops polling.
    watcher
        .hooks
        .admit(json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": session,
            "turn_id": "turn-2",
            "transcript_path": transcript,
        }))
        .unwrap();
    assert!(matches!(
        watcher.drain().0.as_slice(),
        [AgentEvent::ProgramTurnConfirmed {
            conversation_changed: false,
            ..
        }]
    ));
    writeln!(file, "{}", task_complete_record("turn-2", None)).unwrap();
    file.flush().unwrap();
    assert!(matches!(
        watcher.drain().0.as_slice(),
        [AgentEvent::ProgramTurnFinished]
    ));
    assert!(!watcher.inner.parser.reads_transcript());

    // Turn 3: task_complete with an API error emits Failed.
    watcher
        .hooks
        .admit(json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": session,
            "turn_id": "turn-3",
            "transcript_path": transcript,
        }))
        .unwrap();
    assert!(matches!(
        watcher.drain().0.as_slice(),
        [AgentEvent::ProgramTurnConfirmed {
            conversation_changed: false,
            ..
        }]
    ));
    writeln!(
        file,
        "{}",
        task_complete_record(
            "turn-3",
            Some(json!({
                "message": "Selected model is at capacity.",
                "codex_error_info": "server_overloaded",
            }))
        )
    )
    .unwrap();
    file.flush().unwrap();
    assert!(matches!(
        watcher.drain().0.as_slice(),
        [AgentEvent::ProgramTurnOutcome {
            outcome: TurnOutcome::Failed,
            ..
        }]
    ));
    assert!(!watcher.inner.parser.reads_transcript());
}

#[test]
fn large_transcript_opens_from_tail_without_missing_current_turn_abort() {
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("large.jsonl");
    let padding = format!(
        "{}\n",
        json!({"type":"event_msg","payload":{"type":"note","turn_id":"old","body":"x".repeat(4096)}})
    );
    let mut content = String::with_capacity(1_300_000);
    while content.len() < 1_200_000 {
        content.push_str(&padding);
    }
    content.push_str(&format!("{}\n", abort_record("turn-big", "interrupted")));
    fs::write(&transcript, content).unwrap();

    let mut watcher = TestWatcher::new("current");
    let session = uuid::Uuid::new_v4().to_string();
    watcher
        .hooks
        .admit(json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": session,
            "turn_id": "turn-big",
            "transcript_path": transcript,
        }))
        .unwrap();
    let (events, starts) = watcher.drain();
    assert_eq!(starts, vec![session]);
    assert!(matches!(
        events.as_slice(),
        [
            AgentEvent::ProgramTurnConfirmed {
                conversation_changed: false,
                ..
            },
            AgentEvent::ProgramTurnOutcome {
                outcome: TurnOutcome::Interrupted,
                ..
            },
        ]
    ));
}
