use crate::model::Runtime;
use crate::session::state::agent::{AdapterFeedback, AgentEvent};
#[cfg(test)]
use crate::session::state::StatusSource;
#[cfg(test)]
use crate::session::state::CTRL_C_INTERRUPT;
#[cfg(test)]
use crate::session::state::ESCAPE_INTERRUPT;
#[cfg(test)]
use std::fs::{self, File};
use std::io::BufRead;
use std::path::PathBuf;

use serde::Deserialize;

use crate::error::Result;

pub(crate) use crate::session::hook_feed::clear_leftovers;
use crate::session::hook_feed::HookFeed;
#[cfg(test)]
use crate::session::runtime::SessionActivityState;
use crate::session::status::{TurnOutcome, WaitReason};

#[cfg(test)]
use crate::session::status::{Activity, AgentObservation, ObservationSource, WorkDetail};

// Runner-owned per-invocation helper follows cmux's hook bridge shape
// (manaflow-ai/cmux, GPL-3.0-or-later); the status records are Runner's.

#[derive(Debug, Default, Deserialize)]
struct StatusReport {
    #[serde(default)]
    hook_event_name: String,
    notification_type: Option<String>,
    source: Option<String>,
    trigger: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) transcript_path: Option<PathBuf>,
    pub(crate) prompt_id: Option<String>,
    prompt: Option<String>,
    agent_id: Option<String>,
    tool_use_id: Option<String>,
    tool_name: Option<String>,
    tool_input: Option<serde_json::Value>,
    #[serde(default)]
    is_interrupt: bool,
    elicitation_id: Option<String>,
    mcp_server_name: Option<String>,
    background_tasks: Option<serde_json::Value>,
    session_crons: Option<serde_json::Value>,
}

/// The `background_tasks` labels Claude Code documents for Stop input; other
/// values are raw discriminants of types it does not name.
const BACKGROUND_TASK_TYPES: [&str; 7] = [
    "shell",
    "subagent",
    "monitor",
    "workflow",
    "teammate",
    "cloud session",
    "MCP task",
];

/// Whether a Stop leaves work the agent will wake from: a documented background
/// task or any scheduled wakeup (#1183).
fn background_pending(
    tasks: Option<&serde_json::Value>,
    crons: Option<&serde_json::Value>,
) -> bool {
    let tasks = tasks.and_then(serde_json::Value::as_array);
    let crons = crons.and_then(serde_json::Value::as_array);
    tasks.is_some_and(|tasks| {
        tasks.iter().any(|task| {
            task["type"]
                .as_str()
                .is_some_and(|kind| BACKGROUND_TASK_TYPES.contains(&kind))
        })
    }) || crons.is_some_and(|crons| !crons.is_empty())
}

#[cfg(test)]
fn parse_transition(line: &[u8], generation: &str) -> Option<SessionActivityState> {
    let report: serde_json::Value = serde_json::from_slice(line).ok()?;
    if report.get("generation")?.as_str()? != generation {
        return None;
    }
    let value = ClaudeObservation::default().observe(
        serde_json::from_value::<StatusReport>(report).ok()?,
        crate::session::clock::timestamp_millis(),
    )?;
    if value.needs_you() {
        return None;
    }
    Some(if value.activity == Activity::Working {
        SessionActivityState::Busy
    } else {
        SessionActivityState::Idle
    })
}

#[derive(Clone)]
struct PendingTool {
    name: String,
    input: Option<serde_json::Value>,
    prompted: bool,
}
#[derive(Clone)]
struct PendingElicitation {
    id: Option<String>,
    server: String,
    prompted: bool,
}
/// Claude Code fires `PreCompact` before a compaction can fail, be skipped or be blocked, and
/// then sends no end. Of 64 compactions measured on Claude Code 2.1.3xx the longest took 133 s,
/// so ten minutes leaves room for API retries before the prior status comes back (#1224).
const COMPACTION_TIMEOUT_MS: i64 = 10 * 60 * 1000;

