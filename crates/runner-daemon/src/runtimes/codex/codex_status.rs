use crate::session::state::agent::{AdapterFeedback, AgentEvent};
use std::io::BufRead;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

use crate::error::Result;
use crate::session::hook_feed::HookFeed;
use crate::session::status::TurnOutcome;

pub(crate) const EVENTS: &[&str] = &["SessionStart", "UserPromptSubmit"];

#[derive(Default, Deserialize)]
struct StatusReport {
    #[serde(default)]
    hook_event_name: String,
    source: Option<String>,
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
    transcript_path: Option<PathBuf>,
}

impl CodexParser {
    fn hook(&mut self, report: StatusReport) -> Option<AgentEvent> {
        if report.agent_id.is_some() || !EVENTS.contains(&report.hook_event_name.as_str()) {
            return None;
        }
        let session = report
            .session_id
            .as_ref()
            .filter(|id| !id.is_empty())?
            .clone();
        let start = report.hook_event_name == "SessionStart";
        if self.retired_sessions.contains(&session) && !start {
            return None;
        }
        let new_session = self.session_id.as_ref().is_some_and(|id| *id != session);
        if new_session {
            if let Some(previous) = self.session_id.take() {
                self.retired_sessions.insert(previous);
            }
            self.retired_sessions.remove(&session);
            self.turn_id = None;
            self.retired_turns.clear();
            self.turn_finished = false;
            self.transcript_path = None;
        }
        self.session_id = Some(session);
        if let Some(path) = report.transcript_path {
            self.transcript_path = Some(path);
        }
        if start {
            return Some(AgentEvent::ProgramSessionStarted {
                conversation_changed: new_session,
                compacted: report.source.as_deref() == Some("compact"),
            });
        }
        let turn = report.turn_id.filter(|id| !id.is_empty())?;
        if self.retired_turns.contains(&turn) {
            return None;
        }
        if self.turn_id.as_ref() == Some(&turn) {
            if self.turn_finished {
                return None;
            }
            // Codex 0.170 attaches mid-turn steered input to the active turn and reuses its turn_id.
            return Some(AgentEvent::ProgramTurnConfirmed {
                conversation_changed: false,
                prompt: None,
            });
        }
        if let Some(previous) = self.turn_id.replace(turn) {
            self.retired_turns.insert(previous);
        }
        self.turn_finished = false;
        Some(AgentEvent::ProgramTurnConfirmed {
            conversation_changed: new_session,
            prompt: None,
        })
    }

    fn record(&mut self, record: &Value) -> Option<AgentEvent> {
        if self.turn_finished || record["type"] != "event_msg" {
            return None;
        }
        let payload = &record["payload"];
        let turn = payload["turn_id"].as_str()?;
        if self.turn_id.as_deref() != Some(turn) {
            return None;
        }
        if payload["type"] == "turn_aborted" && payload["reason"] == "interrupted" {
            self.turn_finished = true;
            return Some(AgentEvent::ProgramTurnOutcome {
                outcome: TurnOutcome::Interrupted,
                policy: crate::session::state::agent::ProgramOutcomePolicy::Immediate,
            });
        }
        if payload["type"] == "task_complete" {
            self.turn_finished = true;
            if payload.get("error").is_some_and(|error| !error.is_null()) {
                return Some(AgentEvent::ProgramTurnOutcome {
                    outcome: TurnOutcome::Failed,
                    policy: crate::session::state::agent::ProgramOutcomePolicy::Immediate,
                });
            }
            return Some(AgentEvent::ProgramTurnFinished);
        }
        None
    }

    fn reads_transcript(&self) -> bool {
        self.turn_id.is_some() && !self.turn_finished
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
                let event = self.parser.hook(report);
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
                if let Some(event) = event {
                    emit(event);
                }
            }
        })?;
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
        while self.parser.reads_transcript() {
            let Ok(count) = tail.reader.read_until(b'\n', &mut tail.pending) else {
                break;
            };
            if count == 0 || tail.pending.last() != Some(&b'\n') {
                break;
            }
            if let Ok(record) = serde_json::from_slice(&tail.pending) {
                if let Some(event) = self.parser.record(&record) {
                    emit(event);
                }
            }
            tail.pending.clear();
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
mod tests;
