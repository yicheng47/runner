use super::*;

#[test]
fn an_oversized_event_tells_the_client_it_missed_one() {
    let event = |size: usize| wire::Event {
        event: Some(runner_core::protocol::ClientEvent {
            name: "files/status".into(),
            payload: serde_json::Value::String("x".repeat(size)),
        }),
        lagged: None,
    };
    let frame = event_frame(&event(1024)).unwrap();
    let decoded: wire::Event = frame.decode().unwrap();
    assert!(decoded.event.is_some());
    assert_eq!(decoded.lagged, None);

    let frame = event_frame(&event(wire::MAX_FRAME)).unwrap();
    frame.encode().unwrap();
    let decoded: wire::Event = frame.decode().unwrap();
    assert!(decoded.event.is_none());
    assert_eq!(decoded.lagged, Some(1));
}

#[test]
fn connection_cleanup_and_desktop_takeback_release_owned_compositions() {
    use runner_core::protocol::terminal::InputEvent;
    let root = tempfile::tempdir().unwrap();
    let core = crate::test_support::test_core_in(root.path().to_owned());
    let events: Arc<dyn crate::session::manager::SessionEvents> = Arc::new(core.session_events());
    core.sessions
        .prepare_unlisted_terminal("preedit", (80, 24), &core.db, &events)
        .unwrap();
    let owners = Arc::new(Mutex::default());
    let connection = |id| ConnectionWindows {
        core: core.clone(),
        owners: owners.clone(),
        id,
        closed: CancellationToken::new(),
    };
    let first = connection(1);
    let second = connection(2);
    let model = core.sessions.terminal_model("preedit").unwrap();
    let compose = || Request::terminal_observe_input {
        session_id: "preedit".into(),
        input: InputEvent::Composing { composing: true },
    };
    assert!(matches!(
        first.dispatch(compose()),
        Some(Response::terminal_observe_input(Ok(())))
    ));
    let revision = model.input_reset_guard();
    second.remove();
    assert_eq!(
        model.input_reset_guard(),
        revision,
        "unrelated connection cleanup"
    );
    first.closed.cancel();
    first.remove();
    assert!(
        model.input_reset_guard() > revision,
        "disconnect must clear the preedit"
    );
    assert!(
        first.dispatch(compose()).is_none(),
        "a queued request cannot recreate an orphan"
    );
    second.dispatch(compose()).unwrap();
    let revision = model.input_reset_guard();
    connection(3)
        .dispatch(Request::terminal_take_back {
            session_id: "preedit".into(),
            cols: 80,
            rows: 24,
        })
        .unwrap();
    assert!(
        model.input_reset_guard() > revision,
        "another desktop client's takeback clears preedit"
    );
    drop(model);
    core.sessions.drain_terminal_workers();
}

#[tokio::test]
async fn desktop_transports_reject_stale_input_after_split_tab_takeover() {
    use runner_core::protocol::terminal::TerminalController;
    let root = tempfile::tempdir().unwrap();
    let core = crate::test_support::test_core_in(root.path().to_owned());
    let events: Arc<dyn crate::session::manager::SessionEvents> = Arc::new(core.session_events());
    for id in ["left", "right"] {
        core.sessions
            .prepare_unlisted_terminal(id, (80, 24), &core.db, &events)
            .unwrap();
    }
    crate::repo::node::create_tab(
        &core.db.get().unwrap(),
        None,
        "Split",
        0,
        &serde_json::json!({"slots": ["left", "right"]}).to_string(),
    )
    .unwrap();
    let queued = [
        core.sessions.record_terminal_input("left"),
        core.sessions.record_terminal_input("right"),
    ];
    #[cfg(unix)]
    let endpoint = IpcEndpoint(root.path().join("runnerd.sock"));
    #[cfg(windows)]
    let endpoint = IpcEndpoint(std::path::PathBuf::from(format!(
        r"\\.\pipe\runner-input-control-test-{}",
        root.path().file_name().unwrap().to_string_lossy()
    )));
    let mut listener = IpcListener::bind(&endpoint).unwrap();
    let shutdown = CancellationToken::new();
    let server_shutdown = shutdown.clone();
    let server_core = core.clone();
    let server = tokio::spawn(async move {
        let stream = listener.accept().await.unwrap();
        let windows = Arc::new(ConnectionWindows {
            core: server_core.clone(),
            owners: Arc::new(Mutex::default()),
            id: 1,
            closed: CancellationToken::new(),
        });
        let (_report, resume_report) = tokio::sync::watch::channel(None);
        connection(
            stream,
            server_core,
            wire::Welcome {
                exe_sha256: "hash".into(),
                pid: 1,
                started_at: "now".into(),
                app_clients: 0,
            },
            Default::default(),
            server_shutdown,
            true,
            windows,
            resume_report,
        )
        .await
    });
    let client_core = core.clone();
    let client_shutdown = shutdown.clone();
    tokio::task::spawn_blocking(move || {
        let socket = runner_core::protocol::socket::SocketTransport::connect(
            &endpoint,
            wire::Hello {
                exe_sha256: "hash".into(),
                client: "test".into(),
            },
        )
        .unwrap();
        let local = super::super::InProcessTransport::client(client_core.clone());
        let stale = [
            local.attach("left").unwrap(),
            local.attach("right").unwrap(),
        ];
        client_core
            .sessions
            .browser_resize("left", 1, "Remote", 100, 30, true, false, &client_core.db)
            .unwrap();
        let mut errors = client_core.events.subscribe();
        for ((id, queued), stale) in ["left", "right"].into_iter().zip(&queued).zip(&stale) {
            assert_eq!(stale.snapshot.controller, TerminalController::Desktop);
            assert!(local.input(id, b"stale in-process").is_err());
            socket.client().input(id, b"stale socket").unwrap();
            super::super::block_on(async {
                tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        let event = errors.recv().await.unwrap();
                        if event.name == "session/input-error" && event.payload["session_id"] == id
                        {
                            break;
                        }
                    }
                })
                .await
                .unwrap();
            });
            assert!(queued.try_recv().is_err());
            client_core
                .sessions
                .browser_input(id, 1, b"remote")
                .unwrap();
            assert_eq!(
                queued.recv_timeout(Duration::from_secs(2)).unwrap(),
                b"remote"
            );
        }
        client_core
            .sessions
            .take_back_terminal("left", 80, 24, &client_core.db)
            .unwrap();
        for (id, queued) in ["left", "right"].into_iter().zip(&queued) {
            local.input(id, b"local in-process").unwrap();
            assert_eq!(
                queued.recv_timeout(Duration::from_secs(2)).unwrap(),
                b"local in-process"
            );
            socket.client().input(id, b"local socket").unwrap();
            assert_eq!(
                queued.recv_timeout(Duration::from_secs(2)).unwrap(),
                b"local socket"
            );
        }
        client_shutdown.cancel();
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    core.sessions.drain_terminal_workers();
}

