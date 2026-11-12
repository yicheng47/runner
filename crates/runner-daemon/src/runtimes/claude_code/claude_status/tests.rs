use std::io::Write;

use super::*;

struct TestWatcher {
    inner: ClaudeStatusWatcher,
    hooks: crate::session::hook_queue::TestHookRoute,
    observation: ClaudeObservation,
    cancel: u8,
}
impl std::ops::Deref for TestWatcher {
    type Target = ClaudeStatusWatcher;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl std::ops::DerefMut for TestWatcher {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
impl TestWatcher {
    fn new(generation: String) -> Self {
        let (hooks, receiver) =
            crate::session::hook_queue::TestHookRoute::new(Runtime::ClaudeCode, generation);
        Self {
            inner: ClaudeStatusWatcher::from_receiver(receiver),
            hooks,
            observation: Default::default(),
            cancel: 0,
        }
    }
    fn drain_with_session_starts(
        &mut self,
        mut publish: impl FnMut(AgentObservation, StatusSource),
        starts: impl FnMut(String),
    ) -> Result<()> {
        self.inner.parser = self.observation.parser.clone();
        let cancel = std::mem::take(&mut self.cancel);
        let result = self.inner.drain_events(
            cancel,
            |event| {
                let source = if let AgentEvent::Batch { events, .. } = &event {
                    match events.first() {
                        Some(AgentEvent::LocalCancel { kind })
                            if kind & crate::session::state::CTRL_C_INTERRUPT != 0 =>
                        {
                            StatusSource::InputInterrupt
                        }
                        Some(AgentEvent::LocalCancel { .. }) => StatusSource::InputEscape,
                        _ => StatusSource::Hook,
                    }
                } else {
                    StatusSource::Hook
                };
                let (value, feedback) = self
                    .observation
                    .model
                    .reduce_with_feedback(event, crate::session::clock::timestamp_millis());
                if let Some(value) = value {
                    publish(value, source);
                }
                feedback
            },
            starts,
        );
        self.observation.value = self.observation.model.value.clone();
        self.observation.parser = self.inner.parser.clone();
        result
    }
    fn drain_status(&mut self, publish: impl FnMut(AgentObservation, StatusSource)) -> Result<()> {
        self.drain_with_session_starts(publish, |_| {})
    }
    fn drain(&mut self, mut publish: impl FnMut(SessionActivityState, StatusSource)) -> Result<()> {
        self.drain_status(|value, source| {
            publish(
                if value.activity == Activity::Working {
                    SessionActivityState::Busy
                } else {
                    SessionActivityState::Idle
                },
                source,
            )
        })
    }
}

fn report(event: &str, notification: Option<&str>, generation: &str) -> serde_json::Value {
    serde_json::json!({
        "generation": generation,
        "hook_event_name": event,
        "notification_type": notification,
    })
}

fn observe(
    model: &mut ClaudeObservation,
    event: &str,
    fields: serde_json::Value,
) -> Option<AgentObservation> {
    let mut fields = fields;
    fields["hook_event_name"] = event.into();
    model.observe(
        serde_json::from_value::<StatusReport>(fields).unwrap(),
        crate::session::clock::timestamp_millis(),
    )
}

#[test]
fn a_submitted_prompt_is_reported_beside_its_turn_start() {
    let report = |fields| serde_json::from_value::<StatusReport>(fields).unwrap();
    let mut parser = ClaudeParser::default();
    let events = parser
        .hook(report(serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "prompt": "[runner] line",
        })))
        .unwrap();
    assert!(matches!(
        events.as_slice(),
        [AgentEvent::PromptSubmitted { prompt }, AgentEvent::TurnStarted]
            if prompt == "[runner] line"
    ));
    let events = parser
        .hook(report(
            serde_json::json!({"hook_event_name": "UserPromptSubmit"}),
        ))
        .unwrap();
    assert!(matches!(events.as_slice(), [AgentEvent::TurnStarted]));
}

#[test]
fn session_start_publishes_hook_idle_and_preserves_compaction() {
    let mut model = ClaudeObservation::default();
    let started = observe(
        &mut model,
        "SessionStart",
        serde_json::json!({"session_id":"main","source":"startup"}),
    )
    .unwrap();
    assert_eq!(started.activity, Activity::Idle);
    assert_eq!(started.source, ObservationSource::Hook);
    assert_eq!(started.outcome, None);
    assert_eq!(started.detail, None);

    observe(
        &mut model,
        "UserPromptSubmit",
        serde_json::json!({"prompt_id":"first"}),
    );
    let stopped = observe(&mut model, "Stop", serde_json::json!({"prompt_id":"first"})).unwrap();
    assert_eq!(stopped.activity, Activity::Ready);
    assert_eq!(stopped.outcome, Some(TurnOutcome::Completed));

    let cleared = observe(
        &mut model,
        "SessionStart",
        serde_json::json!({"session_id":"main","source":"clear"}),
    )
    .unwrap();
    assert_eq!(cleared.activity, Activity::Idle);
    assert_eq!(cleared.source, ObservationSource::Hook);
    assert_eq!(cleared.outcome, None);

    observe(
        &mut model,
        "UserPromptSubmit",
        serde_json::json!({"prompt_id":"turn"}),
    );
    observe(&mut model, "PreCompact", serde_json::json!({}));
    let compacting = model.value.clone();
    assert!(observe(
        &mut model,
        "SessionStart",
        serde_json::json!({"session_id":"main","source":"compact"}),
    )
    .is_none());
    assert_eq!(model.value, compacting);
}

#[test]
fn question_is_immediate_and_transcript_resolution_is_correlated() {
    let mut model = ClaudeObservation::default();
    observe(
        &mut model,
        "SessionStart",
        serde_json::json!({"session_id":"main"}),
    );
    observe(
        &mut model,
        "UserPromptSubmit",
        serde_json::json!({"prompt_id":"current"}),
    );
    let question = observe(
        &mut model,
        "PreToolUse",
        serde_json::json!({"tool_name":"AskUserQuestion","tool_use_id":"question"}),
    )
    .unwrap();
    assert_eq!(question.interactions[0].reason, WaitReason::Answer);
    assert_eq!(question.interactions[0].owners, ["question"]);
    let result = serde_json::json!({"type":"user","sessionId":"main","promptId":"current","message":{"content":[{"type":"tool_result","tool_use_id":"question","is_error":true}]},"toolDenialKind":"user-rejected"});
    for (key, value) in [
        ("sessionId", serde_json::json!("other")),
        ("promptId", serde_json::json!("old")),
        ("isSidechain", serde_json::json!(true)),
    ] {
        let mut unrelated = result.clone();
        unrelated[key] = value;
        assert!(model.observe_transcript(&unrelated).is_none());
        assert!(model.value.needs_you());
    }
    let mut unrelated = result.clone();
    unrelated["message"]["content"][0]["tool_use_id"] = "other".into();
    assert!(model.observe_transcript(&unrelated).is_none());

    observe(
        &mut model,
        "Elicitation",
        serde_json::json!({"elicitation_id":"form","mcp_server_name":"server"}),
    );
    observe(
        &mut model,
        "Notification",
        serde_json::json!({"notification_type":"elicitation_dialog"}),
    );
    let cancelled = model.observe_transcript(&result).unwrap();
    assert_eq!(cancelled.outcome, Some(TurnOutcome::Interrupted));
    assert_eq!(cancelled.interactions.len(), 1);
    assert_eq!(cancelled.interactions[0].owners, ["form"]);
    assert!(model
        .observe_transcript(
            &serde_json::json!({"type":"system","subtype":"turn_duration","sessionId":"main"})
        )
        .is_none());
    assert!(model.value.needs_you());
}

