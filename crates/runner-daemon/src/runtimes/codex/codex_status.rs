use crate::model::Runtime;
use crate::session::state::agent::{AdapterFeedback, AgentEvent};
#[cfg(test)]
use crate::session::state::StatusSource;
use std::io::BufRead;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

use crate::error::Result;
use crate::session::hook_feed::HookFeed;
use crate::session::status::TurnOutcome;

#[cfg(test)]
use crate::session::status::{Activity, AgentObservation, ObservationSource, WorkDetail};
pub(crate) const EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PreCompact",
    "PostCompact",
    "Stop",
    "Interrupt",
    "SessionEnd",
];

#[derive(Default, Deserialize)]
struct StatusReport {
    #[serde(default)]
    hook_event_name: String,
    source: Option<String>,
    trigger: Option<String>,
    pub(crate) session_id: Option<String>,
    turn_id: Option<String>,
    pub(crate) transcript_path: Option<PathBuf>,
    agent_id: Option<String>,
}

#[derive(Clone, Default)]
struct CodexParser {
    session_id: Option<String>,
    retired_sessions: std::collections::BTreeSet<String>,
    turn_id: Option<String>,
    retired_turns: std::collections::BTreeSet<String>,
    turn_finished: bool,
    standalone_compaction: bool,
    pending_tools: usize,
    compacting: bool,
    ended: bool,
    transcript_path: Option<PathBuf>,
    abort_reported: bool,
    failure_reported: bool,
    aborted_turn: Option<String>,
    seen: bool,
    abort_settled: bool,
}
impl CodexParser {
    fn hook(&mut self, report: StatusReport) -> Option<Vec<AgentEvent>> {
        if report.agent_id.is_some() || !EVENTS.contains(&report.hook_event_name.as_str()) {
            return None;
        }
        let session = report
            .session_id
            .as_ref()
            .filter(|id| !id.is_empty())?
            .clone();
        // Codex internal workers inherit the slot's hooks, so an end for another
        // session is never evidence that this slot switched conversations.
        if report.hook_event_name == "SessionEnd" && self.session_id.as_ref() != Some(&session) {
            return None;
        }
        let new_session = self.session_id.as_ref().is_some_and(|id| *id != session);
        let start = report.hook_event_name == "SessionStart";
        if self.retired_sessions.contains(&session) && !start {
            return None;
        }
        let recovered = !start && (self.session_id.is_none() || new_session);
        if new_session || (start && self.ended) {
            if new_session {
                if let Some(previous) = self.session_id.take() {
                    self.retired_sessions.insert(previous);
                }
                self.retired_sessions.remove(&session);
                self.retired_turns.clear();
            } else if let Some(turn) = self.turn_id.take() {
                self.retired_turns.insert(turn);
            }
            self.turn_id = None;
            self.turn_finished = false;
            self.standalone_compaction = false;
            self.pending_tools = 0;
            self.compacting = false;
            self.ended = false;
            self.transcript_path = None;
            self.abort_reported = false;
            self.failure_reported = false;

            self.abort_settled = false;
            self.seen = false;
        }
        self.session_id = Some(session);
        if let Some(path) = report.transcript_path.as_ref() {
            self.transcript_path = Some(path.clone());
        }
        let mut events = Vec::new();
        if recovered {
            events.push(AgentEvent::ConversationRecovered);
        }
        if let Some(event) = self.current_hook(report) {
            events.push(event);
        }
        (!events.is_empty()).then_some(events)
    }