#[tokio::test]
async fn oversized_response_fails_only_its_call_and_connection_serves_next_request() {
    let root = tempfile::tempdir().unwrap();
    let core = crate::test_support::test_core_in(root.path().to_owned());
    core.windows.set_subjects(
        "fixture",
        vec![crate::windows::Subject::DirectChat(
            "x".repeat(wire::MAX_FRAME),
        )],
    );
    #[cfg(unix)]
    let endpoint = IpcEndpoint(root.path().join("runnerd.sock"));
    #[cfg(windows)]
    let endpoint = IpcEndpoint(std::path::PathBuf::from(format!(
        r"\\.\pipe\runner-oversized-response-test-{}",
        root.path().file_name().unwrap().to_string_lossy()
    )));
    let mut listener = IpcListener::bind(&endpoint).unwrap();
    let shutdown = CancellationToken::new();
    let server_shutdown = shutdown.clone();
    let server_core = core.clone();
    let server = tokio::spawn(async move {
        let stream = listener.accept().await.unwrap();
        let windows = Arc::new(ConnectionWindows {
            core: server_core.clone(),
            owners: Arc::new(Mutex::default()),
            id: 1,
            closed: CancellationToken::new(),
        });
        let (_report, resume_report) = tokio::sync::watch::channel(None);
        connection(
            stream,
            server_core,
            wire::Welcome {
                exe_sha256: "hash".into(),
                pid: 1,
                started_at: "now".into(),
                app_clients: 0,
            },
            Default::default(),
            server_shutdown,
            true,
            windows,
            resume_report,
        )
        .await
    });
    let client_core = core.clone();
    let client_shutdown = shutdown.clone();
    tokio::task::spawn_blocking(move || {
        let socket = runner_core::protocol::socket::SocketTransport::connect(
            &endpoint,
            wire::Hello {
                exe_sha256: "hash".into(),
                client: "test".into(),
            },
        )
        .unwrap();
        let mut events = socket.event_stream();
        let error = socket.client().window_snapshot().unwrap_err();
        assert_eq!(
            error.message,
            "response too large from runnerd: frame too large (64.0 MB, limit 64 MB)"
        );
        assert!(!socket.is_closed());
        assert_eq!(socket.client().app_version().unwrap(), "0.0.0-test");
        assert!(!socket.is_closed());
        assert!(matches!(
            events.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
        client_core.windows.unregister("fixture");
        client_shutdown.cancel();
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn a_stopping_daemon_welcomes_no_new_client() {
    let root = tempfile::tempdir().unwrap();
    let core = crate::test_support::test_core_in(root.path().to_owned());
    #[cfg(unix)]
    let endpoint = IpcEndpoint(root.path().join("runnerd.sock"));
    #[cfg(windows)]
    let endpoint = IpcEndpoint(std::path::PathBuf::from(format!(
        r"\\.\pipe\runner-stopping-welcome-test-{}",
        root.path().file_name().unwrap().to_string_lossy()
    )));
    let mut listener = IpcListener::bind(&endpoint).unwrap();
    let shutdown = CancellationToken::new();
    shutdown.cancel();
    let server = tokio::spawn(async move {
        let stream = listener.accept().await.unwrap();
        remote_test_connection(stream, core, 1, shutdown).await
    });
    let connected = tokio::task::spawn_blocking(move || {
        runner_core::protocol::socket::SocketTransport::connect(
            &endpoint,
            wire::Hello {
                exe_sha256: "remote-test".into(),
                client: "app".into(),
            },
        )
        .map(|socket| socket.welcome.pid)
    })
    .await
    .unwrap();
    assert!(
        matches!(
            connected,
            Err(runner_core::protocol::socket::ConnectError::NotRunning)
        ),
        "{connected:?}"
    );
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn client_accept_errors_back_off_and_preserve_live_sessions() {
    let root = tempfile::tempdir().unwrap();
    let mut core = crate::test_support::test_core_in(root.path().to_owned());
    core.sessions = crate::session::SessionManager::new(
        core.runtime_shell_env.clone(),
        core.runtime_discovery.clone(),
        Arc::new(crate::session::pty_runtime::PtyRuntime::new()),
    );
    let spawned = crate::ops::session::session_start_shell_in(
        &core,
        runner_core::protocol::ProjectScope::Root,
        Some(root.path().to_string_lossy().into_owned()),
        Some(80),
        Some(24),
        None,
    )
    .unwrap();
    #[cfg(unix)]
    let raw_error = libc::EMFILE;
    #[cfg(windows)]
    let raw_error = windows_sys::Win32::Foundation::ERROR_NO_DATA as i32;
    #[cfg(unix)]
    let endpoint = IpcEndpoint(root.path().join("runnerd.sock"));
    #[cfg(windows)]
    let endpoint = IpcEndpoint(std::path::PathBuf::from(format!(
        r"\\.\pipe\runner-accept-test-{}",
        root.path().file_name().unwrap().to_string_lossy()
    )));
    #[cfg(unix)]
    let mcp_endpoint = IpcEndpoint(root.path().join("mcp.sock"));
    #[cfg(windows)]
    let mcp_endpoint = IpcEndpoint(std::path::PathBuf::from(format!(
        r"\\.\pipe\runner-accept-mcp-test-{}",
        root.path().file_name().unwrap().to_string_lossy()
    )));
    let mut listener = IpcListener::bind(&endpoint).unwrap();
    listener.accept_error = Some(std::io::Error::from_raw_os_error(raw_error));
    let config = Config {
        paths: NativePaths::new(root.path().to_owned(), root.path().join("logs")),
        endpoint: endpoint.clone(),
        mcp_endpoint,
        isolated: true,
    };
    let server_core = core.clone();
    let server = tokio::spawn(serve_with_listener(
        config,
        server_core,
        "hash".into(),
        Settings {
            resume_on_launch: false,
            ..Settings::default()
        },
        listener,
        #[cfg(windows)]
        Err(std::io::Error::other(
            "injected shutdown window creation failure",
        )),
    ));
    let started = tokio::time::Instant::now();
    let connected = tokio::task::spawn_blocking(move || {
        runner_core::protocol::socket::SocketTransport::connect(
            &endpoint,
            wire::Hello {
                exe_sha256: "hash".into(),
                client: "test".into(),
            },
        )
    })
    .await
    .unwrap();
    let elapsed = started.elapsed();
    let live = core.sessions.live_session_ids();
    let row = crate::repo::session::get_row(&core.db.get().unwrap(), &spawned.id)
        .unwrap()
        .unwrap();
    let stopped = if let Ok(client) = &connected {
        let remote = client.client();
        tokio::task::spawn_blocking(move || {
            assert_eq!(remote.remote_trust().unwrap(), Default::default());
            let trust = remote
                .remote_set_pin(runner_core::remote::Secret("001234".into()))
                .unwrap();
            assert!(trust.pin_set);
            assert!(trust.fingerprint.is_some());
            assert!(trust.devices.is_empty());
        })
        .await
        .unwrap();
        assert!(!root.path().join("remote.json").exists());
        assert!(!root.path().join("remote-trust.json").exists());
        assert!(!root.path().join("remote-identity.json").exists());
        assert!(!root.path().join("remote-secrets.json").exists());
        client.shutdown(true).unwrap();
        Some(tokio::time::timeout(SHUTDOWN_TIMEOUT * 2, server).await)
    } else {
        server.abort();
        crate::ops::session::session_stop(&core, &spawned.id).unwrap();
        core.sessions.drain_terminal_workers();
        None
    };
    assert!(connected.is_ok(), "accept error must return to serving");
    stopped.unwrap().unwrap().unwrap().unwrap();
    assert!(elapsed >= Duration::from_millis(100));
    assert_eq!(live, vec![spawned.id]);
    assert!(!row.resume_on_launch);
    assert_eq!(row.status, crate::model::SessionStatus::Running);
    assert!(core.sessions.live_session_ids().is_empty());
}

#[cfg(unix)]
#[test]
fn descriptor_limit_is_raised_in_an_isolated_process() {
    const CHILD: &str = "RUNNER_TEST_DESCRIPTOR_LIMIT";
    if std::env::var_os(CHILD).is_some() {
        let mut limits: libc::rlimit = unsafe { std::mem::zeroed() };
        assert_eq!(
            unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limits) },
            0
        );
        let hard = limits.rlim_max;
        limits.rlim_cur = 64;
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limits) }, 0);
        raise_descriptor_limit();
        assert_eq!(
            unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limits) },
            0
        );
        assert!(limits.rlim_cur > 64);
        assert_eq!(limits.rlim_max, hard);
        return;
    }
    assert!(std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "daemon::server::tests::descriptor_limit_is_raised_in_an_isolated_process"
        ])
        .env(CHILD, "1")
        .status()
        .unwrap()
        .success());
}