#[test]
fn escape_cancellation_clears_without_a_stop_or_post_tool_hook() {
    let root = tempfile::tempdir().unwrap();
    let transcript = root.path().join("transcript.jsonl");
    let mut transcript_file = File::create(&transcript).unwrap();
    writeln!(transcript_file, "{}", "x".repeat(1024 * 1024 + 10)).unwrap();

    let mut watcher = TestWatcher::new("current".into());

    for payload in [
        serde_json::json!({"hook_event_name":"SessionStart","session_id":"main","transcript_path":transcript}),
        serde_json::json!({"hook_event_name":"UserPromptSubmit","prompt_id":"turn"}),
        serde_json::json!({"hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_use_id":"question"}),
    ] {
        let mut payload = payload;
        payload["generation"] = "current".into();
        watcher.hooks.admit(payload).unwrap();
    }
    let mut observations = Vec::new();
    watcher
        .drain_status(|value, _| observations.push(value))
        .unwrap();
    assert!(observations.last().unwrap().needs_you());
    watcher.cancel = ESCAPE_INTERRUPT;
    watcher
        .drain_status(|value, _| observations.push(value))
        .unwrap();
    assert!(observations.last().unwrap().needs_you());
    assert_eq!(observations.last().unwrap().detail, None);

    let result = serde_json::json!({"type":"user","sessionId":"main","promptId":"turn","isSidechain":false,"toolDenialKind":"user-rejected","message":{"content":[{"type":"tool_result","tool_use_id":"question","is_error":true}]}}).to_string();
    write!(transcript_file, "{}", &result[..20]).unwrap();

    watcher
        .drain_status(|value, _| observations.push(value))
        .unwrap();
    assert!(observations.last().unwrap().needs_you());
    writeln!(transcript_file, "{}", &result[20..]).unwrap();
    watcher
        .drain_status(|value, _| observations.push(value))
        .unwrap();
    assert!(!observations.last().unwrap().needs_you());
    assert_eq!(observations.last().unwrap().activity, Activity::Ready);
    writeln!(
        transcript_file,
        "{}",
        serde_json::json!({"type":"system","subtype":"turn_duration","sessionId":"main"})
    )
    .unwrap();
    watcher
        .drain_status(|value, _| observations.push(value))
        .unwrap();
    assert_eq!(observations.last().unwrap().activity, Activity::Ready);
    assert_eq!(
        observations.last().unwrap().outcome,
        Some(TurnOutcome::Interrupted)
    );
    assert!(!watcher.observation.model.cancelled_tool_result());
    for interrupt in [ESCAPE_INTERRUPT, CTRL_C_INTERRUPT] {
        watcher.cancel = interrupt;
        watcher
            .drain_status(|value, _| observations.push(value))
            .unwrap();
        assert_eq!(observations.last().unwrap().activity, Activity::Ready);
        assert_eq!(
            observations.last().unwrap().outcome,
            Some(TurnOutcome::Interrupted)
        );
    }
    watcher
        .hooks
        .admit(report("Notification", Some("permission_prompt"), "current"))
        .unwrap();

    watcher
        .drain_status(|value, _| observations.push(value))
        .unwrap();
    assert!(!observations.last().unwrap().needs_you());
    assert!(!observations
        .iter()
        .any(|value| value.outcome == Some(TurnOutcome::Completed)));
}

#[test]
fn rejected_bash_tool_without_turn_duration_settles_and_preserves_other_waits() {
    for other_wait in [false, true] {
        let mut model = ClaudeObservation::default();
        observe(
            &mut model,
            "SessionStart",
            serde_json::json!({"session_id":"main"}),
        );
        observe(
            &mut model,
            "UserPromptSubmit",
            serde_json::json!({"prompt_id":"turn"}),
        );
        observe(
            &mut model,
            "PreToolUse",
            serde_json::json!({"tool_name":"Bash","tool_use_id":"wait","tool_input":{"command":"sleep 30"}}),
        );
        if other_wait {
            observe(
                &mut model,
                "PreToolUse",
                serde_json::json!({"tool_name":"AskUserQuestion","tool_use_id":"question"}),
            );
        }
        let value = model
            .observe_transcript(&serde_json::json!({
                "type":"user","sessionId":"main","promptId":"turn","isSidechain":false,
                "toolDenialKind":"user-rejected",
                "message":{"content":[{"type":"tool_result","tool_use_id":"wait","is_error":true}]},
            }))
            .unwrap();
        assert_eq!(
            value.activity,
            if other_wait {
                Activity::Unavailable
            } else {
                Activity::Ready
            }
        );
        assert_eq!(value.outcome, Some(TurnOutcome::Interrupted));
        assert_eq!(value.needs_you(), other_wait);
        assert_eq!(value.detail, None);
        if other_wait {
            assert_eq!(value.interactions[0].owners, ["question"]);
        }
        observe(
            &mut model,
            "UserPromptSubmit",
            serde_json::json!({"prompt_id":"recovery"}),
        );
        assert_eq!(model.value.activity, Activity::Working);
        assert_eq!(model.value.outcome, None);
        let recovered = observe(
            &mut model,
            "Stop",
            serde_json::json!({"prompt_id":"recovery"}),
        )
        .unwrap();
        assert_eq!(recovered.activity, Activity::Ready);
        assert_eq!(recovered.outcome, Some(TurnOutcome::Completed));
        assert!(!recovered.needs_you());
    }
}

#[test]
fn answered_question_and_unreadable_transcript_preserve_hook_delivery() {
    let mut model = ClaudeObservation::default();
    observe(
        &mut model,
        "SessionStart",
        serde_json::json!({"session_id":"main"}),
    );
    observe(
        &mut model,
        "PreToolUse",
        serde_json::json!({"tool_name":"AskUserQuestion","tool_use_id":"question"}),
    );
    let answered = model.observe_transcript(&serde_json::json!({"type":"user","sessionId":"main","message":{"content":[{"type":"tool_result","tool_use_id":"question"}]}})).unwrap();
    assert!(!answered.needs_you());
    assert_eq!(answered.outcome, None);
    assert_eq!(answered.activity, Activity::Working);

    let root = tempfile::tempdir().unwrap();

    let mut watcher = TestWatcher::new("current".into());
    watcher.observation = model;
    watcher.observation.transcript_path = Some(root.path().join("missing.jsonl"));
    observe(
        &mut watcher.observation,
        "PreToolUse",
        serde_json::json!({"tool_name":"AskUserQuestion","tool_use_id":"next"}),
    );
    watcher.drain_status(|_, _| {}).unwrap();
    assert!(watcher.observation.value.needs_you());

    watcher.hooks.admit(serde_json::json!({"generation":"current","hook_event_name":"PostToolUse","tool_use_id":"next"})).unwrap();

    watcher.drain_status(|_, _| {}).unwrap();
    assert!(!watcher.observation.value.needs_you());
    assert_eq!(watcher.observation.value.source, ObservationSource::Hook);
}

#[test]
fn ordinary_tools_do_not_wait_and_permission_requests_are_immediate() {
    let mut model = ClaudeObservation::default();
    observe(
        &mut model,
        "UserPromptSubmit",
        serde_json::json!({"prompt_id":"turn"}),
    );
    observe(
        &mut model,
        "PreToolUse",
        serde_json::json!({"tool_use_id":"auto", "tool_name":"Bash"}),
    );
    assert!(!model.value.needs_you());
    observe(
        &mut model,
        "PostToolUse",
        serde_json::json!({"tool_use_id":"auto"}),
    );
    assert!(!model.value.needs_you());
    observe(
        &mut model,
        "PreToolUse",
        serde_json::json!({"tool_use_id":"human", "tool_name":"Bash"}),
    );
    observe(
        &mut model,
        "PermissionRequest",
        serde_json::json!({"tool_name":"Bash"}),
    );
    assert_eq!(model.value.interactions[0].reason, WaitReason::Approval);
    assert_eq!(model.value.interactions[0].owners, ["human"]);
    let wait_id = model.value.interactions[0].id.clone();
    observe(
        &mut model,
        "Notification",
        serde_json::json!({"notification_type":"permission_prompt"}),
    );
    assert_eq!(model.value.interactions.len(), 1);
    assert_eq!(model.value.interactions[0].id, wait_id);
    assert_eq!(model.value.interactions[0].owners, ["human"]);
    observe(
        &mut model,
        "PostToolUse",
        serde_json::json!({"tool_use_id":"unrelated"}),
    );
    assert!(model.value.needs_you());
    observe(
        &mut model,
        "PermissionDenied",
        serde_json::json!({"tool_use_id":"human"}),
    );
    assert!(!model.value.needs_you());
    assert_eq!(model.value.activity, Activity::Working);
}

