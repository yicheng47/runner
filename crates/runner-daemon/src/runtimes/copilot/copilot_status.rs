use crate::model::Runtime;
use crate::session::state::agent::{AdapterFeedback, AgentEvent};
#[cfg(test)]
use crate::session::state::StatusSource;
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::error::Result;
use crate::session::hook_feed::HookFeed;
use crate::session::status::{TurnOutcome, WaitReason};

#[cfg(test)]
use crate::session::status::{Activity, AgentObservation, ObservationSource, WorkDetail};
pub(crate) const EVENTS: &[&str] = &[
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "PermissionRequest",
    "Notification",
    "Stop",
    "ErrorOccurred",
    "PreCompact",
    "SubagentStart",
    "SubagentStop",
];

const PLUGIN_DIR: &str = "copilot-hooks";
const IDENTITY_PLUGIN_DIR: &str = "copilot-identity-hooks";

pub(crate) fn identity_plugin_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(IDENTITY_PLUGIN_DIR)
}

pub(crate) fn plugin_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(PLUGIN_DIR)
}

fn hooks_path(app_data_dir: &Path) -> PathBuf {
    plugin_dir(app_data_dir).join("hooks/hooks.json")
}

pub(crate) fn plugin_available(app_data_dir: &Path) -> bool {
    [
        plugin_dir(app_data_dir).join("plugin.json"),
        hooks_path(app_data_dir),
    ]
    .iter()
    .all(|path| path.is_file())
}

fn write_plugin_file(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path.parent().expect("plugin file has a parent");
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(contents)?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

// Copilot runs this slot instead of `command` on Windows.

pub(crate) fn install_plugin(app_data_dir: &Path) -> Result<()> {
    install_plugin_at(&plugin_dir(app_data_dir), app_data_dir, EVENTS)?;
    install_plugin_at(
        &identity_plugin_dir(app_data_dir),
        app_data_dir,
        &["SessionStart"],
    )
}

fn install_plugin_at(plugin_dir: &Path, app_data_dir: &Path, events: &[&str]) -> Result<()> {
    fs::create_dir_all(plugin_dir.join("hooks"))?;
    write_plugin_file(
        &plugin_dir.join("plugin.json"),
        &serde_json::to_vec(&serde_json::json!({
            "name": "runner-status",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Runner session status hooks",
        }))?,
    )?;
    let mut hooks = serde_json::Map::new();
    for event in events {
        let command = crate::runtimes::helpers::hook_report_command(
            app_data_dir,
            Runtime::Copilot,
            event,
            false,
        );
        let mut entry = serde_json::json!({
            "hooks": [{
                "type": "command",
                "command": command,
                "powershell": crate::runtimes::helpers::hook_report_command(app_data_dir, Runtime::Copilot, event, true),
                "timeout": crate::runtimes::helpers::HOOK_TIMEOUT_SECS,
            }],
        });
        if *event == "Notification" {
            entry["matcher"] =
                Value::String("^(permission_prompt|elicitation_dialog|agent_idle)$".into());
        }
        hooks.insert((*event).into(), Value::Array(vec![entry]));
    }
    write_plugin_file(
        &plugin_dir.join("hooks/hooks.json"),
        &serde_json::to_vec(&serde_json::json!({"version": 1, "hooks": hooks}))?,
    )?;
    Ok(())
}

#[derive(Default, Deserialize)]
struct StatusReport {
    #[serde(default)]
    hook_event_name: String,
    source: Option<String>,
    #[serde(alias = "sessionId")]
    pub(crate) session_id: Option<String>,
    #[serde(alias = "notificationType")]
    notification_type: Option<String>,
    #[serde(alias = "toolName")]
    tool_name: Option<String>,
    #[serde(alias = "toolInput")]
    tool_input: Option<Value>,
    pub(crate) transcript_path: Option<PathBuf>,
    stop_reason: Option<String>,
    #[serde(alias = "agentId")]
    agent_id: Option<String>,
}

fn transcript_is_subagent(record: &Value, data: &Value) -> bool {
    [record, data].into_iter().any(|value| {
        value.get("agentId").is_some_and(|agent| !agent.is_null())
            || value.get("agent_id").is_some_and(|agent| !agent.is_null())
            || value.get("isSidechain").and_then(Value::as_bool) == Some(true)
            || value.get("is_sidechain").and_then(Value::as_bool) == Some(true)
    })
}

fn canonical_tool_name(name: &str) -> String {
    let compact: String = name
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    match compact.as_str() {
        "askuser" | "askuserquestion" => "ask_user".into(),
        "applypatch" | "edit" => "edit".into(),
        "bash" | "shell" | "powershell" => "bash".into(),
        _ => compact,
    }
}

fn inputs_match(left: Option<&Value>, right: Option<&Value>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => value_contains(left, right) || value_contains(right, left),
        (None, None) => true,
        _ => false,
    }
}

