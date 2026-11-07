use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::time::Duration;

use runner_cli::client::{ClientError, SocketClient, ToolResponse};
use runner_core::protocol::{ClientEvent, EventError, EventSubscription};
use serde_json::{json, Value};

use crate::env::{BusContext, MissionEnv};
use crate::msg as bus;
use crate::{env, help, output, signal};
#[cfg(test)]
use clap::Parser;
#[cfg(test)]
use std::path::PathBuf;

mod ask;
mod chat;
mod cli;
mod crew;
mod daemon;
mod feed;
mod mission;
mod msg;
mod project;
mod role;
mod session;
mod shared;
mod status;

use ask::*;
use chat::*;
pub use cli::Cli;
use cli::*;
use crew::*;
use daemon::*;
use feed::*;
use mission::*;
use msg::*;
use project::*;
use role::*;
use session::*;
use shared::*;
use status::*;

pub fn run(cli: Cli) -> i32 {
    if let Command::Remote { command } = &cli.command {
        return crate::remote::run(command, cli.json);
    }
    if let Command::Hook { command } = &cli.command {
        return runner_cli::hook::run(command);
    }
    if let Command::Help { topic } = &cli.command {
        help::print(topic.as_deref());
        return 0;
    }
    let context = env::resolve();
    if let BusContext::Partial { missing } = &context {
        eprintln!(
            "runner: missing required env var(s): {}",
            missing.join(", ")
        );
        return 2;
    }
    if let Some(code) = run_local(&cli, &context) {
        return code;
    }
    let session = env::session_id();
    if let Err(error) = validate_remote(&cli, &context).and_then(|()| match &cli.command {
        Command::Mission { command } => require_chat(command, session.as_deref()),
        _ => Ok(()),
    }) {
        eprintln!("{}", error.message);
        return error.code;
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("runner: failed to start runtime: {error}");
            return 1;
        }
    };
    let code = match runtime.block_on(run_remote(&cli, &context, session.as_deref())) {
        Ok(Some(response)) => {
            let response = postprocess_response(&cli, response);
            output::print(&response, cli.json, cli.quiet, output_view(&cli.command));
            0
        }
        Ok(None) => 0,
        Err(error) => {
            eprintln!("{}", error.message);
            error.code
        }
    };
    finish_runtime(runtime);
    code
}

fn finish_runtime(runtime: tokio::runtime::Runtime) {
    // A cancelled feed call may still be waiting on the longer blocking request deadline.
    runtime.shutdown_background();
}

