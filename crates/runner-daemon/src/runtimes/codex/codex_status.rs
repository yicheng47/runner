use crate::model::Runtime;
use crate::session::state::agent::{AdapterFeedback, AgentEvent};
#[cfg(test)]
use crate::session::state::StatusSource;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::error::Result;
use crate::session::hook_feed::{self, HookFeed};
use crate::session::status::TurnOutcome;

#[cfg(test)]
use crate::session::status::{Activity, AgentObservation, ObservationSource, WorkDetail};
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
    source: Option<String>,
    pub(crate) session_id: Option<String>,
    turn_id: Option<String>,
    pub(crate) transcript_path: Option<PathBuf>,
    agent_id: Option<String>,
}

#[derive(Clone, Default)]
struct CodexParser {
    session_id: Option<String>,
    turn_id: Option<String>,
    retired_turns: std::collections::BTreeSet<String>,
    pending_tools: usize,
    compacting: bool,
    ended: bool,
    transcript_path: Option<PathBuf>,
    abort_reported: bool,
    seen: bool,
    abort_settled: bool,
}
impl CodexParser {
    fn hook(&mut self, report: StatusReport) -> Option<Vec<AgentEvent>> {
        if report.agent_id.is_some() || !EVENTS.contains(&report.hook_event_name.as_str()) {
            return None;
        }
        let session = report.session_id.filter(|id| !id.is_empty())?;
        let new_session = self.session_id.as_ref().is_some_and(|id| *id != session);
        if new_session && report.hook_event_name != "SessionStart" {
            return None;
        }
        if report.hook_event_name == "SessionStart" && (new_session || self.ended) {
            if new_session {
                self.retired_turns.clear();
            } else if let Some(turn) = self.turn_id.take() {
                self.retired_turns.insert(turn);
            }
            self.turn_id = None;
            self.pending_tools = 0;
            self.compacting = false;
            self.ended = false;
            self.transcript_path = None;
            self.abort_reported = false;

            self.abort_settled = false;
        }
        self.session_id = Some(session);
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
            return Some(vec![AgentEvent::StartupReady]);
        }
        if self.ended {
            return None;
        }
        if report.hook_event_name == "SessionEnd" {
            self.ended = true;
            self.pending_tools = 0;
            self.compacting = false;
            return self.seen.then(|| vec![AgentEvent::SessionEnded]);
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

            self.abort_settled = false;
            AgentEvent::TurnStarted
        } else {
            if self.turn_id.is_none() && report.hook_event_name == "Interrupt" {
                self.turn_id = Some(turn.clone());
            }
            if self.turn_id.as_ref() != Some(&turn) {
                return None;
            }
            match report.hook_event_name.as_str() {
                "Interrupt" => {
                    self.abort_reported = true;

                    self.pending_tools = 0;
                    self.compacting = false;
                    AgentEvent::TurnEnded {
                        outcome: TurnOutcome::Interrupted,
                    }
                }
                "Stop" => {
                    self.pending_tools = 0;
                    self.compacting = false;
                    AgentEvent::TurnEnded {
                        outcome: TurnOutcome::Completed,
                    }
                }
                "PreToolUse" => {
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
                    AgentEvent::CompactionEnded
                }
                _ => return None,
            }
        };
        if let Some(path) = report.transcript_path {
            self.transcript_path = Some(path);
        }
        self.seen = true;
        Some(vec![event])
    }
    fn record(&mut self, record: &Value) -> Option<Vec<AgentEvent>> {
        let payload = &record["payload"];
        if !self.ended
            && self.abort_reported
            && !self.abort_settled
            && record["type"] == "event_msg"
            && payload["type"] == "turn_aborted"
            && payload["reason"] == "interrupted"
            && payload["turn_id"].as_str().is_some()
            && payload["turn_id"].as_str() == self.turn_id.as_deref()
        {
            self.abort_settled = true;
            return Some(vec![AgentEvent::AbortSettled]);
        }
        None
    }
}

