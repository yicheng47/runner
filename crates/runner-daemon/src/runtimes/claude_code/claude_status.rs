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
pub(crate) const HOOK_TIMEOUT_SECS: u64 = 2;

// Runner-owned per-invocation helper follows cmux's hook bridge shape
// (manaflow-ai/cmux, GPL-3.0-or-later); the status records are Runner's.

#[derive(Debug, Default, Deserialize)]
struct StatusReport {
    #[serde(default)]
    hook_event_name: String,
    notification_type: Option<String>,
    source: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) transcript_path: Option<PathBuf>,
    pub(crate) prompt_id: Option<String>,
    agent_id: Option<String>,
    tool_use_id: Option<String>,
    tool_name: Option<String>,
    tool_input: Option<serde_json::Value>,
    #[serde(default)]
    is_interrupt: bool,
    elicitation_id: Option<String>,
    mcp_server_name: Option<String>,
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
#[derive(Clone, Default)]
struct ClaudeParser {
    session_id: Option<String>,
    prompt_id: Option<String>,
    transcript_path: Option<PathBuf>,
    tools: std::collections::BTreeMap<String, PendingTool>,
    compacting: bool,
    elicitations: Vec<PendingElicitation>,
}
impl ClaudeParser {
    fn hook(&mut self, report: StatusReport) -> Option<Vec<AgentEvent>> {
        if report.agent_id.is_some() || report.hook_event_name.starts_with("Subagent") {
            return None;
        }
        if report.hook_event_name == "SessionStart" {
            if report.source.as_deref() == Some("compact")
                && self.compacting
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
                return None;
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
        {
            return None;
        }
        if let Some(path) = report.transcript_path {
            self.transcript_path = Some(path);
        }
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
            "Stop" | "StopFailure" => {
                self.clear();
                AgentEvent::TurnEnded {
                    outcome: if report.hook_event_name == "Stop" {
                        TurnOutcome::Completed
                    } else {
                        TurnOutcome::Failed
                    },
                }
            }
            _ => return None,
        };
        Some(vec![event])
    }
    fn clear(&mut self) {
        self.tools.clear();
        self.compacting = false;
        self.elicitations.clear();
    }
    fn accept(&mut self, events: &[AgentEvent], feedback: AdapterFeedback) {
        self.compacting = feedback.compacting;
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
        }
        (!events.is_empty()).then_some(events)
    }
}

pub(crate) struct ClaudeStatusWatcher {
    feed: HookFeed,
    parser: ClaudeParser,
    transcript: Option<crate::session::hook_feed::TranscriptTail>,
    read_transcript: bool,
}
impl ClaudeStatusWatcher {
    pub(crate) fn from_receiver(receiver: crate::session::hook_queue::HookReceiver) -> Self {
        Self {
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
        self.feed.drain(|report| {
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
                if let Some(events) = self.parser.hook(report) {
                    let feedback = emit(AgentEvent::Batch {
                        runtime: Runtime::ClaudeCode,
                        events: events.clone(),
                    });
                    self.parser.accept(&events, feedback);
                    self.read_transcript = feedback.read_transcript;
                }
            }
        })?;
        if cancel != 0 {
            let feedback = emit(AgentEvent::Batch {
                runtime: Runtime::ClaudeCode,
                events: vec![AgentEvent::LocalCancel { kind: cancel }],
            });
            self.parser.compacting = feedback.compacting;
            self.read_transcript = feedback.read_transcript;
        }
        self.drain_transcript(&mut emit);
        Ok(())
    }
    fn drain_transcript(&mut self, emit: &mut impl FnMut(AgentEvent) -> AdapterFeedback) {
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
                    self.parser.accept(&events, feedback);
                    self.read_transcript = feedback.read_transcript;
                }
            }
            tail.pending.clear();
        }
    }
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
        self.parser.accept(&events, feedback);
        self.value = self.model.value.clone();
        reduced
    }
    fn observe_transcript(&mut self, record: &serde_json::Value) -> Option<AgentObservation> {
        let events = self.parser.record(record)?;
        let (reduced, feedback) = self.model.reduce_with_feedback(
            AgentEvent::Transcript {
                runtime: Runtime::ClaudeCode,
                events: events.clone(),
            },
            crate::session::clock::timestamp_millis(),
        );
        self.parser.accept(&events, feedback);
        self.value = self.model.value.clone();
        reduced
    }
}
#[cfg(test)]
mod tests {
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
        fn drain_status(
            &mut self,
            publish: impl FnMut(AgentObservation, StatusSource),
        ) -> Result<()> {
            self.drain_with_session_starts(publish, |_| {})
        }
        fn drain(
            &mut self,
            mut publish: impl FnMut(SessionActivityState, StatusSource),
        ) -> Result<()> {
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
        let stopped =
            observe(&mut model, "Stop", serde_json::json!({"prompt_id":"first"})).unwrap();
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
            let value = model.observe_transcript(&serde_json::json!({
                "type":"user","sessionId":"main","promptId":"turn","isSidechain":false,
                "toolDenialKind":"user-rejected",
                "message":{"content":[{"type":"tool_result","tool_use_id":"wait","is_error":true}]},
            })).unwrap();
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
        let request = serde_json::json!({"tool_name":"Bash","tool_input":{"command":"curl https://example.com"}});
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
}