    fn current_hook(&mut self, report: StatusReport) -> Option<AgentEvent> {
        if report.hook_event_name == "SessionStart" {
            if report.source.as_deref() == Some("compact") && self.compacting {
                if let Some(path) = report.transcript_path {
                    self.transcript_path = Some(path);
                }
                return None;
            }
            if self.turn_id.is_some() || self.ended {
                return None;
            }
            self.transcript_path = report.transcript_path;
            self.seen = true;
            self.turn_finished = true;
            return Some(AgentEvent::StartupReady);
        }
        if self.ended {
            return None;
        }
        if report.hook_event_name == "SessionEnd" {
            if !self.seen {
                return None;
            }
            self.ended = true;
            self.pending_tools = 0;
            self.compacting = false;
            return Some(AgentEvent::SessionEnded);
        }
        let turn = report.turn_id.filter(|id| !id.is_empty())?;
        if self.retired_turns.contains(&turn) {
            return None;
        }
        let event = if report.hook_event_name == "UserPromptSubmit" {
            if self.turn_id.as_ref() == Some(&turn) {
                return None;
            }
            if let Some(previous) = self.turn_id.replace(turn) {
                self.retired_turns.insert(previous);
            }
            self.pending_tools = 0;
            self.compacting = false;
            self.abort_reported = false;
            self.failure_reported = false;
            self.abort_settled = false;
            self.turn_finished = false;
            self.standalone_compaction = false;
            AgentEvent::TurnStarted
        } else if report.hook_event_name == "PreCompact"
            && report.trigger.as_deref() == Some("manual")
            && self.turn_id.as_ref() != Some(&turn)
            && self.seen
            && self.turn_finished
            && !self.compacting
        {
            // Codex /compact creates its own turn without UserPromptSubmit.
            if let Some(previous) = self.turn_id.replace(turn) {
                self.retired_turns.insert(previous);
            }
            self.turn_finished = false;
            self.standalone_compaction = true;
            self.pending_tools = 0;
            self.compacting = true;
            self.abort_reported = false;
            self.failure_reported = false;
            self.abort_settled = false;
            AgentEvent::ManualCompactionStarted
        } else {
            if self.turn_id.is_none() && report.hook_event_name == "Interrupt" {
                self.turn_id = Some(turn.clone());
            }
            if self.turn_id.as_ref() != Some(&turn) {
                return None;
            }
            if self.abort_reported || self.failure_reported {
                return None;
            }
            if self.standalone_compaction
                && (self.turn_finished
                    || !matches!(report.hook_event_name.as_str(), "PostCompact" | "Interrupt")
                    || (report.hook_event_name == "PostCompact"
                        && report.trigger.as_deref() != Some("manual")))
            {
                return None;
            }
            match report.hook_event_name.as_str() {
                "Interrupt" => {
                    self.abort_reported = true;
                    self.turn_finished = false;
                    self.pending_tools = 0;
                    self.compacting = false;
                    AgentEvent::TurnEnded {
                        outcome: TurnOutcome::Interrupted,
                    }
                }
                "Stop" => {
                    self.turn_finished = true;
                    self.pending_tools = 0;
                    self.compacting = false;
                    AgentEvent::TurnEnded {
                        outcome: TurnOutcome::Completed,
                    }
                }
                "PreToolUse" => {
                    self.turn_finished = false;
                    self.pending_tools += 1;
                    AgentEvent::ToolStarted {
                        count: self.pending_tools,
                        question: None,
                    }
                }
                "PostToolUse" => {
                    self.pending_tools = self.pending_tools.saturating_sub(1);
                    AgentEvent::ToolEnded {
                        owner: None,
                        count: self.pending_tools,
                        interrupted: false,
                        transcript: false,
                    }
                }
                "PreCompact" => {
                    self.compacting = true;
                    AgentEvent::CompactionStarted
                }
                "PostCompact" => {
                    self.compacting = false;
                    if self.standalone_compaction {
                        self.turn_finished = true;
                    }
                    AgentEvent::CompactionEnded
                }
                _ => return None,
            }
        };
        if let Some(path) = report.transcript_path {
            self.transcript_path = Some(path);
        }
        self.seen = true;
        Some(event)
    }
    fn record(&mut self, record: &Value) -> Option<Vec<AgentEvent>> {
        let payload = &record["payload"];
        let aborted = !self.ended
            && record["type"] == "event_msg"
            && payload["type"] == "turn_aborted"
            && payload["reason"] == "interrupted"
            && payload["turn_id"].as_str().is_some()
            && payload["turn_id"].as_str() == self.turn_id.as_deref();
        if aborted && !self.abort_reported {
            // The transcript can show the abort before its Interrupt hook arrives.
            self.aborted_turn = self.turn_id.clone();
            return None;
        }
        if aborted && !self.abort_settled {
            self.abort_settled = true;
            self.turn_finished = true;
            return Some(vec![AgentEvent::AbortSettled]);
        }
        // Codex runs no Stop hook for a turn that ends in an API error.
        if !self.ended
            && !self.abort_reported
            && !self.turn_finished
            && record["type"] == "event_msg"
            && payload["type"] == "task_complete"
            && payload.get("error").is_some_and(|error| !error.is_null())
            && payload["turn_id"].as_str().is_some()
            && payload["turn_id"].as_str() == self.turn_id.as_deref()
        {
            self.failure_reported = true;
            self.turn_finished = true;
            self.pending_tools = 0;
            self.compacting = false;
            return Some(vec![AgentEvent::TurnEnded {
                outcome: TurnOutcome::Failed,
            }]);
        }
        None
    }