#[derive(Clone, Default)]
struct ClaudeParser {
    session_id: Option<String>,
    prompt_id: Option<String>,
    transcript_path: Option<PathBuf>,
    tools: std::collections::BTreeMap<String, PendingTool>,
    compacting: bool,
    compaction_started: Option<i64>,
    elicitations: Vec<PendingElicitation>,
}
impl ClaudeParser {
    /// `None` drops the hook; an empty list admits it without a status change.
    fn hook(&mut self, report: StatusReport) -> Option<Vec<AgentEvent>> {
        if report.agent_id.is_some() || report.hook_event_name.starts_with("Subagent") {
            return None;
        }
        if report.hook_event_name == "SessionStart" {
            // Claude sends `SessionStart(compact)` before `PostCompact` on some paths and after
            // it on others; either way it continues the conversation the compaction ran in.
            if report.source.as_deref() == Some("compact")
                && report
                    .session_id
                    .as_ref()
                    .is_none_or(|id| self.session_id.as_ref().is_none_or(|current| current == id))
            {
                if let Some(id) = report.session_id {
                    self.session_id = Some(id);
                }
                if let Some(path) = report.transcript_path {
                    self.transcript_path = Some(path);
                }
                return Some(Vec::new());
            }
            self.session_id = report.session_id;
            self.transcript_path = report.transcript_path;
            self.prompt_id = None;
            self.clear();
            return Some(vec![AgentEvent::StartupReady]);
        }
        if self.session_id.is_some()
            && report.session_id.is_some()
            && self.session_id != report.session_id
        {
            return None;
        }
        if report.hook_event_name == "UserPromptSubmit" {
            self.prompt_id = report.prompt_id.clone();
        } else if self.prompt_id.is_some()
            && report.prompt_id.is_some()
            && self.prompt_id != report.prompt_id
            // `/compact` runs under a prompt id of its own that no UserPromptSubmit announces.
            && !matches!(report.hook_event_name.as_str(), "PreCompact" | "PostCompact")
        {
            return None;
        }
        if let Some(path) = report.transcript_path {
            self.transcript_path = Some(path);
        }
        let prompt = if report.hook_event_name == "UserPromptSubmit" {
            report.prompt
        } else {
            None
        };
        let event = match report.hook_event_name.as_str() {
            "UserPromptSubmit" => {
                self.clear();
                AgentEvent::TurnStarted
            }
            "PreToolUse" => {
                let question =
                    if let (Some(id), Some(name)) = (report.tool_use_id, report.tool_name) {
                        let wait = match name.as_str() {
                            "AskUserQuestion" => Some(WaitReason::Answer),
                            "ExitPlanMode" => Some(WaitReason::Approval),
                            _ => None,
                        };
                        let prompted =
                            wait.is_some() || self.tools.get(&id).is_some_and(|tool| tool.prompted);
                        self.tools.insert(
                            id.clone(),
                            PendingTool {
                                name,
                                input: report.tool_input,
                                prompted,
                            },
                        );
                        wait.map(|reason| AgentEvent::InteractionOpened {
                            reason,
                            owners: vec![id],
                        })
                    } else {
                        None
                    };
                let mut events = vec![AgentEvent::ToolStarted {
                    count: self.tools.len(),
                    question: None,
                }];
                if let Some(question) = question {
                    events.push(question);
                }
                return Some(events);
            }
            "PreCompact" => AgentEvent::CompactionStarted,
            "PostCompact" => AgentEvent::CompactionEnded,
            "PermissionRequest" => {
                let name = report.tool_name?;
                let owners: Vec<_> = self
                    .tools
                    .iter()
                    .filter_map(|(id, tool)| {
                        let matches = if let Some(request) = report.tool_use_id.as_ref() {
                            id == request
                        } else {
                            tool.name == name && tool.input == report.tool_input
                        };
                        matches.then(|| id.clone())
                    })
                    .collect();
                AgentEvent::PermissionRequested {
                    reason: if name == "AskUserQuestion" {
                        WaitReason::Answer
                    } else {
                        WaitReason::Approval
                    },
                    owners,
                }
            }
            "Elicitation" => {
                let server = report.mcp_server_name?;
                let prompted = report.elicitation_id.as_ref().is_some_and(|id| {
                    self.elicitations
                        .iter()
                        .any(|pending| pending.id.as_ref() == Some(id) && pending.prompted)
                });
                if let Some(id) = report.elicitation_id.as_ref() {
                    self.elicitations
                        .retain(|current| current.id.as_ref() != Some(id));
                }
                self.elicitations.push(PendingElicitation {
                    id: report.elicitation_id.clone(),
                    server: server.clone(),
                    prompted,
                });
                AgentEvent::ElicitationRequested {
                    id: report.elicitation_id,
                    server,
                }
            }
            "ElicitationResult" => {
                let matches: Vec<_> = self
                    .elicitations
                    .iter()
                    .enumerate()
                    .filter(|(_, pending)| {
                        if let Some(request) = report.elicitation_id.as_ref() {
                            pending.id.as_ref() == Some(request)
                        } else {
                            report.mcp_server_name.as_ref() == Some(&pending.server)
                        }
                    })
                    .map(|(index, _)| index)
                    .collect();
                if matches.len() == 1 {
                    self.elicitations.remove(matches[0]);
                }
                AgentEvent::ElicitationClosed {
                    id: report.elicitation_id,
                    server: report.mcp_server_name,
                }
            }
            "PostToolUse" | "PostToolUseFailure" | "PermissionDenied" => {
                if let Some(id) = report.tool_use_id.as_ref() {
                    self.tools.remove(id);
                }
                AgentEvent::ToolEnded {
                    owner: report.tool_use_id,
                    count: self.tools.len(),
                    interrupted: report.is_interrupt,
                    transcript: false,
                }
            }
            "Notification" => match report.notification_type.as_deref() {
                Some("permission_prompt") => AgentEvent::PermissionPrompt {
                    candidates: self
                        .tools
                        .iter()
                        .map(|(id, tool)| {
                            (
                                id.clone(),
                                match tool.name.as_str() {
                                    "AskUserQuestion" => WaitReason::Answer,
                                    "ExitPlanMode" | "EnterPlanMode" => WaitReason::Approval,
                                    _ => WaitReason::Unknown,
                                },
                            )
                        })
                        .collect(),
                },
                Some("elicitation_dialog") => AgentEvent::ElicitationPrompt { owners: Vec::new() },
                Some("idle_prompt") => AgentEvent::IdlePrompt,
                _ => return None,
            },
            "Stop" => {
                self.clear();
                return Some(vec![
                    AgentEvent::TurnEnded {
                        outcome: TurnOutcome::Completed,
                    },
                    AgentEvent::BackgroundWork {
                        pending: background_pending(
                            report.background_tasks.as_ref(),
                            report.session_crons.as_ref(),
                        ),
                    },
                ]);
            }
            "StopFailure" => {
                self.clear();
                AgentEvent::TurnEnded {
                    outcome: TurnOutcome::Failed,
                }
            }
            _ => return None,
        };
        Some(match prompt {
            Some(prompt) => vec![AgentEvent::PromptSubmitted { prompt }, event],
            None => vec![event],
        })
    }
    fn clear(&mut self) {
        self.tools.clear();
        self.compacting = false;
        self.compaction_started = None;
        self.elicitations.clear();
    }
    fn accept(&mut self, events: &[AgentEvent], feedback: AdapterFeedback, now: i64) {
        self.note_compaction(feedback.compacting, events, now);
        if !feedback.accepted {
            return;
        }
        for event in events {
            match event {
                AgentEvent::PermissionRequested { owners, .. } => {
                    for owner in owners {
                        if let Some(tool) = self.tools.get_mut(owner) {
                            tool.prompted = true;
                        }
                    }
                }
                AgentEvent::PermissionPrompt { .. } => {
                    for tool in self.tools.values_mut() {
                        tool.prompted |= matches!(
                            tool.name.as_str(),
                            "AskUserQuestion" | "ExitPlanMode" | "EnterPlanMode"
                        );
                    }
                }
                AgentEvent::ElicitationPrompt { .. } => {
                    for pending in &mut self.elicitations {
                        pending.prompted = true;
                    }
                }
                AgentEvent::ToolEnded {
                    owner: Some(owner), ..
                } => {
                    for pending in &mut self.elicitations {
                        if pending.id.as_ref() == Some(owner) {
                            pending.prompted = false;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    fn note_compaction(&mut self, compacting: bool, events: &[AgentEvent], now: i64) {
        self.compacting = compacting;
        if !compacting {
            self.compaction_started = None;
        } else if self.compaction_started.is_none()
            || events
                .iter()
                .any(|event| matches!(event, AgentEvent::CompactionStarted))
        {
            self.compaction_started = Some(now);
        }
    }
    fn compaction_expired(&self, now: i64) -> bool {
        self.compaction_started
            .is_some_and(|started| now - started >= COMPACTION_TIMEOUT_MS)
    }
    /// The boundary of a finished compaction, or the output of a `/compact` that did not
    /// compact, made after the current compaction's `PreCompact` was admitted. Claude waits
    /// for that admission before continuing, so earlier records belong to earlier attempts.
    fn ends_compaction(&self, entry: &serde_json::Value) -> bool {
        let Some(started) = self.compaction_started else {
            return false;
        };
        (entry["subtype"] == "compact_boundary"
            || (entry["subtype"] == "local_command"
                && entry.pointer("/commandRun/command") == Some(&"compact".into())))
            && entry["timestamp"]
                .as_str()
                .and_then(|time| chrono::DateTime::parse_from_rfc3339(time).ok())
                .is_some_and(|time| time.timestamp_millis() >= started)
    }
    fn record(&mut self, entry: &serde_json::Value) -> Option<Vec<AgentEvent>> {
        let session = self.session_id.as_deref()?;
        if entry["sessionId"].as_str() != Some(session)
            || entry["isSidechain"].as_bool() == Some(true)
            || entry["promptId"].as_str().is_some_and(|id| {
                self.prompt_id
                    .as_deref()
                    .is_some_and(|current| current != id)
            })
        {
            return None;
        }
        let mut events = Vec::new();
        if entry["type"] == "user" {
            if let Some(blocks) = entry
                .pointer("/message/content")
                .and_then(serde_json::Value::as_array)
            {
                for block in blocks {
                    if block["type"] != "tool_result" {
                        continue;
                    }
                    let Some(id) = block["tool_use_id"].as_str() else {
                        continue;
                    };
                    let tool = self.tools.remove(id);
                    let mut prompted = false;
                    for pending in &mut self.elicitations {
                        if pending.id.as_deref() == Some(id) {
                            prompted |= pending.prompted;
                            pending.prompted = false;
                        }
                    }
                    if tool.is_none() && !prompted {
                        continue;
                    }
                    let rejected = entry["toolDenialKind"] == "user-rejected";
                    events.push(AgentEvent::RejectedToolResult {
                        owner: id.into(),
                        count: self.tools.len(),
                        rejected,
                    });
                }
            }
        } else if entry["type"] == "system" && entry["subtype"] == "turn_duration" {
            events.push(AgentEvent::RejectionSettled);
        } else if entry["type"] == "system" && self.ends_compaction(entry) {
            events.push(AgentEvent::CompactionEnded);
        }
        (!events.is_empty()).then_some(events)
    }
}

pub(crate) struct ClaudeStatusWatcher {
    feed: HookFeed,
    session: String,
    parser: ClaudeParser,
    transcript: Option<crate::session::hook_feed::TranscriptTail>,
    read_transcript: bool,
}
impl ClaudeStatusWatcher {
    pub(crate) fn from_receiver(receiver: crate::session::hook_queue::HookReceiver) -> Self {
        Self {
            session: receiver.session().to_owned(),
            feed: HookFeed::from_receiver(receiver),
            parser: Default::default(),
            transcript: None,
            read_transcript: false,
        }
    }
    pub(crate) fn drain_events(
        &mut self,
        cancel: u8,
        mut emit: impl FnMut(AgentEvent) -> AdapterFeedback,
        mut session_start: impl FnMut(String),
    ) -> Result<()> {
        let now = crate::session::clock::timestamp_millis();
        let session = &self.session;
        self.feed.drain_admitted(|report, admitted_at| {
            if let Ok(report) = serde_json::from_value::<StatusReport>(report) {
                if report.hook_event_name == "SessionStart" && report.agent_id.is_none() {
                    if let Some(id) = report
                        .session_id
                        .as_deref()
                        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
                    {
                        session_start(id.to_owned());
                    }
                }
                let hook = report.hook_event_name.clone();
                let kind: String = [
                    ("source", &report.source),
                    ("trigger", &report.trigger),
                    ("type", &report.notification_type),
                ]
                .into_iter()
                .filter_map(|(name, value)| {
                    value.as_ref().map(|value| format!(" {name}={value:?}"))
                })
                .collect();
                match self.parser.hook(report) {
                    None => log::debug!("session {session}: dropped Claude hook {hook}{kind}"),
                    Some(events) if events.is_empty() => {
                        log::info!("session {session}: Claude hook {hook}{kind} -> no change")
                    }
                    Some(events) => {
                        let feedback = emit(AgentEvent::Batch {
                            runtime: Runtime::ClaudeCode,
                            events: events.clone(),
                        });
                        log::info!(
                            "session {session}: Claude hook {hook}{kind} -> {}",
                            describe(feedback)
                        );
                        self.parser.accept(&events, feedback, admitted_at);
                        self.read_transcript = feedback.read_transcript || feedback.compacting;
                    }
                }
            }
        })?;
        if cancel != 0 {
            let feedback = emit(AgentEvent::Batch {
                runtime: Runtime::ClaudeCode,
                events: vec![AgentEvent::LocalCancel { kind: cancel }],
            });
            self.parser.note_compaction(feedback.compacting, &[], now);
            self.read_transcript = feedback.read_transcript || feedback.compacting;
        }
        self.drain_transcript(now, &mut emit);
        if self.parser.compaction_expired(now) {
            log::warn!(
                "session {}: Claude compaction sent no end within {} minutes; restoring the status it interrupted",
                self.session,
                COMPACTION_TIMEOUT_MS / 60_000
            );
            let events = vec![AgentEvent::CompactionEnded];
            let feedback = emit(AgentEvent::Batch {
                runtime: Runtime::ClaudeCode,
                events: events.clone(),
            });
            self.parser.accept(&events, feedback, now);
            self.read_transcript = feedback.read_transcript || feedback.compacting;
        }
        Ok(())
    }
    fn drain_transcript(&mut self, now: i64, emit: &mut impl FnMut(AgentEvent) -> AdapterFeedback) {
        if !self.read_transcript {
            return;
        }
        let Some(path) = self.parser.transcript_path.clone() else {
            return;
        };
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
                    let feedback = emit(AgentEvent::Transcript {
                        runtime: Runtime::ClaudeCode,
                        events: events.clone(),
                    });
                    if events
                        .iter()
                        .any(|event| matches!(event, AgentEvent::CompactionEnded))
                    {
                        log::info!(
                            "session {}: Claude transcript ended compaction -> {}",
                            self.session,
                            describe(feedback)
                        );
                    }
                    self.parser.accept(&events, feedback, now);
                    self.read_transcript = feedback.read_transcript || feedback.compacting;
                }
            }
            tail.pending.clear();
        }
    }
}

fn describe(feedback: AdapterFeedback) -> String {
    if !feedback.accepted {
        return "ignored".into();
    }
    let mut status = format!("{:?}", feedback.activity);
    if let Some(detail) = feedback.detail {
        status.push_str(&format!(" ({detail:?})"));
    }
    if let Some(outcome) = feedback.outcome {
        status.push_str(&format!(", {outcome:?}"));
    }
    status
}
impl crate::session::hook_feed::HookWatcher for ClaudeStatusWatcher {
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
struct ClaudeObservation {
    parser: ClaudeParser,
    model: crate::session::state::agent::AgentModel,
    value: crate::session::state::agent::TurnState,
}
#[cfg(test)]
impl std::ops::Deref for ClaudeObservation {
    type Target = ClaudeParser;
    fn deref(&self) -> &Self::Target {
        &self.parser
    }
}
#[cfg(test)]
impl std::ops::DerefMut for ClaudeObservation {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.parser
    }
}
#[cfg(test)]
impl ClaudeObservation {
    fn observe(&mut self, event: StatusReport, now: i64) -> Option<AgentObservation> {
        let events = self.parser.hook(event)?;
        let (reduced, feedback) = self.model.reduce_with_feedback(
            AgentEvent::Batch {
                runtime: Runtime::ClaudeCode,
                events: events.clone(),
            },
            now,
        );
        self.parser.accept(&events, feedback, now);
        self.value = self.model.value.clone();
        reduced
    }
    fn observe_transcript(&mut self, record: &serde_json::Value) -> Option<AgentObservation> {
        let events = self.parser.record(record)?;
        let now = crate::session::clock::timestamp_millis();
        let (reduced, feedback) = self.model.reduce_with_feedback(
            AgentEvent::Transcript {
                runtime: Runtime::ClaudeCode,
                events: events.clone(),
            },
            now,
        );
        self.parser.accept(&events, feedback, now);
        self.value = self.model.value.clone();
        reduced
    }
}
#[cfg(test)]
mod tests;
