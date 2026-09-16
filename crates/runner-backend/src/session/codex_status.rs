use std::collections::BTreeSet;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use super::hook_feed::{self, HookFeed, TranscriptTail};
use super::status::{Activity, AgentObservation, ObservationSource, TurnOutcome};
use crate::error::Result;

pub(crate) const PATH_ENV: &str = "RUNNER_CODEX_STATUS_PATH";
pub(crate) const GENERATION_ENV: &str = "RUNNER_CODEX_STATUS_GENERATION";
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

const APPEND_SCRIPT: &str = r#"#!/bin/sh
payload=$(mktemp "$1.XXXXXXXX") || { cat >/dev/null; exit 0; }
cat >"$payload" || { rm -f "$payload"; exit 0; }
printf '{"generation":"%s","hook_event_name":"%s","payload_file":"%s"}\n' "$RUNNER_CODEX_STATUS_GENERATION" "$2" "${payload##*/}" >> "$1" 2>/dev/null || rm -f "$payload"
exit 0
"#;

/// The Windows reporter, written per session to `<feed>.ps1`: nine copies inline in
/// `-c` values would push an npm `codex.cmd` launch past cmd.exe's 8,191-character line.
pub(crate) fn windows_reporter() -> String {
    hook_feed::powershell_reporter("$args[0]", GENERATION_ENV, "$args[1]")
}

pub(crate) fn hook_command(path: &Path, event: &str) -> String {
    if cfg!(windows) {
        // A script block is not subject to execution policy, unlike running the file.
        return format!(
            "try{{& ([ScriptBlock]::Create([IO.File]::ReadAllText({}))) {} {}}}\
             catch{{[Console]::OpenStandardInput().CopyTo([IO.Stream]::Null)}};'{{}}'",
            hook_feed::powershell_quote(&hook_feed::hook_path(&hook_feed::powershell_script_path(
                path
            ))),
            hook_feed::powershell_quote(&hook_feed::hook_path(path)),
            hook_feed::powershell_quote(event),
        );
    }
    format!(
        "({}); printf '{{}}\\n'; exit 0",
        hook_feed::hook_command(path, event)
    )
}

#[derive(Default, Deserialize)]
struct StatusReport {
    #[serde(default)]
    hook_event_name: String,
    session_id: Option<String>,
    turn_id: Option<String>,
    transcript_path: Option<PathBuf>,
    agent_id: Option<String>,
}

#[derive(Default)]
struct CodexObservation {
    value: AgentObservation,
    session_id: Option<String>,
    turn_id: Option<String>,
    retired_turns: BTreeSet<String>,
    ended: bool,
    transcript_path: Option<PathBuf>,
}

