use std::fs::File;
use std::io::Write;

use super::*;
use crate::model::Runtime;

fn report(fields: serde_json::Value) -> StatusReport {
    serde_json::from_value(fields).unwrap()
}

#[test]
fn only_identity_hooks_are_admitted_and_subagents_are_ignored() {
    let mut parser = ClaudeParser::default();
    let started = parser
        .hook(report(serde_json::json!({
            "hook_event_name": "SessionStart",
            "session_id": "main",
            "source": "startup",
            "transcript_path": "/tmp/main.jsonl",
        })))
        .unwrap();
    assert!(matches!(
        started,
        AgentEvent::ProgramSessionStarted {
            conversation_changed: false,
            compacted: false,
        }
    ));

    // Subagent hooks and legacy status hooks are dropped.
    assert!(parser
        .hook(report(serde_json::json!({
            "hook_event_name": "SessionStart",
            "session_id": "sub",
            "agent_id": "subagent-1",
        })))
        .is_none());
    for dropped in [
        "PreToolUse",
        "PostToolUse",
        "PostToolUseFailure",
        "PermissionRequest",
        "PermissionDenied",
        "Elicitation",
        "ElicitationResult",
        "PreCompact",
        "PostCompact",
        "Notification",
        "Stop",
        "StopFailure",
    ] {
        assert!(
            parser
                .hook(report(serde_json::json!({
                    "hook_event_name": dropped,
                    "session_id": "main",
                })))
                .is_none(),
            "legacy status hook {dropped} must be dropped"
        );
    }

    // UserPromptSubmit confirms the turn and carries its prompt text.
    let confirmed = parser
        .hook(report(serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": "main",
            "prompt_id": "p1",
            "prompt": "[runner] line",
        })))
        .unwrap();
    assert!(matches!(
        confirmed,
        AgentEvent::ProgramTurnConfirmed {
            conversation_changed: false,
            prompt: Some(ref text),
        } if text == "[runner] line"
    ));

    // Mismatched session_id on UserPromptSubmit is dropped.
    assert!(parser
        .hook(report(serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": "other",
            "prompt_id": "p2",
        })))
        .is_none());

    // Compaction SessionStart keeps conversation_changed=false and compacted=true.
    let compacted = parser
        .hook(report(serde_json::json!({
            "hook_event_name": "SessionStart",
            "session_id": "main",
            "source": "compact",
            "transcript_path": "/tmp/main-compact.jsonl",
        })))
        .unwrap();
    assert!(matches!(
        compacted,
        AgentEvent::ProgramSessionStarted {
            conversation_changed: false,
            compacted: true,
        }
    ));
    assert_eq!(
        parser.transcript_path.as_deref(),
        Some(std::path::Path::new("/tmp/main-compact.jsonl"))
    );

    // `/clear` switches to a new conversation key.
    let cleared = parser
        .hook(report(serde_json::json!({
            "hook_event_name": "SessionStart",
            "session_id": "next",
            "source": "clear",
            "transcript_path": "/tmp/next.jsonl",
        })))
        .unwrap();
    assert!(matches!(
        cleared,
        AgentEvent::ProgramSessionStarted {
            conversation_changed: true,
            compacted: false,
        }
    ));
}

