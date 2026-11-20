use std::io::BufRead;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::*;
use crate::session::clock;
use crate::session::hook_feed::{HookFeed, TranscriptTail};
use crate::session::state::agent::{AdapterFeedback, AgentEvent, ProgramOutcomePolicy};
use crate::session::status::TurnOutcome;

const EVENTS: &[&str] = &["SessionStart", "UserPromptSubmit"];
const INITIAL_FULL_SCAN_BACKOFF: Duration = Duration::from_millis(250);
const MAX_FULL_SCAN_BACKOFF: Duration = Duration::from_secs(10);

fn hooks() -> Value {
    let mut hooks = serde_json::Map::new();
    for event in EVENTS {
        let group = json!({"hooks":[{"type":"command","command":format!("if [ -n \"$RUNNER_HOOK_EXECUTABLE\" ]; then \"$RUNNER_HOOK_EXECUTABLE\" hook report --runtime grok --event {event} >/dev/null 2>&1; fi; exit 0"),"timeout":2}]});
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

#[derive(Debug, Default)]
struct StatusReport {
    hook_event_name: String,
    session_id: Option<String>,
    transcript_path: Option<PathBuf>,
    cwd: Option<PathBuf>,
    prompt_id: Option<String>,
    prompt: Option<String>,
    subagent_type: Option<Value>,
    agent_id: Option<Value>,
}

fn first_str<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
}

fn first_non_empty_str<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    })
}

fn first_non_null(value: &Value, keys: &[&str]) -> Option<Value> {
    keys.iter()
        .find_map(|key| value.get(*key).filter(|v| !v.is_null()).cloned())
}

impl StatusReport {
    /// Parses Grok's stdin hook JSON directly from `Value`. Grok 1.0.89 emits
    /// both camelCase keys (`sessionId`, `transcriptPath`, `hookEventName`) and
    /// Claude-compatible snake_case aliases (`session_id`, `transcript_path`,
    /// `hook_event_name`) in the same object, which rejects `#[serde(alias)]`
    /// with a duplicate-field error.
    fn from_value(value: &Value) -> Option<Self> {
        if !value.is_object() {
            return None;
        }
        let transcript_path = first_non_empty_str(value, &["transcriptPath", "transcript_path"])
            .map(|raw| {
                let path = PathBuf::from(raw);
                if path.file_name().and_then(|name| name.to_str()) == Some("chat_history.jsonl") {
                    path.with_file_name("updates.jsonl")
                } else {
                    path
                }
            });
        Some(Self {
            hook_event_name: first_str(value, &["hook_event_name", "hookEventName"])
                .unwrap_or_default()
                .to_owned(),
            session_id: first_non_empty_str(value, &["sessionId", "session_id"]).map(str::to_owned),
            transcript_path,
            cwd: first_non_empty_str(value, &["cwd", "workspaceRoot", "workspace_root"])
                .map(PathBuf::from),
            prompt_id: first_non_empty_str(value, &["promptId", "prompt_id"]).map(str::to_owned),
            prompt: first_str(value, &["prompt"]).map(str::to_owned),
            subagent_type: first_non_null(value, &["subagentType", "subagent_type"]),
            agent_id: first_non_null(value, &["agentId", "agent_id"]),
        })
    }
}

#[derive(Clone, Default)]
struct GrokParser {
    key: Option<String>,
    cwd: Option<PathBuf>,
    prompt_id: Option<String>,
    transcript_path: Option<PathBuf>,
    explicit_transcript: bool,
    turn_finished: bool,
}

fn encode_session_cwd(path: &Path) -> String {
    use std::fmt::Write;
    let raw = path.to_string_lossy();
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            let _ = write!(&mut out, "%{byte:02X}");
        }
    }
    out
}

