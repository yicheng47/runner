use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Runtime;

pub const VERSION: u8 = 1;
pub const MAX_ENVELOPE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_QUEUE_REPORTS: usize = 64;
pub const MAX_QUEUE_BYTES: usize = 16 * 1024 * 1024;
pub const DEADLINE: std::time::Duration = std::time::Duration::from_millis(250);
pub const ENDPOINT_ENV: &str = "RUNNER_HOOK_ENDPOINT";
pub const SESSION_ENV: &str = "RUNNER_HOOK_SESSION";
pub const GENERATION_ENV: &str = "RUNNER_HOOK_GENERATION";
pub const EXECUTABLE_ENV: &str = "RUNNER_HOOK_EXECUTABLE";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HookReport {
    #[serde(default)]
    pub bridge_unavailable: bool,
    pub version: u8,
    pub runtime: Runtime,
    pub session_id: String,
    pub generation: String,
    pub event: String,
    pub payload: Value,
    pub caller_thread_id: Option<String>,
}

impl HookReport {
    pub fn valid(&self) -> bool {
        self.version == VERSION
            && !self.session_id.is_empty()
            && !self.generation.is_empty()
            && !self.event.is_empty()
            && self.payload.is_object()
            && self.payload.get("hook_event_name").is_none_or(|event| {
                event
                    .as_str()
                    .is_some_and(|name| name.is_empty() || name == self.event)
            })
            && (self.runtime != Runtime::Codex
                || self
                    .payload
                    .get("session_id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| {
                        !id.is_empty() && self.caller_thread_id.as_deref() == Some(id)
                    }))
    }
}

/// Identifies a Runner session's own Codex conversation, in its first prompt
/// or its startup context, so the daemon can find that conversation's rollout.
pub fn codex_session_marker(session_id: &str) -> String {
    format!("<!-- runner-codex-session-key-capture:{session_id} -->")
}

/// The SessionStart reply that marks a fresh startup of the session's own
/// Codex thread; every other hook gets no output.
pub fn codex_startup_output(
    session_id: &str,
    payload: &Value,
    caller_thread_id: Option<&str>,
) -> Option<Value> {
    (payload["hook_event_name"] == "SessionStart"
        && payload["source"] == "startup"
        && caller_thread_id.is_some_and(|id| payload["session_id"].as_str() == Some(id)))
    .then(|| {
        serde_json::json!({"hookSpecificOutput": {
            "hookEventName": "SessionStart",
            "additionalContext": codex_session_marker(session_id),
        }})
    })
}
