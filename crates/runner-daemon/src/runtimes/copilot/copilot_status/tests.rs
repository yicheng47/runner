use std::fs::{self, OpenOptions};
use std::io::Write;

use serde_json::json;

use super::*;

struct TestWatcher {
    inner: CopilotStatusWatcher,
    hooks: crate::session::hook_queue::TestHookRoute,
    observation: CopilotObservation,
    cancel: u8,
}
impl std::ops::Deref for TestWatcher {
    type Target = CopilotStatusWatcher;
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
    fn new(generation: String, copilot_home: PathBuf) -> Self {
        let (hooks, receiver) =
            crate::session::hook_queue::TestHookRoute::new(Runtime::Copilot, generation);
        Self {
            inner: CopilotStatusWatcher::from_receiver(receiver, copilot_home),
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
        let mut events = Vec::new();
        let result = self.inner.drain_events(
            cancel,
            |event| {
                events.push(event);
                Default::default()
            },
            starts,
        );
        for event in events {
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
            if let Some(value) = self
                .observation
                .model
                .reduce(event, crate::session::clock::timestamp_millis())
            {
                publish(value, source);
            }
        }
        self.observation.value = self.observation.model.value.clone();
        self.observation.parser = self.inner.parser.clone();
        result
    }
    fn drain_status(&mut self, publish: impl FnMut(AgentObservation, StatusSource)) -> Result<()> {
        self.drain_with_session_starts(publish, |_| {})
    }
}

fn report(event: &str) -> Value {
    json!({
        "hook_event_name": event,
        "session_id": "main",
    })
}

fn observe(state: &mut CopilotObservation, value: Value) -> Option<AgentObservation> {
    state.observe(
        serde_json::from_value::<StatusReport>(value).unwrap(),
        crate::session::clock::timestamp_millis(),
    )
}

fn pre_tool(name: &str, input: Value) -> Value {
    let mut value = report("PreToolUse");
    value["tool_name"] = json!(name);
    value["tool_input"] = input;
    value
}

fn post_tool(name: &str, input: Value) -> Value {
    let mut value = report("PostToolUse");
    value["tool_name"] = json!(name);
    value["tool_input"] = input;
    value
}

#[test]
fn changed_root_session_start_rekeys_after_prompt_but_not_duplicate_child_or_stale_reports() {
    let root = tempfile::tempdir().unwrap();

    let mut watcher = TestWatcher::new("current".into(), root.path().join("copilot-home"));
    let first = "11111111-1111-4111-8111-111111111111";
    let next = "22222222-2222-4222-8222-222222222222";
    let child = "33333333-3333-4333-8333-333333333333";

    for value in [
        json!({"generation":"current","hook_event_name":"UserPromptSubmit","session_id":first}),
        json!({"generation":"current","hook_event_name":"SessionStart","session_id":first}),
        json!({"generation":"current","hook_event_name":"SessionStart","session_id":child,"agentId":"child"}),
        json!({"generation":"old","hook_event_name":"SessionStart","session_id":child}),
        json!({"generation":"current","hook_event_name":"UserPromptSubmit","session_id":next}),
        json!({"generation":"current","hook_event_name":"SessionStart","session_id":next}),
        json!({"generation":"current","hook_event_name":"SessionStart","session_id":next}),
    ] {
        if value["generation"] == "old" {
            assert!(watcher.hooks.admit(value).is_err());
        } else {
            watcher.hooks.admit(value).unwrap();
        }
    }

    let conn = crate::db::test_connection().unwrap();
    let mut row = crate::repo::session::SessionRowDb::new_running("copilot-rekey".into());
    row.agent_session_key = Some(first.into());
    row.started_at = Some(chrono::Utc::now());
    let started = row.started_at.unwrap().to_rfc3339();
    crate::repo::session::insert(&conn, &row).unwrap();
    let mut keys = Vec::new();
    watcher
        .drain_with_session_starts(
            |_, _| {},
            |key| {
                assert!(crate::repo::session::rekey_agent_session_key(
                    &conn, &row.id, &key, &started
                )
                .unwrap());
                keys.push(key);
            },
        )
        .unwrap();
    assert_eq!(keys, [next]);
    let saved = crate::repo::session::get_row(&conn, &row.id)
        .unwrap()
        .unwrap();
    assert_eq!(saved.agent_session_key.as_deref(), Some(next));
    let plan =
        crate::runtimes::adapter(Runtime::Copilot).resume_plan(saved.agent_session_key.as_deref());
    assert!(plan.resuming);
    assert!(plan
        .args
        .windows(2)
        .any(|pair| pair == ["--session-id", next]));
    assert!(
        !crate::repo::session::rekey_agent_session_key(&conn, &row.id, child, "old-spawn").unwrap()
    );
    crate::repo::session::set_exit_status(
        &conn,
        &row.id,
        crate::model::SessionStatus::Stopped,
        chrono::Utc::now(),
    )
    .unwrap();
    assert!(
        !crate::repo::session::rekey_agent_session_key(&conn, &row.id, child, &started).unwrap()
    );
    assert_eq!(
        crate::repo::session::get_row(&conn, &row.id)
            .unwrap()
            .unwrap()
            .agent_session_key
            .as_deref(),
        Some(next)
    );
}

#[test]
fn session_start_publishes_hook_idle_and_preserves_session_guards() {
    let mut state = CopilotObservation::default();
    let mut start = report("SessionStart");
    start["source"] = json!("startup");
    let started = observe(&mut state, start.clone()).unwrap();
    assert_eq!(started.activity, Activity::Idle);
    assert_eq!(started.source, ObservationSource::Hook);
    assert_eq!(started.outcome, None);
    assert_eq!(started.detail, None);

    let mut handover = start.clone();
    handover["session_id"] = json!("next");
    let handed_over = observe(&mut state, handover).unwrap();
    assert_eq!(handed_over.activity, Activity::Idle);
    assert_eq!(handed_over.source, ObservationSource::Hook);

    let mut next_prompt = report("UserPromptSubmit");
    next_prompt["session_id"] = json!("next");
    observe(&mut state, next_prompt);
    let working = state.value.clone();
    let mut delayed = start.clone();
    delayed["session_id"] = json!("next");
    assert!(observe(&mut state, delayed).is_none());
    assert_eq!(state.value, working);

    let mut pre_compact = report("PreCompact");
    pre_compact["session_id"] = json!("next");
    observe(&mut state, pre_compact);
    let compacting = state.value.clone();
    let mut compact_start = start;
    compact_start["session_id"] = json!("next");
    compact_start["source"] = json!("compact");
    assert!(observe(&mut state, compact_start).is_none());
    assert_eq!(state.value, compacting);
}

#[test]
fn plugin_generation_has_the_exact_event_catalog_and_reporter_commands() {
    let root = tempfile::tempdir().unwrap();
    let app_data = root.path().join("runner app data");
    install_plugin(&app_data).unwrap();
    let identity: Value = serde_json::from_slice(
        &fs::read(identity_plugin_dir(&app_data).join("hooks/hooks.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        identity["hooks"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["SessionStart"]
    );
    assert!(plugin_available(&app_data));
    fs::write(hooks_path(&app_data), "{}").unwrap();
    install_plugin(&app_data).unwrap();
    assert!(!plugin_dir(&app_data).join("report.sh").exists());
    let plugin: Value =
        serde_json::from_slice(&fs::read(plugin_dir(&app_data).join("plugin.json")).unwrap())
            .unwrap();
    assert_eq!(plugin["name"], "runner-status");
    assert_eq!(plugin["version"], env!("CARGO_PKG_VERSION"));
    let hooks: Value = serde_json::from_slice(&fs::read(hooks_path(&app_data)).unwrap()).unwrap();
    assert_eq!(hooks["version"], 1);
    assert_eq!(hooks["hooks"].as_object().unwrap().len(), EVENTS.len());
    for event in EVENTS {
        let entry = &hooks["hooks"][event][0];
        assert_eq!(entry["hooks"].as_array().unwrap().len(), 1);
        assert_eq!(entry["hooks"][0]["type"], "command");
        assert_eq!(entry["hooks"][0]["timeout"], 10);
        assert_eq!(
            entry["hooks"][0]["command"],
            crate::runtimes::helpers::hook_report_command(
                &app_data,
                Runtime::Copilot,
                event,
                false
            )
        );
        let powershell = entry["hooks"][0]["powershell"].as_str().unwrap();
        assert_eq!(
            powershell,
            crate::runtimes::helpers::hook_report_command(&app_data, Runtime::Copilot, event, true)
        );
        assert!(powershell.ends_with(";exit 0"));
        assert!(powershell.contains(&format!("'{event}'")));
        assert!(!powershell.contains('"'));
    }
    assert_eq!(
        hooks["hooks"]["Notification"][0]["matcher"],
        "^(permission_prompt|elicitation_dialog|agent_idle)$"
    );
    let incomplete = root.path().join("incomplete");
    fs::create_dir_all(plugin_dir(&incomplete)).unwrap();
    fs::write(plugin_dir(&incomplete).join("report.sh"), "stale reporter").unwrap();
    assert!(!plugin_available(&incomplete));
}

#[test]
fn lifecycle_handles_live_start_order_continuation_end_and_isolation() {
    let mut state = CopilotObservation::default();
    assert_eq!(
        observe(&mut state, report("UserPromptSubmit"))
            .unwrap()
            .activity,
        Activity::Working
    );
    let mut delayed_start = report("SessionStart");
    delayed_start["source"] = json!("new");
    assert!(observe(&mut state, delayed_start).is_none());
    assert_eq!(state.value.activity, Activity::Working);

    let mut stop = report("Stop");
    stop["stop_reason"] = json!("end_turn");
    stop["stop_hook_active"] = json!(true);
    assert_eq!(
        observe(&mut state, stop.clone()).unwrap().outcome,
        Some(TurnOutcome::Completed)
    );
    assert_eq!(
        observe(&mut state, pre_tool("Bash", json!({"command":"echo ok"})))
            .unwrap()
            .activity,
        Activity::Working
    );
    observe(&mut state, stop);

    let mut stale = report("UserPromptSubmit");
    stale["session_id"] = json!("other");
    assert!(observe(&mut state, stale).is_none());
    assert_eq!(state.value.activity, Activity::Ready);
    assert_eq!(
        observe(&mut state, report("SessionEnd")).unwrap().activity,
        Activity::Unavailable
    );
    assert!(observe(&mut state, report("UserPromptSubmit")).is_none());
}

#[test]
fn tool_and_compaction_detail_follow_owned_work_events() {
    let mut state = CopilotObservation::default();
    assert_eq!(
        observe(&mut state, report("UserPromptSubmit"))
            .unwrap()
            .detail,
        None
    );
    let edit = json!({"file_path":"/tmp/one","diff":"+one"});
    let command = json!({"command":"echo two"});
    for (name, input) in [("Edit", edit.clone()), ("Bash", command.clone())] {
        assert_eq!(
            observe(&mut state, pre_tool(name, input)).unwrap().detail,
            Some(WorkDetail::UsingTools)
        );
    }
    assert_eq!(
        observe(&mut state, report("PreCompact")).unwrap().detail,
        Some(WorkDetail::CompactingContext)
    );
    assert_eq!(
        observe(&mut state, post_tool("Edit", edit)).unwrap().detail,
        Some(WorkDetail::UsingTools)
    );

    let start = json!({
        "type":"tool.execution_start",
        "data":{"toolCallId":"call-two","toolName":"shell","arguments":command},
    });
    assert!(state.observe_transcript(&start).is_none());
    let complete = json!({
        "type":"tool.execution_complete",
        "data":{"toolCallId":"call-two","success":true},
    });
    assert_eq!(state.observe_transcript(&complete).unwrap().detail, None);

    observe(&mut state, report("PreCompact"));
    assert_eq!(
        observe(&mut state, pre_tool("Bash", json!({"command":"echo next"})))
            .unwrap()
            .detail,
        Some(WorkDetail::UsingTools)
    );
    observe(&mut state, report("PreCompact"));
    assert_eq!(
        observe(
            &mut state,
            post_tool("Bash", json!({"command":"echo next"}))
        )
        .unwrap()
        .detail,
        None
    );
    observe(&mut state, report("PreCompact"));
    assert_eq!(
        observe(&mut state, report("UserPromptSubmit"))
            .unwrap()
            .detail,
        None
    );
    observe(&mut state, report("PreCompact"));
    let mut stop = report("Stop");
    stop["stop_reason"] = json!("end_turn");
    assert_eq!(observe(&mut state, stop).unwrap().detail, None);
}

#[test]
fn compaction_end_restores_idle_or_in_turn_work() {
    let mut state = CopilotObservation::default();
    observe(&mut state, report("UserPromptSubmit"));
    let mut stop = report("Stop");
    stop["stop_reason"] = json!("end_turn");
    observe(&mut state, stop);

    assert_eq!(
        observe(&mut state, report("PreCompact")).unwrap().detail,
        Some(WorkDetail::CompactingContext)
    );
    let mut compact_start = report("SessionStart");
    compact_start["source"] = json!("compact");
    assert!(observe(&mut state, compact_start).is_none());
    assert_eq!(state.value.detail, Some(WorkDetail::CompactingContext));
    let manual_done = observe(&mut state, report("PostToolUse")).unwrap();
    assert_eq!(manual_done.activity, Activity::Ready);
    assert_eq!(manual_done.outcome, Some(TurnOutcome::Completed));
    assert_eq!(manual_done.detail, None);

    observe(&mut state, report("PreCompact"));
    let mut stop = report("Stop");
    stop["stop_reason"] = json!("end_turn");
    let stopped_compaction = observe(&mut state, stop).unwrap();
    assert_eq!(stopped_compaction.activity, Activity::Ready);
    assert_eq!(stopped_compaction.outcome, Some(TurnOutcome::Completed));
    assert_eq!(stopped_compaction.detail, None);

    observe(&mut state, report("UserPromptSubmit"));
    let one = json!({"command":"echo one"});
    let two = json!({"command":"echo two"});
    observe(&mut state, pre_tool("Bash", one.clone()));
    observe(&mut state, pre_tool("Bash", two));
    observe(&mut state, report("PreCompact"));
    let mut compact_start = report("SessionStart");
    compact_start["source"] = json!("compact");
    assert!(observe(&mut state, compact_start).is_none());
    let automatic_done = observe(&mut state, post_tool("Bash", one)).unwrap();
    assert_eq!(automatic_done.activity, Activity::Working);
    assert_eq!(automatic_done.outcome, None);
    assert_eq!(automatic_done.detail, Some(WorkDetail::UsingTools));
}

#[test]
fn transcript_completion_tombstone_protects_parallel_same_named_tool() {
    let mut state = CopilotObservation::default();
    observe(&mut state, report("UserPromptSubmit"));
    let one = json!({"command":"echo one"});
    let two = json!({"command":"echo two"});
    observe(&mut state, pre_tool("Bash", one.clone()));
    observe(&mut state, pre_tool("Bash", two.clone()));

    let start = json!({
        "type":"tool.execution_start",
        "data":{"toolCallId":"call-one","toolName":"shell","arguments":one},
    });
    assert!(state.observe_transcript(&start).is_none());
    let complete = json!({
        "type":"tool.execution_complete",
        "data":{"toolCallId":"call-one","success":true},
    });
    assert!(state.observe_transcript(&complete).is_none());
    assert_eq!(state.value.detail, Some(WorkDetail::UsingTools));
    assert_eq!(state.tools.len(), 1);
    assert_eq!(state.completed_tools.len(), 1);

    observe(&mut state, report("PreCompact"));
    let after_hook = observe(&mut state, post_tool("Bash", json!({"command":"echo one"}))).unwrap();
    assert_eq!(after_hook.detail, Some(WorkDetail::UsingTools));
    assert_eq!(state.tools.len(), 1);
    assert_eq!(state.tools[0].input.as_ref(), Some(&two));
    assert!(state.completed_tools.is_empty());
}

#[test]
fn approval_requires_notification_and_result_clears_only_its_owner() {
    let mut state = CopilotObservation::default();
    observe(&mut state, report("UserPromptSubmit"));
    let patch =
        json!("*** Begin Patch\n*** Add File: /tmp/evidence.txt\n+evidence\n*** End Patch\n");
    observe(&mut state, pre_tool("Edit", patch.clone()));
    let permission = json!({
        "hook_event_name":"PermissionRequest",
        "sessionId":"main",
        "toolName":"edit",
        "toolInput":{"file_path":"/tmp/evidence.txt","diff":"+evidence"},
    });
    assert!(observe(&mut state, permission).is_none());
    assert!(!state.value.needs_you());
    let notification = json!({
        "hook_event_name":"Notification",
        "sessionId":"main",
        "notification_type":"permission_prompt",
    });
    let waiting = observe(&mut state, notification.clone()).unwrap();
    assert_eq!(waiting.interactions.len(), 1);
    assert_eq!(waiting.interactions[0].reason, WaitReason::Approval);
    assert!(observe(&mut state, notification).is_none());
    let working = observe(&mut state, post_tool("Edit", patch)).unwrap();
    assert_eq!(working.activity, Activity::Working);
    assert!(!working.needs_you());
}

#[test]
fn auto_approved_command_never_waits_and_subset_input_correlates() {
    let mut state = CopilotObservation::default();
    observe(&mut state, report("UserPromptSubmit"));
    let input = json!({"command":"echo runner-copilot-auto","description":"Print marker"});
    observe(&mut state, pre_tool("Bash", input.clone()));
    let permission = json!({
        "hook_event_name":"PermissionRequest",
        "sessionId":"main",
        "toolName":"bash",
        "toolInput":{"command":"echo runner-copilot-auto"},
    });
    assert!(observe(&mut state, permission).is_none());
    assert!(!state.value.needs_you());
    let result = observe(&mut state, post_tool("Bash", input)).unwrap();
    assert_eq!(result.activity, Activity::Working);
    assert!(!result.needs_you());
}

#[test]
fn named_question_waits_immediately_and_clears_on_answer() {
    let mut state = CopilotObservation::default();
    observe(&mut state, report("UserPromptSubmit"));
    let input = json!({"message":"Is the sky blue?"});
    let waiting = observe(&mut state, pre_tool("AskUserQuestion", input.clone())).unwrap();
    assert_eq!(waiting.interactions.len(), 1);
    assert_eq!(waiting.interactions[0].reason, WaitReason::Answer);
    let notification = json!({
        "hook_event_name":"Notification",
        "sessionId":"main",
        "notification_type":"elicitation_dialog",
    });
    assert!(observe(&mut state, notification).is_none());
    let working = observe(&mut state, post_tool("AskUserQuestion", input)).unwrap();
    assert_eq!(working.activity, Activity::Working);
    assert!(!working.needs_you());
}

#[test]
fn unmatched_permission_notification_never_claims_the_latest_tool() {
    let mut state = CopilotObservation::default();
    observe(&mut state, report("UserPromptSubmit"));
    observe(
        &mut state,
        pre_tool("Bash", json!({"command":"echo runner"})),
    );
    let notification = json!({
        "hook_event_name":"Notification",
        "sessionId":"main",
        "notification_type":"permission_prompt",
    });
    assert!(observe(&mut state, notification).is_none());
    assert!(!state.value.needs_you());
}

#[test]
fn live_edit_hook_and_transcript_shapes_resolve_interruption() {
    let mut state = CopilotObservation::default();
    observe(&mut state, report("UserPromptSubmit"));
    let input = json!(
            "*** Begin Patch\n*** Add File: /tmp/runner-copilot-live-evidence.txt\n+runner-copilot-live-evidence\n*** End Patch\n"
        );
    observe(&mut state, pre_tool("Edit", input.clone()));
    observe(
        &mut state,
        json!({
            "hook_event_name":"PermissionRequest",
            "hookName":"permissionRequest",
            "sessionId":"main",
            "toolName":"edit",
            "toolInput":{
                "file_path":"/tmp/runner-copilot-live-evidence.txt",
                "diff":"\ndiff --git a/tmp/runner-copilot-live-evidence.txt b/tmp/runner-copilot-live-evidence.txt\ncreate file mode 100644\nindex 0000000..0000000\n--- a/dev/null\n+++ b/tmp/runner-copilot-live-evidence.txt\n@@ -1,0 +1,2 @@\n+runner-copilot-live-evidence\n+\n\n",
            },
        }),
    );
    observe(
        &mut state,
        json!({
            "hook_event_name":"Notification",
            "sessionId":"main",
            "notification_type":"permission_prompt",
        }),
    );
    assert!(state.value.needs_you());
    let start = json!({
        "type":"tool.execution_start",
        "data":{
            "toolCallId":"custom_call_UGtgEO25P5H4iLqDGELNYw11",
            "toolName":"apply_patch",
            "arguments":input,
            "turnId":"0",
        },
    });
    assert!(state.observe_transcript(&start).is_none());
    state.model.reduce(
        AgentEvent::LocalCancel {
            kind: crate::session::state::ESCAPE_INTERRUPT,
        },
        crate::session::clock::timestamp_millis(),
    );
    state.value = state.model.value.clone();
    let complete = json!({
        "type":"tool.execution_complete",
        "data":{
            "toolCallId":"custom_call_UGtgEO25P5H4iLqDGELNYw11",
            "turnId":"0",
            "success":false,
        },
    });
    let resolved = state.observe_transcript(&complete).unwrap();
    assert_eq!(resolved.activity, Activity::Ready);
    assert_eq!(resolved.outcome, Some(TurnOutcome::Interrupted));
    assert!(!resolved.needs_you());
}

#[test]
fn transcript_backlog_cannot_claim_a_live_same_name_wait() {
    let mut state = CopilotObservation::default();
    observe(&mut state, report("UserPromptSubmit"));
    let completed = json!({"command":"echo one"});
    observe(&mut state, pre_tool("Bash", completed.clone()));
    observe(&mut state, post_tool("Bash", completed.clone()));

    let pending = json!({"command":"rm -rf /tmp/x"});
    observe(&mut state, pre_tool("Bash", pending.clone()));
    observe(
        &mut state,
        json!({
            "hook_event_name":"PermissionRequest",
            "sessionId":"main",
            "toolName":"bash",
            "toolInput":pending,
        }),
    );
    observe(
        &mut state,
        json!({
            "hook_event_name":"Notification",
            "sessionId":"main",
            "notification_type":"permission_prompt",
        }),
    );
    assert!(state.value.needs_you());

    let historical_start = json!({
        "type":"tool.execution_start",
        "data":{
            "toolCallId":"call-completed",
            "toolName":"shell",
            "arguments":completed,
            "turnId":"0",
        },
    });
    let historical_complete = json!({
        "type":"tool.execution_complete",
        "data":{"toolCallId":"call-completed","turnId":"0","success":true},
    });
    assert!(state.observe_transcript(&historical_start).is_none());
    assert!(state.observe_transcript(&historical_complete).is_none());
    assert!(state.value.needs_you());
    assert_eq!(state.tools.len(), 1);

    let live_start = json!({
        "type":"tool.execution_start",
        "data":{
            "toolCallId":"call-pending",
            "toolName":"shell",
            "arguments":{"command":"rm -rf /tmp/x"},
            "turnId":"0",
        },
    });
    assert!(state.observe_transcript(&live_start).is_none());
    assert_eq!(state.transcript_calls["call-pending"], state.tools[0].id);
    assert!(state.value.needs_you());
}

#[test]
fn camel_case_hook_and_transcript_subagents_cannot_touch_main_state() {
    let mut state = CopilotObservation::default();
    observe(&mut state, report("UserPromptSubmit"));
    let mut subagent = pre_tool("Bash", json!({"command":"echo child"}));
    subagent["agentId"] = json!("child");
    assert!(observe(&mut state, subagent).is_none());
    assert!(state.tools.is_empty());

    let input = json!({"message":"Choose"});
    observe(&mut state, pre_tool("AskUserQuestion", input.clone()));
    let start = json!({
        "type":"tool.execution_start",
        "data":{
            "toolCallId":"child-question",
            "toolName":"ask_user",
            "arguments":input,
            "agentId":"child",
        },
    });
    let complete = json!({
        "type":"tool.execution_complete",
        "data":{"toolCallId":"child-question","agentId":"child","success":false},
    });
    assert!(state.observe_transcript(&start).is_none());
    assert!(state.observe_transcript(&complete).is_none());
    assert!(state.value.needs_you());
    assert!(state.transcript_calls.is_empty());

    let main_start = json!({
        "type":"tool.execution_start",
        "data":{
            "toolCallId":"main-question",
            "toolName":"ask_user",
            "arguments":{"message":"Choose"},
            "agentId":null,
        },
    });
    let main_complete = json!({
        "type":"tool.execution_complete",
        "data":{"toolCallId":"main-question","agentId":null,"success":false},
    });
    assert!(state.observe_transcript(&main_start).is_none());
    assert_eq!(state.transcript_calls.len(), 1);
    let resolved = state.observe_transcript(&main_complete).unwrap();
    assert!(!resolved.needs_you());
}

#[test]
fn input_interrupt_keeps_the_wait_until_transcript_completion() {
    let root = tempfile::tempdir().unwrap();

    let home = root.path().join("copilot-home");
    let transcript = home.join("session-state/main/events.jsonl");
    fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    let input = json!({"message":"Choose"});
    fs::write(
        &transcript,
        format!(
            "{}\n",
            json!({
                "type":"tool.execution_start",
                "data":{
                    "toolCallId":"call-question",
                    "toolName":"ask_user",
                    "arguments":input,
                },
            })
        ),
    )
    .unwrap();

    let mut watcher = TestWatcher::new("current".into(), home);

    for value in [
        report("UserPromptSubmit"),
        pre_tool("AskUserQuestion", json!({"message":"Choose"})),
    ] {
        let mut value = value;
        value["generation"] = json!("current");
        watcher.hooks.admit(value).unwrap();
    }

    watcher.drain_status(|_, _| {}).unwrap();
    assert!(watcher.observation.value.needs_you());
    watcher.cancel = crate::session::state::ESCAPE_INTERRUPT;
    watcher.drain_status(|_, _| {}).unwrap();
    assert_eq!(watcher.observation.value.activity, Activity::Unavailable);
    assert!(watcher.observation.value.needs_you());
    assert_eq!(watcher.observation.value.detail, None);

    writeln!(
        OpenOptions::new().append(true).open(&transcript).unwrap(),
        "{}",
        json!({
            "type":"tool.execution_complete",
            "data":{"toolCallId":"call-question","success":false},
        })
    )
    .unwrap();
    watcher.drain_status(|_, _| {}).unwrap();
    assert_eq!(watcher.observation.value.activity, Activity::Ready);
    assert_eq!(
        watcher.observation.value.outcome,
        Some(TurnOutcome::Interrupted)
    );
    assert!(!watcher.observation.value.needs_you());
}

#[test]
fn approved_wait_cancellation_without_tool_completion_closes_its_approval() {
    for (completion_before_hooks, completion_before_escape) in
        [(false, true), (false, false), (true, true)]
    {
        let root = tempfile::tempdir().unwrap();

        let home = root.path().join("copilot-home");
        let transcript = home.join("session-state/main/events.jsonl");
        fs::create_dir_all(transcript.parent().unwrap()).unwrap();
        // Sanitised #777 sleep-60 records: permission completes, but the tool never does.
        let input = json!({"command":"sleep 60","description":"Bounded wait","initial_wait":70});
        fs::write(&transcript, format!("{}\n", json!({
                "type":"tool.execution_start",
                "data":{"toolCallId":"wait-call","toolName":"bash","arguments":input,"turnId":"0"},
            }))).unwrap();

        let mut watcher = TestWatcher::new("current".into(), home);

        let approval = [
            json!({"hook_event_name":"PermissionRequest","session_id":"main","tool_name":"bash","tool_input":{"command":"sleep 60"}}),
            json!({"hook_event_name":"Notification","session_id":"main","notification_type":"permission_prompt"}),
        ];
        for mut value in [report("UserPromptSubmit"), pre_tool("Bash", input.clone())] {
            value["generation"] = json!("current");
            watcher.hooks.admit(value).unwrap();
        }
        if !completion_before_hooks {
            for mut value in approval.clone() {
                value["generation"] = json!("current");
                watcher.hooks.admit(value).unwrap();
            }
        }
        watcher.drain_status(|_, _| {}).unwrap();
        assert_eq!(
            watcher.observation.value.needs_you(),
            !completion_before_hooks
        );
        let mut events = OpenOptions::new().append(true).open(&transcript).unwrap();
        let completion = json!({
            "type":"permission.completed",
            "data":{"requestId":"wait-permission","toolCallId":"wait-call","result":{"kind":"approved"},"decisionSource":"human_response"},
        });
        if completion_before_escape {
            writeln!(events, "{completion}").unwrap();
            watcher.drain_status(|_, _| {}).unwrap();
            assert_eq!(watcher.observation.value.activity, Activity::Working);
            assert!(!watcher.observation.value.needs_you());
            assert_eq!(
                watcher.observation.value.detail,
                Some(WorkDetail::UsingTools)
            );
        }
        if completion_before_hooks {
            for mut value in approval {
                value["generation"] = json!("current");
                watcher.hooks.admit(value).unwrap();
            }

            watcher.drain_status(|_, _| {}).unwrap();
            assert!(
                !watcher.observation.value.needs_you(),
                "late hooks reopened approval"
            );
        }
        watcher.cancel = crate::session::pty_runtime::interrupt_key(b"\x1b[27u").unwrap();
        watcher.drain_status(|_, _| {}).unwrap();
        assert_eq!(
            watcher.observation.value.needs_you(),
            !completion_before_escape
        );
        if !completion_before_escape {
            assert_eq!(watcher.observation.value.activity, Activity::Unavailable);
            writeln!(events, "{completion}").unwrap();
        }
        writeln!(
            events,
            "{}",
            json!({"type":"assistant.turn_end","data":{"turnId":"0"}})
        )
        .unwrap();
        writeln!(
            events,
            "{}",
            json!({"type":"abort","data":{"reason":"user_abort"}})
        )
        .unwrap();
        watcher.drain_status(|_, _| {}).unwrap();
        assert_eq!(watcher.observation.value.activity, Activity::Ready);
        assert_eq!(
            watcher.observation.value.outcome,
            Some(TurnOutcome::Interrupted)
        );
        assert!(!watcher.observation.value.needs_you());
        assert_eq!(watcher.observation.value.detail, None);
        assert_eq!(watcher.observation.tools.len(), 1);

        for mut value in [
            report("UserPromptSubmit"),
            json!({"hook_event_name":"Stop","session_id":"main","stop_reason":"end_turn"}),
        ] {
            value["generation"] = json!("current");
            watcher.hooks.admit(value).unwrap();
        }

        watcher.drain_status(|_, _| {}).unwrap();
        assert_eq!(watcher.observation.value.activity, Activity::Ready);
        assert_eq!(
            watcher.observation.value.outcome,
            Some(TurnOutcome::Completed)
        );
        assert!(watcher.observation.tools.is_empty());
    }
}

#[test]
fn permission_completion_resolves_only_the_correlated_approval() {
    for (native_name, commands) in [
        ("bash", ["sleep 60", "sleep 30"]),
        (
            "powershell",
            ["Start-Sleep -Seconds 60", "Start-Sleep -Seconds 30"],
        ),
    ] {
        let mut state = CopilotObservation::default();
        observe(&mut state, report("UserPromptSubmit"));
        for (call_id, command) in [("one", commands[0]), ("two", commands[1])] {
            let input = json!({"command":command,"description":"Bounded wait","initial_wait":45});
            observe(&mut state, pre_tool("Bash", input.clone()));
            observe(
                &mut state,
                json!({"hook_event_name":"PermissionRequest","hookName":"permissionRequest","sessionId":"main","toolName":native_name,"toolInput":{"command":command}}),
            );
            state.observe_transcript(&json!({"type":"tool.execution_start","data":{"toolCallId":call_id,"toolName":native_name,"arguments":input}}));
        }
        assert!(!state.value.needs_you());
        observe(
            &mut state,
            json!({"hook_event_name":"Notification","session_id":"main","notification_type":"permission_prompt"}),
        );
        assert!(state.value.needs_you());
        assert_eq!(state.transcript_calls.len(), 2);
        let second = state.transcript_calls["two"].clone();
        state.wait(
            crate::session::clock::timestamp_millis(),
            WaitReason::Answer,
            vec![state.transcript_calls["one"].clone()],
        );
        let before = state.value.clone();
        for record in [
            json!({"type":"permission.completed","data":{"toolCallId":"unknown"}}),
            json!({"type":"permission.completed","data":{"toolCallId":"one","agentId":"worker"}}),
        ] {
            assert!(state.observe_transcript(&record).is_none());
            assert_eq!(state.value, before);
        }
        let value = state.observe_transcript(&json!({"type":"permission.completed","data":{"toolCallId":"one","result":{"kind":"approved"}}})).unwrap();
        assert_eq!(value.interactions.len(), 2);
        assert_eq!(value.interactions[0].reason, WaitReason::Approval);
        assert_eq!(value.interactions[0].owners, std::slice::from_ref(&second));
        assert_eq!(value.interactions[1].reason, WaitReason::Answer);
        assert_eq!(
            value.interactions[1].owners,
            std::slice::from_ref(&state.transcript_calls["one"])
        );
        assert_eq!(state.permission_owners, std::slice::from_ref(&second));
        assert_eq!(state.tools.len(), 2);
        assert!(observe(&mut state, json!({"hook_event_name":"PermissionRequest","hookName":"permissionRequest","sessionId":"main","toolName":native_name,"toolInput":{"command":commands[0]}})).is_none());
        assert!(observe(&mut state, json!({"hook_event_name":"Notification","session_id":"main","notification_type":"permission_prompt"})).is_none());
        assert_eq!(state.value.published(), value);
        assert!(state
            .observe_transcript(&json!({"type":"permission.completed","data":{"toolCallId":"one"}}))
            .is_none());
        let value = state.observe_transcript(&json!({"type":"tool.execution_complete","data":{"toolCallId":"one","success":true}})).unwrap();
        assert_eq!(state.tools.len(), 1);
        assert_eq!(state.tools[0].id, second);
        assert_eq!(value.interactions.len(), 1);
        assert_eq!(value.interactions[0].owners, std::slice::from_ref(&second));
        observe(
            &mut state,
            post_tool(
                "Bash",
                json!({"command":commands[0],"description":"Bounded wait","initial_wait":45}),
            ),
        );
        assert_eq!(state.tools.len(), 1);
        assert_eq!(state.tools[0].id, second);
        assert!(state.value.needs_you());
    }
}

#[test]
fn uncaptured_error_event_remains_unmapped() {
    let mut state = CopilotObservation::default();
    observe(&mut state, report("UserPromptSubmit"));
    let mut error = report("ErrorOccurred");
    error["recoverable"] = json!(false);
    error["error_context"] = json!("model_call");
    assert!(observe(&mut state, error).is_none());
    assert_eq!(state.value.activity, Activity::Working);
    assert_eq!(state.value.outcome, None);
}

#[test]
fn malformed_unacknowledged_generation_and_session_reports_are_isolated() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("copilot-home");
    let mut watcher = TestWatcher::new("current".into(), home);
    assert!(watcher.hooks.admit_json("not json").is_err());
    assert!(watcher
        .hooks
        .admit(json!({"generation":"old","hook_event_name":"UserPromptSubmit","session_id":"main"}))
        .is_err());
    let pending = watcher.hooks.admit(json!({"generation":"current","hook_event_name":"UserPromptSubmit","session_id":"main"})).unwrap();
    watcher
        .drain_status(|_, _| panic!("unacknowledged report"))
        .unwrap();
    drop(pending);
    watcher.hooks.admit(json!({"generation":"current","hook_event_name":"Stop","session_id":"foreign","stop_reason":"end_turn"})).unwrap();
    let mut values = Vec::new();
    watcher.drain_status(|value, _| values.push(value)).unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].activity, Activity::Working);
}

#[test]
fn admitted_large_payload_preserves_status_and_reports_bridge_loss() {
    let root = tempfile::tempdir().unwrap();
    let mut watcher = TestWatcher::new("current".into(), root.path().join("copilot-home"));
    let mut payload = report("UserPromptSubmit");
    payload["prompt"] = json!("x\n".repeat(128 * 1024));
    watcher
        .hooks
        .admit_json(&serde_json::to_string_pretty(&payload).unwrap())
        .unwrap();
    let mut values = Vec::new();
    watcher.drain_status(|value, _| values.push(value)).unwrap();
    assert_eq!(values.last().unwrap().activity, Activity::Working);
    watcher.hooks.retire();
    assert!(watcher.drain_status(|_, _| {}).is_err());
}

#[test]
fn disabled_hooks_leave_the_baseline_unlatched() {
    let root = tempfile::tempdir().unwrap();

    let mut watcher = TestWatcher::new("current".into(), root.path().join("copilot-home"));
    let mut transitions = 0;
    watcher.drain_status(|_, _| transitions += 1).unwrap();
    assert_eq!(transitions, 0);
    assert_eq!(
        watcher.observation.value.source,
        ObservationSource::Unavailable
    );
}