#[test]
fn tool_and_compaction_detail_follow_owned_work() {
    let mut model = ClaudeObservation::default();
    observe(
        &mut model,
        "SessionStart",
        serde_json::json!({"session_id":"main"}),
    );
    let prompt = observe(
        &mut model,
        "UserPromptSubmit",
        serde_json::json!({"prompt_id":"turn"}),
    )
    .unwrap();
    assert_eq!(prompt.detail, None);

    for id in ["one", "two"] {
        let using = observe(
            &mut model,
            "PreToolUse",
            serde_json::json!({"tool_use_id":id,"tool_name":"Bash"}),
        )
        .unwrap();
        assert_eq!(using.detail, Some(WorkDetail::UsingTools));
    }
    let compacting = observe(&mut model, "PreCompact", serde_json::json!({})).unwrap();
    assert_eq!(compacting.detail, Some(WorkDetail::CompactingContext));
    let restored = observe(&mut model, "PostCompact", serde_json::json!({})).unwrap();
    assert_eq!(restored.detail, Some(WorkDetail::UsingTools));

    let one_left = observe(
        &mut model,
        "PostToolUse",
        serde_json::json!({"tool_use_id":"one"}),
    )
    .unwrap();
    assert_eq!(one_left.detail, Some(WorkDetail::UsingTools));
    let transcript = serde_json::json!({
        "type":"user",
        "sessionId":"main",
        "promptId":"turn",
        "message":{"content":[{"type":"tool_result","tool_use_id":"two"}]},
    });
    assert_eq!(model.observe_transcript(&transcript).unwrap().detail, None);

    observe(
        &mut model,
        "PreToolUse",
        serde_json::json!({"tool_use_id":"failed","tool_name":"Bash"}),
    );
    let failed = observe(&mut model, "StopFailure", serde_json::json!({})).unwrap();
    assert_eq!(failed.outcome, Some(TurnOutcome::Failed));
    assert_eq!(failed.detail, None);

    assert_eq!(
        observe(
            &mut model,
            "UserPromptSubmit",
            serde_json::json!({"prompt_id":"next"}),
        )
        .unwrap()
        .detail,
        None
    );
    observe(
        &mut model,
        "PreToolUse",
        serde_json::json!({"tool_use_id":"cancelled","tool_name":"Bash"}),
    );
    let interrupted = observe(
        &mut model,
        "PostToolUseFailure",
        serde_json::json!({"tool_use_id":"cancelled","is_interrupt":true}),
    )
    .unwrap();
    assert_eq!(interrupted.outcome, Some(TurnOutcome::Interrupted));
    assert_eq!(interrupted.detail, None);

    observe(
        &mut model,
        "UserPromptSubmit",
        serde_json::json!({"prompt_id":"third"}),
    );
    observe(
        &mut model,
        "PreToolUse",
        serde_json::json!({"tool_use_id":"stopped","tool_name":"Bash"}),
    );
    let stopped = observe(&mut model, "Stop", serde_json::json!({})).unwrap();
    assert_eq!(stopped.outcome, Some(TurnOutcome::Completed));
    assert_eq!(stopped.detail, None);
}

#[test]
fn compact_session_start_restores_manual_idle_and_in_turn_work() {
    let mut model = ClaudeObservation::default();
    observe(
        &mut model,
        "SessionStart",
        serde_json::json!({"session_id":"main","source":"startup"}),
    );
    observe(
        &mut model,
        "UserPromptSubmit",
        serde_json::json!({"prompt_id":"first"}),
    );
    observe(&mut model, "Stop", serde_json::json!({}));

    assert_eq!(
        observe(&mut model, "PreCompact", serde_json::json!({}))
            .unwrap()
            .detail,
        Some(WorkDetail::CompactingContext)
    );
    assert!(observe(
        &mut model,
        "SessionStart",
        serde_json::json!({"session_id":"main","source":"compact"}),
    )
    .is_none());
    assert_eq!(model.value.detail, Some(WorkDetail::CompactingContext));
    let manual_done = observe(&mut model, "PostCompact", serde_json::json!({})).unwrap();
    assert_eq!(manual_done.activity, Activity::Ready);
    assert_eq!(manual_done.outcome, Some(TurnOutcome::Completed));
    assert_eq!(manual_done.detail, None);

    observe(
        &mut model,
        "UserPromptSubmit",
        serde_json::json!({"prompt_id":"second"}),
    );
    observe(
        &mut model,
        "PreToolUse",
        serde_json::json!({"tool_use_id":"tool","tool_name":"Bash"}),
    );
    observe(&mut model, "PreCompact", serde_json::json!({}));
    assert!(observe(
        &mut model,
        "SessionStart",
        serde_json::json!({"session_id":"main","source":"compact"}),
    )
    .is_none());
    let automatic_done = observe(&mut model, "PostCompact", serde_json::json!({})).unwrap();
    assert_eq!(automatic_done.activity, Activity::Working);
    assert_eq!(automatic_done.outcome, None);
    assert_eq!(automatic_done.detail, Some(WorkDetail::UsingTools));
}

#[test]
fn compact_session_start_never_resets_in_either_order() {
    for compact_first in [true, false] {
        for in_turn in [false, true] {
            let case = format!("compact_first={compact_first} in_turn={in_turn}");
            let mut model = ClaudeObservation::default();
            observe(
                &mut model,
                "SessionStart",
                serde_json::json!({"session_id":"main","source":"startup"}),
            );
            observe(
                &mut model,
                "UserPromptSubmit",
                serde_json::json!({"prompt_id":"turn"}),
            );
            if !in_turn {
                observe(&mut model, "Stop", serde_json::json!({"prompt_id":"turn"}));
            }
            // A `/compact` gets a prompt id of its own; an automatic one keeps the turn's.
            let prompt = if in_turn { "turn" } else { "compact" };
            let compacting = observe(
                    &mut model,
                    "PreCompact",
                    serde_json::json!({"prompt_id":prompt,"trigger": if in_turn { "auto" } else { "manual" }}),
                )
                .unwrap();
            assert_eq!(compacting.activity, Activity::Working, "{case}");
            assert_eq!(
                compacting.detail,
                Some(WorkDetail::CompactingContext),
                "{case}"
            );
            let compact = |model: &mut ClaudeObservation| {
                assert!(
                        observe(
                            model,
                            "SessionStart",
                            serde_json::json!({"session_id":"main","source":"compact","prompt_id":prompt,"transcript_path":"/compacted.jsonl"}),
                        )
                        .is_none(),
                        "{case}"
                    );
            };
            if compact_first {
                compact(&mut model);
            }
            observe(
                &mut model,
                "PostCompact",
                serde_json::json!({"prompt_id":prompt}),
            );
            if !compact_first {
                compact(&mut model);
            }
            assert_eq!(
                model.transcript_path.as_deref(),
                Some(std::path::Path::new("/compacted.jsonl")),
                "{case}"
            );
            if in_turn {
                assert_eq!(model.value.activity, Activity::Working, "{case}");
                assert_eq!(model.value.outcome, None, "{case}");
                assert_eq!(model.value.detail, None, "{case}");
                observe(&mut model, "Stop", serde_json::json!({}));
            }
            assert_eq!(model.value.activity, Activity::Ready, "{case}");
            assert_eq!(model.value.outcome, Some(TurnOutcome::Completed), "{case}");
            assert_eq!(model.value.detail, None, "{case}");
        }
    }
}