#[tokio::test]
async fn the_served_daemon_tracks_mission_states_for_reports() {
    let root = tempfile::tempdir().unwrap();
    let core = crate::test_support::test_core_in(root.path().to_owned());
    let mission = {
        let mut conn = core.db.get().unwrap();
        let crew = crate::ops::crew::create(
            &conn,
            crate::ops::crew::CreateCrewInput {
                name: "Crew".into(),
                ..Default::default()
            },
        )
        .unwrap()
        .id;
        crate::test_support::insert_test_role(&conn, "role", "worker", "codex", "codex");
        crate::test_support::insert_test_slot(&conn, "slot", &crew, "role", "worker", 0, false);
        crate::ops::mission::start(
            &mut conn,
            &core.app_data_dir,
            runner_core::protocol::MissionStart {
                crew_id: Some(crew),
                scope: runner_core::protocol::ProjectScope::Root,
                title: "Tracked".into(),
                goal_override: None,
                cwd: Some(root.path().to_string_lossy().into_owned()),
                worktree: None,
                started_by_session_id: None,
                no_report: false,
            },
        )
        .unwrap()
        .mission
        .id
    };
    #[cfg(unix)]
    let endpoint = IpcEndpoint(root.path().join("runnerd.sock"));
    #[cfg(windows)]
    let endpoint = IpcEndpoint(std::path::PathBuf::from(format!(
        r"\\.\pipe\runner-reports-test-{}",
        root.path().file_name().unwrap().to_string_lossy()
    )));
    #[cfg(unix)]
    let mcp_endpoint = IpcEndpoint(root.path().join("mcp.sock"));
    #[cfg(windows)]
    let mcp_endpoint = IpcEndpoint(std::path::PathBuf::from(format!(
        r"\\.\pipe\runner-reports-mcp-test-{}",
        root.path().file_name().unwrap().to_string_lossy()
    )));
    let config = Config {
        paths: NativePaths::new(root.path().to_owned(), root.path().join("logs")),
        endpoint: endpoint.clone(),
        mcp_endpoint,
        isolated: true,
    };
    let server = tokio::spawn(serve_with_listener(
        config,
        core.clone(),
        "hash".into(),
        Settings {
            resume_on_launch: false,
            ..Settings::default()
        },
        IpcListener::bind(&endpoint).unwrap(),
        #[cfg(windows)]
        Err(std::io::Error::other("no shutdown window in tests")),
    ));
    let client = tokio::task::spawn_blocking(move || {
        runner_core::protocol::socket::SocketTransport::connect(
            &endpoint,
            wire::Hello {
                exe_sha256: "hash".into(),
                client: "test".into(),
            },
        )
    })
    .await
    .unwrap()
    .unwrap();
    // The report service derives the state; without it the daemon never reports.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while core.reports.phase(&mission).is_none() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the served daemon must run the report service"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    client.shutdown(true).unwrap();
    tokio::time::timeout(SHUTDOWN_TIMEOUT * 2, server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn sentinel_accept_errors_back_off_and_return_to_serving() {
    #[cfg(unix)]
    let raw_error = libc::EMFILE;
    #[cfg(windows)]
    let raw_error = windows_sys::Win32::Foundation::ERROR_NO_DATA as i32;
    for error in [
        std::io::Error::from_raw_os_error(raw_error),
        std::io::ErrorKind::ConnectionAborted.into(),
    ] {
        let started = tokio::time::Instant::now();
        tokio::time::timeout(
            Duration::from_secs(1),
            discard_sentinel_connection(Err(error)),
        )
        .await
        .expect("a sentinel accept error must return to the serve loop");
        assert!(started.elapsed() >= Duration::from_millis(100));
    }
}

#[tokio::test]
async fn queued_remote_start_cannot_outlive_its_disconnected_client() {
    let core = crate::test_support::test_core();
    let windows = ConnectionWindows {
        core,
        owners: Arc::new(Mutex::default()),
        id: 1,
        closed: CancellationToken::new(),
    };
    windows.closed.cancel();
    let response = tokio::task::spawn_blocking(move || {
        windows.dispatch(Request::remote_enroll_start {
            origin: None,
            confirm_account: true,
        })
    })
    .await
    .unwrap()
    .unwrap();
    match response {
        Response::remote_enroll_start(Err(error)) => {
            assert_eq!(error.message, "Enrollment client disconnected.");
        }
        response => panic!("unexpected response: {response:?}"),
    }
}

#[test]
fn a_disconnect_releases_the_connection_s_file_watches() {
    let core = crate::test_support::test_core();
    let root = tempfile::tempdir().unwrap();
    let connection = |id| ConnectionWindows {
        core: core.clone(),
        owners: Arc::new(Mutex::default()),
        id,
        closed: CancellationToken::new(),
    };
    let (first, second) = (connection(1), connection(2));
    let watch = |windows: &ConnectionWindows| {
        windows.dispatch(Request::files_watch {
            root: root.path().to_path_buf(),
        })
    };
    assert!(matches!(watch(&first), Some(Response::files_watch(Ok(())))));
    assert!(matches!(
        watch(&second),
        Some(Response::files_watch(Ok(())))
    ));
    first.closed.cancel();
    first.remove();
    assert!(watch(&first).is_none());
    assert_eq!(core.files.watched_roots(), [root.path().to_path_buf()]);
    second.closed.cancel();
    second.remove();
    assert!(core.files.watched_roots().is_empty());
}

#[test]
fn a_watch_preparing_on_a_slow_disk_holds_up_no_other_connection() {
    use runner_core::protocol::terminal::InputEvent;
    let core = crate::test_support::test_core();
    let root = tempfile::tempdir().unwrap();
    let owners = Arc::new(Mutex::default());
    let connection = |id| {
        Arc::new(ConnectionWindows {
            core: core.clone(),
            owners: Arc::clone(&owners),
            id,
            closed: CancellationToken::new(),
        })
    };
    let (watching, typing) = (connection(1), connection(2));
    let (entered, release) = core.files.hold_next_watch();
    let path = root.path().to_path_buf();
    let worker = std::thread::spawn(move || watching.dispatch(Request::files_watch { root: path }));
    entered.recv_timeout(Duration::from_secs(10)).unwrap();

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        typing.dispatch(Request::terminal_observe_input {
            session_id: "elsewhere".into(),
            input: InputEvent::Composing { composing: false },
        });
        typing.dispatch(Request::window_register {
            label: "other".into(),
        });
        done_tx.send(()).unwrap();
    });
    let passed = done_rx.recv_timeout(Duration::from_secs(5)).is_ok();
    release.send(()).unwrap();
    assert!(matches!(
        worker.join().unwrap(),
        Some(Response::files_watch(Ok(())))
    ));
    assert!(passed, "typing waited on a watch's disk work");
    assert_eq!(core.files.watched_roots(), [root.path().to_path_buf()]);
}