    fn settle_early_abort(&mut self) -> Option<Vec<AgentEvent>> {
        if self.ended
            || !self.abort_reported
            || self.abort_settled
            || self.aborted_turn.is_none()
            || self.aborted_turn != self.turn_id
        {
            return None;
        }
        self.aborted_turn = None;
        self.abort_settled = true;
        self.turn_finished = true;
        Some(vec![AgentEvent::AbortSettled])
    }

    fn reads_transcript(&self) -> bool {
        !self.ended
            && if self.abort_reported {
                !self.abort_settled
            } else {
                self.turn_id.is_some() && !self.turn_finished
            }
    }
}

pub(crate) struct CodexStatusWatcher {
    feed: HookFeed,
    parser: CodexParser,
    transcript: Option<crate::session::hook_feed::TranscriptTail>,
}
impl CodexStatusWatcher {
    pub(crate) fn from_receiver(receiver: crate::session::hook_queue::HookReceiver) -> Self {
        Self {
            feed: HookFeed::from_receiver(receiver),
            parser: Default::default(),
            transcript: None,
        }
    }
    pub(crate) fn drain_events(
        &mut self,
        _cancel: u8,
        mut emit: impl FnMut(AgentEvent) -> AdapterFeedback,
        mut session_start: impl FnMut(String),
    ) -> Result<()> {
        self.feed.drain(|report| {
            if let Ok(report) = serde_json::from_value::<StatusReport>(report) {
                let previous = self.parser.session_id.clone();
                if let Some(events) = self.parser.hook(report) {
                    emit(AgentEvent::Batch {
                        runtime: Runtime::Codex,
                        events,
                    });
                }
                if self.parser.session_id != previous {
                    self.transcript = None;
                    if let Some(id) = self
                        .parser
                        .session_id
                        .as_deref()
                        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
                    {
                        session_start(id.to_owned());
                    }
                }
            }
        })?;
        if let Some(events) = self.parser.settle_early_abort() {
            emit(AgentEvent::Transcript {
                runtime: Runtime::Codex,
                events,
            });
        }
        self.drain_transcript(&mut emit);
        Ok(())
    }
    fn drain_transcript(&mut self, emit: &mut impl FnMut(AgentEvent) -> AdapterFeedback) {
        if !self.parser.reads_transcript() {
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
        let mut events = Vec::new();
        while let Ok(count) = tail.reader.read_until(b'\n', &mut tail.pending) {
            if count == 0 || tail.pending.last() != Some(&b'\n') {
                break;
            }
            if let Ok(record) = serde_json::from_slice(&tail.pending) {
                if let Some(record_events) = self.parser.record(&record) {
                    events.extend(record_events);
                }
            }
            tail.pending.clear();
        }
        if !events.is_empty() {
            emit(AgentEvent::Transcript {
                runtime: Runtime::Codex,
                events,
            });
        }
    }
}
impl crate::session::hook_feed::HookWatcher for CodexStatusWatcher {
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
struct CodexObservation {
    parser: CodexParser,
    model: crate::session::state::agent::AgentModel,
    value: crate::session::state::agent::TurnState,
}
#[cfg(test)]
impl std::ops::Deref for CodexObservation {
    type Target = CodexParser;
    fn deref(&self) -> &Self::Target {
        &self.parser
    }
}
#[cfg(test)]
impl std::ops::DerefMut for CodexObservation {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.parser
    }
}
#[cfg(test)]
impl CodexObservation {
    fn observe(&mut self, event: StatusReport, now: i64) -> Option<AgentObservation> {
        let events = self.parser.hook(event)?;
        let reduced = self.model.reduce(
            AgentEvent::Batch {
                runtime: Runtime::Codex,
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
                runtime: Runtime::Codex,
                events,
            },
            crate::session::clock::timestamp_millis(),
        );
        self.value = self.model.value.clone();
        reduced
    }
}
#[cfg(test)]
mod tests;