fn value_contains(value: &Value, expected: &Value) -> bool {
    match (value, expected) {
        (Value::Object(value), Value::Object(expected)) => {
            expected.iter().all(|(key, expected)| {
                value
                    .get(key)
                    .is_some_and(|value| value_contains(value, expected))
            })
        }
        (Value::Array(value), Value::Array(expected)) => value == expected,
        _ => value == expected,
    }
}

#[derive(Clone)]
struct PendingTool {
    id: String,
    name: String,
    input: Option<Value>,
    prompted: bool,
}
#[derive(Clone, Default)]
struct CopilotParser {
    session_id: Option<String>,
    ended: bool,
    transcript_path: Option<PathBuf>,
    seen: bool,
    tools: Vec<PendingTool>,
    completed_tools: Vec<PendingTool>,
    compacting: bool,
    permission_owners: Vec<String>,
    completed_permissions: std::collections::BTreeSet<String>,
    transcript_calls: std::collections::BTreeMap<String, String>,
    next_tool: u64,
}
impl CopilotParser {
    fn hook(&mut self, report: StatusReport) -> Option<Vec<AgentEvent>> {
        if report.agent_id.is_some() || report.hook_event_name.starts_with("Subagent") {
            return None;
        }
        let session = report.session_id.filter(|id| !id.is_empty())?;
        if report.hook_event_name == "SessionStart" {
            if report.source.as_deref() == Some("compact")
                && self.compacting
                && self
                    .session_id
                    .as_ref()
                    .is_none_or(|current| current == &session)
            {
                self.session_id = Some(session);
                if let Some(path) = report.transcript_path {
                    self.transcript_path = Some(path);
                }
                return None;
            }
            if self.session_id.as_ref() == Some(&session) && self.seen {
                return None;
            }
            self.session_id = Some(session);
            self.ended = false;
            self.clear();
            self.seen = true;
            return Some(vec![AgentEvent::StartupReady]);
        }
        if self
            .session_id
            .as_ref()
            .is_some_and(|current| *current != session)
        {
            return None;
        }
        self.session_id.get_or_insert(session);
        if self.ended {
            return None;
        }
        if let Some(path) = report.transcript_path {
            self.transcript_path = Some(path);
        }
        let event = match report.hook_event_name.as_str() {
            "SessionEnd" => {
                self.ended = true;
                self.clear();
                AgentEvent::SessionEnded
            }
            "UserPromptSubmit" => {
                self.clear();
                AgentEvent::TurnStarted
            }
            "PreToolUse" => {
                let name = report.tool_name?;
                self.compacting = false;
                self.next_tool += 1;
                let owner = format!("copilot-tool-{}", self.next_tool);
                let question = (canonical_tool_name(&name) == "ask_user").then(|| owner.clone());
                self.tools.push(PendingTool {
                    id: owner,
                    name,
                    input: report.tool_input,
                    prompted: question.is_some(),
                });
                AgentEvent::ToolStarted {
                    count: self.tools.len(),
                    question,
                }
            }
            "PermissionRequest" => {
                let owner = self.matching_tool(
                    report.tool_name.as_deref()?,
                    report.tool_input.as_ref(),
                    true,
                )?;
                if self.completed_permissions.contains(&owner) {
                    return None;
                }
                if !self.permission_owners.contains(&owner) {
                    self.permission_owners.push(owner.clone());
                }
                AgentEvent::PermissionRequested {
                    reason: WaitReason::Approval,
                    owners: vec![owner],
                }
            }
            "Notification" => match report.notification_type.as_deref() {
                Some("permission_prompt") => {
                    if !self
                        .tools
                        .iter()
                        .any(|tool| self.permission_owners.contains(&tool.id))
                    {
                        return None;
                    }
                    for tool in &mut self.tools {
                        tool.prompted |= self.permission_owners.contains(&tool.id);
                    }
                    AgentEvent::PermissionPrompt {
                        candidates: self
                            .tools
                            .iter()
                            .map(|tool| (tool.id.clone(), WaitReason::Approval))
                            .collect(),
                    }
                }
                Some("elicitation_dialog") => {
                    let owners: Vec<_> = self
                        .tools
                        .iter()
                        .filter(|tool| canonical_tool_name(&tool.name) == "ask_user")
                        .map(|tool| tool.id.clone())
                        .collect();
                    if owners.is_empty() {
                        return None;
                    }
                    AgentEvent::ElicitationPrompt { owners }
                }
                Some("agent_idle") => {
                    if !self.tools.iter().any(|tool| tool.prompted) {
                        self.compacting = false;
                    }
                    AgentEvent::IdlePrompt
                }
                _ => return None,
            },
            "PostToolUse" | "PostToolUseFailure" => {
                let owner = report.tool_name.as_deref().and_then(|name| {
                    if self.consume_completed_tool(name, report.tool_input.as_ref()) {
                        Some(None)
                    } else {
                        self.remove_tool(name, report.tool_input.as_ref()).map(Some)
                    }
                });
                let owner = match owner {
                    Some(owner) => owner,
                    None if self.compacting => None,
                    _ => return None,
                };
                self.compacting = false;
                AgentEvent::ToolEnded {
                    owner,
                    count: self.tools.len(),
                    interrupted: false,
                    transcript: false,
                }
            }
            "PreCompact" => {
                self.compacting = true;
                AgentEvent::CompactionStarted
            }
            "Stop" if report.stop_reason.as_deref() == Some("end_turn") => {
                self.clear();
                AgentEvent::TurnEnded {
                    outcome: TurnOutcome::Completed,
                }
            }
            _ => return None,
        };
        self.seen = true;
        Some(vec![event])
    }
    fn clear(&mut self) {
        self.tools.clear();
        self.completed_tools.clear();
        self.compacting = false;
        self.permission_owners.clear();
        self.completed_permissions.clear();
        self.transcript_calls.clear();
    }
    fn record(&mut self, record: &Value) -> Option<Vec<AgentEvent>> {
        let data = record.get("data")?;
        if transcript_is_subagent(record, data) {
            return None;
        }
        let event = match record["type"].as_str()? {
            "tool.execution_start" => {
                let call = data["toolCallId"].as_str()?;
                let name = data["toolName"].as_str()?;
                let owner = self.matching_tool(name, data.get("arguments"), false)?;
                self.transcript_calls.insert(call.into(), owner);
                return None;
            }
            "tool.execution_complete" => {
                let call = data["toolCallId"].as_str()?;
                let owner = self.transcript_calls.remove(call)?;
                let tool = self.remove_tool_by_id(&owner)?;
                self.completed_tools.push(tool);
                AgentEvent::ToolEnded {
                    owner: Some(owner),
                    count: self.tools.len(),
                    interrupted: false,
                    transcript: true,
                }
            }
            "permission.completed" => {
                let call = data["toolCallId"].as_str()?;
                let owner = self.transcript_calls.get(call)?.clone();
                self.completed_permissions.insert(owner.clone());
                self.permission_owners.retain(|id| *id != owner);
                if let Some(tool) = self.tools.iter_mut().find(|tool| tool.id == owner) {
                    tool.prompted = canonical_tool_name(&tool.name) == "ask_user";
                }
                AgentEvent::InteractionClosed {
                    owner,
                    reason: Some(WaitReason::Approval),
                }
            }
            _ => return None,
        };
        Some(vec![event])
    }
    fn matching_tool(
        &self,
        name: &str,
        input: Option<&Value>,
        unique_name_fallback: bool,
    ) -> Option<String> {
        let name = canonical_tool_name(name);
        let candidates: Vec<_> = self
            .tools
            .iter()
            .filter(|tool| canonical_tool_name(&tool.name) == name)
            .collect();
        let matches: Vec<_> = candidates
            .iter()
            .filter(|tool| inputs_match(tool.input.as_ref(), input))
            .collect();
        if let Some(tool) = matches.first() {
            return Some(tool.id.clone());
        }
        (unique_name_fallback && candidates.len() == 1).then(|| candidates[0].id.clone())
    }

