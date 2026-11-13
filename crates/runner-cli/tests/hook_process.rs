use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Arc;

use runner_core::app_paths::IpcEndpoint;
use runner_core::protocol::hook;
use runner_core::protocol::wire::{self, Frame};
use runner_core::protocol::{Request, Response, Runtime};
use runner_daemon::ipc::IpcListener;
use runner_daemon::session::hook_queue::HookRoutes;
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

#[test]
fn command_failures_are_neutral_without_autostart() {
    let root = tempfile::tempdir().unwrap();
    for payload in [b"{".as_slice(), b"{}"] {
        for provider in ["codex", "claude-code", "copilot", "antigravity"] {
            let mut child = Command::new(env!("CARGO_BIN_EXE_runner-agent-cli"))
                .args([
                    "hook",
                    "report",
                    "--runtime",
                    provider,
                    "--event",
                    "PreInvocation",
                ])
                .env(hook::ENDPOINT_ENV, root.path().join("absent.sock"))
                .env(hook::SESSION_ENV, "absent")
                .env(hook::GENERATION_ENV, "old")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(payload).unwrap();
            let result = child.wait_with_output().unwrap();
            assert!(result.status.success());
            assert!(result.stderr.is_empty());
            assert_eq!(
                String::from_utf8(result.stdout).unwrap().trim(),
                if matches!(provider, "codex" | "antigravity") {
                    "{}"
                } else {
                    ""
                }
            );
        }
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn acknowledged_opencode_reports_fail_silently_when_bridge_is_absent() {
    let root = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_runner-agent-cli"))
        .args([
            "hook",
            "report",
            "--runtime",
            "opencode",
            "--event",
            "snapshot",
            "--acknowledge",
        ])
        .env(hook::ENDPOINT_ENV, root.path().join("absent.sock"))
        .env(hook::SESSION_ENV, "owned")
        .env(hook::GENERATION_ENV, "current")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"{}").unwrap();
    let result = child.wait_with_output().unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty() && result.stderr.is_empty());
}

#[tokio::test]
async fn acknowledged_opencode_reports_confirm_admission_and_bridge_loss() {
    for lost in [false, true] {
        #[cfg(unix)]
        let root = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let endpoint = IpcEndpoint(root.path().join("hook.sock"));
        #[cfg(windows)]
        let endpoint =
            IpcEndpoint(format!("\\\\.\\pipe\\opencode-ack-{}", ulid::Ulid::new()).into());
        let mut listener = IpcListener::bind(&endpoint).unwrap();
        let routes = Arc::new(HookRoutes::default());
        let mut receiver = routes.register(Runtime::OpenCode, "owned".into(), "current".into());
        let admission = routes.clone();
        let server = tokio::spawn(async move {
            let (mut read, mut write) = listener.accept().await.unwrap().into_split();
            assert_eq!(read_frame(&mut read).await.kind, wire::HELLO);
            write
                .write_all(
                    &Frame::json(wire::WELCOME, &json!({}))
                        .unwrap()
                        .encode()
                        .unwrap(),
                )
                .await
                .unwrap();
            let call: wire::Call = read_frame(&mut read).await.decode().unwrap();
            let Request::hook_report { report } = call.request else {
                panic!()
            };
            assert_eq!(report.bridge_unavailable, lost);
            let receipt = admission.admit(report).unwrap();
            write
                .write_all(
                    &Frame::json(
                        wire::RESPONSE,
                        &wire::Reply {
                            id: call.id,
                            response: Response::hook_report(Ok(())),
                        },
                    )
                    .unwrap()
                    .encode()
                    .unwrap(),
                )
                .await
                .unwrap();
            drop(receipt);
        });
        let child = tokio::task::spawn_blocking(move || {
            let mut command = Command::new(env!("CARGO_BIN_EXE_runner-agent-cli"));
            command.args([
                "hook",
                "report",
                "--runtime",
                "opencode",
                "--event",
                "snapshot",
                "--acknowledge",
            ]);
            if lost {
                command.arg("--bridge-unavailable");
            }
            let mut child = command
                .env(hook::ENDPOINT_ENV, endpoint.0)
                .env(hook::SESSION_ENV, "owned")
                .env(hook::GENERATION_ENV, "current")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(b"{}").unwrap();
            child.wait_with_output().unwrap()
        });
        let result = child.await.unwrap();
        assert!(result.status.success());
        assert!(result.stdout.is_empty() && result.stderr.is_empty());
        server.await.unwrap();
        let mut count = 0;
        assert_eq!(receiver.drain(|_| count += 1).is_err(), lost);
        assert_eq!(count, usize::from(!lost));
    }
}