#[test]
fn transcript_reads_user_interrupts_and_turn_duration_and_skips_preexisting_history_on_resume() {
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("session.jsonl");
    let mut file = File::create(&transcript).unwrap();
    let session_id = "11111111-1111-4111-8111-111111111111";
    // Pre-existing interrupt from before resume must be skipped by open_at_end.
    writeln!(
        file,
        "{}",
        serde_json::json!({
            "type": "user",
            "sessionId": session_id,
            "message": {"content": [{"type": "text", "text": "[Request interrupted by user]"}]}
        })
    )
    .unwrap();
    file.flush().unwrap();

    let (hooks, receiver) =
        crate::session::hook_queue::TestHookRoute::new(Runtime::ClaudeCode, "gen-1".into());
    let mut watcher = ClaudeStatusWatcher::from_receiver(receiver);
    hooks
        .admit(serde_json::json!({
            "generation": "gen-1",
            "hook_event_name": "SessionStart",
            "session_id": session_id,
            "source": "resume",
            "transcript_path": transcript,
        }))
        .unwrap();

    let mut events = Vec::new();
    let mut starts = Vec::new();
    watcher
        .drain_events(
            0,
            |event| {
                events.push(event);
                Default::default()
            },
            |key| starts.push(key),
        )
        .unwrap();
    assert_eq!(starts, [session_id]);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        AgentEvent::ProgramSessionStarted {
            conversation_changed: false,
            compacted: false,
        }
    ));
    events.clear();

    // New turn + interrupt marker appended after resume emits Interrupted once.
    hooks
        .admit(serde_json::json!({
            "generation": "gen-1",
            "hook_event_name": "UserPromptSubmit",
            "session_id": session_id,
            "prompt_id": "turn-1",
            "prompt": "run sleep",
        }))
        .unwrap();
    // Sidechain and foreign session records must be ignored.
    for ignored in [
        serde_json::json!({
            "type": "user",
            "sessionId": "other",
            "message": {"content": "[Request interrupted by user]"}
        }),
        serde_json::json!({
            "type": "user",
            "sessionId": session_id,
            "isSidechain": true,
            "message": {"content": "[Request interrupted by user]"}
        }),
        serde_json::json!({
            "type": "user",
            "sessionId": session_id,
            "promptId": "other-turn",
            "message": {"content": "[Request interrupted by user]"}
        }),
    ] {
        writeln!(file, "{ignored}").unwrap();
    }
    writeln!(
        file,
        "{}",
        serde_json::json!({
            "type": "user",
            "sessionId": session_id,
            "promptId": "turn-1",
            "message": {"content": [{"type": "text", "text": "[Request interrupted by user for tool use]"}]}
        })
    )
    .unwrap();
    writeln!(
        file,
        "{}",
        serde_json::json!({
            "type": "system",
            "subtype": "turn_duration",
            "sessionId": session_id,
        })
    )
    .unwrap();
    file.flush().unwrap();

    watcher
        .drain_events(
            0,
            |event| {
                events.push(event);
                Default::default()
            },
            |key| starts.push(key),
        )
        .unwrap();
    assert_eq!(events.len(), 3);
    assert!(matches!(
        events[0],
        AgentEvent::ProgramTurnConfirmed {
            conversation_changed: false,
            prompt: Some(ref p),
        } if p == "run sleep"
    ));
    assert!(matches!(
        events[1],
        AgentEvent::ProgramTurnOutcome {
            outcome: TurnOutcome::Interrupted,
            policy: crate::session::state::agent::ProgramOutcomePolicy::DeferWhileNativeActive,
        }
    ));
    assert!(matches!(events[2], AgentEvent::ProgramTurnFinished));

    // Terminal API error message on root/current turn emits NativeFallbackOnly Failed
    // even when turn_duration is omitted; sidechain, non-error, and other-prompt records are ignored.
    events.clear();
    hooks
        .admit(serde_json::json!({
            "generation": "gen-1",
            "hook_event_name": "UserPromptSubmit",
            "session_id": session_id,
            "prompt_id": "turn-2",
            "transcript_path": transcript,
            "prompt": "next",
        }))
        .unwrap();
    for entry in [
        serde_json::json!({
            "type": "assistant",
            "isSidechain": true,
            "isApiErrorMessage": true,
            "sessionId": session_id,
        }),
        serde_json::json!({
            "type": "assistant",
            "isSidechain": false,
            "isApiErrorMessage": false,
            "sessionId": session_id,
        }),
        serde_json::json!({
            "type": "assistant",
            "isSidechain": false,
            "isApiErrorMessage": true,
            "sessionId": session_id,
            "promptId": "turn-1",
        }),
        serde_json::json!({
            "type": "assistant",
            "isSidechain": false,
            "isApiErrorMessage": true,
            "sessionId": session_id,
            "error": "rate_limit",
            "message": {"role": "assistant", "model": "<synthetic>"},
        }),
        serde_json::json!({
            "type": "assistant",
            "isSidechain": false,
            "isApiErrorMessage": true,
            "sessionId": session_id,
        }),
    ] {
        writeln!(file, "{entry}").unwrap();
    }
    file.flush().unwrap();
    watcher
        .drain_events(
            0,
            |event| {
                events.push(event);
                Default::default()
            },
            |key| starts.push(key),
        )
        .unwrap();
    assert_eq!(events.len(), 2);
    assert!(matches!(
        events[0],
        AgentEvent::ProgramTurnConfirmed {
            conversation_changed: false,
            prompt: Some(ref p),
        } if p == "next"
    ));
    assert!(matches!(
        events[1],
        AgentEvent::ProgramTurnOutcome {
            outcome: TurnOutcome::Failed,
            policy: crate::session::state::agent::ProgramOutcomePolicy::NativeFallbackOnly,
        }
    ));
}