    fn remove_tool(&mut self, name: &str, input: Option<&Value>) -> Option<String> {
        let owner = self.matching_tool(name, input, true)?;
        self.remove_tool_by_id(&owner)?;
        Some(owner)
    }

    fn remove_tool_by_id(&mut self, owner: &str) -> Option<PendingTool> {
        let index = self.tools.iter().position(|tool| tool.id == owner)?;
        let tool = self.tools.remove(index);
        self.permission_owners
            .retain(|candidate| candidate != owner);
        self.completed_permissions.remove(owner);
        self.transcript_calls
            .retain(|_, candidate| candidate != owner);
        Some(tool)
    }

    fn consume_completed_tool(&mut self, name: &str, input: Option<&Value>) -> bool {
        let name = canonical_tool_name(name);
        let candidates: Vec<_> = self
            .completed_tools
            .iter()
            .enumerate()
            .filter(|(_, tool)| canonical_tool_name(&tool.name) == name)
            .map(|(index, _)| index)
            .collect();
        let matching: Vec<_> = candidates
            .iter()
            .copied()
            .filter(|index| inputs_match(self.completed_tools[*index].input.as_ref(), input))
            .collect();
        let index = matching.first().copied().or_else(|| {
            if input.is_none() && candidates.len() == 1 {
                Some(candidates[0])
            } else {
                None
            }
        });
        if let Some(index) = index {
            self.completed_tools.remove(index);
            true
        } else {
            false
        }
    }
}

