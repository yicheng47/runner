use super::*;
use crate::session::hook_feed::HookFeed;
use crate::session::state::agent::{AdapterFeedback, AgentEvent};
use crate::session::status::{TurnOutcome, WaitReason};
use serde_json::{json, Value};
use std::collections::HashSet;

const EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PostToolUse",
    "PostToolUseFailure",
    "Notification",
    "Stop",
    "StopFailure",
    "StopCancelled",
];
fn hooks() -> Value {
    let mut hooks = serde_json::Map::new();
    for event in EVENTS {
        let status_guard = if matches!(*event, "SessionStart" | "UserPromptSubmit") {
            ""
        } else {
            "[ \"$RUNNER_STATUS_HOOKS\" != \"false\" ] && "
        };
        let mut group = json!({"hooks":[{"type":"command","command":format!("if {status_guard}[ -n \"$RUNNER_HOOK_EXECUTABLE\" ]; then \"$RUNNER_HOOK_EXECUTABLE\" hook report --runtime grok --event {event} >/dev/null 2>&1; fi; exit 0"),"timeout":2}]});
        if *event == "Notification" {
            group["matcher"] = json!("^(permission_prompt|idle_prompt)$");
        }
        hooks.insert((*event).into(), json!([group]));
    }
    json!({"hooks":hooks})
}
pub(super) fn install(home: &Path) -> crate::error::Result<()> {
    if !home.is_dir() {
        return Ok(());
    }
    write_config_atomically(
        &home.join("hooks/runner-status.json"),
        &serde_json::to_vec_pretty(&hooks())?,
    )
}
pub(super) struct Watcher {
    feed: HookFeed,
    key: Option<String>,
    prompt: Option<String>,
    started_prompts: HashSet<String>,
}
impl Watcher {
    pub(super) fn new(
        receiver: crate::session::hook_queue::HookReceiver,
        spec: &SpawnSpec,
    ) -> Self {
        let key = spec
            .args
            .windows(2)
            .find(|args| matches!(args[0].as_str(), "-s" | "--session-id" | "--resume"))
            .map(|args| args[1].clone());
        Self {
            feed: HookFeed::from_receiver(receiver),
            key,
            prompt: None,
            started_prompts: HashSet::new(),
        }
    }
    fn events(report: &Value) -> Vec<AgentEvent> {
        let ended = |outcome| vec![AgentEvent::TurnEnded { outcome }];
        match report["hook_event_name"].as_str().unwrap_or_default() {
            "UserPromptSubmit" => vec![AgentEvent::TurnStarted],
            "PostToolUse" | "PostToolUseFailure" => vec![
                AgentEvent::ClearInteractions,
                AgentEvent::Working { detail: None },
            ],
            "Notification" => match report["notificationType"].as_str() {
                Some("permission_prompt") => vec![AgentEvent::InteractionOpened {
                    reason: WaitReason::Approval,
                    owners: vec![report["promptId"].as_str().unwrap_or("permission").into()],
                }],
                Some("idle_prompt") => {
                    vec![AgentEvent::ClearInteractions, AgentEvent::IdlePrompt]
                }
                _ => Vec::new(),
            },
            "Stop" if report["reason"] == "end_turn" => ended(TurnOutcome::Completed),
            "StopFailure" => ended(TurnOutcome::Failed),
            "StopCancelled" => ended(TurnOutcome::Interrupted),
            _ => Vec::new(),
        }
    }
}
impl HookWatcher for Watcher {
    fn drain_events(
        &mut self,
        cancel: u8,
        emit: &mut dyn FnMut(AgentEvent) -> AdapterFeedback,
        session_start: &mut dyn FnMut(String),
    ) -> crate::error::Result<()> {
        let key = &mut self.key;
        let prompt = &mut self.prompt;
        let started_prompts = &mut self.started_prompts;
        self.feed.drain(|report| {
            if ["subagentType", "subagent_type", "agentId", "agent_id"]
                .iter()
                .any(|field| report.get(*field).is_some_and(|value| !value.is_null()))
            {
                return;
            }
            let Some(id) = report["sessionId"].as_str().filter(|id| is_uuid(id)) else {
                return;
            };
            let event = report["hook_event_name"].as_str().unwrap_or_default();
            if matches!(event, "SessionStart" | "UserPromptSubmit") {
                if key.as_deref() != Some(id) {
                    *key = Some(id.into());
                    *prompt = None;
                    started_prompts.clear();
                    session_start(id.into());
                    emit(AgentEvent::Batch {
                        runtime: Runtime::Grok,
                        events: vec![AgentEvent::ConversationRecovered, AgentEvent::Ready],
                    });
                }
                if event == "SessionStart" {
                    return;
                }
            }
            if key.as_deref() != Some(id) {
                return;
            }
            let report_prompt = report["promptId"].as_str();
            if event == "UserPromptSubmit" {
                *prompt = report_prompt.map(str::to_owned);
                if let Some(prompt) = report_prompt {
                    started_prompts.insert(prompt.into());
                }
            } else if report_prompt
                .is_some_and(|id| started_prompts.contains(id) && prompt.as_deref() != Some(id))
            {
                return;
            }
            let events = Self::events(&report);
            if !events.is_empty() {
                emit(AgentEvent::Batch {
                    runtime: Runtime::Grok,
                    events,
                });
            }
        })?;
        if cancel & crate::session::state::CTRL_C_INTERRUPT != 0 {
            emit(AgentEvent::Batch {
                runtime: Runtime::Grok,
                events: vec![AgentEvent::LocalCancel { kind: cancel }],
            });
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests;
