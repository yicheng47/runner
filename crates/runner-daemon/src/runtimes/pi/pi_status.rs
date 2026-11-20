use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::error::Result;
use crate::session::hook_feed::{HookFeed, TranscriptTail};
use crate::session::state::agent::{AdapterFeedback, AgentEvent, ProgramOutcomePolicy};
use crate::session::status::TurnOutcome;

#[cfg(test)]
pub(crate) const SESSION_KEY_ENV: &str = "RUNNER_PI_SESSION_KEY";
#[cfg(test)]
pub(crate) const REKEY_PATH_ENV: &str = "RUNNER_PI_REKEY_PATH";

const EXTENSION_DIR: &str = "pi-hooks";
const EXTENSION_FILE: &str = "runner-status.ts";

const EXTENSION_SOURCE: &str = r#"import net from "node:net";
import { setTimeout as deadlineTimeout, clearTimeout as clearDeadline } from "node:timers";

const minimumVersion = [1, 1, 0];

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

  function sessionFileOf(ctx) {
    try {
      return typeof ctx.sessionManager?.getSessionFile === "function"
        ? ctx.sessionManager.getSessionFile() ?? null
        : null;
    } catch {
      return null;
    }
  }

  pi.on("session_start", async (event, ctx) => {
    stopEditorTracking();
    if (ctx.mode !== "tui") return;
    try {
      const sessionId = ctx.sessionManager.getSessionId();
      await append(event, {
        reason: event.reason,
        session_id: sessionId,
        session_file: sessionFileOf(ctx),
      });
    } catch {}
    trackEditor(ctx);
  });

  pi.on("input", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    let sessionId = null;
    try {
      sessionId = typeof ctx.sessionManager?.getSessionId === "function"
        ? ctx.sessionManager.getSessionId() ?? null
        : null;
    } catch {}
    return append(event, {
      text: typeof event.text === "string" ? event.text.slice(0, 4096) : null,
      session_id: sessionId,
      session_file: sessionFileOf(ctx),
    });
  });

  pi.on("ui_prompt_start", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    return append(event, { kind: event.kind });
  });

  pi.on("ui_prompt_end", (event, ctx) => {
    if (ctx.mode !== "tui") return;
    return append(event, { kind: event.kind });
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
    kind: Option<String>,
    pub(crate) session_id: Option<String>,
    #[serde(alias = "transcript_path")]
    session_file: Option<PathBuf>,
    #[serde(alias = "prompt")]
    text: Option<String>,
    drafting: Option<bool>,
}

#[derive(Clone, Default)]
struct PiParser {
    session_id: Option<String>,
    transcript_path: Option<PathBuf>,
}

impl PiParser {
    fn hook(&mut self, report: StatusReport) -> Option<AgentEvent> {
        match report.hook_event_name.as_str() {
            "editor_draft" => Some(AgentEvent::EditorDraft {
                drafting: report.drafting?,
            }),
            "session_start" => {
                if report.reason.as_deref() == Some("reload") {
                    if let Some(path) = report.session_file {
                        self.transcript_path = Some(path);
                    }
                    return None;
                }
                let session = report.session_id.filter(|id| !id.is_empty())?;
                let conversation_changed = self
                    .session_id
                    .as_ref()
                    .is_some_and(|prev| *prev != session);
                self.session_id = Some(session);
                if let Some(path) = report.session_file {
                    self.transcript_path = Some(path);
                } else if conversation_changed {
                    self.transcript_path = None;
                }
                Some(AgentEvent::ProgramSessionStarted {
                    conversation_changed,
                    compacted: false,
                })
            }
            "input" => {
                if let Some(id) = report.session_id.filter(|id| !id.is_empty()) {
                    if self.session_id.is_none() {
                        self.session_id = Some(id);
                    }
                }
                if let Some(path) = report.session_file {
                    self.transcript_path = Some(path);
                }
                Some(AgentEvent::ProgramTurnConfirmed {
                    conversation_changed: false,
                    prompt: report.text,
                })
            }
            "ui_prompt_start"
                if matches!(
                    report.kind.as_deref(),
                    Some("confirm" | "select" | "input" | "editor" | "custom")
                ) =>
            {
                Some(AgentEvent::ProgramWaitOverlay { active: true })
            }
            "ui_prompt_end"
                if matches!(
                    report.kind.as_deref(),
                    Some("confirm" | "select" | "input" | "editor" | "custom")
                ) =>
            {
                Some(AgentEvent::ProgramWaitOverlay { active: false })
            }
            "session_shutdown" => Some(AgentEvent::ProgramWaitOverlay { active: false }),
            _ => None,
        }
    }

    fn record(&mut self, entry: &Value) -> Option<AgentEvent> {
        if entry["type"] != "message" {
            return None;
        }
        let message = &entry["message"];
        if message["role"] != "assistant" {
            return None;
        }
        match message["stopReason"].as_str()? {
            "aborted" => Some(AgentEvent::ProgramTurnOutcome {
                outcome: TurnOutcome::Interrupted,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
            }),
            "error" if message["errorMessage"].as_str() == Some("This operation was aborted") => {
                Some(AgentEvent::ProgramTurnOutcome {
                    outcome: TurnOutcome::Interrupted,
                    policy: ProgramOutcomePolicy::DeferWhileNativeActive,
                })
            }
            "error" => Some(AgentEvent::ProgramTurnOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::NativeFallbackOnly,
            }),
            "stop" | "length" => Some(AgentEvent::ProgramTurnFinished),
            _ => None,
        }
    }
}