pub(crate) struct CopilotStatusWatcher {
    feed: HookFeed,
    parser: CopilotParser,
    transcript: Option<crate::session::hook_feed::TranscriptTail>,
    copilot_home: PathBuf,
}
impl CopilotStatusWatcher {
    pub(crate) fn from_receiver(
        receiver: crate::session::hook_queue::HookReceiver,
        copilot_home: PathBuf,
    ) -> Self {
        Self {
            feed: HookFeed::from_receiver(receiver),
            parser: Default::default(),
            transcript: None,
            copilot_home,
        }
    }
    pub(crate) fn drain_events(
        &mut self,
        cancel: u8,
        mut emit: impl FnMut(AgentEvent) -> AdapterFeedback,
        mut session_start: impl FnMut(String),
    ) -> Result<()> {
        self.feed.drain(|report| {
            if let Ok(report) = serde_json::from_value::<StatusReport>(report) {
                if report.hook_event_name == "SessionStart" && report.agent_id.is_none() {
                    if let Some(id) = report.session_id.as_deref() {
                        if uuid::Uuid::parse_str(id).is_ok()
                            && self
                                .parser
                                .session_id
                                .as_deref()
                                .is_some_and(|current| current != id)
                        {
                            session_start(id.to_owned());
                        }
                    }
                }
                if let Some(events) = self.parser.hook(report) {
                    emit(AgentEvent::Batch {
                        runtime: Runtime::Copilot,
                        events,
                    });
                }
            }
        })?;
        if cancel != 0 {
            self.parser.compacting = false;
        }
        if cancel != 0 {
            emit(AgentEvent::Batch {
                runtime: Runtime::Copilot,
                events: vec![AgentEvent::LocalCancel { kind: cancel }],
            });
        }
        self.drain_transcript(&mut emit);
        Ok(())
    }
    fn drain_transcript(&mut self, emit: &mut impl FnMut(AgentEvent) -> AdapterFeedback) {
        if self.parser.tools.is_empty() {
            return;
        }
        let Some(session) = self.parser.session_id.as_deref() else {
            return;
        };
        let path = self.parser.transcript_path.clone().unwrap_or_else(|| {
            self.copilot_home
                .join("session-state")
                .join(session)
                .join("events.jsonl")
        });
        if self
            .transcript
            .as_ref()
            .is_none_or(|tail| tail.path != path)
        {
            self.transcript = crate::session::hook_feed::TranscriptTail::open(&path).ok();
        }
        let Some(tail) = self.transcript.as_mut() else {
            return;
        };
        while let Ok(count) = tail.reader.read_until(b'\n', &mut tail.pending) {
            if count == 0 || tail.pending.last() != Some(&b'\n') {
                break;
            }
            if let Ok(record) = serde_json::from_slice(&tail.pending) {
                if let Some(events) = self.parser.record(&record) {
                    emit(AgentEvent::Transcript {
                        runtime: Runtime::Copilot,
                        events,
                    });
                }
            }
            tail.pending.clear();
        }
    }
}
impl crate::session::hook_feed::HookWatcher for CopilotStatusWatcher {
    fn drain_events(
        &mut self,
        cancel: u8,
        emit: &mut dyn FnMut(AgentEvent) -> AdapterFeedback,
        session_start: &mut dyn FnMut(String),
    ) -> Result<()> {
        self.drain_events(cancel, emit, session_start)
    }
}