#[test]
fn a_watch_preparing_when_its_connection_closes_never_starts() {
    let core = crate::test_support::test_core();
    let root = tempfile::tempdir().unwrap();
    let windows = Arc::new(ConnectionWindows {
        core: core.clone(),
        owners: Arc::new(Mutex::default()),
        id: 1,
        closed: CancellationToken::new(),
    });
    let (entered, release) = core.files.hold_next_watch();
    let (queued, path) = (windows.clone(), root.path().to_path_buf());
    let worker = std::thread::spawn(move || queued.dispatch(Request::files_watch { root: path }));
    entered.recv_timeout(Duration::from_secs(10)).unwrap();
    windows.closed.cancel();
    windows.remove();
    release.send(()).unwrap();
    assert!(worker.join().unwrap().is_none());
    assert!(core.files.watched_roots().is_empty());
}

#[test]
fn queued_window_requests_cannot_recreate_a_disconnected_window() {
    let core = crate::test_support::test_core();
    let windows = Arc::new(ConnectionWindows {
        core: core.clone(),
        owners: Arc::new(Mutex::default()),
        id: 1,
        closed: CancellationToken::new(),
    });
    windows
        .dispatch(Request::window_register {
            label: "gone".into(),
        })
        .unwrap();
    let owners = windows.owners.lock().unwrap();
    let queued = windows.clone();
    let worker = std::thread::spawn(move || {
        queued.dispatch(Request::report_subjects {
            label: "gone".into(),
            subjects: vec![crate::windows::Subject::DirectChat("chat-1".into())],
            viewed_session_id: Some("chat-1".into()),
        })
    });
    windows.closed.cancel();
    drop(owners);
    windows.remove();
    assert!(worker.join().unwrap().is_none());
    assert!(core.windows.snapshot().is_empty());
    let tab = crate::repo::node::create_tab(
        &core.db.get().unwrap(),
        None,
        "chat",
        0,
        r#"{"preset":"single","slots":[],"sizes":{}}"#,
    )
    .unwrap();
    let response = windows
        .dispatch(Request::node_mark_viewed {
            window_label: "gone".into(),
            id: tab.id,
            member_ids: Vec::new(),
            viewed_session_id: None,
        })
        .unwrap();
    assert!(matches!(response, Response::node_mark_viewed(Err(_))));
    assert!(core.windows.snapshot().is_empty());
}