async fn read_frame(read: &mut (impl AsyncRead + Unpin)) -> Frame {
    let length = read.read_u32_le().await.unwrap() as usize;
    assert!((1..=wire::MAX_FRAME).contains(&length));
    let kind = read.read_u8().await.unwrap();
    let mut payload = vec![0; length - 1];
    read.read_exact(&mut payload).await.unwrap();
    Frame { kind, payload }
}

#[tokio::test]
async fn native_command_reporters_preserve_utf8_and_provider_neutral_results() {
    let root = tempfile::tempdir().unwrap();
    let app_data = root.path().join("Jason's 路径 $ ` app data");
    std::fs::create_dir_all(app_data.join("bin")).unwrap();
    let executable = app_data
        .join("bin")
        .join(runner_core::cli_install::AGENT_DEST_BIN_NAME);
    std::fs::copy(env!("CARGO_BIN_EXE_runner-agent-cli"), &executable).unwrap();
    runner_daemon::runtimes::adapter(Runtime::Copilot)
        .status_hooks()
        .unwrap()
        .install(&app_data);
    let hooks: Value = serde_json::from_slice(
        &std::fs::read(app_data.join("copilot-hooks/hooks/hooks.json")).unwrap(),
    )
    .unwrap();
    let codex_args = runner_daemon::runtimes::adapter(Runtime::Codex).launch_args(
        &runner_daemon::runtimes::LaunchContext {
            env: &Default::default(),
            cwd: None,
            default_model: None,
            role_args: &[],
            app_data_dir: &app_data,
            session_id: "runner",
            resuming: false,
            mission: false,
            model: None,
            effort: None,
            codex_speed: None,
            system_prompt: None,
            first_turn: None,
        },
    );
    let codex_config = codex_args
        .iter()
        .find(|arg| arg.starts_with("hooks.SessionEnd="))
        .unwrap()
        .parse::<toml_edit::DocumentMut>()
        .unwrap();
    let codex_command = codex_config["hooks"]["SessionEnd"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    #[cfg(windows)]
    let shells = ["direct", "powershell", "pwsh"];
    #[cfg(unix)]
    let shells = ["direct", "sh"];
    for shell in shells {
        for provider in [
            Runtime::Codex,
            Runtime::ClaudeCode,
            Runtime::Copilot,
            Runtime::Antigravity,
        ] {
            if shell != "direct" && !matches!(provider, Runtime::Codex | Runtime::Copilot) {
                continue;
            }
            #[cfg(windows)]
            let endpoint =
                IpcEndpoint(format!(r"\\.\pipe\runner-report-{}", ulid::Ulid::new()).into());
            #[cfg(unix)]
            let endpoint = IpcEndpoint(root.path().join(format!("{}.sock", ulid::Ulid::new())));
            let mut listener = IpcListener::bind(&endpoint).unwrap();
            let routes = Arc::new(HookRoutes::default());
            let mut receiver = routes.register(provider, "runner".into(), "launch".into());
            let server = tokio::spawn(async move {
                let (mut read, mut write) = listener.accept().await.unwrap().into_split();
                assert_eq!(read_frame(&mut read).await.kind, wire::HELLO);
                write
                    .write_all(
                        &Frame::json(
                            wire::WELCOME,
                            &wire::Welcome {
                                exe_sha256: String::new(),
                                pid: 1,
                                started_at: String::new(),
                                app_clients: 0,
                            },
                        )
                        .unwrap()
                        .encode()
                        .unwrap(),
                    )
                    .await
                    .unwrap();
                let call: wire::Call = read_frame(&mut read).await.decode().unwrap();
                let Request::hook_report { report } = call.request else {
                    panic!("wrong operation")
                };
                let admission = routes.admit(report).unwrap();
                write
                    .write_all(
                        &Frame::json(
                            wire::RESPONSE,
                            &wire::Reply {
                                id: call.id,
                                response: Response::hook_report(Ok(())),
                            },
                        )
                        .unwrap()
                        .encode()
                        .unwrap(),
                    )
                    .await
                    .unwrap();
                drop(admission);
                routes
            });
            let payload = if provider == Runtime::Codex {
                json!({"hook_event_name":"SessionEnd","session_id":"root","transcript_path":null,"cwd":app_data,"reason":"other"})
            } else {
                json!({"session_id":"root", "tool_name":"Bash", "prompt": "你好 空格 ' $ `\n".repeat(32768)})
            };
            let expected = payload.clone();
            let mut command = if shell == "direct" {
                let mut command = Command::new(&executable);
                command.args([
                    "hook",
                    "report",
                    "--runtime",
                    provider.key(),
                    "--event",
                    if provider == Runtime::Codex {
                        "SessionEnd"
                    } else {
                        "PreToolUse"
                    },
                ]);
                command
            } else {
                let mut command = Command::new(shell);
                #[cfg(windows)]
                command.args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    if provider == Runtime::Codex {
                        codex_command
                    } else {
                        hooks["hooks"]["PreToolUse"][0]["hooks"][0]["powershell"]
                            .as_str()
                            .unwrap()
                    },
                ]);
                #[cfg(unix)]
                command.args([
                    "-c",
                    if provider == Runtime::Codex {
                        codex_command
                    } else {
                        hooks["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
                            .as_str()
                            .unwrap()
                    },
                ]);
                command
            };
            command
                .env(hook::ENDPOINT_ENV, &endpoint.0)
                .env(hook::SESSION_ENV, "runner")
                .env(hook::GENERATION_ENV, "launch");
            let child = tokio::task::spawn_blocking(move || {
                let mut child = command
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(&serde_json::to_vec_pretty(&payload).unwrap())
                    .unwrap();
                child.wait_with_output().unwrap()
            });
            let result = child.await.unwrap();
            assert!(result.status.success(), "{shell}: {result:?}");
            assert!(result.stderr.is_empty(), "{shell}: {result:?}");
            assert_eq!(
                String::from_utf8(result.stdout).unwrap().trim(),
                if matches!(provider, Runtime::Codex | Runtime::Antigravity) {
                    "{}"
                } else {
                    ""
                }
            );
            let _routes = tokio::time::timeout(std::time::Duration::from_secs(3), server)
                .await
                .expect("report was not admitted")
                .unwrap();
            let mut reports = Vec::new();
            receiver.drain(|report| reports.push(report)).unwrap();
            assert_eq!(reports.len(), 1);
            assert_eq!(reports[0].payload, expected);
            if provider == Runtime::Codex {
                assert_eq!(reports[0].caller_thread_id.as_deref(), Some("root"));
            }
        }
    }
}

