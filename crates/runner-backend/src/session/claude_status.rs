#[cfg(test)]
use std::fs::{self, File, OpenOptions};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use serde::Deserialize;

use crate::error::Result;

#[cfg(test)]
use super::hook_feed::script_path;
pub(crate) use super::hook_feed::{clear_leftovers, hook_command, hooks_supported, status_path};
use super::hook_feed::{HookFeed, TranscriptTail};
#[cfg(test)]
use super::runtime::RunnerStatus;
use super::status::{
    Activity, AgentObservation, HumanInteraction, ObservationSource, TurnOutcome, WaitReason,
};
use std::collections::BTreeMap;

pub(crate) const PATH_ENV: &str = "RUNNER_CLAUDE_STATUS_PATH";
pub(crate) const GENERATION_ENV: &str = "RUNNER_CLAUDE_STATUS_GENERATION";
pub(crate) const HOOK_TIMEOUT_SECS: u64 = 2;
pub(crate) const CTRL_C_INTERRUPT: u8 = 1;
pub(crate) const ESCAPE_INTERRUPT: u8 = 2;

// Runner-owned per-invocation helper follows cmux's hook bridge shape
// (manaflow-ai/cmux, GPL-3.0-or-later); the status records are Runner's.
const APPEND_SCRIPT: &str = r#"#!/bin/sh
payload=$(mktemp "$1.XXXXXXXX") || { cat >/dev/null; exit 0; }
cat >"$payload" || { rm -f "$payload"; exit 0; }
printf '{"generation":"%s","hook_event_name":"%s","payload_file":"%s"}\n' "$RUNNER_CLAUDE_STATUS_GENERATION" "$2" "${payload##*/}" >> "$1" 2>/dev/null || rm -f "$payload"
exit 0
"#;

#[derive(Debug, Default, Deserialize)]
struct StatusReport {
    #[serde(default)]
    hook_event_name: String,
    notification_type: Option<String>,
    session_id: Option<String>,
    transcript_path: Option<PathBuf>,
    prompt_id: Option<String>,
    agent_id: Option<String>,
    tool_use_id: Option<String>,
    tool_name: Option<String>,
    tool_input: Option<serde_json::Value>,
    #[serde(default)]
    is_interrupt: bool,
    elicitation_id: Option<String>,
    mcp_server_name: Option<String>,
}

struct PendingTool {
    name: String,
    input: Option<serde_json::Value>,
}

#[derive(Default)]
struct ClaudeObservation {
    value: AgentObservation,
    session_id: Option<String>,
    prompt_id: Option<String>,
    transcript_path: Option<PathBuf>,
    cancelled_tool_result: bool,
    tools: BTreeMap<String, PendingTool>,
    permission_tools: Vec<String>,
    elicitations: BTreeMap<String, String>,
    next_interaction: u64,
}

