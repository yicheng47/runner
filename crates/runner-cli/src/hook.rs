use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};

use runner_core::protocol::hook::{self, HookReport, DEADLINE, MAX_ENVELOPE_BYTES};
use runner_core::protocol::wire::{self, Frame};
use runner_core::protocol::{Request, Response, Runtime};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(clap::Subcommand, Debug)]
pub enum Command {
    Report {
        #[arg(long)]
        runtime: String,
        #[arg(long)]
        event: String,
        #[arg(long)]
        acknowledge: bool,
        #[arg(long)]
        bridge_unavailable: bool,
    },
    Serve,
    Title,
}

pub fn run(command: &Command) -> i32 {
    if matches!(command, Command::Title) {
        let _ = write_conversation_title(io::stdin().lock(), io::stdout().lock());
        return 0;
    }
    let route = Route::from_env();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    match command {
        Command::Report {
            runtime: provider,
            event,
            acknowledge,
            bridge_unavailable,
        } => {
            let mut admitted = false;
            let mut bytes = Vec::new();
            let input = std::io::Read::read_to_end(
                &mut std::io::Read::take(io::stdin().lock(), (MAX_ENVELOPE_BYTES + 1) as u64),
                &mut bytes,
            );
            if input.is_ok() && bytes.len() <= MAX_ENVELOPE_BYTES {
                if let (Ok(runtime), Some(provider), Ok(payload)) = (
                    runtime.as_ref(),
                    Runtime::parse(provider),
                    serde_json::from_slice(&bytes),
                ) {
                    if let Some(route) = route.as_ref() {
                        let caller = command_caller_thread_id(provider, event, &payload);
                        let mut report = route.envelope(provider, event.clone(), payload, caller);
                        report.bridge_unavailable = *bridge_unavailable;
                        admitted = runtime.block_on(deliver(&route.endpoint, report)).is_ok();
                    }
                }
            }
            let neutral = neutral(provider, event);
            if !neutral.is_empty() {
                println!("{neutral}");
            }
            if *acknowledge && !admitted {
                return 1;
            }
        }
        Command::Serve => {
            if let Ok(runtime) = runtime {
                let _ = serve(
                    &runtime,
                    route.as_ref(),
                    io::stdin().lock(),
                    io::stdout().lock(),
                );
            }
        }
        Command::Title => {}
    }
    0
}