#[tokio::test]
async fn codex_session_end_rejects_stale_and_retired_launches_neutrally() {
    for retired in [false, true] {
        #[cfg(unix)]
        let root = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        let endpoint = IpcEndpoint(format!(r"\\.\pipe\runner-end-{}", ulid::Ulid::new()).into());
        #[cfg(unix)]
        let endpoint = IpcEndpoint(root.path().join("end.sock"));
        let mut listener = IpcListener::bind(&endpoint).unwrap();
        let routes = Arc::new(HookRoutes::default());
        let mut receiver = routes.register(Runtime::Codex, "runner".into(), "current".into());
        if retired {
            routes.retire("runner");
        }
        let server = tokio::spawn(async move {
            let (mut read, mut write) = listener.accept().await.unwrap().into_split();
            assert_eq!(read_frame(&mut read).await.kind, wire::HELLO);
            write
                .write_all(
                    &Frame::json(
                        wire::WELCOME,
                        &wire::Welcome {
                            exe_sha256: String::new(),
                            pid: 1,
                            started_at: String::new(),
                            app_clients: 0,
                        },
                    )
                    .unwrap()
                    .encode()
                    .unwrap(),
                )
                .await
                .unwrap();
            let call: wire::Call = read_frame(&mut read).await.decode().unwrap();
            let Request::hook_report { report } = call.request else {
                panic!("wrong operation")
            };
            assert!(report.valid());
            assert_eq!(report.caller_thread_id.as_deref(), Some("root"));
            let error = routes.admit(report).err().expect("old launch was admitted");
            write
                .write_all(
                    &Frame::json(
                        wire::RESPONSE,
                        &wire::Reply {
                            id: call.id,
                            response: Response::hook_report(Err(
                                runner_core::protocol::ClientError::msg(error.to_string()),
                            )),
                        },
                    )
                    .unwrap()
                    .encode()
                    .unwrap(),
                )
                .await
                .unwrap();
            routes
        });
        let endpoint_path = endpoint.0.clone();
        let child = tokio::task::spawn_blocking(move || {
            let mut child = Command::new(env!("CARGO_BIN_EXE_runner-agent-cli"))
                .args([
                    "hook",
                    "report",
                    "--runtime",
                    "codex",
                    "--event",
                    "SessionEnd",
                ])
                .env(hook::ENDPOINT_ENV, endpoint_path)
                .env(hook::SESSION_ENV, "runner")
                .env(
                    hook::GENERATION_ENV,
                    if retired { "current" } else { "old" },
                )
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(br#"{"hook_event_name":"SessionEnd","session_id":"root","transcript_path":null,"cwd":"fixture","reason":"other"}"#).unwrap();
            child.wait_with_output().unwrap()
        });
        let result = child.await.unwrap();
        assert!(result.status.success());
        assert!(result.stderr.is_empty());
        assert_eq!(String::from_utf8(result.stdout).unwrap().trim(), "{}");
        let _routes = tokio::time::timeout(std::time::Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
        let mut observed = Vec::new();
        let drained = receiver.drain(|report| observed.push(report));
        assert_eq!(drained.is_err(), retired);
        assert!(observed.is_empty());
    }
}

#[test]
fn antigravity_title_hook_prints_trimmed_title_and_stays_silent_on_blank_or_invalid_stdin() {
    let root = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    let app_data = root.path().join("Jason's 路径 app data");
    #[cfg(windows)]
    let app_data = root.path().join("Jason 路径 app data");
    std::fs::create_dir_all(app_data.join("bin")).unwrap();
    let executable = app_data
        .join("bin")
        .join(runner_core::cli_install::AGENT_DEST_BIN_NAME);
    std::fs::copy(env!("CARGO_BIN_EXE_runner-agent-cli"), &executable).unwrap();

    let decoy_dir = root.path().join("decoy");
    std::fs::create_dir_all(&decoy_dir).unwrap();
    let decoy = decoy_dir.join(runner_core::cli_install::AGENT_DEST_BIN_NAME);
    std::fs::write(&decoy, b"not-an-executable").unwrap();

    let agy_args = runner_daemon::runtimes::adapter(Runtime::Antigravity).launch_args(
        &runner_daemon::runtimes::LaunchContext {
            env: &Default::default(),
            cwd: None,
            default_model: None,
            role_args: &[],
            app_data_dir: &app_data,
            session_id: "runner",
            resuming: false,
            mission: false,
            model: None,
            effort: None,
            codex_speed: None,
            system_prompt: None,
            first_turn: None,
        },
    );
    let settings: Value = serde_json::from_str(
        agy_args
            .windows(2)
            .find(|pair| pair[0] == "--settings")
            .expect("Antigravity should emit --settings")[1]
            .as_str(),
    )
    .unwrap();
    let title_command = settings["title"]["command"].as_str().unwrap();

    for via_shell in [false, true] {
        for (stdin_bytes, expected_stdout) in [
            (
                r#"{"cwd":"/tmp","conversation_title":"  修复 Antigravity 标题  ","product":"antigravity"}"#
                    .as_bytes(),
                "修复 Antigravity 标题\n",
            ),
            (
                br#"{"conversation_title":"ASCII title"}"#.as_slice(),
                "ASCII title\n",
            ),
            (
                br#"{"cwd":"/tmp","product":"antigravity"}"#.as_slice(),
                "",
            ),
            (br#"{"conversation_title":"   \t\r\n "}"#.as_slice(), ""),
            (br#"{"conversation_title":null}"#.as_slice(), ""),
            (b"{malformed".as_slice(), ""),
            (b"".as_slice(), ""),
        ] {
            let mut cmd = if via_shell {
                #[cfg(unix)]
                {
                    let mut c = Command::new("sh");
                    c.args(["-c", title_command]);
                    c
                }
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt as _;
                    let mut c = Command::new("cmd");
                    c.raw_arg("/c").raw_arg(title_command);
                    c
                }
            } else {
                let mut c = Command::new(&executable);
                c.args(["hook", "title"]);
                c
            };
            let mut child = cmd
                .env(runner_core::protocol::SESSION_CLI_ENV, &decoy)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(stdin_bytes).unwrap();
            let result = child.wait_with_output().unwrap();
            assert!(
                result.status.success(),
                "via_shell={via_shell}: {result:?}"
            );
            assert!(
                result.stderr.is_empty(),
                "via_shell={via_shell}: {result:?}"
            );
            assert_eq!(
                String::from_utf8(result.stdout).unwrap(),
                expected_stdout,
                "via_shell={via_shell}"
            );
        }
    }
}