#[cfg(test)]
#[derive(Clone, Default)]
struct CopilotObservation {
    parser: CopilotParser,
    model: crate::session::state::agent::AgentModel,
    value: crate::session::state::agent::TurnState,
}
#[cfg(test)]
impl std::ops::Deref for CopilotObservation {
    type Target = CopilotParser;
    fn deref(&self) -> &Self::Target {
        &self.parser
    }
}
#[cfg(test)]
impl std::ops::DerefMut for CopilotObservation {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.parser
    }
}
#[cfg(test)]
impl CopilotObservation {
    fn observe(&mut self, event: StatusReport, now: i64) -> Option<AgentObservation> {
        let events = self.parser.hook(event)?;
        let reduced = self.model.reduce(
            AgentEvent::Batch {
                runtime: Runtime::Copilot,
                events,
            },
            now,
        );
        self.value = self.model.value.clone();
        reduced
    }
    fn observe_transcript(&mut self, record: &serde_json::Value) -> Option<AgentObservation> {
        let events = self.parser.record(record)?;
        let reduced = self.model.reduce(
            AgentEvent::Transcript {
                runtime: Runtime::Copilot,
                events,
            },
            crate::session::clock::timestamp_millis(),
        );
        self.value = self.model.value.clone();
        reduced
    }
    fn wait(&mut self, now: i64, reason: WaitReason, owners: Vec<String>) {
        self.model.reduce(
            AgentEvent::Batch {
                runtime: Runtime::Copilot,
                events: vec![AgentEvent::InteractionOpened { reason, owners }],
            },
            now,
        );
        self.value = self.model.value.clone();
    }
}
#[cfg(test)]
mod tests;