#[test]
fn a_new_conversation_still_resets_after_a_compaction() {
    for source in ["startup", "resume", "clear"] {
        let mut model = ClaudeObservation::default();
        observe(
            &mut model,
            "SessionStart",
            serde_json::json!({"session_id":"main","source":"startup"}),
        );
        observe(
            &mut model,
            "UserPromptSubmit",
            serde_json::json!({"prompt_id":"turn"}),
        );
        observe(&mut model, "Stop", serde_json::json!({}));
        observe(&mut model, "PreCompact", serde_json::json!({}));
        let started = observe(
            &mut model,
            "SessionStart",
            serde_json::json!({"session_id":"next","source":source}),
        )
        .unwrap();
        assert_eq!(started.activity, Activity::Idle, "{source}");
        assert_eq!(started.outcome, None, "{source}");
        assert_eq!(started.detail, None, "{source}");
        assert_eq!(model.session_id.as_deref(), Some("next"), "{source}");
    }
}

#[test]
fn a_compaction_without_an_end_inside_a_turn_settles_on_stop() {
    let mut model = ClaudeObservation::default();
    observe(
        &mut model,
        "SessionStart",
        serde_json::json!({"session_id":"main","source":"startup"}),
    );
    observe(
        &mut model,
        "UserPromptSubmit",
        serde_json::json!({"prompt_id":"turn"}),
    );
    let compacting = observe(
        &mut model,
        "PreCompact",
        serde_json::json!({"trigger":"auto"}),
    )
    .unwrap();
    assert_eq!(compacting.detail, Some(WorkDetail::CompactingContext));
    let stopped = observe(&mut model, "Stop", serde_json::json!({})).unwrap();
    assert_eq!(stopped.activity, Activity::Ready);
    assert_eq!(stopped.outcome, Some(TurnOutcome::Completed));
    assert_eq!(stopped.detail, None);
    assert!(model.compaction_started.is_none());
}

const START: i64 = 1_767_225_600_000;

fn at<T>(millis: i64, run: impl FnOnce() -> T) -> T {
    crate::session::clock::at(std::time::Instant::now(), millis, run)
}

fn drain_at(watcher: &mut TestWatcher, millis: i64) -> AgentObservation {
    at(millis, || watcher.drain_status(|_, _| {}).unwrap());
    watcher.observation.value.published()
}

/// Admits a hook from the `main` conversation as if Claude fired it at `millis`.
fn admit_at(watcher: &TestWatcher, millis: i64, event: &str, mut fields: serde_json::Value) {
    fields["session_id"] = "main".into();
    at(millis, || watcher.hooks.admit_event(event, fields).unwrap());
}

fn record_at(millis: i64, subtype: &str) -> serde_json::Value {
    serde_json::json!({"type":"system","subtype":subtype,"sessionId":"main",
            "timestamp":chrono::DateTime::from_timestamp_millis(millis).unwrap().to_rfc3339()})
}

fn failed_compact(millis: i64) -> serde_json::Value {
    let mut record = record_at(millis, "local_command");
    record["commandRun"] = serde_json::json!({"command":"compact","args":""});
    record["content"] =
        "<local-command-stdout>Not enough messages to compact.</local-command-stdout>".into();
    record
}

fn boundary(millis: i64) -> serde_json::Value {
    let mut record = record_at(millis, "compact_boundary");
    record["compactMetadata"] = serde_json::json!({"trigger":"manual"});
    record
}

#[derive(Clone, Copy, Debug)]
enum Prior {
    Resumed,
    Completed,
    InTurn,
}

impl Prior {
    const ALL: [Self; 3] = [Self::Resumed, Self::Completed, Self::InTurn];

    /// A watcher left in this status a minute before `START`.
    fn watcher(self, transcript: Option<&std::path::Path>) -> TestWatcher {
        let mut watcher = TestWatcher::new("current".into());
        let before = START - 60_000;
        let mut start = serde_json::json!({"source": match self {
            Self::Resumed => "resume",
            _ => "startup",
        }});
        if let Some(path) = transcript {
            start["transcript_path"] = serde_json::json!(path);
        }
        admit_at(&watcher, before, "SessionStart", start);
        if !matches!(self, Self::Resumed) {
            admit_at(
                &watcher,
                before,
                "UserPromptSubmit",
                serde_json::json!({"prompt_id":"turn"}),
            );
        }
        match self {
            Self::Resumed => {}
            Self::Completed => admit_at(&watcher, before, "Stop", serde_json::json!({})),
            Self::InTurn => admit_at(
                &watcher,
                before,
                "PreToolUse",
                serde_json::json!({"tool_use_id":"tool","tool_name":"Bash"}),
            ),
        }
        drain_at(&mut watcher, before);
        watcher
    }

    fn restored(self) -> (Activity, Option<TurnOutcome>, Option<WorkDetail>) {
        match self {
            Self::Resumed => (Activity::Ready, None, None),
            Self::Completed => (Activity::Ready, Some(TurnOutcome::Completed), None),
            Self::InTurn => (Activity::Working, None, Some(WorkDetail::UsingTools)),
        }
    }

    /// Late `PostCompact` and `SessionStart(compact)` change nothing, and an unfinished
    /// turn still completes on its `Stop`.
    fn finish(self, watcher: &mut TestWatcher, millis: i64) {
        let restored = watcher.observation.value.published();
        let (activity, outcome, detail) = self.restored();
        assert_eq!(restored.activity, activity, "{self:?}");
        assert_eq!(restored.outcome, outcome, "{self:?}");
        assert_eq!(restored.detail, detail, "{self:?}");
        assert!(watcher.observation.parser.compaction_started.is_none());
        admit_at(watcher, millis, "PostCompact", serde_json::json!({}));
        admit_at(
            watcher,
            millis,
            "SessionStart",
            serde_json::json!({"source":"compact"}),
        );
        assert_eq!(drain_at(watcher, millis), restored, "{self:?}");
        assert_eq!(
            drain_at(watcher, millis + 2 * COMPACTION_TIMEOUT_MS),
            restored,
            "{self:?}"
        );
        if matches!(self, Self::InTurn) {
            admit_at(watcher, millis + 1, "Stop", serde_json::json!({}));
            let stopped = drain_at(watcher, millis + 1);
            assert_eq!(stopped.activity, Activity::Ready);
            assert_eq!(stopped.outcome, Some(TurnOutcome::Completed));
            assert_eq!(stopped.detail, None);
        }
    }
}

#[test]
fn a_compaction_without_an_end_restores_its_prior_status_after_the_timeout() {
    for prior in Prior::ALL {
        let mut watcher = prior.watcher(None);
        admit_at(
            &watcher,
            START,
            "PreCompact",
            serde_json::json!({"trigger":"auto"}),
        );
        let compacting = drain_at(&mut watcher, START + 10);
        assert_eq!(compacting.activity, Activity::Working, "{prior:?}");
        assert_eq!(compacting.detail, Some(WorkDetail::CompactingContext));
        assert_eq!(
            drain_at(&mut watcher, START + COMPACTION_TIMEOUT_MS - 1),
            compacting,
            "{prior:?}"
        );
        drain_at(&mut watcher, START + COMPACTION_TIMEOUT_MS);
        prior.finish(&mut watcher, START + COMPACTION_TIMEOUT_MS + 1_000);
    }
}

#[test]
fn a_compaction_end_in_the_transcript_restores_the_prior_status() {
    let ends: [fn(i64) -> serde_json::Value; 2] = [failed_compact, boundary];
    for prior in Prior::ALL {
        for end in ends {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("transcript.jsonl");
            let mut transcript = File::create(&path).unwrap();
            let mut watcher = prior.watcher(Some(&path));
            // An earlier compaction's record, replayed when the tail opens, and another
            // command's output both leave this compaction running.
            writeln!(transcript, "{}", end(START - 30_000)).unwrap();
            let mut other_command = failed_compact(START + 10);
            other_command["commandRun"]["command"] = "cost".into();
            writeln!(transcript, "{other_command}").unwrap();
            admit_at(
                &watcher,
                START,
                "PreCompact",
                serde_json::json!({"trigger":"manual"}),
            );
            let compacting = drain_at(&mut watcher, START + 20);
            assert_eq!(compacting.activity, Activity::Working, "{prior:?}");
            assert_eq!(compacting.detail, Some(WorkDetail::CompactingContext));

            writeln!(transcript, "{}", end(START + 30)).unwrap();
            drain_at(&mut watcher, START + 40);
            prior.finish(&mut watcher, START + 50);
        }
    }
}