#[tokio::test]
async fn changes_viewed_events_reach_only_clients_watching_that_checkout() {
    use runner_core::protocol::changes::{ChangesTarget, ChangesViewed, CHANGES_VIEWED_EVENT};
    use runner_core::protocol::{ClientEvent, EventSubscription, Transport};
    use std::process::Command;

    fn run_git(dir: &std::path::Path, args: &[&str]) {
        let out = Command::new("git")
            .current_dir(dir)
            .args([
                "-c",
                "user.name=Runner",
                "-c",
                "user.email=runner@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    async fn collect_until_barrier(
        sub: &mut dyn EventSubscription,
        barrier_value: u64,
    ) -> Vec<ClientEvent> {
        let mut viewed = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let event = sub.recv().await.unwrap();
                if event.name == "test/barrier" && event.payload == serde_json::json!(barrier_value)
                {
                    break;
                }
                if event.name == CHANGES_VIEWED_EVENT {
                    viewed.push(event);
                }
            }
        })
        .await
        .expect("barrier event did not arrive in time");
        viewed
    }

    let root = tempfile::tempdir().unwrap();
    let repo_a = root.path().join("repo_a");
    let repo_a_sub = repo_a.join("src");
    let repo_b = root.path().join("repo_b");
    std::fs::create_dir_all(&repo_a_sub).unwrap();
    std::fs::create_dir_all(&repo_b).unwrap();

    run_git(&repo_a, &["init", "-q", "-b", "main"]);
    std::fs::write(repo_a_sub.join("lib.rs"), "pub fn a() {}\n").unwrap();
    run_git(&repo_a, &["add", "-A"]);
    run_git(&repo_a, &["commit", "-q", "-m", "init a"]);
    std::fs::write(repo_a_sub.join("lib.rs"), "pub fn a() { 1 }\n").unwrap();

    run_git(&repo_b, &["init", "-q", "-b", "main"]);
    std::fs::write(repo_b.join("b.txt"), "b1\n").unwrap();
    run_git(&repo_b, &["add", "-A"]);
    run_git(&repo_b, &["commit", "-q", "-m", "init b"]);
    std::fs::write(repo_b.join("b.txt"), "b2\n").unwrap();

    let core = crate::test_support::test_core_in(root.path().to_owned());
    #[cfg(unix)]
    let endpoint = IpcEndpoint(root.path().join("runnerd.sock"));
    #[cfg(windows)]
    let endpoint = IpcEndpoint(std::path::PathBuf::from(format!(
        r"\\.\pipe\runner-changes-test-{}",
        root.path().file_name().unwrap().to_string_lossy()
    )));
    #[cfg(unix)]
    let mcp_endpoint = IpcEndpoint(root.path().join("mcp.sock"));
    #[cfg(windows)]
    let mcp_endpoint = IpcEndpoint(std::path::PathBuf::from(format!(
        r"\\.\pipe\runner-changes-mcp-test-{}",
        root.path().file_name().unwrap().to_string_lossy()
    )));
    let config = Config {
        paths: NativePaths::new(root.path().to_owned(), root.path().join("logs")),
        endpoint: endpoint.clone(),
        mcp_endpoint,
        isolated: true,
    };
    let server = tokio::spawn(serve_with_listener(
        config,
        core.clone(),
        "hash".into(),
        Settings {
            resume_on_launch: false,
            ..Settings::default()
        },
        IpcListener::bind(&endpoint).unwrap(),
        #[cfg(windows)]
        Err(std::io::Error::other("no shutdown window in tests")),
    ));

    let connect = |ep: IpcEndpoint| {
        tokio::task::spawn_blocking(move || {
            runner_core::protocol::socket::SocketTransport::connect(
                &ep,
                wire::Hello {
                    exe_sha256: "hash".into(),
                    client: "test".into(),
                },
            )
            .unwrap()
        })
    };
    let watcher_top = connect(endpoint.clone()).await.unwrap();
    let watcher_sub = connect(endpoint.clone()).await.unwrap();
    let watcher_b = connect(endpoint.clone()).await.unwrap();
    let unwatched = connect(endpoint).await.unwrap();

    let mut sub_top = watcher_top.subscribe();
    let mut sub_sub = watcher_sub.subscribe();
    let mut sub_b = watcher_b.subscribe();
    let mut sub_unwatched = unwatched.subscribe();

    {
        let client = watcher_top.client();
        let path = repo_a.clone();
        tokio::task::spawn_blocking(move || client.files_watch(&path).unwrap())
            .await
            .unwrap();
    }
    {
        let client = watcher_sub.client();
        let path = repo_a_sub.clone();
        tokio::task::spawn_blocking(move || client.files_watch(&path).unwrap())
            .await
            .unwrap();
    }
    {
        let client = watcher_b.client();
        let path = repo_b.clone();
        tokio::task::spawn_blocking(move || client.files_watch(&path).unwrap())
            .await
            .unwrap();
    }

    // Phase 1: marking in repo_a reaches top watcher and subtree watcher only.
    {
        let client = unwatched.client();
        let path = repo_a.clone();
        tokio::task::spawn_blocking(move || {
            client
                .changes_set_viewed(
                    &path,
                    &ChangesTarget::Uncommitted,
                    "src/lib.rs",
                    None,
                    None,
                    true,
                )
                .unwrap();
        })
        .await
        .unwrap();
    }
    core.events.emit("test/barrier", &1u64);

    let top_events = collect_until_barrier(&mut *sub_top, 1).await;
    let sub_events = collect_until_barrier(&mut *sub_sub, 1).await;
    let b_events = collect_until_barrier(&mut *sub_b, 1).await;
    let unwatched_events = collect_until_barrier(&mut *sub_unwatched, 1).await;

    assert_eq!(top_events.len(), 1);
    assert_eq!(sub_events.len(), 1);
    assert!(b_events.is_empty());
    assert!(unwatched_events.is_empty());
    let decoded: ChangesViewed = serde_json::from_value(sub_events[0].payload.clone()).unwrap();
    assert_eq!(decoded.path, "src/lib.rs");
    assert!(decoded.viewed);

    // Phase 2: after top unwatches, only watcher_sub gets repo_a; watcher_b only gets repo_b.
    {
        let client = watcher_top.client();
        let path = repo_a.clone();
        tokio::task::spawn_blocking(move || client.files_unwatch(&path).unwrap())
            .await
            .unwrap();
    }
    {
        let client = unwatched.client();
        let path_a = repo_a.clone();
        let path_b = repo_b.clone();
        tokio::task::spawn_blocking(move || {
            client
                .changes_set_viewed(
                    &path_a,
                    &ChangesTarget::Uncommitted,
                    "src/lib.rs",
                    None,
                    None,
                    false,
                )
                .unwrap();
            client
                .changes_set_viewed(
                    &path_b,
                    &ChangesTarget::Uncommitted,
                    "b.txt",
                    None,
                    None,
                    true,
                )
                .unwrap();
        })
        .await
        .unwrap();
    }
    core.events.emit("test/barrier", &2u64);

    let top_events_2 = collect_until_barrier(&mut *sub_top, 2).await;
    let sub_events_2 = collect_until_barrier(&mut *sub_sub, 2).await;
    let b_events_2 = collect_until_barrier(&mut *sub_b, 2).await;
    let unwatched_events_2 = collect_until_barrier(&mut *sub_unwatched, 2).await;

    assert!(top_events_2.is_empty());
    assert!(unwatched_events_2.is_empty());
    assert_eq!(sub_events_2.len(), 1);
    assert_eq!(b_events_2.len(), 1);
    let sub_decoded: ChangesViewed =
        serde_json::from_value(sub_events_2[0].payload.clone()).unwrap();
    assert_eq!(sub_decoded.path, "src/lib.rs");
    assert!(!sub_decoded.viewed);
    let b_decoded: ChangesViewed = serde_json::from_value(b_events_2[0].payload.clone()).unwrap();
    assert_eq!(b_decoded.path, "b.txt");
    assert!(b_decoded.viewed);

    drop((sub_top, sub_sub, sub_b, sub_unwatched));
    watcher_b.shutdown(true).unwrap();
    tokio::time::timeout(SHUTDOWN_TIMEOUT * 2, server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop((watcher_top, watcher_sub, watcher_b, unwatched));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connection_bounds_pending_requests_and_rejects_overflow_1377() {
    let root = tempfile::tempdir().unwrap();
    let core = crate::test_support::test_core_in(root.path().to_owned());
    #[cfg(unix)]
    let endpoint = IpcEndpoint(root.path().join("runnerd.sock"));
    #[cfg(windows)]
    let endpoint = IpcEndpoint(std::path::PathBuf::from(format!(
        r"\\.\pipe\runner-bounded-requests-test-{}",
        root.path().file_name().unwrap().to_string_lossy()
    )));
    let mut listener = IpcListener::bind(&endpoint).unwrap();
    let shutdown = CancellationToken::new();
    let owners = Arc::new(Mutex::new(HashMap::new()));
    let server_owners = owners.clone();
    let server_shutdown = shutdown.clone();
    let server_core = core.clone();
    let server = tokio::spawn(async move {
        let stream = listener.accept().await.unwrap();
        let windows = Arc::new(ConnectionWindows {
            core: server_core.clone(),
            owners: server_owners,
            id: 1,
            closed: CancellationToken::new(),
        });
        let (_report, resume_report) = tokio::sync::watch::channel(None);
        connection_with_limits(
            stream,
            server_core,
            wire::Welcome {
                exe_sha256: "hash".into(),
                pid: 1,
                started_at: "now".into(),
                app_clients: 0,
            },
            Default::default(),
            server_shutdown,
            true,
            windows,
            resume_report,
            ConnectionRequestLimits {
                max_in_flight: 1,
                max_queued: 2,
            },
        )
        .await
    });

    #[cfg(unix)]
    let mut client = tokio::net::UnixStream::connect(&endpoint.0).await.unwrap();
    #[cfg(windows)]
    let mut client = tokio::net::windows::named_pipe::ClientOptions::new()
        .open(&endpoint.0)
        .unwrap();
    write_frame(
        &mut client,
        Frame::json(
            wire::HELLO,
            &wire::Hello {
                exe_sha256: "hash".into(),
                client: "test".into(),
            },
        )
        .unwrap(),
    )
    .await
    .unwrap();
    let welcome = read_frame(&mut client).await.unwrap();
    assert_eq!(welcome.kind, wire::WELCOME);

    // Hold `owners` on a helper thread so `window_register` blocks inside dispatch.
    let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
    let (unlock_tx, unlock_rx) = std::sync::mpsc::channel();
    let hold_owners = owners.clone();
    let holder = std::thread::spawn(move || {
        let _guard = hold_owners.lock().unwrap();
        let _ = locked_tx.send(());
        let _ = unlock_rx.recv();
    });
    locked_rx.await.unwrap();

    for id in 1..=4u64 {
        write_frame(
            &mut client,
            Frame::json(
                wire::REQUEST,
                &wire::Call {
                    id,
                    request: Request::window_register {
                        label: format!("win-{id}"),
                    },
                },
            )
            .unwrap(),
        )
        .await
        .unwrap();
    }

    // Request 4 exceeds `1 in_flight + 2 queued` and must fail immediately with REQUEST_ERROR.
    let overflow = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let frame = read_frame(&mut client).await.unwrap();
            if frame.kind == wire::REQUEST_ERROR {
                return frame.decode::<wire::RequestError>().unwrap();
            }
        }
    })
    .await
    .expect("overflow request receives immediate REQUEST_ERROR");
    assert_eq!(overflow.id, 4);
    assert!(overflow.message.contains("too many pending requests"));

    let _ = unlock_tx.send(());
    holder.join().unwrap();

    let mut replies = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while replies.len() < 3 {
            let frame = read_frame(&mut client).await.unwrap();
            if frame.kind == wire::RESPONSE {
                let reply: wire::Reply = frame.decode().unwrap();
                replies.push(reply.id);
            }
        }
    })
    .await
    .expect("admitted requests 1..=3 complete once unblocked");
    replies.sort_unstable();
    assert_eq!(replies, vec![1, 2, 3]);

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(client);
}