/// O(1) direct probe of Grok's standard `<grok_home>/sessions/<encoded-cwd>/<session_id>/updates.jsonl`
/// layout (checking both raw and canonicalized `cwd`, e.g. `/tmp` vs `/private/tmp`).
fn direct_session_transcript(
    grok_home: &Path,
    session_id: &str,
    cwd: Option<&Path>,
) -> Option<PathBuf> {
    let cwd = cwd?;
    let sessions_dir = grok_home.join("sessions");
    let direct = sessions_dir
        .join(encode_session_cwd(cwd))
        .join(session_id)
        .join("updates.jsonl");
    if direct.is_file() {
        return Some(direct);
    }
    if let Ok(canonical) = cwd.canonicalize() {
        if canonical != cwd {
            let canon = sessions_dir
                .join(encode_session_cwd(&canonical))
                .join(session_id)
                .join("updates.jsonl");
            if canon.is_file() {
                return Some(canon);
            }
        }
    }
    None
}

/// Fallback full bucket scan under `<grok_home>/sessions/*/<session_id>/updates.jsonl`
/// for slug+hash (>255-byte) cwd buckets or cross-workspace dashboard switches.
fn scan_session_transcript(
    grok_home: &Path,
    session_id: &str,
    cwd: Option<&Path>,
) -> Option<PathBuf> {
    let entries = std::fs::read_dir(grok_home.join("sessions")).ok()?;
    let mut matches: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path().join(session_id).join("updates.jsonl"))
        .filter(|path| path.is_file())
        .collect();
    if matches.is_empty() {
        return None;
    }
    if matches.len() == 1 {
        return matches.pop();
    }
    matches.sort();
    if let Some(target) = cwd {
        let target = target
            .canonicalize()
            .unwrap_or_else(|_| target.to_path_buf());
        if let Some(found) = matches.iter().find(|path| {
            let Some(summary) = path.parent().map(|dir| dir.join("summary.json")) else {
                return false;
            };
            std::fs::read(summary)
                .ok()
                .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
                .and_then(|doc| doc["info"]["cwd"].as_str().map(PathBuf::from))
                .is_some_and(|saved| saved.canonicalize().unwrap_or(saved) == target)
        }) {
            return Some(found.clone());
        }
    }
    matches.into_iter().next()
}

fn record_before_min_timestamp(entry: &Value, min_ms: i64) -> bool {
    if let Some(ms) = entry
        .pointer("/params/_meta/agentTimestampMs")
        .and_then(Value::as_i64)
        .or_else(|| {
            entry
                .get("timestamp_us")
                .and_then(Value::as_i64)
                .map(|us| us / 1_000)
        })
    {
        return ms < min_ms;
    }
    if let Some(secs) = entry.get("timestamp").and_then(Value::as_i64) {
        return secs.saturating_mul(1_000).saturating_add(999) < min_ms;
    }
    false
}

impl GrokParser {
    fn hook(
        &mut self,
        report: StatusReport,
        session_start: &mut dyn FnMut(String),
    ) -> Option<AgentEvent> {
        if report
            .subagent_type
            .as_ref()
            .is_some_and(|value| !value.is_null())
            || report
                .agent_id
                .as_ref()
                .is_some_and(|value| !value.is_null())
            || !EVENTS.contains(&report.hook_event_name.as_str())
        {
            return None;
        }
        let id = report.session_id.as_deref().filter(|id| is_uuid(id))?;
        let conversation_changed = self.key.as_deref().is_some_and(|prev| prev != id);
        if self.key.as_deref() != Some(id) {
            self.key = Some(id.to_owned());
            self.prompt_id = None;
            session_start(id.to_owned());
        }
        if let Some(cwd) = report.cwd {
            self.cwd = Some(cwd);
        }
        if let Some(path) = report.transcript_path {
            self.transcript_path = Some(path);
            self.explicit_transcript = true;
        } else if conversation_changed {
            self.transcript_path = None;
            self.explicit_transcript = false;
        }
        self.turn_finished = false;
        match report.hook_event_name.as_str() {
            "SessionStart" => {
                self.prompt_id = None;
                Some(AgentEvent::ProgramSessionStarted {
                    conversation_changed,
                    compacted: false,
                })
            }
            "UserPromptSubmit" => {
                self.prompt_id = report.prompt_id.filter(|prompt| !prompt.is_empty());
                Some(AgentEvent::ProgramTurnConfirmed {
                    conversation_changed,
                    prompt: report.prompt,
                })
            }
            _ => None,
        }
    }