fn write_conversation_title(input: impl Read, mut output: impl Write) -> io::Result<()> {
    let mut bytes = Vec::new();
    input
        .take((MAX_ENVELOPE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_ENVELOPE_BYTES {
        return Ok(());
    }
    let Ok(payload) = serde_json::from_slice::<Value>(&bytes) else {
        return Ok(());
    };
    if let Some(title) = payload
        .get("conversation_title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
    {
        writeln!(output, "{title}")?;
        output.flush()?;
    }
    Ok(())
}

fn command_caller_thread_id(runtime: Runtime, event: &str, payload: &Value) -> Option<String> {
    // Codex has no MCP handler at teardown, so this is the payload's own claim, not caller
    // evidence: internal workers also emit SessionEnd, and the parser ignores foreign ends.
    if runtime != Runtime::Codex
        || event != "SessionEnd"
        || payload["hook_event_name"] != "SessionEnd"
        || payload["reason"] != "other"
        || !payload["cwd"].as_str().is_some_and(|cwd| !cwd.is_empty())
        || !payload
            .get("transcript_path")
            .is_some_and(|path| path.is_null() || path.is_string())
        || payload.get("agent_id").is_some()
        || payload.get("agent_type").is_some()
    {
        return None;
    }
    payload["session_id"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .map(str::to_owned)
}

fn neutral(runtime: &str, _event: &str) -> String {
    match runtime {
        "codex" | "antigravity" => "{}".into(),
        _ => String::new(),
    }
}

struct Route {
    endpoint: PathBuf,
    session: String,
    generation: String,
}

impl Route {
    fn from_env() -> Option<Self> {
        Some(Self {
            endpoint: std::env::var_os(hook::ENDPOINT_ENV)?.into(),
            session: std::env::var(hook::SESSION_ENV).ok()?,
            generation: std::env::var(hook::GENERATION_ENV).ok()?,
        })
    }

    fn envelope(
        &self,
        runtime: Runtime,
        event: String,
        payload: Value,
        caller_thread_id: Option<String>,
    ) -> HookReport {
        HookReport {
            bridge_unavailable: false,
            version: hook::VERSION,
            runtime,
            session_id: self.session.clone(),
            generation: self.generation.clone(),
            event,
            payload,
            caller_thread_id,
        }
    }
}

pub async fn deliver(endpoint: &Path, report: HookReport) -> io::Result<()> {
    if !report.valid() || serde_json::to_vec(&report)?.len() > MAX_ENVELOPE_BYTES {
        return Err(wire::invalid("invalid hook envelope"));
    }
    tokio::time::timeout(DEADLINE, async {
        #[cfg(unix)]
        let mut stream = tokio::net::UnixStream::connect(endpoint).await?;
        #[cfg(windows)]
        let mut stream = loop {
            match tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint) {
                Ok(stream) => break stream,
                Err(error) if error.raw_os_error() == Some(231) => {
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await
                }
                Err(error) => return Err(error),
            }
        };
        write_frame(
            &mut stream,
            Frame::json(
                wire::HELLO,
                &wire::Hello {
                    exe_sha256: String::new(),
                    client: "hook".into(),
                },
            )?,
        )
        .await?;
        if read_frame(&mut stream).await?.kind != wire::WELCOME {
            return Err(wire::invalid("hook handshake rejected"));
        }
        write_frame(
            &mut stream,
            Frame::json(
                wire::REQUEST,
                &wire::Call {
                    id: 1,
                    request: Request::hook_report { report },
                },
            )?,
        )
        .await?;
        let frame = read_frame(&mut stream).await?;
        if frame.kind != wire::RESPONSE {
            return Err(wire::invalid("hook acknowledgment missing"));
        }
        let reply: wire::Reply = frame.decode()?;
        match reply.response {
            Response::hook_report(Ok(())) if reply.id == 1 => Ok(()),
            _ => Err(wire::invalid("hook admission rejected")),
        }
    })
    .await
    .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
}

async fn write_frame(stream: &mut (impl AsyncWrite + Unpin), frame: Frame) -> io::Result<()> {
    stream.write_all(&frame.encode()?).await
}

async fn read_frame(stream: &mut (impl AsyncRead + Unpin)) -> io::Result<Frame> {
    let len = stream.read_u32_le().await? as usize;
    if !(1..=64 * 1024).contains(&len) {
        return Err(wire::invalid("invalid hook response length"));
    }
    let kind = stream.read_u8().await?;
    let mut payload = vec![0; len - 1];
    stream.read_exact(&mut payload).await?;
    Ok(Frame { kind, payload })
}

fn serve(
    runtime: &tokio::runtime::Runtime,
    route: Option<&Route>,
    mut input: impl BufRead,
    mut output: impl Write,
) -> io::Result<()> {
    loop {
        let mut line = Vec::new();
        if std::io::Read::take(&mut input, wire::MAX_FRAME as u64).read_until(b'\n', &mut line)?
            == 0
        {
            return Ok(());
        }
        if line.last() != Some(&b'\n') {
            while !input.fill_buf()?.is_empty() {
                let buffer = input.fill_buf()?;
                let end = buffer.iter().position(|byte| *byte == b'\n');
                let count = end.map_or(buffer.len(), |index| index + 1);
                input.consume(count);
                if end.is_some() {
                    break;
                }
            }
            continue;
        }
        let request: Value = match serde_json::from_slice(&line) {
            Ok(request) => request,
            Err(_) => {
                writeln!(
                    output,
                    "{}",
                    json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Invalid JSON"}})
                )?;
                output.flush()?;
                continue;
            }
        };
        let Some(id) = request.get("id") else {
            continue;
        };
        let method = request["method"].as_str().unwrap_or_default();
        let result = match method {
            "initialize" => {
                json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"runner-hooks","version":env!("CARGO_PKG_VERSION")}})
            }
            "ping" => json!({}),
            "tools/list" => {
                json!({"tools":[{"name":"report","description":"Admit native hook telemetry","inputSchema":{"type":"object","properties":{"hook_event_name":{"type":"string"},"session_id":{"type":"string"},"transcript_path":{"type":["string","null"]},"turn_id":{"type":"string"},"source":{"type":"string"}},"required":["hook_event_name","session_id","transcript_path"]}}]})
            }
            "tools/call" if request["params"]["name"] == "report" => {
                let payload = request["params"]["arguments"].clone();
                let event = payload["hook_event_name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                let thread = request["params"]["_meta"]["threadId"]
                    .as_str()
                    .map(str::to_owned);
                let mut response = json!({});
                if let Some(route) = route {
                    if let Some(output) =
                        hook::codex_startup_output(&route.session, &payload, thread.as_deref())
                    {
                        response = output;
                    }
                    let _ = runtime.block_on(deliver(
                        &route.endpoint,
                        route.envelope(Runtime::Codex, event, payload, thread),
                    ));
                }
                json!({"content":[{"type":"text","text":response.to_string()}],"isError":false})
            }
            _ => {
                writeln!(
                    output,
                    "{}",
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Unknown hook operation"}})
                )?;
                output.flush()?;
                continue;
            }
        };
        writeln!(
            output,
            "{}",
            json!({"jsonrpc":"2.0","id":id,"result":result})
        )?;
        output.flush()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runner_core::app_paths::IpcEndpoint;
    use runner_daemon::ipc::IpcListener;
    use runner_daemon::session::hook_queue::HookRoutes;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn endpoint(root: &Path) -> IpcEndpoint {
        #[cfg(unix)]
        {
            IpcEndpoint(root.join("hooks.sock"))
        }
        #[cfg(windows)]
        {
            IpcEndpoint(PathBuf::from(format!(
                r"\\.\pipe\runner-hook-test-{}",
                root.file_name().unwrap().to_string_lossy()
            )))
        }
    }

    fn report() -> HookReport {
        HookReport {
            bridge_unavailable: false,
            version: hook::VERSION,
            runtime: Runtime::Codex,
            session_id: "runner".into(),
            generation: "launch".into(),
            event: "UserPromptSubmit".into(),
            payload: json!({"hook_event_name":"UserPromptSubmit","session_id":"root","turn_id":"turn","transcript_path":null}),
            caller_thread_id: Some("root".into()),
        }
    }

    #[test]
    fn only_proven_command_session_end_can_supply_codex_caller_identity() {
        let payload = json!({"hook_event_name":"SessionEnd","session_id":"root","transcript_path":null,"cwd":"C:/路径 空格","reason":"other"});
        for transcript in [Value::Null, json!("C:/目录/rollout.jsonl")] {
            let mut root = payload.clone();
            root["transcript_path"] = transcript;
            assert_eq!(
                command_caller_thread_id(Runtime::Codex, "SessionEnd", &root).as_deref(),
                Some("root")
            );
        }
        let mut rejected = Vec::new();
        for field in payload.as_object().unwrap().keys() {
            let mut missing = payload.clone();
            missing.as_object_mut().unwrap().remove(field);
            rejected.push(missing);
        }
        for (field, value) in [
            ("session_id", json!("")),
            ("session_id", json!("  ")),
            ("session_id", Value::Null),
            ("hook_event_name", json!("Interrupt")),
            ("reason", json!("different")),
            ("reason", Value::Null),
            ("cwd", json!(42)),
            ("cwd", json!("")),
            ("transcript_path", json!(42)),
            ("agent_id", json!("child")),
            ("agent_id", Value::Null),
            ("agent_type", json!("worker")),
            ("agent_type", Value::Null),
        ] {
            let mut invalid = payload.clone();
            invalid[field] = value;
            rejected.push(invalid);
        }
        for payload in rejected {
            assert_eq!(
                command_caller_thread_id(Runtime::Codex, "SessionEnd", &payload),
                None,
                "{payload}"
            );
        }
        for event in [
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "PreCompact",
            "PostCompact",
            "Stop",
            "Interrupt",
        ] {
            let mut ordinary = payload.clone();
            ordinary["hook_event_name"] = json!(event);
            assert_eq!(
                command_caller_thread_id(Runtime::Codex, event, &ordinary),
                None
            );
            assert_eq!(
                command_caller_thread_id(Runtime::Codex, event, &payload),
                None
            );
        }
        let mut mcp = report();
        mcp.event = "SessionEnd".into();
        mcp.payload = payload;
        mcp.caller_thread_id = None;
        assert!(!mcp.valid());
    }

    async fn receive(read: &mut (impl AsyncRead + Unpin)) -> Frame {
        let len = read.read_u32_le().await.unwrap() as usize;
        assert!((1..=wire::MAX_FRAME).contains(&len));
        let kind = read.read_u8().await.unwrap();
        let mut payload = vec![0; len - 1];
        read.read_exact(&mut payload).await.unwrap();
        Frame { kind, payload }
    }

    async fn welcome(write: &mut (impl AsyncWrite + Unpin)) {
        write_frame(
            write,
            Frame::json(
                wire::WELCOME,
                &wire::Welcome {
                    exe_sha256: String::new(),
                    pid: 1,
                    started_at: String::new(),
                    app_clients: 0,
                },
            )
            .unwrap(),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn native_transport_admits_utf8_and_preserves_null_transcript() {
        let root = tempfile::tempdir().unwrap();
        let endpoint = endpoint(root.path());
        let mut listener = IpcListener::bind(&endpoint).unwrap();
        let routes = Arc::new(HookRoutes::default());
        let mut receiver = routes.register(Runtime::Codex, "runner".into(), "launch".into());
        let admission_routes = routes.clone();
        let server = tokio::spawn(async move {
            let (mut read, mut write) = listener.accept().await.unwrap().into_split();
            let hello: wire::Hello = receive(&mut read).await.decode().unwrap();
            assert_eq!(hello.client, "hook");
            welcome(&mut write).await;
            let call: wire::Call = receive(&mut read).await.decode().unwrap();
            let Request::hook_report { report } = call.request else {
                panic!("wrong request")
            };
            let receipt = admission_routes.admit(report).unwrap();
            write_frame(
                &mut write,
                Frame::json(
                    wire::RESPONSE,
                    &wire::Reply {
                        id: call.id,
                        response: Response::hook_report(Ok(())),
                    },
                )
                .unwrap(),
            )
            .await
            .unwrap();
            drop(receipt);
        });
        let mut report = report();
        report.payload["prompt"] = json!("路径 空格 ' $ ` ".repeat(64 * 1024));
        deliver(&endpoint.0, report.clone()).await.unwrap();
        server.await.unwrap();
        let mut received = Vec::new();
        receiver
            .drain(|report| received.push(report.payload))
            .unwrap();
        assert_eq!(received, vec![report.payload]);
    }

    #[tokio::test]
    async fn total_deadline_bounds_handshake_write_and_ack_without_retry() {
        for stage in ["handshake", "write", "ack"] {
            let root = tempfile::tempdir().unwrap();
            let endpoint = endpoint(root.path());
            let mut listener = IpcListener::bind(&endpoint).unwrap();
            let server = tokio::spawn(async move {
                let (mut read, mut write) = listener.accept().await.unwrap().into_split();
                receive(&mut read).await;
                if stage != "handshake" {
                    welcome(&mut write).await;
                }
                if stage == "ack" {
                    receive(&mut read).await;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            });
            let mut report = report();
            if stage == "write" {
                report.payload["large"] = json!("x".repeat(4 * 1024 * 1024));
            }
            let started = Instant::now();
            let error = deliver(&endpoint.0, report).await.unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::TimedOut, "{stage}: {error}");
            assert!(started.elapsed() >= DEADLINE);
            assert!(
                started.elapsed() < Duration::from_secs(1),
                "{stage}: {:?}",
                started.elapsed()
            );
            server.abort();
        }
    }

    #[tokio::test]
    async fn daemon_shutdown_during_exchange_fails_without_replay() {
        for welcomed in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let endpoint = endpoint(root.path());
            let mut listener = IpcListener::bind(&endpoint).unwrap();
            let server = tokio::spawn(async move {
                let (mut read, mut write) = listener.accept().await.unwrap().into_split();
                receive(&mut read).await;
                if welcomed {
                    welcome(&mut write).await;
                    receive(&mut read).await;
                }
            });
            let started = Instant::now();
            assert!(deliver(&endpoint.0, report()).await.is_err());
            assert!(started.elapsed() < Duration::from_secs(1));
            server.await.unwrap();
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn busy_pipe_uses_the_same_total_deadline() {
        use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};
        let root = tempfile::tempdir().unwrap();
        let endpoint = endpoint(root.path());
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .max_instances(1)
            .create(&endpoint.0)
            .unwrap();
        let _occupied = ClientOptions::new().open(&endpoint.0).unwrap();
        server.connect().await.unwrap();
        assert_eq!(
            ClientOptions::new()
                .open(&endpoint.0)
                .unwrap_err()
                .raw_os_error(),
            Some(231)
        );
        let started = Instant::now();
        assert_eq!(
            deliver(&endpoint.0, report()).await.unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn startup_hook_marks_runtime_only_chats_without_starting_a_turn() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let root = tempfile::tempdir().unwrap();
        let route = Route {
            endpoint: endpoint(root.path()).0,
            session: "01M6K3Q2XF5NTWR80G0MREWQ2E".into(),
            generation: "launch".into(),
        };
        for (event, source, caller, routed, marked) in [
            ("SessionStart", "startup", Some("root"), true, true),
            ("SessionStart", "resume", Some("root"), true, false),
            ("SessionStart", "clear", Some("root"), true, false),
            ("SessionStart", "compact", Some("root"), true, false),
            ("SessionStart", "fork", Some("root"), true, false),
            ("UserPromptSubmit", "startup", Some("root"), true, false),
            ("SessionStart", "startup", Some("child"), true, false),
            ("SessionStart", "startup", None, true, false),
            ("SessionStart", "startup", Some("root"), false, false),
        ] {
            let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
                "name":"report",
                "arguments":{"hook_event_name":event,"source":source,"session_id":"root","transcript_path":null},
                "_meta":{"threadId":caller},
            }});
            let input = format!("{request}\n");
            let mut output = Vec::new();
            serve(&rt, routed.then_some(&route), input.as_bytes(), &mut output).unwrap();
            let reply: Value = serde_json::from_slice(&output).unwrap();
            let context: Value =
                serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            if marked {
                assert_eq!(
                    context,
                    json!({"hookSpecificOutput": {
                        "hookEventName": "SessionStart",
                        "additionalContext": hook::codex_session_marker(&route.session),
                    }})
                );
            } else {
                assert_eq!(context, json!({}), "{event} {source} {caller:?} {routed}");
            }
            assert_eq!(reply["result"]["isError"], false);
        }
    }

    #[test]
    fn stdio_initializes_without_daemon_and_keeps_forwarding_failures_neutral() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let root = tempfile::tempdir().unwrap();
        let route = Route {
            endpoint: endpoint(root.path()).0,
            session: "runner".into(),
            generation: "launch".into(),
        };
        let mut requests = vec![
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        ];
        for (id, caller) in [(3, json!(null)), (4, json!("child")), (5, json!("root"))] {
            requests.push(json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"report","arguments":report().payload,"_meta":{"threadId":caller}}}));
        }
        let mut large = report().payload;
        large["large"] = json!("x".repeat(MAX_ENVELOPE_BYTES));
        requests.push(json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"report","arguments":large,"_meta":{"threadId":"root"}}}));
        let input = requests
            .iter()
            .map(|value| format!("{value}\n"))
            .collect::<String>();
        let mut output = Vec::new();
        serve(&rt, Some(&route), input.as_bytes(), &mut output).unwrap();
        let replies: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(replies.len(), 6);
        assert_eq!(replies[0]["result"]["serverInfo"]["name"], "runner-hooks");
        let tools = replies[1]["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "report");
        for reply in &replies[2..] {
            assert_eq!(reply["result"]["isError"], false);
            assert_eq!(reply["result"]["content"][0]["text"], "{}");
        }
        assert_eq!(neutral("copilot", "PreToolUse"), "");
        assert_eq!(neutral("claude-code", "PreToolUse"), "");
    }

    #[test]
    fn title_hook_extracts_trimmed_conversation_title_and_ignores_other_inputs() {
        let run_case = |input: &[u8]| {
            let mut output = Vec::new();
            write_conversation_title(input, &mut output).unwrap();
            String::from_utf8(output).unwrap()
        };

        assert_eq!(
            run_case(br#"{"conversation_title":"Fix session title"}"#),
            "Fix session title\n"
        );
        assert_eq!(
            run_case(
                r#"{"cwd":"/tmp","conversation_title":"  修复会话标题 🚀 \t\r\n","product":"antigravity"}"#
                    .as_bytes()
            ),
            "修复会话标题 🚀\n"
        );

        for silent in [
            b"".as_slice(),
            b"   \n",
            b"{",
            b"not json",
            b"null",
            b"[]",
            b"\"string\"",
            b"42",
            br#"{"cwd":"/tmp","session_id":"s","product":"antigravity"}"#,
            br#"{"conversation_title":""}"#,
            br#"{"conversation_title":"   \t\r\n\u3000"}"#,
            br#"{"conversation_title":null}"#,
            br#"{"conversation_title":123}"#,
            br#"{"conversation_title":true}"#,
            br#"{"conversation_title":["topic"]}"#,
            br#"{"conversation_title":{"text":"topic"}}"#,
        ] {
            assert_eq!(
                run_case(silent),
                "",
                "{:?}",
                String::from_utf8_lossy(silent)
            );
        }

        let mut oversized = br#"{"conversation_title":"Oversized","pad":""#.to_vec();
        oversized.resize(MAX_ENVELOPE_BYTES + 16, b'x');
        oversized.extend_from_slice(br#""}"#);
        let reader = std::io::Read::chain(oversized.as_slice(), io::repeat(b'x'));
        let mut output = Vec::new();
        write_conversation_title(reader, &mut output).unwrap();
        assert!(output.is_empty());
    }
}
