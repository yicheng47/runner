use std::io::BufRead;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

use crate::error::Result;
pub(crate) use crate::session::hook_feed::clear_leftovers;
use crate::session::hook_feed::{HookFeed, TranscriptTail};
use crate::session::state::agent::{AdapterFeedback, AgentEvent, ProgramOutcomePolicy};
use crate::session::status::TurnOutcome;

pub(crate) const EVENTS: &[&str] = &["SessionStart", "UserPromptSubmit"];

#[derive(Debug, Default, Deserialize)]
struct StatusReport {
    #[serde(default)]
    hook_event_name: String,
    source: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) transcript_path: Option<PathBuf>,
    pub(crate) prompt_id: Option<String>,
    prompt: Option<String>,
    agent_id: Option<String>,
}

#[derive(Clone, Default)]
struct ClaudeParser {
    session_id: Option<String>,
    prompt_id: Option<String>,
    transcript_path: Option<PathBuf>,
    turn_finished: bool,
}

impl ClaudeParser {
    fn hook(&mut self, report: StatusReport) -> Option<AgentEvent> {
        if report.agent_id.is_some() || !EVENTS.contains(&report.hook_event_name.as_str()) {
            return None;
        }
        if report.hook_event_name == "SessionStart" {
            // Claude sends `SessionStart(compact)` before or after compaction while continuing
            // the same conversation; keep the active turn/transcript state intact.
            if report.source.as_deref() == Some("compact")
                && report
                    .session_id
                    .as_ref()
                    .is_none_or(|id| self.session_id.as_ref().is_none_or(|current| current == id))
            {
                if let Some(id) = report.session_id.filter(|id| !id.is_empty()) {
                    self.session_id = Some(id);
                }
                if let Some(path) = report.transcript_path {
                    self.transcript_path = Some(path);
                }
                return Some(AgentEvent::ProgramSessionStarted {
                    conversation_changed: false,
                    compacted: true,
                });
            }
            let next_session = report.session_id.filter(|id| !id.is_empty());
            let conversation_changed = next_session
                .as_ref()
                .is_some_and(|next| self.session_id.as_ref().is_some_and(|prev| prev != next));
            if let Some(id) = next_session {
                self.session_id = Some(id);
            }
            if let Some(path) = report.transcript_path {
                self.transcript_path = Some(path);
            } else if conversation_changed {
                self.transcript_path = None;
            }
            self.prompt_id = None;
            self.turn_finished = false;
            return Some(AgentEvent::ProgramSessionStarted {
                conversation_changed,
                compacted: false,
            });
        }
        if self.session_id.is_some()
            && report.session_id.is_some()
            && self.session_id != report.session_id
        {
            return None;
        }
        if self.session_id.is_none() {
            self.session_id = report.session_id.filter(|id| !id.is_empty());
        }
        self.prompt_id = report.prompt_id.filter(|id| !id.is_empty());
        if let Some(path) = report.transcript_path {
            self.transcript_path = Some(path);
        }
        self.turn_finished = false;
        Some(AgentEvent::ProgramTurnConfirmed {
            conversation_changed: false,
            prompt: report.prompt,
        })
    }

    fn record(&mut self, entry: &Value) -> Option<AgentEvent> {
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
        if entry["type"] == "user" {
            if is_user_interrupt(entry) {
                if !self.turn_finished {
                    self.turn_finished = true;
                    return Some(AgentEvent::ProgramTurnOutcome {
                        outcome: TurnOutcome::Interrupted,
                        policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                    });
                }
                return None;
            }
            self.turn_finished = false;
            return None;
        }
        if entry["type"] == "assistant" && entry["isApiErrorMessage"].as_bool() == Some(true) {
            if !self.turn_finished {
                self.turn_finished = true;
                return Some(AgentEvent::ProgramTurnOutcome {
                    outcome: TurnOutcome::Failed,
                    policy: ProgramOutcomePolicy::NativeFallbackOnly,
                });
            }
            return None;
        }
        if entry["type"] == "system" && entry["subtype"] == "turn_duration" {
            return Some(AgentEvent::ProgramTurnFinished);
        }
        None
    }
}

fn is_user_interrupt(entry: &Value) -> bool {
    let Some(content) = entry.pointer("/message/content") else {
        return false;
    };
    if let Some(text) = content.as_str() {
        return text.starts_with("[Request interrupted by user");
    }
    content.as_array().is_some_and(|blocks| {
        blocks.iter().any(|block| {
            block["type"] == "text"
                && block["text"]
                    .as_str()
                    .is_some_and(|text| text.starts_with("[Request interrupted by user"))
        })
    })
}

pub(crate) struct ClaudeStatusWatcher {
    feed: HookFeed,
    session: String,
    parser: ClaudeParser,
    transcript: Option<TranscriptTail>,
}

impl ClaudeStatusWatcher {
    pub(crate) fn from_receiver(receiver: crate::session::hook_queue::HookReceiver) -> Self {
        Self {
            session: receiver.session().to_owned(),
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
        let session = &self.session;
        self.feed.drain(|report| {
            if let Ok(report) = serde_json::from_value::<StatusReport>(report) {
                let is_start = report.hook_event_name == "SessionStart";
                let is_compact = report.source.as_deref() == Some("compact");
                if is_start && report.agent_id.is_none() {
                    if let Some(id) = report
                        .session_id
                        .as_deref()
                        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
                    {
                        session_start(id.to_owned());
                    }
                }
                let hook = report.hook_event_name.clone();
                let previous_path = self.parser.transcript_path.clone();
                let previous_session = self.parser.session_id.clone();
                match self.parser.hook(report) {
                    None => log::debug!("session {session}: dropped Claude hook {hook}"),
                    Some(event) => {
                        if self.parser.session_id != previous_session
                            || self.parser.transcript_path != previous_path
                        {
                            self.transcript = None;
                            // Skip pre-existing records on startup/resume so an older interrupt
                            // from prior history never replays as a current outcome.
                            if is_start && !is_compact {
                                if let Some(path) = self.parser.transcript_path.as_deref() {
                                    self.transcript = TranscriptTail::open_at_end(path).ok();
                                }
                            }
                        }
                        emit(event);
                    }
                }
            }
        })?;
        self.drain_transcript(&mut emit);
        Ok(())
    }

    fn drain_transcript(&mut self, emit: &mut impl FnMut(AgentEvent) -> AdapterFeedback) {
        if self.parser.session_id.is_none() {
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
                if let Some(event) = self.parser.record(&record) {
                    emit(event);
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
mod tests;
