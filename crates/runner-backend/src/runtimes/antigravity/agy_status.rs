use crate::model::Runtime;
use crate::session::state::agent::{AdapterFeedback, AgentEvent};
#[cfg(test)]
use crate::session::state::StatusSource;
// Hook status for Antigravity CLI on macOS (spec 644 decision 5).
//
// Every spawn loads `--add-dir <app data>/antigravity-hooks`, a Runner-owned
// folder whose `.agents/hooks.json` agy reads for that launch only, next to
// the user's global hooks. Nothing is written into `~/.gemini`. The hooks
// call one reporter script that appends to the session's `hook_feed` through
// the per-session env vars below.
//
// agy reads a JSON reply from every hook's stdout, and a `PreToolUse` reply is
// a permission decision (`{}` denied every tool in the probe), so Runner
// registers no `PreToolUse` and answers `{}` to every event it does register.
// agy has no event for a permission prompt or a question on screen, so
// hooks report Working, Idle and Response failed only.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::Result;
use crate::session::hook_feed::HookFeed;
#[cfg(test)]
use crate::session::state::CTRL_C_INTERRUPT;
use crate::session::status::TurnOutcome;

#[cfg(test)]
use crate::session::status::{Activity, AgentObservation, ObservationSource};
pub(crate) const PATH_ENV: &str = "RUNNER_ANTIGRAVITY_STATUS_PATH";
pub(crate) const GENERATION_ENV: &str = "RUNNER_ANTIGRAVITY_STATUS_GENERATION";
pub(crate) const EVENTS: &[&str] = &["PreInvocation", "PostToolUse", "PostInvocation", "Stop"];

const HOOKS_DIR: &str = "antigravity-hooks";
const HOOK_NAME: &str = "runner-status";
const REPORTER: &str = "report.sh";
const REPORTER_SCRIPT: &str = r#"#!/bin/sh
if [ -n "$1" ] && payload=$(mktemp "$1.XXXXXXXX" 2>/dev/null); then
  if cat >"$payload" 2>/dev/null; then
    printf '{"generation":"%s","hook_event_name":"%s","payload_file":"%s"}\n' "$RUNNER_ANTIGRAVITY_STATUS_GENERATION" "$2" "${payload##*/}" >> "$1" 2>/dev/null || rm -f "$payload"
  else
    rm -f "$payload"
  fi
else
  cat >/dev/null 2>&1
fi
printf '{}\n'
exit 0
"#;

pub(crate) fn hooks_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(HOOKS_DIR)
}

pub(crate) fn reporter_path(app_data_dir: &Path) -> PathBuf {
    hooks_dir(app_data_dir).join(REPORTER)
}

fn hooks_path(app_data_dir: &Path) -> PathBuf {
    hooks_dir(app_data_dir).join(".agents/hooks.json")
}

pub(crate) fn hooks_available(app_data_dir: &Path) -> bool {
    reporter_path(app_data_dir).is_file() && hooks_path(app_data_dir).is_file()
}