    fn record(&mut self, entry: &Value, min_timestamp_ms: Option<i64>) -> Option<AgentEvent> {
        if min_timestamp_ms.is_some_and(|min_ms| record_before_min_timestamp(entry, min_ms)) {
            return None;
        }
        let session = self.key.as_deref()?;
        let params = entry.get("params")?;
        if params["sessionId"].as_str() != Some(session) {
            return None;
        }
        let update = params.get("update")?;
        if update["sessionUpdate"].as_str() != Some("turn_completed") {
            return None;
        }
        let record_prompt = update
            .get("prompt_id")
            .or_else(|| update.get("promptId"))
            .and_then(Value::as_str);
        if record_prompt.is_some_and(|id| {
            self.prompt_id
                .as_deref()
                .is_some_and(|current| current != id)
        }) {
            return None;
        }
        let stop_reason = update
            .get("stop_reason")
            .or_else(|| update.get("stopReason"))
            .and_then(Value::as_str);
        let has_error = update
            .get("error_kind")
            .or_else(|| update.get("errorKind"))
            .is_some_and(|value| !value.is_null())
            || update.get("error").is_some_and(|value| !value.is_null())
            || matches!(
                stop_reason,
                Some(
                    "model_error"
                        | "retries_exhausted"
                        | "connection_lost"
                        | "startup_failed"
                        | "auth_required"
                        | "error"
                        | "failed"
                )
            );
        if has_error {
            if !self.turn_finished {
                self.turn_finished = true;
                return Some(AgentEvent::ProgramTurnOutcome {
                    outcome: TurnOutcome::Failed,
                    policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                });
            }
            return None;
        }
        if matches!(
            stop_reason,
            Some(
                "cancelled"
                    | "interrupted"
                    | "user_interrupt"
                    | "permission_rejected"
                    | "permission_cancelled"
                    | "hook_blocked"
                    | "hook_stopped"
            )
        ) {
            if !self.turn_finished {
                self.turn_finished = true;
                return Some(AgentEvent::ProgramTurnOutcome {
                    outcome: TurnOutcome::Interrupted,
                    policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                });
            }
            return None;
        }
        if stop_reason == Some("end_turn") {
            self.turn_finished = true;
            return Some(AgentEvent::ProgramTurnFinished);
        }
        None
    }
}

pub(crate) struct Watcher {
    feed: HookFeed,
    parser: GrokParser,
    grok_home: Option<PathBuf>,
    /// Launch timestamp (advanced on conversation switches) used as the lower
    /// bound when a transcript opens from `TranscriptTail::open`'s recent tail
    /// window so pre-launch or pre-switch history never replays.
    session_watermark_ms: i64,
    resuming: bool,
    hook_seen: bool,
    transcript: Option<TranscriptTail>,
    /// While `true` (resumed/switched/explicit-path `SessionStart` whose
    /// `open_at_end` has not succeeded yet and before any new prompt turn),
    /// recovery opens at EOF so even timestamp-less history is skipped.
    skip_history_until_turn: bool,
    min_record_timestamp_ms: Option<i64>,
    next_full_scan_at: Option<Instant>,
    full_scan_backoff: Duration,
    #[cfg(test)]
    full_scan_count: usize,
    outcome_unavailable_emitted: bool,
}