#[test]
fn connection_cleanup_and_attachments_drop_do_not_block_single_tokio_worker_1377() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let core = crate::test_support::test_core_in(root.path().to_owned());
    let events: Arc<dyn crate::session::manager::SessionEvents> = Arc::new(core.session_events());
    core.sessions
        .prepare_unlisted_terminal("wedged", (80, 24), &core.db, &events)
        .unwrap();
    let mut attached_worker = core.sessions.attach_terminal("wedged").unwrap();
    let mut attached_detach = core.sessions.attach_terminal("wedged").unwrap();
    let mut attached_cleanup = core.sessions.attach_terminal("wedged").unwrap();
    let sub_worker = attached_worker.subscriber_id;
    let sub_detach = attached_detach.subscriber_id;
    let sub_cleanup = attached_cleanup.subscriber_id;

    let (unlock_tx, holder) = core.sessions.lock_session_state_for_test("wedged");

    let (entered_remove_tx, entered_remove_rx) = std::sync::mpsc::channel();
    let entered_remove_tx = Mutex::new(Some(entered_remove_tx));
    let worker_cancel: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        if let Some(tx) = entered_remove_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
    });

    let attachments = Arc::new(Attachments {
        core: core.clone(),
        ids: Mutex::new(HashMap::from([
            (1, ("wedged".into(), sub_worker)),
            (2, ("wedged".into(), sub_detach)),
            (3, ("wedged".into(), sub_cleanup)),
        ])),
        remote: Mutex::new(HashMap::from([(1, worker_cancel)])),
        resyncing: Mutex::default(),
    });

    // Simulate an ATTACH forwarding worker finishing `attachments.remove(1)` while `"wedged"`'s
    // `SessionState` is held: `ids` and `remote` must already be released before blocking in
    // `detach_terminal`, so `DETACH` (`remove_nonblocking`) and `ConnectionCleanup::drop` never
    // stall the single Tokio worker.
    let worker_attachments = attachments.clone();
    let bg_remove = std::thread::spawn(move || {
        worker_attachments.remove(1);
    });
    entered_remove_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("background attachment removal reached cancel before detach_terminal");
    assert!(
        !bg_remove.is_finished(),
        "background remove must still be blocked on wedged SessionState"
    );

    rt.block_on(async {
            let reader = tokio::spawn(std::future::pending::<()>());
            let cleanup = ConnectionCleanup {
                cancel: CancellationToken::new(),
                attachments: attachments.clone(),
                reader: reader.abort_handle(),
                windows: Arc::new(ConnectionWindows {
                    core: core.clone(),
                    owners: Arc::new(Mutex::default()),
                    id: 7,
                    closed: CancellationToken::new(),
                }),
            };
            attachments.remove_nonblocking(2);
            drop(cleanup);
            drop(attachments);

            let tick = tokio::time::timeout(
                Duration::from_secs(5),
                tokio::spawn(async { 99u32 }),
            )
            .await
            .expect("single Tokio worker stays responsive while detach_terminal waits on wedged session")
            .unwrap();
            assert_eq!(tick, 99);
            assert!(
                !bg_remove.is_finished(),
                "wedged SessionState is still held while Tokio worker continues serving"
            );
        });

    let _ = unlock_tx.send(());
    holder.join().unwrap();
    bg_remove.join().unwrap();
    // `detach_terminal` closes each subscriber's FrameQueue once all offloaded cleanups finish.
    assert!(attached_worker.frames.recv().is_err());
    assert!(attached_detach.frames.recv().is_err());
    assert!(attached_cleanup.frames.recv().is_err());
    core.sessions.drain_terminal_workers();
}