#[test]
fn an_earlier_compactions_unread_boundary_does_not_end_the_next_one() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("transcript.jsonl");
    let mut transcript = File::create(&path).unwrap();
    let mut watcher = Prior::Completed.watcher(Some(&path));
    admit_at(
        &watcher,
        START,
        "PreCompact",
        serde_json::json!({"trigger":"manual"}),
    );
    admit_at(
        &watcher,
        START + 3_000,
        "SessionStart",
        serde_json::json!({"source":"compact"}),
    );
    admit_at(
        &watcher,
        START + 3_100,
        "PostCompact",
        serde_json::json!({"trigger":"manual"}),
    );
    let compacted = drain_at(&mut watcher, START + 3_200);
    assert_eq!(compacted.activity, Activity::Ready);
    assert_eq!(compacted.outcome, Some(TurnOutcome::Completed));
    // Claude writes the boundary after PostCompact, so nothing reads it yet.
    writeln!(transcript, "{}", boundary(START + 3_150)).unwrap();

    admit_at(
        &watcher,
        START + 8_000,
        "PreCompact",
        serde_json::json!({"trigger":"manual"}),
    );
    let compacting = drain_at(&mut watcher, START + 8_050);
    assert_eq!(compacting.activity, Activity::Working);
    assert_eq!(compacting.detail, Some(WorkDetail::CompactingContext));

    writeln!(transcript, "{}", boundary(START + 9_000)).unwrap();
    drain_at(&mut watcher, START + 9_050);
    Prior::Completed.finish(&mut watcher, START + 9_100);
}

#[test]
fn parallel_same_tool_notifications_merge_and_hold_all_possible_owners() {
    let mut model = ClaudeObservation::default();
    for id in ["a", "b"] {
        observe(
            &mut model,
            "PreToolUse",
            serde_json::json!({"tool_use_id":id,"tool_name":"Bash"}),
        );
    }
    observe(
        &mut model,
        "PermissionRequest",
        serde_json::json!({"tool_name":"Bash"}),
    );
    assert!(model.value.needs_you());
    observe(
        &mut model,
        "Notification",
        serde_json::json!({"notification_type":"permission_prompt"}),
    );
    let original_id = model.value.interactions[0].id.clone();
    observe(
        &mut model,
        "PreToolUse",
        serde_json::json!({"tool_use_id":"c","tool_name":"Bash"}),
    );
    observe(
        &mut model,
        "PermissionRequest",
        serde_json::json!({"tool_name":"Bash"}),
    );
    observe(
        &mut model,
        "Notification",
        serde_json::json!({"notification_type":"permission_prompt"}),
    );
    assert_eq!(model.value.interactions.len(), 1);
    assert_eq!(model.value.interactions[0].id, original_id);
    assert_eq!(model.value.interactions[0].owners, ["a", "b", "c"]);
    for id in ["a", "b"] {
        observe(
            &mut model,
            "PostToolUse",
            serde_json::json!({"tool_use_id":id}),
        );
        assert!(model.value.needs_you());
    }
    observe(
        &mut model,
        "PostToolUse",
        serde_json::json!({"tool_use_id":"c"}),
    );
    assert!(!model.value.needs_you());
}

#[test]
fn early_approval_matches_tool_input_and_keeps_question_reason() {
    let mut model = ClaudeObservation::default();
    for (id, command) in [
        ("quiet", "sleep 10"),
        ("approval", "curl https://example.com"),
    ] {
        observe(
            &mut model,
            "PreToolUse",
            serde_json::json!({
                "tool_use_id":id,"tool_name":"Bash","tool_input":{"command":command}
            }),
        );
    }
    let request =
        serde_json::json!({"tool_name":"Bash","tool_input":{"command":"curl https://example.com"}});
    let value = observe(&mut model, "PermissionRequest", request.clone()).unwrap();
    assert_eq!(value.source, ObservationSource::Hook);
    assert_eq!(value.interactions[0].reason, WaitReason::Approval);
    assert_eq!(value.interactions[0].owners, ["approval"]);
    observe(&mut model, "PermissionRequest", request);
    assert_eq!(model.value.interactions.len(), 1);
    observe(
        &mut model,
        "PostToolUse",
        serde_json::json!({"tool_use_id":"quiet"}),
    );
    assert_eq!(model.value.interactions[0].owners, ["approval"]);
    observe(
        &mut model,
        "PostToolUse",
        serde_json::json!({"tool_use_id":"approval"}),
    );
    assert!(!model.value.needs_you());

    observe(
        &mut model,
        "PreToolUse",
        serde_json::json!({"tool_use_id":"question","tool_name":"AskUserQuestion"}),
    );
    let question_id = model.value.interactions[0].id.clone();
    observe(
        &mut model,
        "PermissionRequest",
        serde_json::json!({"tool_name":"AskUserQuestion"}),
    );
    observe(
        &mut model,
        "Notification",
        serde_json::json!({"notification_type":"permission_prompt"}),
    );
    assert_eq!(model.value.interactions.len(), 1);
    assert_eq!(model.value.interactions[0].id, question_id);
    assert_eq!(model.value.interactions[0].reason, WaitReason::Answer);

    observe(
        &mut model,
        "PreToolUse",
        serde_json::json!({"tool_use_id":"command","tool_name":"Bash"}),
    );
    observe(
        &mut model,
        "PermissionRequest",
        serde_json::json!({"tool_name":"Bash"}),
    );
    let interactions = model.value.interactions.clone();
    observe(
        &mut model,
        "Notification",
        serde_json::json!({"notification_type":"permission_prompt"}),
    );
    assert_eq!(model.value.interactions, interactions);
    observe(
        &mut model,
        "PostToolUse",
        serde_json::json!({"tool_use_id":"command"}),
    );
    assert_eq!(model.value.interactions.len(), 1);
    assert_eq!(model.value.interactions[0].id, question_id);
    assert_eq!(model.value.interactions[0].reason, WaitReason::Answer);
}

#[test]
fn early_approval_resolves_without_a_notification_and_ignores_finished_turns() {
    for event in ["PostToolUse", "PostToolUseFailure", "PermissionDenied"] {
        let mut model = ClaudeObservation::default();
        observe(
            &mut model,
            "PreToolUse",
            serde_json::json!({"tool_use_id":"command","tool_name":"Bash"}),
        );
        observe(
            &mut model,
            "PermissionRequest",
            serde_json::json!({"tool_name":"Bash"}),
        );
        assert!(model.value.needs_you());
        observe(
            &mut model,
            event,
            serde_json::json!({"tool_use_id":"command"}),
        );
        assert!(!model.value.needs_you());
        assert_eq!(model.value.activity, Activity::Working);
        observe(&mut model, "Stop", serde_json::json!({}));
        assert!(observe(
            &mut model,
            "PermissionRequest",
            serde_json::json!({"tool_name":"Bash"})
        )
        .is_none());
        assert_eq!(model.value.outcome, Some(TurnOutcome::Completed));
    }
}

