use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

use super::claude_status::CTRL_C_INTERRUPT;
use super::hook_feed::{HookFeed, TranscriptTail};
use super::status::{
    Activity, AgentObservation, HumanInteraction, ObservationSource, TurnOutcome, WaitReason,
    WorkDetail,
};
use crate::error::Result;

pub(crate) const PATH_ENV: &str = "RUNNER_COPILOT_STATUS_PATH";
pub(crate) const GENERATION_ENV: &str = "RUNNER_COPILOT_STATUS_GENERATION";
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
const REPORTER: &str = "report.sh";
const REPORTER_SCRIPT: &str = r#"#!/bin/sh
if [ -z "$1" ]; then
  cat >/dev/null
  exit 0
fi
payload=$(mktemp "$1.XXXXXXXX") || { cat >/dev/null; exit 0; }
cat >"$payload" || { rm -f "$payload"; exit 0; }
printf '{"generation":"%s","hook_event_name":"%s","payload_file":"%s"}\n' "$RUNNER_COPILOT_STATUS_GENERATION" "$2" "${payload##*/}" >> "$1" 2>/dev/null || rm -f "$payload"
exit 0
"#;

pub(crate) fn plugin_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(PLUGIN_DIR)
}

pub(crate) fn reporter_path(app_data_dir: &Path) -> PathBuf {
    plugin_dir(app_data_dir).join(REPORTER)
}

fn hooks_path(app_data_dir: &Path) -> PathBuf {
    plugin_dir(app_data_dir).join("hooks/hooks.json")
}