fn write_hooks_file(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path.parent().expect("hooks file has a parent");
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(contents)?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn hooks_json(app_data_dir: &Path) -> serde_json::Value {
    let reporter =
        crate::session::launch::shell_quote(&reporter_path(app_data_dir).to_string_lossy());
    let mut events = serde_json::Map::new();
    for event in EVENTS {
        let handler = serde_json::json!({
            "type": "command",
            "command": format!("sh {reporter} \"${PATH_ENV}\" {event}"),
            "timeout": 2,
        });
        // Tool events wrap their handlers in a matcher group; the others
        // take a flat list.
        let entry = if *event == "PostToolUse" {
            serde_json::json!({"matcher": "*", "hooks": [handler]})
        } else {
            handler
        };
        events.insert((*event).into(), serde_json::Value::Array(vec![entry]));
    }
    serde_json::json!({ HOOK_NAME: events })
}

pub(crate) fn install_hooks(app_data_dir: &Path) -> Result<()> {
    fs::create_dir_all(hooks_dir(app_data_dir).join(".agents"))?;
    write_hooks_file(&reporter_path(app_data_dir), REPORTER_SCRIPT.as_bytes())?;
    write_hooks_file(
        &hooks_path(app_data_dir),
        &serde_json::to_vec_pretty(&hooks_json(app_data_dir))?,
    )
}

#[derive(Default, Deserialize)]
struct StatusReport {
    #[serde(default)]
    hook_event_name: String,
    #[serde(rename = "fullyIdle")]
    fully_idle: Option<bool>,
    error: Option<String>,
}

#[derive(Clone, Default)]
struct AgyParser;
impl AgyParser {
    fn hook(&mut self, report: StatusReport) -> Option<Vec<AgentEvent>> {
        Some(vec![match report.hook_event_name.as_str() {
            "PreInvocation" => AgentEvent::TurnStarted,
            "PostToolUse" | "PostInvocation" => AgentEvent::Working { detail: None },
            "Stop"
                if report
                    .error
                    .as_deref()
                    .is_some_and(|error| !error.is_empty()) =>
            {
                AgentEvent::TurnEnded {
                    outcome: TurnOutcome::Failed,
                }
            }
            "Stop" if report.fully_idle == Some(true) => AgentEvent::TurnEnded {
                outcome: TurnOutcome::Completed,
            },
            _ => return None,
        }])
    }
}

pub(crate) struct AgyStatusWatcher {
    feed: HookFeed,
    parser: AgyParser,
}
impl AgyStatusWatcher {
    pub(crate) fn start(path: &Path, generation: String) -> Result<Self> {
        let app_data_dir = path
            .parent()
            .and_then(Path::parent)
            .expect("status file is under app data");
        Ok(Self {
            feed: HookFeed::start_external(path, generation, &reporter_path(app_data_dir))?,
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
                        runtime: Runtime::Antigravity,
                        events,
                    });
                }
            }
        })?;
        if cancel != 0 {
            emit(AgentEvent::Batch {
                runtime: Runtime::Antigravity,
                events: vec![AgentEvent::LocalCancel { kind: cancel }],
            });
        }
        Ok(())
    }
}
impl crate::session::hook_feed::HookWatcher for AgyStatusWatcher {
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
struct AgyObservation {
    parser: AgyParser,
    model: crate::session::state::agent::AgentModel,
    value: crate::session::state::agent::TurnState,
}
#[cfg(test)]
impl std::ops::Deref for AgyObservation {
    type Target = AgyParser;
    fn deref(&self) -> &Self::Target {
        &self.parser
    }
}
#[cfg(test)]
impl std::ops::DerefMut for AgyObservation {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.parser
    }
}
#[cfg(test)]
impl AgyObservation {
    fn observe(&mut self, event: StatusReport, now: i64) -> Option<AgentObservation> {
        let events = self.parser.hook(event)?;
        let reduced = self.model.reduce(
            AgentEvent::Batch {
                runtime: Runtime::Antigravity,
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
    use serde_json::{json, Value};

    use super::*;

    struct TestWatcher {
        inner: AgyStatusWatcher,
        observation: AgyObservation,
        cancel: u8,
    }
    impl std::ops::Deref for TestWatcher {
        type Target = AgyStatusWatcher;
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
                inner: AgyStatusWatcher::start(path, generation)?,
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

    fn observe(state: &mut AgyObservation, value: Value) -> Option<AgentObservation> {
        state.observe(
            serde_json::from_value::<StatusReport>(value).unwrap(),
            crate::session::clock::timestamp_millis(),
        )
    }

    #[test]
    fn hooks_register_no_pre_tool_use_and_follow_agys_schema() {
        let root = tempfile::tempdir().unwrap();
        assert!(!hooks_available(root.path()));
        install_hooks(root.path()).unwrap();
        assert!(hooks_available(root.path()));

        let hooks: Value =
            serde_json::from_slice(&fs::read(hooks_path(root.path())).unwrap()).unwrap();
        let named = hooks.as_object().unwrap();
        assert_eq!(named.keys().collect::<Vec<_>>(), [HOOK_NAME]);
        let events = named[HOOK_NAME].as_object().unwrap();
        assert_eq!(
            events.keys().map(String::as_str).collect::<Vec<_>>(),
            EVENTS
        );
        assert!(!events.contains_key("PreToolUse"));
        assert!(!fs::read_to_string(hooks_path(root.path()))
            .unwrap()
            .contains("PreToolUse"));

        let reporter =
            crate::session::launch::shell_quote(&reporter_path(root.path()).to_string_lossy());
        for event in EVENTS {
            let handler = if *event == "PostToolUse" {
                assert_eq!(events[*event][0]["matcher"], "*");
                &events[*event][0]["hooks"][0]
            } else {
                &events[*event][0]
            };
            assert_eq!(handler["type"], "command");
            assert_eq!(handler["timeout"], 2);
            assert_eq!(
                handler["command"],
                format!("sh {reporter} \"${PATH_ENV}\" {event}")
            );
        }

        fs::write(reporter_path(root.path()), "broken").unwrap();
        install_hooks(root.path()).unwrap();
        assert_eq!(
            fs::read_to_string(reporter_path(root.path())).unwrap(),
            REPORTER_SCRIPT
        );
    }

    #[test]
    fn working_idle_and_failed_follow_decision_five() {
        let mut state = AgyObservation::default();
        for event in ["PreInvocation", "PostToolUse", "PostInvocation"] {
            let working = observe(&mut state, json!({"hook_event_name": event})).unwrap();
            assert_eq!(working.activity, Activity::Working);
            assert_eq!(working.outcome, None);
            assert_eq!(working.source, ObservationSource::Hook);
        }
        assert!(observe(
            &mut state,
            json!({"hook_event_name":"Stop","fullyIdle":false,"terminationReason":"NO_TOOL_CALL"}),
        )
        .is_none());
        assert_eq!(state.value.activity, Activity::Working);

        let idle = observe(
            &mut state,
            json!({"hook_event_name":"Stop","fullyIdle":true,"terminationReason":"NO_TOOL_CALL","error":""}),
        )
        .unwrap();
        assert_eq!(idle.activity, Activity::Ready);
        assert_eq!(idle.outcome, Some(TurnOutcome::Completed));

        observe(&mut state, json!({"hook_event_name":"PreInvocation"}));
        let failed = observe(
            &mut state,
            json!({"hook_event_name":"Stop","fullyIdle":true,"terminationReason":"error","error":"quota exceeded"}),
        )
        .unwrap();
        assert_eq!(failed.activity, Activity::Ready);
        assert_eq!(failed.outcome, Some(TurnOutcome::Failed));

        assert!(observe(&mut state, json!({"hook_event_name":"PreToolUse"})).is_none());
        assert!(observe(&mut state, json!({"hook_event_name":"Notification"})).is_none());
    }

    #[test]
    fn interrupted_invocation_without_stop_returns_to_ready() {
        use crate::session::state::ESCAPE_INTERRUPT;

        let root = tempfile::tempdir().unwrap();
        install_hooks(root.path()).unwrap();
        let path = crate::session::hook_feed::status_path(root.path(), "agy");
        let mut watcher = TestWatcher::start(&path, "current".into()).unwrap();
        let mut feed = fs::OpenOptions::new().append(true).open(&path).unwrap();
        let mut transitions = Vec::new();
        fn drain(
            watcher: &mut TestWatcher,
            transitions: &mut Vec<(Activity, Option<TurnOutcome>, StatusSource)>,
        ) {
            watcher
                .drain_status(|value, source| {
                    transitions.push((value.activity, value.outcome, source));
                })
                .unwrap();
        }

        writeln!(
            feed,
            r#"{{"generation":"current","hook_event_name":"PreInvocation"}}"#
        )
        .unwrap();
        drain(&mut watcher, &mut transitions);
        watcher.cancel = ESCAPE_INTERRUPT;
        drain(&mut watcher, &mut transitions);
        assert_eq!(
            transitions,
            [
                (Activity::Working, None, StatusSource::Hook),
                (
                    Activity::Ready,
                    Some(TurnOutcome::Interrupted),
                    StatusSource::InputEscape
                ),
            ]
        );

        writeln!(
            feed,
            r#"{{"generation":"current","hook_event_name":"PostToolUse"}}"#
        )
        .unwrap();
        drain(&mut watcher, &mut transitions);
        assert_eq!(transitions.len(), 2);
        assert_eq!(watcher.observation.value.activity, Activity::Ready);
        assert_eq!(
            watcher.observation.value.outcome,
            Some(TurnOutcome::Interrupted)
        );

        writeln!(
            feed,
            r#"{{"generation":"current","hook_event_name":"PreInvocation"}}"#
        )
        .unwrap();
        drain(&mut watcher, &mut transitions);
        watcher.cancel = CTRL_C_INTERRUPT;
        drain(&mut watcher, &mut transitions);
        assert_eq!(
            transitions[2],
            (Activity::Working, None, StatusSource::Hook)
        );
        assert_eq!(
            transitions[3],
            (
                Activity::Ready,
                Some(TurnOutcome::Interrupted),
                StatusSource::InputInterrupt
            )
        );

        writeln!(
            feed,
            r#"{{"generation":"current","hook_event_name":"PostInvocation"}}"#
        )
        .unwrap();
        writeln!(
            feed,
            r#"{{"generation":"current","hook_event_name":"Stop","fullyIdle":true}}"#
        )
        .unwrap();
        drain(&mut watcher, &mut transitions);
        assert_eq!(transitions.len(), 4);
        assert_eq!(watcher.observation.value.activity, Activity::Ready);
        assert_eq!(
            watcher.observation.value.outcome,
            Some(TurnOutcome::Interrupted)
        );

        writeln!(
            feed,
            r#"{{"generation":"current","hook_event_name":"PreInvocation"}}"#
        )
        .unwrap();
        writeln!(
            feed,
            r#"{{"generation":"current","hook_event_name":"Stop","fullyIdle":true}}"#
        )
        .unwrap();
        watcher.cancel = ESCAPE_INTERRUPT;
        drain(&mut watcher, &mut transitions);
        assert_eq!(
            transitions[4],
            (Activity::Working, None, StatusSource::Hook)
        );
        assert_eq!(
            transitions[5],
            (
                Activity::Ready,
                Some(TurnOutcome::Completed),
                StatusSource::Hook
            )
        );
        assert_eq!(transitions.len(), 6);
    }

    #[test]
    #[cfg(unix)]
    fn reporter_answers_empty_json_and_appends_a_payload_pointer() {
        use std::process::{Command, Stdio};

        let root = tempfile::tempdir().unwrap();
        install_hooks(root.path()).unwrap();
        let feed = crate::session::hook_feed::status_path(root.path(), "agy");
        fs::create_dir_all(feed.parent().unwrap()).unwrap();
        fs::write(&feed, "").unwrap();

        let run = |feed_env: Option<&Path>| {
            let mut command = Command::new("sh");
            command
                .arg("-c")
                .arg(format!(
                    "sh {} \"${PATH_ENV}\" Stop",
                    crate::session::launch::shell_quote(
                        &reporter_path(root.path()).to_string_lossy()
                    )
                ))
                .env_remove(PATH_ENV)
                .env(GENERATION_ENV, "gen-1")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped());
            if let Some(feed) = feed_env {
                command.env(PATH_ENV, feed);
            }
            let mut child = command.spawn().unwrap();
            {
                use std::io::Write as _;
                let mut stdin = child.stdin.take().unwrap();
                stdin
                    .write_all(br#"{"conversationId":"c","fullyIdle":true}"#)
                    .unwrap();
            }
            let output = child.wait_with_output().unwrap();
            assert!(output.status.success());
            String::from_utf8(output.stdout).unwrap()
        };

        assert_eq!(run(Some(&feed)), "{}\n");
        let pointer: Value =
            serde_json::from_str(fs::read_to_string(&feed).unwrap().trim()).unwrap();
        assert_eq!(pointer["generation"], "gen-1");
        assert_eq!(pointer["hook_event_name"], "Stop");
        let payload = feed.with_file_name(pointer["payload_file"].as_str().unwrap());
        assert_eq!(
            fs::read_to_string(payload).unwrap(),
            r#"{"conversationId":"c","fullyIdle":true}"#
        );

        assert_eq!(run(None), "{}\n");
    }
}