#[test]
fn approval_hook_is_immediate_and_escape_uses_the_question_resolution_path() {
    let root = tempfile::tempdir().unwrap();

    let transcript = root.path().join("claude.jsonl");
    let mut file = File::create(&transcript).unwrap();
    let mut watcher = TestWatcher::new("current".into());
    for (event, fields) in [
        (
            "SessionStart",
            serde_json::json!({"session_id":"main","transcript_path":transcript}),
        ),
        (
            "UserPromptSubmit",
            serde_json::json!({"session_id":"main","prompt_id":"turn"}),
        ),
        (
            "PreToolUse",
            serde_json::json!({"session_id":"main","prompt_id":"turn","tool_use_id":"command","tool_name":"Bash","tool_input":{"command":"curl https://example.com"}}),
        ),
        (
            "PermissionRequest",
            serde_json::json!({"session_id":"main","prompt_id":"turn","tool_name":"Bash","tool_input":{"command":"curl https://example.com"}}),
        ),
    ] {
        watcher.hooks.admit_event(event, fields).unwrap();
    }
    watcher.drain_status(|_, _| {}).unwrap();
    assert_eq!(
        watcher.observation.value.interactions[0].reason,
        WaitReason::Approval
    );
    assert_eq!(
        watcher.observation.value.interactions[0].owners,
        ["command"]
    );
    watcher.cancel = ESCAPE_INTERRUPT;
    watcher.drain_status(|_, _| {}).unwrap();
    assert!(watcher.observation.value.needs_you());

    writeln!(file, "{}", serde_json::json!({"type":"user","sessionId":"main","promptId":"turn","toolDenialKind":"user-rejected","message":{"content":[{"type":"tool_result","tool_use_id":"command"}]}})).unwrap();
    writeln!(file, "{}", serde_json::json!({"type":"system","subtype":"turn_duration","sessionId":"main","promptId":"turn"})).unwrap();
    watcher.drain_status(|_, _| {}).unwrap();
    assert!(!watcher.observation.value.needs_you());
    assert_eq!(watcher.observation.value.activity, Activity::Ready);
    assert_eq!(
        watcher.observation.value.outcome,
        Some(TurnOutcome::Interrupted)
    );
    watcher.cancel = ESCAPE_INTERRUPT;
    watcher.drain_status(|_, _| {}).unwrap();
    assert_eq!(watcher.observation.value.activity, Activity::Ready);
}

#[test]
fn invalid_report_is_rejected_without_losing_the_next_record_or_bridge() {
    let mut watcher = TestWatcher::new("current".into());
    assert!(watcher
        .hooks
        .admit(serde_json::json!({"hook_event_name":42}))
        .is_err());
    watcher
        .hooks
        .admit(report("UserPromptSubmit", None, "current"))
        .unwrap();
    watcher
        .hooks
        .admit(report("Stop", None, "current"))
        .unwrap();
    let mut observations = Vec::new();
    watcher
        .drain_status(|value, _| observations.push(value))
        .unwrap();
    assert_eq!(observations.len(), 2);
    assert_eq!(observations[0].activity, Activity::Working);
    assert_eq!(observations[1].activity, Activity::Ready);
    watcher
        .drain_status(|_, _| panic!("records must not replay"))
        .unwrap();
    watcher.hooks.retire();
    assert!(watcher.drain_status(|_, _| {}).is_err());
}

#[test]
fn question_plan_and_mcp_waits_keep_ownership() {
    for (tool, reason) in [
        ("AskUserQuestion", WaitReason::Answer),
        ("ExitPlanMode", WaitReason::Approval),
    ] {
        let mut model = ClaudeObservation::default();
        observe(
            &mut model,
            "PreToolUse",
            serde_json::json!({"tool_use_id":"human", "tool_name":tool}),
        );
        assert_eq!(model.value.interactions[0].reason, reason);
        let interaction_id = model.value.interactions[0].id.clone();
        observe(
            &mut model,
            "Notification",
            serde_json::json!({"notification_type":"permission_prompt"}),
        );
        assert_eq!(model.value.interactions[0].reason, reason);
        assert_eq!(model.value.interactions[0].id, interaction_id);
        assert_eq!(model.value.interactions.len(), 1);
        observe(
            &mut model,
            "Elicitation",
            serde_json::json!({"elicitation_id":"mcp-1","mcp_server_name":"server"}),
        );
        observe(
            &mut model,
            "Notification",
            serde_json::json!({"notification_type":"elicitation_dialog"}),
        );
        assert_eq!(model.value.interactions.len(), 2);
        observe(
            &mut model,
            "PostToolUseFailure",
            serde_json::json!({"tool_use_id":"human"}),
        );
        assert_eq!(model.value.interactions.len(), 1);
        observe(
            &mut model,
            "ElicitationResult",
            serde_json::json!({"elicitation_id":"mcp-2","mcp_server_name":"server"}),
        );
        assert!(model.value.needs_you());
        observe(
            &mut model,
            "ElicitationResult",
            serde_json::json!({"elicitation_id":"mcp-1","mcp_server_name":"server"}),
        );
        assert!(!model.value.needs_you());
    }
}

#[test]
fn late_form_notifications_and_results_do_not_restore_resolved_waits() {
    for action in ["accept", "decline", "cancel"] {
        let mut model = ClaudeObservation::default();
        observe(&mut model, "UserPromptSubmit", serde_json::json!({}));
        observe(
            &mut model,
            "Elicitation",
            serde_json::json!({"mcp_server_name":"server"}),
        );
        observe(
            &mut model,
            "Notification",
            serde_json::json!({"notification_type":"elicitation_dialog"}),
        );
        assert!(model.value.needs_you());
        observe(
            &mut model,
            "ElicitationResult",
            serde_json::json!({"mcp_server_name":"server", "action":action}),
        );
        assert!(!model.value.needs_you());
        assert!(observe(
            &mut model,
            "Notification",
            serde_json::json!({"notification_type":"elicitation_dialog"}),
        )
        .is_none());
        assert!(!model.value.needs_you());
        observe(&mut model, "Stop", serde_json::json!({}));
        assert!(observe(
            &mut model,
            "ElicitationResult",
            serde_json::json!({"mcp_server_name":"server", "action":action}),
        )
        .is_none());
        assert_eq!(model.value.activity, Activity::Ready);
        assert_eq!(model.value.outcome, Some(TurnOutcome::Completed));
        assert!(observe(
            &mut model,
            "Notification",
            serde_json::json!({"notification_type":"elicitation_dialog"}),
        )
        .is_none());
    }
}

#[test]
fn tool_results_preserve_finished_turns_until_new_work_starts() {
    for (event, outcome) in [
        ("Stop", TurnOutcome::Completed),
        ("StopFailure", TurnOutcome::Failed),
    ] {
        let mut model = ClaudeObservation::default();
        observe(&mut model, "UserPromptSubmit", serde_json::json!({}));
        observe(&mut model, event, serde_json::json!({}));
        for event in ["PostToolUse", "PostToolUseFailure", "PermissionDenied"] {
            observe(&mut model, event, serde_json::json!({"tool_use_id":"late"}));
            assert_eq!(model.value.activity, Activity::Ready);
            assert_eq!(model.value.outcome, Some(outcome));
        }
        observe(&mut model, "PreToolUse", serde_json::json!({}));
        assert_eq!(model.value.activity, Activity::Working);
        assert_eq!(model.value.outcome, None);
    }
}

#[test]
fn ambiguous_idless_forms_remain_until_owning_turn_ends() {
    let mut model = ClaudeObservation::default();
    observe(&mut model, "UserPromptSubmit", serde_json::json!({}));
    for _ in 0..2 {
        observe(
            &mut model,
            "Elicitation",
            serde_json::json!({"mcp_server_name":"server"}),
        );
    }
    observe(
        &mut model,
        "Notification",
        serde_json::json!({"notification_type":"elicitation_dialog"}),
    );
    assert_eq!(model.value.interactions[0].owners.len(), 2);
    observe(
        &mut model,
        "ElicitationResult",
        serde_json::json!({"mcp_server_name":"server"}),
    );
    assert!(model.value.needs_you());
    observe(&mut model, "Stop", serde_json::json!({}));
    assert!(!model.value.needs_you());
}

#[test]
fn interrupted_tool_failure_does_not_complete_or_clear_another_wait() {
    let mut model = ClaudeObservation::default();
    observe(
        &mut model,
        "PreToolUse",
        serde_json::json!({"tool_use_id":"question", "tool_name":"AskUserQuestion"}),
    );
    observe(
        &mut model,
        "Notification",
        serde_json::json!({"notification_type":"permission_prompt"}),
    );
    observe(
        &mut model,
        "PostToolUseFailure",
        serde_json::json!({"tool_use_id":"other", "is_interrupt":true}),
    );
    assert_eq!(model.value.outcome, Some(TurnOutcome::Interrupted));
    assert!(model.value.needs_you());
    observe(
        &mut model,
        "PostToolUse",
        serde_json::json!({"tool_use_id":"question"}),
    );
    assert!(!model.value.needs_you());
    assert_eq!(model.value.activity, Activity::Unavailable);
    assert_eq!(model.value.outcome, Some(TurnOutcome::Interrupted));
    observe(&mut model, "Stop", serde_json::json!({}));
    assert_eq!(model.value.outcome, Some(TurnOutcome::Interrupted));
}

