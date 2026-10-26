use super::*;
use runner_core::remote::{RemoteState, StopReason};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::protocol::Role;

struct Service {
    origin: String,
    task: tokio::task::JoinHandle<()>,
    polls: Arc<AtomicUsize>,
    signouts: Arc<AtomicUsize>,
    connections: Arc<AtomicUsize>,
    close_code: Arc<AtomicUsize>,
    refusal: Arc<AtomicUsize>,
    welcome: Arc<std::sync::atomic::AtomicBool>,
    start_gate: Arc<std::sync::Mutex<Option<Arc<CallGate>>>>,
    start_status: Arc<AtomicUsize>,
    signout_gate: Arc<std::sync::Mutex<Option<Arc<CallGate>>>>,
    signout_status: Arc<AtomicUsize>,
}

struct HeartbeatWriter {
    read: tokio::io::DuplexStream,
    entered: Arc<tokio::sync::Notify>,
    fail: bool,
}
impl tokio::io::AsyncRead for HeartbeatWriter {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.read).poll_read(cx, buffer)
    }
}
impl tokio::io::AsyncWrite for HeartbeatWriter {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        _: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.entered.notify_one();
        if self.fail {
            std::task::Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()))
        } else {
            std::task::Poll::Pending
        }
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Pending
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn heartbeat_write_failures_release_local_access_removal_commands() {
    for fail in [false, true] {
        for action in ["pause", "revoke", "reset", "logout"] {
            let service = Service::new(vec![]).await;
            let root = tempfile::tempdir().unwrap();
            let handle = spawn_with_store(
                root.path().into(),
                Arc::new(store::MemoryStore::default()),
                crate::events::EventChannel::new(),
            );
            handle
                .start(Some(service.origin.clone()), false)
                .await
                .unwrap();
            until(&handle, |s| {
                matches!(s.state, RemoteState::Connected { .. })
            })
            .await;
            handle
                .set_pin(remote::Secret("012345".into()))
                .await
                .unwrap();
            handle
                .record_paired(vec![7; 32], "test device".into())
                .await
                .unwrap();
            let fingerprint = handle.trust().devices[0].fingerprint.clone();
            let entered = Arc::new(tokio::sync::Notify::new());
            let (host, peer) = tokio::io::duplex(1024);
            let host = tokio_tungstenite::WebSocketStream::from_raw_socket(
                Box::new(HeartbeatWriter {
                    read: host,
                    entered: entered.clone(),
                    fail,
                }) as Box<dyn transport::Connection>,
                Role::Client,
                None,
            )
            .await;
            let mut peer =
                tokio_tungstenite::WebSocketStream::from_raw_socket(peer, Role::Server, None).await;
            handle
                .request(Command::TestSocket(transport::TestSocket(Box::new(host))))
                .await
                .unwrap();
            peer.send(Message::Ping(vec![1].into())).await.unwrap();
            entered.notified().await;
            let command = match action {
                "pause" => Command::Pause(true),
                "revoke" => Command::RevokeDevice(fingerprint),
                "reset" => Command::ResetIdentity(store::Identity::generate().unwrap()),
                _ => Command::Logout,
            };
            let caller = handle.clone();
            let removal = tokio::spawn(async move { caller.request(command).await });
            if !fail {
                tokio::time::pause();
                tokio::time::advance(std::time::Duration::from_secs(11)).await;
                tokio::time::resume();
            }
            tokio::time::timeout(std::time::Duration::from_secs(3), removal)
                .await
                .expect(action)
                .unwrap()
                .unwrap();
            assert!(!matches!(
                handle.status().state,
                RemoteState::Connected { .. }
            ));
            if matches!(action, "revoke" | "reset") {
                assert!(handle.trust().devices.is_empty());
            }
            handle.shutdown().await;
        }
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Service {
    async fn new(outcomes: Vec<&'static str>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let polls = Arc::new(AtomicUsize::new(0));
        let signouts = Arc::new(AtomicUsize::new(0));
        let connections = Arc::new(AtomicUsize::new(0));
        let close_code = Arc::new(AtomicUsize::new(0));
        let refusal = Arc::new(AtomicUsize::new(0));
        let welcome = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let (o, p, s, c, close, refuse, send_welcome) = (
            origin.clone(),
            polls.clone(),
            signouts.clone(),
            connections.clone(),
            close_code.clone(),
            refusal.clone(),
            welcome.clone(),
        );
        let start_gate = Arc::new(std::sync::Mutex::new(None::<Arc<CallGate>>));
        let start_status = Arc::new(AtomicUsize::new(200));
        let signout_gate = Arc::new(std::sync::Mutex::new(None::<Arc<CallGate>>));
        let signout_status = Arc::new(AtomicUsize::new(200));
        let (start_hold, start_code, signout_hold, signout_code) = (
            start_gate.clone(),
            start_status.clone(),
            signout_gate.clone(),
            signout_status.clone(),
        );
        let claim_deadline = (Utc::now() + chrono::Duration::minutes(5)).to_rfc3339();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let (origin, polls, signouts, connections, close, refuse, outcomes) = (
                    o.clone(),
                    p.clone(),
                    s.clone(),
                    c.clone(),
                    close.clone(),
                    refuse.clone(),
                    outcomes.clone(),
                );
                let claim_deadline = claim_deadline.clone();
                let send_welcome = send_welcome.clone();
                let (start_hold, start_code, signout_hold, signout_code) = (
                    start_hold.clone(),
                    start_code.clone(),
                    signout_hold.clone(),
                    signout_code.clone(),
                );
                tokio::spawn(async move {
                    let mut bytes = Vec::new();
                    while !bytes.ends_with(b"\r\n\r\n") {
                        let Ok(b) = stream.read_u8().await else {
                            return;
                        };
                        bytes.push(b);
                        if bytes.len() > 16 * 1024 {
                            return;
                        }
                    }
                    let headers = String::from_utf8(bytes).unwrap();
                    let lower = headers.to_ascii_lowercase();
                    let len = lower
                        .lines()
                        .find_map(|l| {
                            l.strip_prefix("content-length: ")
                                .and_then(|v| v.parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    let mut body = vec![0; len];
                    stream.read_exact(&mut body).await.unwrap();
                    let mut status = 200;
                    let response = if headers.starts_with("GET /relay/host ") {
                        if lower.lines().any(|l| l.starts_with("origin:")) {
                            stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                            return;
                        }
                        assert!(lower.contains("sec-websocket-protocol: runner-relay.v1"));
                        assert!(lower.contains("authorization: bearer rhc_"));
                        let refusal = refuse.load(Ordering::SeqCst);
                        if refusal != 0 {
                            status = refusal;
                            serde_json::json!({"code":"unauthenticated"})
                        } else {
                            connections.fetch_add(1, Ordering::SeqCst);
                            let key = headers
                                .lines()
                                .find_map(|l| {
                                    l.split_once(':')
                                        .filter(|(k, _)| {
                                            k.eq_ignore_ascii_case("sec-websocket-key")
                                        })
                                        .map(|(_, v)| v.trim())
                                })
                                .unwrap();
                            let accept =
                                tokio_tungstenite::tungstenite::handshake::derive_accept_key(
                                    key.as_bytes(),
                                );
                            stream.write_all(format!("HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: {accept}\r\nSec-WebSocket-Protocol: runner-relay.v1\r\n\r\n").as_bytes()).await.unwrap();
                            let mut socket = tokio_tungstenite::WebSocketStream::from_raw_socket(
                                stream,
                                Role::Server,
                                None,
                            )
                            .await;
                            let hello = socket.next().await.unwrap().unwrap();
                            let value: serde_json::Value =
                                serde_json::from_str(hello.to_text().unwrap()).unwrap();
                            assert_eq!(value["type"], "hello");
                            if send_welcome.load(Ordering::SeqCst) {
                                socket.send(Message::Text(r#"{"type":"welcome","hostId":"7383920468475904","name":"Mac Studio","account":{"username":"jason","displayName":"Jason"}}"#.into())).await.unwrap();
                            }
                            let code = close.swap(0, Ordering::SeqCst);
                            if code != 0 {
                                let _ = socket
                                    .close(Some(CloseFrame {
                                        code: CloseCode::from(code as u16),
                                        reason: "".into(),
                                    }))
                                    .await;
                            } else {
                                while let Some(Ok(message)) = socket.next().await {
                                    if message.is_close() {
                                        break;
                                    }
                                }
                            }
                            return;
                        }
                    } else if headers.starts_with(
                        "POST /api/runner_server.v1.HostEnrollmentService/StartEnrollment ",
                    ) {
                        let gate = start_hold.lock().unwrap().clone();
                        if let Some(gate) = gate {
                            gate.wait().await;
                        }
                        assert!(!lower.contains("authorization:"));
                        let start: remote::StartRequest = serde_json::from_slice(&body).unwrap();
                        assert!(!start.host.hostname.is_empty());
                        status = start_code.load(Ordering::SeqCst);
                        if status == 200 {
                            serde_json::json!({"deviceCode":"test-device-code","userCode":"BCDF-GHJK","verificationUri":format!("{origin}/hosts/add"),"verificationUriComplete":format!("{origin}/hosts/add?code=BCDF-GHJK"),"expiresIn":600,"interval":1})
                        } else {
                            serde_json::json!({"code":"unavailable"})
                        }
                    } else if headers.starts_with(
                        "POST /api/runner_server.v1.HostEnrollmentService/PollEnrollment ",
                    ) {
                        let request: remote::PollRequest = serde_json::from_slice(&body).unwrap();
                        assert_eq!(request.device_code.0, "test-device-code");
                        let at = polls.fetch_add(1, Ordering::SeqCst);
                        let outcome = outcomes.get(at).copied().unwrap_or("approved");
                        match outcome {
                            "lost" => return,
                            "limited" => {
                                status = 429;
                                serde_json::json!({"code":"resource_exhausted","message":"rhc_testsecret test-device-code"})
                            }
                            "denied" => serde_json::json!({"state":"ENROLLMENT_STATE_DENIED"}),
                            "expired" => serde_json::json!({"state":"ENROLLMENT_STATE_EXPIRED"}),
                            "pending" => serde_json::json!({"state":"ENROLLMENT_STATE_PENDING"}),
                            _ => {
                                serde_json::json!({"state":"ENROLLMENT_STATE_APPROVED","hostId":"7383920468475904","credential":format!("rhc_{}","x".repeat(43)),"relayUrl":format!("{}/relay/host",origin.replacen("http:","ws:",1)),"account":{"username":"jason","displayName":"Jason"},"claimExpiresAt":claim_deadline.clone()})
                            }
                        }
                    } else if headers.starts_with(
                        "POST /api/runner_server.v1.HostEnrollmentService/SignOutHost ",
                    ) {
                        assert!(lower.contains("authorization: bearer rhc_"));
                        assert_eq!(body, b"{}");
                        signouts.fetch_add(1, Ordering::SeqCst);
                        let gate = signout_hold.lock().unwrap().clone();
                        if let Some(gate) = gate {
                            gate.wait().await;
                        }
                        status = signout_code.load(Ordering::SeqCst);
                        if status == 200 {
                            serde_json::json!({})
                        } else {
                            serde_json::json!({"code":"unavailable"})
                        }
                    } else {
                        panic!("unexpected fake-service route");
                    };
                    let body = response.to_string();
                    stream.write_all(format!("HTTP/1.1 {status} Result\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
                });
            }
        });
        Self {
            origin,
            task,
            polls,
            signouts,
            connections,
            close_code,
            refusal,
            welcome,
            start_gate,
            start_status,
            signout_gate,
            signout_status,
        }
    }
}
async fn until(handle: &RemoteHandle, predicate: impl Fn(&RemoteStatus) -> bool) -> RemoteStatus {
    let mut status = handle.status.clone();
    for _ in 0..100 {
        let value = status.borrow_and_update().clone();
        if predicate(&value) {
            return value;
        }
        if matches!(
            value.state,
            RemoteState::Enrolling { .. } | RemoteState::Reconnecting { .. }
        ) {
            handle.status_command(Command::TestDeadline).await.unwrap();
        } else {
            status.changed().await.unwrap();
        }
    }
    panic!(
        "fake service did not reach expected state: {:?}",
        handle.status()
    );
}
#[tokio::test(start_paused = true)]
async fn status_wall_clock_advances_independently_of_tokio_time() {
    let production = Clock::default();
    let initial = Utc::now();
    let wall = Arc::new(std::sync::Mutex::new(initial));
    let clock = Clock {
        wall: Some(wall.clone()),
    };
    let instant = tokio::time::Instant::now();
    *wall.lock().unwrap() += chrono::Duration::hours(12);
    assert_eq!(clock.now(), initial + chrono::Duration::hours(12));
    assert_eq!(tokio::time::Instant::now(), instant);
    let before = Utc::now();
    tokio::time::advance(std::time::Duration::from_secs(12 * 60 * 60)).await;
    let observed = production.now();
    assert!((before..=Utc::now()).contains(&observed));
    assert_eq!(clock.now(), initial + chrono::Duration::hours(12));
}
#[tokio::test]
async fn fake_service_app_enrollment_pause_restart_logout() {
    let service = Service::new(vec!["pending", "limited", "lost", "approved"]).await;
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(store::MemoryStore::default());
    let handle = spawn_with_store(
        root.path().into(),
        store.clone(),
        crate::events::EventChannel::new(),
    );
    let start = handle
        .start(Some(service.origin.clone()), false)
        .await
        .unwrap();
    assert!(
        matches!(start.state, RemoteState::Enrolling { .. }),
        "{start:?}"
    );
    let connected = until(&handle, |s| {
        matches!(s.state, RemoteState::Connected { .. })
    })
    .await;
    assert!(service.polls.load(Ordering::SeqCst) >= 4);
    assert!(service.connections.load(Ordering::SeqCst) > 0);
    let secret = store.get(&service.origin).unwrap().unwrap();
    let persisted = std::fs::read_to_string(root.path().join("remote.json")).unwrap();
    assert!(!persisted.contains("rhc_"));
    assert!(!persisted.contains("test-device-code"));
    handle.pause(true).await.unwrap();
    assert_eq!(store.get(&service.origin).unwrap().unwrap(), secret);
    handle.pause(false).await.unwrap();
    until(&handle, |s| {
        matches!(s.state, RemoteState::Connected { .. })
    })
    .await;
    handle.shutdown().await;
    let handle = spawn_with_store(
        root.path().into(),
        store.clone(),
        crate::events::EventChannel::new(),
    );
    until(&handle, |s| {
        matches!(s.state, RemoteState::Connected { .. })
    })
    .await;
    assert!(matches!(
        handle.logout().await.unwrap(),
        RemoteLogout::Removed
    ));
    assert!(store.get(&service.origin).unwrap().is_none());
    assert!(store.get(store::IDENTITY_NAME).unwrap().is_some());
    assert!(!root.path().join("remote.json").exists());
    assert_eq!(service.signouts.load(Ordering::SeqCst), 1);
    let again = handle
        .start(Some(service.origin.clone()), false)
        .await
        .unwrap();
    assert_eq!(connected.fingerprint, again.fingerprint);
    assert!(store.get(store::IDENTITY_NAME).unwrap().is_some());
    handle.cancel().await.unwrap();
    handle.shutdown().await;
}
#[tokio::test]
async fn fake_service_cli_allow_refuse_and_owner_disconnect() {
    let service = Service::new(vec!["approved"]).await;
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(store::MemoryStore::default());
    let handle = spawn_with_store(
        root.path().into(),
        store.clone(),
        crate::events::EventChannel::new(),
    );
    for allow in [false, true] {
        handle
            .start_owned(Some(service.origin.clone()), true, Some(0))
            .await
            .unwrap();
        until(&handle, |s| {
            matches!(s.state, RemoteState::Confirming { .. })
        })
        .await;
        assert!(store.get(&service.origin).unwrap().is_none());
        handle.confirm(allow).await.unwrap();
        if allow {
            until(&handle, |s| {
                matches!(s.state, RemoteState::Connected { .. })
            })
            .await;
            handle.logout().await.unwrap();
        } else {
            assert!(matches!(handle.status().state, RemoteState::Off));
        }
    }
    handle
        .start_owned(Some(service.origin.clone()), true, Some(0))
        .await
        .unwrap();
    until(&handle, |s| {
        matches!(s.state, RemoteState::Confirming { .. })
    })
    .await;
    handle.owner_gone(0);
    until(&handle, |s| matches!(s.state, RemoteState::Off)).await;
    handle.shutdown().await;
}
#[tokio::test]
async fn fake_service_denial_expiry_removed_retry_and_401() {
    for outcome in ["denied", "expired"] {
        let s = Service::new(vec![outcome]).await;
        let root = tempfile::tempdir().unwrap();
        let h = spawn_with_store(
            root.path().into(),
            Arc::new(store::MemoryStore::default()),
            crate::events::EventChannel::new(),
        );
        h.start(Some(s.origin.clone()), false).await.unwrap();
        until(&h, |s| matches!(s.state, RemoteState::Stopped { .. })).await;
        h.shutdown().await;
    }
    for code in [1012, 4001, 4003, 4444] {
        let s = Service::new(vec![]).await;
        s.close_code.store(code, Ordering::SeqCst);
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(store::MemoryStore::default());
        let h = spawn_with_store(
            root.path().into(),
            store.clone(),
            crate::events::EventChannel::new(),
        );
        h.start(Some(s.origin.clone()), false).await.unwrap();
        if matches!(code, 4001 | 4003) {
            until(&h, |s| matches!(s.state, RemoteState::Stopped { .. })).await;
            assert!(store.get(&s.origin).unwrap().is_some());
            h.retry().await.unwrap();
        }
        until(&h, |s| matches!(s.state, RemoteState::Connected { .. })).await;
        s.refusal.store(401, Ordering::SeqCst);
        h.pause(true).await.unwrap();
        h.pause(false).await.unwrap();
        until(&h, |s| {
            matches!(
                s.state,
                RemoteState::Stopped {
                    reason: StopReason::Removed
                }
            )
        })
        .await;
        assert!(store.get(&s.origin).unwrap().is_some());
        h.logout().await.unwrap();
        h.shutdown().await;
    }
}
#[tokio::test]
async fn offline_logout_is_local_first_once_and_restart_stays_off() {
    let s = Service::new(vec![]).await;
    let origin = s.origin.clone();
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(store::MemoryStore::default());
    let h = spawn_with_store(
        root.path().into(),
        store.clone(),
        crate::events::EventChannel::new(),
    );
    h.start(Some(origin.clone()), false).await.unwrap();
    until(&h, |s| matches!(s.state, RemoteState::Connected { .. })).await;
    drop(s);
    assert!(matches!(
        h.logout().await.unwrap(),
        RemoteLogout::StillListed { .. }
    ));
    assert!(store.get(&origin).unwrap().is_none());
    assert!(!root.path().join("remote.json").exists());
    h.shutdown().await;
    let h = spawn_with_store(
        root.path().into(),
        store,
        crate::events::EventChannel::new(),
    );
    h.shutdown().await;
    assert!(matches!(h.status().state, RemoteState::Off));
}

#[tokio::test]
async fn offline_refusal_and_dead_connection_leave_daemon_requests_serving() {
    let service = Service::new(vec![]).await;
    let origin = service.origin.clone();
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(store::MemoryStore::default());
    let mut core = crate::test_support::test_core_in(root.path().into());
    core.remote = spawn_with_store(root.path().into(), store.clone(), core.events.clone());
    let client = crate::daemon::InProcessTransport::client(core.clone());
    core.remote.start(Some(origin.clone()), true).await.unwrap();
    until(&core.remote, |s| {
        matches!(s.state, RemoteState::Confirming { .. })
    })
    .await;
    drop(service);
    core.remote.confirm(false).await.unwrap();
    assert!(matches!(core.remote.status().state, RemoteState::Off));
    assert!(core.remote.status().last_error.is_some());
    assert!(store.get(&origin).unwrap().is_none());
    assert!(store.get(store::IDENTITY_NAME).unwrap().is_some());
    assert!(client.app_version().is_ok());
    assert!(client.session_live_ids().is_ok());
    core.remote.shutdown().await;
    let service = Service::new(vec![]).await;
    let h = spawn_with_store(root.path().into(), store, core.events.clone());
    h.start(Some(service.origin.clone()), false).await.unwrap();
    until(&h, |s| matches!(s.state, RemoteState::Connected { .. })).await;
    h.status_command(Command::TestDeadline).await.unwrap();
    assert!(matches!(h.status().state, RemoteState::Reconnecting { .. }));
    assert!(client.app_version().is_ok());
    h.logout().await.unwrap();
    h.shutdown().await;
}

#[tokio::test]
async fn fake_service_refusals_origin_rejection_and_upgrade_validation() {
    let service = Service::new(vec![]).await;
    let client = transport::client().unwrap();
    let relay = format!("{}/relay/host", service.origin.replacen("http:", "ws:", 1));
    let credential = remote::Secret(format!("rhc_{}", "x".repeat(43)));
    for (status, action) in [
        (400, remote::Retry::Stop(StopReason::UpdateRequired)),
        (401, remote::Retry::Stop(StopReason::Removed)),
        (403, remote::Retry::Stop(StopReason::UpdateRequired)),
        (429, remote::Retry::After(60)),
        (503, remote::Retry::Backoff),
        (101, remote::Retry::Backoff),
    ] {
        service.refusal.store(status, Ordering::SeqCst);
        let failure =
            transport::connect(&client, &relay, &credential, &remote::HostInfo::default())
                .await
                .expect_err("fake refusal must fail");
        assert_eq!(failure.retry, action);
    }
    service.refusal.store(0, Ordering::SeqCst);
    let response = client
        .get(format!("{}/relay/host", service.origin))
        .header("Origin", &service.origin)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 403);
}

#[tokio::test]
async fn saved_stop_is_the_initial_snapshot_before_the_task_runs() {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(store::MemoryStore::default());
    let saved = Saved {
        version: 1,
        origin: "http://127.0.0.1:1".into(),
        host_id: "7383920468475904".into(),
        relay_url: "ws://127.0.0.1:1/relay/host".into(),
        account: remote::RemoteAccount {
            username: "jason".into(),
            display_name: "Jason".into(),
        },
        host_name: "Mac Studio".into(),
        fingerprint: "630d cd29 66c4 3366 9112".into(),
        activated: true,
        claim_expires_at: Utc::now(),
        paused: false,
        stopped: Some(StopReason::InUseElsewhere),
    };
    saved.save(root.path()).unwrap();
    store
        .put(
            &saved.origin,
            &serde_json::to_vec(&Secrets {
                v: 1,
                key: Some(remote::Secret(remote::encode_key(
                    &remote::HostKey::generate().unwrap().private,
                ))),
                credential: remote::Secret(format!("rhc_{}", "x".repeat(43))),
            })
            .unwrap(),
        )
        .unwrap();
    let handle = spawn_with_store(
        root.path().into(),
        store,
        crate::events::EventChannel::new(),
    );
    assert!(matches!(
        handle.status().state,
        RemoteState::Stopped {
            reason: StopReason::InUseElsewhere
        }
    ));
    handle.shutdown().await;
}

#[tokio::test]
async fn claim_deadline_before_first_welcome_deletes_the_provisional_enrollment() {
    let service = Service::new(vec![]).await;
    service.welcome.store(false, Ordering::SeqCst);
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(store::MemoryStore::default());
    let handle = spawn_with_store(
        root.path().into(),
        store.clone(),
        crate::events::EventChannel::new(),
    );
    handle
        .start(Some(service.origin.clone()), false)
        .await
        .unwrap();
    until(&handle, |s| {
        matches!(s.state, RemoteState::Connecting { .. })
    })
    .await;
    assert_eq!(service.connections.load(Ordering::SeqCst), 1);
    handle
        .status_command(Command::TestClaimDeadline)
        .await
        .unwrap();
    assert!(matches!(
        handle.status().state,
        RemoteState::Stopped {
            reason: StopReason::Expired
        }
    ));
    assert!(store.get(&service.origin).unwrap().is_none());
    assert!(store.get(store::IDENTITY_NAME).unwrap().is_some());
    assert!(!root.path().join("remote.json").exists());
    assert_eq!(service.signouts.load(Ordering::SeqCst), 0);
    handle.shutdown().await;
}

pub(super) struct SocketHarness {
    endpoint: runner_core::app_paths::IpcEndpoint,
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}
impl SocketHarness {
    pub(super) async fn new(_root: &std::path::Path, core: crate::AppCore) -> Self {
        #[cfg(unix)]
        let endpoint = runner_core::app_paths::IpcEndpoint(_root.join("remote.sock"));
        #[cfg(windows)]
        let endpoint = runner_core::app_paths::IpcEndpoint(
            format!(r"\\.\pipe\runner-remote-test-{}", uuid::Uuid::new_v4()).into(),
        );
        let mut listener = crate::ipc::IpcListener::bind(&endpoint).unwrap();
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        let task = tokio::spawn(async move {
            let mut id = 0;
            loop {
                tokio::select! {
                    _ = stop.cancelled() => break,
                    stream = listener.accept() => { let Ok(stream)=stream else {break}; id+=1; let core=core.clone(); let stop=stop.clone(); tokio::spawn(async move { let _=crate::daemon::server::remote_test_connection(stream,core,id,stop).await; }); }
                }
            }
        });
        Self {
            endpoint,
            cancel,
            task,
        }
    }
    pub(super) async fn client(
        &self,
        timeout: std::time::Duration,
    ) -> runner_core::protocol::DaemonClient {
        let endpoint = self.endpoint.clone();
        tokio::task::spawn_blocking(move || {
            runner_core::protocol::socket::SocketTransport::connect_with_timeout(
                &endpoint,
                runner_core::protocol::wire::Hello {
                    exe_sha256: "remote-test".into(),
                    client: "remote-test".into(),
                },
                timeout,
            )
            .unwrap()
            .client()
        })
        .await
        .unwrap()
    }
}
impl Drop for SocketHarness {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}
pub(super) fn socket_root() -> tempfile::TempDir {
    #[cfg(unix)]
    {
        tempfile::tempdir_in("/tmp").unwrap()
    }
    #[cfg(windows)]
    {
        tempfile::tempdir().unwrap()
    }
}
#[tokio::test]
async fn rejected_and_replaced_socket_clients_do_not_own_another_claim() {
    let service = Service::new(vec![]).await;
    for replace in [false, true] {
        let root = socket_root();
        let mut core = crate::test_support::test_core_in(root.path().into());
        core.remote = spawn_with_store(
            root.path().into(),
            Arc::new(store::MemoryStore::default()),
            core.events.clone(),
        );
        let server = SocketHarness::new(root.path(), core.clone()).await;
        let a = server.client(std::time::Duration::from_secs(10)).await;
        let b = server.client(std::time::Duration::from_secs(10)).await;
        let origin = service.origin.clone();
        let request_a = a.clone();
        let first =
            tokio::task::spawn_blocking(move || request_a.remote_enroll_start(Some(origin), true))
                .await
                .unwrap()
                .unwrap();
        if !replace {
            until(&core.remote, |s| {
                matches!(s.state, RemoteState::Confirming { .. })
            })
            .await;
        }
        let origin = service.origin.clone();
        let request_b = b.clone();
        let second =
            tokio::task::spawn_blocking(move || request_b.remote_enroll_start(Some(origin), true))
                .await
                .unwrap();
        if replace {
            let second = second.unwrap();
            assert_eq!(first.fingerprint, second.fingerprint);
            core.remote
                .status_command(Command::OwnerGone(1))
                .await
                .unwrap();
            until(&core.remote, |s| {
                matches!(s.state, RemoteState::Confirming { .. })
            })
            .await;
            assert_eq!(core.remote.status().fingerprint, second.fingerprint);
            drop(b);
            until(&core.remote, |s| matches!(s.state, RemoteState::Off)).await;
        } else {
            assert!(second.is_err());
            core.remote
                .status_command(Command::OwnerGone(2))
                .await
                .unwrap();
            assert!(matches!(
                core.remote.status().state,
                RemoteState::Confirming { .. }
            ));
            core.remote.confirm(true).await.unwrap();
            until(&core.remote, |s| {
                matches!(s.state, RemoteState::Connected { .. })
            })
            .await;
            core.remote.logout().await.unwrap();
        }
        core.remote.shutdown().await;
    }
}
#[tokio::test]
async fn incoming_text_at_claim_expiry_runs_local_cleanup() {
    for text in [
        r#"{"type":"welcome","hostId":"7383920468475904","name":"Mac Studio","account":{"username":"jason"}}"#,
        r#"{"type":"future"}"#,
    ] {
        let service = Service::new(vec![]).await;
        service.welcome.store(false, Ordering::SeqCst);
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(store::MemoryStore::default());
        let handle = spawn_with_store(
            root.path().into(),
            store.clone(),
            crate::events::EventChannel::new(),
        );
        handle
            .start(Some(service.origin.clone()), false)
            .await
            .unwrap();
        until(&handle, |s| {
            matches!(s.state, RemoteState::Connecting { .. })
        })
        .await;
        handle
            .status_command(Command::TestTextAtClaimDeadline(text.into()))
            .await
            .unwrap();
        assert!(matches!(
            handle.status().state,
            RemoteState::Stopped {
                reason: StopReason::Expired
            }
        ));
        assert!(store.get(&service.origin).unwrap().is_none());
        assert!(!root.path().join("remote.json").exists());
        assert_eq!(service.signouts.load(Ordering::SeqCst), 0);
        handle.shutdown().await;
    }
}
#[tokio::test(start_paused = true)]
async fn startup_failures_cannot_hold_shutdown_or_daemon_requests() {
    struct FailedStore;
    impl SecretStore for FailedStore {
        fn get(&self, _: &str) -> store::StoreResult<Option<Vec<u8>>> {
            Err("test secret store inaccessible")
        }
        fn put(&self, _: &str, _: &[u8]) -> store::StoreResult<()> {
            Err("test secret store inaccessible")
        }
        fn delete(&self, _: &str) -> store::StoreResult<()> {
            Err("test secret store inaccessible")
        }
    }
    for read_failure in [false, true] {
        let root = tempfile::tempdir().unwrap();
        if read_failure {
            std::fs::create_dir(root.path().join("remote.json")).unwrap();
        } else {
            Saved {
                version: 1,
                origin: "http://127.0.0.1:1".into(),
                host_id: "1".into(),
                relay_url: "ws://127.0.0.1:1/relay/host".into(),
                account: Default::default(),
                host_name: String::new(),
                fingerprint: String::new(),
                activated: true,
                claim_expires_at: Utc::now(),
                paused: true,
                stopped: None,
            }
            .save(root.path())
            .unwrap();
        }
        let mut core = crate::test_support::test_core_in(root.path().into());
        core.remote = spawn_with_store(
            root.path().into(),
            Arc::new(FailedStore),
            core.events.clone(),
        );
        let client = crate::daemon::InProcessTransport::client(core.clone());
        let mut restarts = core.remote.restarts.clone();
        while *restarts.borrow_and_update() == 0 {
            restarts.changed().await.unwrap();
        }
        assert!(client.app_version().is_ok());
        tokio::time::timeout(std::time::Duration::from_secs(4), core.remote.shutdown())
            .await
            .expect("remote shutdown must not depend on startup success");
        assert!(*core.remote.stopped.borrow());
        assert!(client.session_live_ids().is_ok());
    }
}

#[tokio::test]
async fn startup_failure_logs_reason_rejects_requests_and_never_replays_them() {
    struct StartupLogs(std::sync::Mutex<Vec<String>>);
    impl log::Log for StartupLogs {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            if record.target() == "runner_daemon::remote" && record.level() == log::Level::Error {
                self.0.lock().unwrap().push(record.args().to_string());
            }
        }
        fn flush(&self) {}
    }
    static LOGS: StartupLogs = StartupLogs(std::sync::Mutex::new(Vec::new()));
    log::set_logger(&LOGS).unwrap();
    log::set_max_level(log::LevelFilter::Error);

    struct FailingStore {
        failing: std::sync::atomic::AtomicBool,
        panic: bool,
        saved: store::MemoryStore,
        block_recovery: std::sync::atomic::AtomicBool,
        recovering: tokio::sync::Notify,
        release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl SecretStore for FailingStore {
        fn get(&self, name: &str) -> store::StoreResult<Option<Vec<u8>>> {
            if self.failing.load(Ordering::SeqCst) {
                assert!(!self.panic, "startup test panic rhc_secret_canary");
                return Err("startup test store inaccessible rhc_secret_canary");
            }
            if self.block_recovery.swap(false, Ordering::SeqCst) {
                self.recovering.notify_one();
                self.release
                    .lock()
                    .unwrap()
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
            }
            self.saved.get(name)
        }
        fn put(&self, name: &str, bytes: &[u8]) -> store::StoreResult<()> {
            self.saved.put(name, bytes)
        }
        fn delete(&self, name: &str) -> store::StoreResult<()> {
            self.saved.delete(name)
        }
    }
    for panic in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let (release, blocked) = std::sync::mpsc::channel();
        let store = Arc::new(FailingStore {
            failing: true.into(),
            panic,
            saved: store::MemoryStore::default(),
            block_recovery: true.into(),
            recovering: tokio::sync::Notify::new(),
            release: std::sync::Mutex::new(blocked),
        });
        let events = crate::events::EventChannel::new();
        let mut snapshots = events.subscribe();
        let handle = spawn_with_store(root.path().into(), store.clone(), events);
        let start = handle.start(Some("http://127.0.0.1:1".into()), false);
        let cancel = handle.logout();
        tokio::pin!(start, cancel);
        assert!(futures_util::poll!(&mut start).is_pending());
        assert!(futures_util::poll!(&mut cancel).is_pending());
        let (started, cancelled) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            tokio::join!(start, cancel)
        })
        .await
        .expect("queued requests must receive the startup error promptly");
        let error = started.unwrap_err();
        assert!(error.contains(if panic {
            "Remote startup worker failed:"
        } else {
            "startup test store inaccessible"
        }));
        assert!(error.contains("[redacted]"));
        assert!(!error.contains("rhc_secret_canary"));
        assert_eq!(cancelled.unwrap_err(), error);
        assert_eq!(handle.status().last_error.as_ref(), Some(&error));
        assert!(LOGS.0.lock().unwrap().iter().any(|line| {
            line.contains("remote task ended unexpectedly:")
                && line.contains(&error)
                && !line.contains("rhc_secret_canary")
        }));
        let event = snapshots.recv().await.unwrap();
        assert_eq!(event.name, "remote/status");
        assert_eq!(event.payload["last_error"], error);
        let new = tokio::time::timeout(std::time::Duration::from_secs(1), handle.cancel())
            .await
            .expect("new requests must fail during backoff and startup");
        assert!(new.unwrap_err().contains("startup test"));

        store.failing.store(false, Ordering::SeqCst);
        tokio::time::timeout(
            std::time::Duration::from_secs(4),
            store.recovering.notified(),
        )
        .await
        .expect("retry must reach preparation");
        let during_retry = tokio::time::timeout(std::time::Duration::from_secs(1), handle.cancel())
            .await
            .expect("requests must fail while retry preparation is pending");
        assert!(during_retry.unwrap_err().contains("startup test"));
        release.send(()).unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(4),
            until(&handle, |status| status.last_error.is_none()),
        )
        .await
        .expect("remote task must recover when its secret store recovers");
        assert!(store.saved.get(store::IDENTITY_NAME).unwrap().is_none());
        assert!(matches!(handle.status().state, RemoteState::Off));
        assert!(handle
            .set_pin(remote::Secret("001234".into()))
            .await
            .is_ok());
        handle.shutdown().await;
    }
}

struct CallGate {
    entered: tokio::sync::Notify,
    release: tokio::sync::Semaphore,
}
impl CallGate {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Semaphore::new(0),
        })
    }
    async fn wait(&self) {
        self.entered.notify_one();
        self.release.acquire().await.unwrap().forget();
    }
}
#[tokio::test]
async fn unreachable_start_returns_off_through_client_transport_and_can_start_again() {
    let service = Service::new(vec!["pending"]).await;
    service.start_status.store(503, Ordering::SeqCst);
    let start = CallGate::new();
    *service.start_gate.lock().unwrap() = Some(start.clone());
    let initial = Utc::now();
    let wall = Arc::new(std::sync::Mutex::new(initial));
    let root = socket_root();
    let mut core = crate::test_support::test_core_in(root.path().into());
    core.remote = spawn_with_clock(
        root.path().into(),
        Arc::new(store::MemoryStore::default()),
        core.events.clone(),
        Clock {
            wall: Some(wall.clone()),
        },
    );
    let server = SocketHarness::new(root.path(), core.clone()).await;
    let client = server.client(std::time::Duration::from_millis(40)).await;
    let caller = client.clone();
    let origin = service.origin.clone();
    let (done, received) = std::sync::mpsc::channel();
    let login = tokio::task::spawn_blocking(move || {
        let _ = done.send(caller.remote_enroll_start(Some(origin), false));
    });
    start.entered.notified().await;
    *wall.lock().unwrap() = initial + chrono::Duration::seconds(31);
    start.release.add_permits(1);
    let status = tokio::task::spawn_blocking(move || {
        received.recv_timeout(std::time::Duration::from_secs(2))
    })
    .await
    .unwrap()
    .expect("the pending client request must finish after the startup budget")
    .unwrap();
    login.await.unwrap();
    assert!(matches!(status.state, RemoteState::Off));
    assert_eq!(
        status.last_error.as_deref(),
        Some("Couldn't reach 127.0.0.1.")
    );
    assert!(status.fingerprint.is_none());
    core.remote
        .status_command(Command::TestDeadline)
        .await
        .unwrap();
    assert_eq!(core.remote.status(), status);
    service.start_status.store(200, Ordering::SeqCst);
    start.release.add_permits(1);
    let origin = service.origin.clone();
    let next = tokio::task::spawn_blocking(move || client.remote_enroll_start(Some(origin), false))
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(next.state, RemoteState::Enrolling { .. }));
    assert!(next.last_error.is_none());
    core.remote.logout().await.unwrap();
    core.remote.shutdown().await;
}
#[tokio::test]
async fn delayed_start_and_signout_survive_the_client_transport_budget() {
    let service = Service::new(vec![]).await;
    let root = socket_root();
    let store = Arc::new(store::MemoryStore::default());
    let mut core = crate::test_support::test_core_in(root.path().into());
    core.remote = spawn_with_store(root.path().into(), store.clone(), core.events.clone());
    let server = SocketHarness::new(root.path(), core.clone()).await;
    let client = server.client(std::time::Duration::from_millis(40)).await;
    let start = CallGate::new();
    *service.start_gate.lock().unwrap() = Some(start.clone());
    let origin = service.origin.clone();
    let caller = client.clone();
    let (done, received) = std::sync::mpsc::channel();
    let login = tokio::task::spawn_blocking(move || {
        let result = caller.remote_enroll_start(Some(origin), false);
        let _ = done.send(());
        result
    });
    start.entered.notified().await;
    assert!(matches!(
        tokio::task::spawn_blocking(
            move || received.recv_timeout(std::time::Duration::from_millis(100))
        )
        .await
        .unwrap(),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    start.release.add_permits(1);
    assert!(matches!(
        login.await.unwrap().unwrap().state,
        RemoteState::Enrolling { .. }
    ));
    until(&core.remote, |s| {
        matches!(s.state, RemoteState::Connected { .. })
    })
    .await;
    let signout = CallGate::new();
    *service.signout_gate.lock().unwrap() = Some(signout.clone());
    service.signout_status.store(503, Ordering::SeqCst);
    let caller = client.clone();
    let (done, received) = std::sync::mpsc::channel();
    let logout = tokio::task::spawn_blocking(move || {
        let result = caller.remote_logout();
        let _ = done.send(());
        result
    });
    signout.entered.notified().await;
    assert!(store.get(&service.origin).unwrap().is_none());
    assert!(store.get(store::IDENTITY_NAME).unwrap().is_some());
    assert!(!root.path().join("remote.json").exists());
    assert!(matches!(
        tokio::task::spawn_blocking(
            move || received.recv_timeout(std::time::Duration::from_millis(100))
        )
        .await
        .unwrap(),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    signout.release.add_permits(1);
    assert!(matches!(
        logout.await.unwrap().unwrap(),
        RemoteLogout::StillListed { .. }
    ));
    *service.start_gate.lock().unwrap() = None;
    core.remote
        .start(Some(service.origin.clone()), false)
        .await
        .unwrap();
    until(&core.remote, |s| {
        matches!(s.state, RemoteState::Connected { .. })
    })
    .await;
    let original = core
        .remote
        .set_pin(remote::Secret("001234".into()))
        .await
        .unwrap();
    let caller = client.clone();
    let (reset_done, reset_received) = std::sync::mpsc::channel();
    let reset = tokio::task::spawn_blocking(move || {
        let result = caller.remote_reset_identity();
        let _ = reset_done.send(());
        result
    });
    signout.entered.notified().await;
    let caller = client.clone();
    let snapshot = tokio::task::spawn_blocking(move || caller.remote_trust())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot, original);
    let (pin_done, pin_received) = std::sync::mpsc::channel();
    let queued_pin = tokio::task::spawn_blocking(move || {
        let result = client.remote_set_pin(remote::Secret("903617".into()));
        let _ = pin_done.send(());
        result
    });
    for received in [reset_received, pin_received] {
        assert!(matches!(
            tokio::task::spawn_blocking(
                move || received.recv_timeout(std::time::Duration::from_millis(100))
            )
            .await
            .unwrap(),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
    }
    signout.release.add_permits(1);
    let reset = reset.await.unwrap().unwrap();
    assert_ne!(reset.fingerprint, original.fingerprint);
    assert!(!reset.pin_set);
    let updated = queued_pin.await.unwrap().unwrap();
    assert_eq!(updated.fingerprint, reset.fingerprint);
    assert!(updated.pin_set);
    assert_eq!(
        Identity::load(store.as_ref())
            .unwrap()
            .unwrap()
            .pin
            .unwrap()
            .0,
        "903617"
    );
    assert!(matches!(core.remote.status().state, RemoteState::Off));
    core.remote.shutdown().await;
}

#[tokio::test]
async fn weak_pin_commands_are_allowed_without_unpairing_existing_devices() {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(store::MemoryStore::default());
    let handle = spawn_with_store(
        root.path().into(),
        store.clone(),
        crate::events::EventChannel::new(),
    );
    handle
        .set_pin(remote::Secret("019438".into()))
        .await
        .unwrap();
    handle
        .record_paired(vec![3; 32], "Test device".into())
        .await
        .unwrap();
    let before = handle.trust();
    for pin in [
        "000000", "123456", "111111", "121212", "654321", "112233", "123123",
    ] {
        let result = handle.set_pin(remote::Secret(pin.into())).await;
        assert!(result.is_ok(), "six-digit PIN must remain valid");
        assert!(handle.trust() == before);
        assert!(
            Identity::load(store.as_ref())
                .unwrap()
                .unwrap()
                .pin
                .unwrap()
                .0
                == pin
        );
    }
    handle.shutdown().await;
}

#[tokio::test]
async fn trust_commands_persist_without_exposing_secrets_and_reset_signs_out() {
    let service = Service::new(vec!["approved"]).await;
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(store::MemoryStore::default());
    let events = crate::events::EventChannel::new();
    let mut pushed = events.subscribe();
    let handle = spawn_with_store(root.path().into(), store.clone(), events.clone());
    assert!(handle
        .record_paired(vec![3; 32], "Browser".into())
        .await
        .is_err());
    let pin = "001234";
    let trust = handle.set_pin(remote::Secret(pin.into())).await.unwrap();
    let original = trust.fingerprint.clone();
    assert!(trust.pin_set);
    let identity = Identity::load(store.as_ref()).unwrap().unwrap();
    assert_eq!(identity.pin.as_ref().unwrap().0, pin);
    assert!(!format!("{identity:?}").contains(pin));
    assert!(!format!("{:?}", Command::SetPin(remote::Secret(pin.into()))).contains(pin));
    handle.record_failed_pin().await.unwrap();
    handle
        .record_paired(vec![3; 32], "Browser".into())
        .await
        .unwrap();
    assert_eq!(trust::Trust::load(root.path()).unwrap().failed_pins, 0);
    handle
        .set_pin(remote::Secret("903617".into()))
        .await
        .unwrap();
    assert_eq!(handle.trust().devices.len(), 1);
    for _ in 0..5 {
        handle.record_failed_pin().await.unwrap();
    }
    assert!(handle.trust().pin_locked);
    handle.shutdown().await;
    let handle = spawn_with_store(root.path().into(), store.clone(), events);
    let trust_path = root.path().join("remote-trust.json");
    let persisted = std::fs::read(&trust_path).unwrap();
    assert_eq!(
        handle.revoke_device("unknown".into()).await.unwrap_err(),
        "No paired device has fingerprint unknown."
    );
    assert_eq!(std::fs::read(&trust_path).unwrap(), persisted);
    let restored = handle.trust();
    assert!(restored.pin_locked);
    assert_eq!(restored.devices.len(), 1);
    assert!(handle
        .record_paired(vec![4; 32], "Other".into())
        .await
        .is_err());
    handle.unlock_pin().await.unwrap();
    assert!(!handle.trust().pin_locked);
    assert_eq!(trust::Trust::load(root.path()).unwrap().failed_pins, 0);
    let revoked = handle
        .revoke_device(restored.devices[0].fingerprint.clone())
        .await
        .unwrap();
    assert!(revoked.devices.is_empty());
    assert!(trust::Trust::load(root.path()).unwrap().devices.is_empty());
    handle
        .record_paired(vec![3; 32], "Browser".into())
        .await
        .unwrap();
    handle
        .start(Some(service.origin.clone()), false)
        .await
        .unwrap();
    until(&handle, |s| {
        matches!(s.state, RemoteState::Connected { .. })
    })
    .await;
    let public = serde_json::to_string(&handle.trust()).unwrap();
    let persisted = std::fs::read_to_string(root.path().join("remote-trust.json")).unwrap();
    for text in [&public, &persisted] {
        assert!(!text.contains(pin));
        assert!(!text.contains(&identity.key.0));
    }
    let mut saw_lock = false;
    while let Ok(event) = pushed.try_recv() {
        let debug = format!("{event:?}");
        assert!(!debug.contains(pin));
        assert!(!debug.contains(&identity.key.0));
        if event.name == "remote/trust" && event.payload["pin_locked"] == true {
            saw_lock = true;
        }
    }
    assert!(saw_lock);
    let reset = handle.reset_identity().await.unwrap();
    assert_ne!(reset.fingerprint, original);
    assert!(!reset.pin_set);
    assert!(!reset.pin_locked);
    assert!(reset.devices.is_empty());
    assert!(matches!(handle.status().state, RemoteState::Off));
    assert_eq!(service.signouts.load(Ordering::SeqCst), 1);
    assert!(store.get(&service.origin).unwrap().is_none());
    assert!(store.get(store::IDENTITY_NAME).unwrap().is_some());
    assert!(!root.path().join("remote.json").exists());
    assert!(Identity::load(store.as_ref())
        .unwrap()
        .unwrap()
        .pin
        .is_none());
    assert!(trust::Trust::load(root.path()).unwrap().devices.is_empty());
    handle.shutdown().await;
}

#[tokio::test]
async fn failed_reset_identity_save_preserves_trust_after_signout() {
    struct FailedIdentityStore(store::MemoryStore);
    impl SecretStore for FailedIdentityStore {
        fn get(&self, name: &str) -> store::StoreResult<Option<Vec<u8>>> {
            self.0.get(name)
        }
        fn put(&self, name: &str, value: &[u8]) -> store::StoreResult<()> {
            if name == store::IDENTITY_NAME {
                Err("test identity write failed")
            } else {
                self.0.put(name, value)
            }
        }
        fn delete(&self, name: &str) -> store::StoreResult<()> {
            self.0.delete(name)
        }
    }

    let service = Service::new(vec!["approved"]).await;
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(FailedIdentityStore(store::MemoryStore::default()));
    let mut identity = Identity::generate().unwrap();
    identity.pin = Some(remote::Secret("001234".into()));
    identity.save(&store.0).unwrap();
    let original_identity = store.get(store::IDENTITY_NAME).unwrap();
    let mut trust = trust::Trust::default();
    trust
        .record_paired(&[3; 32], "Browser".into(), Utc::now())
        .unwrap();
    for _ in 0..5 {
        trust.record_failed_pin();
    }
    trust.save(root.path()).unwrap();
    let trust_path = root.path().join("remote-trust.json");
    let original_trust = std::fs::read(&trust_path).unwrap();
    let handle = spawn_with_store(
        root.path().into(),
        store.clone(),
        crate::events::EventChannel::new(),
    );
    handle
        .start(Some(service.origin.clone()), false)
        .await
        .unwrap();
    until(&handle, |s| {
        matches!(s.state, RemoteState::Connected { .. })
    })
    .await;
    assert!(handle.reset_identity().await.is_err());
    handle.shutdown().await;

    assert_eq!(service.signouts.load(Ordering::SeqCst), 1);
    assert!(matches!(handle.status().state, RemoteState::Off));
    assert!(store.get(&service.origin).unwrap().is_none());
    assert!(!root.path().join("remote.json").exists());
    assert!(store.get(store::IDENTITY_NAME).unwrap() == original_identity);
    assert_eq!(std::fs::read(&trust_path).unwrap(), original_trust);
    let restored = trust::Trust::load(root.path()).unwrap();
    assert_eq!(restored.devices, trust.devices);
    assert!(restored.pin_locked);
}

#[tokio::test]
async fn trust_requests_round_trip_over_the_client_socket() {
    let root = socket_root();
    let mut core = crate::test_support::test_core_in(root.path().into());
    core.remote = spawn_with_store(
        root.path().into(),
        Arc::new(store::MemoryStore::default()),
        core.events.clone(),
    );
    let server = SocketHarness::new(root.path(), core.clone()).await;
    let client = server.client(std::time::Duration::from_secs(10)).await;
    let handle = core.remote.clone();
    tokio::task::spawn_blocking(move || {
        assert_eq!(
            client
                .remote_set_pin(remote::Secret("0012345".into()))
                .unwrap_err()
                .message,
            "The PIN must be exactly six digits."
        );
        assert!(!client.remote_trust().unwrap().pin_set);
        let original = client
            .remote_set_pin(remote::Secret("001234".into()))
            .unwrap();
        assert!(original.pin_set);
        assert_eq!(client.remote_trust().unwrap(), original);
        assert!(!client.remote_unlock_pin().unwrap().pin_locked);
        assert_eq!(
            client
                .remote_revoke_device("unknown".into())
                .unwrap_err()
                .message,
            "No paired device has fingerprint unknown."
        );
        let reset = client.remote_reset_identity().unwrap();
        assert_ne!(reset.fingerprint, original.fingerprint);
        assert!(!reset.pin_set);
        assert_eq!(client.remote_trust().unwrap(), reset);
    })
    .await
    .unwrap();
    handle.shutdown().await;
}

#[tokio::test]
async fn legacy_enrollment_upgrade_reconnects_with_its_original_fingerprint() {
    let service = Service::new(vec!["approved"]).await;
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(store::MemoryStore::default());
    let handle = spawn_with_store(
        root.path().into(),
        store.clone(),
        crate::events::EventChannel::new(),
    );
    handle
        .start(Some(service.origin.clone()), false)
        .await
        .unwrap();
    let connected = until(&handle, |s| {
        matches!(s.state, RemoteState::Connected { .. })
    })
    .await;
    handle.shutdown().await;
    let identity = Identity::load(store.as_ref()).unwrap().unwrap();
    let mut enrollment: Secrets =
        serde_json::from_slice(&store.get(&service.origin).unwrap().unwrap()).unwrap();
    enrollment.v = 1;
    enrollment.key = Some(identity.key);
    store
        .put(&service.origin, &serde_json::to_vec(&enrollment).unwrap())
        .unwrap();
    store.delete(store::IDENTITY_NAME).unwrap();
    let handle = spawn_with_store(
        root.path().into(),
        store.clone(),
        crate::events::EventChannel::new(),
    );
    let restored = until(&handle, |s| {
        matches!(s.state, RemoteState::Connected { .. })
    })
    .await;
    assert_eq!(restored.fingerprint, connected.fingerprint);
    assert_eq!(handle.trust().fingerprint, connected.fingerprint);
    assert_eq!(
        handle.trust().fingerprint,
        Some(remote::fingerprint(
            &Identity::load(store.as_ref())
                .unwrap()
                .unwrap()
                .host_key()
                .unwrap()
                .public
        ))
    );
    let saved: serde_json::Value =
        serde_json::from_slice(&store.get(&service.origin).unwrap().unwrap()).unwrap();
    assert_eq!(saved["v"], 2);
    assert!(saved.get("key").is_none());
    handle.shutdown().await;
}
