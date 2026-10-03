use crate::model::Runtime;
use crate::session::state::agent::{AdapterFeedback, AgentEvent};
#[cfg(test)]
use crate::session::state::StatusSource;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::Result;
use crate::session::hook_feed::HookFeed;
use crate::session::status::{TurnOutcome, WaitReason};

#[cfg(test)]
use crate::session::status::{Activity, AgentObservation, ObservationSource, WorkDetail};
pub(crate) const PATH_ENV: &str = "RUNNER_PI_STATUS_PATH";
pub(crate) const GENERATION_ENV: &str = "RUNNER_PI_STATUS_GENERATION";
pub(crate) const SESSION_KEY_ENV: &str = "RUNNER_PI_SESSION_KEY";
pub(crate) const REKEY_PATH_ENV: &str = "RUNNER_PI_REKEY_PATH";

const EXTENSION_DIR: &str = "pi-hooks";
const EXTENSION_FILE: &str = "runner-status.ts";

const EXTENSION_SOURCE: &str = r#"import fs from "node:fs";
import path from "node:path";

const minimumVersion = [0, 84, 4];

function versionIsTooOld(version) {
  const parts = String(version).split(".").map((part) => Number.parseInt(part, 10) || 0);
  for (let index = 0; index < minimumVersion.length; index += 1) {
    if ((parts[index] || 0) < minimumVersion[index]) return true;
    if ((parts[index] || 0) > minimumVersion[index]) return false;
  }
  return false;
}

export default async function (pi) {
  try {
    const { VERSION } = await import("@earendil-works/pi-coding-agent");
    if (versionIsTooOld(VERSION)) return;
  } catch {}

  const statusPath = process.env.RUNNER_PI_STATUS_PATH;
  const generation = process.env.RUNNER_PI_STATUS_GENERATION;
  if (!statusPath || !generation) return;

  const sessionKey = process.env.RUNNER_PI_SESSION_KEY || "";
  const rekeyPath = process.env.RUNNER_PI_REKEY_PATH || "";
  let lastReportedSessionId = sessionKey;
  let rekeySequence = 0;

  function append(event, fields = {}) {
    try {
      fs.appendFileSync(statusPath, JSON.stringify({
        generation,
        hook_event_name: event.type,
        ...fields,
      }) + "\n", { flag: fs.constants.O_WRONLY | fs.constants.O_APPEND });
    } catch {}
  }

  function rekey(sessionId) {
    if (!rekeyPath || !sessionId || sessionId === lastReportedSessionId) return;
    try {
      fs.mkdirSync(path.dirname(rekeyPath), { recursive: true });
      rekeySequence += 1;
      const temporary = `${rekeyPath}.${process.pid}.${rekeySequence}.tmp`;
      fs.writeFileSync(temporary, JSON.stringify({ session_id: sessionId }));
      fs.renameSync(temporary, rekeyPath);
      lastReportedSessionId = sessionId;
    } catch {}
  }

  pi.on("session_start", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    try {
      const sessionId = ctx.sessionManager.getSessionId();
      append(event, { reason: event.reason, session_id: sessionId });
      rekey(sessionId);
    } catch {}
  });

  pi.on("agent_start", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    append(event);
  });

  pi.on("tool_execution_start", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    append(event, { toolCallId: event.toolCallId, toolName: event.toolName });
  });

  pi.on("tool_execution_end", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    append(event, { toolCallId: event.toolCallId, toolName: event.toolName });
  });

  pi.on("session_before_compact", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    append(event, { reason: event.reason });
  });

  pi.on("session_compact", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    append(event);
  });

  pi.on("session_compact_failed", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    append(event);
  });

  pi.on("message_end", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    if (event.message?.role !== "assistant") return;
    append(event, {
      stopReason: event.message.stopReason,
      errorMessage: typeof event.message.errorMessage === "string"
        ? event.message.errorMessage.slice(0, 512)
        : null,
    });
  });

  pi.on("agent_settled", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    append(event);
  });

  pi.on("ui_prompt_start", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    append(event, { kind: event.kind, title: event.title });
  });

  pi.on("ui_prompt_end", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    append(event, { kind: event.kind, title: event.title });
  });

  pi.on("session_shutdown", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    append(event, { reason: event.reason });
  });
}
"#;

pub(crate) fn extension_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(EXTENSION_DIR)
}

pub(crate) fn extension_path(app_data_dir: &Path) -> PathBuf {
    extension_dir(app_data_dir).join(EXTENSION_FILE)
}