fn run_local(cli: &Cli, context: &BusContext) -> Option<i32> {
    let BusContext::Mission(mission) = context else {
        return None;
    };
    match &cli.command {
        Command::Msg {
            command:
                MsgCommand::Post {
                    text,
                    to,
                    mission: target,
                    from,
                },
        } if same_mission(target.as_deref(), mission) => {
            if from.is_some() {
                eprintln!("runner msg post: --as is only valid outside the caller's mission");
                Some(2)
            } else {
                Some(bus::post(mission, text, to))
            }
        }
        Command::Msg {
            command:
                MsgCommand::Read {
                    all,
                    since,
                    from,
                    mission: target,
                },
        } if same_mission(target.as_deref(), mission) => {
            Some(bus::read(mission, *all, since.as_deref(), from.as_deref()))
        }
        Command::Signal {
            r#type,
            payload,
            mission: target,
            from,
        } if same_mission(target.as_deref(), mission) => {
            if from.is_some() {
                eprintln!("runner signal: --as is only valid outside the caller's mission");
                Some(2)
            } else {
                Some(signal::run(mission, r#type, payload.as_deref()))
            }
        }
        Command::Ask(args) if same_mission(args.mission.as_deref(), mission) => {
            if args.from.is_some() {
                eprintln!("runner ask: --as is only valid outside the caller's mission");
                return Some(2);
            }
            match ask_signal(args) {
                Ok((kind, payload)) => Some(signal::append(mission, kind, payload)),
                Err(error) => {
                    eprintln!("{}", error.message);
                    Some(error.code)
                }
            }
        }
        _ => None,
    }
}

fn same_mission(target: Option<&str>, mission: &MissionEnv) -> bool {
    target.is_none_or(|target| mission.mission_id.starts_with(target))
}

fn validate_remote(cli: &Cli, context: &BusContext) -> Result<(), CliError> {
    match &cli.command {
        Command::Mission {
            command: MissionCommand::AddRole { .. },
        } if matches!(context, BusContext::Mission(_)) => Err(CliError::usage(
            "runner mission add-role is for the person; ask the person to add a role",
        )),
        Command::Mission {
            command: MissionCommand::RemoveRole { .. },
        } if matches!(context, BusContext::Mission(_)) => Err(CliError::usage(
            "runner mission remove-role is for the person; ask the person to remove a role",
        )),
        Command::Role {
            command: RoleCommand::Create { fields, .. },
        } => {
            let runtime = fields
                .runtime
                .as_deref()
                .ok_or_else(|| CliError::usage("runner role create: --runtime is required"))?;
            runtime_command(runtime)?;
            parse_env(&fields.env)?;
            validate_permission(fields.permission.as_deref())?;
            Ok(())
        }
        Command::Role {
            command: RoleCommand::Update { fields, .. },
        } => {
            if let Some(runtime) = fields.runtime.as_deref() {
                runtime_command(runtime)?;
            }
            parse_env(&fields.env)?;
            validate_permission(fields.permission.as_deref())?;
            Ok(())
        }
        Command::Crew {
            command: CrewCommand::Add { runtime, .. },
        } => {
            if let Some(runtime) = runtime.as_deref() {
                runtime_command(runtime)?;
            }
            Ok(())
        }
        Command::Crew {
            command: CrewCommand::Set { runtime, .. },
        } => {
            if let Some(runtime) = runtime.as_deref().filter(|runtime| !runtime.is_empty()) {
                runtime_command(runtime)?;
            }
            Ok(())
        }
        Command::Msg {
            command: MsgCommand::Post { mission, .. },
        } if mission.is_none() && matches!(context, BusContext::OffBus) => Err(CliError::usage(
            "runner: --mission is required outside a mission",
        )),
        Command::Signal {
            r#type,
            payload,
            mission,
            ..
        } => {
            if r#type == "ask_lead" {
                return Err(CliError::usage(runner_core::model::ASK_LEAD_REMOVED_HINT));
            }
            if runner_core::model::KnownSignalType::from_name(r#type).is_none() {
                return Err(CliError::usage(format!(
                    "runner signal: unknown type {:?}",
                    r#type
                )));
            }
            signal::parse_payload(payload.as_deref()).map_err(CliError::usage)?;
            if mission.is_none() && matches!(context, BusContext::OffBus) {
                return Err(CliError::usage(
                    "runner: --mission is required outside a mission",
                ));
            }
            Ok(())
        }
        Command::Msg {
            command: MsgCommand::Read { .. },
        } => Err(CliError::usage(
            "runner msg read is available only inside a mission; use runner mission feed <mission>",
        )),
        Command::Ask(args) => {
            ask_signal(args)?;
            if args.mission.is_none() && matches!(context, BusContext::OffBus) {
                return Err(CliError::usage(
                    "runner ask: --mission is required outside a mission",
                ));
            }
            if args.from.is_none() {
                return Err(CliError::usage(
                    "runner ask: --as <handle> is required outside the caller's mission",
                ));
            }
            Ok(())
        }
        Command::Call { arguments, .. } => {
            parse_json_object(arguments.as_deref().unwrap_or("{}"))?;
            Ok(())
        }
        Command::Chat {
            command: ChatCommand::Start { role, runtime, .. },
        } => {
            if role.is_none() == runtime.is_none() {
                return Err(CliError::usage(
                    "runner chat start requires exactly one role or --runtime",
                ));
            }
            if let Some(runtime) = runtime.as_deref() {
                runtime_command(runtime)?;
            }
            Ok(())
        }
        Command::Mission {
            command:
                MissionCommand::Feed {
                    types,
                    follow,
                    limit,
                    ..
                },
        } => {
            parse_feed_types(types.as_deref())?;
            if *follow && *limit == Some(0) {
                return Err(CliError::usage(
                    "runner mission feed: --follow requires a positive --limit",
                ));
            }
            Ok(())
        }
        Command::Mission {
            command: MissionCommand::Move {
                project, unfile, ..
            },
        } if project.is_none() == !*unfile => Err(CliError::usage(
            "runner mission move requires exactly one of --project or --unfile",
        )),
        Command::Mission { command }
            if mission_arg(command).is_some_and(|target| {
                target.is_none() && matches!(context, BusContext::OffBus)
            }) =>
        {
            Err(CliError::usage(
                "runner: a mission reference is required outside a mission",
            ))
        }
        _ => Ok(()),
    }
}

async fn run_remote(
    cli: &Cli,
    context: &BusContext,
    session: Option<&str>,
) -> Result<Option<ToolResponse>, CliError> {
    if let Command::Daemon { command } = &cli.command {
        return daemon_command(command).map(Some);
    }
    let client = SocketClient::connect().await?;
    if matches!(cli.command, Command::Status) {
        return status_response(&client, context).map(Some);
    }
    if let Command::Mission {
        command:
            MissionCommand::Feed {
                mission,
                follow: true,
                since,
                limit,
                oldest_first,
                types,
                from,
                all,
            },
    } = &cli.command
    {
        let mission_id = resolve_mission_arg(&client, mission.as_deref(), context).await?;
        let filter = FeedFilter::new(types.as_deref(), from.as_deref(), *all)?;
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();
        follow_feed(
            &client,
            &mission_id,
            *since,
            *limit,
            *oldest_first,
            &filter,
            cli.json,
            cli.quiet,
            &mut stdout,
            &mut std::io::stderr(),
        )
        .await?;
        return Ok(None);
    }
    run_connected(&client, cli, context, session)
        .await
        .map(Some)
}

/// `session` is the caller's `RUNNER_SESSION_ID`, when it runs in an agent session.
async fn run_connected(
    client: &impl Caller,
    cli: &Cli,
    context: &BusContext,
    session: Option<&str>,
) -> Result<ToolResponse, CliError> {
    match &cli.command {
        Command::Remote { .. } | Command::Daemon { .. } | Command::Hook { .. } => unreachable!(),
        Command::Status => unreachable!(),
        Command::Project { command } => run_project(client, command).await,
        Command::Role { command } => run_role(client, command).await,
        Command::Crew { command } => run_crew(client, command).await,
        Command::Mission { command } => run_mission(client, command, context, session).await,
        Command::Chat { command } => run_chat(client, command).await,
        Command::Session { command } => run_session(client, command).await,
        Command::Msg { command } => run_msg(client, command, context).await,
        Command::Signal {
            r#type,
            payload,
            mission,
            from,
        } => {
            let mission_id = resolve_scoped_mission(client, mission.as_deref(), context).await?;
            let payload = signal::parse_payload(payload.as_deref()).map_err(CliError::usage)?;
            call(
                client,
                "mission_signal",
                mission_signal_args(&mission_id, r#type, payload, from.as_deref()),
            )
            .await
        }
        Command::Ask(args) => run_ask(client, args, context).await,
        Command::Call { tool, arguments } => {
            call(
                client,
                tool,
                parse_json_object(arguments.as_deref().unwrap_or("{}"))?,
            )
            .await
        }
        Command::Help { .. } => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingClient {
        calls: Mutex<Vec<(String, Value)>>,
        requests: Mutex<Vec<String>>,
    }

    #[derive(Default)]
    struct RequestRecorder(Mutex<Vec<String>>);
    impl runner_core::protocol::Transport for RequestRecorder {
        fn call(
            &self,
            request: runner_core::protocol::Request,
        ) -> Result<runner_core::protocol::Response, runner_core::protocol::ClientError> {
            let value = serde_json::to_value(request).unwrap();
            self.0
                .lock()
                .unwrap()
                .push(value.as_object().unwrap().keys().next().unwrap().clone());
            Err(runner_core::protocol::ClientError::msg("recorded"))
        }
        fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
            panic!("recorder does not subscribe")
        }
    }

    struct FailingMissionGetClient {
        code: i32,
    }

    struct SequenceClient {
        responses: Mutex<VecDeque<(String, Result<Value, CliError>)>>,
        calls: Mutex<Vec<(String, Value)>>,
    }

    impl SequenceClient {
        fn new(responses: Vec<(&str, Result<Value, CliError>)>) -> Self {
            Self {
                responses: Mutex::new(
                    responses
                        .into_iter()
                        .map(|(name, response)| (name.to_owned(), response))
                        .collect(),
                ),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl Caller for RecordingClient {
        async fn call(&self, name: &str, arguments: Value) -> Result<ToolResponse, CliError> {
            let transport = std::sync::Arc::new(RequestRecorder::default());
            let client = runner_core::protocol::DaemonClient::new(transport.clone());
            let result = runner_cli::client::call_daemon(&client, name, arguments.clone());
            let requests = transport.0.lock().unwrap();
            assert_eq!(
                requests.len(),
                1,
                "{name} did not reach a typed request: {result:?}"
            );
            self.requests
                .lock()
                .unwrap()
                .extend(requests.iter().cloned());
            self.calls
                .lock()
                .unwrap()
                .push((name.to_owned(), arguments));
            let value = match name {
                "project_list" => json!([{"id": "project-id", "name": "Runner"}]),
                "role_list" => json!([{"id": "role-id", "handle": "coder"}]),
                "crew_list" => json!([{"id": "crew-id", "name": "Peer"}]),
                "slot_list" => json!([
                    {"id": "slot-lead", "slot_handle": "lead"},
                    {"id": "slot-impl", "slot_handle": "impl"}
                ]),
                "mission_list" => json!([{"id": "01M00000000000000000000000"}]),
                "session_list" => json!([{"session_id": "01S00000000000000000000000"}]),
                "slot_create" => json!({"id": "slot-new"}),
                "mission_get" | "mission_status" | "crew_get" | "role_get" => {
                    json!({"id": "object-id"})
                }
                _ => json!({"id": "changed-id"}),
            };
            response(value)
        }
    }

    impl Caller for FailingMissionGetClient {
        async fn call(&self, name: &str, _arguments: Value) -> Result<ToolResponse, CliError> {
            match name {
                "mission_list" => response(json!([])),
                "mission_get" => Err(CliError {
                    code: self.code,
                    message: "mission does not exist".into(),
                }),
                _ => panic!("unexpected tool {name}"),
            }
        }
    }

    struct LifecycleSubscription;
    impl EventSubscription for LifecycleSubscription {
        fn recv(
            &mut self,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<ClientEvent, EventError>> + Send + '_>,
        > {
            Box::pin(async {
                tokio::task::yield_now().await;
                Ok(ClientEvent {
                    name: "mission/changed".into(),
                    payload: Value::Null,
                })
            })
        }
    }
    impl Caller for SequenceClient {
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            Box::new(LifecycleSubscription)
        }
        async fn call(&self, name: &str, arguments: Value) -> Result<ToolResponse, CliError> {
            self.calls
                .lock()
                .unwrap()
                .push((name.to_owned(), arguments));
            let (expected, response_value) = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| panic!("unexpected tool call {name}"));
            assert_eq!(name, expected);
            response_value.and_then(response)
        }
    }

    fn feed_event(id: &str, next_offset: u64, kind: &str, from: &str) -> Value {
        json!({
            "next_offset": next_offset,
            "event": {
                "id": id,
                "ts": format!("2026-09-18T10:20:{id}Z"),
                "kind": if kind == "message" { "message" } else { "signal" },
                "from": from,
                "to": null,
                "type": if kind == "message" { Value::Null } else { json!(kind) },
                "payload": {"text": id},
            }
        })
    }

    fn feed_snapshot(status: &str, sessions: &[(&str, &str)], offset: u64) -> Value {
        json!({
            "mission": {"status": status, "archived_at": null},
            "sessions": sessions.iter().enumerate().map(|(index, (handle, status))| json!({
                "id": format!("{:026}", index + 1), "handle": handle, "status": status,
            })).collect::<Vec<_>>(),
            "last_event_offset": offset,
        })
    }

    #[test]
    fn cancelled_requests_do_not_extend_cli_runtime_shutdown() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (started, ready) = std::sync::mpsc::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let (finished, done) = std::sync::mpsc::channel();
        runtime.spawn_blocking(move || {
            started.send(()).unwrap();
            let _ = blocked.recv_timeout(Duration::from_secs(2));
            let _ = finished.send(());
        });
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        let started = std::time::Instant::now();
        finish_runtime(runtime);
        let elapsed = started.elapsed();
        let _ = release.send(());
        done.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(elapsed < Duration::from_secs(1));
    }

    #[cfg(unix)]
    #[test]
    fn daemon_stop_waits_past_ten_seconds_for_the_daemon_lock() {
        let root = tempfile::tempdir().unwrap();
        let daemon = runner_core::daemon_process::lock_file(root.path()).unwrap();
        daemon.try_lock_exclusive().unwrap();
        let data = root.path().to_owned();
        let (done, result) = std::sync::mpsc::channel();
        let wait = std::thread::spawn(move || {
            done.send(wait_for_daemon_stop(&data, std::time::Instant::now()))
                .unwrap();
        });
        let held = result.recv_timeout(Duration::from_secs(11));
        drop(daemon);
        let released = result.recv_timeout(Duration::from_secs(2));
        wait.join().unwrap();
        assert!(matches!(
            held,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        released.unwrap().unwrap();
    }

    #[test]
    fn feed_request_timeout_has_a_fast_test_seam() {
        assert_eq!(DEFAULT_FEED_REQUEST_TIMEOUT, Duration::from_secs(30));
        assert_eq!(FEED_REQUEST_TIMEOUT, Duration::from_millis(50));
    }

    #[test]
    fn follow_rejects_zero_limit_without_changing_one_shot_feed() {
        let args = ["runner", "mission", "feed", "mission", "--limit", "0"];
        let one_shot = Cli::try_parse_from(args).unwrap();
        validate_remote(&one_shot, &BusContext::OffBus).unwrap();
        let follow = Cli::try_parse_from(args.into_iter().chain(["--follow"])).unwrap();
        let error = validate_remote(&follow, &BusContext::OffBus).unwrap_err();
        assert_eq!(error.code, 2);
        assert!(error.message.contains("positive --limit"));
    }

    #[test]
    fn role_change_flags_and_person_only_refusals() {
        let cli = Cli::try_parse_from([
            "runner",
            "mission",
            "add-role",
            "m",
            "coder",
            "--handle",
            "coder-2",
            "--task-file",
            "-",
        ])
        .unwrap();
        assert!(
            matches!(cli.command, Command::Mission { command: MissionCommand::AddRole { ref task_file, .. } } if task_file.as_deref() == Some(Path::new("-")))
        );
        assert!(Cli::try_parse_from([
            "runner",
            "mission",
            "add-role",
            "m",
            "coder",
            "--task",
            "work",
            "--task-file",
            "-"
        ])
        .is_err());
        let context = BusContext::Mission(MissionEnv {
            crew_id: Some("c".into()),
            mission_id: "m".into(),
            handle: "lead".into(),
            event_log: "events.ndjson".into(),
        });
        for verb in ["add-role", "remove-role"] {
            let cli = Cli::try_parse_from(["runner", "mission", verb, "m", "coder"]).unwrap();
            assert!(validate_remote(&cli, &context)
                .unwrap_err()
                .message
                .contains("is for the person"));
            assert!(
                Cli::try_parse_from(["runner", "mission", verb, "m", "coder", "--as", "lead"])
                    .is_err()
            );
        }
    }

    #[test]
    fn answer_note_flags_parse_and_conflict() {
        let answer = ["runner", "mission", "answer", "m", "q", "yes"];
        let cli = Cli::try_parse_from(answer.into_iter().chain(["--note-file", "-"])).unwrap();
        assert!(
            matches!(cli.command, Command::Mission { command: MissionCommand::Answer { ref note_file, .. } } if note_file.as_deref() == Some(Path::new("-")))
        );
        let error =
            Cli::try_parse_from(
                answer
                    .into_iter()
                    .chain(["--note", "text", "--note-file", "-"]),
            )
            .unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    async fn answer_with(flags: &[&str]) -> (Result<ToolResponse, CliError>, Vec<(String, Value)>) {
        let answer = ["runner", "mission", "answer", "01M", "q", "yes"];
        let cli = Cli::try_parse_from(answer.iter().chain(flags)).unwrap();
        let client = RecordingClient::default();
        let result = run_connected(&client, &cli, &BusContext::OffBus, None).await;
        (result, client.calls.into_inner().unwrap())
    }

    #[tokio::test]
    async fn answer_notes_are_trimmed_from_either_source() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("note.txt");
        std::fs::write(&file, "\n  Only on Friday\n  or Monday\n\n").unwrap();
        for (flag, value, note) in [
            ("--note", "  Only on Friday\n", "Only on Friday"),
            (
                "--note-file",
                file.to_str().unwrap(),
                "Only on Friday\n  or Monday",
            ),
        ] {
            let (result, calls) = answer_with(&[flag, value]).await;
            assert!(result.is_ok(), "{flag}");
            assert_eq!(
                calls.last().unwrap().1["payload"],
                json!({"question_id": "q", "choice": "yes", "note": note}),
                "{flag}"
            );
        }
    }

    #[tokio::test]
    async fn a_blank_answer_note_is_a_usage_error_before_any_request() {
        let temp = tempfile::tempdir().unwrap();
        let blank = temp.path().join("blank.txt");
        std::fs::write(&blank, " \n\t\n").unwrap();
        for (flag, value) in [
            ("--note", ""),
            ("--note", " \n "),
            ("--note-file", blank.to_str().unwrap()),
        ] {
            let (result, calls) = answer_with(&[flag, value]).await;
            let error = result.err().unwrap();
            assert_eq!(error.code, 2, "{flag} {value:?}");
            assert!(error.message.contains("note is blank"), "{}", error.message);
            assert!(calls.is_empty(), "{flag} {value:?}: {calls:?}");
        }
    }

    #[tokio::test]
    async fn follow_prints_each_event_once_across_empty_and_multi_event_drains() {
        let e1 = feed_event("01", 10, "message", "human");
        let e2 = feed_event("02", 20, "ask_lead", "coder");
        let e3 = feed_event("03", 30, "message", "reviewer");
        let e4 = feed_event("04", 40, "human_question", "human");
        let running = feed_snapshot("running", &[("coder", "running")], 10);
        let mut archived = feed_snapshot("completed", &[("coder", "stopped")], 40);
        archived["mission"]["archived_at"] = json!("2026-09-18T10:21:00Z");
        let client = SequenceClient::new(vec![
            ("mission_status", Ok(running.clone())),
            (
                "mission_feed",
                Ok(json!({"events": [e1.clone()], "next_offset": 10})),
            ),
            ("mission_status", Ok(running.clone())),
            (
                "mission_feed",
                Ok(json!({"events": [], "next_offset": null})),
            ),
            (
                "mission_status",
                Ok(feed_snapshot("running", &[("coder", "running")], 20)),
            ),
            (
                "mission_feed",
                Ok(json!({"events": [e1, e2], "next_offset": 20})),
            ),
            (
                "mission_status",
                Ok(feed_snapshot("running", &[("coder", "running")], 40)),
            ),
            (
                "mission_feed",
                Ok(json!({"events": [e3, e4], "next_offset": 40})),
            ),
            ("mission_status", Ok(archived)),
            (
                "mission_feed",
                Ok(json!({"events": [], "next_offset": null})),
            ),
        ]);
        let mut output = Vec::new();
        let mut diagnostics = Vec::new();
        follow_feed(
            &client,
            "mission",
            None,
            None,
            false,
            &FeedFilter::new(None, None, false).unwrap(),
            true,
            false,
            &mut output,
            &mut diagnostics,
        )
        .await
        .unwrap();
        let lines = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        let ids = lines
            .iter()
            .map(|line| line["id"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["01", "02", "03", "04"]);
        let resume_offsets = lines
            .iter()
            .map(|line| line["next_offset"].as_u64().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(resume_offsets, [10, 20, 30, 40]);
        assert!(String::from_utf8(diagnostics)
            .unwrap()
            .contains("mission archived; watch ended"));
        let calls = client.calls.into_inner().unwrap();
        let cursors = calls
            .iter()
            .filter(|(name, _)| name == "mission_feed")
            .skip(1)
            .map(|(_, args)| args["since_offset"].as_u64().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(cursors, [10, 10, 20, 40]);
    }

    #[tokio::test]
    async fn follow_drains_terminal_snapshot_pages_before_exiting() {
        let client = SequenceClient::new(vec![
            ("mission_status", Ok(feed_snapshot("completed", &[], 30))),
            (
                "mission_feed",
                Ok(
                    json!({"events": [feed_event("01", 10, "message", "coder")], "next_offset": 10}),
                ),
            ),
            (
                "mission_feed",
                Ok(
                    json!({"events": [feed_event("02", 20, "human_question", "coder")], "next_offset": 20}),
                ),
            ),
            (
                "mission_feed",
                Ok(
                    json!({"events": [feed_event("03", 30, "mission_stopped", "system")], "next_offset": 30}),
                ),
            ),
        ]);
        let mut output = Vec::new();
        follow_feed(
            &client,
            "mission",
            Some(0),
            Some(1),
            true,
            &FeedFilter::new(None, None, false).unwrap(),
            true,
            false,
            &mut output,
            &mut Vec::new(),
        )
        .await
        .unwrap();
        let lines = String::from_utf8(output).unwrap();
        assert_eq!(lines.lines().count(), 3);
        assert!(lines.contains("human_question"));
        assert!(lines.contains("mission_stopped"));
        let calls = client.calls.into_inner().unwrap();
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[3].1["since_offset"], 20);
    }

    #[tokio::test]
    async fn follow_reports_partial_crash_once_then_stops_when_all_sessions_exit() {
        let running = feed_snapshot(
            "running",
            &[("coder", "crashed"), ("reviewer", "running")],
            0,
        );
        let ended = feed_snapshot(
            "running",
            &[("coder", "crashed"), ("reviewer", "stopped")],
            0,
        );
        let client = SequenceClient::new(vec![
            ("mission_status", Ok(running.clone())),
            ("mission_feed", Ok(json!({"events": []}))),
            ("mission_status", Ok(running)),
            ("mission_feed", Ok(json!({"events": []}))),
            ("mission_status", Ok(ended)),
            ("mission_feed", Ok(json!({"events": []}))),
        ]);
        let mut diagnostics = Vec::new();
        let error = follow_feed(
            &client,
            "mission",
            None,
            None,
            false,
            &FeedFilter::new(Some("message"), Some("reviewer"), false).unwrap(),
            true,
            false,
            &mut Vec::new(),
            &mut diagnostics,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, 1);
        assert!(error.message.contains("crashed sessions"));
        let notices = String::from_utf8(diagnostics).unwrap();
        assert_eq!(notices.matches("session @coder crashed").count(), 1);
        assert!(notices.contains("session @reviewer stopped"));
    }

    #[tokio::test]
    async fn follow_exits_on_stop_completion_and_abort_without_archive() {
        for (status, sessions, reason, code) in [
            (
                "running",
                vec![("coder", "stopped")],
                "all mission sessions exited",
                0,
            ),
            ("completed", vec![], "mission completed", 0),
            ("aborted", vec![], "mission aborted", 1),
        ] {
            let client = SequenceClient::new(vec![
                ("mission_status", Ok(feed_snapshot(status, &sessions, 0))),
                ("mission_feed", Ok(json!({"events": []}))),
            ]);
            let mut diagnostics = Vec::new();
            let result = follow_feed(
                &client,
                "mission",
                None,
                None,
                false,
                &FeedFilter::new(None, None, false).unwrap(),
                true,
                false,
                &mut Vec::new(),
                &mut diagnostics,
            )
            .await;
            if code == 0 {
                result.unwrap();
                assert!(String::from_utf8(diagnostics).unwrap().contains(reason));
            } else {
                let error = result.unwrap_err();
                assert_eq!(error.code, code);
                assert!(error.message.contains(reason));
            }
        }
        assert_eq!(feed_end_reason(&feed_snapshot("running", &[], 0)), None);
        assert_eq!(
            feed_end_reason(&feed_snapshot("running", &[("coder", "running")], 0)),
            None
        );
    }

    #[tokio::test]
    async fn follow_reports_app_disconnect_and_missing_mission_with_recovery_cursor() {
        for (tool, code, message) in [
            ("mission_status", 3, runner_cli::client::NOT_RUNNING_MESSAGE),
            ("mission_status", 1, "mission does not exist"),
            ("mission_feed", 3, runner_cli::client::NOT_RUNNING_MESSAGE),
            ("mission_feed", 1, "mission does not exist"),
        ] {
            let mut responses = vec![
                ("mission_status", Ok(feed_snapshot("running", &[], 10))),
                (
                    "mission_feed",
                    Ok(
                        json!({"events": [feed_event("01", 10, "message", "coder")], "next_offset": 10}),
                    ),
                ),
            ];
            if tool == "mission_feed" {
                responses.push(("mission_status", Ok(feed_snapshot("running", &[], 10))));
            }
            responses.push((
                tool,
                Err(CliError {
                    code,
                    message: message.into(),
                }),
            ));
            let client = SequenceClient::new(responses);
            let error = follow_feed(
                &client,
                "mission",
                None,
                None,
                false,
                &FeedFilter::new(None, None, false).unwrap(),
                false,
                false,
                &mut Vec::new(),
                &mut Vec::new(),
            )
            .await
            .unwrap_err();
            assert_eq!(error.code, code);
            assert!(error.message.contains(message));
            assert!(error.message.contains("watch ended"));
            assert!(error.message.contains("Use the same Runner executable"));
            assert!(error
                .message
                .contains("mission feed mission --since 10 --oldest-first --follow --json"));
        }
    }

    #[tokio::test]
    async fn follow_reports_an_unresponsive_app() {
        struct UnresponsiveClient;
        impl Caller for UnresponsiveClient {
            async fn call(&self, _name: &str, _arguments: Value) -> Result<ToolResponse, CliError> {
                std::future::pending().await
            }
        }
        let error = follow_feed(
            &UnresponsiveClient,
            "mission",
            None,
            None,
            false,
            &FeedFilter::new(None, None, false).unwrap(),
            true,
            false,
            &mut Vec::new(),
            &mut Vec::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, 1);
        assert!(error.message.contains("timed out"));
        assert!(error.message.contains("Runner may still be running"));
    }

    #[tokio::test]
    async fn follow_reports_a_broken_cursor_instead_of_spinning() {
        let client = SequenceClient::new(vec![
            ("mission_status", Ok(feed_snapshot("completed", &[], 10))),
            (
                "mission_feed",
                Ok(json!({"events": [], "next_offset": null})),
            ),
        ]);
        let error = follow_feed(
            &client,
            "mission",
            None,
            None,
            true,
            &FeedFilter::new(None, None, false).unwrap(),
            true,
            false,
            &mut Vec::new(),
            &mut Vec::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, 1);
        assert!(error.message.contains("cursor did not advance"));
    }

    #[test]
    fn default_feed_filter_preserves_real_failed_status_payloads() {
        for kind in ["session_status", "runner_status"] {
            for state in ["busy", "idle"] {
                for (lifecycle, outcome, visible) in [
                    ("running", "failed", true),
                    ("error", "completed", true),
                    ("running", "completed", false),
                    ("running", "interrupted", false),
                ] {
                    let mut event = feed_event("01", 10, kind, "coder");
                    event["event"]["payload"] = json!({
                        "state": state,
                        "source": "hook",
                        "status": {
                            "lifecycle": lifecycle,
                            "observation": {
                                "activity": if state == "busy" { "working" } else { "idle" },
                                "source": "hook",
                                "outcome": outcome,
                                "interactions": [],
                                "detail": null,
                            },
                            "exit_code": null,
                            "error_since": null,
                            "failed_since": null,
                            "unread_since": null,
                        },
                    });
                    assert_eq!(
                        FeedFilter::new(None, None, false).unwrap().matches(&event),
                        visible
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn follow_allows_a_slow_response_before_the_request_deadline() {
        struct SlowClient;
        impl Caller for SlowClient {
            async fn call(&self, name: &str, _arguments: Value) -> Result<ToolResponse, CliError> {
                tokio::time::sleep(Duration::from_millis(2)).await;
                match name {
                    "mission_status" => response(feed_snapshot("completed", &[], 0)),
                    "mission_feed" => response(json!({"events": []})),
                    _ => panic!("unexpected tool {name}"),
                }
            }
        }
        follow_feed(
            &SlowClient,
            "mission",
            None,
            None,
            false,
            &FeedFilter::new(None, None, false).unwrap(),
            true,
            false,
            &mut Vec::new(),
            &mut Vec::new(),
        )
        .await
        .unwrap();
    }

    struct BufferedSubscription(VecDeque<Result<ClientEvent, EventError>>);
    impl EventSubscription for BufferedSubscription {
        fn recv(
            &mut self,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<ClientEvent, EventError>> + Send + '_>,
        > {
            Box::pin(async { self.0.pop_front().expect("unexpected subscription receive") })
        }
    }
    struct PushClient {
        sequence: SequenceClient,
        updates: Mutex<Option<VecDeque<Result<ClientEvent, EventError>>>>,
    }
    impl PushClient {
        fn new(
            responses: Vec<(&str, Result<Value, CliError>)>,
            updates: Vec<Result<ClientEvent, EventError>>,
        ) -> Self {
            Self {
                sequence: SequenceClient::new(responses),
                updates: Mutex::new(Some(updates.into())),
            }
        }
    }
    impl Caller for PushClient {
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            Box::new(BufferedSubscription(
                self.updates.lock().unwrap().take().unwrap(),
            ))
        }
        async fn call(&self, name: &str, arguments: Value) -> Result<ToolResponse, CliError> {
            assert!(
                self.updates.lock().unwrap().is_none(),
                "subscribe before reading the initial snapshot"
            );
            self.sequence.call(name, arguments).await
        }
    }
    fn pushed(entry: Value) -> Result<ClientEvent, EventError> {
        Ok(ClientEvent {
            name: "event/appended".into(),
            payload: json!({"mission_id": "mission", "next_offset": entry["next_offset"], "event": entry["event"]}),
        })
    }

    #[tokio::test]
    async fn follow_consumes_pushes_without_requests_and_recovers_lag_without_duplicates() {
        let events = (1..=7)
            .map(|i| feed_event(&format!("{i:02}"), i * 10, "message", "coder"))
            .collect::<Vec<_>>();
        let mut other_mission = pushed(events[6].clone()).unwrap();
        other_mission.payload["mission_id"] = json!("other");
        let client = PushClient::new(
            vec![
                ("mission_status", Ok(feed_snapshot("running", &[], 20))),
                (
                    "mission_feed",
                    Ok(json!({"events": [events[1]], "next_offset": 20})),
                ),
                ("mission_status", Ok(feed_snapshot("running", &[], 50))),
                (
                    "mission_feed",
                    Ok(json!({"events": [events[3]], "next_offset": 40})),
                ),
                (
                    "mission_feed",
                    Ok(json!({"events": [events[4]], "next_offset": 50})),
                ),
                ("mission_status", Ok(feed_snapshot("completed", &[], 70))),
                (
                    "mission_feed",
                    Ok(json!({"events": [events[6]], "next_offset": 70})),
                ),
            ],
            vec![
                pushed(events[0].clone()),
                pushed(events[1].clone()),
                pushed(events[2].clone()),
                pushed(events[2].clone()),
                Ok(other_mission),
                Err(EventError::Lagged(2)),
                pushed(events[3].clone()),
                pushed(events[4].clone()),
                pushed(events[5].clone()),
                Ok(ClientEvent {
                    name: "mission/changed".into(),
                    payload: json!({"mission_id": "mission"}),
                }),
            ],
        );
        let mut output = Vec::new();
        follow_feed(
            &client,
            "mission",
            None,
            Some(1),
            false,
            &FeedFilter::new(None, None, false).unwrap(),
            true,
            false,
            &mut output,
            &mut Vec::new(),
        )
        .await
        .unwrap();
        let lines = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            lines
                .iter()
                .map(|line| line["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["02", "03", "04", "05", "06", "07"]
        );
        assert_eq!(
            lines
                .iter()
                .map(|line| line["next_offset"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [20, 30, 40, 50, 60, 70]
        );
        let calls = client.sequence.calls.lock().unwrap();
        assert_eq!(calls.len(), 7);
        let feeds = calls
            .iter()
            .filter(|(name, _)| name == "mission_feed")
            .map(|(_, args)| args.clone())
            .collect::<Vec<_>>();
        assert_eq!(feeds[0]["order"], "newest_first");
        assert!(feeds[0]["since_offset"].is_null());
        assert_eq!(
            feeds
                .iter()
                .skip(1)
                .map(|args| args["since_offset"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [30, 40, 60]
        );
        assert!(feeds
            .iter()
            .skip(1)
            .all(|args| args["order"] == "oldest_first"));
    }

    #[tokio::test]
    async fn follow_disconnect_mid_subscription_preserves_output_and_recovery_cursor() {
        for disconnect in [
            Err(EventError::Closed),
            Ok(ClientEvent {
                name: "daemon/disconnected".into(),
                payload: Value::Null,
            }),
        ] {
            let client = PushClient::new(
                vec![
                    ("mission_status", Ok(feed_snapshot("running", &[], 10))),
                    (
                        "mission_feed",
                        Ok(
                            json!({"events": [feed_event("01", 10, "message", "coder")], "next_offset": 10}),
                        ),
                    ),
                ],
                vec![
                    pushed(feed_event("02", 20, "message", "reviewer")),
                    pushed(feed_event("03", 30, "inbox_read", "coder")),
                    disconnect,
                ],
            );
            let mut output = Vec::new();
            let error = follow_feed(
                &client,
                "mission",
                Some(0),
                None,
                true,
                &FeedFilter::new(None, None, false).unwrap(),
                true,
                false,
                &mut output,
                &mut Vec::new(),
            )
            .await
            .unwrap_err();
            assert_eq!(error.code, 3);
            assert!(error
                .message
                .contains(runner_cli::client::NOT_RUNNING_MESSAGE));
            assert!(error.message.contains("watch ended"));
            assert!(error
                .message
                .contains("--since 30 --oldest-first --follow --json"));
            assert_eq!(String::from_utf8(output).unwrap().lines().count(), 2);
            assert_eq!(client.sequence.calls.lock().unwrap().len(), 2);
        }
    }

    #[test]
    fn feed_filters_types_senders_and_default_noise() {
        let message = feed_event("01", 10, "message", "human");
        let status = feed_event("02", 20, "session_status", "coder");
        let inbox = feed_event("03", 30, "inbox_read", "coder");
        let ask = feed_event("04", 40, "ask_lead", "reviewer");
        let legacy = feed_event("05", 50, "runner_status", "coder");
        let response = response(json!({
            "events": [message, status, inbox, ask, legacy],
            "next_offset": 50,
            "skipped": []
        }))
        .unwrap();

        let default = filter_feed_response(&response, &FeedFilter::new(None, None, false).unwrap());
        assert_eq!(default.value["events"].as_array().unwrap().len(), 2);
        let all = filter_feed_response(&response, &FeedFilter::new(None, None, true).unwrap());
        assert_eq!(all.value["events"].as_array().unwrap().len(), 5);
        let statuses = filter_feed_response(
            &response,
            &FeedFilter::new(Some("session_status,inbox_read"), None, false).unwrap(),
        );
        assert_eq!(statuses.value["events"].as_array().unwrap().len(), 2);
        let reviewer = filter_feed_response(
            &response,
            &FeedFilter::new(None, Some("reviewer"), false).unwrap(),
        );
        assert_eq!(reviewer.value["events"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn plain_json_feed_without_filters_stays_verbatim() {
        let cli = Cli::try_parse_from(["runner", "mission", "feed", "01M", "--json"]).unwrap();
        let response = ToolResponse {
            value: json!({"events": [], "next_offset": null}),
            raw_json: "{ \"events\" : [], \"next_offset\" : null }".into(),
        };
        assert_eq!(
            postprocess_response(&cli, response).raw_json,
            "{ \"events\" : [], \"next_offset\" : null }"
        );
    }

    #[test]
    fn runner_status_classifies_managed_foreign_and_missing_skill_roots() {
        let home = tempfile::tempdir().unwrap();
        let managed = home.path().join(".claude/skills/runner-dev");
        std::fs::create_dir_all(&managed).unwrap();
        std::fs::write(managed.join(runner_core::RUNNER_SKILL_MARKER), "managed").unwrap();
        let foreign = home.path().join(".agents/skills/runner-dev");
        std::fs::create_dir_all(&foreign).unwrap();

        let statuses = skill_statuses(home.path(), true);

        assert_eq!(statuses[0]["state"], "managed");
        assert_eq!(statuses[1]["state"], "foreign");
        assert_eq!(statuses[2]["state"], "missing");
    }

    #[test]
    fn updates_send_empty_strings_for_backend_clears() {
        let fields = RoleFields {
            runtime: Some("trae".into()),
            model: Some(String::new()),
            effort: Some(String::new()),
            prompt: Some(String::new()),
            cwd: Some(String::new()),
            ..Default::default()
        };
        assert_eq!(
            runtime_command(fields.runtime.as_deref().unwrap()).unwrap(),
            "traecli"
        );
        assert_eq!(runtime_command("antigravity").unwrap(), "agy");
        assert_eq!(runtime_command("cursor").unwrap(), "cursor-agent");
        assert!(runtime_command("agent").is_err());
        let update = role_fields(&fields, true).unwrap();
        assert_eq!(update["model"], json!(""));
        assert_eq!(update["effort"], json!(""));
        assert_eq!(update["system_prompt"], json!(""));
        assert_eq!(update["working_dir"], json!(""));
    }

    #[tokio::test]
    async fn speed_flags_send_structured_values_and_preserve_omissions() {
        for (flag, expected) in [
            (None, None),
            (Some("inherit"), Some(Value::Null)),
            (Some("standard"), Some(json!("standard"))),
            (Some("fast"), Some(json!("fast"))),
        ] {
            for (command, key, tool) in [
                (
                    vec!["role", "create", "coder", "--runtime", "codex"],
                    "codex_speed",
                    "role_create",
                ),
                (
                    vec!["role", "update", "coder"],
                    "codex_speed",
                    "role_update",
                ),
                (
                    vec!["crew", "add", "Peer", "coder"],
                    "codex_speed_override",
                    "slot_update",
                ),
                (
                    vec!["crew", "set", "Peer", "impl"],
                    "codex_speed_override",
                    "slot_update",
                ),
                (
                    vec!["chat", "start", "coder"],
                    "speed",
                    "session_start_direct",
                ),
                (
                    vec!["chat", "start", "--runtime", "codex"],
                    "speed",
                    "session_start_direct",
                ),
            ] {
                let mut args = vec!["runner"];
                args.extend(command.iter().copied());
                if let Some(flag) = flag {
                    args.extend(["--speed", flag]);
                }
                let cli = Cli::try_parse_from(args).unwrap();
                let client = RecordingClient::default();
                run_connected(&client, &cli, &BusContext::OffBus, None)
                    .await
                    .unwrap();
                let calls = client.calls.lock().unwrap();
                if command[1] == "add" && flag.is_none() {
                    assert!(calls.iter().all(|(name, _)| name != "slot_update"));
                    continue;
                }
                let (_, request) = calls.iter().find(|(name, _)| name == tool).unwrap();
                let fields = if tool == "role_update" || tool == "slot_update" {
                    &request["input"]
                } else {
                    request
                };
                assert_eq!(fields.get(key), expected.as_ref(), "{tool}: {flag:?}");
            }
        }
    }

    #[test]
    fn speed_flag_help_lists_only_supported_choices() {
        for command in [
            &["role", "create"][..],
            &["role", "update"],
            &["crew", "add"],
            &["crew", "set"],
            &["chat", "start"],
        ] {
            let mut args = vec!["runner"];
            args.extend_from_slice(command);
            args.push("--help");
            let help = Cli::try_parse_from(args).unwrap_err().to_string();
            assert!(help.contains("--speed <SPEED>"), "{help}");
            assert!(help.contains("inherit, standard, fast"), "{help}");
        }
        assert!(Cli::try_parse_from([
            "runner",
            "chat",
            "start",
            "--runtime",
            "codex",
            "--speed",
            "turbo"
        ])
        .is_err());
    }

    #[test]
    fn crew_commands_reject_removed_flags_before_connecting() {
        for action in ["create", "update"] {
            for flag in ["--purpose", "--goal"] {
                let error = Cli::try_parse_from(["runner", "crew", action, "Peer", flag, "old"])
                    .unwrap_err();
                assert_eq!(error.exit_code(), 2, "{action} {flag}");
                assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
            }
        }
        assert!(Cli::try_parse_from([
            "runner", "mission", "start", "--crew", "Peer", "--goal", "Ship", "--title", "Ship"
        ])
        .is_ok());
    }

    #[test]
    fn mission_start_requires_an_explicit_title_with_or_without_a_crew_or_goal() {
        for options in [
            vec![],
            vec!["--goal", "Ship"],
            vec!["--crew", "Peer"],
            vec!["--crew", "Peer", "--goal", "Ship"],
        ] {
            let mut args = vec!["runner", "mission", "start"];
            args.extend(options);
            let error = Cli::try_parse_from(&args).unwrap_err();
            assert_eq!(
                error.kind(),
                clap::error::ErrorKind::MissingRequiredArgument
            );
            assert!(error.to_string().contains("--title <TITLE>"));
            args.extend(["--title", "Explicit title"]);
            assert!(Cli::try_parse_from(args).is_ok());
        }
    }

    #[test]
    fn ask_shorthand_and_human_alias_ask_the_person() {
        let lead = AskArgs {
            question: Some("Ship?".into()),
            human: None,
            choices: None,
            mission: None,
            from: None,
        };
        assert_eq!(
            ask_signal(&lead).unwrap(),
            ("ask_human", json!({"prompt":"Ship?", "choices":[]}))
        );
        let human = AskArgs {
            question: None,
            human: Some("Ship?".into()),
            choices: Some("yes, no".into()),
            mission: None,
            from: None,
        };
        assert_eq!(
            ask_signal(&human).unwrap(),
            (
                "ask_human",
                json!({"prompt": "Ship?", "choices": ["yes", "no"]})
            )
        );
    }

    #[test]
    fn outside_identity_is_omitted_for_the_person_and_forwarded_for_a_handle() {
        assert_eq!(
            mission_post_args("mission", "hello", &["lead".into()], None),
            json!({"mission_id": "mission", "text": "hello", "to": "lead"})
        );
        assert_eq!(
            mission_post_args("mission", "hello", &[], Some("coder"))["from"],
            "coder"
        );
        assert_eq!(
            mission_signal_args("mission", "ask_lead", json!({}), Some("coder")),
            json!({
                "mission_id": "mission",
                "from": "coder",
                "signal_type": "ask_lead",
                "payload": {},
            })
        );
        assert!(mission_signal_args(
            "mission",
            "human_response",
            json!({"question_id": "q", "choice": "yes"}),
            None,
        )
        .get("from")
        .is_none());
    }

    #[test]
    fn call_requires_object_arguments() {
        assert!(parse_json_object("[]").is_err());
        assert_eq!(
            parse_json_object(r#"{"id":"x"}"#).unwrap(),
            json!({"id": "x"})
        );
    }

    #[test]
    fn client_failures_map_to_documented_exit_codes() {
        assert_eq!(CliError::from(ClientError::Refused("no".into())).code, 1);
        assert_eq!(CliError::from(ClientError::NotRunning).code, 3);
        let blocked = CliError::from(ClientError::Blocked);
        assert_eq!(blocked.code, 5);
        assert_eq!(blocked.message, runner_cli::client::BLOCKED_MESSAGE);
    }

    #[test]
    fn references_resolve_handles_names_and_prefixes() {
        let roles = vec![json!({"id": "role-id", "handle": "coder"})];
        assert_eq!(resolve_role_rows(&roles, "@coder").unwrap().id, "role-id");

        let crews = vec![
            json!({"id": "crew-a", "name": "Peer"}),
            json!({"id": "crew-b", "name": "Solo"}),
        ];
        assert_eq!(
            resolve_named_rows(&crews, "crew", "Peer").unwrap().id,
            "crew-a"
        );
        let ambiguous = vec![
            json!({"id": "crew-a", "name": "Peer"}),
            json!({"id": "crew-b", "name": "Peer"}),
        ];
        let error = resolve_named_rows(&ambiguous, "crew", "Peer").unwrap_err();
        assert_eq!(error.code, 2);
        assert!(error.message.contains("crew-a") && error.message.contains("crew-b"));

        let missions = vec![
            json!({"id": "01MISSIONAAAA"}),
            json!({"id": "01MISSIONBBBB"}),
        ];
        assert_eq!(
            resolve_prefix_rows(&missions, "mission", "01MISSIONA").unwrap(),
            "01MISSIONAAAA"
        );
        let error = resolve_prefix_rows(&missions, "mission", "01MISSION").unwrap_err();
        assert_eq!(error.code, 2);
        assert!(error.message.contains("01MISSIONAAAA"));
        assert!(error.message.contains("01MISSIONBBBB"));
    }

    #[test]
    fn own_mission_prefix_stays_on_the_local_event_log_path() {
        let mission = MissionEnv {
            crew_id: Some("crew".into()),
            mission_id: "01M00000000000000000000000".into(),
            handle: "coder".into(),
            event_log: PathBuf::from("events.ndjson"),
        };
        assert!(same_mission(None, &mission));
        assert!(same_mission(Some("01M"), &mission));
        assert!(!same_mission(Some("01A"), &mission));
    }

    #[test]
    fn paths_are_made_absolute_and_normalized_without_filesystem_access() {
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(
            absolute_path(Path::new(".")).unwrap(),
            cwd.to_string_lossy()
        );
        assert_eq!(
            absolute_path(Path::new("./x/../y")).unwrap(),
            cwd.join("y").to_string_lossy()
        );
    }

    #[test]
    fn command_status_path_filter_drops_only_runner_app_data_entries() {
        assert_eq!(
            path_without_runner_entries(
                "/app/data/missions/id/shims/coder/bin:/app/data/bin:/usr/local/bin",
                Path::new("/app/data"),
                runner_core::command_install::PathStyle::Unix,
            ),
            "/usr/local/bin"
        );
        assert_eq!(
            path_without_runner_entries(
                r"C:\Runner\Data\bin;C:\Runner\Data\missions\id\shims\coder\bin;C:\Tools",
                Path::new(r"c:\runner\data"),
                runner_core::command_install::PathStyle::Windows,
            ),
            r"C:\Tools"
        );
    }

    #[cfg(unix)]
    #[test]
    fn command_status_ignores_runner_sidecar_and_shim_before_the_owned_link() {
        use std::os::unix::fs::{symlink, PermissionsExt as _};

        let temp = tempfile::tempdir().unwrap();
        let app_data = temp.path().join("app-data");
        let sidecar = app_data.join("bin/runner");
        let shim_dir = app_data.join("missions/mission/shims/coder/bin");
        let shim = shim_dir.join("runner");
        let home = temp.path().join("home");
        let local_bin = home.join(".local/bin");
        let link = local_bin.join("runner");
        for executable in [&sidecar, &shim] {
            std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
            std::fs::write(executable, "runner").unwrap();
            std::fs::set_permissions(executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::create_dir_all(&local_bin).unwrap();
        symlink(&sidecar, &link).unwrap();
        let path = format!(
            "{}:{}:{}",
            shim_dir.display(),
            sidecar.parent().unwrap().display(),
            local_bin.display()
        );

        let installed = command_install_status_with_path(&home, &sidecar, &app_data, false, &path);
        assert_eq!(installed["state"], "installed");

        let foreign_dir = temp.path().join("foreign-bin");
        let foreign = foreign_dir.join("runner");
        std::fs::create_dir_all(&foreign_dir).unwrap();
        std::fs::write(&foreign, "foreign").unwrap();
        std::fs::set_permissions(&foreign, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("{}:{path}", foreign_dir.display());
        let shadowed = command_install_status_with_path(&home, &sidecar, &app_data, false, &path);
        assert_eq!(shadowed["state"], "shadowed");
        assert_eq!(shadowed["shadowed_by"], foreign.display().to_string());
    }

    #[tokio::test]
    async fn nonexistent_full_mission_id_is_an_unresolvable_reference() {
        let client = FailingMissionGetClient { code: 1 };
        let error = resolve_mission(&client, "01MISSING00000000000000000")
            .await
            .unwrap_err();
        assert_eq!(error.code, 2);
        assert!(error.message.contains("mission not found"));
    }

    #[tokio::test]
    async fn full_mission_validation_preserves_not_running_exit_three() {
        let client = FailingMissionGetClient { code: 3 };
        let error = resolve_mission(&client, "01MISSING00000000000000000")
            .await
            .unwrap_err();
        assert_eq!(error.code, 3);
    }

    #[tokio::test]
    async fn project_delete_adds_the_resolved_id_for_quiet_output_only() {
        let cli = Cli::try_parse_from(["runner", "project", "delete", "Runner"]).unwrap();
        let response = run_connected(&RecordingClient::default(), &cli, &BusContext::OffBus, None)
            .await
            .unwrap();
        assert_eq!(response.value["id"], "project-id");
        assert_eq!(response.raw_json, r#"{"id":"changed-id"}"#);
    }

    #[tokio::test]
    async fn mission_lifecycle_uses_real_shapes_and_adds_the_crew_name() {
        const MISSION_ID: &str = "01M00000000000000000000000";
        let cases = [
            (
                "mission_stop",
                json!({"id": MISSION_ID, "title": "CLI", "status": "running", "crew_id": "crew-id"}),
                None,
                "stopped",
            ),
            (
                "mission_resume",
                json!({"mission_id": MISSION_ID, "resumed_session_ids": ["session-id"], "sessions": []}),
                Some(
                    json!({"id": MISSION_ID, "title": "CLI", "status": "running", "crew_id": "crew-id"}),
                ),
                "running",
            ),
            (
                "mission_archive",
                json!({"id": MISSION_ID, "title": "CLI", "status": "completed", "crew_id": "crew-id"}),
                None,
                "completed",
            ),
        ];
        for (tool, tool_value, fetched_mission, expected_status) in cases {
            let mut calls = vec![
                (
                    "mission_list",
                    Ok(json!([{"id": MISSION_ID, "title": "CLI"}])),
                ),
                (tool, Ok(tool_value.clone())),
            ];
            if let Some(mission) = fetched_mission {
                calls.push(("mission_get", Ok(mission)));
            }
            calls.push(("crew_list", Ok(json!([{"id": "crew-id", "name": "Peer"}]))));
            let client = SequenceClient::new(calls);
            let response = mission_lifecycle(&client, tool, Some("01M"), &BusContext::OffBus)
                .await
                .unwrap();
            assert_eq!(response.value["crew_name"], "Peer");
            assert_eq!(response.value["status"], expected_status);
            assert_eq!(
                response.raw_json,
                serde_json::to_string(&tool_value).unwrap()
            );
        }
    }

    #[tokio::test]
    async fn every_remote_command_builds_the_expected_tool_calls_without_a_socket() {
        const MISSION_ID: &str = "01M00000000000000000000000";
        const ARCHIVED_ID: &str = "01A00000000000000000000000";
        const SESSION_ID: &str = "01S00000000000000000000000";
        const MISSION_SESSION_ID: &str = "01X00000000000000000000000";
        let cwd = absolute_path(Path::new(".")).unwrap();
        let nested_cwd = absolute_path(Path::new("./x/../y")).unwrap();
        let cases = vec![
            (vec!["project", "list"], vec!["project_list"], json!({})),
            (
                vec!["project", "show", "Runner"],
                vec!["project_list", "project_get"],
                json!({"id": "project-id"}),
            ),
            (
                vec!["project", "create", "New"],
                vec!["project_create"],
                json!({"name": "New", "cwd": cwd}),
            ),
            (
                vec!["project", "rename", "Runner", "New"],
                vec!["project_list", "project_rename"],
                json!({"id": "project-id", "name": "New"}),
            ),
            (
                vec!["project", "delete", "Runner", "--force"],
                vec!["project_list", "project_delete"],
                json!({"id": "project-id", "force": true}),
            ),
            (vec!["role", "list"], vec!["role_list"], json!({})),
            (
                vec!["role", "show", "coder"],
                vec!["role_list", "role_get_by_handle"],
                json!({"handle": "coder"}),
            ),
            (
                vec!["role", "create", "coder", "--runtime", "codex"],
                vec!["role_create"],
                json!({
                    "handle": "coder",
                    "display_name": "coder",
                    "runtime": "codex",
                    "command": "codex",
                    "args": [],
                    "env": {},
                }),
            ),
            (
                vec!["role", "update", "coder", "--model", "gpt"],
                vec!["role_list", "role_update"],
                json!({"id": "role-id", "input": {"model": "gpt"}}),
            ),
            (
                vec!["role", "update", "coder", "--model", ""],
                vec!["role_list", "role_update"],
                json!({"id": "role-id", "input": {"model": ""}}),
            ),
            (
                vec!["role", "delete", "coder"],
                vec!["role_list", "role_delete"],
                json!({"id": "role-id"}),
            ),
            (vec!["crew", "list"], vec!["crew_list"], json!({})),
            (
                vec!["crew", "show", "Peer"],
                vec!["crew_list", "crew_get", "slot_list"],
                json!({"crew_id": "crew-id"}),
            ),
            (
                vec!["crew", "create", "Peer"],
                vec!["crew_create"],
                json!({"name": "Peer"}),
            ),
            (
                vec!["crew", "update", "Peer", "--name", "Renamed"],
                vec!["crew_list", "crew_update"],
                json!({"id": "crew-id", "input": {"name": "Renamed"}}),
            ),
            (
                vec!["crew", "delete", "Peer"],
                vec!["crew_list", "crew_delete"],
                json!({"id": "crew-id"}),
            ),
            (
                vec![
                    "crew", "add", "Peer", "coder", "--as", "impl", "--effort", "high",
                ],
                vec!["crew_list", "role_list", "slot_create", "slot_update"],
                json!({"slot_id": "slot-new", "input": {"effort_override": "high"}}),
            ),
            (
                vec!["crew", "set", "Peer", "impl", "--model", "gpt"],
                vec!["crew_list", "slot_list", "slot_update"],
                json!({"slot_id": "slot-impl", "input": {"model_override": "gpt"}}),
            ),
            (
                vec!["crew", "remove", "Peer", "impl"],
                vec!["crew_list", "slot_list", "slot_delete"],
                json!({"slot_id": "slot-impl"}),
            ),
            (
                vec!["crew", "order", "Peer", "lead", "impl"],
                vec!["crew_list", "slot_list", "slot_reorder"],
                json!({
                    "crew_id": "crew-id",
                    "ordered_slot_ids": ["slot-lead", "slot-impl"]
                }),
            ),
            (
                vec![
                    "mission", "start", "--crew", "Peer", "--title", "CLI", "--cwd", "./x/../y",
                ],
                vec!["crew_list", "mission_start"],
                json!({
                    "crew_id": "crew-id",
                    "project_id": null,
                    "title": "CLI",
                    "goal_override": null,
                    "cwd": nested_cwd,
                    "started_by_session_id": null,
                    "no_report": false,
                }),
            ),
            (
                vec!["mission", "list", "--crew", "Peer"],
                vec!["crew_list", "mission_list_summary"],
                json!({"crew_id": "crew-id"}),
            ),
            (
                vec!["mission", "show", "01M"],
                vec!["mission_list", "mission_status"],
                json!({"id": MISSION_ID}),
            ),
            (
                vec!["mission", "show", ARCHIVED_ID],
                vec!["mission_list", "mission_get", "mission_status"],
                json!({"id": ARCHIVED_ID}),
            ),
            (
                vec![
                    "mission",
                    "start",
                    "--crew",
                    "Peer",
                    "--goal",
                    "Ship it",
                    "--title",
                    "CLI",
                    "--project",
                    "Runner",
                ],
                vec!["crew_list", "project_list", "mission_start"],
                json!({
                    "crew_id": "crew-id",
                    "project_id": "project-id",
                    "title": "CLI",
                    "goal_override": "Ship it",
                    "cwd": null,
                    "started_by_session_id": null,
                    "no_report": false,
                }),
            ),
            (
                vec!["msg", "post", "hello", "--mission", ARCHIVED_ID],
                vec!["mission_list", "mission_get", "mission_post"],
                json!({
                    "mission_id": ARCHIVED_ID,
                    "text": "hello",
                    "to": null,
                }),
            ),
            (
                vec!["mission", "stop", "01M"],
                vec!["mission_list", "mission_stop"],
                json!({"id": MISSION_ID}),
            ),
            (
                vec!["mission", "resume", "01M"],
                vec!["mission_list", "mission_resume", "mission_get"],
                json!({"id": MISSION_ID}),
            ),
            (
                vec!["mission", "archive", "01M"],
                vec!["mission_list", "mission_archive"],
                json!({"id": MISSION_ID}),
            ),
            (
                vec!["mission", "unarchive", ARCHIVED_ID],
                vec!["mission_list", "mission_get", "mission_unarchive"],
                json!({"id": ARCHIVED_ID}),
            ),
            (
                vec!["mission", "rename", "01M", "New title"],
                vec!["mission_list", "mission_rename"],
                json!({"id": MISSION_ID, "title": "New title"}),
            ),
            (
                vec!["mission", "pin", "01M"],
                vec!["mission_list", "mission_pin"],
                json!({"id": MISSION_ID, "pinned": true}),
            ),
            (
                vec!["mission", "unpin", "01M"],
                vec!["mission_list", "mission_pin"],
                json!({"id": MISSION_ID, "pinned": false}),
            ),
            (
                vec!["mission", "move", "01M", "--unfile"],
                vec!["mission_list", "mission_set_project"],
                json!({"mission_id": MISSION_ID, "project_id": null}),
            ),
            (
                vec!["mission", "link", "01M", "--chat", "01S"],
                vec!["mission_list", "session_list", "mission_link"],
                json!({
                    "mission_id": MISSION_ID,
                    "session_id": "01S00000000000000000000000",
                    "no_report": false,
                    "caller_session_id": null,
                }),
            ),
            (
                vec!["mission", "unlink", "01M"],
                vec!["mission_list", "mission_unlink"],
                json!({"mission_id": MISSION_ID}),
            ),
            (
                vec!["mission", "feed", "01M", "--since", "7", "--limit", "2"],
                vec!["mission_list", "mission_feed"],
                json!({
                    "mission_id": MISSION_ID,
                    "since_offset": 7,
                    "limit": 2,
                    "order": "newest_first",
                }),
            ),
            (
                vec!["mission", "answer", "01M", "question", "yes"],
                vec!["mission_list", "mission_signal"],
                json!({
                    "mission_id": MISSION_ID,
                    "signal_type": "human_response",
                    "payload": {"question_id": "question", "choice": "yes"},
                }),
            ),
            (
                vec![
                    "mission",
                    "answer",
                    "01M",
                    "question",
                    "yes",
                    "--note",
                    "Only on Friday",
                ],
                vec!["mission_list", "mission_signal"],
                json!({
                    "mission_id": MISSION_ID,
                    "signal_type": "human_response",
                    "payload": {"question_id": "question", "choice": "yes", "note": "Only on Friday"},
                }),
            ),
            (
                vec!["chat", "start", "coder", "--project", "Runner"],
                vec!["role_list", "project_list", "session_start_direct"],
                json!({
                    "role_id": "role-id",
                    "runtime": null,
                    "model": null,
                    "effort": null,
                    "project_id": "project-id",
                    "cwd": null,
                }),
            ),
            (
                vec!["chat", "start", "--runtime", "codex", "--cwd", "."],
                vec!["session_start_direct"],
                json!({
                    "role_id": null,
                    "runtime": "codex",
                    "model": null,
                    "effort": null,
                    "project_id": null,
                    "cwd": cwd,
                }),
            ),
            (vec!["session", "list"], vec!["session_list"], json!({})),
            (
                vec!["session", "show", "01S"],
                vec!["session_list", "session_get"],
                json!({"session_id": SESSION_ID}),
            ),
            (
                vec!["session", "stop", "01S"],
                vec!["session_list", "session_stop"],
                json!({"session_id": SESSION_ID}),
            ),
            (
                vec!["session", "archive", "01S"],
                vec!["session_list", "session_archive"],
                json!({"session_id": SESSION_ID}),
            ),
            (
                vec!["session", "resume", "01S"],
                vec!["session_list", "session_resume"],
                json!({"session_id": SESSION_ID}),
            ),
            (
                vec!["session", "restart", "01S"],
                vec!["session_list", "session_restart"],
                json!({"session_id": SESSION_ID}),
            ),
            (
                vec!["session", "restart", MISSION_SESSION_ID],
                vec!["session_list", "session_restart"],
                json!({"session_id": MISSION_SESSION_ID}),
            ),
            (
                vec![
                    "msg",
                    "post",
                    "hello",
                    "--mission",
                    "01M",
                    "--to",
                    "lead",
                    "--as",
                    "coder",
                ],
                vec!["mission_list", "mission_post"],
                json!({
                    "mission_id": MISSION_ID,
                    "text": "hello",
                    "to": "lead",
                    "from": "coder",
                }),
            ),
            (
                vec![
                    "signal",
                    "ask_human",
                    "--mission",
                    "01M",
                    "--as",
                    "coder",
                    "--payload",
                    "{}",
                ],
                vec!["mission_list", "mission_signal"],
                json!({
                    "mission_id": MISSION_ID,
                    "signal_type": "ask_human",
                    "payload": {},
                    "from": "coder",
                }),
            ),
            (
                vec![
                    "ask",
                    "--human",
                    "Ship?",
                    "--choices",
                    "yes,no",
                    "--mission",
                    "01M",
                    "--as",
                    "coder",
                ],
                vec!["mission_list", "mission_signal"],
                json!({
                    "mission_id": MISSION_ID,
                    "signal_type": "ask_human",
                    "payload": {"prompt": "Ship?", "choices": ["yes", "no"]},
                    "from": "coder",
                }),
            ),
            (
                vec!["call", "role_get", r#"{"id":"role-id"}"#],
                vec!["role_get"],
                json!({"id": "role-id"}),
            ),
        ];

        let mut reached_requests = std::collections::BTreeSet::new();
        for (args, expected_tools, expected_arguments) in cases {
            let argv = std::iter::once("runner")
                .chain(args.iter().copied())
                .collect::<Vec<_>>();
            let cli = Cli::try_parse_from(&argv).unwrap_or_else(|error| {
                panic!("failed to parse {argv:?}: {error}");
            });
            let context = BusContext::OffBus;
            validate_remote(&cli, &context).unwrap_or_else(|error| {
                panic!("failed to validate {argv:?}: {}", error.message);
            });
            let client = RecordingClient::default();
            run_connected(&client, &cli, &context, None)
                .await
                .unwrap_or_else(|error| panic!("failed to run {argv:?}: {}", error.message));
            reached_requests.extend(client.requests.into_inner().unwrap());
            let calls = client.calls.into_inner().unwrap();
            let tools = calls
                .iter()
                .map(|(tool, _)| tool.as_str())
                .collect::<Vec<_>>();
            assert_eq!(tools, expected_tools, "wrong tool sequence for {argv:?}");
            assert_eq!(
                calls.last().unwrap().1,
                expected_arguments,
                "wrong final arguments for {argv:?}"
            );
        }
        let operations = runner_core::protocol::api::API_OPERATIONS
            .iter()
            .map(|(name, _)| *name)
            .collect::<std::collections::BTreeSet<_>>();
        assert!(reached_requests
            .iter()
            .all(|name| operations.contains(name.as_str())));
        let expected_requests = [
            "crew_list_all",
            "crew_get",
            "crew_create",
            "crew_update",
            "crew_delete",
            "role_list",
            "role_get",
            "role_get_by_handle",
            "role_create",
            "role_update",
            "role_delete",
            "slot_list",
            "slot_create",
            "slot_update",
            "slot_delete",
            "slot_reorder",
            "project_list",
            "project_get",
            "project_create_checked",
            "project_rename",
            "project_delete_checked",
            "mission_list",
            "mission_get",
            "mission_list_summary_impl",
            "mission_feed",
            "mission_status",
            "mission_start_impl_with_size",
            "mission_stop_impl",
            "mission_resume",
            "mission_archive_impl",
            "mission_unarchive_impl",
            "mission_pin_impl",
            "mission_rename_impl",
            "mission_set_project",
            "mission_link",
            "mission_unlink",
            "mission_post_impl",
            "mission_signal_impl",
            "session_list_with_activity",
            "session_get_with_status",
            "session_stop",
            "session_archive_direct",
            "session_start_chat",
            "session_resume",
            "session_restart",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(reached_requests, expected_requests);
    }

    #[test]
    fn every_command_leaf_parses_without_a_socket() {
        let cases: &[&[&str]] = &[
            &["status"],
            &["daemon", "status"],
            &["daemon", "stop"],
            &["project", "list"],
            &["project", "show", "Runner"],
            &["project", "create", "Runner", "--path", "."],
            &["project", "rename", "Runner", "New"],
            &["project", "delete", "Runner", "--force"],
            &["role", "list"],
            &["role", "show", "coder"],
            &["role", "create", "coder", "--runtime", "codex"],
            &["role", "update", "coder", "--model", "gpt"],
            &["role", "delete", "coder"],
            &["crew", "list"],
            &["crew", "show", "Peer"],
            &["crew", "create", "Peer"],
            &["crew", "update", "Peer", "--name", "Renamed"],
            &["crew", "delete", "Peer"],
            &["crew", "add", "Peer", "coder", "--as", "impl"],
            &["crew", "set", "Peer", "impl", "--effort", "high"],
            &["crew", "remove", "Peer", "impl"],
            &["crew", "order", "Peer", "lead", "impl"],
            &["mission", "list", "--crew", "Peer"],
            &["mission", "show", "01M"],
            &[
                "mission", "start", "--crew", "Peer", "--goal", "ship", "--title", "Ship",
            ],
            &["mission", "stop", "01M"],
            &["mission", "resume", "01M"],
            &["mission", "archive", "01M"],
            &["mission", "unarchive", "01M"],
            &["mission", "rename", "01M", "Title"],
            &["mission", "pin", "01M"],
            &["mission", "unpin", "01M"],
            &["mission", "move", "01M", "--unfile"],
            &["mission", "link", "01M", "--chat", "01S"],
            &["mission", "unlink", "01M"],
            &["mission", "feed", "01M", "--oldest-first"],
            &["mission", "answer", "01M", "question", "yes"],
            &["chat", "start", "coder"],
            &["chat", "start", "--runtime", "codex"],
            &["session", "list"],
            &["session", "show", "01S"],
            &["session", "stop", "01S"],
            &["session", "archive", "01S"],
            &["session", "resume", "01S"],
            &["session", "restart", "01S"],
            &["msg", "post", "hello", "--mission", "01M"],
            &["msg", "read"],
            &["signal", "ask_lead", "--mission", "01M"],
            &["ask", "question", "--mission", "01M", "--as", "coder"],
            &[
                "ask",
                "--human",
                "ship?",
                "--choices",
                "yes,no",
                "--mission",
                "01M",
                "--as",
                "coder",
            ],
            &["call", "mission_list", "{}"],
            &["help", "mission"],
        ];
        for args in cases {
            let argv = std::iter::once("runner")
                .chain(args.iter().copied())
                .collect::<Vec<_>>();
            assert!(
                Cli::try_parse_from(&argv).is_ok(),
                "failed to parse {argv:?}"
            );
            let leaf_len = if matches!(args[0], "status" | "signal" | "ask" | "call") {
                1
            } else {
                2
            };
            let leaf = format!("$ runner {}", args[..leaf_len].join(" "));
            assert!(
                include_str!("../tests/goldens/cli.txt")
                    .lines()
                    .any(|line| !line.ends_with("--help")
                        && line
                            .strip_prefix(&leaf)
                            .is_some_and(|tail| tail.is_empty() || tail.starts_with(' '))),
                "missing built-binary golden for {leaf}"
            );
        }
    }

    #[tokio::test]
    async fn mission_commands_carry_the_calling_chat() {
        async fn run(
            argv: &[&str],
            session: Option<&str>,
        ) -> (Result<(), CliError>, Vec<(String, Value)>) {
            let mut args = vec!["runner"];
            args.extend_from_slice(argv);
            let cli = Cli::try_parse_from(args).unwrap();
            let client = RecordingClient::default();
            let result = run_connected(&client, &cli, &BusContext::OffBus, session)
                .await
                .map(|_| ());
            (result, client.calls.into_inner().unwrap())
        }
        fn request<'a>(calls: &'a [(String, Value)], tool: &str) -> &'a Value {
            &calls.iter().find(|(name, _)| name == tool).unwrap().1
        }
        const MISSION: &str = "01M00000000000000000000000";
        const CHAT: &str = "01S00000000000000000000000";

        let (_, calls) = run(
            &["mission", "start", "--title", "t", "--no-report"],
            Some(CHAT),
        )
        .await;
        let start = request(&calls, "mission_start");
        assert_eq!(start["started_by_session_id"], CHAT);
        assert_eq!(start["no_report"], true);
        let (_, calls) = run(&["mission", "start", "--title", "t"], None).await;
        assert_eq!(
            request(&calls, "mission_start")["started_by_session_id"],
            Value::Null
        );

        let (_, calls) = run(&["mission", "link", "01M"], Some(CHAT)).await;
        let link = request(&calls, "mission_link");
        assert_eq!(link["mission_id"], MISSION);
        assert_eq!(link["session_id"], CHAT, "inside a chat, that chat");
        assert_eq!(link["caller_session_id"], CHAT);
        let (_, calls) = run(
            &["mission", "link", MISSION, "--chat", "01S", "--no-report"],
            None,
        )
        .await;
        let link = request(&calls, "mission_link");
        assert_eq!(link["session_id"], CHAT, "outside one, --chat");
        assert_eq!(link["caller_session_id"], Value::Null);
        assert_eq!(link["no_report"], true);
        let (result, calls) = run(&["mission", "link", MISSION], None).await;
        assert_eq!(
            result.unwrap_err().code,
            2,
            "outside a chat, --chat is needed"
        );
        assert!(calls.iter().all(|(name, _)| name != "mission_link"));

        let (_, calls) = run(&["mission", "unlink", MISSION], None).await;
        assert_eq!(request(&calls, "mission_unlink")["mission_id"], MISSION);

        let (_, calls) = run(&["mission", "list", "--linked-here"], Some(CHAT)).await;
        assert_eq!(request(&calls, "mission_list_linked")["session_id"], CHAT);
        let (result, calls) = run(&["mission", "list", "--linked-here"], None).await;
        let error = result.unwrap_err();
        assert_eq!(error.code, 2);
        assert!(
            error.message.contains("use runner mission list"),
            "{}",
            error.message
        );
        assert!(calls.is_empty());
    }
}
