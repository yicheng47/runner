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
#[cfg(test)]
pub(crate) const SESSION_KEY_ENV: &str = "RUNNER_PI_SESSION_KEY";
#[cfg(test)]
pub(crate) const REKEY_PATH_ENV: &str = "RUNNER_PI_REKEY_PATH";

const EXTENSION_DIR: &str = "pi-hooks";
const EXTENSION_FILE: &str = "runner-status.ts";

const EXTENSION_SOURCE: &str = r#"import net from "node:net";
import { setTimeout as deadlineTimeout, clearTimeout as clearDeadline } from "node:timers";

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

  const endpoint = process.env.RUNNER_HOOK_ENDPOINT;
  const generation = process.env.RUNNER_HOOK_GENERATION;
  const session = process.env.RUNNER_HOOK_SESSION;
  if (!endpoint || !generation || !session) return;

  let pending = Promise.resolve();
  let pendingCount = 0;
  let pendingBytes = 0;
  let bridgeFailed = false;
  let stopEditorTracking = () => {};

  function frame(kind, value) {
    const payload = Buffer.from(JSON.stringify(value), "utf8");
    const header = Buffer.alloc(5);
    header.writeUInt32LE(payload.length + 1);
    header[4] = kind;
    return Buffer.concat([header, payload]);
  }

  function deliver(report, expires) {
    return new Promise((resolve) => {
      const remaining = expires - performance.now();
      if (remaining <= 0) { resolve(); return; }
      const socket = new net.Socket();
      let buffer = Buffer.alloc(0);
      let welcomed = false;
      const done = () => { clearDeadline(timer); socket.destroy(); resolve(); };
      const timer = deadlineTimeout(done, remaining);
      socket.on("error", done);
      socket.on("close", done);
      socket.on("data", (bytes) => {
        buffer = Buffer.concat([buffer, bytes]);
        if (buffer.length < 4) return;
        const length = buffer.readUInt32LE(0);
        if (length < 1 || length > 65536) { done(); return; }
        if (buffer.length < length + 4) return;
        const kind = buffer[4];
        if (!welcomed && kind === 2) {
          welcomed = true;
          buffer = Buffer.alloc(0);
          socket.write(frame(5, { id: 1, request: { hook_report: { report } } }));
        } else {
          // Admission (including a lost reply) is single-use; never replay.
          done();
        }
      });
      socket.connect(endpoint, () => socket.write(frame(1, { exe_sha256: "", client: "hook" })));
    });
  }

  function append(event, fields = {}) {
    if (bridgeFailed) return;
    try {
      const report = {
        version: 1, runtime: "pi", session_id: session, generation,
        event: event.type, payload: { hook_event_name: event.type, ...fields }, caller_thread_id: null,
      };
      const bytes = Buffer.byteLength(JSON.stringify(report), "utf8");
      if (bytes > 8 * 1024 * 1024) return;
      if (pendingCount >= 64 || pendingBytes + bytes > 16 * 1024 * 1024) {
        bridgeFailed = true;
        return deliver({ ...report, event: "bridge_unavailable", payload: {}, bridge_unavailable: true }, performance.now() + 250).catch(() => {});
      }
      const expires = performance.now() + 250;
      pendingCount += 1;
      pendingBytes += bytes;
      pending = pending.then(() => bridgeFailed ? undefined : deliver(report, expires)).catch(() => {}).finally(() => {
        pendingCount -= 1;
        pendingBytes -= bytes;
      });
      return pending;
    } catch {}
  }

  function trackEditor(ctx) {
    if (typeof ctx.ui?.getEditorText !== "function"
        || typeof ctx.ui?.onTerminalInput !== "function") return;
    let lastDraft;
    let deferred;
    function sample() {
      const drafting = ctx.ui.getEditorText().length !== 0;
      if (drafting === lastDraft) return;
      lastDraft = drafting;
      append({ type: "editor_draft" }, { drafting });
    }
    const unsubscribe = ctx.ui.onTerminalInput(() => {
      // Raw input arrives before pi updates its editor.
      clearTimeout(deferred);
      deferred = setTimeout(sample, 0);
    });
    const interval = setInterval(sample, 100);
    stopEditorTracking = () => {
      clearInterval(interval);
      clearTimeout(deferred);
      unsubscribe();
      stopEditorTracking = () => {};
    };
    sample();
  }

  pi.on("session_start", async (event, ctx) => {
    stopEditorTracking();
    if (ctx.mode !== "tui") return;
    try {
      const sessionId = ctx.sessionManager.getSessionId();
      await append(event, { reason: event.reason, session_id: sessionId });
    } catch {}
    trackEditor(ctx);
  });

  pi.on("agent_start", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    return append(event);
  });

  pi.on("tool_execution_start", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    return append(event, { toolCallId: event.toolCallId, toolName: event.toolName });
  });

  pi.on("tool_execution_end", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    return append(event, { toolCallId: event.toolCallId, toolName: event.toolName });
  });

  pi.on("session_before_compact", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    return append(event, { reason: event.reason });
  });

  pi.on("session_compact", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    return append(event);
  });

  pi.on("session_compact_failed", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    return append(event);
  });

  pi.on("message_end", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    if (event.message?.role !== "assistant") return;
    return append(event, {
      stopReason: event.message.stopReason,
      errorMessage: typeof event.message.errorMessage === "string"
        ? event.message.errorMessage.slice(0, 512)
        : null,
    });
  });

  pi.on("agent_settled", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    return append(event);
  });

  pi.on("ui_prompt_start", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    return append(event, { kind: event.kind, title: event.title });
  });

  pi.on("ui_prompt_end", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    return append(event, { kind: event.kind, title: event.title });
  });

  pi.on("session_shutdown", (event, ctx) => {
    stopEditorTracking();
    if (ctx.mode !== "tui") return;
    return append(event, { reason: event.reason });
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
    drafting: Option<bool>,
}

#[derive(Clone, Default)]
struct PiParser {
    open_tools: std::collections::BTreeSet<String>,
    compacting: bool,
}
impl PiParser {
    fn hook(&mut self, report: StatusReport) -> Option<Vec<AgentEvent>> {
        let event = match report.hook_event_name.as_str() {
            "editor_draft" => AgentEvent::EditorDraft {
                drafting: report.drafting?,
            },
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
    pub(crate) fn from_receiver(receiver: crate::session::hook_queue::HookReceiver) -> Self {
        Self {
            feed: HookFeed::from_receiver(receiver),
            parser: Default::default(),
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
                if report.hook_event_name == "session_start" {
                    if let Some(id) = report
                        .session_id
                        .as_deref()
                        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
                    {
                        session_start(id.to_owned());
                    }
                }
                if let Some(events) = self.parser.hook(report) {
                    if let [AgentEvent::EditorDraft { drafting }] = events.as_slice() {
                        emit(AgentEvent::EditorDraft {
                            drafting: *drafting,
                        });
                        return;
                    }
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
    use std::fs;
    use std::process::Command;

    use serde_json::{json, Value};

    use super::*;

    struct TestWatcher {
        inner: PiStatusWatcher,
        hooks: crate::session::hook_queue::TestHookRoute,
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
        fn new(generation: String) -> Self {
            let (hooks, receiver) =
                crate::session::hook_queue::TestHookRoute::new(Runtime::Pi, generation);
            Self {
                inner: PiStatusWatcher::from_receiver(receiver),
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

    #[test]
    fn editor_draft_requires_a_boolean_and_does_not_change_turn_status() {
        let mut parser = PiParser::default();
        for drafting in [true, false] {
            let events = parser
                .hook(
                    serde_json::from_value(json!({
                        "hook_event_name":"editor_draft", "drafting":drafting,
                    }))
                    .unwrap(),
                )
                .unwrap();
            assert!(
                matches!(events.as_slice(), [AgentEvent::EditorDraft { drafting: value }] if *value == drafting)
            );
            let mut state = PiObservation::default();
            let before = state.value.clone();
            assert!(observe(
                &mut state,
                json!({
                    "hook_event_name":"editor_draft", "drafting":drafting,
                })
            )
            .is_none());
            assert_eq!(state.value, before);
        }
        assert!(parser
            .hook(serde_json::from_value(report("editor_draft")).unwrap())
            .is_none());
        assert!(serde_json::from_value::<StatusReport>(json!({
            "hook_event_name":"editor_draft", "drafting":"true",
        }))
        .is_err());
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
    fn watcher_rejects_wrong_generation_malformed_reports_and_reports_bridge_loss() {
        let mut watcher = TestWatcher::new("current".into());

        assert!(watcher.hooks.admit_json("not json").is_err());
        assert!(watcher
            .hooks
            .admit(json!({"generation":"old","hook_event_name":"agent_start"}))
            .is_err());
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
            watcher.hooks.admit(value).unwrap();
        }

        let mut values = Vec::new();
        watcher.drain_status(|value, _| values.push(value)).unwrap();
        assert_eq!(values.len(), 3);
        assert_eq!(values[0].activity, Activity::Idle);
        assert_eq!(values[0].source, ObservationSource::Hook);
        assert_eq!(values[1].activity, Activity::Working);
        assert_eq!(values[2].outcome, Some(TurnOutcome::Failed));
        watcher.hooks.retire();

        assert!(watcher.drain_status(|_, _| {}).is_err());
        drop(watcher);
    }

    const IPC_FIXTURE: &str = r#"import net from "node:net";
import fs from "node:fs";
const reports = [];
const envelopes = [];
const endpoint = process.platform === "win32" ? `\\\\.\\pipe\\runner-pi-fixture-${process.pid}` : `${process.cwd()}/pi-${process.pid}.sock`;
const encode = (kind, value) => {
  const payload = Buffer.from(JSON.stringify(value));
  const header = Buffer.alloc(5); header.writeUInt32LE(payload.length + 1); header[4] = kind;
  return Buffer.concat([header, payload]);
};
const server = net.createServer(socket => {
  let buffered = Buffer.alloc(0);
  socket.on("data", bytes => {
    buffered = Buffer.concat([buffered, bytes]);
    while (buffered.length >= 4 && buffered.length >= buffered.readUInt32LE(0) + 4) {
      const length = buffered.readUInt32LE(0);
      const kind = buffered[4];
      const value = JSON.parse(buffered.subarray(5, length + 4));
      buffered = buffered.subarray(length + 4);
      if (kind === 1) socket.write(encode(2, {}));
      else if (kind === 5) {
        const report = value.request.hook_report.report;
        if (report.runtime !== "pi" || report.session_id !== "runner" || report.generation !== "current") throw new Error("routing");
        reports.push(report.payload);
        envelopes.push(report);
        if (report.bridge_unavailable && process.env.STALL_CONTROL) return;
        socket.end(encode(6, { id: 1, result: { hook_report: null } }));
      } else throw new Error(`frame ${kind}`);
    }
  });
});
await new Promise(resolve => server.listen(endpoint, resolve));
process.env.RUNNER_HOOK_ENDPOINT = endpoint;
process.env.RUNNER_HOOK_SESSION = "runner";
process.env.RUNNER_HOOK_GENERATION = "current";
const finish = async () => {
  await new Promise(resolve => server.close(resolve));
  if (process.env.CAPTURE_REPORTS) fs.writeFileSync(process.env.CAPTURE_REPORTS, JSON.stringify(reports));
  if (process.env.CAPTURE_ENVELOPES) fs.writeFileSync(process.env.CAPTURE_ENVELOPES, JSON.stringify(envelopes));
};
"#;

    #[test]
    fn embedded_extension_overflow_fails_closed_and_signals_once() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("runner-status.mjs"), EXTENSION_SOURCE).unwrap();
        let driver = root.path().join("overflow.mjs");
        fs::write(&driver, format!("{IPC_FIXTURE}{}", r#"
import assert from "node:assert/strict";
import extension from "./runner-status.mjs";
const handlers = new Map();
await extension({ on(name, handler) { handlers.set(name, handler); } });
const fire = (type, fields = {}) => handlers.get(type)({ type, ...fields }, { mode: "tui" });
let connects = 0;
const connect = net.Socket.prototype.connect;
net.Socket.prototype.connect = function(...args) { connects += 1; return connect.apply(this, args); };
await fire("agent_start");
if (process.env.ABSENT_CONTROL) await new Promise(resolve => server.close(resolve));
const started = performance.now();
const queued = [];
const bytes = process.env.OVERFLOW === "bytes";
for (let index = 0; index < (bytes ? 3 : 65); index += 1) {
  queued.push(fire("tool_execution_start", { toolCallId: String(index), toolName: bytes ? "你".repeat(2 * 1024 * 1024) : "bash" }));
}
await Promise.all(queued);
await fire("agent_settled");
await fire("agent_start");
await fire("session_shutdown", { reason: "quit" });
assert.equal(connects, 2, "one ordinary report and one control attempt only");
assert.ok(performance.now() - started < 1500, "overflow must remain bounded and neutral");
if (!process.env.ABSENT_CONTROL) {
  assert.equal(envelopes.length, 2);
  assert.equal(envelopes[1].bridge_unavailable, true);
  assert.deepEqual(envelopes[1].payload, {});
  await finish();
} else {
  assert.equal(envelopes.length, 1);
}
"#)).unwrap();
        for mode in ["count", "bytes", "stalled", "absent"] {
            let capture = root.path().join(format!("{mode}.json"));
            let mut command = Command::new("node");
            command
                .arg(&driver)
                .current_dir(root.path())
                .env("OVERFLOW", mode)
                .env("CAPTURE_ENVELOPES", &capture);
            if mode == "stalled" {
                command.env("STALL_CONTROL", "1");
            }
            if mode == "absent" {
                command.env("ABSENT_CONTROL", "1");
            }
            let output = command
                .output()
                .expect("Node is required for pi producer regression");
            assert!(output.status.success(), "{mode}: {output:?}");
            assert!(
                output.stdout.is_empty() && output.stderr.is_empty(),
                "{mode}: {output:?}"
            );
            if mode == "absent" {
                continue;
            }
            let reports: Vec<runner_core::protocol::hook::HookReport> =
                serde_json::from_slice(&fs::read(capture).unwrap()).unwrap();
            let routes = std::sync::Arc::new(crate::session::hook_queue::HookRoutes::default());
            let mut receiver = routes.register(Runtime::Pi, "runner".into(), "current".into());
            for report in reports {
                drop(routes.admit(report).unwrap());
            }
            assert!(receiver
                .drain(|_| panic!("overflowed telemetry reached the parser"))
                .is_err());
        }
    }

    #[test]
    fn embedded_editor_tracking_samples_transitions_and_stops_on_restart_and_shutdown() {
        if Command::new("node").arg("--version").output().is_err() {
            eprintln!("skipping pi extension test: node is not on PATH");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("runner-status.mjs"), EXTENSION_SOURCE).unwrap();
        let driver = root.path().join("driver.mjs");
        fs::write(&driver, format!("{IPC_FIXTURE}{}", r#"
import assert from "node:assert/strict";
import extension from "./runner-status.mjs";
const intervals = new Map();
const deferred = new Map();
let nextTimer = 0;
globalThis.setInterval = (callback, ms) => {
  assert.equal(ms, 100);
  intervals.set(++nextTimer, callback);
  return nextTimer;
};
globalThis.clearInterval = id => intervals.delete(id);
globalThis.setTimeout = (callback, ms) => {
  assert.equal(ms, 0);
  deferred.set(++nextTimer, callback);
  return nextTimer;
};
globalThis.clearTimeout = id => deferred.delete(id);
const handlers = new Map();
await extension({ on(name, handler) { handlers.set(name, handler); } });
let text = "";
let listener;
let unsubscribed = 0;
const ui = {
  getEditorText() { return text; },
  onTerminalInput(callback) {
    listener = callback;
    return () => { listener = undefined; unsubscribed += 1; };
  },
};
const ctx = { mode: "tui", ui, sessionManager: { getSessionId: () => "probe-session" } };
const fire = (type, fields = {}) => handlers.get(type)({ type, ...fields }, ctx);
const drafts = () => reports.filter(report => report.hook_event_name === "editor_draft");
const settle = () => fire("agent_settled");
const flush = () => {
  const callbacks = [...deferred.values()];
  deferred.clear();
  callbacks.forEach(callback => callback());
};
const poll = () => [...intervals.values()].forEach(callback => callback());
await fire("session_start", { reason: "startup" });
await settle();
assert.deepEqual(drafts().map(report => report.drafting), [false]);
listener("x");
await settle();
assert.equal(drafts().length, 1);
text = "x";
flush();
await settle();
assert.deepEqual(drafts().map(report => report.drafting), [false, true]);
listener("more"); text += "more"; flush(); poll();
await settle();
assert.equal(drafts().length, 2);
listener("backspace"); text = ""; flush(); poll();
await settle();
assert.deepEqual(drafts().map(report => report.drafting), [false, true, false]);
text = "pasted\ntext"; poll();
text = ""; poll();
await settle();
assert.deepEqual(drafts().map(report => report.drafting), [false, true, false, true, false]);
listener("pending");
await fire("session_start", { reason: "reload" });
assert.equal(unsubscribed, 1);
assert.equal(intervals.size, 1);
assert.equal(deferred.size, 0);
await settle();
assert.equal(drafts().length, 6);
text = "submitted prompt"; poll();
listener("return"); text = ""; flush();
await settle();
assert.deepEqual(drafts().slice(-2).map(report => report.drafting), [true, false]);
listener("pending shutdown");
await fire("session_shutdown", { reason: "quit" });
assert.equal(unsubscribed, 2);
assert.equal(intervals.size, 0);
assert.equal(deferred.size, 0);
assert.equal(listener, undefined);
await settle();
const count = drafts().length;
text = "after shutdown"; poll(); flush();
await settle();
assert.equal(drafts().length, count);
await fire("session_start", { reason: "new" });
ctx.mode = "print";
await fire("session_start", { reason: "restart" });
assert.equal(unsubscribed, 3);
assert.equal(intervals.size, 0);
assert.equal(deferred.size, 0);
ctx.mode = "tui";
for (const missing of [{}, { getEditorText: ui.getEditorText }, { onTerminalInput: ui.onTerminalInput }]) {
  ctx.ui = missing;
  await fire("session_start", { reason: "startup" });
  assert.equal(intervals.size, 0);
  assert.equal(deferred.size, 0);
}
await settle();
assert.equal(drafts().length, count + 1);
await settle();
assert.ok(drafts().every(report => Object.keys(report).sort().join() === "drafting,hook_event_name"));
await finish();
"#)).unwrap();
        let output = Command::new("node")
            .arg(&driver)
            .current_dir(root.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stdout.is_empty() && output.stderr.is_empty());
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
            format!("{IPC_FIXTURE}{}", r#"
import extension from "./runner-status.mjs";
const handlers = new Map();
await extension({ on(name, handler) { handlers.set(name, handler); } });
const expected = Number(process.env.EXPECT_HANDLERS || "12");
if (handlers.size !== expected) throw new Error(`handlers ${handlers.size}, expected ${expected}`);
if (handlers.size === 0) { await finish(); process.exit(0); }
let sessionId = process.env.TEST_SESSION_ID;
const ctx = {
  mode: "print",
  sessionManager: { getSessionId() { return sessionId; } },
};
await handlers.get("agent_start")({ type: "agent_start" }, ctx);
ctx.mode = "tui";
const fire = (type, fields = {}) => handlers.get(type)({ type, ...fields }, ctx);
await fire("session_start", { reason: "startup" });
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
}
if (process.env.RETURN_SESSION_ID) {
  sessionId = process.env.RETURN_SESSION_ID;
  await fire("session_start", { reason: "resume" });
}
await finish();
"#),
        )
        .unwrap();

        let session_key = "11111111-1111-4111-8111-111111111111";
        let new_key = "22222222-2222-4222-8222-222222222222";
        let capture = root.path().join("reports.json");
        let output = Command::new("node")
            .arg(&driver)
            .current_dir(root.path())
            .env("CAPTURE_REPORTS", &capture)
            .env("TEST_SESSION_ID", session_key)
            .env("NEXT_SESSION_ID", new_key)
            .env("RETURN_SESSION_ID", session_key)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stdout.is_empty() && output.stderr.is_empty());
        let reports: Vec<Value> = serde_json::from_slice(&fs::read(&capture).unwrap()).unwrap();
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
        let mut watcher = TestWatcher::new("current".into());
        for payload in reports {
            watcher.hooks.admit(payload).unwrap();
        }
        let mut values = Vec::new();
        let mut keys = Vec::new();
        watcher
            .drain_with_session_starts(|value, _| values.push(value), |key| keys.push(key))
            .unwrap();
        assert_eq!(keys, [session_key, new_key, session_key]);
        assert_eq!(values.first().unwrap().activity, Activity::Idle);
        assert_eq!(values.first().unwrap().source, ObservationSource::Hook);
        assert_eq!(values[1].activity, Activity::Working);
        assert!(values
            .iter()
            .any(|value| value.outcome == Some(TurnOutcome::Failed)));
        assert_eq!(values.last().unwrap().activity, Activity::Idle);
        assert_eq!(values.last().unwrap().outcome, None);

        fs::write(
            package.join("index.js"),
            "export const VERSION = '0.84.3';\n",
        )
        .unwrap();
        let output = Command::new("node")
            .arg(&driver)
            .env("EXPECT_HANDLERS", "0")
            .current_dir(root.path())
            .env("TEST_SESSION_ID", new_key)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stdout.is_empty() && output.stderr.is_empty());
    }
}