impl CodexObservation {
    fn observe(&mut self, report: StatusReport) -> Option<AgentObservation> {
        if report.agent_id.is_some() || !EVENTS.contains(&report.hook_event_name.as_str()) {
            return None;
        }
        let session_id = report.session_id.filter(|id| !id.is_empty())?;
        if self.session_id.as_ref().is_some_and(|id| *id != session_id) {
            if report.hook_event_name != "SessionStart" {
                return None;
            }
            self.turn_id = None;
            self.retired_turns.clear();
            self.ended = false;
            self.transcript_path = None;
        }
        self.session_id = Some(session_id);
        if report.hook_event_name == "SessionStart" {
            // Delayed startup/resume and compaction hooks cannot reset a running turn.
            if self.turn_id.is_some() || self.ended {
                return None;
            }
            self.transcript_path = report.transcript_path;
            self.value.activity = Activity::Unavailable;
            self.value.outcome = None;
            return (self.value.source == ObservationSource::Hook).then(|| self.value.clone());
        }
        if self.ended {
            return None;
        }
        if report.hook_event_name == "SessionEnd" {
            self.ended = true;
            self.value.activity = Activity::Unavailable;
            return (self.value.source == ObservationSource::Hook).then(|| self.value.clone());
        }
        let turn_id = report.turn_id.filter(|id| !id.is_empty())?;
        if self.retired_turns.contains(&turn_id) {
            return None;
        }
        if report.hook_event_name == "UserPromptSubmit" {
            if self.turn_id.as_ref() == Some(&turn_id) {
                return None;
            }
            if let Some(previous) = self.turn_id.replace(turn_id) {
                self.retired_turns.insert(previous);
            }
            self.value.outcome = None;
            self.value.activity = Activity::Working;
        } else {
            if self.turn_id.as_ref() != Some(&turn_id) {
                return None;
            }
            match report.hook_event_name.as_str() {
                "Interrupt" => {
                    if self.value.outcome == Some(TurnOutcome::Interrupted) {
                        return None;
                    }
                    self.value.activity = Activity::Unavailable;
                    self.value.outcome = Some(TurnOutcome::Interrupted);
                }
                "Stop" => {
                    if self.value.outcome == Some(TurnOutcome::Interrupted) {
                        return None;
                    }
                    self.value.activity = Activity::Ready;
                    self.value.outcome = Some(TurnOutcome::Completed);
                }
                "PreToolUse" | "PreCompact" => {
                    if self.value.outcome == Some(TurnOutcome::Interrupted) {
                        return None;
                    }
                    self.value.activity = Activity::Working;
                    self.value.outcome = None;
                }
                "PostToolUse" | "PostCompact" => {
                    if self.value.outcome.is_some() {
                        return None;
                    }
                    self.value.activity = Activity::Working;
                }
                _ => return None,
            }
        }
        if let Some(path) = report.transcript_path {
            self.transcript_path = Some(path);
        }
        self.value.source = ObservationSource::Hook;
        Some(self.value.clone())
    }

    fn awaiting_abort(&self) -> bool {
        !self.ended
            && self.value.outcome == Some(TurnOutcome::Interrupted)
            && self.value.activity == Activity::Unavailable
    }

    fn observe_transcript(&mut self, record: &Value) {
        let payload = &record["payload"];
        // Interrupt hooks run before Codex publishes the native abort boundary.
        if self.awaiting_abort()
            && record["type"] == "event_msg"
            && payload["type"] == "turn_aborted"
            && payload["reason"] == "interrupted"
            && payload["turn_id"].as_str().is_some()
            && payload["turn_id"].as_str() == self.turn_id.as_deref()
        {
            self.value.activity = Activity::Ready;
        }
    }
}

pub(crate) struct CodexStatusWatcher {
    feed: HookFeed,
    observation: CodexObservation,
    transcript: Option<TranscriptTail>,
}

impl CodexStatusWatcher {
    pub(crate) fn start(path: &Path, generation: String) -> Result<Self> {
        Ok(Self {
            feed: if cfg!(windows) {
                HookFeed::start_powershell(path, generation, &windows_reporter())?
            } else {
                HookFeed::start(path, generation, APPEND_SCRIPT)?
            },
            observation: CodexObservation::default(),
            transcript: None,
        })
    }