#[tokio::test]
async fn slow_stage_warnings_fire_past_two_seconds_for_initialize_status_and_dispatch_1377() {
    let temp = tempfile::tempdir().unwrap();
    let core = crate::test_support::test_core_in(temp.path().to_path_buf());

    // 1. Adapter::initialize (< 2s quiet, >= 2s warns on ok and deadline).
    let channel_id = 137_799u64;
    let adapter_probe =
        crate::slow_stage::SlowStageProbeGuard::install(format!("adapter:{channel_id}"));
    let arc_core = Arc::new(core.clone());
    let daemon_core = Arc::new(crate::companion::core::DaemonCore(Arc::downgrade(
        &arc_core,
    )));
    let mut adapter =
        crate::companion::Adapter::new(daemon_core.clone(), channel_id, "Test".into()).unwrap();

    adapter_probe.set_elapsed_step(Duration::from_millis(500));
    adapter.initialize().await.unwrap();
    assert!(adapter_probe.take_warnings().is_empty());

    adapter_probe.set_elapsed_step(Duration::from_millis(2_500));
    adapter.initialize().await.unwrap();
    let init_warnings = adapter_probe.take_warnings();
    assert_eq!(init_warnings.len(), 1);
    assert!(init_warnings[0].contains("slow companion Adapter::initialize"));
    assert!(init_warnings[0].contains("channel=137799"));
    assert!(init_warnings[0].contains("elapsed_ms=2500"));
    assert!(init_warnings[0].contains("outcome=ok"));

    let init_worker_gate = Arc::new(tokio::sync::Semaphore::new(1));
    let worker_permit = Arc::clone(&init_worker_gate).acquire_owned().await.unwrap();
    let held_conns: Vec<_> = (0..core.db.max_size())
        .map(|_| core.db.get().unwrap())
        .collect();
    adapter.set_initialize_timeout(Duration::ZERO);
    adapter_probe.set_elapsed_step(Duration::from_millis(5_000));
    assert_eq!(
        adapter.initialize_with_permit(Some(worker_permit)).await,
        Err("initialize deadline")
    );
    drop(held_conns);
    // Drain the timed-out spawn_blocking worker before installing core_probe on shared AppCore.
    let _drained = init_worker_gate.acquire().await.unwrap();
    let deadline_warnings = adapter_probe.take_warnings();
    assert_eq!(deadline_warnings.len(), 1);
    assert!(deadline_warnings[0].contains("slow companion Adapter::initialize"));
    assert!(deadline_warnings[0].contains("elapsed_ms=5000"));
    assert!(deadline_warnings[0].contains("outcome=deadline"));

    // 2. session_status_snapshot & ConnectionWindows::dispatch (< 2s quiet, >= 2s warns without leaking payload secrets).
    let core_probe = crate::slow_stage::SlowStageProbeGuard::install(format!(
        "core:{}",
        core.app_data_dir.display()
    ));

    core_probe.set_elapsed_step(Duration::from_millis(200));
    crate::ops::session::session_status_snapshot(&core).unwrap();
    assert!(core_probe.take_warnings().is_empty());

    core_probe.set_elapsed_step(Duration::from_millis(600));
    crate::ops::session::session_status_snapshot(&core).unwrap();
    let status_warnings = core_probe.take_warnings();
    assert_eq!(status_warnings.len(), 1);
    assert!(status_warnings[0].contains("slow session_status_snapshot"));
    assert!(status_warnings[0].contains("db_checkout_ms=600"));
    assert!(status_warnings[0].contains("rows_ms=600"));
    assert!(status_warnings[0].contains("outcome=ok"));

    let windows = Arc::new(ConnectionWindows {
        core: core.clone(),
        owners: Arc::new(Mutex::default()),
        id: 42,
        closed: CancellationToken::new(),
    });
    core_probe.set_elapsed_step(Duration::from_millis(500));
    let win_fast = Arc::clone(&windows);
    assert!(tokio::task::spawn_blocking(move || {
        win_fast.dispatch(Request::remote_set_pin {
            pin: runner_core::remote::Secret("999999".into()),
        })
    })
    .await
    .unwrap()
    .is_some());
    assert!(core_probe.take_warnings().is_empty());

    core_probe.set_elapsed_step(Duration::from_millis(2_400));
    let win_slow = Arc::clone(&windows);
    assert!(tokio::task::spawn_blocking(move || {
        win_slow.dispatch(Request::remote_set_pin {
            pin: runner_core::remote::Secret("999999".into()),
        })
    })
    .await
    .unwrap()
    .is_some());
    let dispatch_warnings = core_probe.take_warnings();
    assert_eq!(dispatch_warnings.len(), 1);
    assert!(dispatch_warnings[0]
            .contains("slow ConnectionWindows::dispatch: connection=42 method=remote_set_pin elapsed_ms=2400 cancelled=false"));
    assert!(
        !dispatch_warnings[0].contains("999999"),
        "slow dispatch logs must never include private request payload fields"
    );
}