#[test]
fn old_turns_subagents_and_secondary_idle_cannot_finish_work_or_waits() {
    let mut model = ClaudeObservation::default();
    observe(
        &mut model,
        "SessionStart",
        serde_json::json!({"session_id":"main"}),
    );
    observe(
        &mut model,
        "UserPromptSubmit",
        serde_json::json!({"session_id":"main", "prompt_id":"new"}),
    );
    for fields in [
        serde_json::json!({"prompt_id":"old"}),
        serde_json::json!({"agent_id":"child"}),
        serde_json::json!({"session_id":"child"}),
    ] {
        assert!(observe(&mut model, "Stop", fields).is_none());
        assert_eq!(model.value.activity, Activity::Working);
    }
    observe(
        &mut model,
        "Notification",
        serde_json::json!({"notification_type":"permission_prompt"}),
    );
    observe(
        &mut model,
        "Notification",
        serde_json::json!({"notification_type":"idle_prompt"}),
    );
    assert!(model.value.needs_you());
    observe(&mut model, "Stop", serde_json::json!({"prompt_id":"new"}));
    assert!(!model.value.needs_you());
    assert_eq!(model.value.outcome, Some(TurnOutcome::Completed));
    observe(
        &mut model,
        "PreToolUse",
        serde_json::json!({"prompt_id":"new"}),
    );
    assert_eq!(model.value.activity, Activity::Working);
    assert_eq!(model.value.outcome, None);
}

#[test]
fn interrupt_preserves_dialog_until_correlated_resolution_and_never_completes() {
    let mut watcher = TestWatcher::new("current".into());

    watcher.hooks.admit_json(r#"{"generation":"current","hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_use_id":"question"}"#).unwrap();
    watcher
        .hooks
        .admit(report("Notification", Some("permission_prompt"), "current"))
        .unwrap();
    watcher.cancel = ESCAPE_INTERRUPT;
    let mut observations = Vec::new();
    watcher
        .drain_status(|value, _| observations.push(value))
        .unwrap();
    let interrupted = observations.last().unwrap();
    assert!(interrupted.needs_you());
    assert_eq!(interrupted.outcome, Some(TurnOutcome::Interrupted));
    assert_eq!(interrupted.activity, Activity::Unavailable);
    watcher
        .hooks
        .admit(report("Stop", None, "current"))
        .unwrap();

    watcher
        .drain_status(|value, _| observations.push(value))
        .unwrap();
    assert_eq!(
        observations.last().unwrap().outcome,
        Some(TurnOutcome::Interrupted)
    );
    assert!(!observations.last().unwrap().needs_you());
}

#[test]
fn maps_turn_boundaries_work_and_real_idle_notifications() {
    for (event, notification, expected) in [
        ("UserPromptSubmit", None, Some(SessionActivityState::Busy)),
        ("PreToolUse", None, Some(SessionActivityState::Busy)),
        ("PostToolUse", None, Some(SessionActivityState::Busy)),
        (
            "Notification",
            Some("idle_prompt"),
            Some(SessionActivityState::Idle),
        ),
        ("Notification", Some("permission_prompt"), None),
        ("Notification", Some("agent_completed"), None),
        ("Notification", None, None),
        ("Stop", None, Some(SessionActivityState::Idle)),
        ("StopFailure", None, Some(SessionActivityState::Idle)),
        ("SubagentStop", None, None),
        ("SessionStart", None, Some(SessionActivityState::Idle)),
        ("unknown", None, None),
    ] {
        assert_eq!(
            parse_transition(
                report(event, notification, "current")
                    .to_string()
                    .as_bytes(),
                "current"
            ),
            expected,
            "{event} {notification:?}"
        );
    }
    assert_eq!(parse_transition(b"not JSON\n", "current"), None);
    assert_eq!(
        parse_transition(
            report("Notification", Some("idle_prompt"), "old")
                .to_string()
                .as_bytes(),
            "current"
        ),
        None
    );
}

#[test]
fn watcher_waits_for_admission_and_skips_stale_or_invalid_records() {
    let mut watcher = TestWatcher::new("current".into());
    let pending = watcher
        .hooks
        .admit(report("UserPromptSubmit", None, "current"))
        .unwrap();
    let mut states = Vec::new();
    watcher.drain(|state, _| states.push(state)).unwrap();
    assert!(states.is_empty());
    drop(pending);
    assert!(watcher.hooks.admit_json("broken").is_err());
    assert!(watcher
        .hooks
        .admit(report("Notification", Some("idle_prompt"), "old"))
        .is_err());
    watcher
        .hooks
        .admit(report("Stop", None, "current"))
        .unwrap();
    watcher
        .hooks
        .admit(report("Notification", Some("idle_prompt"), "current"))
        .unwrap();
    watcher.drain(|state, _| states.push(state)).unwrap();
    assert_eq!(
        states,
        [
            SessionActivityState::Busy,
            SessionActivityState::Idle,
            SessionActivityState::Idle
        ]
    );
    watcher.drain(|state, _| states.push(state)).unwrap();
    assert_eq!(states.len(), 3);
}

#[test]
fn ordinary_interrupt_recovers_idle_without_a_transcript_or_completion_hook() {
    for kind in [ESCAPE_INTERRUPT, CTRL_C_INTERRUPT] {
        let mut watcher = TestWatcher::new("current".into());

        watcher.hooks.admit(serde_json::json!({"generation":"current","hook_event_name":"UserPromptSubmit","prompt_id":"turn"})).unwrap();
        watcher.cancel = kind;
        let mut observations = Vec::new();
        watcher
            .drain_status(|value, _| observations.push(value))
            .unwrap();
        assert_eq!(observations.len(), 2);
        assert_eq!(observations[0].activity, Activity::Working);
        let interrupted = observations.last().unwrap();
        assert_eq!(interrupted.activity, Activity::Ready);
        assert_eq!(interrupted.outcome, Some(TurnOutcome::Interrupted));
        assert!(!interrupted.needs_you());

        watcher.cancel = kind;
        watcher
            .drain_status(|_, _| panic!("repeated interrupt must preserve Idle"))
            .unwrap();
        for event in ["PostToolUse", "PostToolUseFailure", "Notification", "Stop"] {
            watcher.hooks.admit(serde_json::json!({"generation":"current","hook_event_name":event,"prompt_id":"turn","tool_use_id":"late","is_interrupt":event == "PostToolUseFailure","notification_type":"permission_prompt"})).unwrap();

            watcher
                .drain_status(|value, _| {
                    assert_eq!(value.activity, Activity::Ready);
                    assert_eq!(value.outcome, Some(TurnOutcome::Interrupted));
                    assert!(!value.needs_you());
                })
                .unwrap();
        }
        watcher.hooks.admit(serde_json::json!({"generation":"current","hook_event_name":"UserPromptSubmit","prompt_id":"next"})).unwrap();

        watcher
            .drain_status(|value, _| observations.push(value))
            .unwrap();
        assert_eq!(observations.last().unwrap().activity, Activity::Working);
        assert_eq!(observations.last().unwrap().outcome, None);
    }
}

#[test]
fn interrupt_drains_buffered_tool_records_before_idle_and_allows_new_work() {
    for (kind, source) in [
        (CTRL_C_INTERRUPT, StatusSource::InputInterrupt),
        (ESCAPE_INTERRUPT, StatusSource::InputEscape),
        (
            CTRL_C_INTERRUPT | ESCAPE_INTERRUPT,
            StatusSource::InputInterrupt,
        ),
    ] {
        let mut watcher = TestWatcher::new("current".into());
        watcher.drain(|_, _| panic!("no records yet")).unwrap();

        watcher
            .hooks
            .admit(report("PreToolUse", None, "current"))
            .unwrap();

        watcher.cancel = kind;
        let mut transitions = Vec::new();
        watcher
            .drain(|state, source| transitions.push((state, source)))
            .unwrap();
        assert_eq!(
            transitions,
            [
                (SessionActivityState::Busy, StatusSource::Hook),
                (SessionActivityState::Idle, source)
            ]
        );
        watcher
            .hooks
            .admit(report("PreToolUse", None, "current"))
            .unwrap();

        watcher
            .drain(|state, source| transitions.push((state, source)))
            .unwrap();
        assert_eq!(
            transitions.last(),
            Some(&(SessionActivityState::Busy, StatusSource::Hook))
        );
        assert_eq!(transitions.len(), 3);
    }
}

#[test]
fn notification_uses_payload_type_even_when_matcher_is_bypassed() {
    let mut watcher = TestWatcher::new("current".into());
    for kind in [
        None,
        Some(""),
        Some("permission_prompt"),
        Some("agent_completed"),
        Some("idle_prompt"),
    ] {
        let mut payload = serde_json::json!({
            "hook_event_name": "Notification",
            "message": "Do not trust embedded \"notification_type\":\"idle_prompt\" text",
        });
        if let Some(kind) = kind {
            payload["notification_type"] = serde_json::json!(kind);
        }
        for text in [
            serde_json::to_string(&payload).unwrap(),
            serde_json::to_string_pretty(&payload).unwrap(),
        ] {
            watcher.hooks.admit_json(&text).unwrap();
            let mut observations = Vec::new();

            watcher
                .drain_status(|value, _| observations.push(value))
                .unwrap();
            assert_eq!(observations.len(), usize::from(kind == Some("idle_prompt")));
            if let Some(value) = observations.first() {
                assert_eq!(value.activity, Activity::Ready);
            }
        }
    }
}

#[test]
fn stop_followed_by_pre_tool_use_ends_busy() {
    let mut watcher = TestWatcher::new("current".into());
    for event in ["Stop", "PreToolUse"] {
        watcher
            .hooks
            .admit_event(event, serde_json::json!({}))
            .unwrap();
    }
    let mut states = Vec::new();
    watcher.drain(|state, _| states.push(state)).unwrap();
    assert_eq!(
        states,
        [SessionActivityState::Idle, SessionActivityState::Busy]
    );
}

#[test]
fn tool_hooks_drain_large_admitted_payloads() {
    let mut watcher = TestWatcher::new("current".into());
    let payload = serde_json::json!({"tool_response": "x".repeat(2 * 1024 * 1024)});
    for event in [
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "Stop",
        "StopFailure",
    ] {
        watcher.hooks.admit_event(event, payload.clone()).unwrap();
    }
    let mut observations = Vec::new();
    watcher
        .drain_status(|value, _| observations.push(value))
        .unwrap();
    assert_eq!(observations.len(), 5);
    watcher.hooks.retire();
    assert!(watcher.hooks.admit_event("PostToolUse", payload).is_err());
}

#[test]
fn admitted_hook_events_preserve_delivery_order() {
    let mut watcher = TestWatcher::new("current".into());
    for event in [
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "Notification",
        "Stop",
        "StopFailure",
    ] {
        watcher
            .hooks
            .admit_event(
                event,
                serde_json::json!({"notification_type":"idle_prompt"}),
            )
            .unwrap();
    }
    let mut states = Vec::new();
    watcher.drain(|state, _| states.push(state)).unwrap();
    assert_eq!(
        states,
        [
            SessionActivityState::Busy,
            SessionActivityState::Busy,
            SessionActivityState::Busy,
            SessionActivityState::Idle,
            SessionActivityState::Idle,
            SessionActivityState::Idle,
        ]
    );
}

#[test]
fn startup_clears_status_files_left_by_a_crash() {
    let root = tempfile::tempdir().unwrap();
    clear_leftovers(root.path()).unwrap();
    let path = root.path().join("session-status/stale.ndjson");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"stale report").unwrap();
    fs::write(path.with_extension("sh"), "stale reporter").unwrap();
    fs::write(path.with_extension("sh.tmp"), "stale reporter").unwrap();
    let unremovable = path.with_file_name("directory.sh");
    fs::create_dir(&unremovable).unwrap();
    clear_leftovers(root.path()).unwrap();
    assert!(unremovable.is_dir());
    assert!(!path.exists());
    assert!(!path.with_extension("sh").exists());
    assert!(!path.with_extension("sh.tmp").exists());
}