pub(crate) struct PiStatusWatcher {
    feed: HookFeed,
    parser: PiParser,
    transcript: Option<TranscriptTail>,
}

impl PiStatusWatcher {
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
                let is_start = report.hook_event_name == "session_start";
                let is_reload = report.reason.as_deref() == Some("reload");
                if is_start {
                    if let Some(id) = report
                        .session_id
                        .as_deref()
                        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
                    {
                        session_start(id.to_owned());
                    }
                }
                let previous_path = self.parser.transcript_path.clone();
                let previous_session = self.parser.session_id.clone();
                if let Some(event) = self.parser.hook(report) {
                    if self.parser.session_id != previous_session
                        || self.parser.transcript_path != previous_path
                    {
                        self.transcript = None;
                        if is_start && !is_reload {
                            if let Some(path) = self.parser.transcript_path.as_deref() {
                                self.transcript = TranscriptTail::open_at_end(path).ok();
                            }
                        }
                    }
                    emit(event);
                }
            }
        })?;
        self.drain_transcript(&mut emit);
        Ok(())
    }

    fn drain_transcript(&mut self, emit: &mut impl FnMut(AgentEvent) -> AdapterFeedback) {
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
mod tests {
    use std::fs::{self, File};
    use std::process::Command;

    use serde_json::{json, Value};

    use super::*;
    use crate::model::Runtime;

    fn report(value: Value) -> StatusReport {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn editor_draft_requires_a_boolean_and_legacy_status_hooks_are_dropped() {
        let mut parser = PiParser::default();
        for drafting in [true, false] {
            let event = parser
                .hook(report(json!({
                    "hook_event_name": "editor_draft",
                    "drafting": drafting,
                })))
                .unwrap();
            assert!(matches!(event, AgentEvent::EditorDraft { drafting: v } if v == drafting));
        }
        assert!(parser
            .hook(report(json!({"hook_event_name": "editor_draft"})))
            .is_none());
        assert!(serde_json::from_value::<StatusReport>(json!({
            "hook_event_name": "editor_draft",
            "drafting": "true",
        }))
        .is_err());

        for dropped in [
            "agent_start",
            "tool_execution_start",
            "tool_execution_end",
            "session_before_compact",
            "session_compact",
            "session_compact_failed",
            "message_end",
            "agent_settled",
        ] {
            assert!(
                parser
                    .hook(report(json!({"hook_event_name": dropped})))
                    .is_none(),
                "legacy pi status hook {dropped} must be dropped"
            );
        }

        // ui_prompt_start/ui_prompt_end track the outer extension prompt lifetime as a wait
        // overlay (deduplicated behind native OSC 7501 blocked) so stacked unannotated custom
        // dialogs retain a hold; unknown or missing kinds are dropped.
        for ignored_kind in [None, Some("unknown")] {
            for event_name in ["ui_prompt_start", "ui_prompt_end"] {
                let mut payload = json!({"hook_event_name": event_name});
                if let Some(kind) = ignored_kind {
                    payload["kind"] = json!(kind);
                }
                assert!(
                    parser.hook(report(payload)).is_none(),
                    "{event_name} kind={ignored_kind:?} must be dropped"
                );
            }
        }
        for kind in ["confirm", "select", "input", "editor", "custom"] {
            assert!(matches!(
                parser.hook(report(
                    json!({"hook_event_name": "ui_prompt_start", "kind": kind})
                )),
                Some(AgentEvent::ProgramWaitOverlay { active: true })
            ));
            assert!(matches!(
                parser.hook(report(
                    json!({"hook_event_name": "ui_prompt_end", "kind": kind})
                )),
                Some(AgentEvent::ProgramWaitOverlay { active: false })
            ));
        }
        assert!(matches!(
            parser.hook(report(
                json!({"hook_event_name": "session_shutdown", "reason": "quit"})
            )),
            Some(AgentEvent::ProgramWaitOverlay { active: false })
        ));
    }

    #[test]
    fn session_start_input_and_transcript_outcomes_preserve_reload_and_skip_resume_history() {
        let dir = tempfile::tempdir().unwrap();
        let session_file = dir.path().join("session.jsonl");
        let mut file = File::create(&session_file).unwrap();
        // Pre-existing aborted assistant message before resume must be skipped.
        writeln!(
            file,
            "{}",
            json!({
                "type": "message",
                "message": {"role": "assistant", "stopReason": "aborted"}
            })
        )
        .unwrap();
        file.flush().unwrap();

        let (hooks, receiver) =
            crate::session::hook_queue::TestHookRoute::new(Runtime::Pi, "current".into());
        let mut watcher = PiStatusWatcher::from_receiver(receiver);
        let key = "11111111-1111-4111-8111-111111111111";
        let next_key = "22222222-2222-4222-8222-222222222222";
        hooks
            .admit(json!({
                "generation": "current",
                "hook_event_name": "session_start",
                "reason": "resume",
                "session_id": key,
                "session_file": session_file,
            }))
            .unwrap();
        hooks
            .admit(json!({
                "generation": "current",
                "hook_event_name": "session_start",
                "reason": "reload",
                "session_id": key,
                "session_file": session_file,
            }))
            .unwrap();

        let mut events = Vec::new();
        let mut starts = Vec::new();
        watcher
            .drain_events(
                0,
                |event| {
                    events.push(event);
                    Default::default()
                },
                |id| starts.push(id),
            )
            .unwrap();
        assert_eq!(starts, [key, key]);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            AgentEvent::ProgramSessionStarted {
                conversation_changed: false,
                compacted: false,
            }
        ));
        events.clear();

        hooks
            .admit(json!({
                "generation": "current",
                "hook_event_name": "input",
                "text": "[inbox] hello",
                "session_id": key,
                "session_file": session_file,
            }))
            .unwrap();
        for entry in [
            json!({"type": "message", "message": {"role": "user", "content": "hi"}}),
            json!({"type": "message", "message": {"role": "assistant", "stopReason": "error", "errorMessage": "This operation was aborted"}}),
            json!({"type": "message", "message": {"role": "assistant", "stopReason": "error", "errorMessage": "Provider unavailable"}}),
            json!({"type": "message", "message": {"role": "assistant", "stopReason": "stop"}}),
        ] {
            writeln!(file, "{entry}").unwrap();
        }
        file.flush().unwrap();

        watcher
            .drain_events(
                0,
                |event| {
                    events.push(event);
                    Default::default()
                },
                |id| starts.push(id),
            )
            .unwrap();
        assert_eq!(events.len(), 4);
        assert!(matches!(
            events[0],
            AgentEvent::ProgramTurnConfirmed {
                conversation_changed: false,
                prompt: Some(ref text),
            } if text == "[inbox] hello"
        ));
        assert!(matches!(
            events[1],
            AgentEvent::ProgramTurnOutcome {
                outcome: TurnOutcome::Interrupted,
                policy: ProgramOutcomePolicy::DeferWhileNativeActive,
            }
        ));
        assert!(matches!(
            events[2],
            AgentEvent::ProgramTurnOutcome {
                outcome: TurnOutcome::Failed,
                policy: ProgramOutcomePolicy::NativeFallbackOnly,
            }
        ));
        assert!(matches!(events[3], AgentEvent::ProgramTurnFinished));

        events.clear();
        hooks
            .admit(json!({
                "generation": "current",
                "hook_event_name": "session_start",
                "reason": "new",
                "session_id": next_key,
            }))
            .unwrap();
        watcher
            .drain_events(
                0,
                |event| {
                    events.push(event);
                    Default::default()
                },
                |id| starts.push(id),
            )
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [AgentEvent::ProgramSessionStarted {
                conversation_changed: true,
                compacted: false,
            }]
        ));
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
  socket.on("error", () => {});
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

    // A starved CI runner can fire the 250 ms deadline before a reply is read,
    // so tests that check delivery run the extension with a longer one.
    fn extension_with_deadline(ms: u32) -> String {
        let deadline = "performance.now() + 250";
        assert_eq!(EXTENSION_SOURCE.matches(deadline).count(), 2);
        EXTENSION_SOURCE.replace(deadline, &format!("performance.now() + {ms}"))
    }

    #[test]
    fn embedded_extension_overflow_fails_closed_and_signals_once() {
        let root = tempfile::tempdir().unwrap();
        let driver = root.path().join("overflow.mjs");
        fs::write(&driver, format!("{IPC_FIXTURE}{}", r#"
import assert from "node:assert/strict";
import extension from "./runner-status.mjs";
const handlers = new Map();
await extension({ on(name, handler) { handlers.set(name, handler); } });
const ctx = {
  mode: "tui",
  sessionManager: {
    getSessionId: () => "11111111-1111-4111-8111-111111111111",
    getSessionFile: () => "/tmp/pi.jsonl",
  },
};
const fire = (type, fields = {}) => handlers.get(type)({ type, ...fields }, ctx);
let connects = 0;
const connect = net.Socket.prototype.connect;
net.Socket.prototype.connect = function(...args) { connects += 1; return connect.apply(this, args); };
await fire("input", { text: "first" });
if (process.env.ABSENT_CONTROL) await new Promise(resolve => server.close(resolve));
const started = performance.now();
const queued = [];
const bytes = process.env.OVERFLOW === "bytes";
if (bytes) {
  ctx.sessionManager.getSessionFile = () => "你".repeat(2 * 1024 * 1024);
}
for (let index = 0; index < (bytes ? 3 : 65); index += 1) {
  queued.push(fire("input", { text: String(index) }));
}
await Promise.all(queued);
await fire("input", { text: "after" });
await fire("session_shutdown", { reason: "quit" });
assert.equal(connects, 2, "one ordinary report and one control attempt only");
assert.ok(performance.now() - started < Number(process.env.DEADLINE_MS) + 1250, "overflow must remain bounded and neutral");
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
            let deadline = if mode == "stalled" { 2_000 } else { 10_000 };
            fs::write(
                root.path().join("runner-status.mjs"),
                extension_with_deadline(deadline),
            )
            .unwrap();
            let capture = root.path().join(format!("{mode}.json"));
            let mut command = Command::new("node");
            command
                .arg(&driver)
                .current_dir(root.path())
                .env("OVERFLOW", mode)
                .env("DEADLINE_MS", deadline.to_string())
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
        fs::write(
            root.path().join("runner-status.mjs"),
            extension_with_deadline(10_000),
        )
        .unwrap();
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
const settle = () => fire("input", { text: "sync" });
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
    fn embedded_extension_runs_as_mjs_reports_identity_and_rekeys() {
        if Command::new("node").arg("--version").output().is_err() {
            eprintln!("skipping pi extension test: node is not on PATH");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let app_data = root.path().join("app data");
        install_extension(&app_data).unwrap();
        let source = root.path().join("runner-status.mjs");
        fs::write(&source, extension_with_deadline(10_000)).unwrap();
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
            "export const VERSION = '1.1.0';\n",
        )
        .unwrap();
        let driver = root.path().join("driver.mjs");
        fs::write(
            &driver,
            format!(
                "{IPC_FIXTURE}{}",
                r#"
import extension from "./runner-status.mjs";
const handlers = new Map();
await extension({ on(name, handler) { handlers.set(name, handler); } });
const expected = Number(process.env.EXPECT_HANDLERS || "5");
if (handlers.size !== expected) throw new Error(`handlers ${handlers.size}, expected ${expected}`);
if (handlers.size === 0) { await finish(); process.exit(0); }
for (const removed of [
  "agent_start", "tool_execution_start", "tool_execution_end",
  "session_before_compact", "session_compact", "session_compact_failed",
  "message_end", "agent_settled",
]) {
  if (handlers.has(removed)) throw new Error(`unexpected status handler ${removed}`);
}
let sessionId = process.env.TEST_SESSION_ID;
let sessionFile = "/tmp/pi-1.jsonl";
const ctx = {
  mode: "print",
  sessionManager: {
    getSessionId() { return sessionId; },
    getSessionFile() { return sessionFile; },
  },
};
await handlers.get("input")({ type: "input", text: "ignored-in-print" }, ctx);
await handlers.get("ui_prompt_start")({ type: "ui_prompt_start", kind: "custom" }, ctx);
await handlers.get("session_shutdown")({ type: "session_shutdown", reason: "quit" }, ctx);
ctx.mode = "tui";
const fire = (type, fields = {}) => handlers.get(type)({ type, ...fields }, ctx);
await fire("session_start", { reason: "startup" });
await fire("input", { text: "x".repeat(5000) });
await fire("ui_prompt_start", { kind: "confirm", title: "Confirm" });
await fire("ui_prompt_end", { kind: "confirm", title: "Confirm" });
await fire("ui_prompt_start", { kind: "custom" });
await fire("ui_prompt_end", { kind: "custom" });
await fire("session_shutdown", { reason: "quit" });
if (process.env.NEXT_SESSION_ID) {
  sessionId = process.env.NEXT_SESSION_ID;
  sessionFile = "/tmp/pi-2.jsonl";
  await fire("session_start", { reason: "new" });
}
if (process.env.RETURN_SESSION_ID) {
  sessionId = process.env.RETURN_SESSION_ID;
  sessionFile = "/tmp/pi-1.jsonl";
  await fire("session_start", { reason: "resume" });
}
await finish();
"#
            ),
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
        assert_eq!(reports.len(), 9);
        assert_eq!(reports[0]["hook_event_name"], "session_start");
        assert_eq!(reports[0]["session_id"], session_key);
        assert_eq!(reports[0]["session_file"], "/tmp/pi-1.jsonl");
        assert_eq!(reports[1]["hook_event_name"], "input");
        assert_eq!(reports[1]["text"].as_str().unwrap().len(), 4096);
        assert_eq!(reports[2]["hook_event_name"], "ui_prompt_start");
        assert_eq!(reports[2]["kind"], "confirm");
        assert_eq!(reports[3]["hook_event_name"], "ui_prompt_end");
        assert_eq!(reports[3]["kind"], "confirm");
        assert_eq!(reports[4]["hook_event_name"], "ui_prompt_start");
        assert_eq!(reports[4]["kind"], "custom");
        assert_eq!(reports[5]["hook_event_name"], "ui_prompt_end");
        assert_eq!(reports[5]["kind"], "custom");
        assert_eq!(reports[6]["hook_event_name"], "session_shutdown");
        assert_eq!(reports[7]["session_id"], new_key);
        assert_eq!(reports[8]["session_id"], session_key);

        let (hooks, receiver) =
            crate::session::hook_queue::TestHookRoute::new(Runtime::Pi, "current".into());
        let mut watcher = PiStatusWatcher::from_receiver(receiver);
        for mut payload in reports {
            payload["generation"] = json!("current");
            hooks.admit(payload).unwrap();
        }
        let mut events = Vec::new();
        let mut keys = Vec::new();
        watcher
            .drain_events(
                0,
                |event| {
                    events.push(event);
                    Default::default()
                },
                |key| keys.push(key),
            )
            .unwrap();
        assert_eq!(keys, [session_key, new_key, session_key]);
        assert_eq!(events.len(), 9);
        assert!(matches!(
            events[0],
            AgentEvent::ProgramSessionStarted {
                conversation_changed: false,
                ..
            }
        ));
        assert!(matches!(
            events[1],
            AgentEvent::ProgramTurnConfirmed {
                conversation_changed: false,
                prompt: Some(_),
            }
        ));
        assert!(matches!(
            events[2],
            AgentEvent::ProgramWaitOverlay { active: true }
        ));
        assert!(matches!(
            events[3],
            AgentEvent::ProgramWaitOverlay { active: false }
        ));
        assert!(matches!(
            events[4],
            AgentEvent::ProgramWaitOverlay { active: true }
        ));
        assert!(matches!(
            events[5],
            AgentEvent::ProgramWaitOverlay { active: false }
        ));
        assert!(matches!(
            events[6],
            AgentEvent::ProgramWaitOverlay { active: false }
        ));
        assert!(matches!(
            events[7],
            AgentEvent::ProgramSessionStarted {
                conversation_changed: true,
                ..
            }
        ));
        assert!(matches!(
            events[8],
            AgentEvent::ProgramSessionStarted {
                conversation_changed: true,
                ..
            }
        ));

        fs::write(
            package.join("index.js"),
            "export const VERSION = '1.0.9';\n",
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