pub(crate) fn extension_available(app_data_dir: &Path) -> bool {
    extension_path(app_data_dir).is_file()
}

pub(crate) fn install_extension(app_data_dir: &Path) -> Result<()> {
    let directory = extension_dir(app_data_dir);
    fs::create_dir_all(&directory)?;
    let path = extension_path(app_data_dir);
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(EXTENSION_SOURCE.as_bytes())?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[derive(Default, Deserialize)]
struct StatusReport {
    #[serde(default)]
    hook_event_name: String,
    reason: Option<String>,
    pub(crate) session_id: Option<String>,
    #[serde(rename = "toolCallId")]
    tool_call_id: Option<String>,
    #[serde(rename = "toolName")]
    tool_name: Option<String>,
    #[serde(rename = "stopReason")]
    stop_reason: Option<String>,
    #[serde(rename = "errorMessage")]
    error_message: Option<String>,
    kind: Option<String>,
    title: Option<String>,
}

#[derive(Clone, Default)]
struct PiParser {
    open_tools: std::collections::BTreeSet<String>,
    compacting: bool,
}
impl PiParser {
    fn hook(&mut self, report: StatusReport) -> Option<Vec<AgentEvent>> {
        let event = match report.hook_event_name.as_str() {
            "session_start" => {
                if report.reason.as_deref() == Some("reload") {
                    return None;
                }
                report.session_id.filter(|id| !id.is_empty())?;
                self.open_tools.clear();
                self.compacting = false;
                AgentEvent::StartupReady
            }
            "agent_start" => {
                self.open_tools.clear();
                self.compacting = false;
                AgentEvent::TurnStarted
            }
            "tool_execution_start" => {
                let id = report.tool_call_id.filter(|id| !id.is_empty())?;
                report.tool_name.filter(|name| !name.is_empty())?;
                if !self.open_tools.insert(id) {
                    return None;
                }
                AgentEvent::ToolStarted {
                    count: self.open_tools.len(),
                    question: None,
                }
            }
            "tool_execution_end" => {
                let id = report.tool_call_id.filter(|id| !id.is_empty())?;
                report.tool_name.filter(|name| !name.is_empty())?;
                if !self.open_tools.remove(&id) {
                    return None;
                }
                AgentEvent::ToolEnded {
                    owner: None,
                    count: self.open_tools.len(),
                    interrupted: false,
                    transcript: false,
                }
            }
            "session_before_compact" => {
                self.compacting = true;
                AgentEvent::CompactionStarted
            }
            "session_compact" | "session_compact_failed" => {
                if !self.compacting {
                    return None;
                }
                self.compacting = false;
                AgentEvent::CompactionEnded
            }
            "message_end" => AgentEvent::Outcome {
                outcome: match report.stop_reason.as_deref() {
                    Some("error")
                        if report.error_message.as_deref()
                            == Some("This operation was aborted") =>
                    {
                        TurnOutcome::Interrupted
                    }
                    Some("error") => TurnOutcome::Failed,
                    Some("aborted") => TurnOutcome::Interrupted,
                    _ => TurnOutcome::Completed,
                },
            },
            "agent_settled" => {
                self.open_tools.clear();
                self.compacting = false;
                AgentEvent::Settled
            }
            "ui_prompt_start" => AgentEvent::ReplaceInteraction {
                reason: match report.kind.as_deref()? {
                    "confirm" => WaitReason::Approval,
                    "select" | "input" | "editor" => WaitReason::Answer,
                    "custom" => WaitReason::Unknown,
                    _ => return None,
                },
                owner: report.title.unwrap_or_else(|| "pi-ui".into()),
            },
            "ui_prompt_end" | "session_shutdown" => AgentEvent::ClearInteractions,
            _ => return None,
        };
        Some(vec![event])
    }
}

pub(crate) struct PiStatusWatcher {
    feed: HookFeed,
    parser: PiParser,
}
impl PiStatusWatcher {
    pub(crate) fn start(path: &Path, generation: String) -> Result<Self> {
        let app_data_dir = path
            .parent()
            .and_then(Path::parent)
            .expect("status file is under app data");
        Ok(Self {
            feed: HookFeed::start_external(path, generation, &extension_path(app_data_dir))?,
            parser: Default::default(),
        })
    }
    pub(crate) fn drain_events(
        &mut self,
        cancel: u8,
        mut emit: impl FnMut(AgentEvent) -> AdapterFeedback,
        _session_start: impl FnMut(String),
    ) -> Result<()> {
        self.feed.drain(cancel != 0, |report| {
            if let Ok(report) = serde_json::from_value::<StatusReport>(report) {
                if let Some(events) = self.parser.hook(report) {
                    emit(AgentEvent::Batch {
                        runtime: Runtime::Pi,
                        events,
                    });
                }
            }
        })?;
        Ok(())
    }
}
impl crate::session::hook_feed::HookWatcher for PiStatusWatcher {
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
struct PiObservation {
    parser: PiParser,
    model: crate::session::state::agent::AgentModel,
    value: crate::session::state::agent::TurnState,
}
#[cfg(test)]
impl std::ops::Deref for PiObservation {
    type Target = PiParser;
    fn deref(&self) -> &Self::Target {
        &self.parser
    }
}
#[cfg(test)]
impl std::ops::DerefMut for PiObservation {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.parser
    }
}
#[cfg(test)]
impl PiObservation {
    fn observe(&mut self, event: StatusReport, now: i64) -> Option<AgentObservation> {
        let events = self.parser.hook(event)?;
        let reduced = self.model.reduce(
            AgentEvent::Batch {
                runtime: Runtime::Pi,
                events,
            },
            now,
        );
        self.value = self.model.value.clone();
        reduced
    }
}
#[cfg(test)]
mod tests {
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::process::Command;
    use std::sync::atomic::Ordering;

    use serde_json::{json, Value};

    use super::*;

    struct TestWatcher {
        inner: PiStatusWatcher,
        observation: PiObservation,
        cancel: u8,
    }
    impl std::ops::Deref for TestWatcher {
        type Target = PiStatusWatcher;
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
                inner: PiStatusWatcher::start(path, generation)?,
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

    fn report(event: &str) -> Value {
        json!({"hook_event_name": event})
    }

    fn observe(state: &mut PiObservation, value: Value) -> Option<AgentObservation> {
        state.observe(
            serde_json::from_value::<StatusReport>(value).unwrap(),
            crate::session::clock::timestamp_millis(),
        )
    }

    #[test]
    fn session_start_publishes_hook_idle_and_preserves_reload_guard() {
        let mut state = PiObservation::default();
        let startup = json!({
            "hook_event_name":"session_start",
            "reason":"startup",
            "session_id":"11111111-1111-4111-8111-111111111111",
        });
        let started = observe(&mut state, startup.clone()).unwrap();
        assert_eq!(started.activity, Activity::Idle);
        assert_eq!(started.source, ObservationSource::Hook);
        assert_eq!(started.outcome, None);
        assert_eq!(started.detail, None);

        let restarted = observe(&mut state, startup).unwrap();
        assert_eq!(restarted.activity, Activity::Idle);
        assert_eq!(restarted.source, ObservationSource::Hook);

        let before = state.value.clone();
        assert!(observe(
            &mut state,
            json!({
                "hook_event_name":"session_start",
                "reason":"reload",
                "session_id":"11111111-1111-4111-8111-111111111111",
            }),
        )
        .is_none());
        assert_eq!(state.value, before);
    }

    #[test]
    fn extension_install_is_atomic_idempotent_and_available() {
        let root = tempfile::tempdir().unwrap();
        assert!(!extension_available(root.path()));
        install_extension(root.path()).unwrap();
        assert!(extension_available(root.path()));
        assert_eq!(
            fs::read_to_string(extension_path(root.path())).unwrap(),
            EXTENSION_SOURCE
        );
        fs::write(extension_path(root.path()), "broken").unwrap();
        install_extension(root.path()).unwrap();
        assert_eq!(
            fs::read_to_string(extension_path(root.path())).unwrap(),
            EXTENSION_SOURCE
        );
        assert_eq!(fs::read_dir(extension_dir(root.path())).unwrap().count(), 1);
    }

    #[test]
    fn lifecycle_tools_compaction_outcomes_and_boundaries_follow_pi_events() {
        let mut state = PiObservation::default();
        let started = observe(
            &mut state,
            json!({
                "hook_event_name":"session_start",
                "reason":"startup",
                "session_id":"11111111-1111-4111-8111-111111111111",
            }),
        )
        .unwrap();
        assert_eq!(started.activity, Activity::Idle);
        assert_eq!(started.source, ObservationSource::Hook);
        assert_eq!(
            observe(&mut state, report("agent_start")).unwrap().activity,
            Activity::Working
        );
        let start = json!({
            "hook_event_name":"tool_execution_start",
            "toolCallId":"call-one",
            "toolName":"bash",
        });
        assert_eq!(
            observe(&mut state, start).unwrap().detail,
            Some(WorkDetail::UsingTools)
        );
        let end = json!({
            "hook_event_name":"tool_execution_end",
            "toolCallId":"call-one",
            "toolName":"bash",
        });
        assert_eq!(observe(&mut state, end).unwrap().detail, None);
        assert_eq!(
            observe(&mut state, report("session_before_compact"))
                .unwrap()
                .detail,
            Some(WorkDetail::CompactingContext)
        );
        assert_eq!(
            observe(&mut state, report("session_compact"))
                .unwrap()
                .detail,
            None
        );
        assert!(observe(&mut state, report("agent_end")).is_none());
        assert!(observe(
            &mut state,
            json!({"hook_event_name":"message_end","stopReason":"stop"}),
        )
        .is_none());
        assert_eq!(
            observe(&mut state, report("agent_settled"))
                .unwrap()
                .outcome,
            Some(TurnOutcome::Completed)
        );
        for reason in ["new", "resume", "fork"] {
            let value = observe(
                &mut state,
                json!({
                    "hook_event_name":"session_start",
                    "reason":reason,
                    "session_id":"11111111-1111-4111-8111-111111111111",
                }),
            )
            .unwrap();
            assert_eq!(value.activity, Activity::Idle);
            assert_eq!(value.outcome, None);
        }
        for (stop_reason, outcome) in [
            ("stop", TurnOutcome::Completed),
            ("error", TurnOutcome::Failed),
            ("aborted", TurnOutcome::Interrupted),
        ] {
            observe(&mut state, report("agent_start"));
            assert!(observe(
                &mut state,
                json!({
                    "hook_event_name":"message_end",
                    "stopReason":stop_reason,
                    "errorMessage":"x".repeat(2048),
                }),
            )
            .is_none());
            let settled = observe(&mut state, report("agent_settled")).unwrap();
            assert_eq!(settled.activity, Activity::Ready);
            assert_eq!(settled.outcome, Some(outcome));
        }
        let before = state.value.clone();
        assert!(observe(
            &mut state,
            json!({
                "hook_event_name":"session_start",
                "reason":"reload",
                "session_id":"11111111-1111-4111-8111-111111111111",
            }),
        )
        .is_none());
        assert_eq!(state.value, before);
    }

    #[test]
    fn aborted_error_settles_interrupted_while_provider_errors_remain_failed() {
        for (message, expected) in [
            (Some("This operation was aborted"), TurnOutcome::Interrupted),
            (Some("Provider unavailable"), TurnOutcome::Failed),
            (
                Some("Request failed: This operation was aborted"),
                TurnOutcome::Failed,
            ),
            (None, TurnOutcome::Failed),
        ] {
            let mut state = PiObservation::default();
            observe(&mut state, report("agent_start"));
            assert!(observe(
                &mut state,
                json!({"hook_event_name":"message_end","stopReason":"error","errorMessage":message}),
            ).is_none());
            let settled = observe(&mut state, report("agent_settled")).unwrap();
            assert_eq!(settled.activity, Activity::Ready);
            assert_eq!(settled.outcome, Some(expected));
            assert_eq!(
                observe(&mut state, report("agent_start")).unwrap().outcome,
                None
            );
            observe(
                &mut state,
                json!({"hook_event_name":"message_end","stopReason":"stop"}),
            );
            assert_eq!(
                observe(&mut state, report("agent_settled"))
                    .unwrap()
                    .outcome,
                Some(TurnOutcome::Completed)
            );
        }
    }

    #[test]
    fn prompt_kinds_raise_one_wait_and_end_or_shutdown_clear_it() {
        for (kind, reason) in [
            ("confirm", WaitReason::Approval),
            ("select", WaitReason::Answer),
            ("input", WaitReason::Answer),
            ("editor", WaitReason::Answer),
            ("custom", WaitReason::Unknown),
        ] {
            let mut state = PiObservation::default();
            observe(
                &mut state,
                json!({
                    "hook_event_name":"session_start",
                    "reason":"startup",
                    "session_id":"11111111-1111-4111-8111-111111111111",
                }),
            );
            let waiting = observe(
                &mut state,
                json!({"hook_event_name":"ui_prompt_start","kind":kind,"title":"Choose"}),
            )
            .unwrap();
            assert_eq!(waiting.interactions.len(), 1);
            assert_eq!(waiting.interactions[0].reason, reason);
            assert!(observe(
                &mut state,
                json!({"hook_event_name":"ui_prompt_end","kind":kind,"title":"Choose"}),
            )
            .unwrap()
            .interactions
            .is_empty());
            observe(
                &mut state,
                json!({"hook_event_name":"ui_prompt_start","kind":kind,"title":"Choose"}),
            );
            assert!(observe(
                &mut state,
                json!({"hook_event_name":"session_shutdown","reason":"new"}),
            )
            .unwrap()
            .interactions
            .is_empty());
        }
    }

    #[test]
    fn watcher_ignores_wrong_generation_malformed_lines_and_reports_bridge_loss() {
        let root = tempfile::tempdir().unwrap();
        install_extension(root.path()).unwrap();
        let path = crate::session::hook_feed::status_path(root.path(), "pi-status");
        let mut watcher = TestWatcher::start(&path, "current".into()).unwrap();
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "not json").unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "generation":"old",
                "hook_event_name":"agent_start",
            })
        )
        .unwrap();
        for value in [
            json!({
                "generation":"current",
                "hook_event_name":"session_start",
                "reason":"startup",
                "session_id":"11111111-1111-4111-8111-111111111111",
            }),
            json!({"generation":"current","hook_event_name":"agent_start"}),
            json!({
                "generation":"current",
                "hook_event_name":"message_end",
                "stopReason":"error",
                "errorMessage":"x".repeat(4096),
            }),
            json!({"generation":"current","hook_event_name":"agent_settled"}),
        ] {
            writeln!(file, "{value}").unwrap();
        }
        watcher.feed.dirty.store(true, Ordering::Release);
        let mut values = Vec::new();
        watcher.drain_status(|value, _| values.push(value)).unwrap();
        assert_eq!(values.len(), 3);
        assert_eq!(values[0].activity, Activity::Idle);
        assert_eq!(values[0].source, ObservationSource::Hook);
        assert_eq!(values[1].activity, Activity::Working);
        assert_eq!(values[2].outcome, Some(TurnOutcome::Failed));
        fs::remove_file(extension_path(root.path())).unwrap();
        watcher.feed.dirty.store(true, Ordering::Release);
        assert!(watcher.drain_status(|_, _| {}).is_err());
        drop(watcher);
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 0);
    }

    #[test]
    fn embedded_extension_runs_as_mjs_reports_and_rekeys() {
        if Command::new("node").arg("--version").output().is_err() {
            eprintln!("skipping pi extension test: node is not on PATH");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let app_data = root.path().join("app data");
        install_extension(&app_data).unwrap();
        let source = root.path().join("runner-status.mjs");
        fs::write(&source, EXTENSION_SOURCE).unwrap();
        let package = root
            .path()
            .join("node_modules/@earendil-works/pi-coding-agent");
        fs::create_dir_all(&package).unwrap();
        fs::write(
            package.join("package.json"),
            r#"{"name":"@earendil-works/pi-coding-agent","type":"module","exports":"./index.js"}"#,
        )
        .unwrap();
        fs::write(
            package.join("index.js"),
            "export const VERSION = '0.85.1';\n",
        )
        .unwrap();
        let driver = root.path().join("driver.mjs");
        fs::write(
            &driver,
            r#"import fs from "node:fs";
import extension from "./runner-status.mjs";
const handlers = new Map();
await extension({ on(name, handler) { handlers.set(name, handler); } });
const expected = Number(process.env.EXPECT_HANDLERS || "12");
if (handlers.size !== expected) throw new Error(`handlers ${handlers.size}, expected ${expected}`);
if (handlers.size === 0) process.exit(0);
let sessionId = process.env.TEST_SESSION_ID;
const ctx = {
  mode: "print",
  sessionManager: { getSessionId() { return sessionId; } },
};
await handlers.get("agent_start")({ type: "agent_start" }, ctx);
ctx.mode = "tui";
const fire = (type, fields = {}) => handlers.get(type)({ type, ...fields }, ctx);
await fire("session_start", { reason: "startup" });
if (fs.existsSync(process.env.RUNNER_PI_REKEY_PATH)) throw new Error("startup rekeyed");
await fire("agent_start");
await fire("tool_execution_start", { toolCallId: "call-one", toolName: "bash" });
await fire("tool_execution_end", { toolCallId: "call-one", toolName: "bash" });
await fire("session_before_compact", { reason: "manual" });
await fire("session_compact", { reason: "manual" });
await fire("message_end", { message: { role: "user" } });
await fire("message_end", { message: { role: "assistant", stopReason: "error", errorMessage: "x".repeat(700) } });
await fire("ui_prompt_start", { kind: "confirm", title: "Approve" });
await fire("ui_prompt_end", { kind: "confirm", title: "Approve" });
await fire("agent_settled");
await fire("session_shutdown", { reason: "quit" });
if (process.env.NEXT_SESSION_ID) {
  sessionId = process.env.NEXT_SESSION_ID;
  await fire("session_start", { reason: "new" });
  const report = JSON.parse(fs.readFileSync(process.env.RUNNER_PI_REKEY_PATH, "utf8"));
  if (report.session_id !== sessionId) throw new Error(`new rekey ${report.session_id}`);
}
if (process.env.RETURN_SESSION_ID) {
  sessionId = process.env.RETURN_SESSION_ID;
  await fire("session_start", { reason: "resume" });
}
"#,
        )
        .unwrap();

        let session_key = "11111111-1111-4111-8111-111111111111";
        let new_key = "22222222-2222-4222-8222-222222222222";
        let drop_path = crate::session::claude_rekey::drop_path(&app_data, "runner-session");
        let feed_path = crate::session::hook_feed::status_path(&app_data, "extension-e2e");
        let mut watcher = TestWatcher::start(&feed_path, "current".into()).unwrap();
        let output = Command::new("node")
            .arg(&driver)
            .env(PATH_ENV, &feed_path)
            .env(GENERATION_ENV, "current")
            .env(SESSION_KEY_ENV, session_key)
            .env(REKEY_PATH_ENV, &drop_path)
            .env("TEST_SESSION_ID", session_key)
            .env("NEXT_SESSION_ID", new_key)
            .env("RETURN_SESSION_ID", session_key)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stdout.is_empty() && output.stderr.is_empty());
        let reports: Vec<Value> = fs::read_to_string(&feed_path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(reports.len(), 13);
        assert_eq!(reports[0]["hook_event_name"], "session_start");
        assert_eq!(reports[0]["session_id"], session_key);
        assert_eq!(reports[11]["session_id"], new_key);
        assert_eq!(reports[12]["session_id"], session_key);
        let error = reports
            .iter()
            .find(|report| report["hook_event_name"] == "message_end")
            .unwrap();
        assert_eq!(error["errorMessage"].as_str().unwrap().len(), 512);
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(&drop_path).unwrap()).unwrap(),
            json!({"session_id":session_key})
        );
        watcher.feed.dirty.store(true, Ordering::Release);
        let mut values = Vec::new();
        watcher.drain_status(|value, _| values.push(value)).unwrap();
        assert_eq!(values.first().unwrap().activity, Activity::Idle);
        assert_eq!(values.first().unwrap().source, ObservationSource::Hook);
        assert_eq!(values[1].activity, Activity::Working);
        assert!(values
            .iter()
            .any(|value| value.outcome == Some(TurnOutcome::Failed)));
        assert_eq!(values.last().unwrap().activity, Activity::Idle);
        assert_eq!(values.last().unwrap().outcome, None);

        fs::remove_file(&drop_path).unwrap();
        let matching_feed = crate::session::hook_feed::status_path(&app_data, "matching-key");
        let _matching_watcher = TestWatcher::start(&matching_feed, "matching".into()).unwrap();
        let output = Command::new("node")
            .arg(&driver)
            .env(PATH_ENV, &matching_feed)
            .env(GENERATION_ENV, "matching")
            .env(SESSION_KEY_ENV, session_key)
            .env(REKEY_PATH_ENV, &drop_path)
            .env("TEST_SESSION_ID", session_key)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(!drop_path.exists());

        fs::write(
            package.join("index.js"),
            "export const VERSION = '0.84.3';\n",
        )
        .unwrap();
        let output = Command::new("node")
            .arg(&driver)
            .env("EXPECT_HANDLERS", "0")
            .env(PATH_ENV, &matching_feed)
            .env(GENERATION_ENV, "old")
            .env(SESSION_KEY_ENV, session_key)
            .env(REKEY_PATH_ENV, &drop_path)
            .env("TEST_SESSION_ID", new_key)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stdout.is_empty() && output.stderr.is_empty());
        assert!(!drop_path.exists());
    }
}