pub(crate) fn plugin_available(app_data_dir: &Path) -> bool {
    [
        plugin_dir(app_data_dir).join("plugin.json"),
        reporter_path(app_data_dir),
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
fn powershell_command(event: &str) -> String {
    format!(
        "{};exit 0",
        super::hook_feed::powershell_reporter(
            &format!("$env:{PATH_ENV}"),
            GENERATION_ENV,
            &super::hook_feed::powershell_quote(event),
        )
    )
}

pub(crate) fn install_plugin(app_data_dir: &Path) -> Result<()> {
    let plugin_dir = plugin_dir(app_data_dir);
    fs::create_dir_all(plugin_dir.join("hooks"))?;
    write_plugin_file(
        &plugin_dir.join("plugin.json"),
        &serde_json::to_vec(&serde_json::json!({
            "name": "runner-status",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Runner session status hooks",
        }))?,
    )?;
    write_plugin_file(&reporter_path(app_data_dir), REPORTER_SCRIPT.as_bytes())?;

    let reporter =
        crate::session::launch::shell_quote(&reporter_path(app_data_dir).to_string_lossy());
    let mut hooks = serde_json::Map::new();
    for event in EVENTS {
        let command = format!("sh {reporter} \"${PATH_ENV}\" {event}");
        let mut entry = serde_json::json!({
            "hooks": [{
                "type": "command",
                "command": command,
                "powershell": powershell_command(event),
                "timeout": 2,
            }],
        });
        if *event == "Notification" {
            entry["matcher"] =
                Value::String("^(permission_prompt|elicitation_dialog|agent_idle)$".into());
        }
        hooks.insert((*event).into(), Value::Array(vec![entry]));
    }
    write_plugin_file(
        &hooks_path(app_data_dir),
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
    session_id: Option<String>,
    #[serde(alias = "notificationType")]
    notification_type: Option<String>,
    #[serde(alias = "toolName")]
    tool_name: Option<String>,
    #[serde(alias = "toolInput")]
    tool_input: Option<Value>,
    transcript_path: Option<PathBuf>,
    stop_reason: Option<String>,
    #[serde(alias = "agentId")]
    agent_id: Option<String>,
}

struct PendingTool {
    id: String,
    name: String,
    input: Option<Value>,
}

struct CompactionResume {
    working: bool,
    outcome: Option<TurnOutcome>,
}

#[derive(Default)]
struct CopilotObservation {
    value: AgentObservation,
    session_id: Option<String>,
    ended: bool,
    transcript_path: Option<PathBuf>,
    tools: Vec<PendingTool>,
    completed_tools: Vec<PendingTool>,
    compacting: bool,
    compaction_resume: Option<CompactionResume>,
    permission_owners: Vec<String>,
    transcript_calls: BTreeMap<String, String>,
    next_tool: u64,
    next_interaction: u64,
}

impl CopilotObservation {
    fn observe(&mut self, report: StatusReport) -> Option<AgentObservation> {
        if report.agent_id.is_some() || report.hook_event_name.starts_with("Subagent") {
            return None;
        }
        let session_id = report.session_id.filter(|id| !id.is_empty())?;
        if report.hook_event_name == "SessionStart" {
            if report.source.as_deref() == Some("compact")
                && self.compacting
                && self
                    .session_id
                    .as_ref()
                    .is_none_or(|current| current == &session_id)
            {
                self.session_id = Some(session_id);
                if let Some(path) = report.transcript_path {
                    self.transcript_path = Some(path);
                }
                return None;
            }
            if self.session_id.as_ref() == Some(&session_id)
                && self.value.source == ObservationSource::Hook
            {
                return None;
            }
            let owned = self.value.source == ObservationSource::Hook;
            self.session_id = Some(session_id);
            self.ended = false;
            self.clear_turn();
            self.value.activity = Activity::Unavailable;
            self.value.outcome = None;
            return owned.then(|| self.value.clone());
        }
        if self
            .session_id
            .as_ref()
            .is_some_and(|current| *current != session_id)
        {
            return None;
        }
        self.session_id.get_or_insert(session_id);
        if self.ended {
            return None;
        }
        if let Some(path) = report.transcript_path {
            self.transcript_path = Some(path);
        }
        match report.hook_event_name.as_str() {
            "SessionEnd" => {
                self.ended = true;
                self.clear_turn();
                self.value.activity = Activity::Unavailable;
                self.update_detail();
            }
            "UserPromptSubmit" => {
                self.clear_turn();
                self.work();
            }
            "PreToolUse" => {
                let name = report.tool_name?;
                self.finish_compaction();
                self.work();
                self.next_tool += 1;
                let owner = format!("copilot-tool-{}", self.next_tool);
                let question = canonical_tool_name(&name) == "ask_user";
                self.tools.push(PendingTool {
                    id: owner.clone(),
                    name,
                    input: report.tool_input,
                });
                self.update_detail();
                if question {
                    self.wait(WaitReason::Answer, vec![owner]);
                }
            }
            "PermissionRequest" => {
                if self.value.activity != Activity::Working || self.value.outcome.is_some() {
                    return None;
                }
                let owner = self.matching_tool(
                    report.tool_name.as_deref()?,
                    report.tool_input.as_ref(),
                    true,
                )?;
                if !self.permission_owners.contains(&owner) {
                    self.permission_owners.push(owner);
                }
                return None;
            }
            "Notification" => match report.notification_type.as_deref() {
                Some("permission_prompt") => {
                    if self.value.activity != Activity::Working {
                        return None;
                    }
                    let owners: Vec<_> = self
                        .permission_owners
                        .iter()
                        .filter(|owner| {
                            self.tools
                                .iter()
                                .any(|tool| tool.id.as_str() == owner.as_str())
                        })
                        .cloned()
                        .collect();
                    if owners.is_empty() || !self.wait(WaitReason::Approval, owners) {
                        return None;
                    }
                }
                Some("elicitation_dialog") => {
                    let owners: Vec<_> = self
                        .tools
                        .iter()
                        .filter(|tool| canonical_tool_name(&tool.name) == "ask_user")
                        .map(|tool| tool.id.clone())
                        .collect();
                    if owners.is_empty() || !self.wait(WaitReason::Answer, owners) {
                        return None;
                    }
                }
                Some("agent_idle") if !self.value.needs_you() => {
                    self.finish_compaction();
                    self.value.activity = Activity::Ready;
                    self.update_detail();
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
                    None => return None,
                };
                let compaction_was_working = self.finish_compaction();
                if let Some(owner) = owner {
                    self.resolve(&owner);
                }
                if self.value.outcome == Some(TurnOutcome::Interrupted) {
                    if !self.value.needs_you() {
                        self.value.activity = Activity::Ready;
                    }
                } else if compaction_was_working != Some(false) {
                    self.work();
                }
                self.update_detail();
            }
            "PreCompact" => {
                if !self.compacting {
                    self.compaction_resume = Some(CompactionResume {
                        working: self.value.activity == Activity::Working
                            && self.value.outcome.is_none(),
                        outcome: self.value.outcome,
                    });
                }
                self.work();
                self.compacting = true;
                self.update_detail();
            }
            "Stop" if report.stop_reason.as_deref() == Some("end_turn") => {
                let manual_compaction_outcome = self
                    .compaction_resume
                    .as_ref()
                    .filter(|resume| !resume.working)
                    .map(|resume| resume.outcome);
                self.clear_turn();
                self.value.activity = Activity::Ready;
                if let Some(outcome) = manual_compaction_outcome {
                    self.value.outcome = outcome;
                } else if self.value.outcome != Some(TurnOutcome::Interrupted) {
                    self.value.outcome = Some(TurnOutcome::Completed);
                }
            }
            "ErrorOccurred" => return None,
            _ => return None,
        }
        self.value.source = ObservationSource::Hook;
        Some(self.value.clone())
    }

    fn observe_transcript(&mut self, record: &Value) -> Option<AgentObservation> {
        let data = record.get("data")?;
        if transcript_is_subagent(record, data) {
            return None;
        }
        let before = self.value.clone();
        match record.get("type").and_then(Value::as_str) {
            Some("tool.execution_start") => {
                let call_id = data.get("toolCallId")?.as_str()?;
                let name = data.get("toolName")?.as_str()?;
                let arguments = data.get("arguments");
                let owner = self.matching_tool(name, arguments, false)?;
                self.transcript_calls.insert(call_id.into(), owner);
            }
            Some("tool.execution_complete") => {
                let call_id = data.get("toolCallId")?.as_str()?;
                let owner = self.transcript_calls.remove(call_id)?;
                let tool = self.remove_tool_by_id(&owner)?;
                self.completed_tools.push(tool);
                self.resolve(&owner);
                if self.value.outcome == Some(TurnOutcome::Interrupted) && !self.value.needs_you() {
                    self.value.activity = Activity::Ready;
                }
                self.update_detail();
            }
            _ => return None,
        }
        (before != self.value).then(|| self.value.clone())
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

    fn work(&mut self) {
        self.value.activity = Activity::Working;
        self.value.outcome = None;
        self.update_detail();
    }

    fn clear_turn(&mut self) {
        self.value.interactions.clear();
        self.tools.clear();
        self.completed_tools.clear();
        self.compacting = false;
        self.compaction_resume = None;
        self.permission_owners.clear();
        self.transcript_calls.clear();
        self.value.detail = None;
    }

    fn update_detail(&mut self) {
        self.value.detail =
            if self.value.activity != Activity::Working || self.value.outcome.is_some() {
                None
            } else if self.compacting {
                Some(WorkDetail::CompactingContext)
            } else if !self.tools.is_empty() {
                Some(WorkDetail::UsingTools)
            } else {
                None
            };
    }

    fn finish_compaction(&mut self) -> Option<bool> {
        if !self.compacting {
            return None;
        }
        self.compacting = false;
        let resume = self.compaction_resume.take()?;
        if resume.working {
            self.value.activity = Activity::Working;
            self.value.outcome = None;
        } else {
            self.value.activity = Activity::Ready;
            self.value.outcome = resume.outcome;
        }
        self.update_detail();
        Some(resume.working)
    }

    fn resolve(&mut self, owner: &str) {
        self.value.interactions.retain_mut(|wait| {
            wait.owners.retain(|candidate| candidate != owner);
            !wait.owners.is_empty()
        });
    }

    fn wait(&mut self, reason: WaitReason, owners: Vec<String>) -> bool {
        if let Some(wait) = self.value.interactions.iter_mut().find(|wait| {
            wait.reason == reason
                && (wait.owners == owners || wait.owners.iter().any(|id| owners.contains(id)))
        }) {
            let before = wait.owners.len();
            for owner in owners {
                if !wait.owners.contains(&owner) {
                    wait.owners.push(owner);
                }
            }
            return wait.owners.len() != before;
        }
        self.next_interaction += 1;
        self.value.interactions.push(HumanInteraction {
            id: format!("copilot-{}", self.next_interaction),
            reason,
            owners,
            since: chrono::Utc::now().timestamp_millis(),
        });
        true
    }
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
        "bash" | "shell" => "bash".into(),
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

pub(crate) struct CopilotStatusWatcher {
    feed: HookFeed,
    observation: CopilotObservation,
    interrupt: Arc<AtomicU8>,
    transcript: Option<TranscriptTail>,
    copilot_home: PathBuf,
}

impl CopilotStatusWatcher {
    pub(crate) fn start(path: &Path, generation: String, copilot_home: PathBuf) -> Result<Self> {
        let app_data_dir = path
            .parent()
            .and_then(Path::parent)
            .expect("status file is under app data");
        Ok(Self {
            feed: HookFeed::start_external(path, generation, &reporter_path(app_data_dir))?,
            observation: CopilotObservation::default(),
            interrupt: Arc::new(AtomicU8::new(0)),
            transcript: None,
            copilot_home,
        })
    }

    pub(crate) fn interrupt_signal(&self) -> Arc<AtomicU8> {
        Arc::clone(&self.interrupt)
    }

    pub(crate) fn drain_observations(
        &mut self,
        mut transition: impl FnMut(AgentObservation, &'static str),
    ) -> Result<()> {
        let interrupted = self.interrupt.swap(0, Ordering::AcqRel);
        self.feed.drain(interrupted != 0, |report| {
            if let Ok(report) = serde_json::from_value(report) {
                if let Some(value) = self.observation.observe(report) {
                    transition(value, "hook");
                }
            }
        })?;
        if interrupted != 0
            && self.observation.value.source == ObservationSource::Hook
            && (self.observation.value.activity == Activity::Working
                || self.observation.value.needs_you())
        {
            let source = if interrupted & CTRL_C_INTERRUPT != 0 {
                "input-interrupt"
            } else {
                "input-escape"
            };
            self.observation.value.activity = if self.observation.value.needs_you() {
                Activity::Unavailable
            } else {
                Activity::Ready
            };
            self.observation.value.outcome = Some(TurnOutcome::Interrupted);
            self.observation.compacting = false;
            self.observation.compaction_resume = None;
            self.observation.update_detail();
            transition(self.observation.value.clone(), source);
        }
        self.drain_transcript(&mut transition);
        Ok(())
    }

    fn drain_transcript(&mut self, transition: &mut impl FnMut(AgentObservation, &'static str)) {
        if !self.observation.value.needs_you() && self.observation.tools.is_empty() {
            return;
        }
        let Some(session_id) = self.observation.session_id.as_deref() else {
            return;
        };
        let path = self.observation.transcript_path.clone().unwrap_or_else(|| {
            self.copilot_home
                .join("session-state")
                .join(session_id)
                .join("events.jsonl")
        });
        if self
            .transcript
            .as_ref()
            .is_none_or(|tail| tail.path != path)
        {
            self.transcript = TranscriptTail::open(&path).ok();
        }
        let Some(tail) = self.transcript.as_mut() else {
            return;
        };
        while let Ok(count) = tail.reader.read_until(b'\n', &mut tail.pending) {
            if count == 0 || tail.pending.last() != Some(&b'\n') {
                break;
            }
            if let Ok(record) = serde_json::from_slice(&tail.pending) {
                if let Some(value) = self.observation.observe_transcript(&record) {
                    transition(value, "hook");
                }
            }
            tail.pending.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    #[cfg(unix)]
    use std::process::{Command, Stdio};
    use std::sync::atomic::Ordering;

    use serde_json::json;

    use super::*;

    fn report(event: &str) -> Value {
        json!({
            "hook_event_name": event,
            "session_id": "main",
        })
    }

    fn observe(state: &mut CopilotObservation, value: Value) -> Option<AgentObservation> {
        state.observe(serde_json::from_value(value).unwrap())
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
    fn plugin_generation_has_the_exact_event_catalog_and_reporter_commands() {
        let root = tempfile::tempdir().unwrap();
        let app_data = root.path().join("runner app data");
        install_plugin(&app_data).unwrap();
        assert!(plugin_available(&app_data));
        fs::write(reporter_path(&app_data), "broken").unwrap();
        fs::write(hooks_path(&app_data), "{}").unwrap();
        install_plugin(&app_data).unwrap();
        assert_eq!(
            fs::read_to_string(reporter_path(&app_data)).unwrap(),
            REPORTER_SCRIPT
        );
        let plugin: Value =
            serde_json::from_slice(&fs::read(plugin_dir(&app_data).join("plugin.json")).unwrap())
                .unwrap();
        assert_eq!(plugin["name"], "runner-status");
        assert_eq!(plugin["version"], env!("CARGO_PKG_VERSION"));
        let hooks: Value =
            serde_json::from_slice(&fs::read(hooks_path(&app_data)).unwrap()).unwrap();
        assert_eq!(hooks["version"], 1);
        assert_eq!(hooks["hooks"].as_object().unwrap().len(), EVENTS.len());
        for event in EVENTS {
            let entry = &hooks["hooks"][event][0];
            assert_eq!(entry["hooks"].as_array().unwrap().len(), 1);
            assert_eq!(entry["hooks"][0]["type"], "command");
            assert_eq!(entry["hooks"][0]["timeout"], 2);
            assert_eq!(
                entry["hooks"][0]["command"],
                format!(
                    "sh {} \"${PATH_ENV}\" {event}",
                    crate::session::launch::shell_quote(
                        &reporter_path(&app_data).to_string_lossy()
                    )
                )
            );
            let powershell = entry["hooks"][0]["powershell"].as_str().unwrap();
            assert_eq!(powershell, powershell_command(event));
            assert!(powershell.starts_with(&format!(
                "$ErrorActionPreference='Stop';$f=$env:{PATH_ENV};$g=$env:{GENERATION_ENV};"
            )));
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
        fs::write(reporter_path(&incomplete), REPORTER_SCRIPT).unwrap();
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
        let after_hook =
            observe(&mut state, post_tool("Bash", json!({"command":"echo one"}))).unwrap();
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
        state.value.activity = Activity::Unavailable;
        state.value.outcome = Some(TurnOutcome::Interrupted);
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
        install_plugin(root.path()).unwrap();
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
        let path = super::super::hook_feed::status_path(root.path(), "session");
        let mut watcher = CopilotStatusWatcher::start(&path, "current".into(), home).unwrap();
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        for value in [
            report("UserPromptSubmit"),
            pre_tool("AskUserQuestion", json!({"message":"Choose"})),
        ] {
            let mut value = value;
            value["generation"] = json!("current");
            writeln!(file, "{value}").unwrap();
        }
        watcher.feed.dirty.store(true, Ordering::Release);
        watcher.drain_observations(|_, _| {}).unwrap();
        assert!(watcher.observation.value.needs_you());
        watcher.interrupt.store(
            super::super::claude_status::ESCAPE_INTERRUPT,
            Ordering::Release,
        );
        watcher.drain_observations(|_, _| {}).unwrap();
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
        watcher.drain_observations(|_, _| {}).unwrap();
        assert_eq!(watcher.observation.value.activity, Activity::Ready);
        assert_eq!(
            watcher.observation.value.outcome,
            Some(TurnOutcome::Interrupted)
        );
        assert!(!watcher.observation.value.needs_you());
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
    fn malformed_partial_generation_and_session_reports_are_isolated() {
        let root = tempfile::tempdir().unwrap();
        install_plugin(root.path()).unwrap();
        let home = root.path().join("copilot-home");
        let path = super::super::hook_feed::status_path(root.path(), "session");
        let mut watcher = CopilotStatusWatcher::start(&path, "current".into(), home).unwrap();
        fs::write(&path, "not json\n").unwrap();
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "generation":"old",
                "hook_event_name":"UserPromptSubmit",
                "session_id":"main",
            })
        )
        .unwrap();
        let current = format!(
            "{}\n",
            json!({
                "generation":"current",
                "hook_event_name":"UserPromptSubmit",
                "session_id":"main",
            })
        );
        file.write_all(&current.as_bytes()[..20]).unwrap();
        watcher.feed.dirty.store(true, Ordering::Release);
        watcher
            .drain_observations(|_, _| panic!("partial report"))
            .unwrap();
        file.write_all(&current.as_bytes()[20..]).unwrap();
        let foreign = json!({
            "generation":"current",
            "hook_event_name":"Stop",
            "session_id":"foreign",
            "stop_reason":"end_turn",
        });
        writeln!(file, "{foreign}").unwrap();
        watcher.feed.dirty.store(true, Ordering::Release);
        let mut values = Vec::new();
        watcher
            .drain_observations(|value, _| values.push(value))
            .unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].activity, Activity::Working);
    }

    #[cfg(unix)]
    fn run_reporter(
        reporter: &Path,
        path: &Path,
        event: &str,
        payload: &[u8],
    ) -> std::process::Output {
        let mut child = Command::new("sh")
            .arg(reporter)
            .arg(path)
            .arg(event)
            .env(GENERATION_ENV, "current")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(payload).unwrap();
        child.wait_with_output().unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn reporter_drains_missing_path_handles_spaces_large_payload_and_teardown() {
        let root = tempfile::tempdir().unwrap();
        let app_data = root.path().join("runner app data");
        install_plugin(&app_data).unwrap();
        let reporter = reporter_path(&app_data);
        let missing = run_reporter(&reporter, Path::new(""), "Stop", &vec![b'x'; 256 * 1024]);
        assert!(missing.status.success());
        assert!(missing.stdout.is_empty());
        assert!(missing.stderr.is_empty());

        let path = super::super::hook_feed::status_path(&app_data, "session with spaces");
        let mut watcher =
            CopilotStatusWatcher::start(&path, "current".into(), root.path().join("copilot-home"))
                .unwrap();
        let mut payload = report("UserPromptSubmit");
        payload["prompt"] = json!("x\n".repeat(128 * 1024));
        let output = run_reporter(
            &reporter,
            &path,
            "UserPromptSubmit",
            &serde_json::to_vec_pretty(&payload).unwrap(),
        );
        assert!(output.status.success());
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
        let mut values = Vec::new();
        watcher
            .drain_observations(|value, _| values.push(value))
            .unwrap();
        assert_eq!(values.last().unwrap().activity, Activity::Working);

        fs::remove_file(&reporter).unwrap();
        watcher.feed.dirty.store(true, Ordering::Release);
        assert!(watcher.drain_observations(|_, _| {}).is_err());
        drop(watcher);
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 0);
    }

    #[test]
    fn disabled_hooks_leave_the_baseline_unlatched() {
        let root = tempfile::tempdir().unwrap();
        install_plugin(root.path()).unwrap();
        let path = super::super::hook_feed::status_path(root.path(), "disabled");
        let mut watcher =
            CopilotStatusWatcher::start(&path, "current".into(), root.path().join("copilot-home"))
                .unwrap();
        let mut transitions = 0;
        watcher.drain_observations(|_, _| transitions += 1).unwrap();
        assert_eq!(transitions, 0);
        assert_eq!(
            watcher.observation.value.source,
            ObservationSource::Unavailable
        );
    }

    #[cfg(windows)]
    #[test]
    fn powershell_entry_drains_missing_path_handles_spaces_large_payload_and_teardown() {
        use super::super::hook_feed::{hook_path, run_powershell, POWERSHELLS};
        for shell in POWERSHELLS {
            let root = tempfile::tempdir().unwrap();
            let app_data = root.path().join("Jason's runner app data");
            install_plugin(&app_data).unwrap();
            let hooks: Value =
                serde_json::from_slice(&fs::read(hooks_path(&app_data)).unwrap()).unwrap();
            let command = |event: &str| {
                hooks["hooks"][event][0]["hooks"][0]["powershell"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            };
            let Some(missing) = run_powershell(
                shell,
                &command("Stop"),
                &[(PATH_ENV, ""), (GENERATION_ENV, "current")],
                &vec![b'x'; 256 * 1024],
            ) else {
                continue;
            };
            assert!(missing.status.success(), "{shell}: {missing:?}");
            assert!(missing.stdout.is_empty() && missing.stderr.is_empty());

            let path = super::super::hook_feed::status_path(&app_data, "session with spaces");
            let path = PathBuf::from(hook_path(&path));
            let mut watcher = CopilotStatusWatcher::start(
                &path,
                "current".into(),
                root.path().join("copilot-home"),
            )
            .unwrap();
            let mut payload = report("UserPromptSubmit");
            payload["prompt"] = json!(format!("你好 {}", "x\n".repeat(128 * 1024)));
            let env = [
                (PATH_ENV, hook_path(&path)),
                (GENERATION_ENV, "current".into()),
            ];
            let env = env
                .iter()
                .map(|(k, v)| (*k, v.as_str()))
                .collect::<Vec<_>>();
            let output = run_powershell(
                shell,
                &command("UserPromptSubmit"),
                &env,
                &serde_json::to_vec_pretty(&payload).unwrap(),
            )
            .unwrap();
            assert!(output.status.success(), "{shell}: {output:?}");
            assert!(output.stdout.is_empty() && output.stderr.is_empty());
            let mut values = Vec::new();
            watcher
                .drain_observations(|value, _| values.push(value))
                .unwrap();
            assert_eq!(values.last().unwrap().activity, Activity::Working);

            fs::remove_file(reporter_path(&app_data)).unwrap();
            watcher.feed.dirty.store(true, Ordering::Release);
            assert!(watcher.drain_observations(|_, _| {}).is_err());
            drop(watcher);
            assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 0);
            let late = run_powershell(shell, &command("Stop"), &env, b"{}").unwrap();
            assert!(late.status.success());
            assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 0);
        }
    }
}