impl Watcher {
    pub(crate) fn new(
        receiver: crate::session::hook_queue::HookReceiver,
        spec: &SpawnSpec,
    ) -> Self {
        let key = spec
            .args
            .windows(2)
            .find(|args| matches!(args[0].as_str(), "-s" | "--session-id" | "--resume" | "-r"))
            .map(|args| args[1].clone());
        let resuming = spec
            .args
            .iter()
            .any(|arg| matches!(arg.as_str(), "--resume" | "-r" | "-c" | "--continue"));
        let grok_home = config_dir(&spec.env, spec.cwd.as_deref(), None);
        Self {
            feed: HookFeed::from_receiver(receiver),
            parser: GrokParser {
                key,
                cwd: spec.cwd.clone(),
                ..Default::default()
            },
            grok_home,
            session_watermark_ms: clock::timestamp_millis(),
            resuming,
            hook_seen: false,
            transcript: None,
            skip_history_until_turn: false,
            min_record_timestamp_ms: None,
            next_full_scan_at: None,
            full_scan_backoff: INITIAL_FULL_SCAN_BACKOFF,
            #[cfg(test)]
            full_scan_count: 0,
            outcome_unavailable_emitted: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_receiver(receiver: crate::session::hook_queue::HookReceiver) -> Self {
        Self::with_home(receiver, None, 0)
    }

    #[cfg(test)]
    pub(crate) fn with_home(
        receiver: crate::session::hook_queue::HookReceiver,
        grok_home: Option<PathBuf>,
        launched_at_ms: i64,
    ) -> Self {
        Self {
            feed: HookFeed::from_receiver(receiver),
            parser: Default::default(),
            grok_home,
            session_watermark_ms: launched_at_ms,
            resuming: false,
            hook_seen: false,
            transcript: None,
            skip_history_until_turn: false,
            min_record_timestamp_ms: None,
            next_full_scan_at: None,
            full_scan_backoff: INITIAL_FULL_SCAN_BACKOFF,
            full_scan_count: 0,
            outcome_unavailable_emitted: false,
        }
    }

    #[cfg(test)]
    fn full_scan_count(&self) -> usize {
        self.full_scan_count
    }

    fn emit_outcome_unavailable(&mut self, emit: &mut dyn FnMut(AgentEvent) -> AdapterFeedback) {
        if !self.outcome_unavailable_emitted {
            self.outcome_unavailable_emitted = true;
            emit(AgentEvent::ProgramOutcomeUnavailable);
        }
    }

    fn reset_scan_backoff(&mut self) {
        self.next_full_scan_at = None;
        self.full_scan_backoff = INITIAL_FULL_SCAN_BACKOFF;
    }

    fn resolve_session_transcript(&mut self) -> Option<PathBuf> {
        let home = self.grok_home.as_deref()?;
        let session_id = self.parser.key.as_deref()?;
        if let Some(found) = direct_session_transcript(home, session_id, self.parser.cwd.as_deref())
        {
            self.reset_scan_backoff();
            return Some(found);
        }
        let now = clock::now();
        if self.next_full_scan_at.is_some_and(|due| now < due) {
            return None;
        }
        #[cfg(test)]
        {
            self.full_scan_count += 1;
        }
        if let Some(found) = scan_session_transcript(home, session_id, self.parser.cwd.as_deref()) {
            self.reset_scan_backoff();
            return Some(found);
        }
        self.next_full_scan_at = Some(now + self.full_scan_backoff);
        self.full_scan_backoff = self
            .full_scan_backoff
            .saturating_mul(2)
            .min(MAX_FULL_SCAN_BACKOFF);
        None
    }

    fn drain_transcript(&mut self, emit: &mut dyn FnMut(AgentEvent) -> AdapterFeedback) {
        if !self.hook_seen || self.parser.key.is_none() {
            return;
        }
        if self.parser.transcript_path.is_none() {
            if let Some(found) = self.resolve_session_transcript() {
                self.parser.transcript_path = Some(found);
            }
        }
        let Some(path) = self.parser.transcript_path.clone() else {
            return;
        };
        if self
            .transcript
            .as_ref()
            .is_none_or(|tail| tail.path != path)
        {
            let opened = if self.skip_history_until_turn {
                TranscriptTail::open_at_end(&path)
            } else {
                // `TranscriptTail::open` reads from the recent tail window (up
                // to 1 MiB) and pairs with `min_record_timestamp_ms` so late
                // post-handoff turns are admitted while older history is skipped.
                TranscriptTail::open(&path)
            };
            match opened {
                Ok(tail) => {
                    self.transcript = Some(tail);
                    if self.skip_history_until_turn {
                        self.skip_history_until_turn = false;
                        self.min_record_timestamp_ms = None;
                    } else if self.min_record_timestamp_ms.is_none()
                        && self.session_watermark_ms > 0
                    {
                        self.min_record_timestamp_ms = Some(self.session_watermark_ms);
                    }
                    self.outcome_unavailable_emitted = false;
                }
                Err(_) => {
                    self.transcript = None;
                    self.emit_outcome_unavailable(emit);
                    return;
                }
            }
        }
        let Some(tail) = self.transcript.as_mut() else {
            return;
        };
        while let Ok(count) = tail.reader.read_until(b'\n', &mut tail.pending) {
            if count == 0 || tail.pending.last() != Some(&b'\n') {
                break;
            }
            if let Ok(record) = serde_json::from_slice(&tail.pending) {
                if let Some(event) = self.parser.record(&record, self.min_record_timestamp_ms) {
                    emit(event);
                }
            }
            tail.pending.clear();
        }
    }
}

impl HookWatcher for Watcher {
    fn drain_events(
        &mut self,
        _cancel: u8,
        emit: &mut dyn FnMut(AgentEvent) -> AdapterFeedback,
        session_start: &mut dyn FnMut(String),
    ) -> crate::error::Result<()> {
        let mut reports = std::collections::VecDeque::new();
        self.feed.drain_admitted(|report, admitted_at| {
            if let Some(parsed) = StatusReport::from_value(&report) {
                reports.push_back((parsed, admitted_at));
            }
        })?;
        while let Some((report, admitted_at)) = reports.pop_front() {
            let previous_key = self.parser.key.clone();
            let previous_path = self.parser.transcript_path.clone();
            let first_hook = !self.hook_seen;
            let Some(event) = self.parser.hook(report, session_start) else {
                continue;
            };
            self.hook_seen = true;
            self.reset_scan_backoff();
            let is_prompt = matches!(event, AgentEvent::ProgramTurnConfirmed { .. });
            let conversation_changed = previous_key
                .as_deref()
                .is_some_and(|prev| self.parser.key.as_deref() != Some(prev));
            let has_following_prompt = !is_prompt
                && self.parser.key.as_deref().is_some_and(|sid| {
                    reports.iter().any(|(later, _)| {
                        later.hook_event_name == "UserPromptSubmit"
                            && later.session_id.as_deref() == Some(sid)
                            && later.subagent_type.as_ref().is_none_or(Value::is_null)
                            && later.agent_id.as_ref().is_none_or(Value::is_null)
                    })
                });
            if (conversation_changed || is_prompt) && self.session_watermark_ms > 0 {
                self.session_watermark_ms = admitted_at.max(self.session_watermark_ms);
            }
            if self.parser.transcript_path.is_none() {
                self.parser.transcript_path = self.resolve_session_transcript();
            }
            let path_changed = self.parser.transcript_path != previous_path;
            let open_eof = !is_prompt
                && !has_following_prompt
                && (self.parser.explicit_transcript
                    || (self.resuming && first_hook)
                    || conversation_changed);
            if conversation_changed
                || path_changed
                || (first_hook && self.transcript.is_none())
                || (is_prompt && self.transcript.is_none())
            {
                self.transcript = None;
                self.skip_history_until_turn = open_eof;
                self.min_record_timestamp_ms =
                    (self.session_watermark_ms > 0).then_some(self.session_watermark_ms);
                if !has_following_prompt {
                    if let Some(path) = self.parser.transcript_path.as_deref() {
                        let opened = if open_eof {
                            // Skip pre-existing records on idle startup/resume/switch so an
                            // older turn outcome never replays as live state.
                            TranscriptTail::open_at_end(path).ok()
                        } else {
                            TranscriptTail::open(path).ok()
                        };
                        if let Some(tail) = opened {
                            self.transcript = Some(tail);
                            if open_eof {
                                self.skip_history_until_turn = false;
                                self.min_record_timestamp_ms = None;
                            }
                            self.outcome_unavailable_emitted = false;
                        }
                    }
                }
            } else if is_prompt {
                // Once a new turn starts on the current session while its transcript
                // is still recovering, admit post-watermark records instead of
                // seeking past the turn on open.
                self.skip_history_until_turn = false;
            }
            emit(event);
            if !has_following_prompt
                && self.parser.transcript_path.is_none()
                && !self.outcome_unavailable_emitted
            {
                self.outcome_unavailable_emitted = true;
                emit(AgentEvent::ProgramOutcomeUnavailable);
            }
        }
        self.drain_transcript(emit);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