    pub(crate) fn drain_observations(
        &mut self,
        mut transition: impl FnMut(AgentObservation, &'static str),
    ) -> Result<()> {
        self.feed.drain(false, |report| {
            if let Ok(report) = serde_json::from_value(report) {
                if let Some(value) = self.observation.observe(report) {
                    transition(value, "hook");
                }
            }
        })?;
        if !self.observation.awaiting_abort() {
            return Ok(());
        }
        let Some(path) = self.observation.transcript_path.as_ref() else {
            return Ok(());
        };
        if self
            .transcript
            .as_ref()
            .is_none_or(|tail| tail.path != *path)
        {
            self.transcript = TranscriptTail::open(path).ok();
        }
        let Some(tail) = self.transcript.as_mut() else {
            return Ok(());
        };
        let before = self.observation.value.clone();
        while let Ok(count) = tail.reader.read_until(b'\n', &mut tail.pending) {
            if count == 0 || tail.pending.last() != Some(&b'\n') {
                break;
            }
            if let Ok(record) = serde_json::from_slice(&tail.pending) {
                self.observation.observe_transcript(&record);
            }
            tail.pending.clear();
        }
        if before != self.observation.value {
            transition(self.observation.value.clone(), "hook");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::sync::atomic::Ordering;

    fn report(event: &str, turn: &str) -> Value {
        json!({"hook_event_name":event,"session_id":"main","turn_id":turn,"generation":"current"})
    }

    fn observe(state: &mut CodexObservation, value: Value) -> Option<AgentObservation> {
        state.observe(serde_json::from_value(value).unwrap())
    }

    fn abort(turn: &str) -> Value {
        json!({"type":"event_msg","payload":{"type":"turn_aborted","turn_id":turn,"reason":"interrupted"}})
    }

    #[test]
    fn launch_resume_completion_continuation_and_late_results() {
        for source in ["startup", "resume"] {
            let mut state = CodexObservation::default();
            let mut start = report("SessionStart", "");
            start["source"] = json!(source);
            assert!(observe(&mut state, start.clone()).is_none());
            assert_eq!(state.value.source, ObservationSource::Unavailable);
            assert_eq!(
                observe(&mut state, report("UserPromptSubmit", "one"))
                    .unwrap()
                    .activity,
                Activity::Working
            );
            assert!(observe(&mut state, start).is_none());
            assert_eq!(state.value.activity, Activity::Working);
            for event in ["PreToolUse", "PostToolUse", "PreCompact", "PostCompact"] {
                assert_eq!(
                    observe(&mut state, report(event, "one")).unwrap().activity,
                    Activity::Working
                );
            }
            assert_eq!(
                observe(&mut state, report("Stop", "one")).unwrap().outcome,
                Some(TurnOutcome::Completed)
            );
            assert!(observe(&mut state, report("PostToolUse", "one")).is_none());
            assert!(observe(&mut state, report("UserPromptSubmit", "one")).is_none());
            assert_eq!(state.value.activity, Activity::Ready);
            assert_eq!(
                observe(&mut state, report("PreToolUse", "one"))
                    .unwrap()
                    .outcome,
                None
            );
            assert_eq!(state.value.activity, Activity::Working);
            observe(&mut state, report("Stop", "one"));
            observe(&mut state, report("UserPromptSubmit", "two"));
            for event in EVENTS {
                if *event != "SessionEnd" && *event != "SessionStart" {
                    assert!(
                        observe(&mut state, report(event, "one")).is_none(),
                        "{event}"
                    );
                }
            }
            assert_eq!(state.value.activity, Activity::Working);
        }
    }

    #[test]
    fn child_foreign_session_and_closed_session_isolation() {
        let mut state = CodexObservation::default();
        for event in EVENTS {
            let mut child = report(event, "child");
            child["agent_id"] = json!("child");
            assert!(observe(&mut state, child).is_none());
        }
        observe(&mut state, report("UserPromptSubmit", "one"));
        for event in EVENTS.iter().filter(|event| **event != "SessionStart") {
            let mut foreign = report(event, "other");
            foreign["session_id"] = json!("foreign");
            assert!(observe(&mut state, foreign).is_none());
        }
        observe(&mut state, report("SessionEnd", "one"));
        assert_eq!(state.value.activity, Activity::Unavailable);
        assert!(observe(&mut state, report("UserPromptSubmit", "two")).is_none());
        let mut clear = report("SessionStart", "");
        clear["session_id"] = json!("new");
        clear["source"] = json!("clear");
        observe(&mut state, clear);
        let mut prompt = report("UserPromptSubmit", "fresh");
        prompt["session_id"] = json!("new");
        observe(&mut state, prompt);
        assert!(observe(&mut state, report("Stop", "one")).is_none());
        assert_eq!(state.session_id.as_deref(), Some("new"));
        assert_eq!(state.value.activity, Activity::Working);
    }

    #[test]
    fn root_session_start_handover_is_independent_of_source_and_can_resume_an_old_session() {
        for source in ["startup", "resume", "clear", "compact", "future-source"] {
            let mut state = CodexObservation::default();
            observe(&mut state, report("UserPromptSubmit", "one"));
            observe(&mut state, report("Stop", "one"));
            let mut start = report("SessionStart", "");
            start["session_id"] = json!("new");
            start["source"] = json!(source);
            let handed_over = observe(&mut state, start).unwrap();
            assert_eq!(handed_over.activity, Activity::Unavailable);
            assert_eq!(handed_over.outcome, None);
            assert_eq!(handed_over.source, ObservationSource::Hook);
            for event in ["UserPromptSubmit", "Stop", "Interrupt", "SessionEnd"] {
                assert!(observe(&mut state, report(event, "one")).is_none());
            }
            for event in ["UserPromptSubmit", "Stop"] {
                let mut report = report(event, "two");
                report["session_id"] = json!("new");
                observe(&mut state, report);
            }
            assert_eq!(state.value.activity, Activity::Ready);
            let mut resume = report("SessionStart", "");
            resume["source"] = json!("resume");
            observe(&mut state, resume);
            observe(&mut state, report("UserPromptSubmit", "three"));
            assert_eq!(state.session_id.as_deref(), Some("main"));
            assert_eq!(state.value.activity, Activity::Working);
            assert_eq!(state.value.outcome, None);
        }
    }

    #[test]
    fn interrupt_waits_for_correlated_native_abort_and_never_completes() {
        let mut state = CodexObservation::default();
        observe(&mut state, report("UserPromptSubmit", "one"));
        observe(&mut state, report("Interrupt", "one"));
        assert_eq!(state.value.activity, Activity::Unavailable);
        assert_eq!(state.value.outcome, Some(TurnOutcome::Interrupted));
        state.observe_transcript(&abort("old"));
        assert_eq!(state.value.activity, Activity::Unavailable);
        for event in [
            "Stop",
            "PreToolUse",
            "PostToolUse",
            "PreCompact",
            "PostCompact",
            "Interrupt",
            "UserPromptSubmit",
        ] {
            assert!(
                observe(&mut state, report(event, "one")).is_none(),
                "{event}"
            );
        }
        state.observe_transcript(&abort("one"));
        assert_eq!(state.value.activity, Activity::Ready);
        assert_eq!(state.value.outcome, Some(TurnOutcome::Interrupted));
        assert!(observe(&mut state, report("Interrupt", "one")).is_none());
        observe(&mut state, report("UserPromptSubmit", "two"));
        state.observe_transcript(&abort("one"));
        assert_eq!(state.value.activity, Activity::Working);
        assert_eq!(state.value.outcome, None);
    }

    #[test]
    fn interruption_during_stop_hooks_overrides_provisional_completion() {
        let mut state = CodexObservation::default();
        observe(&mut state, report("UserPromptSubmit", "one"));
        observe(&mut state, report("Stop", "one"));
        assert_eq!(state.value.outcome, Some(TurnOutcome::Completed));
        observe(&mut state, report("Interrupt", "one"));
        assert_eq!(state.value.activity, Activity::Unavailable);
        assert_eq!(state.value.outcome, Some(TurnOutcome::Interrupted));
        state.observe_transcript(&abort("one"));
        assert_eq!(state.value.activity, Activity::Ready);
        assert_eq!(state.value.outcome, Some(TurnOutcome::Interrupted));
        assert!(observe(&mut state, report("Stop", "one")).is_none());
        assert!(observe(&mut state, report("Interrupt", "one")).is_none());
    }

    #[test]
    fn question_candidates_and_raw_permissions_never_prove_a_surfaced_wait() {
        // 0.154.0 emits this same prefix for a shown Plan question and a hook-denied call.
        for mode in ["plan", "default"] {
            for resolution in ["answer", "denial", "cancel"] {
                let mut state = CodexObservation::default();
                observe(&mut state, report("UserPromptSubmit", "one"));
                let mut pre = report("PreToolUse", "one");
                pre["tool_name"] = json!("request_user_input");
                pre["tool_use_id"] = json!("question");
                observe(&mut state, pre);
                state.observe_transcript(&json!({"type":"turn_context","payload":{"turn_id":"one","collaboration_mode":{"mode":mode}}}));
                state.observe_transcript(&json!({"type":"response_item","payload":{"type":"function_call","name":"request_user_input","call_id":"question","internal_chat_message_metadata_passthrough":{"turn_id":"one"}}}));
                assert!(!state.value.needs_you());
                assert!(observe(&mut state, report("PermissionRequest", "one")).is_none());
                state.observe_transcript(&json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"question","output":resolution,"internal_chat_message_metadata_passthrough":{"turn_id":"one"}}}));
                if resolution == "cancel" {
                    observe(&mut state, report("Interrupt", "one"));
                    state.observe_transcript(&abort("one"));
                    assert_eq!(state.value.outcome, Some(TurnOutcome::Interrupted));
                } else {
                    observe(&mut state, report("PostToolUse", "one"));
                    observe(&mut state, report("Stop", "one"));
                    assert_eq!(state.value.outcome, Some(TurnOutcome::Completed));
                }
                assert!(!state.value.needs_you());
            }
        }
    }