fn background_after(event: &str, fields: serde_json::Value) -> bool {
    let mut model = ClaudeObservation::default();
    observe(&mut model, "UserPromptSubmit", serde_json::json!({}));
    let ended = observe(&mut model, event, fields).unwrap();
    assert_eq!(ended.activity, Activity::Ready);
    assert_eq!(ended.background, model.value.background);
    model.value.background
}

#[test]
fn a_stop_reports_background_work_the_agent_will_wake_from() {
    let shell = serde_json::json!({"id":"task-001","type":"shell","status":"running",
            "description":"sleep","command":"sleep 90"});
    let cron = serde_json::json!({"id":"cron-001","schedule":"0 9 * * 1-5",
            "recurring":true,"prompt":"check the build"});
    for (fields, pending) in [
        (
            serde_json::json!({"background_tasks":[shell],"session_crons":[]}),
            true,
        ),
        (
            serde_json::json!({"background_tasks":[{"id":"t","type":"local_future","status":"running"}],
                    "session_crons":[]}),
            false,
        ),
        (
            serde_json::json!({"background_tasks":[],"session_crons":[cron]}),
            true,
        ),
        (
            serde_json::json!({"background_tasks":[],"session_crons":[{"id":"once","recurring":false}]}),
            true,
        ),
        (
            serde_json::json!({"background_tasks":[],"session_crons":[]}),
            false,
        ),
        (serde_json::json!({}), false),
    ] {
        assert_eq!(
            background_after("Stop", fields.clone()),
            pending,
            "{fields}"
        );
    }
    for kind in BACKGROUND_TASK_TYPES {
        assert!(background_after(
            "Stop",
            serde_json::json!({"background_tasks":[{"id":"t","type":kind}]})
        ));
    }
    assert!(!background_after(
        "StopFailure",
        serde_json::json!({"background_tasks":[shell],"session_crons":[cron]}),
    ));
}

#[test]
fn background_work_lasts_until_the_next_turn_startup_or_exit() {
    let pending = serde_json::json!({"background_tasks":[{"id":"t","type":"shell"}]});
    let wait = |model: &mut ClaudeObservation| {
        observe(model, "UserPromptSubmit", serde_json::json!({}));
        observe(model, "Stop", pending.clone());
        assert!(model.value.background);
    };

    let mut model = ClaudeObservation::default();
    wait(&mut model);
    let woken = observe(&mut model, "UserPromptSubmit", serde_json::json!({})).unwrap();
    assert_eq!(woken.activity, Activity::Working);
    assert!(!woken.background);
    let ended = observe(&mut model, "Stop", serde_json::json!({})).unwrap();
    assert!(!ended.background);

    wait(&mut model);
    assert!(
        !observe(&mut model, "SessionStart", serde_json::json!({}))
            .unwrap()
            .background
    );

    let now = crate::session::clock::timestamp_millis();
    for event in [AgentEvent::SessionEnded, AgentEvent::ConversationRecovered] {
        wait(&mut model);
        model.model.reduce(
            AgentEvent::Batch {
                runtime: Runtime::ClaudeCode,
                events: vec![event],
            },
            now,
        );
        assert!(!model.model.value.background);
    }

    // An interrupted tool leaves the turn Interrupted; its Stop still reports the work.
    let mut model = ClaudeObservation::default();
    observe(&mut model, "UserPromptSubmit", serde_json::json!({}));
    observe(
        &mut model,
        "PostToolUseFailure",
        serde_json::json!({"tool_use_id":"tool","is_interrupt":true}),
    );
    observe(&mut model, "Stop", pending.clone());
    assert_eq!(model.value.outcome, Some(TurnOutcome::Interrupted));
    assert!(model.value.background);
}