impl ClaudeObservation {
    fn observe(&mut self, report: StatusReport) -> Option<AgentObservation> {
        if report.agent_id.is_some() || report.hook_event_name.starts_with("Subagent") {
            return None;
        }
        if report.hook_event_name == "SessionStart" {
            let owned = self.value.source == ObservationSource::Hook;
            self.session_id = report.session_id;
            self.transcript_path = report.transcript_path;
            self.prompt_id = None;
            self.clear_turn();
            self.value.activity = Activity::Unavailable;
            self.value.outcome = None;
            return owned.then(|| self.value.clone());
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
        match report.hook_event_name.as_str() {
            "UserPromptSubmit" => {
                self.clear_turn();
                self.work();
            }
            "PreToolUse" => {
                self.work();
                if let (Some(id), Some(name)) = (report.tool_use_id, report.tool_name) {
                    match name.as_str() {
                        "AskUserQuestion" => self.wait(WaitReason::Answer, vec![id.clone()]),
                        "ExitPlanMode" => self.wait(WaitReason::Approval, vec![id.clone()]),
                        _ => {}
                    }
                    self.tools.insert(
                        id,
                        PendingTool {
                            name,
                            input: report.tool_input,
                        },
                    );
                }
            }
            "PermissionRequest" => {
                if self.value.activity != Activity::Working || self.value.outcome.is_some() {
                    return None;
                }
                let name = report.tool_name?;
                let owners: Vec<_> = self
                    .tools
                    .iter()
                    .filter_map(|(id, tool)| {
                        let matches = if let Some(request_id) = report.tool_use_id.as_ref() {
                            id == request_id
                        } else {
                            tool.name == name && tool.input == report.tool_input
                        };
                        matches.then(|| id.clone())
                    })
                    .collect();
                if owners.is_empty() {
                    return None;
                }
                for id in &owners {
                    if !self.permission_tools.contains(id) {
                        self.permission_tools.push(id.clone());
                    }
                }
                self.wait(
                    if name == "AskUserQuestion" {
                        WaitReason::Answer
                    } else {
                        WaitReason::Approval
                    },
                    owners,
                );
            }
            "Elicitation" => {
                if let Some(server) = report.mcp_server_name {
                    let id = report.elicitation_id.unwrap_or_else(|| {
                        self.next_interaction += 1;
                        format!("server:{server}:{}", self.next_interaction)
                    });
                    self.elicitations.insert(id, server);
                }
                return None;
            }
            "PostToolUse" | "PostToolUseFailure" | "PermissionDenied" => {
                if let Some(id) = report.tool_use_id {
                    self.tools.remove(&id);
                    self.permission_tools.retain(|owner| *owner != id);
                    self.resolve(&id);
                }
                if report.is_interrupt {
                    self.value.activity = if self.value.needs_you() {
                        Activity::Unavailable
                    } else {
                        Activity::Ready
                    };
                    self.value.outcome = Some(TurnOutcome::Interrupted);
                } else if self.value.outcome.is_none() {
                    self.work();
                }
            }
            "ElicitationResult" => {
                let id = report.elicitation_id.or_else(|| {
                    let server = report.mcp_server_name?;
                    let mut candidates = self
                        .elicitations
                        .iter()
                        .filter(|(_, owner)| **owner == server);
                    let (id, _) = candidates.next()?;
                    candidates.next().is_none().then(|| id.clone())
                });
                let id = id?;
                self.elicitations.remove(&id)?;
                self.resolve(&id);
                if self.value.outcome.is_none() {
                    self.work();
                }
            }
            "Notification" => match report.notification_type.as_deref() {
                Some("permission_prompt") => {
                    if self.value.activity != Activity::Working {
                        return None;
                    }
                    let mut owners = self.permission_tools.clone();
                    for (id, tool) in &self.tools {
                        if matches!(
                            tool.name.as_str(),
                            "AskUserQuestion" | "ExitPlanMode" | "EnterPlanMode"
                        ) && !owners.contains(id)
                        {
                            owners.push(id.clone());
                        }
                    }
                    let reason = if !owners.is_empty()
                        && owners.iter().all(|id| {
                            self.tools
                                .get(id)
                                .is_some_and(|tool| tool.name == "AskUserQuestion")
                        }) {
                        WaitReason::Answer
                    } else {
                        WaitReason::Approval
                    };
                    if owners.is_empty()
                        || owners.iter().any(|owner| {
                            !self
                                .value
                                .interactions
                                .iter()
                                .any(|wait| wait.owners.contains(owner))
                        })
                    {
                        self.wait(reason, owners);
                    }
                }
                Some("elicitation_dialog") => {
                    if self.value.activity != Activity::Working || self.elicitations.is_empty() {
                        return None;
                    }
                    self.wait(
                        WaitReason::Answer,
                        self.elicitations.keys().cloned().collect(),
                    );
                }
                Some("idle_prompt") if !self.value.needs_you() => {
                    self.value.activity = Activity::Ready;
                }
                _ => return None,
            },
            "Stop" | "StopFailure" => {
                self.clear_turn();
                self.value.activity = Activity::Ready;
                if self.value.outcome != Some(TurnOutcome::Interrupted) {
                    self.value.outcome = Some(if report.hook_event_name == "Stop" {
                        TurnOutcome::Completed
                    } else {
                        TurnOutcome::Failed
                    });
                }
            }
            _ => return None,
        }
        self.value.source = ObservationSource::Hook;
        Some(self.value.clone())
    }

    fn work(&mut self) {
        self.value.activity = Activity::Working;
        self.value.outcome = None;
        self.cancelled_tool_result = false;
    }

    fn clear_turn(&mut self) {
        self.cancelled_tool_result = false;
        self.value.interactions.clear();
        self.tools.clear();
        self.permission_tools.clear();
        self.elicitations.clear();
    }

    fn observe_transcript(&mut self, entry: &serde_json::Value) -> Option<AgentObservation> {
        let session_id = self.session_id.as_deref()?;
        if entry.get("sessionId").and_then(|value| value.as_str()) != Some(session_id)
            || entry.get("isSidechain").and_then(|value| value.as_bool()) == Some(true)
            || entry
                .get("promptId")
                .and_then(|value| value.as_str())
                .is_some_and(|id| {
                    self.prompt_id
                        .as_deref()
                        .is_some_and(|current| current != id)
                })
        {
            return None;
        }
        let before = self.value.clone();
        if entry.get("type").and_then(|value| value.as_str()) == Some("user") {
            if let Some(blocks) = entry
                .pointer("/message/content")
                .and_then(|value| value.as_array())
            {
                for block in blocks {
                    if block.get("type").and_then(|value| value.as_str()) != Some("tool_result") {
                        continue;
                    }
                    let Some(id) = block.get("tool_use_id").and_then(|value| value.as_str()) else {
                        continue;
                    };
                    if !self
                        .value
                        .interactions
                        .iter()
                        .any(|wait| wait.owners.iter().any(|owner| owner == id))
                    {
                        continue;
                    }
                    self.tools.remove(id);
                    self.permission_tools.retain(|owner| owner != id);
                    self.resolve(id);
                    if entry.get("toolDenialKind").and_then(|value| value.as_str())
                        == Some("user-rejected")
                    {
                        self.cancelled_tool_result = true;
                        self.value.activity = Activity::Unavailable;
                        self.value.outcome = Some(TurnOutcome::Interrupted);
                    }
                }
            }
        } else if entry.get("type").and_then(|value| value.as_str()) == Some("system")
            && entry.get("subtype").and_then(|value| value.as_str()) == Some("turn_duration")
            && self.cancelled_tool_result
            && !self.value.needs_you()
        {
            self.value.activity = Activity::Ready;
            self.cancelled_tool_result = false;
        }
        (before != self.value).then(|| self.value.clone())
    }

    fn resolve(&mut self, id: &str) {
        self.value.interactions.retain_mut(|wait| {
            if !wait.owners.iter().any(|owner| owner == id) {
                return true;
            }
            wait.owners.retain(|owner| owner != id);
            !wait.owners.is_empty()
        });
    }

    fn wait(&mut self, reason: WaitReason, owners: Vec<String>) {
        if let Some(wait) = self.value.interactions.iter_mut().find(|wait| {
            wait.reason == reason
                && (wait.owners == owners || wait.owners.iter().any(|owner| owners.contains(owner)))
        }) {
            for owner in owners {
                if !wait.owners.contains(&owner) {
                    wait.owners.push(owner);
                }
            }
            return;
        }
        self.next_interaction += 1;
        self.value.interactions.push(HumanInteraction {
            id: format!("claude-{}", self.next_interaction),
            reason,
            owners,
            since: chrono::Utc::now().timestamp_millis(),
        });
    }
}

#[cfg(test)]
fn parse_transition(line: &[u8], generation: &str) -> Option<RunnerStatus> {
    let report: serde_json::Value = serde_json::from_slice(line).ok()?;
    if report.get("generation")?.as_str()? != generation {
        return None;
    }
    let value = ClaudeObservation::default().observe(serde_json::from_value(report).ok()?)?;
    if value.needs_you() {
        return None;
    }
    Some(if value.activity == Activity::Working {
        RunnerStatus::Busy
    } else {
        RunnerStatus::Idle
    })
}

pub(crate) struct ClaudeStatusWatcher {
    feed: HookFeed,
    observation: ClaudeObservation,
    interrupt: Arc<AtomicU8>,
    transcript: Option<TranscriptTail>,
}

impl ClaudeStatusWatcher {
    pub(crate) fn start(path: &Path, generation: String) -> Result<Self> {
        Ok(Self {
            feed: HookFeed::start(path, generation, APPEND_SCRIPT)?,
            observation: ClaudeObservation::default(),
            interrupt: Arc::new(AtomicU8::new(0)),
            transcript: None,
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
        // Keep buffered tool hooks ahead of the interrupt on the same output channel.
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
            transition(self.observation.value.clone(), source);
        }
        self.drain_transcript(&mut transition);
        Ok(())
    }

    fn drain_transcript(&mut self, transition: &mut impl FnMut(AgentObservation, &'static str)) {
        if !self.observation.value.needs_you() && !self.observation.cancelled_tool_result {
            return;
        }
        let Some(path) = self.observation.transcript_path.as_ref() else {
            return;
        };
        if self
            .transcript
            .as_ref()
            .is_none_or(|tail| tail.path != *path)
        {
            self.transcript = TranscriptTail::open(path).ok();
        }
        let Some(tail) = self.transcript.as_mut() else {
            return;
        };
        while let Ok(count) = tail.reader.read_until(b'\n', &mut tail.pending) {
            if count == 0 || tail.pending.last() != Some(&b'\n') {
                break;
            }
            if let Ok(entry) = serde_json::from_slice(&tail.pending) {
                if let Some(value) = self.observation.observe_transcript(&entry) {
                    transition(value, "hook");
                }
            }
            tail.pending.clear();
        }
    }
    #[cfg(test)]
    fn drain(&mut self, mut transition: impl FnMut(RunnerStatus, &'static str)) -> Result<()> {
        self.drain_observations(|value, source| {
            transition(
                if value.activity == Activity::Working {
                    RunnerStatus::Busy
                } else {
                    RunnerStatus::Idle
                },
                source,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    #[cfg(unix)]
    use std::process::{Command, Stdio};

    use super::*;

    fn report(event: &str, notification: Option<&str>, generation: &str) -> String {
        serde_json::json!({
            "generation": generation,
            "hook_event_name": event,
            "notification_type": notification,
        })
        .to_string()
    }

    #[cfg(unix)]
    fn run_hook(path: &Path, event: &str, payload: &[u8]) {
        let mut child = Command::new("/bin/sh")
            .args(["-c", &hook_command(path, event)])
            .env(GENERATION_ENV, "current")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(payload).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{event}: {output:?}");
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }

    fn observe(
        model: &mut ClaudeObservation,
        event: &str,
        fields: serde_json::Value,
    ) -> Option<AgentObservation> {
        let mut fields = fields;
        fields["hook_event_name"] = event.into();
        model.observe(serde_json::from_value(fields).unwrap())
    }

    #[test]
    fn windows_is_explicitly_baseline_only() {
        assert!(!hooks_supported(true));
        assert!(hooks_supported(false));
        assert_eq!(hooks_supported(cfg!(windows)), cfg!(unix));
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
        let path = status_path(root.path(), "question");
        let mut watcher = ClaudeStatusWatcher::start(&path, "current".into()).unwrap();
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        for payload in [
            serde_json::json!({"hook_event_name":"SessionStart","session_id":"main","transcript_path":transcript}),
            serde_json::json!({"hook_event_name":"UserPromptSubmit","prompt_id":"turn"}),
            serde_json::json!({"hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_use_id":"question"}),
        ] {
            let mut payload = payload;
            payload["generation"] = "current".into();
            writeln!(file, "{payload}").unwrap();
        }
        let mut observations = Vec::new();
        watcher
            .drain_observations(|value, _| observations.push(value))
            .unwrap();
        assert!(observations.last().unwrap().needs_you());
        watcher
            .interrupt_signal()
            .store(ESCAPE_INTERRUPT, Ordering::Release);
        watcher
            .drain_observations(|value, _| observations.push(value))
            .unwrap();
        assert!(observations.last().unwrap().needs_you());

        let result = serde_json::json!({"type":"user","sessionId":"main","promptId":"turn","isSidechain":false,"toolDenialKind":"user-rejected","message":{"content":[{"type":"tool_result","tool_use_id":"question","is_error":true}]}}).to_string();
        write!(transcript_file, "{}", &result[..20]).unwrap();
        watcher.feed.dirty.store(false, Ordering::Release);
        watcher
            .drain_observations(|value, _| observations.push(value))
            .unwrap();
        assert!(observations.last().unwrap().needs_you());
        writeln!(transcript_file, "{}", &result[20..]).unwrap();
        watcher
            .drain_observations(|value, _| observations.push(value))
            .unwrap();
        assert!(!observations.last().unwrap().needs_you());
        assert_eq!(observations.last().unwrap().activity, Activity::Unavailable);
        writeln!(
            transcript_file,
            "{}",
            serde_json::json!({"type":"system","subtype":"turn_duration","sessionId":"main"})
        )
        .unwrap();
        watcher
            .drain_observations(|value, _| observations.push(value))
            .unwrap();
        assert_eq!(observations.last().unwrap().activity, Activity::Ready);
        assert_eq!(
            observations.last().unwrap().outcome,
            Some(TurnOutcome::Interrupted)
        );
        assert!(!watcher.observation.cancelled_tool_result);
        for interrupt in [ESCAPE_INTERRUPT, CTRL_C_INTERRUPT] {
            watcher
                .interrupt_signal()
                .store(interrupt, Ordering::Release);
            watcher
                .drain_observations(|value, _| observations.push(value))
                .unwrap();
            assert_eq!(observations.last().unwrap().activity, Activity::Ready);
            assert_eq!(
                observations.last().unwrap().outcome,
                Some(TurnOutcome::Interrupted)
            );
        }
        writeln!(
            file,
            "{}",
            report("Notification", Some("permission_prompt"), "current")
        )
        .unwrap();
        watcher.feed.dirty.store(true, Ordering::Release);
        watcher
            .drain_observations(|value, _| observations.push(value))
            .unwrap();
        assert!(!observations.last().unwrap().needs_you());
        assert!(!observations
            .iter()
            .any(|value| value.outcome == Some(TurnOutcome::Completed)));
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
        let path = status_path(root.path(), "missing-transcript");
        let mut watcher = ClaudeStatusWatcher::start(&path, "current".into()).unwrap();
        watcher.observation = model;
        watcher.observation.transcript_path = Some(root.path().join("missing.jsonl"));
        observe(
            &mut watcher.observation,
            "PreToolUse",
            serde_json::json!({"tool_name":"AskUserQuestion","tool_use_id":"next"}),
        );
        watcher.drain_observations(|_, _| {}).unwrap();
        assert!(watcher.observation.value.needs_you());
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{}", serde_json::json!({"generation":"current","hook_event_name":"PostToolUse","tool_use_id":"next"})).unwrap();
        watcher.feed.dirty.store(true, Ordering::Release);
        watcher.drain_observations(|_, _| {}).unwrap();
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
    #[cfg(unix)]
    fn approval_hook_is_immediate_and_escape_uses_the_question_resolution_path() {
        use std::sync::atomic::Ordering;

        let root = tempfile::tempdir().unwrap();
        let path = status_path(root.path(), "approval");
        let transcript = root.path().join("claude.jsonl");
        let mut file = File::create(&transcript).unwrap();
        let mut watcher = ClaudeStatusWatcher::start(&path, "current".into()).unwrap();
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
            run_hook(&path, event, fields.to_string().as_bytes());
        }
        watcher.drain_observations(|_, _| {}).unwrap();
        assert_eq!(
            watcher.observation.value.interactions[0].reason,
            WaitReason::Approval
        );
        assert_eq!(
            watcher.observation.value.interactions[0].owners,
            ["command"]
        );
        watcher
            .interrupt_signal()
            .store(ESCAPE_INTERRUPT, Ordering::Release);
        watcher.drain_observations(|_, _| {}).unwrap();
        assert!(watcher.observation.value.needs_you());

        writeln!(file, "{}", serde_json::json!({"type":"user","sessionId":"main","promptId":"turn","toolDenialKind":"user-rejected","message":{"content":[{"type":"tool_result","tool_use_id":"command"}]}})).unwrap();
        writeln!(file, "{}", serde_json::json!({"type":"system","subtype":"turn_duration","sessionId":"main","promptId":"turn"})).unwrap();
        watcher.drain_observations(|_, _| {}).unwrap();
        assert!(!watcher.observation.value.needs_you());
        assert_eq!(watcher.observation.value.activity, Activity::Ready);
        assert_eq!(
            watcher.observation.value.outcome,
            Some(TurnOutcome::Interrupted)
        );
        watcher
            .interrupt_signal()
            .store(ESCAPE_INTERRUPT, Ordering::Release);
        watcher.drain_observations(|_, _| {}).unwrap();
        assert_eq!(watcher.observation.value.activity, Activity::Ready);
    }

    #[test]
    fn unreadable_payload_is_skipped_without_losing_the_next_record_or_bridge() {
        let root = tempfile::tempdir().unwrap();
        let path = status_path(root.path(), "session");
        let mut watcher = ClaudeStatusWatcher::start(&path, "current".into()).unwrap();
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{}", serde_json::json!({"generation":"current","hook_event_name":"Stop","payload_file":"session.ndjson.missing"})).unwrap();
        writeln!(file, "{}", report("UserPromptSubmit", None, "current")).unwrap();
        writeln!(file, "{}", report("Stop", None, "current")).unwrap();
        let mut observations = Vec::new();
        watcher
            .drain_observations(|value, _| observations.push(value))
            .unwrap();
        assert_eq!(observations.len(), 2);
        assert_eq!(observations[0].activity, Activity::Working);
        assert_eq!(observations[1].activity, Activity::Ready);
        assert!(watcher.feed.pending.is_empty());
        fs::remove_file(&path).unwrap();
        watcher.feed.dirty.store(true, Ordering::Release);
        assert!(watcher.drain_observations(|_, _| {}).is_err());
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
        let root = tempfile::tempdir().unwrap();
        let path = status_path(root.path(), "dialog");
        let mut watcher = ClaudeStatusWatcher::start(&path, "current".into()).unwrap();
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, r#"{{"generation":"current","hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_use_id":"question"}}"#).unwrap();
        writeln!(
            file,
            "{}",
            report("Notification", Some("permission_prompt"), "current")
        )
        .unwrap();
        watcher
            .interrupt_signal()
            .store(ESCAPE_INTERRUPT, Ordering::Release);
        let mut observations = Vec::new();
        watcher
            .drain_observations(|value, _| observations.push(value))
            .unwrap();
        let interrupted = observations.last().unwrap();
        assert!(interrupted.needs_you());
        assert_eq!(interrupted.outcome, Some(TurnOutcome::Interrupted));
        assert_eq!(interrupted.activity, Activity::Unavailable);
        writeln!(file, "{}", report("Stop", None, "current")).unwrap();
        watcher.feed.dirty.store(true, Ordering::Release);
        watcher
            .drain_observations(|value, _| observations.push(value))
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
            ("UserPromptSubmit", None, Some(RunnerStatus::Busy)),
            ("PreToolUse", None, Some(RunnerStatus::Busy)),
            ("PostToolUse", None, Some(RunnerStatus::Busy)),
            (
                "Notification",
                Some("idle_prompt"),
                Some(RunnerStatus::Idle),
            ),
            ("Notification", Some("permission_prompt"), None),
            ("Notification", Some("agent_completed"), None),
            ("Notification", None, None),
            ("Stop", None, Some(RunnerStatus::Idle)),
            ("StopFailure", None, Some(RunnerStatus::Idle)),
            ("SubagentStop", None, None),
            ("SessionStart", None, None),
            ("unknown", None, None),
        ] {
            assert_eq!(
                parse_transition(report(event, notification, "current").as_bytes(), "current"),
                expected,
                "{event} {notification:?}"
            );
        }
        assert_eq!(parse_transition(b"not JSON\n", "current"), None);
        assert_eq!(
            parse_transition(
                report("Notification", Some("idle_prompt"), "old").as_bytes(),
                "current"
            ),
            None
        );
    }

    #[test]
    fn watcher_preserves_partial_lines_and_skips_stale_or_invalid_records() {
        let root = tempfile::tempdir().unwrap();
        let path = status_path(root.path(), "session");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!(
                "{}\n",
                report("Notification", Some("idle_prompt"), "current")
            ),
        )
        .unwrap();
        let mut watcher = ClaudeStatusWatcher::start(&path, "current".into()).unwrap();
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        let prompt = report("UserPromptSubmit", None, "current");
        write!(file, "{}", &prompt[..10]).unwrap();
        let mut states = Vec::new();
        watcher.drain(|state, _| states.push(state)).unwrap();
        assert!(states.is_empty());

        writeln!(file, "{}", &prompt[10..]).unwrap();
        writeln!(file, "broken").unwrap();
        writeln!(
            file,
            "{}",
            report("Notification", Some("idle_prompt"), "old")
        )
        .unwrap();
        writeln!(file, "{}", report("Stop", None, "current")).unwrap();
        writeln!(
            file,
            "{}",
            report("Notification", Some("idle_prompt"), "current")
        )
        .unwrap();
        watcher.feed.dirty.store(true, Ordering::Release);
        watcher.drain(|state, _| states.push(state)).unwrap();
        assert_eq!(
            states,
            [RunnerStatus::Busy, RunnerStatus::Idle, RunnerStatus::Idle]
        );
        watcher.feed.dirty.store(true, Ordering::Release);
        watcher.drain(|state, _| states.push(state)).unwrap();
        assert_eq!(states.len(), 3);
        drop(file);
        drop(watcher);
        assert!(!path.exists());
        assert!(!script_path(&path).exists());
    }

    #[test]
    fn ordinary_interrupt_recovers_idle_without_a_transcript_or_completion_hook() {
        for kind in [ESCAPE_INTERRUPT, CTRL_C_INTERRUPT] {
            let root = tempfile::tempdir().unwrap();
            let path = status_path(root.path(), "early-interrupt");
            let mut watcher = ClaudeStatusWatcher::start(&path, "current".into()).unwrap();
            let mut file = OpenOptions::new().append(true).open(&path).unwrap();
            writeln!(file, "{}", serde_json::json!({"generation":"current","hook_event_name":"UserPromptSubmit","prompt_id":"turn"})).unwrap();
            watcher.interrupt_signal().store(kind, Ordering::Release);
            let mut observations = Vec::new();
            watcher
                .drain_observations(|value, _| observations.push(value))
                .unwrap();
            assert_eq!(observations.len(), 2);
            assert_eq!(observations[0].activity, Activity::Working);
            let interrupted = observations.last().unwrap();
            assert_eq!(interrupted.activity, Activity::Ready);
            assert_eq!(interrupted.outcome, Some(TurnOutcome::Interrupted));
            assert!(!interrupted.needs_you());

            watcher.interrupt_signal().store(kind, Ordering::Release);
            watcher
                .drain_observations(|_, _| panic!("repeated interrupt must preserve Idle"))
                .unwrap();
            for event in ["PostToolUse", "PostToolUseFailure", "Notification", "Stop"] {
                writeln!(file, "{}", serde_json::json!({"generation":"current","hook_event_name":event,"prompt_id":"turn","tool_use_id":"late","is_interrupt":event == "PostToolUseFailure","notification_type":"permission_prompt"})).unwrap();
                watcher.feed.dirty.store(true, Ordering::Release);
                watcher
                    .drain_observations(|value, _| {
                        assert_eq!(value.activity, Activity::Ready);
                        assert_eq!(value.outcome, Some(TurnOutcome::Interrupted));
                        assert!(!value.needs_you());
                    })
                    .unwrap();
            }
            writeln!(file, "{}", serde_json::json!({"generation":"current","hook_event_name":"UserPromptSubmit","prompt_id":"next"})).unwrap();
            watcher.feed.dirty.store(true, Ordering::Release);
            watcher
                .drain_observations(|value, _| observations.push(value))
                .unwrap();
            assert_eq!(observations.last().unwrap().activity, Activity::Working);
            assert_eq!(observations.last().unwrap().outcome, None);
        }
    }

    #[test]
    fn interrupt_drains_buffered_tool_records_before_idle_and_allows_new_work() {
        for (kind, source) in [
            (CTRL_C_INTERRUPT, "input-interrupt"),
            (ESCAPE_INTERRUPT, "input-escape"),
            (CTRL_C_INTERRUPT | ESCAPE_INTERRUPT, "input-interrupt"),
        ] {
            let root = tempfile::tempdir().unwrap();
            let path = status_path(root.path(), "interrupt");
            let mut watcher = ClaudeStatusWatcher::start(&path, "current".into()).unwrap();
            watcher.drain(|_, _| panic!("no records yet")).unwrap();
            let mut file = OpenOptions::new().append(true).open(&path).unwrap();
            writeln!(file, "{}", report("PreToolUse", None, "current")).unwrap();
            watcher.feed.dirty.store(false, Ordering::Release);
            watcher.interrupt_signal().store(kind, Ordering::Release);
            let mut transitions = Vec::new();
            watcher
                .drain(|state, source| transitions.push((state, source)))
                .unwrap();
            assert_eq!(
                transitions,
                [(RunnerStatus::Busy, "hook"), (RunnerStatus::Idle, source)]
            );
            writeln!(file, "{}", report("PreToolUse", None, "current")).unwrap();
            watcher.feed.dirty.store(true, Ordering::Release);
            watcher
                .drain(|state, source| transitions.push((state, source)))
                .unwrap();
            assert_eq!(transitions.last(), Some(&(RunnerStatus::Busy, "hook")));
            assert_eq!(transitions.len(), 3);
        }
    }

    #[test]
    #[cfg(unix)]
    fn notification_script_uses_payload_type_even_when_matcher_is_bypassed() {
        let root = tempfile::tempdir().unwrap();
        let path = status_path(root.path(), "notifications");
        let mut watcher = ClaudeStatusWatcher::start(&path, "current".into()).unwrap();
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
                run_hook(&path, "Notification", text.as_bytes());
                let mut observations = Vec::new();
                watcher.feed.dirty.store(true, Ordering::Release);
                watcher
                    .drain_observations(|value, _| observations.push(value))
                    .unwrap();
                assert_eq!(observations.len(), usize::from(kind == Some("idle_prompt")));
                if let Some(value) = observations.first() {
                    assert_eq!(value.activity, Activity::Ready);
                }
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn stop_followed_by_pre_tool_use_ends_busy() {
        let root = tempfile::tempdir().unwrap();
        let path = status_path(root.path(), "continuation");
        let mut watcher = ClaudeStatusWatcher::start(&path, "current".into()).unwrap();
        for event in ["Stop", "PreToolUse"] {
            run_hook(&path, event, b"{}");
        }
        let mut states = Vec::new();
        watcher.drain(|state, _| states.push(state)).unwrap();
        assert_eq!(states, [RunnerStatus::Idle, RunnerStatus::Busy]);
    }

    #[test]
    #[cfg(unix)]
    fn tool_hooks_drain_payloads_larger_than_the_pipe_buffer() {
        let root = tempfile::tempdir().unwrap();
        let path = status_path(root.path(), "large-payload");
        let mut watcher = ClaudeStatusWatcher::start(&path, "current".into()).unwrap();
        let payload = serde_json::json!({"tool_response": "x".repeat(2 * 1024 * 1024)}).to_string();
        for event in [
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "Stop",
            "StopFailure",
        ] {
            run_hook(&path, event, payload.as_bytes());
        }
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 5);
        let mut observations = Vec::new();
        watcher
            .drain_observations(|value, _| observations.push(value))
            .unwrap();
        assert_eq!(observations.len(), 5);
        // A failed bridge setup leaves no helper; the command must still consume stdin.
        run_hook(
            &status_path(root.path(), "missing-helper"),
            "PostToolUse",
            payload.as_bytes(),
        );
    }

    #[test]
    fn startup_clears_status_files_left_by_a_crash() {
        let root = tempfile::tempdir().unwrap();
        clear_leftovers(root.path()).unwrap();
        let path = status_path(root.path(), "stale");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"stale report").unwrap();
        fs::write(script_path(&path), APPEND_SCRIPT).unwrap();
        fs::write(path.with_extension("sh.tmp"), APPEND_SCRIPT).unwrap();
        let unremovable = path.with_file_name("directory.sh");
        fs::create_dir(&unremovable).unwrap();
        clear_leftovers(root.path()).unwrap();
        assert!(unremovable.is_dir());
        assert!(!path.exists());
        assert!(!script_path(&path).exists());
        assert!(!path.with_extension("sh.tmp").exists());
    }

    #[test]
    fn failed_setup_removes_its_helper() {
        let root = tempfile::tempdir().unwrap();
        let path = status_path(root.path(), "bad-file");
        fs::create_dir_all(&path).unwrap();
        assert!(ClaudeStatusWatcher::start(&path, "current".into()).is_err());
        assert!(!script_path(&path).exists());
        assert!(!path.with_extension("sh.tmp").exists());
    }

    #[test]
    #[cfg(unix)]
    fn injected_commands_append_silently_and_fail_open() {
        let root = tempfile::tempdir().unwrap();
        let path = status_path(&root.path().join("Jason's status $dir"), "session");
        let mut watcher = ClaudeStatusWatcher::start(&path, "current".into()).unwrap();
        for event in [
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "Notification",
            "Stop",
            "StopFailure",
        ] {
            run_hook(&path, event, br#"{"notification_type":"idle_prompt"}"#);
        }
        let records = fs::read_to_string(&path).unwrap();
        let mut states = Vec::new();
        watcher.drain(|state, _| states.push(state)).unwrap();
        assert_eq!(records.lines().count(), 6);
        assert_eq!(
            states,
            [
                RunnerStatus::Busy,
                RunnerStatus::Busy,
                RunnerStatus::Busy,
                RunnerStatus::Idle,
                RunnerStatus::Idle,
                RunnerStatus::Idle,
            ]
        );

        let output = std::process::Command::new("/bin/sh")
            .args(["-c", &hook_command(root.path(), "Stop")])
            .env(GENERATION_ENV, "current")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
}