    fn append(path: &Path, value: &Value) {
        writeln!(
            OpenOptions::new().append(true).open(path).unwrap(),
            "{value}"
        )
        .unwrap();
    }

    #[test]
    fn watcher_recovers_partial_abort_after_hooks_without_replaying_resumed_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("status.ndjson");
        let transcript = dir.path().join("rollout.jsonl");
        fs::write(&transcript, format!("{}\n", abort("old"))).unwrap();
        let mut watcher = CodexStatusWatcher::start(&path, "current".into()).unwrap();
        for event in ["UserPromptSubmit", "Interrupt"] {
            let mut value = report(event, "one");
            value["transcript_path"] = json!(transcript);
            append(&path, &value);
        }
        let mut values = Vec::new();
        watcher
            .drain_observations(|value, _| values.push(value))
            .unwrap();
        assert_eq!(values.last().unwrap().activity, Activity::Unavailable);
        let line = format!("{}\n", abort("one"));
        let mut file = OpenOptions::new().append(true).open(&transcript).unwrap();
        file.write_all(&line.as_bytes()[..20]).unwrap();
        watcher
            .drain_observations(|_, _| panic!("partial abort"))
            .unwrap();
        file.write_all(&line.as_bytes()[20..]).unwrap();
        watcher
            .drain_observations(|value, _| values.push(value))
            .unwrap();
        assert_eq!(values.last().unwrap().activity, Activity::Ready);
        assert_eq!(
            values.last().unwrap().outcome,
            Some(TurnOutcome::Interrupted)
        );
    }

    #[test]
    fn missing_transcript_preserves_interrupted_unavailable_until_new_work() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("status.ndjson");
        let mut watcher = CodexStatusWatcher::start(&path, "current".into()).unwrap();
        for event in ["UserPromptSubmit", "Interrupt"] {
            let mut value = report(event, "one");
            value["transcript_path"] = json!(dir.path().join("missing"));
            append(&path, &value);
        }
        watcher.drain_observations(|_, _| {}).unwrap();
        assert_eq!(watcher.observation.value.activity, Activity::Unavailable);
        assert_eq!(
            watcher.observation.value.outcome,
            Some(TurnOutcome::Interrupted)
        );
        append(&path, &report("UserPromptSubmit", "two"));
        watcher.feed.dirty.store(true, Ordering::Release);
        watcher.drain_observations(|_, _| {}).unwrap();
        assert_eq!(watcher.observation.value.activity, Activity::Working);
    }

    #[cfg(unix)]
    fn run_hook(path: &Path, event: &str, payload: &[u8]) {
        use std::process::{Command, Stdio};
        let mut child = Command::new("sh")
            .args(["-c", &hook_command(path, event)])
            .env(GENERATION_ENV, "current")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(payload).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"{}\n");
        assert!(output.stderr.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn real_helper_large_malformed_partial_generation_quoting_and_teardown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("quote ' triple ''' dollar $ backtick ` space.ndjson");
        let mut watcher = CodexStatusWatcher::start(&path, "current".into()).unwrap();
        let mut large = report("UserPromptSubmit", "one");
        large["prompt"] = json!("x\n".repeat(128 * 1024));
        run_hook(
            &path,
            "UserPromptSubmit",
            &serde_json::to_vec_pretty(&large).unwrap(),
        );
        run_hook(&path, "Stop", b"not JSON");
        run_hook(&path, "Stop", br#"{"hook_event_name":42}"#);
        run_hook(
            &path,
            "Stop",
            &serde_json::to_vec(&report("PreToolUse", "one")).unwrap(),
        );
        let mut stale = report("Stop", "one");
        stale["generation"] = json!("old");
        append(&path, &stale);
        let stop = format!("{}\n", report("Stop", "one"));
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(&stop.as_bytes()[..20]).unwrap();
        let mut values = Vec::new();
        watcher
            .drain_observations(|value, _| values.push(value))
            .unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].activity, Activity::Working);
        file.write_all(&stop.as_bytes()[20..]).unwrap();
        watcher.feed.dirty.store(true, Ordering::Release);
        watcher
            .drain_observations(|value, _| values.push(value))
            .unwrap();
        assert_eq!(values.last().unwrap().activity, Activity::Ready);
        fs::remove_file(hook_feed::script_path(&path)).unwrap();
        watcher.feed.dirty.store(true, Ordering::Release);
        assert!(watcher.drain_observations(|_, _| {}).is_err());
        drop(watcher);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        run_hook(&path, "Stop", &vec![b'x'; 256 * 1024]);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[cfg(windows)]
    fn run_powershell_hook(shell: &str, path: &Path, event: &str, payload: &[u8]) -> bool {
        let Some(output) = hook_feed::run_powershell(
            shell,
            &hook_command(path, event),
            &[(GENERATION_ENV, "current")],
            payload,
        ) else {
            return false;
        };
        assert!(output.status.success(), "{shell}: {output:?}");
        assert_eq!(output.stdout, b"{}\r\n", "{shell}");
        assert!(output.stderr.is_empty(), "{shell}: {output:?}");
        true
    }

    #[cfg(windows)]
    #[test]
    fn real_powershell_reporter_large_malformed_partial_generation_quoting_and_teardown() {
        for shell in hook_feed::POWERSHELLS {
            let dir = tempfile::tempdir().unwrap();
            let path = dir
                .path()
                .join("quote ' triple ''' dollar $ backtick ` space 你好.ndjson");
            let path = PathBuf::from(hook_feed::hook_path(&path));
            let mut watcher = CodexStatusWatcher::start(&path, "current".into()).unwrap();
            let mut large = report("UserPromptSubmit", "one");
            large["prompt"] = json!(format!("你好 {}", "x\n".repeat(128 * 1024)));
            if !run_powershell_hook(
                shell,
                &path,
                "UserPromptSubmit",
                &serde_json::to_vec_pretty(&large).unwrap(),
            ) {
                continue;
            }
            run_powershell_hook(shell, &path, "Stop", b"not JSON");
            run_powershell_hook(shell, &path, "Stop", br#"{"hook_event_name":42}"#);
            run_powershell_hook(
                shell,
                &path,
                "Stop",
                &serde_json::to_vec(&report("PreToolUse", "one")).unwrap(),
            );
            let mut stale = report("Stop", "one");
            stale["generation"] = json!("old");
            append(&path, &stale);
            let stop = format!("{}\n", report("Stop", "one"));
            let mut file = OpenOptions::new().append(true).open(&path).unwrap();
            file.write_all(&stop.as_bytes()[..20]).unwrap();
            let mut values = Vec::new();
            watcher
                .drain_observations(|value, _| values.push(value))
                .unwrap();
            assert_eq!(values.len(), 1, "{shell}");
            assert_eq!(values[0].activity, Activity::Working);
            file.write_all(&stop.as_bytes()[20..]).unwrap();
            drop(file);
            watcher.feed.dirty.store(true, Ordering::Release);
            watcher
                .drain_observations(|value, _| values.push(value))
                .unwrap();
            assert_eq!(values.last().unwrap().activity, Activity::Ready);
            fs::remove_file(&path).unwrap();
            watcher.feed.dirty.store(true, Ordering::Release);
            assert!(watcher.drain_observations(|_, _| {}).is_err());
            drop(watcher);
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
            run_powershell_hook(shell, &path, "Stop", &vec![b'x'; 256 * 1024]);
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        }
    }
}