pub(crate) struct CodexStatusWatcher {
    feed: HookFeed,
    parser: CodexParser,
    transcript: Option<crate::session::hook_feed::TranscriptTail>,
}
impl CodexStatusWatcher {
    pub(crate) fn start(path: &Path, generation: String) -> Result<Self> {
        Ok(Self {
            feed: if cfg!(windows) {
                HookFeed::start_powershell(path, generation, &windows_reporter())?
            } else {
                HookFeed::start(path, generation, APPEND_SCRIPT)?
            },
            parser: Default::default(),
            transcript: None,
        })
    }
    pub(crate) fn drain_events(
        &mut self,
        cancel: u8,
        mut emit: impl FnMut(AgentEvent) -> AdapterFeedback,
        mut session_start: impl FnMut(String),
    ) -> Result<()> {
        self.feed.drain(cancel != 0, |report| {
            if let Ok(report) = serde_json::from_value::<StatusReport>(report) {
                if report.hook_event_name == "SessionStart" && report.agent_id.is_none() {
                    if let Some(id) = report.session_id.as_deref() {
                        if uuid::Uuid::parse_str(id).is_ok() {
                            session_start(id.to_owned());
                        }
                    }
                }
                if let Some(events) = self.parser.hook(report) {
                    emit(AgentEvent::Batch {
                        runtime: Runtime::Codex,
                        events,
                    });
                }
            }
        })?;
        self.drain_transcript(&mut emit);
        Ok(())
    }
    fn drain_transcript(&mut self, emit: &mut impl FnMut(AgentEvent) -> AdapterFeedback) {
        if !self.parser.abort_reported || self.parser.abort_settled || self.parser.ended {
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
mod tests {
    use super::*;

    struct TestWatcher {
        inner: CodexStatusWatcher,
        observation: CodexObservation,
        cancel: u8,
    }
    impl std::ops::Deref for TestWatcher {
        type Target = CodexStatusWatcher;
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
        fn start(path: &Path, generation: String) -> Result<Self> {
            Ok(Self {
                inner: CodexStatusWatcher::start(path, generation)?,
                observation: Default::default(),
                cancel: 0,
            })
        }
        fn drain_with_session_starts(
            &mut self,
            mut publish: impl FnMut(AgentObservation, StatusSource),
            starts: impl FnMut(String),
        ) -> Result<()> {
            self.inner.parser = self.observation.parser.clone();
            let cancel = std::mem::take(&mut self.cancel);
            let mut events = Vec::new();
            let result = self.inner.drain_events(
                cancel,
                |event| {
                    events.push(event);
                    Default::default()
                },
                starts,
            );
            for event in events {
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
                if let Some(value) = self
                    .observation
                    .model
                    .reduce(event, crate::session::clock::timestamp_millis())
                {
                    publish(value, source);
                }
            }
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
    }

    use serde_json::json;
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::sync::atomic::Ordering;

    fn report(event: &str, turn: &str) -> Value {
        json!({"hook_event_name":event,"session_id":"main","turn_id":turn,"generation":"current"})
    }

    fn observe(state: &mut CodexObservation, value: Value) -> Option<AgentObservation> {
        state.observe(
            serde_json::from_value::<StatusReport>(value).unwrap(),
            crate::session::clock::timestamp_millis(),
        )
    }

    fn abort(turn: &str) -> Value {
        json!({"type":"event_msg","payload":{"type":"turn_aborted","turn_id":turn,"reason":"interrupted"}})
    }

    #[test]
    fn only_current_generation_root_session_starts_report_keys() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("codex.ndjson");
        let mut watcher = TestWatcher::start(&path, "current".into()).unwrap();
        let old = uuid::Uuid::new_v4().to_string();
        let new = uuid::Uuid::new_v4().to_string();
        let reports = [
            json!({"generation":"old","hook_event_name":"SessionStart","session_id":old}),
            json!({"generation":"current","hook_event_name":"SessionStart","session_id":old,"agent_id":"child"}),
            json!({"generation":"current","hook_event_name":"SessionStart","session_id":"invalid"}),
            json!({"generation":"current","hook_event_name":"SessionStart","session_id":new,"source":"clear"}),
        ];
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        for report in reports {
            writeln!(file, "{report}").unwrap();
        }
        watcher.feed.dirty.store(true, Ordering::Release);
        let mut starts = Vec::new();
        watcher
            .drain_with_session_starts(|_, _| {}, |id| starts.push(id))
            .unwrap();
        assert_eq!(starts, vec![new]);
    }

    #[test]
    fn session_start_publishes_hook_idle_and_preserves_turn_guards() {
        let mut state = CodexObservation::default();
        let mut start = report("SessionStart", "");
        start["source"] = json!("startup");
        let started = observe(&mut state, start.clone()).unwrap();
        assert_eq!(started.activity, Activity::Idle);
        assert_eq!(started.source, ObservationSource::Hook);
        assert_eq!(started.outcome, None);
        assert_eq!(started.detail, None);

        let restarted = observe(&mut state, start.clone()).unwrap();
        assert_eq!(restarted.activity, Activity::Idle);
        assert_eq!(restarted.source, ObservationSource::Hook);

        observe(&mut state, report("UserPromptSubmit", "one"));
        let working = state.value.clone();
        assert!(observe(&mut state, start.clone()).is_none());
        assert_eq!(state.value, working);

        observe(&mut state, report("PreCompact", "one"));
        let compacting = state.value.clone();
        start["source"] = json!("compact");
        assert!(observe(&mut state, start).is_none());
        assert_eq!(state.value, compacting);
    }

    #[test]
    fn launch_resume_completion_continuation_and_late_results() {
        for source in ["startup", "resume"] {
            let mut state = CodexObservation::default();
            let mut start = report("SessionStart", "");
            start["source"] = json!(source);
            let started = observe(&mut state, start.clone()).unwrap();
            assert_eq!(started.activity, Activity::Idle);
            assert_eq!(started.source, ObservationSource::Hook);
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
    fn tool_and_compaction_detail_follow_in_flight_work() {
        let mut state = CodexObservation::default();
        let prompt = observe(&mut state, report("UserPromptSubmit", "one")).unwrap();
        assert_eq!(prompt.detail, None);
        assert_eq!(
            observe(&mut state, report("PreToolUse", "one"))
                .unwrap()
                .detail,
            Some(WorkDetail::UsingTools)
        );
        assert_eq!(
            observe(&mut state, report("PreToolUse", "one"))
                .unwrap()
                .detail,
            Some(WorkDetail::UsingTools)
        );
        assert_eq!(
            observe(&mut state, report("PreCompact", "one"))
                .unwrap()
                .detail,
            Some(WorkDetail::CompactingContext)
        );
        assert_eq!(
            observe(&mut state, report("PostCompact", "one"))
                .unwrap()
                .detail,
            Some(WorkDetail::UsingTools)
        );
        assert_eq!(
            observe(&mut state, report("PostToolUse", "one"))
                .unwrap()
                .detail,
            Some(WorkDetail::UsingTools)
        );
        assert_eq!(
            observe(&mut state, report("PostToolUse", "one"))
                .unwrap()
                .detail,
            None
        );

        observe(&mut state, report("PreCompact", "one"));
        let stopped = observe(&mut state, report("Stop", "one")).unwrap();
        assert_eq!(stopped.outcome, Some(TurnOutcome::Completed));
        assert_eq!(stopped.detail, None);
        let next = observe(&mut state, report("UserPromptSubmit", "two")).unwrap();
        assert_eq!(next.detail, None);
        observe(&mut state, report("PreToolUse", "two"));
        let interrupted = observe(&mut state, report("Interrupt", "two")).unwrap();
        assert_eq!(interrupted.outcome, Some(TurnOutcome::Interrupted));
        assert_eq!(interrupted.detail, None);
    }

    #[test]
    fn compact_session_start_restores_manual_idle_and_in_turn_work() {
        let mut state = CodexObservation::default();
        observe(&mut state, report("UserPromptSubmit", "one"));
        observe(&mut state, report("Stop", "one"));

        assert_eq!(
            observe(&mut state, report("PreCompact", "one"))
                .unwrap()
                .detail,
            Some(WorkDetail::CompactingContext)
        );
        let mut compact_start = report("SessionStart", "");
        compact_start["source"] = json!("compact");
        assert!(observe(&mut state, compact_start).is_none());
        assert_eq!(state.value.detail, Some(WorkDetail::CompactingContext));
        let manual_done = observe(&mut state, report("PostCompact", "one")).unwrap();
        assert_eq!(manual_done.activity, Activity::Ready);
        assert_eq!(manual_done.outcome, Some(TurnOutcome::Completed));
        assert_eq!(manual_done.detail, None);

        observe(&mut state, report("UserPromptSubmit", "two"));
        observe(&mut state, report("PreToolUse", "two"));
        observe(&mut state, report("PreCompact", "two"));
        let mut compact_start = report("SessionStart", "");
        compact_start["source"] = json!("compact");
        assert!(observe(&mut state, compact_start).is_none());
        let automatic_done = observe(&mut state, report("PostCompact", "two")).unwrap();
        assert_eq!(automatic_done.activity, Activity::Working);
        assert_eq!(automatic_done.outcome, None);
        assert_eq!(automatic_done.detail, Some(WorkDetail::UsingTools));
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
    fn same_session_start_after_end_recovers_from_interrupt_and_rejects_old_turn_hooks() {
        for source in ["startup", "resume", "clear", "compact"] {
            let mut state = CodexObservation::default();
            let mut start = report("SessionStart", "");
            start["transcript_path"] = json!("old-transcript.jsonl");
            observe(&mut state, start);
            observe(&mut state, report("UserPromptSubmit", "earlier"));
            observe(&mut state, report("Stop", "earlier"));
            observe(&mut state, report("UserPromptSubmit", "old"));
            observe(&mut state, report("PreToolUse", "old"));
            observe(&mut state, report("PreCompact", "old"));
            let interrupted = observe(&mut state, report("Interrupt", "old")).unwrap();
            assert_eq!(interrupted.outcome, Some(TurnOutcome::Interrupted));
            let ended = observe(&mut state, report("SessionEnd", "old")).unwrap();
            assert_eq!(ended.activity, Activity::Ready);
            assert_eq!(ended.outcome, Some(TurnOutcome::Interrupted));
            state.observe_transcript(&abort("old"));
            assert_eq!(state.value.activity, Activity::Ready);
            let mut child_start = report("SessionStart", "");
            child_start["agent_id"] = json!("child");
            assert!(observe(&mut state, child_start).is_none());
            assert!(state.ended);

            let mut restart = report("SessionStart", "");
            restart["source"] = json!(source);
            let restarted = observe(&mut state, restart).unwrap();
            assert_eq!(restarted.activity, Activity::Idle);
            assert_eq!(restarted.outcome, None);
            assert_eq!(restarted.detail, None);
            assert_eq!(state.session_id.as_deref(), Some("main"));
            assert_eq!(state.turn_id, None);
            assert!(state.retired_turns.contains("old"));
            assert_eq!(state.pending_tools, 0);
            assert!(!state.compacting);
            assert!(state.model.compaction_resume.is_none());
            assert!(!state.ended);
            assert_eq!(state.transcript_path, None);

            for event in [
                "UserPromptSubmit",
                "PreToolUse",
                "PostToolUse",
                "Stop",
                "Interrupt",
            ] {
                assert!(
                    observe(&mut state, report(event, "old")).is_none(),
                    "{event}"
                );
            }
            assert!(observe(&mut state, report("UserPromptSubmit", "earlier")).is_none());
            state.observe_transcript(&abort("old"));
            assert_eq!(state.value.activity, Activity::Idle);

            let prompt = observe(&mut state, report("UserPromptSubmit", "fresh")).unwrap();
            assert_eq!(prompt.activity, Activity::Working);
            assert_eq!(prompt.outcome, None);
            let tool = observe(&mut state, report("PreToolUse", "fresh")).unwrap();
            assert_eq!(tool.detail, Some(WorkDetail::UsingTools));
            assert!(observe(&mut state, report("SessionStart", "")).is_none());
            assert_eq!(state.value.published(), tool);
            assert!(observe(&mut state, report("Stop", "old")).is_none());
            let stopped = observe(&mut state, report("Stop", "fresh")).unwrap();
            assert_eq!(stopped.activity, Activity::Ready);
            assert_eq!(stopped.outcome, Some(TurnOutcome::Completed));
        }
    }

    #[test]
    fn completed_turn_session_end_stays_idle_until_same_session_restart() {
        let mut state = CodexObservation::default();
        observe(&mut state, report("UserPromptSubmit", "one"));
        let stopped = observe(&mut state, report("Stop", "one")).unwrap();
        assert_eq!(stopped.activity, Activity::Ready);
        assert_eq!(stopped.outcome, Some(TurnOutcome::Completed));

        let ended = observe(&mut state, report("SessionEnd", "one")).unwrap();
        assert_eq!(ended.activity, Activity::Ready);
        assert_eq!(ended.outcome, Some(TurnOutcome::Completed));
        assert!(observe(&mut state, report("UserPromptSubmit", "two")).is_none());

        let restarted = observe(&mut state, report("SessionStart", "")).unwrap();
        assert_eq!(restarted.activity, Activity::Idle);
        assert_eq!(restarted.outcome, None);
        assert_eq!(
            observe(&mut state, report("UserPromptSubmit", "two"))
                .unwrap()
                .activity,
            Activity::Working
        );
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
            assert_eq!(handed_over.activity, Activity::Idle);
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
    fn first_interrupt_does_not_require_a_prompt_hook() {
        for source in [None, Some("startup"), Some("resume")] {
            let mut state = CodexObservation::default();
            let mut child = report("Interrupt", "child");
            child["agent_id"] = json!("child");
            assert!(observe(&mut state, child).is_none());
            if let Some(source) = source {
                let mut start = report("SessionStart", "");
                start["source"] = json!(source);
                observe(&mut state, start);
            }
            for event in ["Stop", "PreToolUse", "Interrupt"] {
                assert!(observe(&mut state, report(event, "")).is_none());
            }
            let interrupted = observe(&mut state, report("Interrupt", "one")).unwrap();
            assert_eq!(interrupted.activity, Activity::Unavailable);
            assert_eq!(interrupted.source, ObservationSource::Hook);
            assert_eq!(interrupted.outcome, Some(TurnOutcome::Interrupted));
            assert_eq!(state.turn_id.as_deref(), Some("one"));
            state.observe_transcript(&abort("old"));
            assert_eq!(state.value.activity, Activity::Unavailable);
            state.observe_transcript(&abort("one"));
            assert_eq!(state.value.activity, Activity::Ready);
            assert_eq!(state.value.outcome, Some(TurnOutcome::Interrupted));
            for event in ["SessionStart", "UserPromptSubmit", "Stop", "Interrupt"] {
                assert!(observe(&mut state, report(event, "one")).is_none());
            }
            observe(&mut state, report("UserPromptSubmit", "two"));
            for turn in ["one", "unknown"] {
                assert!(observe(&mut state, report("Interrupt", turn)).is_none());
                state.observe_transcript(&abort(turn));
            }
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
        let mut watcher = TestWatcher::start(&path, "current".into()).unwrap();
        for event in ["UserPromptSubmit", "Interrupt"] {
            let mut value = report(event, "one");
            value["transcript_path"] = json!(transcript);
            append(&path, &value);
        }
        let mut values = Vec::new();
        watcher.drain_status(|value, _| values.push(value)).unwrap();
        assert_eq!(values.last().unwrap().activity, Activity::Unavailable);
        let line = format!("{}\n", abort("one"));
        let mut file = OpenOptions::new().append(true).open(&transcript).unwrap();
        file.write_all(&line.as_bytes()[..20]).unwrap();
        watcher
            .drain_status(|_, _| panic!("partial abort"))
            .unwrap();
        file.write_all(&line.as_bytes()[20..]).unwrap();
        watcher.drain_status(|value, _| values.push(value)).unwrap();
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
        let mut watcher = TestWatcher::start(&path, "current".into()).unwrap();
        for event in ["UserPromptSubmit", "Interrupt"] {
            let mut value = report(event, "one");
            value["transcript_path"] = json!(dir.path().join("missing"));
            append(&path, &value);
        }
        watcher.drain_status(|_, _| {}).unwrap();
        assert_eq!(watcher.observation.value.activity, Activity::Unavailable);
        assert_eq!(
            watcher.observation.value.outcome,
            Some(TurnOutcome::Interrupted)
        );
        append(&path, &report("UserPromptSubmit", "two"));
        watcher.feed.dirty.store(true, Ordering::Release);
        watcher.drain_status(|_, _| {}).unwrap();
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
        let mut watcher = TestWatcher::start(&path, "current".into()).unwrap();
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
        watcher.drain_status(|value, _| values.push(value)).unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].activity, Activity::Working);
        file.write_all(&stop.as_bytes()[20..]).unwrap();
        watcher.feed.dirty.store(true, Ordering::Release);
        watcher.drain_status(|value, _| values.push(value)).unwrap();
        assert_eq!(values.last().unwrap().activity, Activity::Ready);
        fs::remove_file(hook_feed::script_path(&path)).unwrap();
        watcher.feed.dirty.store(true, Ordering::Release);
        assert!(watcher.drain_status(|_, _| {}).is_err());
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
            let mut watcher = TestWatcher::start(&path, "current".into()).unwrap();
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
            watcher.drain_status(|value, _| values.push(value)).unwrap();
            assert_eq!(values.len(), 1, "{shell}");
            assert_eq!(values[0].activity, Activity::Working);
            file.write_all(&stop.as_bytes()[20..]).unwrap();
            drop(file);
            watcher.feed.dirty.store(true, Ordering::Release);
            watcher.drain_status(|value, _| values.push(value)).unwrap();
            assert_eq!(values.last().unwrap().activity, Activity::Ready);
            fs::remove_file(&path).unwrap();
            watcher.feed.dirty.store(true, Ordering::Release);
            assert!(watcher.drain_status(|_, _| {}).is_err());
            drop(watcher);
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
            run_powershell_hook(shell, &path, "Stop", &vec![b'x'; 256 * 1024]);
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        }
    }
}
